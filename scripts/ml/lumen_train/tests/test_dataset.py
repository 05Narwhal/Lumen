"""The dataset over prepared folders: splits, determinism, flips and the licence gate."""

import json
from pathlib import Path

import numpy as np
import pytest
import torch

from lumen_train.common.licence import LicenceError
from lumen_train.retouch import params
from lumen_train.retouch.dataset import PreparedDataset
from lumen_train.retouch.prepared import EDGE, encode_input


def write_sample(folder: Path, w: int, h: int, level: float) -> None:
    folder.mkdir(parents=True)
    y, x = torch.meshgrid(torch.linspace(0, 1, h), torch.linspace(0, 1, w), indexing="ij")
    lin = torch.stack([level * (0.5 + x), level * (0.5 + y), torch.full_like(x, level)])
    packed = encode_input(lin, w, h)
    raw = (packed.clamp(0, 1) * 65535).round().numpy().astype("<u2")
    raw.tofile(folder / "input.u16")
    (folder / "meta.json").write_text(json.dumps({"width": w, "height": h, "aspect": w / h, "edge": EDGE}), encoding="utf-8")


@pytest.fixture
def root(tmp_path: Path) -> Path:
    for i in range(40):
        write_sample(tmp_path / "synthetic-test" / f"p{i:02d}", 256, 171 if i % 2 else 256, 0.05 + 0.01 * i)
    pair = tmp_path / "julian-catalog" / "x00"
    write_sample(pair, 200, 256, 0.2)
    v = params.neutral_vector()
    v[params.index("exposure")] = 0.7
    v[params.index("lin_on")] = 1.0
    v[params.index("lin_fx")] = 0.2
    (pair / "target.json").write_text(json.dumps({"v": v.tolist()}), encoding="utf-8")
    return tmp_path


def test_train_and_validation_split_every_sample_once(root: Path):
    train = PreparedDataset(root, ["synthetic-test"], split="train")
    val = PreparedDataset(root, ["synthetic-test"], split="val")
    test = PreparedDataset(root, ["synthetic-test"], split="test")
    assert len(train) + len(val) + len(test) == 40 and len(val) > 0
    names = [{s.name for s in d.samples} for d in (train, val, test)]
    assert not (names[0] & names[1]) and not (names[0] & names[2]) and not (names[1] & names[2])


def test_items_have_their_shapes(root: Path):
    item = PreparedDataset(root, ["synthetic-test"], split="train")[0]
    assert item["input"].shape == (4, EDGE, EDGE)
    assert item["aligned"].shape == item["target"].shape == (3, EDGE, EDGE)
    assert item["mask"].shape == (1, EDGE, EDGE)
    assert item["v_target"].shape == item["v_known"].shape == (len(params.PARAMS),)
    assert item["clip"].shape == (512,) and item["clip_flag"].shape == (1,)


def test_the_same_epoch_gives_the_same_item_and_the_next_another(root: Path):
    ds = PreparedDataset(root, ["synthetic-test"], split="train", seed=3)
    a = ds[5]["input"]
    assert torch.equal(a, ds[5]["input"])
    ds.set_epoch(1)
    assert not torch.equal(a, ds[5]["input"])


def test_validation_items_never_change(root: Path):
    ds = PreparedDataset(root, ["synthetic-test"], split="val", seed=3)
    a = ds[0]["input"]
    ds.set_epoch(7)
    assert torch.equal(a, ds[0]["input"])


def test_a_pair_trains_on_its_slider_values(root: Path):
    ds = PreparedDataset(root, ["julian-catalog"], split="all", augment=False)
    item = ds[0]
    assert item["kind"] == 1
    assert item["v_known"].min() == 1.0
    assert abs(item["v_target"][params.index("exposure")].item() - 0.7) < 1e-6


def test_a_flip_mirrors_the_photo_and_its_geometry(root: Path):
    ds = PreparedDataset(root, ["julian-catalog"], split="all", augment=False)
    plain = ds[0]
    flipped = ds.flip(plain)
    w = 200
    assert torch.equal(flipped["input"][:, :, :w], plain["input"][:, :, :w].flip(-1))
    assert torch.equal(flipped["input"][:, :, w:], plain["input"][:, :, w:]), "the padding stays where it is"
    fx = flipped["v_target"][params.index("lin_fx")].item()
    assert abs(fx - 0.8) < 1e-6
    s = params.index("straighten")
    assert abs((flipped["v_target"][s] - 0.5) + (plain["v_target"][s] - 0.5)) < 1e-6


