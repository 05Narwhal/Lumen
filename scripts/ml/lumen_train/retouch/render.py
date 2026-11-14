"""Lumen's global develop stages in PyTorch, batched and differentiable, ported stage by stage
from `photos-media/src/develop/{basic,masks,curve,color,oklab,effects,linear}.rs` and held to
them by `develop-render-golden.json`. Spatial stages (texture, clarity, dehaze, sharpening,
grain) are not here: training scores them on their slider values alone.

Images are (B, 3, H, W) in linear light; settings are `params.decode`'s dict. A photo padded
into a square (as the model reads it) passes `size`, its own width and height from the
top-left corner, so masks and the vignette sit on the photo as they do in the app, not on the
square."""

import math

import torch
from torch.utils.checkpoint import checkpoint

from .params import CURVE_X, LIN_ADJUST, RAD_ADJUST, neutral_value

LUMA = (0.2126, 0.7152, 0.0722)
MID = 0.18
BAND_HUES = (25.0, 60.0, 100.0, 140.0, 195.0, 262.0, 300.0, 330.0)
HALF_WIDTH = 45.0
HUE_SWING = 30.0


def _b(x: torch.Tensor) -> torch.Tensor:
    """A (B,) setting shaped to broadcast over (B, 1, H, W)."""
    return x.view(-1, 1, 1, 1)


def luma(c: torch.Tensor) -> torch.Tensor:
    return LUMA[0] * c[:, 0:1] + LUMA[1] * c[:, 1:2] + LUMA[2] * c[:, 2:3]


def smoothstep(e0, e1, x):
    t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0)
    return t * t * (3.0 - 2.0 * t)


def smoothstep_safe(e0, e1, x):
    """`masks::smoothstep`: the width never below 1e-9."""
    t = ((x - e0) / torch.clamp(torch.as_tensor(e1 - e0), min=1e-9)).clamp(0.0, 1.0)
    return t * t * (3.0 - 2.0 * t)


# ---- Basic -------------------------------------------------------------------------------


def wb_gains(temperature: torch.Tensor, tint: torch.Tensor) -> torch.Tensor:
    t, m = temperature / 100.0, tint / 100.0
    g = torch.stack([torch.pow(2.0, 0.35 * t), torch.pow(2.0, -0.25 * m), torch.pow(2.0, -0.35 * t)], dim=1)
    y = g[:, 0] * LUMA[0] + g[:, 1] * LUMA[1] + g[:, 2] * LUMA[2]
    return g / y.unsqueeze(1)


def apply_gains(img, temperature, tint, exposure):
    g = wb_gains(temperature, tint) * torch.pow(2.0, exposure).unsqueeze(1)
    return img * g.view(-1, 3, 1, 1)


def _bell(s, centre, sigma):
    d = (s - centre) / sigma
    return torch.exp(-0.5 * d * d)


def tone_stops(s, whites, highlights, shadows, blacks):
    return _b(whites) * 1.0 * _bell(s, 2.3, 0.7) + _b(highlights) * 1.5 * _bell(s, 1.2, 1.0) + _b(shadows) * 1.5 * _bell(s, -2.5, 1.3) + _b(blacks) * 1.5 * _bell(s, -4.5, 1.3)


def chroma(c):
    mx = c.amax(dim=1, keepdim=True)
    mn = c.amin(dim=1, keepdim=True)
    ok = mx > 1e-6
    return torch.where(ok, ((mx - mn) / torch.where(ok, mx, torch.ones_like(mx))).clamp(0.0, 1.0), torch.zeros_like(mx))


def apply_tones(c, contrast, highlights, shadows, whites, blacks, vibrance, saturation):
    y = luma(c)
    lit = y > 1e-7
    ys = torch.where(lit, y, torch.full_like(y, MID))
    s = torch.log2(ys / MID)
    s2 = s + tone_stops(s, whites, highlights, shadows, blacks)
    s3 = s2 + _b(contrast) * 1.2 * torch.tanh(s2 / 1.5)
    k = torch.where(lit, torch.pow(2.0, s3 - s), torch.ones_like(y))
    c = c * k
    y = luma(c)
    f = (1.0 + _b(vibrance) * (1.0 - chroma(c))) * (1.0 + _b(saturation))
    return y + (c - y) * f


