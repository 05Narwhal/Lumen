"""Synthetic damage rendered in batches on the device (Round 12-1 Task 2): the same pairs as one
photo at a time, the same draws for the same seed, and the RAM cache."""

from dataclasses import astuple
from pathlib import Path

import pytest
import torch
import torch.nn.functional as F

from lumen_train.retouch import dataset as dataset_mod, params, render, synth
from lumen_train.retouch.dataset import PreparedDataset, collate
from lumen_train.retouch.prepared import EDGE, decode, encode_input
from lumen_train.tests.test_dataset import write_sample


def _flat(d: synth.Degradation) -> list[float]:
    return [float(x) for x in astuple(d)[:-1]] + list(d.pad)


# ---- The per-photo path `apply_batch` replaced (Round 11B), kept here as the reference -----


def _sample(img, gx, gy):
    grid = torch.stack([gx, gy], dim=-1).unsqueeze(0).to(img.dtype)
    return F.grid_sample(img.unsqueeze(0), grid, mode="bilinear", padding_mode="reflection", align_corners=False)[0]


def _loosen(img, pad):
    _, h, w = img.shape
    l, t, r, b = pad
    w2, h2 = synth.fit(w * (1 + l + r), h * (1 + t + b))
    xs = (torch.arange(w2, dtype=torch.float64) + 0.5) / w2 * (1 + l + r) - l
    ys = (torch.arange(h2, dtype=torch.float64) + 0.5) / h2 * (1 + t + b) - t
    gy, gx = torch.meshgrid(ys * 2 - 1, xs * 2 - 1, indexing="ij")
    return _sample(img, gx, gy)


def _tilt(img, deg):
    _, h, w = img.shape
    rad = torch.deg2rad(torch.tensor(deg, dtype=torch.float64))
    s, c = torch.sin(rad), torch.cos(rad)
    z = c.abs() + s.abs() * max(w / h, h / w)
    cx, cy = w / 2, h / 2
    y, x = torch.meshgrid(torch.arange(h, dtype=torch.float64), torch.arange(w, dtype=torch.float64), indexing="ij")
    dx, dy = (x + 0.5 - cx) / z, (y + 0.5 - cy) / z
    sx = c * dx + s * dy + cx - 0.5
    sy = -s * dx + c * dy + cy - 0.5
    return _sample(img, (sx + 0.5) / w * 2 - 1, (sy + 0.5) / h * 2 - 1)


def _canvas(img):
    out = torch.zeros(3, EDGE, EDGE, dtype=img.dtype)
    out[:, : img.shape[1], : img.shape[2]] = img
    return out


def _reference_make_pair(final, d):
    _, h, w = final.shape
    dev = render.from_develop_json(d.develop_json(), dtype=final.dtype)
    aligned = render.render(final.unsqueeze(0), dev)[0]
    shaped, crop_on, box, aspect = final, 0.0, (0.0, 0.0, 1.0, 1.0), w / h
    if any(p > 0 for p in d.pad):
        l, t, r, b = d.pad
        shaped = _loosen(shaped, d.pad)
        crop_on = 1.0
        box = (l / (1 + l + r), t / (1 + t + b), 1 / (1 + l + r), 1 / (1 + t + b))
        aspect = (w * (1 + l + r)) / (h * (1 + t + b))
    if d.tilt:
        shaped = _tilt(shaped, d.tilt)
    degraded = render.render(shaped.unsqueeze(0), dev)[0]
    mask = torch.zeros(1, EDGE, EDGE, dtype=final.dtype)
    mask[:, :h, :w] = 1.0
    return {
        "input": encode_input(degraded, degraded.shape[2], degraded.shape[1]), "aligned": _canvas(aligned), "target": _canvas(final), "mask": mask,
        "straighten": -d.tilt if d.tilt else 0.0, "crop_on": crop_on, "crop_box": box, "aspect": aspect,
    }


