"""The Retouch network: shapes, ranges, size, and that a few steps of training learn."""

import torch

from lumen_train.retouch import params
from lumen_train.retouch.model import CLIP_DIM, RetouchNet

N = len(params.PARAMS)


def inputs(b: int = 2):
    image = torch.rand(b, 4, 256, 256)
    image[:, 3] = 1.0
    return image, torch.randn(b, CLIP_DIM), torch.ones(b, 1)


def test_outputs_one_value_per_slider_in_the_unit_range():
    m = RetouchNet().eval()
    out = m(*inputs())
    assert out.shape == (2, N)
    assert (out >= 0).all() and (out <= 1).all()


def test_an_untrained_model_starts_at_neutral():
    m = RetouchNet().eval()
    out = m(*inputs(4))
    neutral = params.neutral_vector()
    assert (out - neutral).abs().max() < 0.05, "a fresh model changes (almost) nothing"


def test_it_is_small_enough_to_ship():
    n = sum(p.numel() for p in RetouchNet().parameters())
    assert 1_000_000 < n < 6_000_000, n


def test_a_missing_clip_embedding_is_allowed():
    m = RetouchNet().eval()
    image, clip, flag = inputs()
    out = m(image, torch.zeros_like(clip), torch.zeros_like(flag))
    assert torch.isfinite(out).all()


def test_padding_does_not_count_in_its_statistics():
    """The same photo padded differently reads the same (the mask channel says where it is)."""
    m = RetouchNet().eval()
    a = torch.zeros(1, 4, 256, 256)
    a[:, :3, :100, :200] = 0.6
    a[:, 3, :100, :200] = 1.0
    stats = m.statistics(a)
    b = torch.zeros(1, 4, 256, 256)
    b[:, :3, :100, :200] = 0.6
    b[:, :3, 100:, :] = 0.9  # whatever lies outside the photo
    b[:, 3, :100, :200] = 1.0
    assert torch.allclose(stats, m.statistics(b), atol=1e-5)


def test_a_few_steps_learn_a_brightness_rule():
    """Dark images want more exposure: the model picks that up from the pixels."""
    torch.manual_seed(0)
    m = RetouchNet()
    opt = torch.optim.AdamW(m.parameters(), lr=5e-4)
    i = params.index("exposure")
    losses = []
    for _ in range(60):
        image = torch.rand(8, 4, 64, 64)
        image[:, 3] = 1.0
        level = torch.rand(8, 1, 1, 1)
        image[:, :3] = level
        out = m(image, torch.zeros(8, CLIP_DIM), torch.zeros(8, 1))
        loss = (out[:, i] - (1.0 - level.flatten())).abs().mean()
        opt.zero_grad()
        loss.backward()
        opt.step()
        losses.append(loss.item())
    assert sum(losses[-10:]) / 10 < 0.6 * losses[0], losses
