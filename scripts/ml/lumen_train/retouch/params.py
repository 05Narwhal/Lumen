"""The Retouch vector, read from the layout Rust writes (`photos-core::retouch::params`,
`retouch-params.json`). `decode` turns a batch of vectors into the render's settings (see
`render.py`); with `hard=True` it does exactly what Rust's `decode` does, with `hard=False`
the on/off switches are soft so training can learn them."""

import json
import math

import torch

from .paths import PARAMS_JSON

_LAYOUT = json.loads(PARAMS_JSON.read_text(encoding="utf-8"))
LAYOUT: int = _LAYOUT["layout"]
PARAMS: list[dict] = _LAYOUT["params"]
RATIOS: list[float] = _LAYOUT["ratios"]
NAMES = [p["name"] for p in PARAMS]
_INDEX = {n: i for i, n in enumerate(NAMES)}
# `photos-media::retouch_input::FEATURES`.
FEATURES = 80
HSL_BANDS = ["red", "orange", "yellow", "green", "aqua", "blue", "purple", "magenta"]
CURVE_X = [0.0, 0.25, 0.5, 0.75, 1.0]
LIN_ADJUST = ["exposure", "contrast", "highlights", "shadows", "temperature", "saturation"]
RAD_ADJUST = ["exposure", "contrast", "shadows", "saturation", "temperature"]


def index(name: str) -> int:
    return _INDEX[name]


def neutral_unit(name: str) -> float:
    p = PARAMS[index(name)]
    return (p["neutral"] - p["min"]) / (p["max"] - p["min"])


def neutral_vector() -> torch.Tensor:
    return torch.tensor([neutral_unit(n) for n in NAMES])


def unit(v: torch.Tensor, name: str) -> torch.Tensor:
    return v[:, index(name)].clamp(0.0, 1.0)


def value(v: torch.Tensor, name: str) -> torch.Tensor:
    p = PARAMS[index(name)]
    return p["min"] + unit(v, name) * (p["max"] - p["min"])


def neutral_value(name: str) -> float:
    return PARAMS[index(name)]["neutral"]


def _gate(v: torch.Tensor, name: str, hard: bool) -> torch.Tensor:
    u = unit(v, name)
    return (u >= 0.5).to(v.dtype) if hard else torch.sigmoid((u - 0.5) * 10.0)


def _wheel(a: torch.Tensor, b: torch.Tensor, hard: bool) -> tuple[torch.Tensor, torch.Tensor]:
    # Rust: sat = min(hypot, 1); a, b back from (sat, hue).
    h = torch.sqrt(a * a + b * b + 1e-12)
    k = torch.clamp(1.0 / h, max=1.0)
    a, b = a * k, b * k
    if hard:
        small = (h * k) < 1e-4
        a = torch.where(small, torch.zeros_like(a), a)
        b = torch.where(small, torch.zeros_like(b), b)
    return a, b