def basic(img, d):
    img = apply_gains(img, d["temperature"], d["tint"], d["exposure"])
    return apply_tones(img, d["contrast"], d["highlights"], d["shadows"], d["whites"], d["blacks"], d["vibrance"], d["saturation"])


# ---- Masks -------------------------------------------------------------------------------


def photo_size(mask: torch.Tensor) -> torch.Tensor:
    """(B, 1, H, W) masks of photos anchored top-left → (B, 2) widths and heights in pixels."""
    return torch.stack([mask[:, 0, 0, :].sum(dim=-1), mask[:, 0, :, 0].sum(dim=-1)], dim=1)


def _grid(img, size=None):
    """Pixel centres, and each photo's width and height as (B, 1, 1, 1)."""
    b, _, h, w = img.shape
    ys = torch.arange(h, dtype=img.dtype, device=img.device).view(1, 1, h, 1) + 0.5
    xs = torch.arange(w, dtype=img.dtype, device=img.device).view(1, 1, 1, w) + 0.5
    if size is None:
        pw = torch.full((b,), float(w), dtype=img.dtype, device=img.device)
        ph = torch.full((b,), float(h), dtype=img.dtype, device=img.device)
    else:
        size = size.to(img.dtype).clamp(min=1.0)
        pw, ph = size[:, 0], size[:, 1]
    return xs, ys, _b(pw), _b(ph)


def linear_weight(img, frm, to, size=None):
    xs, ys, w, h = _grid(img, size)
    ax, ay = _b(frm[:, 0]) * w, _b(frm[:, 1]) * h
    dx, dy = _b(to[:, 0] - frm[:, 0]) * w, _b(to[:, 1] - frm[:, 1]) * h
    len2 = dx * dx + dy * dy
    t = ((xs - ax) * dx + (ys - ay) * dy) / torch.clamp(len2, min=1e-9)
    m = 1.0 - smoothstep(0.0, 1.0, t)
    return torch.where(len2 < 1e-9, torch.zeros_like(m), m)


def radial_weight(img, center, radius, feather, size=None):
    xs, ys, w, h = _grid(img, size)
    q1 = (xs - _b(center[:, 0]) * w) / (_b(radius[:, 0]) * w)
    q2 = (ys - _b(center[:, 1]) * h) / (_b(radius[:, 1]) * h)
    r = torch.sqrt(q1 * q1 + q2 * q2 + 1e-12)
    inner = _b(1.0 - feather)
    soft = 1.0 - smoothstep_safe(inner, torch.ones_like(inner), r)
    return torch.where(r <= inner, torch.ones_like(r), torch.where(r >= 1.0, torch.zeros_like(r), soft))


def _local(p, adjust: dict):
    z = torch.zeros_like(adjust["exposure"])
    q = apply_gains(p, adjust.get("temperature", z), z, adjust["exposure"])
    return apply_tones(q, adjust.get("contrast", z), adjust.get("highlights", z), adjust.get("shadows", z), z, z, z, adjust.get("saturation", z))


def masks(img, d, size=None):
    if "lin_on" in d:
        m = linear_weight(img, d["lin_from"], d["lin_to"], size) * _b(d["lin_on"])
        adj = {k: d["lin_adjust"][:, i] for i, k in enumerate(LIN_ADJUST)}
        img = img + m * (_local(img, adj) - img)
    if "rad_on" in d:
        m = radial_weight(img, d["rad_center"], d["rad_radius"], d["rad_feather"], size) * _b(d["rad_on"])
        adj = {k: d["rad_adjust"][:, i] for i, k in enumerate(RAD_ADJUST)}
        img = img + m * (_local(img, adj) - img)
    return img


