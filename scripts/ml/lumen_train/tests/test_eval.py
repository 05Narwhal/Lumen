"""Evaluation: the report's numbers, the contact sheet, and the ship bar."""

import json
from pathlib import Path

import numpy as np
import torch

from lumen_train.common import ckpt
from lumen_train.retouch import eval as ev
from lumen_train.retouch import params
from lumen_train.retouch.dataset import PreparedDataset
from lumen_train.retouch.model import RetouchNet
from lumen_train.retouch.prepared import decode_input
from lumen_train.tests.test_dataset import write_sample


def setup(tmp_path: Path) -> tuple[Path, Path]:
    root = tmp_path / "data"
    for i in range(30):
        write_sample(root / "synthetic-test" / f"p{i:02d}", 96, 64, 0.04 + 0.02 * i)
    for stem, ext in [("same", "cr3"), ("same", "jpg")]:
        d = root / "julian-catalog" / f"{stem}-{ext}"
        write_sample(d, 96, 64, 0.2)
        meta = json.loads((d / "meta.json").read_text(encoding="utf-8"))
        meta["source"] = f"C:/photos/{stem}.{ext}"
        meta["auto"] = {"exposure": 0.3, "contrast": 0.1}
        (d / "meta.json").write_text(json.dumps(meta), encoding="utf-8")
        v = params.neutral_vector()
        v[params.index("exposure")] = 0.6
        (d / "target.json").write_text(json.dumps({"v": v.tolist()}), encoding="utf-8")
    path = tmp_path / "m.pt"
    torch.manual_seed(0)
    ckpt.save(path, RetouchNet(), None, 0, 1.0, {"sources": ["synthetic-test"], "ship": False})
    return root, path


def fake_auto(files: list[Path]) -> list[dict]:
    return [{"exposure": 0.2} for _ in files]


def test_the_report_has_every_measure(tmp_path: Path):
    root, path = setup(tmp_path)
    report = ev.run(path, root, ["synthetic-test", "julian-catalog"], tmp_path / "eval", sheet_rows=8, auto_basic=fake_auto)
    for k in ["de_retouch", "de_untouched", "de_auto", "param_error", "crop_iou", "angle_error", "crop_presence_accuracy", "raw_jpeg_gap", "count"]:
        assert k in report, k
    assert report["count"] > 0
    assert set(report["param_error"]) >= {"basic", "geometry"}
    assert report["de_auto"] is not None, "pairs with Auto's measurement are compared with it"
    assert report["auto_count"] == report["count"], "every photo has Auto beside it"
    assert report["raw_jpeg_gap"] is not None and report["raw_jpeg_pairs"] == 1, "the RAW and its JPEG are compared"
    assert json.loads((tmp_path / "eval" / "report.json").read_text(encoding="utf-8"))["count"] == report["count"]
    md = (tmp_path / "eval" / "report.md").read_text(encoding="utf-8")
    assert "ΔE" in md and "Auto" in md


def test_a_synthetic_pairs_auto_is_measured_on_the_photo_retouch_saw(tmp_path: Path):
    root, path = setup(tmp_path)
    seen = []

    def auto(files: list[Path]) -> list[dict]:
        for f in files:
            lin, _, w, h = decode_input(np.fromfile(f, dtype="<u2"))
            seen.append((lin, w, h))
        return [{"exposure": 0.1} for _ in files]

    ev.run(path, root, ["synthetic-test"], tmp_path / "eval", sheet_rows=0, auto_basic=auto)
    ds = PreparedDataset(root, ["synthetic-test"], split="val", augment=False)
    assert len(seen) == len(ds) > 0, "the photos it trained on are scored on their validation split"
    item = ds[0]
    lin, w, h = seen[0]
    assert (w, h) == (96, 64)
    assert torch.allclose(lin[:, :h, :w], item["aligned"][:, :h, :w].clamp(0, 4 - 1 / 64), rtol=3e-3, atol=3e-4), "the degraded photo, not the clean one"


def test_the_sheet_has_a_row_per_photo_and_saves_marks(tmp_path: Path):
    root, path = setup(tmp_path)
    ev.run(path, root, ["synthetic-test"], tmp_path / "eval", sheet_rows=5, auto_basic=None)
    html = (tmp_path / "eval" / "sheet.html").read_text(encoding="utf-8")
    assert html.count('class="row"') == 5
    for word in ["better", "same", "worse", "Download marks", "localStorage"]:
        assert word in html


