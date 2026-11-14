"""Real tiers at export (Round 14 spec §1): `low` is the fp32 network quantised to int8 (static
QDQ, calibrated on validation inputs) or, when that costs more than `LOW_BOUND` ΔE over `high`,
converted to fp16; `high` is the fp32 network; `ultra` runs `high`'s file five times (`tta_predict`)
and averages. Each tier is scored on the trained sources' validation photos exactly as eval
scores a checkpoint, through onnxruntime on the CPU with CLIP missing, as the app runs it."""

import tempfile
from pathlib import Path
from typing import Callable

import numpy as np
import onnx
import onnxruntime as ort
import torch

from . import params
from .dataset import PreparedDataset
from .eval import _batch, item_de

LOW_BOUND = 0.3
TTA = 5
CROP = 0.9
CLIP = 512

# Sliders that place something in the frame: ultra takes them from the full, unmirrored pass
# (a mirror or a crop moves them); every other slider is the mean of the five passes.
SPATIAL = (
    "straighten", "crop_on", "crop_x", "crop_y", "crop_w", "crop_h", "crop_ratio",
    "lin_on", "lin_fx", "lin_fy", "lin_tx", "lin_ty",
    "rad_on", "rad_cx", "rad_cy", "rad_rx", "rad_ry", "rad_feather",
)

Predict = Callable[[dict], torch.Tensor]  # a batch → its sliders (N, len(PARAMS))


def val_items(root: Path, sources: list[str], limit: int) -> list[dict]:
    """Up to `limit` batched validation items of `sources`, spread evenly over them: each source's
    `val` split, or all of it when it has no validation photo. Sources not on disk are skipped."""
    sets = []
    for s in sources:
        if not (Path(root) / s).is_dir():
            continue
        ds = PreparedDataset(root, [s], split="val", augment=False)
        if len(ds) == 0:
            ds = PreparedDataset(root, [s], split="all", augment=False)
        sets += [(ds, i) for i in range(len(ds))]
    if not sets or limit <= 0:
        return []
    pick = np.unique(np.linspace(0, len(sets) - 1, min(limit, len(sets))).round().astype(int))
    return [_batch(sets[i][0][sets[i][1]]) for i in pick]


def onnx_predict(path: Path) -> Predict:
    """The model at `path` as the app runs it: CPU, the image, CLIP zeros and flag 0."""
    session = ort.InferenceSession(str(path), providers=["CPUExecutionProvider"])

    def predict(b: dict) -> torch.Tensor:
        image = b["input"].float().numpy()
        n = image.shape[0]
        (v,) = session.run(["v"], {"image": image, "clip": np.zeros((n, CLIP), np.float32), "clip_flag": np.zeros((n, 1), np.float32)})
        return torch.from_numpy(v)

    return predict


@torch.no_grad()
def mean_de(predict: Predict, items: list[dict]) -> float | None:
    if not items:
        return None
    return sum(item_de(b, predict(b))[0] for b in items) / len(items)


def tta_inputs(image: torch.Tensor) -> torch.Tensor:
    """(B, 4, E, E) prepared inputs → (5B, 4, E, E), pass-major: the full photo, its horizontal
    mirror, then crops of 90 % of the long side (start, centre, end) moved to the top-left."""
    passes = [image.clone() for _ in range(TTA)]
    for i in range(image.shape[0]):
        x = image[i]
        w, h = int((x[3, 0] > 0.5).sum()), int((x[3, :, 0] > 0.5).sum())
        passes[1][i].zero_()
        passes[1][i][:, :h, :w] = x[:, :h, :w].flip(-1)
        long = max(w, h)
        c = round(CROP * long)
        for k, off in zip((2, 3, 4), (0, (long - c) // 2, long - c)):
            passes[k][i].zero_()
            if w >= h:
                passes[k][i][:, :h, :c] = x[:, :h, off : off + c]
            else:
                passes[k][i][:, :c, :w] = x[:, off : off + c, :w]
    return torch.cat(passes)


def tta_predict(predict: Predict) -> Predict:
    spatial = torch.tensor([p["name"] in SPATIAL for p in params.PARAMS])

    def ultra(b: dict) -> torch.Tensor:
        n = b["input"].shape[0]
        rep = lambda t: t.repeat(TTA, *([1] * (t.dim() - 1)))
        v = predict({"input": tta_inputs(b["input"]), "clip": rep(b["clip"]), "clip_flag": rep(b["clip_flag"])})
        v = v.reshape(TTA, n, -1)
        return torch.where(spatial, v[0], v.mean(0))

    return ultra


class _Calibration:
    """onnxruntime's `CalibrationDataReader`: one prepared input at a time, CLIP missing."""

    def __init__(self, inputs: list[np.ndarray]):
        self.feeds = iter([{"image": x.astype(np.float32), "clip": np.zeros((x.shape[0], CLIP), np.float32), "clip_flag": np.zeros((x.shape[0], 1), np.float32)} for x in inputs])

    def get_next(self):
        return next(self.feeds, None)


def calibration_inputs(n: int = 8) -> list[np.ndarray]:
    """Stand-in calibration when no validation photo is on disk: smooth gradients at a few levels."""
    from .prepared import EDGE, encode_input

    out = []
    for i in range(n):
        level = 0.03 * 2 ** (i % 6)
        y, x = torch.meshgrid(torch.linspace(0, 1, 170), torch.linspace(0, 1, EDGE), indexing="ij")
        lin = torch.stack([level * (0.4 + x), level * (0.4 + y), torch.full_like(x, level)])
        out.append(encode_input(lin, EDGE, 170).unsqueeze(0).numpy())
    return out


def quantize_int8(fp32: Path, out: Path, calib: list[np.ndarray]) -> None:
    """Static QDQ int8 (per-channel int8 weights, uint8 activations) calibrated on `calib`."""
    from onnxruntime.quantization import CalibrationDataReader, QuantFormat, QuantType, quantize_static
    from onnxruntime.quantization.shape_inference import quant_pre_process

    reader = type("Reader", (_Calibration, CalibrationDataReader), {})(calib or calibration_inputs())
    with tempfile.TemporaryDirectory() as tmp:
        pre = Path(tmp) / "pre.onnx"
        try:
            quant_pre_process(str(fp32), str(pre))
        except Exception:  # pre-processing only helps; quantise the plain graph without it
            pre = Path(fp32)
        quantize_static(str(pre), str(out), reader, quant_format=QuantFormat.QDQ, per_channel=True, weight_type=QuantType.QInt8, activation_type=QuantType.QUInt8)


def to_fp16(fp32: Path, out: Path) -> None:
    """Half-precision weights and maths; inputs and outputs stay float32, as the app sends them."""
    from onnxruntime.transformers.float16 import convert_float_to_float16

    onnx.save(convert_float_to_float16(onnx.load(str(fp32)), keep_io_types=True), str(out))