# ---- Tone curve --------------------------------------------------------------------------


def srgb_to_linear(v):
    return torch.where(v <= 0.04045, v / 12.92, torch.pow(((v.clamp(min=0.04045) + 0.055) / 1.055), 2.4))


def linear_to_srgb(v):
    a = v.abs()
    enc = torch.where(a <= 0.0031308, a * 12.92, 1.055 * torch.pow(a.clamp(min=0.0031308), 1.0 / 2.4) - 0.055)
    return torch.sign(v) * enc


def to_display(v):
    return linear_to_srgb(v)


def monotone_slopes(xs: list[float], ys: torch.Tensor) -> torch.Tensor:
    """Fritsch–Carlson slopes for knots `xs` (shared) and values `ys` (B, n)."""
    n = len(xs)
    h = torch.tensor([max(xs[k + 1] - xs[k], 1e-6) for k in range(n - 1)], dtype=ys.dtype, device=ys.device)
    d = (ys[:, 1:] - ys[:, :-1]) / h
    inner = torch.where(d[:, :-1] * d[:, 1:] <= 0, torch.zeros_like(d[:, 1:]), (d[:, :-1] + d[:, 1:]) / 2.0)
    ms = list(torch.cat([d[:, :1], inner, d[:, -1:]], dim=1).unbind(1))
    # The limiter stays sequential: step k reads the slope step k - 1 wrote (as Rust does).
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


def monotone_eval(xs: list[float], ys: torch.Tensor, ms: torch.Tensor, x: torch.Tensor) -> torch.Tensor:
    """`Monotone::eval` at `x` (B, C, H, W) for one curve per batch row."""
    n = len(xs)
    B = x.shape[0]
    flat = x.reshape(B, -1)
    knots = torch.tensor(xs, dtype=x.dtype, device=x.device)
    k = (torch.bucketize(flat.detach().contiguous(), knots, right=True) - 1).clamp(0, n - 2)
    x0 = knots[k]
    hk = knots[k + 1] - x0
    y0, y1 = ys.gather(1, k), ys.gather(1, k + 1)
    m0, m1 = ms.gather(1, k), ms.gather(1, k + 1)
    t = (flat - x0) / hk
    t2, t3 = t * t, t * t * t
    inside = (2 * t3 - 3 * t2 + 1) * y0 + (t3 - 2 * t2 + t) * hk * m0 + (-2 * t3 + 3 * t2) * y1 + (t3 - t2) * hk * m1
    below = ys[:, :1] + (flat - xs[0]) * ms[:, :1]
    above = ys[:, -1:] + (flat - xs[-1]) * ms[:, -1:]
    out = torch.where(flat <= xs[0], below, torch.where(flat >= xs[-1], above, inside))
    return out.reshape(x.shape)


def curve(img, d):
    master = d["curve_master"]
    mm = monotone_slopes(CURVE_X, master)
    enc = linear_to_srgb(img)
    y = monotone_eval(CURVE_X, master, mm, enc)
    # The three channel curves as one batch of B·3 single-channel curves.
    b, _, hh, ww = y.shape
    rgb_x = [0.0, 0.5, 1.0]
    ys = d["curve_rgb"].reshape(b * 3, 3)
    y = monotone_eval(rgb_x, ys, monotone_slopes(rgb_x, ys), y.reshape(b * 3, 1, hh, ww)).reshape(b, 3, hh, ww)
    return torch.where(y < 0, -srgb_to_linear(-y), srgb_to_linear(y))


# ---- HSL and colour grading (OKLab) ------------------------------------------------------


def _cbrt(x):
    return torch.sign(x) * torch.pow(x.abs().clamp(min=1e-18), 1.0 / 3.0)


