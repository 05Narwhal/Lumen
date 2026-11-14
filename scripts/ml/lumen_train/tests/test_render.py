"""The PyTorch develop pipeline against Rust's (`develop-render-golden.json`), the parameter
layout against Rust's decode (`retouch-params.json`), and CIEDE2000 against Sharma's data."""

import json
import math

import pytest
import torch

from lumen_train.retouch import params, render
from lumen_train.retouch.paths import PARAMS_JSON, RENDER_GOLDEN

GOLDEN = json.loads(RENDER_GOLDEN.read_text(encoding="utf-8"))
LAYOUT = json.loads(PARAMS_JSON.read_text(encoding="utf-8"))


def image(k: int) -> torch.Tensor:
    w, h = GOLDEN["width"], GOLDEN["height"]
    px = torch.tensor(GOLDEN["images"][k], dtype=torch.float64).reshape(h, w, 3)
    return px.permute(2, 0, 1).unsqueeze(0)


@pytest.mark.parametrize("case", GOLDEN["cases"], ids=lambda c: f'{c["image"]}-{c["name"]}')
def test_render_matches_rust(case):
    dev = render.from_develop_json(case["develop"], dtype=torch.float64)
    out = render.render(image(case["image"]), dev)
    w, h = GOLDEN["width"], GOLDEN["height"]
    want = torch.tensor(case["output"], dtype=torch.float64).reshape(h, w, 3).permute(2, 0, 1).unsqueeze(0)
    shown = lambda x: render.to_display(x).clamp(0, 1)
    diff = (shown(out) - shown(want)).abs().max().item()
    assert diff <= 1.0 / 255.0 + 1e-6, f"{case['name']}: {diff * 255:.2f}/255"


def test_render_is_differentiable_in_every_slider():
    """Every slider moves some golden image, from neutral (where the model starts)."""
    start = params.neutral_vector().double().unsqueeze(0)
    start[0, params.index("lin_on")] = start[0, params.index("rad_on")] = 0.5
    v = start.clone().requires_grad_(True)
    dev = params.decode(v, torch.tensor([1.5], dtype=torch.float64), hard=False)
    sum(render.render(image(k), dev).mean() for k in range(4)).backward()
    g = v.grad[0]
    for name in ["exposure", "temperature", "contrast", "highlights", "curve_m2", "hsl_blue_s", "grade_shadows_a", "vignette_amount", "lin_exposure", "rad_exposure"]:
        assert g[params.index(name)].abs() > 0, name


@pytest.mark.parametrize("sample", LAYOUT["samples"], ids=lambda s: str(s["aspect"]))
def test_decode_matches_rust(sample):
    v = torch.tensor([sample["v"]], dtype=torch.float64)
    got = params.decode(v, torch.tensor([sample["aspect"]], dtype=torch.float64), hard=True)
    want = render.from_develop_json(sample["develop"], dtype=torch.float64)
    for key, value in want.items():
        assert torch.allclose(got[key], value, atol=1e-4), key
    crop = params.decode_crop(v, torch.tensor([sample["aspect"]], dtype=torch.float64))
    if sample["crop"] is None:
        assert crop["on"][0] < 0.5
    else:
        c = sample["crop"]
        assert crop["on"][0] >= 0.5
        assert torch.allclose(crop["box"][0], torch.tensor([c["x"], c["y"], c["w"], c["h"]], dtype=torch.float64), atol=1e-4)
    assert abs(params.value(v, "straighten")[0].item() - sample["straighten"]) < 1e-4


@pytest.mark.parametrize("w,h", [(16, 9), (9, 16)])
def test_a_padded_photo_renders_as_the_photo_alone(w, h):
    """Masks and the vignette sit on the photo, not on the square it is padded into (the
    model's input), whichever way round the photo is."""
    torch.manual_seed(1)
    photo = torch.rand(1, 3, h, w, dtype=torch.float64) * 0.6 + 0.02
    dev = render.from_develop_json(
        {
            "effects": {"vignette": {"amount": -0.6, "midpoint": 0.3, "feather": 0.4}},
            "masks": [
                {"shapes": [{"kind": "linear", "add": True, "from": [0.5, 0.0], "to": [0.5, 0.6]}], "adjust": {"exposure": -1.0}},
                {"shapes": [{"kind": "radial", "add": True, "center": [0.3, 0.6], "radius": [0.3, 0.2], "feather": 0.5, "inside": True}], "adjust": {"exposure": 0.8}},
            ],
        },
        dtype=torch.float64,
    )
    alone = render.render(photo, dev)
    square = torch.zeros(1, 3, 16, 16, dtype=torch.float64)
    square[:, :, :h, :w] = photo
    mask = torch.zeros(1, 1, 16, 16, dtype=torch.float64)
    mask[:, :, :h, :w] = 1.0
    size = render.photo_size(mask)
    assert size.tolist() == [[w, h]]
    padded = render.render(square, dev, size)
    assert torch.allclose(padded[:, :, :h, :w], alone, atol=1e-9)
    assert not torch.allclose(render.render(square, dev)[:, :, :h, :w], alone, atol=1e-3), "the square alone would move them"