def decode(v: torch.Tensor, aspect: torch.Tensor, hard: bool = False) -> dict:
    """`v` is (B, N) in [0, 1]; returns the render settings (each (B, ...))."""
    del aspect  # the crop is decoded on its own (`decode_crop`)
    d: dict[str, torch.Tensor] = {}
    for n in ["temperature", "tint", "exposure", "contrast", "highlights", "shadows", "whites", "blacks", "vibrance", "saturation"]:
        d[n] = value(v, n)
    ys = [value(v, "curve_m0"), 0.25 + value(v, "curve_m1"), 0.5 + value(v, "curve_m2"), 0.75 + value(v, "curve_m3"), value(v, "curve_m4")]
    d["curve_master"] = torch.stack(ys, dim=1).clamp(0.0, 1.0)
    rgb = [torch.stack([torch.zeros_like(v[:, 0]), 0.5 + value(v, f"curve_{c}"), torch.ones_like(v[:, 0])], dim=1) for c in "rgb"]
    d["curve_rgb"] = torch.stack(rgb, dim=1)
    d["hsl"] = torch.stack([torch.stack([value(v, f"hsl_{b}_{k}") for k in "hsl"], dim=1) for b in HSL_BANDS], dim=1)
    wheels = []
    for r in ["shadows", "midtones", "highlights"]:
        a, b = _wheel(value(v, f"grade_{r}_a"), value(v, f"grade_{r}_b"), hard)
        wheels.append(torch.stack([a, b, value(v, f"grade_{r}_lum")], dim=1))
    d["grade"] = torch.stack(wheels, dim=1)
    d["grade_blending"] = value(v, "grade_blending")
    d["grade_balance"] = value(v, "grade_balance")
    d["vignette"] = torch.stack([value(v, "vignette_amount"), value(v, "vignette_midpoint"), value(v, "vignette_feather")], dim=1)

    lin_adjust = torch.stack([value(v, f"lin_{k}") for k in LIN_ADJUST], dim=1)
    lin_from = torch.stack([value(v, "lin_fx"), value(v, "lin_fy")], dim=1)
    lin_to = torch.stack([value(v, "lin_tx"), value(v, "lin_ty")], dim=1)
    lin_on = _gate(v, "lin_on", hard)
    rad_adjust = torch.stack([value(v, f"rad_{k}") for k in RAD_ADJUST], dim=1)
    rad_center = torch.stack([value(v, "rad_cx"), value(v, "rad_cy")], dim=1)
    rad_radius = torch.stack([value(v, "rad_rx"), value(v, "rad_ry")], dim=1)
    rad_feather = value(v, "rad_feather")
    rad_on = _gate(v, "rad_on", hard)
    if hard:
        # Rust drops a mask that is off or changes nothing; an absent mask reads as neutral.
        lin_on = lin_on * (lin_adjust.abs().amax(dim=1) > 0).to(v.dtype)
        rad_on = rad_on * (rad_adjust.abs().amax(dim=1) > 0).to(v.dtype)
        off = lambda on, x, neutral: torch.where(on.view(-1, *([1] * (x.dim() - 1))) > 0.5, x, x.new_tensor(neutral).expand_as(x))
        lin_adjust = off(lin_on, lin_adjust, [0.0] * len(LIN_ADJUST))
        lin_from = off(lin_on, lin_from, [neutral_value("lin_fx"), neutral_value("lin_fy")])
        lin_to = off(lin_on, lin_to, [neutral_value("lin_tx"), neutral_value("lin_ty")])
        rad_adjust = off(rad_on, rad_adjust, [0.0] * len(RAD_ADJUST))
        rad_center = off(rad_on, rad_center, [neutral_value("rad_cx"), neutral_value("rad_cy")])
        rad_radius = off(rad_on, rad_radius, [neutral_value("rad_rx"), neutral_value("rad_ry")])
        rad_feather = off(rad_on, rad_feather, neutral_value("rad_feather"))
    d.update(lin_on=lin_on, lin_from=lin_from, lin_to=lin_to, lin_adjust=lin_adjust)
    d.update(rad_on=rad_on, rad_center=rad_center, rad_radius=rad_radius, rad_feather=rad_feather, rad_adjust=rad_adjust)
    return d


def _ratio_of(bin_: int, aspect: float) -> float | None:
    r = RATIOS[bin_]
    if r == 0.0:
        return None
    if r < 0.0:
        return aspect
    return 1.0 / r if aspect < 1.0 else r


def decode_crop(v: torch.Tensor, aspect: torch.Tensor) -> dict:
    """Rust's `decode_crop`, row by row: `on` (B,) and `box` (B, 4) as x, y, w, h."""
    on, boxes = [], []
    for i in range(v.shape[0]):
        row = v[i : i + 1]
        a = float(aspect[i]) if float(aspect[i]) > 0 else 1.0
        x, y = float(value(row, "crop_x")), float(value(row, "crop_y"))
        w = min(float(value(row, "crop_w")), 1.0 - x)
        b = min(int(float(unit(row, "crop_ratio")) * len(RATIOS)), len(RATIOS) - 1)
        r = _ratio_of(b, a)
        if r is None:
            h = min(float(value(row, "crop_h")), 1.0 - y)
        else:
            h = w * a / r
            if h > 1.0 - y:
                h = 1.0 - y
                w = h * r / a
        whole = x < 1e-4 and y < 1e-4 and w > 0.9999 and h > 0.9999
        on.append(1.0 if float(unit(row, "crop_on")) >= 0.5 and not whole else 0.0)
        boxes.append([x, y, w, h])
    return {"on": v.new_tensor(on), "box": v.new_tensor(boxes)}


def _curve_at(points: list, x: float) -> float:
    if not points:
        return x
    if x <= points[0][0]:
        return points[0][1]
    for (ax, ay), (bx, by) in zip(points, points[1:]):
        if x <= bx:
            t = (x - ax) / (bx - ax) if bx > ax else 1.0
            return ay + t * (by - ay)
    return points[-1][1]


def layout_masks(develop: dict) -> tuple[dict | None, dict | None]:
    """The masks the layout holds, as Rust's `encode` picks them: the first shown, uninverted,
    single-shape, effective linear gradient (adding), and the first such radial (adding,
    acting inside)."""
    lin = rad = None
    for mask in develop.get("masks") or []:
        shapes = mask.get("shapes", [])
        adj = mask.get("adjust", {})
        if not mask.get("visible", True) or mask.get("invert", False) or len(shapes) != 1 or not any(adj.get(f, 0.0) for f in adj):
            continue
        sh = shapes[0]
        if sh.get("kind") == "linear" and sh.get("add", True) and lin is None:
            lin = mask
        elif sh.get("kind") == "radial" and sh.get("add", True) and sh.get("inside", True) and rad is None:
            rad = mask
    return lin, rad


