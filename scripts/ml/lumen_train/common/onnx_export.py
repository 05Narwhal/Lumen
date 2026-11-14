"""ONNX export with a parity check: the file onnxruntime runs must give what PyTorch gives."""

import copy
import hashlib
from pathlib import Path

import numpy as np
import onnxruntime as ort
import torch


def export(model: torch.nn.Module, example: tuple[torch.Tensor, ...], path: Path | str, input_names: list[str], output_names: list[str], opset: int = 17, tolerance: float = 1e-4) -> dict:
    path = Path(path)
    path.parent.mkdir(parents=True, exist_ok=True)
    model = copy.deepcopy(model).eval().cpu()  # the caller's model keeps its device and mode
    example = tuple(t.cpu().float() for t in example)
    axes = {n: {0: "batch"} for n in [*input_names, *output_names]}
    torch.onnx.export(model, example, str(path), input_names=input_names, output_names=output_names, dynamic_axes=axes, opset_version=opset, dynamo=False)
    with torch.no_grad():
        want = model(*example)
    want = [want] if isinstance(want, torch.Tensor) else list(want)
    session = ort.InferenceSession(str(path), providers=["CPUExecutionProvider"])
    got = session.run(output_names, {n: t.numpy() for n, t in zip(input_names, example)})
    diff = max(float(np.abs(g - w.numpy()).max()) for g, w in zip(got, want))
    if diff > tolerance:
        raise RuntimeError(f"onnxruntime differs from PyTorch by {diff:.2e} (> {tolerance:.0e})")
    data = path.read_bytes()
    return {"path": str(path), "bytes": len(data), "sha256": hashlib.sha256(data).hexdigest(), "max_abs_diff": diff}
