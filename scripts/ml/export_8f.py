"""Phase 8f: export Lumen Facet (EoMT-S, COCO panoptic) to ONNX and sanity-check it.

    python scripts/ml/export_8f.py <out dir> [check image] [--fp16] [--base]

Model: tue-mps/coco_panoptic_eomt_small_640_2x, or with --base tue-mps/coco_panoptic_eomt_base_640_2x
(code and weights MIT; COCO panoptic data, the same footing as the shipped D-FINE-S;
docs/THIRD-PARTY-MODELS.md). Revisions pinned below.

Output `facet-eomt-<s|b>-coco.onnx` (fp32) and, with --fp16, `facet-eomt-<s|b>-coco-fp16.onnx`
(fp16 weights, fp32 inputs and outputs):
- input  `pixels`  [1,3,640,640] RGB, ImageNet mean/std, the photo scaled so its long edge is 640
  and padded with black at the bottom and right, before normalising (what EomtImageProcessor does).
- output `class_logits` [1,Q,C+1] (the last class is "no object")
- output `mask_logits`  [1,Q,H,W] (logits, a quarter of the input's size)

Each export is checked against PyTorch on a real image before it is written.
"""

import json
import sys
from pathlib import Path

import numpy as np
import onnx
import onnxruntime as ort
import torch
from onnxconverter_common import float16
from PIL import Image
from transformers import AutoImageProcessor, EomtForUniversalSegmentation

MODELS = {
    "s": ("tue-mps/coco_panoptic_eomt_small_640_2x", "10f5326794a8ce7e0e975791033ae1c13aec3767"),
    "b": ("tue-mps/coco_panoptic_eomt_base_640_2x", "1ac497947a2410fdb05279c5907c5209af37b9f1"),
}
SIZE = 640


class Wrap(torch.nn.Module):
    def __init__(self, model):
        super().__init__()
        self.model = model

    def forward(self, pixels):
        out = self.model(pixel_values=pixels)
        return out.class_queries_logits, out.masks_queries_logits


def main():
    out_dir = Path(sys.argv[1])
    out_dir.mkdir(parents=True, exist_ok=True)
    want_fp16 = "--fp16" in sys.argv
    check_image = next((a for a in sys.argv[2:] if not a.startswith("--")), None)
    size = "b" if "--base" in sys.argv else "s"
    repo, revision = MODELS[size]

    processor = AutoImageProcessor.from_pretrained(repo, revision=revision)
    model = EomtForUniversalSegmentation.from_pretrained(repo, revision=revision, attn_implementation="eager").eval()
    print("processor:", processor)
    print("labels:", len(model.config.id2label), "queries:", model.config.num_queries)
    (out_dir / "id2label.json").write_text(json.dumps(model.config.id2label, indent=0), encoding="utf-8")
    wrap = Wrap(model).eval()

    if check_image:
        pixels = processor(images=Image.open(check_image).convert("RGB"), segmentation_type="panoptic", return_tensors="pt")["pixel_values"]
    else:
        pixels = torch.rand(1, 3, SIZE, SIZE)
    print("input shape:", tuple(pixels.shape))

    fp32 = out_dir / f"facet-eomt-{size}-coco.onnx"
    torch.onnx.export(
        wrap,
        (pixels,),
        str(fp32),
        input_names=["pixels"],
        output_names=["class_logits", "mask_logits"],
        opset_version=17,
        do_constant_folding=True,
        dynamo=False,
    )
    print(f"fp32: {fp32.stat().st_size / 1e6:.1f} MB; {check(wrap, fp32, pixels)}")

    if want_fp16:
        dst = out_dir / f"facet-eomt-{size}-coco-fp16.onnx"
        onnx.save(float16.convert_float_to_float16(onnx.load(str(fp32)), keep_io_types=True), str(dst))
        print(f"fp16: {dst.stat().st_size / 1e6:.1f} MB; {check(wrap, dst, pixels)}")


def check(wrap, path, pixels):
    with torch.no_grad():
        cls, masks = wrap(pixels)
    sess = ort.InferenceSession(str(path), providers=["CPUExecutionProvider"])
    c, m = sess.run(None, {"pixels": pixels.numpy()})
    return (
        f"class shape {c.shape} max diff {np.abs(c - cls.numpy()).max():.4f}; "
        f"mask shape {m.shape} max diff {np.abs(m - masks.numpy()).max():.4f}"
    )


if __name__ == "__main__":
    main()