@pytest.mark.parametrize("seed", [7, 11, 23])
def test_batch_equals_per_sample(seed):
    gen = torch.Generator().manual_seed(seed)
    finals, sizes, degs, singles = [], [], [], []
    for k, (w, h) in enumerate([(256, 170), (170, 256), (256, 256), (120, 90)]):  # landscape, portrait, square, small
        img = torch.rand(3, h, w, generator=gen) * 0.8
        d = synth.Degradation.sample(gen)
        if k == 0:
            d.pad, d.tilt = (0.1, 0.0, 0.05, 0.2), 0.0  # every geometry appears in one batch
        if k == 1:
            d.pad, d.tilt = (0.0, 0.0, 0.0, 0.0), 4.0
        singles.append(_reference_make_pair(img, d))
        canvas = torch.zeros(3, EDGE, EDGE)
        canvas[:, :h, :w] = img
        finals.append(canvas)
        sizes.append(torch.tensor([w, h]))
        degs.append(d.to_tensor())
    out = synth.apply_batch(torch.stack(finals), torch.stack(sizes), torch.stack(degs))
    for i, s in enumerate(singles):
        m = s["mask"].bool().expand(3, -1, -1)
        assert torch.allclose(out["aligned"][i][m], s["aligned"][m], atol=1e-4)
        assert torch.allclose(out["aligned"][i][~m], torch.zeros(()), atol=0)  # padding never leaks
        assert torch.allclose(out["target"][i], s["target"], atol=1e-6)
        assert torch.allclose(out["input"][i], s["input"], atol=2e-3)
        assert torch.equal(out["mask"][i], s["mask"])
        assert abs(float(out["crop_on"][i]) - s["crop_on"]) < 1e-6
        assert torch.allclose(out["crop_box"][i], torch.tensor(s["crop_box"], dtype=torch.float32), atol=1e-6)
        assert abs(float(out["aspect"][i]) - s["aspect"]) < 1e-5
        assert abs(float(out["straighten"][i]) - s["straighten"]) < 1e-6


def test_tensor_round_trip():
    d = synth.Degradation.sample(torch.Generator().manual_seed(3))
    d.hue_band, d.pad = 5, (0.1, 0.0, 0.2, 0.05)
    back = synth.Degradation.from_tensor(d.to_tensor())
    assert back.hue_band == 5 and isinstance(back.hue_band, int)
    assert _flat(back) == pytest.approx(_flat(d), rel=1e-6, abs=1e-6)


@pytest.fixture
def root(tmp_path: Path) -> Path:
    for i in range(6):
        write_sample(tmp_path / "data" / "synthetic-test" / f"p{i:02d}", 96 if i % 2 else 64, 64 if i % 2 else 96, 0.05 + 0.03 * i)
    return tmp_path / "data"


def test_a_lazy_item_draws_the_same_damage_as_before(root: Path):
    """Review focus 1: the seeding and the draw order of `Degradation.sample` are untouched."""
    ds = PreparedDataset(root, ["synthetic-test"], split="train", seed=3, augment=False, lazy=True)  # train: seeded by epoch
    ds.set_epoch(2)
    for i in range(len(ds)):
        gen = torch.Generator().manual_seed(3 * 1_000_003 + 2 * 10_007 + i)
        assert torch.allclose(ds[i]["deg"], synth.Degradation.sample(gen).to_tensor())


def test_lazy_items_render_like_eager_ones(root: Path):
    eager = PreparedDataset(root, ["synthetic-test"], split="all", seed=1, augment=True)
    lazy = PreparedDataset(root, ["synthetic-test"], split="all", seed=1, augment=True, lazy=True)
    got = synth.realise(collate([lazy[i] for i in range(len(lazy))]))
    want = collate([eager[i] for i in range(len(eager))])
    for k in ["input", "aligned", "target", "mask", "v_target", "v_known", "aspect"]:
        assert torch.allclose(got[k], want[k], atol=2e-3), k


