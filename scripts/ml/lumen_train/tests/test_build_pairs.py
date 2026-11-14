"""Building real pairs: a prepared photo, its Lightroom sidecar, and the converted target."""

import json
from pathlib import Path

from lumen_train.retouch import build_pairs, params


def prepared(root: Path, name: str, source: Path, aspect: float = 1.5) -> Path:
    d = root / name
    d.mkdir(parents=True)
    (d / "input.u16").write_bytes(b"")
    (d / "meta.json").write_text(json.dumps({"source": str(source), "aspect": aspect, "width": 256, "height": 171}), encoding="utf-8")
    return d


def test_sidecars_are_found_both_ways(tmp_path: Path):
    a = tmp_path / "a.CR3"
    a.write_bytes(b"")
    (tmp_path / "a.xmp").write_text("x", encoding="utf-8")
    b = tmp_path / "b.jpg"
    b.write_bytes(b"")
    (tmp_path / "b.jpg.xmp").write_text("x", encoding="utf-8")
    c = tmp_path / "c.jpg"
    c.write_bytes(b"")
    assert build_pairs.sidecar(a) == tmp_path / "a.xmp"
    assert build_pairs.sidecar(b) == tmp_path / "b.jpg.xmp"
    assert build_pairs.sidecar(c) is None


def converted(exposure: float, crop: dict | None = None, straighten: float = 0.0) -> dict:
    return {
        "develop": {"basic": {"exposure": exposure}},
        "geometry": {"crop": crop, "straighten": straighten},
        "panels": ["basic"],
        "skipped": ["CameraProfile"],
    }


def test_a_pair_gets_its_target_vector_and_render(tmp_path: Path):
    photos = tmp_path / "photos"
    photos.mkdir()
    raw = photos / "a.dng"
    raw.write_bytes(b"")
    (photos / "a.xmp").write_text("x", encoding="utf-8")
    lone = photos / "b.dng"
    lone.write_bytes(b"")
    out = tmp_path / "prepared"
    pa = prepared(out, "a-1", raw)
    pb = prepared(out, "b-1", lone)
    rendered = []

    def render_target(photo: Path, develop: dict, folder: Path) -> None:
        rendered.append((photo, develop))
        (folder / "target.u16").write_bytes(b"t")

    crop = {"x": 0.1, "y": 0.05, "w": 0.8, "h": 0.9}
    made = build_pairs.build(out, lambda xmp: converted(1.0, crop, 2.0), render_target)
    assert made == 1
    t = json.loads((pa / "target.json").read_text(encoding="utf-8"))
    assert t["skipped"] == ["CameraProfile"]
    assert abs(t["v"][params.index("exposure")] - (1.0 + 3.0) / 6.0) < 1e-6
    assert t["v"][params.index("crop_on")] == 1.0
    assert abs(t["v"][params.index("straighten")] - (2.0 + 10.0) / 20.0) < 1e-6
    assert [p for p, _ in rendered] == [raw]
    assert rendered[0][1]["basic"]["exposure"] == 1.0
    assert "crop" not in rendered[0][1] and "straighten" not in rendered[0][1], "the target is rendered without geometry"
    assert not (pb / "target.json").exists(), "no sidecar, no pair"


def test_the_target_shows_only_what_trainings_render_scores():
    lin = {"shapes": [{"kind": "linear", "add": True, "from": [0.5, 0.0], "to": [0.5, 0.5]}], "adjust": {"exposure": -0.5, "clarity": 0.4}}
    develop = {
        "basic": {"exposure": 0.3, "clarity": 0.5, "texture": 0.2, "dehaze": 0.3},
        "detail": {"sharpen": {"amount": 0.8, "radius": 1.2}, "denoise": {"strength": 0.5, "model": "x"}},
        "effects": {"grain": {"amount": 0.4, "size": 0.3}, "vignette": {"amount": -0.2}},
        "masks": [
            {"shapes": [{"kind": "brush", "add": True, "radius": 0.05, "feather": 0.5, "flow": 1.0, "points": [[1, 1]]}], "adjust": {"exposure": 1.0}},
            lin,
            {"shapes": [{"kind": "linear", "add": True, "from": [0.0, 0.5], "to": [0.5, 0.5]}], "adjust": {"exposure": 0.5}},
        ],
    }
    g = build_pairs.global_look(develop)
    assert g["basic"] == {"exposure": 0.3, "clarity": 0.0, "texture": 0.0, "dehaze": 0.0}
    assert g["detail"]["sharpen"] == {"amount": 0.0, "radius": 1.2} and g["detail"]["denoise"] is None
    assert g["effects"]["grain"]["amount"] == 0.0 and g["effects"]["vignette"] == {"amount": -0.2}
    assert len(g["masks"]) == 1, "the brush and the second gradient have no slot"
    assert g["masks"][0]["shapes"] == lin["shapes"] and g["masks"][0]["adjust"]["exposure"] == -0.5
    assert g["masks"][0]["adjust"]["clarity"] == 0.0
    assert develop["basic"]["clarity"] == 0.5, "the converted develop is left as it was"


def test_a_failed_conversion_skips_the_photo(tmp_path: Path):
    photos = tmp_path / "photos"
    photos.mkdir()
    raw = photos / "a.dng"
    raw.write_bytes(b"")
    (photos / "a.xmp").write_text("x", encoding="utf-8")
    out = tmp_path / "prepared"
    pa = prepared(out, "a-1", raw)

    def boom(_):
        raise RuntimeError("bad xmp")

    assert build_pairs.build(out, boom, lambda *a: None) == 0
    assert not (pa / "target.json").exists()
