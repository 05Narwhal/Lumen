"""Synthetic pairs: a finished photo, made worse in ways Lumen's sliders can undo.

Global adjustments are nearly invertible, so a well-edited photo with its exposure, white
balance, contrast, colour and curve knocked off teaches Retouch to put them back. A tilt
teaches straightening, and a looser frame (the photo mirrored outwards) teaches cropping.
A photo left alone teaches that a good photo needs nothing.

`make_pair` returns the model's `input` (the degraded photo, prepared), the `aligned` copy
(the same global degradation without the geometry, which the prediction is rendered on) and
the `target` (the finished photo), with the geometry the prediction should find."""

from dataclasses import dataclass, field, fields

import torch
import torch.nn.functional as F

from . import render
from .params import CURVE_X, HSL_BANDS
from .prepared import EDGE, encode


@dataclass
class Degradation:
    exposure: float = 0.0
    temperature: float = 0.0
    tint: float = 0.0
    contrast: float = 0.0
    highlights: float = 0.0
    shadows: float = 0.0
    vibrance: float = 0.0
    saturation: float = 0.0
    flatten: float = 0.0  # 0–0.3: blacks raised and whites lowered on the encoded curve
    hue_band: int = 0
    hue_shift: float = 0.0
    vignette: float = 0.0
    tilt: float = 0.0  # degrees, applied as Lumen's straighten
    pad: tuple[float, float, float, float] = field(default=(0.0, 0.0, 0.0, 0.0))  # left, top, right, bottom

    @staticmethod
    def identity() -> "Degradation":
        return Degradation()

    @staticmethod
    def sample(gen: torch.Generator) -> "Degradation":
        def u(lo: float, hi: float) -> float:
            return lo + (hi - lo) * float(torch.rand(1, generator=gen))

        def maybe(p: float) -> bool:
            return float(torch.rand(1, generator=gen)) < p

        d = Degradation()
        if maybe(0.8):
            d.exposure = max(-2.0, min(2.0, float(torch.randn(1, generator=gen)) * 0.8))
        if maybe(0.7):
            d.temperature = u(-40.0, 40.0)
            d.tint = u(-20.0, 20.0)
        if maybe(0.6):
            d.contrast = u(-0.4, 0.4)
        if maybe(0.5):
            d.highlights = u(-0.3, 0.3)
            d.shadows = u(-0.3, 0.3)
        if maybe(0.6):
            d.saturation = u(-0.4, 0.3)
            d.vibrance = u(-0.3, 0.3)
        if maybe(0.3):
            d.flatten = u(0.0, 0.25)
        if maybe(0.3):
            d.hue_band = int(torch.randint(0, len(HSL_BANDS), (1,), generator=gen))
            d.hue_shift = u(-0.3, 0.3)
        if maybe(0.2):
            d.vignette = u(-0.3, 0.3)
        geometry = float(torch.rand(1, generator=gen))
        if geometry < 0.25:
            d.tilt = u(0.5, 6.0) * (1 if maybe(0.5) else -1)
        elif geometry < 0.5:
            sides = [u(0.0, 0.2) if maybe(0.6) else 0.0 for _ in range(4)]
            if max(sides) == 0.0:
                sides[int(torch.randint(0, 4, (1,), generator=gen))] = u(0.05, 0.2)
            d.pad = tuple(sides)
        return d

    def to_tensor(self) -> torch.Tensor:
        """(DEG_LEN,) float32: every field in order, `pad` as its four sides."""
        return torch.tensor([float(getattr(self, k)) for k in DEG_FIELDS] + [float(x) for x in self.pad], dtype=torch.float32)

    @staticmethod
    def from_tensor(t: torch.Tensor) -> "Degradation":
        x = [float(v) for v in t.tolist()]
        d = Degradation(**dict(zip(DEG_FIELDS, x)), pad=tuple(x[len(DEG_FIELDS):]))
        d.hue_band = int(round(d.hue_band))
        return d

    def develop_json(self) -> dict:
        """The global part, as a `Develop` the render reads."""
        f = self.flatten
        master = [] if f == 0 else [[x, x * (1 - f) + 0.5 * f] for x in (0.0, 0.25, 0.5, 0.75, 1.0)]
        hsl = [{} for _ in HSL_BANDS]
        if self.hue_shift:
            hsl[self.hue_band] = {"h": self.hue_shift}
        return {
            "basic": {k: getattr(self, k) for k in ["exposure", "temperature", "tint", "contrast", "highlights", "shadows", "vibrance", "saturation"]},
            "curve": {"master": master},
            "hsl": hsl,
            "effects": {"vignette": {"amount": self.vignette, "midpoint": 0.5, "feather": 0.5}},
        }

