"""Real tiers (Round 14-4a): int8 / fp16 `low`, fp32 `high`, `ultra` = high + 5-pass TTA, each
scored on validation photos the way eval scores them."""

from pathlib import Path

import numpy as np
import onnx
import onnxruntime as ort
import pytest
import torch

from lumen_train.common import ckpt
from lumen_train.retouch import eval as ev
from lumen_train.retouch import export, params, tiers
from lumen_train.retouch.model import RetouchNet


def _net(path: Path) -> RetouchNet:
    net = RetouchNet()
    ckpt.load(path, net)
    return net.eval()


def _fp32(path: Path, out: Path) -> Path:
    export._onnx(ckpt.read(path), out)
    return out


def test_val_items_are_the_trained_sources_validation_photos(scored_run):
    root, _ = scored_run
    items = tiers.val_items(root, ["synthetic-test"], limit=1000)
    assert 0 < len(items) < 30, "the validation split, not every photo"
    assert len(tiers.val_items(root, ["synthetic-test"], limit=3)) == 3
    assert items[0]["input"].shape == (1, 4, 256, 256)
    assert tiers.val_items(root, ["not-on-disk"], limit=10) == []


def test_mean_de_is_evals_de(scored_run, tmp_path: Path):
    root, path = scored_run
    net = _net(path)
    items = tiers.val_items(root, ["synthetic-test"], limit=1000)
    got = tiers.mean_de(lambda b: net(b["input"], b["clip"], b["clip_flag"]), items)
    report = ev.run(path, root, ["synthetic-test"], tmp_path / "eval", sheet_rows=0, auto_basic=None)
    assert got == pytest.approx(report["de_retouch"], abs=1e-4)
    assert tiers.mean_de(lambda b: None, []) is None


def test_spatial_sliders_are_real_names():
    for name in tiers.SPATIAL:
        params.index(name)
    assert "exposure" not in tiers.SPATIAL and "crop_ratio" in tiers.SPATIAL


def test_tta_averages_tone_and_takes_geometry_from_the_full_pass(scored_run):
    root, _ = scored_run
    item = tiers.val_items(root, ["synthetic-test"], limit=1)[0]
    seen = []

    def fake(b):
        seen.append(b["input"])
        n = b["input"].shape[0]
        return torch.arange(n, dtype=torch.float32).unsqueeze(1).repeat(1, len(params.PARAMS))

    v = tiers.tta_predict(fake)(item)
    assert v.shape == (1, len(params.PARAMS))
    for j, p in enumerate(params.PARAMS):
        assert v[0, j].item() == (0.0 if p["name"] in tiers.SPATIAL else 2.0), p["name"]
    (x,) = seen
    assert x.shape == (5, 4, 256, 256)
    full = item["input"][0]
    w, h = int(full[3, 0].sum()), int(full[3, :, 0].sum())
    assert torch.equal(x[0], full)
    assert torch.equal(x[1, :, :h, :w], full[:, :h, :w].flip(-1)) and torch.equal(x[1, 3], full[3])
    long_x = w >= h
    c = round(0.9 * (w if long_x else h))
    for k, off in zip((2, 3, 4), (0, ((w if long_x else h) - c) // 2, (w if long_x else h) - c)):
        m = x[k, 3]
        assert (int(m[0].sum()), int(m[:, 0].sum())) == ((c, h) if long_x else (w, c))
        want = full[:, :h, off : off + c] if long_x else full[:, off : off + c, :w]
        got = x[k, :, :h, :c] if long_x else x[k, :, :c, :w]
        assert torch.equal(got, want)


def test_int8_is_qdq_and_close_to_fp32(scored_run, tmp_path: Path):
    root, path = scored_run
    fp32 = _fp32(path, tmp_path / "high.onnx")
    items = tiers.val_items(root, ["synthetic-test"], limit=8)
    low = tmp_path / "low.onnx"
    tiers.quantize_int8(fp32, low, [b["input"].numpy() for b in items])
    assert any(n.op_type == "QuantizeLinear" for n in onnx.load(str(low)).graph.node)
    a, b = tiers.onnx_predict(fp32), tiers.onnx_predict(low)
    for item in items[:3]:
        assert (a(item) - b(item)).abs().max().item() < 0.05
    assert low.stat().st_size < fp32.stat().st_size


def test_fp16_keeps_float32_io(scored_run, tmp_path: Path):
    root, path = scored_run
    fp32 = _fp32(path, tmp_path / "high.onnx")
    half = tmp_path / "half.onnx"
    tiers.to_fp16(fp32, half)
    s = ort.InferenceSession(str(half), providers=["CPUExecutionProvider"])
    assert {i.type for i in s.get_inputs()} == {"tensor(float)"} and s.get_outputs()[0].type == "tensor(float)"
    item = tiers.val_items(root, ["synthetic-test"], limit=1)[0]
    assert np.isfinite(tiers.onnx_predict(half)(item).numpy()).all()
