"""Edits in another colour space become sRGB before they are targets (review 3): a ProPhoto TIFF
or an Adobe RGB JPEG read as sRGB would teach Auto washed-out colour."""

import struct
import zlib
from io import BytesIO
from pathlib import Path

import cv2
import numpy as np
import pytest
from PIL import Image, ImageCms

from lumen_train.common import icc
from lumen_train.retouch import build_pairs

# Adobe RGB (1998)'s colorants as its ICC profile stores them (adapted to D50), gamma 563/256.
ADOBE = [(0.6097, 0.3111, 0.0195), (0.2053, 0.6257, 0.0609), (0.1492, 0.0632, 0.7446)]


def matrix_profile(colorants, gamma: float) -> bytes:
    """A minimal ICC v2 display profile: three colorants and one gamma curve."""
    def s15(x):
        return struct.pack(">i", round(x * 65536))

    def xyz(c):
        return b"XYZ " + b"\0" * 4 + b"".join(s15(v) for v in c)

    curv = b"curv" + b"\0" * 4 + struct.pack(">IH", 1, round(gamma * 256)) + b"\0\0"
    desc_text = b"Test RGB\0"
    desc = b"desc" + b"\0" * 4 + struct.pack(">I", len(desc_text)) + desc_text + b"\0" * 8 + b"\0" * 2 + b"\0" + b"\0" * 67
    tags = [(b"desc", desc), (b"wtpt", xyz((0.9642, 1.0, 0.8249))), (b"rXYZ", xyz(colorants[0])), (b"gXYZ", xyz(colorants[1])), (b"bXYZ", xyz(colorants[2])), (b"rTRC", curv), (b"gTRC", curv), (b"bTRC", curv)]
    offset = 128 + 4 + 12 * len(tags)
    table, data = b"", b""
    for sig, body in tags:
        body += b"\0" * (-len(body) % 4)
        table += sig + struct.pack(">II", offset + len(data), len(body))
        data += body
    size = offset + len(data)
    header = struct.pack(">I", size) + b"\0" * 4 + bytes([2, 0x10, 0, 0]) + b"mntrRGB XYZ " + b"\0" * 12 + b"acsp" + b"\0" * 24 + struct.pack(">I", 0) + s15(0.9642) + s15(1.0) + s15(0.8249) + b"\0" * 48
    assert len(header) == 128
    return header + struct.pack(">I", len(tags)) + table + data


def ramp(n: int = 24) -> np.ndarray:
    g = np.linspace(0.05, 0.95, n, dtype=np.float32)
    r, gg, b = np.meshgrid(g, g[::-1], g[n // 3 :], indexing="ij")
    return np.stack([r, gg, b], -1).reshape(n * 4, -1, 3)


def lcms(rgb8: np.ndarray, profile: bytes) -> np.ndarray:
    """The reference: LittleCMS through Pillow, 8 bits."""
    im = Image.fromarray(rgb8, "RGB")
    out = ImageCms.profileToProfile(im, ImageCms.ImageCmsProfile(BytesIO(profile)), ImageCms.createProfile("sRGB"), outputMode="RGB")
    return np.asarray(out, np.float32) / 255.0


def test_srgb_stays_srgb():
    profile = ImageCms.ImageCmsProfile(ImageCms.createProfile("sRGB")).tobytes()
    rgb = ramp()
    assert np.abs(icc.to_srgb(rgb, profile) - rgb).max() < 2e-3


def test_adobe_rgb_matches_littlecms():
    profile = matrix_profile(ADOBE, 563 / 256)
    rgb8 = (ramp() * 255).round().astype(np.uint8)
    mine = icc.to_srgb(rgb8.astype(np.float32) / 255.0, profile)
    ref = lcms(rgb8, profile)
    assert np.abs(mine - ref).mean() < 1.5 / 255 and np.abs(mine - ref).max() < 4 / 255
    assert np.abs(mine - rgb8 / 255.0).mean() > 5 / 255, "Adobe RGB is not sRGB: the values must move"


def _png16_with_profile(path: Path, rgb: np.ndarray, profile: bytes) -> None:
    ok, buf = cv2.imencode(".png", (rgb[..., ::-1] * 65535).round().astype(np.uint16))
    assert ok
    png = buf.tobytes()
    body = b"Test RGB\0\0" + zlib.compress(profile)
    chunk = struct.pack(">I", len(body)) + b"iCCP" + body + struct.pack(">I", zlib.crc32(b"iCCP" + body) & 0xFFFFFFFF)
    end_of_ihdr = 8 + 8 + 13 + 4
    path.write_bytes(png[:end_of_ihdr] + chunk + png[end_of_ihdr:])


def test_read_edit_converts_a_tagged_16_bit_png_and_jpeg(tmp_path: Path):
    profile = matrix_profile(ADOBE, 563 / 256)
    rgb = ramp()
    rgb8 = (rgb * 255).round().astype(np.uint8)
    ref = lcms(rgb8, profile)
    _png16_with_profile(tmp_path / "DSC_1-Edit.png", rgb8 / 255.0, profile)
    Image.fromarray(rgb8).save(tmp_path / "DSC_1-Edit.jpg", quality=100, subsampling=0, icc_profile=profile)
    Image.fromarray(rgb8).save(tmp_path / "DSC_1-plain.jpg", quality=100, subsampling=0)
    png = build_pairs.read_edit(tmp_path / "DSC_1-Edit.png")
    assert np.abs(png - ref).mean() < 1.5 / 255
    jpg = build_pairs.read_edit(tmp_path / "DSC_1-Edit.jpg")
    assert np.abs(jpg - ref).mean() < 3 / 255
    plain = build_pairs.read_edit(tmp_path / "DSC_1-plain.jpg")
    assert np.abs(plain - rgb8 / 255.0).mean() < 3 / 255, "untagged means sRGB"


def test_littlecms_profiles_parse_and_garbage_is_an_error():
    """LittleCMS's own sRGB (parametric curves) reads as matrix/TRC; a broken tag is a ValueError,
    which skips that edit with its reason."""
    srgb = ImageCms.ImageCmsProfile(ImageCms.createProfile("sRGB")).tobytes()
    assert icc.matrix_shaper(srgb) is not None
    with pytest.raises(ValueError):
        icc.to_srgb(ramp(), b"not a profile")


def test_an_srgb_profile_leaves_the_pixels_alone():
    """Review 10: most edits are tagged sRGB; converting them costs memory at full size for nothing."""
    profile = ImageCms.ImageCmsProfile(ImageCms.createProfile("sRGB")).tobytes()
    rgb = ramp()
    out = icc.to_srgb(rgb, profile)
    assert out.dtype == np.float32 and np.array_equal(out, rgb)
    assert not np.array_equal(icc.to_srgb(rgb, matrix_profile(ADOBE, 563 / 256)), rgb)
