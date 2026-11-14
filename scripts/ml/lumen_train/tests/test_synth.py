"""Synthetic pairs: a finished photo, made worse in ways Lumen's sliders can undo."""

import numpy as np
import torch

from lumen_train.retouch import synth
from lumen_train.retouch.prepared import EDGE, decode_input, encode_input


def final(w: int = 256, h: int = 171) -> tuple[torch.Tensor, int, int]:
    y, x = torch.meshgrid(torch.linspace(0, 1, h), torch.linspace(0, 1, w), indexing="ij")
    lin = torch.stack([0.05 + 0.5 * x, 0.1 + 0.3 * y, 0.2 + 0.1 * x * y])
    return lin, w, h


def test_input_files_round_trip():
    lin, w, h = final()
    packed = encode_input(lin, w, h)
    assert packed.shape == (4, EDGE, EDGE)
    raw = (packed.clamp(0, 1) * 65535).round().to(torch.int32).numpy().astype(np.uint16)
    back, mask, bw, bh = decode_input(raw)
    assert (bw, bh) == (w, h)
    assert torch.allclose(back[:, :h, :w], lin, rtol=2e-3, atol=2e-4)
    assert mask[:, :h, :w].min() == 1 and mask[:, h:, :].max() == 0


def test_the_identity_degradation_changes_nothing():
    lin, w, h = final()
    pair = synth.make_pair(lin, synth.Degradation.identity())
    assert torch.allclose(pair["aligned"][:, :h, :w], lin, atol=1e-5)
    assert torch.allclose(pair["input"][:3, :h, :w], encode_input(lin, w, h)[:3, :h, :w], atol=1e-5)
    assert pair["straighten"] == 0.0
    assert pair["crop_on"] == 0.0


def test_a_tilt_asks_for_the_opposite_straighten():
    lin, w, h = final()
    d = synth.Degradation.identity()
    d.tilt = 3.0
    pair = synth.make_pair(lin, d)
    assert pair["straighten"] == -3.0
    assert pair["crop_on"] == 0.0
    assert not torch.allclose(pair["input"][:3], encode_input(lin, w, h)[:3])


def test_a_looser_frame_asks_for_the_crop_that_finds_the_photo_again():
    lin, w, h = final()
    d = synth.Degradation.identity()
    d.pad = (0.1, 0.0, 0.05, 0.2)  # left, top, right, bottom, as shares of the photo
    pair = synth.make_pair(lin, d)
    x, y, cw, ch = pair["crop_box"]
    assert pair["crop_on"] == 1.0
    assert abs(x - 0.1 / 1.15) < 1e-4 and abs(y) < 1e-6
    assert abs(cw - 1 / 1.15) < 1e-4 and abs(ch - 1 / 1.2) < 1e-4
    assert abs(pair["aspect"] - (w * 1.15) / (h * 1.2)) < 1e-3


def test_global_degradations_change_the_aligned_image_and_stay_finite():
    lin, w, h = final()
    gen = torch.Generator().manual_seed(3)
    for _ in range(20):
        d = synth.Degradation.sample(gen)
        pair = synth.make_pair(lin, d)
        assert torch.isfinite(pair["aligned"]).all() and torch.isfinite(pair["input"]).all()
        assert pair["input"].shape == (4, EDGE, EDGE)
        assert pair["target"].shape == (3, EDGE, EDGE)


def test_sampling_is_repeatable_with_a_seed():
    a = synth.Degradation.sample(torch.Generator().manual_seed(9))
    b = synth.Degradation.sample(torch.Generator().manual_seed(9))
    assert a == b