def encode_develop_json(develop: dict, crop: dict | None, straighten: float, aspect: float) -> list[float]:
    """Rust's `encode`, from a `Develop` as serde writes it (`lr2lumen`, the golden files)."""
    out = [neutral_unit(n) for n in NAMES]

    def put(name: str, x: float) -> None:
        p = PARAMS[index(name)]
        if x is not None and math.isfinite(x):
            out[index(name)] = min(1.0, max(0.0, (x - p["min"]) / (p["max"] - p["min"])))

    put("straighten", straighten)
    if crop is not None:
        a = aspect if aspect > 0 else 1.0
        for k in "xywh":
            put(f"crop_{k}", crop[k])
        put("crop_on", 1.0)
        pixels = crop["w"] * a / max(crop["h"], 1e-6)
        found = next((b for b in range(1, len(RATIOS)) if (r := _ratio_of(b, a)) is not None and abs(r - pixels) < 1e-3 * r), 0)
        put("crop_ratio", (found + 0.5) / len(RATIOS))
    basic = develop.get("basic", {})
    for n in ["temperature", "tint", "exposure", "contrast", "highlights", "shadows", "whites", "blacks", "texture", "clarity", "dehaze", "vibrance", "saturation"]:
        put(n, basic.get(n, 0.0))
    curve = develop.get("curve", {})
    m = curve.get("master", [])
    for i, x in enumerate(CURVE_X):
        y = _curve_at(m, x)
        put(f"curve_m{i}", y if i in (0, 4) else y - x)
    for c in "rgb":
        put(f"curve_{c}", _curve_at(curve.get(c, []), 0.5) - 0.5)
    for i, b in enumerate(HSL_BANDS):
        band = (develop.get("hsl") or [{}] * 8)[i]
        for k in "hsl":
            put(f"hsl_{b}_{k}", band.get(k, 0.0))
    g = develop.get("grading", {})
    for r in ["shadows", "midtones", "highlights"]:
        w = g.get(r, {})
        hue, sat = math.radians(w.get("hue", 0.0)), w.get("sat", 0.0)
        put(f"grade_{r}_a", sat * math.cos(hue))
        put(f"grade_{r}_b", sat * math.sin(hue))
        put(f"grade_{r}_lum", w.get("lum", 0.0))
    put("grade_blending", g.get("blending", 0.5))
    put("grade_balance", g.get("balance", 0.0))
    s = develop.get("detail", {}).get("sharpen", {})
    put("sharpen_amount", s.get("amount", 0.0))
    put("sharpen_radius", s.get("radius", 1.0))
    put("sharpen_detail", s.get("detail", 0.25))
    put("sharpen_masking", s.get("masking", 0.0))
    e = develop.get("effects", {})
    vig, grain = e.get("vignette", {}), e.get("grain", {})
    put("vignette_amount", vig.get("amount", 0.0))
    put("vignette_midpoint", vig.get("midpoint", 0.5))
    put("vignette_feather", vig.get("feather", 0.5))
    put("grain_amount", grain.get("amount", 0.0))
    put("grain_size", grain.get("size", 0.25))
    lin, rad = layout_masks(develop)
    if lin is not None:
        sh, adj, k = lin["shapes"][0], lin.get("adjust", {}), lin.get("opacity", 1.0)
        put("lin_on", 1.0)
        put("lin_fx", sh["from"][0])
        put("lin_fy", sh["from"][1])
        put("lin_tx", sh["to"][0])
        put("lin_ty", sh["to"][1])
        for f in LIN_ADJUST:
            put(f"lin_{f}", adj.get(f, 0.0) * k)
    if rad is not None:
        sh, adj, k = rad["shapes"][0], rad.get("adjust", {}), rad.get("opacity", 1.0)
        # Upright only: a turned ellipse is read at the nearer quarter turn, in pixels.
        a = aspect if aspect > 0 else 1.0
        rx, ry = sh["radius"]
        if abs(sh.get("angle", 0.0) % 180.0 - 90.0) < 45.0:
            rx, ry = ry / a, rx * a
        put("rad_on", 1.0)
        put("rad_cx", sh["center"][0])
        put("rad_cy", sh["center"][1])
        put("rad_rx", rx)
        put("rad_ry", ry)
        put("rad_feather", sh.get("feather", 0.5))
        for f in RAD_ADJUST:
            put(f"rad_{f}", adj.get(f, 0.0) * k)
    return out