def test_a_turned_radial_is_encoded_as_rust_does():
    """Rust's `a_turned_radial_keeps_its_shape_in_pixels`."""
    dev = {"masks": [{"shapes": [{"kind": "radial", "add": True, "center": [0.4, 0.5], "radius": [0.1, 0.3], "angle": 90.0, "feather": 0.5, "inside": True}], "adjust": {"exposure": 0.5}}]}
    v = params.encode_develop_json(dev, None, 0.0, 1.5)
    rx = params.value(torch.tensor([v]), "rad_rx").item()
    ry = params.value(torch.tensor([v]), "rad_ry").item()
    assert abs(rx - 0.2) < 1e-6 and abs(ry - 0.15) < 1e-6


def test_encode_inverts_decode_for_the_golden_samples():
    for s in LAYOUT["samples"]:
        v = torch.tensor([s["v"]], dtype=torch.float64)
        back = params.encode_develop_json(s["develop"], s["crop"], s["straighten"], s["aspect"])
        again = params.decode(torch.tensor([back], dtype=torch.float64), torch.tensor([s["aspect"]], dtype=torch.float64), hard=True)
        want = params.decode(v, torch.tensor([s["aspect"]], dtype=torch.float64), hard=True)
        for key in want:
            assert torch.allclose(again[key], want[key], atol=1e-4), key


# Sharma, Wu and Dalal (2005), "The CIEDE2000 color-difference formula", Table 1.
SHARMA = [
    ((50.0, 2.6772, -79.7751), (50.0, 0.0, -82.7485), 2.0425),
    ((50.0, 3.1571, -77.2803), (50.0, 0.0, -82.7485), 2.8615),
    ((50.0, 2.8361, -74.0200), (50.0, 0.0, -82.7485), 3.4412),
    ((50.0, -1.3802, -84.2814), (50.0, 0.0, -82.7485), 1.0000),
    ((50.0, -1.1848, -84.8006), (50.0, 0.0, -82.7485), 1.0000),
    ((50.0, -0.9009, -85.5211), (50.0, 0.0, -82.7485), 1.0000),
    ((50.0, 0.0, 0.0), (50.0, -1.0, 2.0), 2.3669),
    ((50.0, -1.0, 2.0), (50.0, 0.0, 0.0), 2.3669),
    ((50.0, 2.4900, -0.0010), (50.0, -2.4900, 0.0009), 7.1792),
    ((50.0, 2.4900, -0.0010), (50.0, -2.4900, 0.0010), 7.1792),
    ((50.0, 2.4900, -0.0010), (50.0, -2.4900, 0.0011), 7.2195),
    ((50.0, 2.4900, -0.0010), (50.0, -2.4900, 0.0012), 7.2195),
    ((50.0, -0.0010, 2.4900), (50.0, 0.0009, -2.4900), 4.8045),
    ((50.0, -0.0010, 2.4900), (50.0, 0.0010, -2.4900), 4.8045),
    ((50.0, -0.0010, 2.4900), (50.0, 0.0011, -2.4900), 4.7461),
    ((50.0, 2.5, 0.0), (50.0, 0.0, -2.5), 4.3065),
    ((50.0, 2.5, 0.0), (73.0, 25.0, -18.0), 27.1492),
    ((50.0, 2.5, 0.0), (61.0, -5.0, 29.0), 22.8977),
    ((50.0, 2.5, 0.0), (56.0, -27.0, -3.0), 31.9030),
    ((50.0, 2.5, 0.0), (58.0, 24.0, 15.0), 19.4535),
    ((50.0, 2.5, 0.0), (50.0, 3.1736, 0.5854), 1.0000),
    ((50.0, 2.5, 0.0), (50.0, 3.2972, 0.0), 1.0000),
    ((50.0, 2.5, 0.0), (50.0, 1.8634, 0.5757), 1.0000),
    ((50.0, 2.5, 0.0), (50.0, 3.2592, 0.3350), 1.0000),
    ((60.2574, -34.0099, 36.2677), (60.4626, -34.1751, 39.4387), 1.2644),
    ((63.0109, -31.0961, -5.8663), (62.8187, -29.7946, -4.0864), 1.2630),
    ((61.2901, 3.7196, -5.3901), (61.4292, 2.2480, -4.9620), 1.8731),
    ((35.0831, -44.1164, 3.7933), (35.0232, -40.0716, 1.5901), 1.8645),
    ((22.7233, 20.0904, -46.6940), (23.0331, 14.9730, -42.5619), 2.0373),
    ((36.4612, 47.8580, 18.3852), (36.2715, 50.5065, 21.2231), 1.4146),
    ((90.8027, -2.0831, 1.4410), (91.1528, -1.6435, 0.0447), 1.4441),
    ((90.9257, -0.5406, -0.9208), (88.6381, -0.8985, -0.7239), 1.5381),
    ((6.7747, -0.2908, -2.4247), (5.8714, -0.0985, -2.2286), 0.6377),
    ((2.0776, 0.0795, -1.1350), (0.9033, -0.0636, -0.5514), 0.9082),
]