DEG_FIELDS = [f.name for f in fields(Degradation) if f.name != "pad"]
DEG_LEN = len(DEG_FIELDS) + 4
TILT = DEG_FIELDS.index("tilt")
PAD = slice(len(DEG_FIELDS), DEG_LEN)


def fit(w: float, h: float) -> tuple[int, int]:
    """A size no bigger than EDGE on its long side, never enlarged."""
    s = min(1.0, EDGE / max(w, h))
    return max(1, round(w * s)), max(1, round(h * s))


# ---- Batched on the device -----------------------------------------------------------------
# A batch is `final` (B, 3, E, E), each photo from the top-left corner of its canvas, `size`
# (B, 2) its width and height, and `deg` (B, DEG_LEN) the damage drawn for it.


def _mask(w: torch.Tensor, h: torch.Tensor, edge: int) -> torch.Tensor:
    """(B, 1, E, E): 1 inside each photo's top-left `w × h`."""
    r = torch.arange(edge, device=w.device, dtype=w.dtype)
    return ((r.view(1, 1, edge) < w.view(-1, 1, 1)) & (r.view(1, edge, 1) < h.view(-1, 1, 1))).unsqueeze(1).to(w.dtype)


def _reflect(x: torch.Tensor, size: torch.Tensor) -> torch.Tensor:
    """Source pixel coordinates reflected at a photo's edges and clamped, as `grid_sample`'s
    reflection padding does for an image of that size (align_corners=False)."""
    size = size.view(-1, 1, 1)
    x = (x + 0.5).abs()
    flips = torch.floor(x / size)
    extra = x - flips * size
    x = torch.where(torch.remainder(flips, 2) == 0, extra, size - extra) - 0.5
    return torch.minimum(x.clamp(min=0.0), size - 1)


def _resample(img: torch.Tensor, w: torch.Tensor, h: torch.Tensor, sx: torch.Tensor, sy: torch.Tensor) -> torch.Tensor:
    """Each row's photo (its top-left `w × h`) sampled bilinearly at pixel coordinates `sx`, `sy`
    (B, E, E): one `grid_sample` for the batch, the canvas around a photo never read."""
    edge = img.shape[-1]
    gx = (_reflect(sx, w) + 0.5) / edge * 2 - 1
    gy = (_reflect(sy, h) + 0.5) / edge * 2 - 1
    return F.grid_sample(img, torch.stack([gx, gy], dim=-1).to(img.dtype), mode="bilinear", padding_mode="zeros", align_corners=False)


def _develop(deg: torch.Tensor) -> dict:
    """`Degradation.develop_json` for a batch, as `render.from_develop_json` reads it (without
    the masks: the damage has none)."""
    b = deg.shape[0]
    col = lambda k: deg[:, DEG_FIELDS.index(k)]
    zero = deg.new_zeros(b)
    d = {k: col(k) for k in ["exposure", "temperature", "tint", "contrast", "highlights", "shadows", "vibrance", "saturation"]}
    d["whites"] = d["blacks"] = zero
    f = col("flatten").unsqueeze(1)
    d["curve_master"] = deg.new_tensor(CURVE_X) * (1 - f) + 0.5 * f
    d["curve_rgb"] = deg.new_tensor([0.0, 0.5, 1.0]).expand(b, 3, 3).contiguous()
    hsl = deg.new_zeros(b, len(HSL_BANDS), 3)
    hsl[torch.arange(b, device=deg.device), col("hue_band").round().long(), 0] = col("hue_shift")
    d["hsl"] = hsl
    d["grade"] = deg.new_zeros(b, 3, 3)
    d["grade_blending"] = deg.new_full((b,), 0.5)
    d["grade_balance"] = zero
    d["vignette"] = torch.stack([col("vignette"), deg.new_full((b,), 0.5), deg.new_full((b,), 0.5)], dim=1)
    return d


def geometry(size: torch.Tensor, deg: torch.Tensor) -> dict:
    """What the prediction should find, per row: straighten, whether to crop, the box (x, y, w,
    h) that finds the photo again in its looser frame, and the frame's aspect."""
    w, h = size[:, 0].to(deg.dtype), size[:, 1].to(deg.dtype)
    pad = deg[:, PAD]
    l, t, r, b = pad.unbind(1)
    return {
        "straighten": -deg[:, TILT],
        "crop_on": (pad > 0).any(dim=1).to(deg.dtype),
        "crop_box": torch.stack([l / (1 + l + r), t / (1 + t + b), 1 / (1 + l + r), 1 / (1 + t + b)], dim=1),
        "aspect": w * (1 + l + r) / (h * (1 + t + b)),
    }