def to_oklab(c):
    r, g, b = c[:, 0:1], c[:, 1:2], c[:, 2:3]
    l = _cbrt(0.41222146 * r + 0.53633255 * g + 0.051445995 * b)
    m = _cbrt(0.2119035 * r + 0.6806995 * g + 0.10739696 * b)
    s = _cbrt(0.08830246 * r + 0.28171885 * g + 0.6299787 * b)
    return torch.cat(
        [
            0.21045426 * l + 0.7936178 * m - 0.004072047 * s,
            1.9779985 * l - 2.4285922 * m + 0.4505937 * s,
            0.025904037 * l + 0.78277177 * m - 0.80867577 * s,
        ],
        dim=1,
    )


def from_oklab(p):
    L, a, b = p[:, 0:1], p[:, 1:2], p[:, 2:3]
    l = (L + 0.39633778 * a + 0.21580376 * b) ** 3
    m = (L - 0.105561346 * a - 0.06385417 * b) ** 3
    s = (L - 0.08948418 * a - 1.2914855 * b) ** 3
    return torch.cat(
        [
            4.0767417 * l - 3.3077116 * m + 0.23096994 * s,
            -1.268438 * l + 2.6097574 * m - 0.34131938 * s,
            -0.0041960863 * l - 0.7034186 * m + 1.7076147 * s,
        ],
        dim=1,
    )


def band_weights(hue):
    """(B, 1, H, W) hues in degrees → (B, 8, H, W) shares summing to 1."""
    centres = torch.tensor(BAND_HUES, dtype=hue.dtype, device=hue.device).view(1, -1, 1, 1)
    dd = (torch.remainder(hue - centres + 540.0, 360.0) - 180.0).abs()
    t = (dd / HALF_WIDTH) * (math.pi / 2)
    w = torch.where(dd >= HALF_WIDTH, torch.zeros_like(dd), torch.cos(t) ** 2)
    s = w.sum(dim=1, keepdim=True)
    return torch.where(s > 0, w / torch.where(s > 0, s, torch.ones_like(s)), w)


def range_weights(L, blending, balance):
    width = _b(0.1 + 0.5 * blending)
    shift = _b(0.15 * balance)
    lo, hi = 0.33 + shift, 0.66 + shift
    s = 1.0 - smoothstep(lo - width / 2, lo + width / 2, L)
    h = smoothstep(hi - width / 2, hi + width / 2, L)
    return s, (1.0 - s - h).clamp(min=0.0), h


def color(img, d):
    lab = to_oklab(img)
    L, a, b = lab[:, 0:1], lab[:, 1:2], lab[:, 2:3]
    c = torch.sqrt(a * a + b * b + 1e-20)
    tiny = c < 1e-9
    h = torch.remainder(torch.rad2deg(torch.atan2(b, torch.where(tiny, torch.ones_like(a), a))), 360.0)
    n = smoothstep(0.0, 0.04, c)
    w = band_weights(h)
    hsl = d["hsl"]  # (B, 8, 3)
    dh = (w * hsl[:, :, 0].view(-1, 8, 1, 1)).sum(dim=1, keepdim=True)
    ds = (w * hsl[:, :, 1].view(-1, 8, 1, 1)).sum(dim=1, keepdim=True)
    dl = (w * hsl[:, :, 2].view(-1, 8, 1, 1)).sum(dim=1, keepdim=True)
    hue = torch.deg2rad(h + n * dh * HUE_SWING)
    cc = c * (1.0 + n * ds).clamp(min=0.0)
    changed = n > 0
    L = torch.where(changed, L * torch.pow(2.0, 0.5 * n * dl), L)
    a = torch.where(changed, cc * torch.cos(hue), a)
    b = torch.where(changed, cc * torch.sin(hue), b)
    ws = range_weights(L, d["grade_blending"], d["grade_balance"])
    g = d["grade"]  # (B, 3, 3): a, b, lum per range
    shift = (torch.stack(ws, dim=1) * (g * g.new_tensor([0.08, 0.08, 0.15])).view(-1, 3, 3, 1, 1)).sum(dim=1)
    L, a, b = L + shift[:, 2:3], a + shift[:, 0:1], b + shift[:, 1:2]
    return from_oklab(torch.cat([L, a, b], dim=1))