def test_ciede2000_matches_sharmas_table():
    a = torch.tensor([p[0] for p in SHARMA], dtype=torch.float64)
    b = torch.tensor([p[1] for p in SHARMA], dtype=torch.float64)
    want = torch.tensor([p[2] for p in SHARMA], dtype=torch.float64)
    got = render.ciede2000(a, b)
    assert torch.allclose(got, want, atol=1e-4), (got - want).abs().max()


def test_lab_of_white_and_grey():
    lab = render.linear_to_lab(torch.tensor([[1.0, 1.0, 1.0], [0.18, 0.18, 0.18]], dtype=torch.float64))
    assert abs(lab[0, 0].item() - 100.0) < 1e-3 and lab[0, 1:].abs().max() < 1e-3
    assert abs(lab[1, 0].item() - 49.5) < 0.1


def test_ciede2000_has_a_gradient_for_greys_and_black():
    a = torch.tensor([[0.0, 0.0, 0.0], [50.0, 0.0, 0.0]], requires_grad=True)
    d = render.ciede2000(a, torch.tensor([[1.0, 0.0, 0.0], [52.0, 0.0, 0.0]]))
    d.sum().backward()
    assert torch.isfinite(a.grad).all(), a.grad


def test_ciede2000_has_a_gradient_at_zero_difference():
    a = torch.tensor([[50.0, 10.0, -10.0]], dtype=torch.float64, requires_grad=True)
    d = render.ciede2000(a, a.detach() + 1e-9)
    d.sum().backward()
    assert torch.isfinite(a.grad).all()
    assert not math.isnan(d.item())


# ---- The vectorised stages against the loops they replaced (Round 12-1) -------------------


def _reference_slopes(xs, ys):
    n = len(xs)
    h = torch.tensor([max(xs[k + 1] - xs[k], 1e-6) for k in range(n - 1)], dtype=ys.dtype)
    d = (ys[:, 1:] - ys[:, :-1]) / h
    ms = [d[:, 0]]
    for k in range(1, n - 1):
        ms.append(torch.where(d[:, k - 1] * d[:, k] <= 0, torch.zeros_like(d[:, k]), (d[:, k - 1] + d[:, k]) / 2.0))
    ms.append(d[:, n - 2])
    for k in range(n - 1):
        zero = d[:, k] == 0
        dk = torch.where(zero, torch.ones_like(d[:, k]), d[:, k])
        a, b = ms[k] / dk, ms[k + 1] / dk
        hh = a * a + b * b
        t = 3.0 / torch.sqrt(torch.clamp(hh, min=1e-12))
        limit = (hh > 9.0) & ~zero
        mk = torch.where(zero, torch.zeros_like(ms[k]), torch.where(limit, t * a * d[:, k], ms[k]))
        mk1 = torch.where(zero, torch.zeros_like(ms[k + 1]), torch.where(limit, t * b * d[:, k], ms[k + 1]))
        ms[k], ms[k + 1] = mk, mk1
    return torch.stack(ms, dim=1)


