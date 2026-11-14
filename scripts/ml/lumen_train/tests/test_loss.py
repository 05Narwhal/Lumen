"""The OKLab training metric (Round 12-1): cheap, masked, float32 under autocast, and it trains."""

import pytest
import torch

from lumen_train.retouch import params, render, train
from lumen_train.retouch.model import RetouchNet


def test_oklab_distance_is_zero_for_equal_images_and_grows_with_exposure():
    gen = torch.Generator().manual_seed(0)
    img = torch.rand(2, 3, 16, 16, generator=gen) * 0.8
    mask = torch.ones(2, 1, 16, 16)
    assert torch.allclose(render.oklab_distance(img, img, mask), torch.zeros(2), atol=1e-4)
    ds = [render.oklab_distance(img * 2.0 ** e, img, mask).mean().item() for e in (0.1, 0.3, 0.6, 1.0)]
    assert ds == sorted(ds) and ds[0] > 0


def test_oklab_distance_ignores_masked_pixels():
    a, b = torch.full((1, 3, 8, 8), 0.2), torch.full((1, 3, 8, 8), 0.2)
    b[:, :, :, 4:] = 0.9
    mask = torch.zeros(1, 1, 8, 8)
    mask[:, :, :, :4] = 1
    assert render.oklab_distance(a, b, mask).item() == pytest.approx(0.0, abs=1e-4)
    assert render.oklab_distance(a, b, None).item() > 1.0


def test_oklab_distance_is_float32_under_autocast():
    a, b = torch.rand(1, 3, 8, 8), torch.rand(1, 3, 8, 8)
    with torch.autocast(device_type="cpu", dtype=torch.bfloat16):
        d = render.oklab_distance(a.bfloat16(), b.bfloat16(), torch.ones(1, 1, 8, 8))
    assert d.dtype == torch.float32


def _batch(n=2, size=32):
    gen = torch.Generator().manual_seed(1)
    img = torch.rand(n, 3, size, size, generator=gen) * 0.6
    return {
        "input": torch.rand(n, 4, size, size, generator=gen),
        "clip": torch.zeros(n, 512), "clip_flag": torch.zeros(n, 1),
        "aligned": img, "target": img * 1.3, "mask": torch.ones(n, 1, size, size),
        "aspect": torch.ones(n), "has_target": torch.ones(n),
        "v_target": params.neutral_vector().repeat(n, 1), "v_known": torch.zeros(n, len(params.PARAMS)),
    }


@pytest.mark.parametrize("metric", ["oklab", "de2000"])
def test_loss_of_with_each_metric_reaches_the_prediction(metric):
    batch = _batch()
    v = params.neutral_vector().repeat(2, 1).requires_grad_(True)
    w = {"param": 1.0, "de": 1.0, "iou": 0.5, "presence": 0.2}
    loss, parts = train.loss_of(v, batch, w, metric=metric)
    assert torch.isfinite(loss)
    loss.backward()
    assert v.grad.abs().sum() > 0
    assert parts["de"] > 0


def test_an_oklab_run_ends_on_ciede2000():
    cfg = {"train_metric": "oklab", "de2000_tail": 0.1}
    got = [train.train_metric(cfg, e, 20) for e in range(20)]
    assert got == ["oklab"] * 18 + ["de2000"] * 2
    assert train.train_metric({}, 0, 20) == "de2000"
    with pytest.raises(ValueError):
        train.train_metric({"train_metric": "l2"}, 0, 20)


@pytest.mark.parametrize("metric", ["oklab", "de2000"])
def test_pixels_outside_his_crop_change_neither_loss_nor_de(metric):
    """Review focus 4: a pair's target is his JPEG warped back; outside it there is nothing to match."""
    batch = _batch()
    void = torch.zeros(2, 1, 32, 32)
    void[:, :, :, 20:] = 1
    batch["target_void"] = void
    v = params.neutral_vector().repeat(2, 1)
    w = {"param": 1.0, "de": 1.0, "iou": 0.5, "presence": 0.2}
    loss, parts = train.loss_of(v, batch, w, metric=metric)
    other = dict(batch, target=batch["target"].clone())
    other["target"][:, :, :, 20:] = 0.95
    loss2, parts2 = train.loss_of(v, other, w, metric=metric)
    assert parts2["de"].item() == pytest.approx(parts["de"].item(), abs=1e-6)
    assert loss2.item() == pytest.approx(loss.item(), abs=1e-6)
    whole = dict(other, target_void=torch.zeros_like(void))
    assert train.loss_of(v, whole, w, metric=metric)[1]["de"] > parts["de"] + 1e-3