def test_the_ship_bar():
    good = {"de_retouch": 4.0, "de_auto": 5.0, "de_untouched": 9.0}
    marks = {"a": "better", "b": "same", "c": "better", "d": "better", "e": "worse"}
    assert ev.passes(good, marks)
    assert not ev.passes({**good, "de_retouch": 5.5}, marks), "worse than Auto"
    assert not ev.passes(good, {**marks, "f": "worse", "g": "worse"}), "under 80 % better or same"
    assert not ev.passes(good, {}), "no marks, no ship"
    assert ev.passes({"de_retouch": 4.0, "de_auto": None, "de_untouched": 9.0}, marks), "without Auto, beat untouched"


def test_scoring_the_test_split_touches_only_test_items(tmp_path: Path):
    from lumen_train.data import splits

    for i in range(300):
        write_sample(tmp_path / "synthetic-test" / f"t{i:03d}", 32, 24, 0.1)
    ((_, ds),) = ev._datasets(tmp_path, ["synthetic-test"], ["synthetic-test"], split="test")
    assert 0 < len(ds) < 30
    assert all(splits.split_of(splits.stem(f.name), splits.Shares()) == "test" for f in ds.samples)


def _his_pair(root: Path, name: str, garbage_from: int) -> None:
    """An aligned pair whose target is the photo itself inside his crop (columns before
    `garbage_from`) and white outside it, where `target_mask.u8` says it is not his."""
    from lumen_train.retouch.prepared import EDGE, encode_input

    d = root / "julian-catalog" / name
    write_sample(d, 96, 64, 0.2)
    lin, _, w, h = decode_input(np.fromfile(d / "input.u16", dtype="<u2"))
    tgt = lin[:, :h, :w].clone()
    tgt[:, :, garbage_from:] = 1.0
    (encode_input(tgt, w, h).clamp(0, 1) * 65535).round().numpy().astype("<u2").tofile(d / "target.u16")
    m = np.zeros((EDGE, EDGE), np.uint8)
    m[:h, :garbage_from] = 1
    m.tofile(d / "target_mask.u8")
    known = ["straighten", "crop_on", "crop_x", "crop_y", "crop_w", "crop_h", "crop_ratio"]
    (d / "target.json").write_text(json.dumps({"v": params.neutral_vector().tolist(), "known": known}), encoding="utf-8")


def test_de_counts_only_the_pixels_his_edit_covers(tmp_path: Path):
    root, path = setup(tmp_path)
    for f in (root / "julian-catalog").iterdir():
        import shutil
        shutil.rmtree(f)
    _his_pair(root, "his-0", 60)
    report = ev.run(path, root, ["julian-catalog"], tmp_path / "eval", sheet_rows=0, auto_basic=None)
    assert report["count"] == 1
    assert report["de_untouched"] < 0.5, "white outside his crop would cost tens of ΔE"


def test_the_report_has_a_line_per_source(tmp_path: Path):
    root, path = setup(tmp_path)
    report = ev.run(path, root, ["synthetic-test", "julian-catalog"], tmp_path / "eval", sheet_rows=0, auto_basic=fake_auto)
    per = report["by_source"]
    assert set(per) == {"synthetic-test", "julian-catalog"}
    assert per["julian-catalog"]["count"] == 2 and per["synthetic-test"]["count"] == report["count"] - 2
    for k in ["de_retouch", "de_auto", "de_untouched", "crop_iou"]:
        assert k in per["synthetic-test"]
    md = (tmp_path / "eval" / "report.md").read_text(encoding="utf-8")
    assert "julian-catalog" in md and "synthetic-test" in md


def test_his_own_photos_are_all_on_the_sheet_first_with_his_edit(tmp_path: Path):
    root, path = setup(tmp_path)
    ev.run(path, root, ["synthetic-test", "julian-catalog"], tmp_path / "eval", sheet_rows=1, auto_basic=None, sheet_all=["julian-catalog"])
    html = (tmp_path / "eval" / "sheet.html").read_text(encoding="utf-8")
    assert html.count('class="row"') == 3, "both of his photos, plus the one row of the others"
    assert html.index("julian-catalog/") < html.index("synthetic-test/")
    assert html.count("<figcaption>edit</figcaption>") == 2 and html.count("<figcaption>target</figcaption>") == 1


def test_a_pair_without_autos_measurement_is_measured_now(tmp_path: Path):
    root, path = setup(tmp_path)
    _his_pair(root, "his-0", 60)
    report = ev.run(path, root, ["julian-catalog"], tmp_path / "eval", sheet_rows=0, auto_basic=fake_auto)
    assert report["by_source"]["julian-catalog"]["auto_count"] == 3