def _reference_curve(img, d):
    master = d["curve_master"]
    y = render.monotone_eval(params.CURVE_X, master, _reference_slopes(params.CURVE_X, master), render.linear_to_srgb(img))
    rgb_x = [0.0, 0.5, 1.0]
    chans = []
    for c in range(3):
        ys = d["curve_rgb"][:, c]
        chans.append(render.monotone_eval(rgb_x, ys, _reference_slopes(rgb_x, ys), y[:, c : c + 1]))
    y = torch.cat(chans, dim=1)
    return torch.where(y < 0, -render.srgb_to_linear(-y), render.srgb_to_linear(y))


def _reference_band_weights(hue):
    ws = []
    for c in render.BAND_HUES:
        dd = (torch.remainder(hue - c + 540.0, 360.0) - 180.0).abs()
        t = (dd / render.HALF_WIDTH) * (math.pi / 2)
        ws.append(torch.where(dd >= render.HALF_WIDTH, torch.zeros_like(dd), torch.cos(t) ** 2))
    w = torch.cat(ws, dim=1)
    s = w.sum(dim=1, keepdim=True)
    return torch.where(s > 0, w / torch.where(s > 0, s, torch.ones_like(s)), w)


def _reference_color(img, d):
    lab = render.to_oklab(img)
    L, a, b = lab[:, 0:1], lab[:, 1:2], lab[:, 2:3]
    c = torch.sqrt(a * a + b * b + 1e-20)
    tiny = c < 1e-9
    h = torch.remainder(torch.rad2deg(torch.atan2(b, torch.where(tiny, torch.ones_like(a), a))), 360.0)
    n = render.smoothstep(0.0, 0.04, c)
    w = _reference_band_weights(h)
    hsl = d["hsl"]
    dh = (w * hsl[:, :, 0].view(-1, 8, 1, 1)).sum(dim=1, keepdim=True)
    ds = (w * hsl[:, :, 1].view(-1, 8, 1, 1)).sum(dim=1, keepdim=True)
    dl = (w * hsl[:, :, 2].view(-1, 8, 1, 1)).sum(dim=1, keepdim=True)
    hue = torch.deg2rad(h + n * dh * render.HUE_SWING)
    cc = c * (1.0 + n * ds).clamp(min=0.0)
    changed = n > 0
    L = torch.where(changed, L * torch.pow(2.0, 0.5 * n * dl), L)
    a = torch.where(changed, cc * torch.cos(hue), a)
    b = torch.where(changed, cc * torch.sin(hue), b)
    ws = render.range_weights(L, d["grade_blending"], d["grade_balance"])
    g = d["grade"]
    for k in range(3):
        a = a + ws[k] * render._b(g[:, k, 0] * 0.08)
        b = b + ws[k] * render._b(g[:, k, 1] * 0.08)
        L = L + ws[k] * render._b(g[:, k, 2] * 0.15)
    return render.from_oklab(torch.cat([L, a, b], dim=1))


@pytest.mark.parametrize("seed", range(20))
def test_vectorised_stages_equal_the_loops(seed):
    gen = torch.Generator().manual_seed(seed)
    v = torch.rand(3, len(params.PARAMS), generator=gen)
    dev = params.decode(v, torch.tensor([1.0, 1.5, 0.66]), hard=False)
    img = torch.rand(3, 3, 24, 32, generator=gen) * 1.4
    assert torch.allclose(render.curve(img, dev), _reference_curve(img, dev), atol=1e-5)
    assert torch.allclose(render.color(img, dev), _reference_color(img, dev), atol=1e-5)
    for xs, ys in [(params.CURVE_X, dev["curve_master"]), ([0.0, 0.5, 1.0], dev["curve_rgb"][:, 1])]:
        assert torch.allclose(render.monotone_slopes(xs, ys), _reference_slopes(xs, ys), atol=1e-6)


def test_recomputing_the_stages_changes_neither_values_nor_gradients():
    gen = torch.Generator().manual_seed(5)
    img = torch.rand(2, 3, 16, 24, generator=gen)
    grads = []
    for recompute in (False, True):
        v = torch.rand(2, len(params.PARAMS), generator=torch.Generator().manual_seed(9)).requires_grad_(True)
        out = render.render(img, params.decode(v, torch.tensor([1.5, 0.7]), hard=False), torch.tensor([[24.0, 16.0], [20.0, 12.0]]), recompute=recompute)
        out.square().mean().backward()
        grads.append((out.detach(), v.grad))
    assert torch.allclose(grads[0][0], grads[1][0], atol=1e-6)
    assert torch.allclose(grads[0][1], grads[1][1], atol=1e-6)