# ---- Effects -----------------------------------------------------------------------------


def vignette(img, d, size=None):
    amount, mid, feather = d["vignette"][:, 0], d["vignette"][:, 1], d["vignette"][:, 2]
    xs, ys, w, h = _grid(img, size)
    dx = (xs - w / 2) / (w / 2)
    dy = (ys - h / 2) / (h / 2)
    r = torch.sqrt(dx * dx + dy * dy)
    start = _b(0.3 + 0.9 * mid)
    # Rust's f = weight · |amount|, written with the signed amount on each side so the
    # gradient does not vanish at 0.
    weight = smoothstep(start, start + _b(0.1 + feather), r)
    a = _b(amount)
    dark = img * (1.0 + weight * a)
    light = img + (1.0 - img).clamp(min=0.0) * weight * a
    return torch.where(a < 0, dark, light)


def render(img: torch.Tensor, d: dict, size: torch.Tensor | None = None, recompute: bool = False) -> torch.Tensor:
    """Every global stage, in `develop_stages`' order. `size` (B, 2): each photo's width and
    height in pixels from the top-left corner (`photo_size(mask)`); None for the whole image.
    `recompute` keeps only each stage's output for the backward pass and recomputes the rest
    (same values): a batch of 64 otherwise holds more than a 12 GB card has."""
    stages = [lambda x: basic(x, d), lambda x: masks(x, d, size), lambda x: curve(x, d), lambda x: color(x, d), lambda x: vignette(x, d, size)]
    for stage in stages:
        img = checkpoint(stage, img, use_reentrant=False) if recompute and torch.is_grad_enabled() else stage(img)
    return img


# ---- From a Develop as JSON (the golden files) -------------------------------------------


def from_develop_json(dev: dict, dtype=torch.float32) -> dict:
    t = lambda x: torch.tensor([x], dtype=dtype)
    basic_ = dev.get("basic", {})
    d = {n: t(basic_.get(n, 0.0)) for n in ["temperature", "tint", "exposure", "contrast", "highlights", "shadows", "whites", "blacks", "vibrance", "saturation"]}
    cv = dev.get("curve", {})

    def ys_at(points, xs):
        if not points:
            return list(xs)
        got = {round(p[0], 4): p[1] for p in points}
        if set(got) != {round(x, 4) for x in xs}:
            raise ValueError(f"curve points {points} are not at {xs}")
        return [got[round(x, 4)] for x in xs]

    d["curve_master"] = torch.tensor([ys_at(cv.get("master", []), CURVE_X)], dtype=dtype)
    d["curve_rgb"] = torch.tensor([[ys_at(cv.get(c, []), [0.0, 0.5, 1.0]) for c in "rgb"]], dtype=dtype)
    hsl = dev.get("hsl") or [{}] * 8
    d["hsl"] = torch.tensor([[[b.get("h", 0.0), b.get("s", 0.0), b.get("l", 0.0)] for b in hsl]], dtype=dtype)
    g = dev.get("grading", {})
    wheels = []
    for r in ["shadows", "midtones", "highlights"]:
        w = g.get(r, {})
        hue, sat = math.radians(w.get("hue", 0.0)), w.get("sat", 0.0)
        wheels.append([sat * math.cos(hue), sat * math.sin(hue), w.get("lum", 0.0)])
    d["grade"] = torch.tensor([wheels], dtype=dtype)
    d["grade_blending"] = t(g.get("blending", 0.5))
    d["grade_balance"] = t(g.get("balance", 0.0))
    v = dev.get("effects", {}).get("vignette", {})
    d["vignette"] = torch.tensor([[v.get("amount", 0.0), v.get("midpoint", 0.5), v.get("feather", 0.5)]], dtype=dtype)
    lin = rad = None
    for mask in dev.get("masks", []) or []:
        sh = (mask.get("shapes") or [{}])[0]
        if sh.get("kind") == "linear" and lin is None:
            lin = (sh, mask.get("adjust", {}))
        elif sh.get("kind") == "radial" and rad is None:
            rad = (sh, mask.get("adjust", {}))
    if lin:
        sh, adj = lin
        d.update(lin_on=t(1.0), lin_from=torch.tensor([sh["from"]], dtype=dtype), lin_to=torch.tensor([sh["to"]], dtype=dtype), lin_adjust=torch.tensor([[adj.get(k, 0.0) for k in LIN_ADJUST]], dtype=dtype))
    else:
        d.update(
            lin_on=t(0.0),
            lin_from=torch.tensor([[neutral_value("lin_fx"), neutral_value("lin_fy")]], dtype=dtype),
            lin_to=torch.tensor([[neutral_value("lin_tx"), neutral_value("lin_ty")]], dtype=dtype),
            lin_adjust=torch.zeros(1, len(LIN_ADJUST), dtype=dtype),
        )
    if rad:
        sh, adj = rad
        d.update(
            rad_on=t(1.0),
            rad_center=torch.tensor([sh["center"]], dtype=dtype),
            rad_radius=torch.tensor([sh["radius"]], dtype=dtype),
            rad_feather=t(sh.get("feather", 0.5)),
            rad_adjust=torch.tensor([[adj.get(k, 0.0) for k in RAD_ADJUST]], dtype=dtype),
        )
    else:
        d.update(
            rad_on=t(0.0),
            rad_center=torch.tensor([[neutral_value("rad_cx"), neutral_value("rad_cy")]], dtype=dtype),
            rad_radius=torch.tensor([[neutral_value("rad_rx"), neutral_value("rad_ry")]], dtype=dtype),
            rad_feather=t(neutral_value("rad_feather")),
            rad_adjust=torch.zeros(1, len(RAD_ADJUST), dtype=dtype),
        )
    return d