@torch.no_grad()
def apply_batch(final: torch.Tensor, size: torch.Tensor, deg: torch.Tensor) -> dict:
    """`make_pair` for a batch, on `final`'s device: the global damage rendered once for the
    batch, the looser frame and the tilt one `grid_sample` each (rows without them pass
    through), then the damage rendered again on the reshaped photos."""
    dtype = torch.float64 if final.dtype == torch.float64 else torch.float32
    final = final.to(dtype)
    deg = deg.to(final.device, dtype)
    edge = final.shape[-1]
    w, h = size[:, 0].to(final.device, dtype), size[:, 1].to(final.device, dtype)
    develop = _develop(deg)
    mask = _mask(w, h, edge)
    target = final * mask
    aligned = render.render(target, develop, torch.stack([w, h], dim=1)) * mask

    pad = deg[:, PAD]
    l, t, r, b = (x.view(-1, 1, 1) for x in pad.unbind(1))
    loose = (pad > 0).any(dim=1)
    # The looser frame's size in pixels, rounded in Python as `fit` rounds it.
    fitted = [fit(ww * (1 + p[0] + p[2]), hh * (1 + p[1] + p[3])) for ww, hh, p in zip(w.tolist(), h.tolist(), pad.tolist())]
    w2 = torch.where(loose, torch.tensor([f[0] for f in fitted], dtype=dtype, device=final.device), w)
    h2 = torch.where(loose, torch.tensor([f[1] for f in fitted], dtype=dtype, device=final.device), h)
    px = torch.arange(edge, device=final.device, dtype=dtype) + 0.5
    sx = ((px.view(1, 1, edge) / w2.view(-1, 1, 1)) * (1 + l + r) - l) * w.view(-1, 1, 1) - 0.5
    sy = ((px.view(1, edge, 1) / h2.view(-1, 1, 1)) * (1 + t + b) - t) * h.view(-1, 1, 1) - 0.5
    shaped = _resample(target, w, h, sx.expand(-1, edge, edge), sy.expand(-1, edge, edge))
    shaped = torch.where(loose.view(-1, 1, 1, 1), shaped, target)

    # Lumen's straighten (`geometry::straighten_source`): rotated about the centre, zoomed to fill.
    tilt = deg[:, TILT]
    rad = torch.deg2rad(tilt).view(-1, 1, 1)
    s, c = torch.sin(rad), torch.cos(rad)
    z = c.abs() + s.abs() * torch.maximum(w2 / h2, h2 / w2).view(-1, 1, 1)
    cx, cy = (w2 / 2).view(-1, 1, 1), (h2 / 2).view(-1, 1, 1)
    dx, dy = (px.view(1, 1, edge) - cx) / z, (px.view(1, edge, 1) - cy) / z
    turned = _resample(shaped, w2, h2, c * dx + s * dy + cx - 0.5, -s * dx + c * dy + cy - 0.5)
    shaped = torch.where((tilt != 0).view(-1, 1, 1, 1), turned, shaped)

    in_mask = _mask(w2, h2, edge)
    degraded = render.render(shaped * in_mask, develop, torch.stack([w2, h2], dim=1))
    return {
        "input": torch.cat([encode(degraded) * in_mask, in_mask], dim=1),
        "aligned": aligned,
        "target": target,
        "mask": mask,
        **geometry(torch.stack([w, h], dim=1), deg),
    }


def realise(batch: dict) -> dict:
    """A collated batch with its synthetic rows (`kind` 0) made into pairs on their device;
    prepared pairs pass through."""
    if "deg" not in batch:
        return batch
    syn = torch.as_tensor(batch["kind"]) == 0
    if not bool(syn.any()):
        return batch
    out = apply_batch(batch["final"][syn], batch["size"][syn], batch["deg"][syn])
    batch = dict(batch)
    for k in ("input", "aligned", "target", "mask"):
        if k in batch:
            t = batch[k].clone()
            t[syn] = out[k].to(t.dtype)
            batch[k] = t
        else:
            batch[k] = out[k]
    return batch


def make_pair(final: torch.Tensor, d: Degradation) -> dict:
    """`final` is a finished photo, (3, h, w) linear with h, w ≤ EDGE: `apply_batch` for one."""
    _, h, w = final.shape
    canvas = final.new_zeros(3, EDGE, EDGE)
    canvas[:, :h, :w] = final
    out = apply_batch(canvas.unsqueeze(0), torch.tensor([[w, h]]), d.to_tensor().unsqueeze(0))
    return {
        "input": out["input"][0],
        "aligned": out["aligned"][0],
        "target": out["target"][0],
        "mask": out["mask"][0],
        "straighten": float(out["straighten"][0]),
        "crop_on": float(out["crop_on"][0]),
        "crop_box": tuple(float(x) for x in out["crop_box"][0]),
        "aspect": float(out["aspect"][0]),
    }