def test_a_mixed_batch_collates_and_keeps_the_pairs():
    pair = {"input": torch.ones(4, 8, 8), "aligned": torch.ones(3, 8, 8), "kind": 1}
    syn = {"final": torch.ones(3, 8, 8), "deg": torch.ones(synth.DEG_LEN), "kind": 0}
    b = collate([pair, syn])
    assert b["input"].shape == (2, 4, 8, 8) and b["final"].shape == (2, 3, 8, 8)
    assert b["input"][1].abs().sum() == 0 and b["final"][0].abs().sum() == 0


def test_the_cache_reads_each_file_once(root: Path, monkeypatch):
    reads = []
    real = dataset_mod.read_raw
    monkeypatch.setattr(dataset_mod, "read_raw", lambda f: reads.append(f) or real(f))
    ds = PreparedDataset(root, ["synthetic-test"], split="all", lazy=True, cache=True)
    for epoch in range(2):
        ds.set_epoch(epoch)
        first = [ds[i]["final"] for i in range(len(ds))]
    assert len(reads) == len(ds)
    assert first[0].dtype == torch.float32


# ---- "Already fine" samples (Round 12-3 Task 1) ---------------------------------------------


def test_already_fine_items_ask_for_nothing(root: Path):
    """Review focus 5: no damage, the target is the input, neutral sliders known, no geometry."""
    ds = PreparedDataset(root, ["synthetic-test"], split="all", seed=4, augment=True, lazy=True, identity_p=1.0)
    items = [ds[i] for i in range(len(ds))]
    none = synth.Degradation.identity().to_tensor()
    first_mask = params.index("lin_on")
    neutral = params.neutral_vector()
    for it in items:
        assert torch.equal(it["deg"], none)
        assert torch.all(it["v_known"][:first_mask] == 1.0)
        assert torch.allclose(it["v_target"][:first_mask], neutral[:first_mask])
        assert it["v_known"][params.index("lin_on")] == 1.0 and it["v_target"][params.index("lin_on")] == 0.0
        assert it["v_known"][params.index("rad_on")] == 1.0 and it["v_target"][params.index("rad_on")] == 0.0
    out = synth.realise(collate(items))
    m = out["mask"].bool().expand_as(out["target"])
    got = decode(out["input"][:, :3])
    assert (got[m] - out["target"][m]).abs().max() < 2e-3
    assert torch.all(out["input"][:, 3:4] == out["mask"])
    g = synth.apply_batch(out["final"], out["size"], out["deg"])
    assert torch.all(g["crop_on"] == 0) and torch.all(g["straighten"] == 0)
    assert torch.equal(g["input"], out["input"])


def test_without_already_fine_items_the_draws_are_unchanged(root: Path):
    """`identity_p = 0` draws nothing more: every item is what 12-1 drew."""
    before = PreparedDataset(root, ["synthetic-test"], split="all", seed=5, augment=True, lazy=True)
    after = PreparedDataset(root, ["synthetic-test"], split="all", seed=5, augment=True, lazy=True, identity_p=0.0)
    for i in range(len(before)):
        a, b = before[i], after[i]
        for k in a:
            assert torch.equal(torch.as_tensor(a[k]), torch.as_tensor(b[k])), k


def test_some_items_are_already_fine_and_the_rest_keep_their_damage(root: Path):
    """The coin is drawn after `Degradation.sample`, so a damaged item's damage is 12-1's."""
    ds = PreparedDataset(root, ["synthetic-test"], split="all", seed=6, augment=False, lazy=True, identity_p=0.5)
    ref = PreparedDataset(root, ["synthetic-test"], split="all", seed=6, augment=False, lazy=True)
    none = synth.Degradation.identity().to_tensor()
    fine = [torch.equal(ds[i]["deg"], none) for i in range(len(ds))]
    for i in range(len(ds)):
        if not fine[i]:
            assert torch.equal(ds[i]["deg"], ref[i]["deg"])
