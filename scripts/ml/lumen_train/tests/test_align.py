"""A RAW and the JPEG a photographer made from it (Round 12-3 Task 3): his crop box and angle
found in the RAW's frame, a wrong match skipped, and his JPEG warped back onto the RAW."""

import json
import math
from pathlib import Path

import cv2
import numpy as np
import pytest
from PIL import Image

from lumen_train.retouch import align, build_pairs, params
from lumen_train.retouch.prepared import EDGE


def textured(w: int, h: int, seed: int = 0) -> np.ndarray:
    """(h, w) float32 in [0, 1]: smooth blobs at three scales, then shapes with hard edges."""
    rng = np.random.default_rng(seed)
    img = np.zeros((h, w), np.float32)
    for cells, weight in ((6, 0.5), (24, 0.3), (90, 0.2)):
        small = rng.random((max(2, cells * h // w), cells)).astype(np.float32)
        img += weight * cv2.resize(small, (w, h), interpolation=cv2.INTER_CUBIC)
    for _ in range(140):
        colour = float(rng.random())
        x, y = int(rng.integers(0, w)), int(rng.integers(0, h))
        r = int(rng.integers(4, max(6, w // 18)))
        if rng.random() < 0.5:
            cv2.circle(img, (x, y), r, colour, -1)
        else:
            cv2.rectangle(img, (x, y), (x + r, y + int(r * rng.uniform(0.4, 2.0))), colour, -1)
    return np.clip(img, 0.0, 1.0)


def similarity(box, angle: float, ref_w: int, ref_h: int, out_w: int, out_h: int) -> np.ndarray:
    """(2, 3): edited pixel coordinates (continuous, 0 at the top-left edge) → the reference's,
    for a crop `box` (unit x, y, w, h of the un-rotated crop) turned by `angle` degrees."""
    x, y, w, h = box
    s = w * ref_w / out_w
    t = math.radians(angle)
    rot = s * np.array([[math.cos(t), -math.sin(t)], [math.sin(t), math.cos(t)]])
    centre = np.array([(x + w / 2) * ref_w, (y + h / 2) * ref_h])
    return np.hstack([rot, (centre - rot @ np.array([out_w / 2, out_h / 2]))[:, None]])


def crop_rotate(ref: np.ndarray, box, angle: float, zoom: float = 1.0):
    """What the photographer exported: the crop `box` of `ref`, turned by `angle`, at `zoom`
    times the reference's resolution."""
    ref_h, ref_w = ref.shape[:2]
    out_w, out_h = round(box[2] * ref_w * zoom), round(box[3] * ref_h * zoom)
    m = similarity(box, angle, ref_w, ref_h, out_w, out_h)
    # Pixel indices: index = continuous − 0.5 on both sides.
    a, b = m[:, :2], m[:, 2]
    idx = np.hstack([a, (b + a @ np.array([0.5, 0.5]) - 0.5)[:, None]])
    out = cv2.warpAffine(ref, idx, (out_w, out_h), flags=cv2.INTER_LINEAR | cv2.WARP_INVERSE_MAP, borderMode=cv2.BORDER_REFLECT)
    return out, tuple(box), angle


def tone_change(img: np.ndarray) -> np.ndarray:
    """His edit: brighter, more contrast."""
    return np.clip(1.15 * img**0.75 - 0.05, 0.0, 1.0).astype(np.float32)


def colour(g: np.ndarray) -> np.ndarray:
    """A grey test image as a colour photo."""
    return np.stack([g, 0.85 * g, 0.6 * g + 0.2], axis=2).astype(np.float32)


def iou(a, b) -> float:
    ix = max(0.0, min(a[0] + a[2], b[0] + b[2]) - max(a[0], b[0]))
    iy = max(0.0, min(a[1] + a[3], b[1] + b[3]) - max(a[1], b[1]))
    inter = ix * iy
    return inter / (a[2] * a[3] + b[2] * b[3] - inter)


def test_recovers_crop_and_angle():
    ref = textured(800, 600)
    edited, truth_box, truth_angle = crop_rotate(ref, box=(0.1, 0.15, 0.7, 0.6), angle=2.0)
    a = align.estimate(ref, tone_change(edited))
    assert isinstance(a, align.Alignment), a
    assert iou(a.box, truth_box) >= 0.95 and abs(a.angle_deg - truth_angle) < 0.3


def test_the_other_way_round_and_a_bigger_export():
    """The sign of the angle is the matrix's, whichever way he turned it; his JPEG's resolution
    does not matter."""
    ref = textured(800, 600, seed=3)
    edited, truth_box, truth_angle = crop_rotate(ref, box=(0.2, 0.1, 0.6, 0.7), angle=-3.5, zoom=1.8)
    a = align.estimate(ref, tone_change(edited))
    assert isinstance(a, align.Alignment), a
    assert iou(a.box, truth_box) >= 0.95 and abs(a.angle_deg - truth_angle) < 0.3


def test_portrait_crop_from_landscape():
    ref = textured(900, 600)
    edited, truth_box, _ = crop_rotate(ref, box=(0.35, 0.05, 0.3, 0.9), angle=0.0)  # a tall portrait
    a = align.estimate(ref, tone_change(edited))
    assert isinstance(a, align.Alignment), a
    assert all(0.0 <= c <= 1.0 for c in (a.box[0], a.box[1], a.box[0] + a.box[2], a.box[1] + a.box[3]))
    assert iou(a.box, truth_box) >= 0.95


def test_unrelated_photo_is_skipped():
    assert isinstance(align.estimate(textured(800, 600, seed=1), textured(600, 800, seed=2)), align.Skip)


def test_a_flat_photo_is_skipped_with_a_reason():
    s = align.estimate(textured(800, 600), np.full((400, 300), 0.5, np.float32))
    assert isinstance(s, align.Skip) and s.reason


def _straighten_source(x, y, w, h, deg):
    """`photos-media::develop::geometry::straighten_source`, in continuous coordinates."""
    s, c = math.sin(math.radians(deg)), math.cos(math.radians(deg))
    z = abs(c) + abs(s) * max(w / h, h / w)
    dx, dy = (x - w / 2) / z, (y - h / 2) / z
    return c * dx + s * dy + w / 2, -s * dx + c * dy + h / 2


@pytest.mark.parametrize("angle", [2.0, -3.0])
def test_lumens_straighten_and_crop_reproduce_his_jpeg(angle):
    """Review focus 1: Lumen straightens by `straighten`, then crops `box` of the straightened
    frame; every point of his JPEG lands where the match put it in the RAW."""
    ref_w, ref_h, out_w, out_h = 1024, 683, 700, 400
    w = 0.4
    h = w * ref_w / out_w * out_h / ref_h  # the same scale both ways
    m = similarity((0.5 - w / 2, 0.5 - h / 2, w, h), angle, ref_w, ref_h, out_w, out_h)
    a = align.Alignment(angle_deg=angle, box=(0, 0, 0, 0), inliers=100, ratio=1.0, matrix=align.to_unit(m, ref_w, ref_h, out_w, out_h))
    straighten, box = align.lumen_geometry(a, ref_w / ref_h)
    assert straighten == pytest.approx(-angle)
    for qx, qy in ((0, 0), (out_w, 0), (0, out_h), (out_w, out_h), (out_w / 3, out_h / 2)):
        want = m @ np.array([qx, qy, 1.0])
        px, py = (box[0] + qx / out_w * box[2]) * ref_w, (box[1] + qy / out_h * box[3]) * ref_h
        got = _straighten_source(px, py, ref_w, ref_h, straighten)
        assert got == pytest.approx(tuple(want), abs=1e-6)


def test_lumen_box_stays_in_the_frame():
    """Review focus 2: a crop that Lumen's zoom-to-fill would cut is clamped into [0, 1]."""
    m = similarity((0.0, 0.0, 0.9, 0.9), 4.0, 1000, 800, 900, 720)
    a = align.Alignment(angle_deg=4.0, box=(0, 0, 0.9, 0.9), inliers=100, ratio=1.0, matrix=align.to_unit(m, 1000, 800, 900, 720))
    _, box = align.lumen_geometry(a, 1000 / 800)
    assert 0.0 <= box[0] and 0.0 <= box[1] and box[0] + box[2] <= 1.0 + 1e-9 and box[1] + box[3] <= 1.0 + 1e-9


@pytest.mark.parametrize(("pixels", "aspect", "want"), [(0.8, 1.5, 0), (1.5, 1.5, 1), (1.0, 1.5, 2), (1.25 * 1.004, 1.5, 3), (0.8, 0.66, 3), (1.6, 1.5, 0)])
def test_crop_ratio_is_the_nearest_or_free(pixels, aspect, want):
    """Review focus 2: a 4:5 portrait from a landscape RAW has no bin (bins follow the photo's
    orientation): free. A ratio within 1 % of a bin is that bin."""
    assert align.ratio_bin(pixels, aspect) == want


def test_warp_back_puts_his_jpeg_on_the_raw():
    ref = textured(800, 600)
    edited, box, angle = crop_rotate(ref, box=(0.1, 0.15, 0.7, 0.6), angle=2.0, zoom=1.3)
    a = align.estimate(ref, edited)
    rgb, valid = align.warp_back(np.repeat(edited[..., None], 3, axis=2), a, 200, 150)
    assert rgb.shape == (150, 200, 3) and valid.shape == (150, 200)
    small = cv2.resize(ref, (200, 150), interpolation=cv2.INTER_AREA)
    assert np.abs(rgb[valid][:, 0] - small[valid]).mean() < 0.03
    assert valid.mean() == pytest.approx(0.7 * 0.6, abs=0.04)
    assert not valid[0, 0] and not valid[-1, -1] and valid[75, 100]


# ---- build_pairs --align ---------------------------------------------------------------------


def _prepared(root: Path, name: str, source: Path, w: int, h: int) -> Path:
    folder = root / name
    folder.mkdir(parents=True)
    np.zeros(4 * EDGE * EDGE, "<u2").tofile(folder / "input.u16")
    (folder / "meta.json").write_text(json.dumps({"width": w, "height": h, "aspect": w / h, "edge": EDGE, "source": str(source)}), encoding="utf-8")
    return folder


def test_build_aligned_writes_partial_targets_and_skips(tmp_path: Path):
    originals = tmp_path / "contributor"
    (originals / "raw").mkdir(parents=True)
    (originals / "jpeg").mkdir()
    scenes = {"dsc_0001": textured(800, 600, seed=10), "dsc_0002": textured(800, 600, seed=11), "dsc_0003": textured(800, 600, seed=12), "dsc_0004": textured(800, 600, seed=13)}
    for stem in scenes:
        (originals / "raw" / f"{stem.upper()}.NEF").write_bytes(b"raw")
    edited, _, _ = crop_rotate(scenes["dsc_0001"], (0.1, 0.15, 0.7, 0.6), 2.0, zoom=1.2)
    Image.fromarray((colour(tone_change(edited)) * 255).astype(np.uint8)).save(originals / "jpeg" / "DSC_0001-Edit.jpg")
    (originals / "jpeg" / "DSC_0001.JPG").write_bytes(b"the camera's own JPEG: never read, his export wins")
    bw, _, _ = crop_rotate(scenes["dsc_0004"], (0.1, 0.1, 0.8, 0.8), 1.0)
    Image.fromarray((bw * 255).astype(np.uint8)).convert("RGB").save(originals / "jpeg" / "DSC_0004-Edit.jpg")  # his black and white
    Image.fromarray((textured(600, 800, seed=99) * 255).astype(np.uint8)).convert("RGB").save(originals / "jpeg" / "DSC_0002.jpg")  # not his crop of this RAW
    prepared = tmp_path / "train" / "contributor-pairs"
    raw1 = _prepared(prepared, "dsc_0001-0000000000000001", originals / "raw" / "DSC_0001.NEF", 256, 192)
    raw2 = _prepared(prepared, "dsc_0002-0000000000000002", originals / "raw" / "DSC_0002.NEF", 256, 192)
    _prepared(prepared, "dsc_0003-0000000000000003", originals / "raw" / "DSC_0003.NEF", 256, 192)  # no JPEG
    _prepared(prepared, "dsc_0004-0000000000000005", originals / "raw" / "DSC_0004.NEF", 256, 192)
    jpeg = _prepared(prepared, "dsc_0001-edit-0000000000000004", originals / "jpeg" / "DSC_0001-Edit.jpg", 256, 192)

    def reference(photo: Path) -> np.ndarray:
        return colour(scenes[photo.stem.lower()])

    made = build_pairs.build_aligned(prepared, originals, reference)
    assert made == 1
    t = json.loads((raw1 / "target.json").read_text(encoding="utf-8"))
    assert t["known"] == ["straighten", "crop_on", "crop_x", "crop_y", "crop_w", "crop_h", "crop_ratio"]
    assert abs(t["aligned"]["angle"] - 2.0) < 0.3 and t["aligned"]["inliers"] >= 40
    v = t["v"]
    assert params.PARAMS[params.index("straighten")]["min"] + v[params.index("straighten")] * (params.PARAMS[params.index("straighten")]["max"] - params.PARAMS[params.index("straighten")]["min"]) == pytest.approx(-2.0, abs=0.3)
    assert v[params.index("crop_on")] == 1.0
    assert v[params.index("exposure")] == pytest.approx(params.neutral_unit("exposure"))
    mask = np.fromfile(raw1 / "target_mask.u8", dtype=np.uint8).reshape(EDGE, EDGE)
    assert mask[:192, :256].mean() == pytest.approx(0.42, abs=0.05) and mask[192:].sum() == 0 and mask[:, 256:].sum() == 0
    assert (raw1 / "target.u16").stat().st_size == 4 * EDGE * EDGE * 2
    assert not (raw2 / "target.json").exists() and not (jpeg / "target.json").exists()
    lines = [json.loads(x) for x in (prepared / "pairs.jsonl").read_text(encoding="utf-8").splitlines()]
    by = {Path(x["raw"]).stem.lower(): x for x in lines}
    assert by["dsc_0001"]["status"] == "ok"
    assert by["dsc_0002"]["status"] == "skipped" and by["dsc_0002"]["reason"]
    assert by["dsc_0003"]["status"] == "skipped" and "jpeg" in by["dsc_0003"]["reason"].lower()
    assert by["dsc_0004"]["status"] == "skipped" and "black and white" in by["dsc_0004"]["reason"]
    assert by["dsc_0001"]["jpeg"].endswith("DSC_0001-Edit.jpg")
    assert len(lines) == 4  # the JPEG's own prepared folder is not a RAW


def test_png_and_tiff_edits_pair_and_win_over_a_jpeg(tmp_path: Path):
    """His export can be a 16-bit PNG or TIFF with a Lightroom ending; a lossless export of the
    same photo wins over a JPEG one."""
    originals = tmp_path / "contributor"
    originals.mkdir()
    scenes = {"dsc_0005": textured(800, 600, seed=20), "dsc_0006": textured(800, 600, seed=21)}
    for stem in scenes:
        (originals / f"{stem.upper()}.NEF").write_bytes(b"raw")
    e5, _, _ = crop_rotate(scenes["dsc_0005"], (0.1, 0.15, 0.7, 0.6), 2.0, zoom=1.2)
    cv2.imwrite(str(originals / "DSC_0005-Enhanced-NR.png"), (colour(tone_change(e5))[..., ::-1] * 65535).astype(np.uint16))
    Image.fromarray((colour(tone_change(e5)) * 255).astype(np.uint8)).save(originals / "DSC_0005.jpg")
    e6, _, _ = crop_rotate(scenes["dsc_0006"], (0.2, 0.1, 0.6, 0.7), -1.5)
    cv2.imwrite(str(originals / "DSC_0006-Edit.tif"), (colour(tone_change(e6))[..., ::-1] * 65535).astype(np.uint16))
    prepared = tmp_path / "train" / "contributor-pairs"
    _prepared(prepared, "dsc_0005-0000000000000001", originals / "DSC_0005.NEF", 256, 192)
    _prepared(prepared, "dsc_0006-0000000000000002", originals / "DSC_0006.NEF", 256, 192)

    made = build_pairs.build_aligned(prepared, originals, lambda p: colour(scenes[p.stem.lower()]))
    by = {Path(x["raw"]).stem.lower(): x for x in map(json.loads, (prepared / "pairs.jsonl").read_text(encoding="utf-8").splitlines())}
    assert made == 2, by
    assert by["dsc_0005"]["jpeg"].endswith("DSC_0005-Enhanced-NR.png")
    assert by["dsc_0006"]["jpeg"].endswith("DSC_0006-Edit.tif") and abs(by["dsc_0006"]["angle"] + 1.5) < 0.3


def test_the_next_edit_is_tried_when_the_first_does_not_match(tmp_path: Path):
    """A PNG that is some other edit of the scene must not cost the pair its matching JPEG."""
    originals = tmp_path / "contributor"
    originals.mkdir()
    scene = textured(800, 600, seed=40)
    (originals / "DSC_0207.NEF").write_bytes(b"raw")
    cv2.imwrite(str(originals / "DSC_0207.png"), (textured(600, 800, seed=41) * 65535).astype(np.uint16))
    edited, _, _ = crop_rotate(scene, (0.15, 0.1, 0.7, 0.7), 1.0)
    Image.fromarray((colour(tone_change(edited)) * 255).astype(np.uint8)).save(originals / "DSC_0207.jpg")
    prepared = tmp_path / "train" / "s"
    _prepared(prepared, "dsc_0207-0000000000000001", originals / "DSC_0207.NEF", 256, 192)
    assert build_pairs.build_aligned(prepared, originals, lambda p: colour(scene)) == 1
    line = json.loads((prepared / "pairs.jsonl").read_text(encoding="utf-8"))
    assert line["status"] == "ok" and line["jpeg"].endswith("DSC_0207.jpg")


def test_a_finer_pass_when_the_first_finds_too_few_matches(tmp_path: Path):
    """Tight crops lose their detail at 1024 px: a second render and match at a finer size, with
    the same bars, only when the first pass is short of matches."""
    originals = tmp_path / "contributor"
    originals.mkdir()
    scene = textured(800, 600, seed=50)
    (originals / "DSC_0258.NEF").write_bytes(b"raw")
    edited, _, _ = crop_rotate(scene, (0.2, 0.2, 0.6, 0.6), 0.5)
    Image.fromarray((colour(tone_change(edited)) * 255).astype(np.uint8)).save(originals / "DSC_0258.jpg")
    prepared = tmp_path / "train" / "s"
    _prepared(prepared, "dsc_0258-0000000000000001", originals / "DSC_0258.NEF", 256, 192)
    flat = np.full((600, 800, 3), 0.5, np.float32)  # the coarse render finds nothing
    fine = []
    made = build_pairs.build_aligned(prepared, originals, lambda p: flat, render_fine=lambda p: fine.append(p) or colour(scene))
    line = json.loads((prepared / "pairs.jsonl").read_text(encoding="utf-8"))
    assert made == 1 and line["pass"] == "fine" and len(fine) == 1


def test_a_photo_whose_raw_is_gone_loses_its_pair(tmp_path: Path):
    """Julian replaced an HE NEF by its DNG: the NEF's stand-in folder must not pair again (the
    photo would train twice), and its old target goes."""
    originals = tmp_path / "contributor"
    originals.mkdir()
    scene = textured(800, 600, seed=60)
    edited, _, _ = crop_rotate(scene, (0.1, 0.1, 0.8, 0.75), 1.0)
    Image.fromarray((colour(tone_change(edited)) * 255).astype(np.uint8)).save(originals / "DSC_5837.jpg")
    prepared = tmp_path / "train" / "s"
    stand_in = prepared / "_embedded" / "DSC_5837.jpg"
    stand_in.parent.mkdir(parents=True)
    stand_in.write_bytes(b"camera jpeg")
    old = _prepared(prepared, "dsc_5837-0000000000000009", stand_in, 256, 192)
    (old / "stand_in.json").write_text(json.dumps({"raw": str(originals / "DSC_5837.NEF"), "via": "embedded JPEG"}), encoding="utf-8")
    (old / "target.json").write_text("{}", encoding="utf-8")  # paired before the NEF was deleted
    (originals / "DSC_5837.DNG").write_bytes(b"dng")
    _prepared(prepared, "dsc_5837-000000000000000a", originals / "DSC_5837.DNG", 256, 192)
    (originals / "DSC_0001.NEF").write_bytes(b"raw")
    gone = _prepared(prepared, "dsc_0002-000000000000000b", originals / "DSC_0002.NEF", 256, 192)  # deleted RAW
    (gone / "target.json").write_text("{}", encoding="utf-8")

    assert build_pairs.build_aligned(prepared, originals, lambda p: colour(scene)) == 1
    by = {x["folder"]: x for x in map(json.loads, (prepared / "pairs.jsonl").read_text(encoding="utf-8").splitlines())}
    assert by["dsc_5837-000000000000000a"]["status"] == "ok"
    assert by["dsc_5837-0000000000000009"]["status"] == "skipped" and "gone" in by["dsc_5837-0000000000000009"]["reason"]
    assert not (old / "target.json").exists() and not (gone / "target.json").exists()


def test_read_edit_keeps_sixteen_bits_and_turns_upright(tmp_path: Path):
    ramp = np.linspace(0, 1, 300 * 200 * 3, dtype=np.float32).reshape(200, 300, 3)
    cv2.imwrite(str(tmp_path / "a.png"), (ramp * 65535).astype(np.uint16))
    got = build_pairs.read_edit(tmp_path / "a.png")
    assert got.shape == (200, 300, 3) and got.dtype == np.float32
    assert len(np.unique(got[..., 0])) > 256  # not squeezed to 8 bits
    im = Image.fromarray((ramp * 255).astype(np.uint8))
    exif = im.getexif()
    exif[0x0112] = 6  # turned: shown 90° clockwise
    im.save(tmp_path / "b.jpg", exif=exif)
    assert build_pairs.read_edit(tmp_path / "b.jpg").shape == (300, 200, 3)


def test_a_stand_in_pairs_as_its_raw(tmp_path: Path):
    """A RAW Lumen cannot decode (Nikon HE*) was prepared from its embedded JPEG: the pair is the
    RAW's, the reference is rendered from the stand-in, and the line says so."""
    originals = tmp_path / "contributor"
    originals.mkdir()
    scene = textured(800, 600, seed=30)
    (originals / "DSC_5837.NEF").write_bytes(b"high efficiency")
    edited, _, _ = crop_rotate(scene, (0.1, 0.1, 0.8, 0.75), 1.0)
    Image.fromarray((colour(tone_change(edited)) * 255).astype(np.uint8)).save(originals / "DSC_5837.jpg")
    prepared = tmp_path / "train" / "contributor-pairs"
    stand_in = prepared / "_embedded" / "DSC_5837.jpg"
    stand_in.parent.mkdir(parents=True)
    stand_in.write_bytes(b"camera jpeg")
    folder = _prepared(prepared, "dsc_5837-0000000000000009", stand_in, 256, 192)
    (folder / "stand_in.json").write_text(json.dumps({"raw": str(originals / "DSC_5837.NEF"), "via": "embedded JPEG"}), encoding="utf-8")
    rendered = []

    def reference(photo: Path) -> np.ndarray:
        rendered.append(photo)
        return colour(scene)

    assert build_pairs.build_aligned(prepared, originals, reference) == 1
    assert rendered == [stand_in]
    line = json.loads((prepared / "pairs.jsonl").read_text(encoding="utf-8").splitlines()[0])
    assert Path(line["raw"]) == originals / "DSC_5837.NEF" and line["via"] == "embedded JPEG"
    assert Path(line["jpeg"]) == originals / "DSC_5837.jpg"  # his edit, never the stand-in


def _jpeg_with_exif(path: Path, rgb: np.ndarray, software: str | None, model: str | None) -> None:
    exif = Image.Exif()
    if model:
        exif[0x010F], exif[0x0110] = "NIKON CORPORATION", model
    if software:
        exif[0x0131] = software
    Image.fromarray((rgb * 255).astype(np.uint8)).convert("RGB").save(path, exif=exif.tobytes())


@pytest.mark.parametrize("software, model, camera", [
    ("Ver.1.03 ", "NIKON D7100", True),
    (None, "NIKON Z f", True),
    ("17.5.1", "iPhone 15 Pro", True),
    ("NIKON D7100 Ver.1.03", "NIKON D7100", True),
    ("Adobe Photoshop Lightroom Classic 13.2 (Windows)", "NIKON D7100", False),
    ("Adobe Photoshop 25.5 (Windows)", None, False),
    (None, None, False),
])
def test_camera_jpegs_are_told_from_exports(tmp_path: Path, software, model, camera):
    """Review 4: by EXIF Software and camera model, never by file name."""
    _jpeg_with_exif(tmp_path / "x.jpg", textured(64, 48), software, model)
    assert build_pairs.camera_jpeg(tmp_path / "x.jpg") is camera


def test_a_camera_jpeg_is_never_the_edit(tmp_path: Path):
    """RAW+JPEG shooting with no export: the camera's JPEG matches the RAW perfectly, and would
    teach Nikon's Picture Control as his edit. Skipped with a reason; with an export beside it,
    the export pairs whatever its name."""
    originals = tmp_path / "contributor"
    originals.mkdir()
    scenes = {"dsc_0301": textured(800, 600, seed=50), "dsc_0302": textured(800, 600, seed=51)}
    for stem, scene in scenes.items():
        (originals / f"{stem.upper()}.NEF").write_bytes(b"raw")
        _jpeg_with_exif(originals / f"{stem.upper()}.JPG", colour(scene), "Ver.1.10", "NIKON Z f")
    edited, _, _ = crop_rotate(scenes["dsc_0302"], (0.1, 0.15, 0.7, 0.6), 2.0)
    _jpeg_with_exif(originals / "DSC_0302-Edit.jpg", colour(tone_change(edited)), "Adobe Photoshop Lightroom Classic 13.2 (Windows)", "NIKON Z f")
    prepared = tmp_path / "train" / "contributor-pairs"
    _prepared(prepared, "dsc_0301-0000000000000001", originals / "DSC_0301.NEF", 256, 192)
    _prepared(prepared, "dsc_0302-0000000000000002", originals / "DSC_0302.NEF", 256, 192)
    made = build_pairs.build_aligned(prepared, originals, lambda p: colour(scenes[p.stem.lower()]))
    by = {Path(x["raw"]).stem.lower(): x for x in map(json.loads, (prepared / "pairs.jsonl").read_text(encoding="utf-8").splitlines())}
    assert made == 1
    assert by["dsc_0301"]["status"] == "skipped" and "camera" in by["dsc_0301"]["reason"]
    assert by["dsc_0302"]["status"] == "ok" and by["dsc_0302"]["jpeg"].endswith("DSC_0302-Edit.jpg")


def test_an_edit_named_as_one_is_never_a_camera_jpeg(tmp_path: Path):
    """Review 8: an editor that keeps the camera's Software tag still names its export `-Edit`."""
    _jpeg_with_exif(tmp_path / "DSC_1-Edit.jpg", textured(64, 48), "Ver.1.03", "NIKON D7100")
    _jpeg_with_exif(tmp_path / "DSC_1-Enhanced-NR.jpg", textured(64, 48), None, "NIKON D7100")
    _jpeg_with_exif(tmp_path / "DSC_1.JPG", textured(64, 48), "Ver.1.03", "NIKON D7100")
    assert not build_pairs.camera_jpeg(tmp_path / "DSC_1-Edit.jpg")
    assert not build_pairs.camera_jpeg(tmp_path / "DSC_1-Enhanced-NR.jpg")
    assert build_pairs.camera_jpeg(tmp_path / "DSC_1.JPG")


def test_an_unreadable_edit_does_not_cost_the_pair(tmp_path: Path):
    """Review 7: a broken TIFF is tried first (lossless wins) and fails; the JPEG still pairs."""
    originals = tmp_path / "contributor"
    originals.mkdir()
    scene = textured(800, 600, seed=60)
    (originals / "DSC_0401.NEF").write_bytes(b"raw")
    (originals / "DSC_0401-Edit.tif").write_bytes(b"not a TIFF")
    edited, _, _ = crop_rotate(scene, (0.1, 0.15, 0.7, 0.6), 1.0)
    Image.fromarray((colour(tone_change(edited)) * 255).astype(np.uint8)).save(originals / "DSC_0401-Edit.jpg")
    prepared = tmp_path / "train" / "s"
    _prepared(prepared, "dsc_0401-0000000000000001", originals / "DSC_0401.NEF", 256, 192)
    assert build_pairs.build_aligned(prepared, originals, lambda p: colour(scene)) == 1
    line = json.loads((prepared / "pairs.jsonl").read_text(encoding="utf-8"))
    assert line["status"] == "ok" and line["jpeg"].endswith("DSC_0401-Edit.jpg")


def test_same_named_raws_on_two_cards_keep_their_own_edits(tmp_path: Path):
    """Review 4: `a/DSC_0001.NEF` and `b/DSC_0001.NEF` of one scene: each pairs with the edit
    beside it, never the other card's (newer) one, though both would align."""
    import os

    originals = tmp_path / "contributor"
    scene = textured(800, 600, seed=70)
    boxes = {"a": (0.1, 0.15, 0.7, 0.6), "b": (0.2, 0.1, 0.6, 0.7)}
    prepared = tmp_path / "train" / "s"
    for i, (card, box) in enumerate(boxes.items()):
        (originals / card).mkdir(parents=True)
        (originals / card / "DSC_0001.NEF").write_bytes(b"raw")
        edited, _, _ = crop_rotate(scene, box, 0.0)
        path = originals / card / "DSC_0001-Edit.jpg"
        Image.fromarray((colour(tone_change(edited)) * 255).astype(np.uint8)).save(path)
        os.utime(path, (1_000_000 + i * 1000, 1_000_000 + i * 1000))
        _prepared(prepared, f"dsc_0001-000000000000000{i}", originals / card / "DSC_0001.NEF", 256, 192)
    assert build_pairs.build_aligned(prepared, originals, lambda p: colour(scene)) == 2
    for x in map(json.loads, (prepared / "pairs.jsonl").read_text(encoding="utf-8").splitlines()):
        assert Path(x["jpeg"]).parent == Path(x["raw"]).parent, x


def test_a_ratio_snapped_crop_keeps_its_centre(monkeypatch):
    """Review 5: a snapped height clamped to the frame narrows the crop around its centre."""
    seen = {}
    monkeypatch.setattr(align, "lumen_geometry", lambda a, aspect: (0.0, (0.1, 0.0, 0.6, 0.995)))
    monkeypatch.setattr(align, "ratio_bin", lambda r, aspect: 1)
    monkeypatch.setattr(params, "_ratio_of", lambda b, aspect: 0.8)
    monkeypatch.setattr(params, "encode_develop_json", lambda dev, crop, s, aspect: seen.setdefault("crop", crop))
    build_pairs.geometry_vector(None, 1.5, 0.8)
    c = seen["crop"]
    assert c["h"] == pytest.approx(1.0) and c["w"] == pytest.approx(0.8 / 1.5)
    assert c["x"] + c["w"] / 2 == pytest.approx(0.1 + 0.3)