# ---- Colour difference -------------------------------------------------------------------

_M = ((0.4124564, 0.3575761, 0.1804375), (0.2126729, 0.7151522, 0.0721750), (0.0193339, 0.1191920, 0.9503041))
_WHITE = (0.95047, 1.0, 1.08883)


def linear_to_lab(c: torch.Tensor) -> torch.Tensor:
    """(..., 3) linear sRGB → CIELAB (D65)."""
    m = torch.tensor(_M, dtype=c.dtype, device=c.device)
    xyz = c @ m.T / torch.tensor(_WHITE, dtype=c.dtype, device=c.device)
    e = (6.0 / 29.0) ** 3
    f = torch.where(xyz > e, torch.pow(xyz.clamp(min=e), 1.0 / 3.0), xyz / (3 * (6.0 / 29.0) ** 2) + 4.0 / 29.0)
    return torch.stack([116 * f[..., 1] - 16, 500 * (f[..., 0] - f[..., 1]), 200 * (f[..., 1] - f[..., 2])], dim=-1)


def ciede2000(lab1: torch.Tensor, lab2: torch.Tensor) -> torch.Tensor:
    """CIEDE2000 between (..., 3) Lab colours (Sharma, Wu and Dalal 2005)."""
    eps = 1e-12
    L1, a1, b1 = lab1.unbind(-1)
    L2, a2, b2 = lab2.unbind(-1)
    C1 = torch.sqrt(a1 * a1 + b1 * b1 + eps)
    C2 = torch.sqrt(a2 * a2 + b2 * b2 + eps)
    Cb7 = ((C1 + C2) / 2) ** 7
    G = 0.5 * (1 - torch.sqrt(Cb7 / (Cb7 + 25.0**7) + eps))
    a1p, a2p = (1 + G) * a1, (1 + G) * a2
    C1p = torch.sqrt(a1p * a1p + b1 * b1 + eps)
    C2p = torch.sqrt(a2p * a2p + b2 * b2 + eps)
    zero1 = (a1p.abs() < 1e-12) & (b1.abs() < 1e-12)
    zero2 = (a2p.abs() < 1e-12) & (b2.abs() < 1e-12)
    h1p = torch.where(zero1, torch.zeros_like(a1), torch.remainder(torch.rad2deg(torch.atan2(b1, torch.where(zero1, torch.ones_like(a1p), a1p))), 360.0))
    h2p = torch.where(zero2, torch.zeros_like(a2), torch.remainder(torch.rad2deg(torch.atan2(b2, torch.where(zero2, torch.ones_like(a2p), a2p))), 360.0))
    either = zero1 | zero2
    dLp = L2 - L1
    dCp = C2p - C1p
    dh = h2p - h1p
    dhp = torch.where(either, torch.zeros_like(dh), torch.where(dh.abs() <= 180, dh, torch.where(dh > 180, dh - 360, dh + 360)))
    dHp = 2 * torch.sqrt(C1p * C2p + eps) * torch.sin(torch.deg2rad(dhp) / 2)
    Lbp = (L1 + L2) / 2
    Cbp = (C1p + C2p) / 2
    hsum = h1p + h2p
    hbp = torch.where(either, hsum, torch.where((h1p - h2p).abs() <= 180, hsum / 2, torch.where(hsum < 360, (hsum + 360) / 2, (hsum - 360) / 2)))
    T = 1 - 0.17 * torch.cos(torch.deg2rad(hbp - 30)) + 0.24 * torch.cos(torch.deg2rad(2 * hbp)) + 0.32 * torch.cos(torch.deg2rad(3 * hbp + 6)) - 0.20 * torch.cos(torch.deg2rad(4 * hbp - 63))
    dtheta = 30 * torch.exp(-(((hbp - 275) / 25) ** 2))
    Cbp7 = Cbp**7
    RC = 2 * torch.sqrt(Cbp7 / (Cbp7 + 25.0**7) + eps)
    SL = 1 + 0.015 * (Lbp - 50) ** 2 / torch.sqrt(20 + (Lbp - 50) ** 2)
    SC = 1 + 0.045 * Cbp
    SH = 1 + 0.015 * Cbp * T
    RT = -torch.sin(torch.deg2rad(2 * dtheta)) * RC
    x = (dLp / SL) ** 2 + (dCp / SC) ** 2 + (dHp / SH) ** 2 + RT * (dCp / SC) * (dHp / SH)
    return torch.sqrt(x.clamp(min=0) + eps)