def test_shipping_refuses_a_non_commercial_source(root: Path):
    with pytest.raises(LicenceError):
        PreparedDataset(root, ["synthetic-test", "julian-catalog"], split="train", ship=True)
    PreparedDataset(root, ["synthetic-test"], split="train", ship=True)


def test_a_raw_and_its_jpeg_land_on_the_same_side(tmp_path: Path):
    for i in range(60):
        for ext in ("cr3", "jpg"):
            write_sample(tmp_path / "synthetic-test" / f"IMG_{i:04d}-{hash((i, ext)) & (2**64 - 1):016x}", 32, 32, 0.1)
    val = PreparedDataset(tmp_path, ["synthetic-test"], split="val")
    train = PreparedDataset(tmp_path, ["synthetic-test"], split="train")
    scenes = lambda ds: {s.name.split("-")[0] for s in ds.samples}
    assert scenes(val) and not scenes(val) & scenes(train)
    assert len(val) % 2 == 0


def test_a_pairs_sources_unfinished_photos_are_left_out(root: Path):
    write_sample(root / "julian-catalog" / "unedited", 64, 64, 0.2)
    ds = PreparedDataset(root, ["julian-catalog"], split="all", augment=False)
    assert [s.name for s in ds.samples] == ["x00"], "an original without a photographer's edit is not a target"


def test_a_synthetic_crop_is_a_free_crop(root: Path):
    ds = PreparedDataset(root, ["synthetic-test"], split="train", augment=False)
    r = params.index("crop_ratio")
    seen = 0
    for i in range(len(ds)):
        item = ds[i]
        if item["v_known"][params.index("crop_w")] > 0:
            seen += 1
            assert item["v_known"][r] == 1.0
            assert params.decode_crop(item["v_target"].unsqueeze(0), item["aspect"].unsqueeze(0))["box"][0, 3] == pytest.approx(params.value(item["v_target"].unsqueeze(0), "crop_h")[0].item(), abs=1e-5)
    assert seen > 0


def _aligned_pair(root: Path, mask_cols: int | None) -> Path:
    """A `raw+jpeg` pair as `build_pairs --align` writes it: geometry known, his edit valid only
    in the first `mask_cols` columns (None: no `target_mask.u8`)."""
    folder = root / "julian-catalog" / "n00"
    write_sample(folder, 200, 150, 0.2)
    v = params.neutral_vector()
    v[params.index("crop_on")] = 1.0
    v[params.index("exposure")] = 0.9  # not known: must not be learned from a JPEG's look
    known = ["straighten", "crop_on", "crop_x", "crop_y", "crop_w", "crop_h", "crop_ratio"]
    (folder / "target.json").write_text(json.dumps({"v": v.tolist(), "known": known}), encoding="utf-8")
    lin = torch.full((3, 150, 200), 0.3)
    (encode_input(lin, 200, 150).clamp(0, 1) * 65535).round().numpy().astype("<u2").tofile(folder / "target.u16")
    if mask_cols is not None:
        m = np.zeros((EDGE, EDGE), np.uint8)
        m[:150, :mask_cols] = 1
        m.tofile(folder / "target_mask.u8")
    return folder


def test_a_pair_with_known_lists_only_those_slots(tmp_path: Path):
    _aligned_pair(tmp_path, None)
    item = PreparedDataset(tmp_path, ["julian-catalog"], split="all", augment=False)[0]
    known = {params.NAMES[i] for i in torch.nonzero(item["v_known"]).flatten().tolist()}
    assert known == {"straighten", "crop_on", "crop_x", "crop_y", "crop_w", "crop_h", "crop_ratio"}
    assert item["target_void"].sum() == 0  # no target mask: every photo pixel counts


def test_a_pair_without_known_keeps_every_slot_known(root: Path):
    item = PreparedDataset(root, ["julian-catalog"], split="all", augment=False)[0]
    assert bool((item["v_known"] == 1).all())


def test_the_target_mask_leaves_pixels_out_of_the_score_and_flips(tmp_path: Path):
    _aligned_pair(tmp_path, 120)
    ds = PreparedDataset(tmp_path, ["julian-catalog"], split="all", augment=False)
    item = ds[0]
    scored = item["mask"] * (1 - item["target_void"])
    assert scored[0, :150, :120].min() == 1 and scored[0, :, 120:].max() == 0
    # The photo's own mask is untouched: it still gives the photo's size.
    assert int(item["mask"][0, 0].sum()) == 200
    flipped = ds.flip(item)
    void = flipped["target_void"][0]
    assert void[:150, :80].max() == 1 and void[:150, 80:200].max() == 0 and void[:, 200:].max() == 0
