"""A photo tagged with an ICC profile (Adobe RGB, ProPhoto, Display P3, …) → sRGB, the space
Lumen's targets are in. Matrix/TRC profiles, the kind editors embed, are converted here in
float at any bit depth; any other profile goes through LittleCMS (Pillow) at 8 bits."""

import struct
from io import BytesIO
from typing import Callable

import numpy as np

# sRGB's colorants adapted to D50, the ICC connection space; its inverse takes XYZ to linear sRGB.
_SRGB_D50 = np.array([
    [0.4360747, 0.3850649, 0.1430804],
    [0.2225045, 0.7168786, 0.0606169],
    [0.0139322, 0.0971045, 0.7141733],
])
_XYZ_TO_SRGB = np.linalg.inv(_SRGB_D50)

Curve = Callable[[np.ndarray], np.ndarray]


def _s15(b: bytes) -> float:
    return struct.unpack(">i", b)[0] / 65536.0


def _tags(profile: bytes) -> dict[str, tuple[int, int]]:
    if len(profile) < 132 or profile[36:40] != b"acsp":
        raise ValueError("not an ICC profile")
    n = struct.unpack(">I", profile[128:132])[0]
    out = {}
    for i in range(n):
        e = profile[132 + 12 * i : 144 + 12 * i]
        out[e[:4].decode("latin-1")] = struct.unpack(">II", e[4:12])
    return out


def _xyz(profile: bytes, off: int) -> list[float]:
    if profile[off : off + 4] != b"XYZ ":
        raise ValueError("colorant is not an XYZ tag")
    return [_s15(profile[off + 8 + 4 * k : off + 12 + 4 * k]) for k in range(3)]


def _curve(profile: bytes, off: int) -> Curve:
    kind = profile[off : off + 4]
    if kind == b"curv":
        n = struct.unpack(">I", profile[off + 8 : off + 12])[0]
        if n == 0:
            return lambda x: x
        if n == 1:
            g = struct.unpack(">H", profile[off + 12 : off + 14])[0] / 256.0
            return lambda x: np.power(x, g)
        table = np.frombuffer(profile, ">u2", n, off + 12).astype(np.float64) / 65535.0
        grid = np.linspace(0.0, 1.0, n)
        return lambda x: np.interp(x, grid, table)
    if kind == b"para":
        ftype = struct.unpack(">H", profile[off + 8 : off + 10])[0]
        count = {0: 1, 1: 3, 2: 4, 3: 5, 4: 7}.get(ftype)
        if count is None:
            raise ValueError(f"parametric curve type {ftype}")
        p = [_s15(profile[off + 12 + 4 * k : off + 16 + 4 * k]) for k in range(count)]
        g = p[0]
        if ftype == 0:
            return lambda x: np.power(x, g)
        a, b = p[1], p[2]
        if ftype in (1, 2):
            c = p[3] if ftype == 2 else 0.0
            return lambda x: np.where(x >= -b / a, np.power(np.maximum(a * x + b, 0.0), g) + c, c)
        c, d = p[3], p[4]
        e, f = (p[5], p[6]) if ftype == 4 else (0.0, 0.0)
        return lambda x: np.where(x >= d, np.power(np.maximum(a * x + b, 0.0), g) + e, c * x + f)
    raise ValueError(f"curve type {kind!r}")


def matrix_shaper(profile: bytes) -> tuple[np.ndarray, list[Curve]] | None:
    """The profile's colorant matrix (columns R, G, B in XYZ D50) and its three tone curves, or
    None for an RGB profile built from LUTs. Raises ValueError for something that is no ICC profile."""
    tags = _tags(profile)
    if profile[16:20] != b"RGB ":
        raise ValueError(f"colour space {profile[16:20]!r} is not RGB")
    names = ["rXYZ", "gXYZ", "bXYZ", "rTRC", "gTRC", "bTRC"]
    if not all(n in tags for n in names):
        return None
    m = np.array([_xyz(profile, tags[n][0]) for n in names[:3]]).T
    return m, [_curve(profile, tags[n][0]) for n in names[3:]]


def _encode_srgb(lin: np.ndarray) -> np.ndarray:
    return np.where(lin <= 0.0031308, lin * 12.92, 1.055 * np.power(lin, 1 / 2.4) - 0.055)


def to_srgb(rgb: np.ndarray, profile: bytes) -> np.ndarray:
    """(h, w, 3) values in [0, 1] encoded in `profile`'s space → sRGB-encoded, float32. Colours
    outside sRGB are clipped (Lumen's targets are sRGB)."""
    ms = matrix_shaper(profile)
    if ms is None:
        return _via_littlecms(rgb, profile)
    m, curves = ms
    if _is_srgb(m, curves):  # most edits: nothing to do, and nothing to allocate at full size
        return rgb.astype(np.float32, copy=False)
    x = np.clip(rgb.astype(np.float32, copy=False), 0.0, 1.0)
    lin = np.stack([curves[k](x[..., k]).astype(np.float32) for k in range(3)], axis=-1)
    srgb_lin = lin @ (_XYZ_TO_SRGB @ m).T.astype(np.float32)
    return _encode_srgb(np.clip(srgb_lin, 0.0, 1.0)).astype(np.float32)


_PROBE = np.linspace(0.0, 1.0, 33)
_SRGB_DECODED = np.where(_PROBE <= 0.04045, _PROBE / 12.92, ((_PROBE + 0.055) / 1.055) ** 2.4)


def _is_srgb(m: np.ndarray, curves: list[Curve]) -> bool:
    """sRGB's colorants and tone curve, within what an 8-bit value could tell apart."""
    return bool(np.abs(m - _SRGB_D50).max() < 2e-3 and all(np.abs(c(_PROBE) - _SRGB_DECODED).max() < 1e-3 for c in curves))


def _via_littlecms(rgb: np.ndarray, profile: bytes) -> np.ndarray:
    from PIL import Image, ImageCms

    im = Image.fromarray((np.clip(rgb, 0.0, 1.0) * 255.0 + 0.5).astype(np.uint8), "RGB")
    out = ImageCms.profileToProfile(im, ImageCms.ImageCmsProfile(BytesIO(profile)), ImageCms.createProfile("sRGB"), outputMode="RGB")
    return np.asarray(out, dtype=np.float32) / 255.0