def oklab_distance(a: torch.Tensor, b: torch.Tensor, mask: torch.Tensor | None = None) -> torch.Tensor:
    """Mean Euclidean OKLab distance ×100 per image between (B, 3, H, W) linear images (inside
    `mask`): a training metric about a tenth of CIEDE2000's kernels, on a similar scale. Always
    float32, autocast or not."""
    with torch.autocast(device_type=a.device.type, enabled=False):
        la = to_oklab(a.float().clamp(min=0))
        lb = to_oklab(b.float().clamp(min=0))
        dist = torch.sqrt(((la - lb) ** 2).sum(dim=1) + 1e-12) * 100.0
        if mask is None:
            return dist.mean(dim=(1, 2))
        m = mask[:, 0].float()
        return (dist * m).sum(dim=(1, 2)) / m.sum(dim=(1, 2)).clamp(min=1.0)


def delta_e(a: torch.Tensor, b: torch.Tensor, mask: torch.Tensor | None = None) -> torch.Tensor:
    """Mean CIEDE2000 per image between (B, 3, H, W) linear images (inside `mask`)."""
    la = linear_to_lab(a.clamp(min=0).permute(0, 2, 3, 1))
    lb = linear_to_lab(b.clamp(min=0).permute(0, 2, 3, 1))
    de = ciede2000(la, lb)
    if mask is None:
        return de.mean(dim=(1, 2))
    m = mask[:, 0]
    return (de * m).sum(dim=(1, 2)) / m.sum(dim=(1, 2)).clamp(min=1.0)
