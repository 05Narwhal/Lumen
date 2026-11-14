"""Round 4C: export the pet models to ONNX.

    PETS_EVAL=<dir with D-FINE, dfine_s_coco.pth, dinov2, dinov2_vits14_pretrain.pth> \
        python scripts/ml/export_pets.py <out dir>

- `pet-dfine-s-coco.onnx` (fp32: in fp16 it finds nothing): D-FINE-S (COCO-only weights, Apache-2.0) with its own
  post-processor. Inputs `images` [N,3,640,640] (RGB 0..1, a plain resize, no mean/std) and
  `orig_target_sizes` [N,2] (width, height); outputs `labels` [N,300] (contiguous COCO
  classes: cat 15, dog 16), `boxes` [N,300,4] (x0, y0, x1, y1 in the photo's pixels) and
  `scores` [N,300].
- `pet-dinov2-vits14-fp16.onnx` (when chosen, fp16 weights, fp32 inputs and outputs): DINOv2 ViT-S/14 (Apache-2.0). Input `crops`
  [N,3,224,224] (ImageNet mean/std), output the 384-d CLS embedding (not normalised).

Each export is checked against PyTorch on real images before it is written.
"""

import os
import sys
import types
from pathlib import Path

import numpy as np
import onnx
import onnxruntime as ort
import torch
from onnxconverter_common import float16

sys.path.insert(0, str(Path(__file__).parent))
import eval_pets  # noqa: E402

OUT = Path(sys.argv[1])
OUT.mkdir(parents=True, exist_ok=True)


def to_fp16(src: Path, dst: Path):
    model = float16.convert_float_to_float16(onnx.load(str(src)), keep_io_types=True)
    onnx.save(model, str(dst))


def export_detector(images):
    model = eval_pets.load_dfine()
    fp32 = OUT / "pet-dfine-s-fp32.onnx"
    data, size = torch.rand(1, 3, 640, 640), torch.tensor([[640, 480]])
    torch.onnx.export(
        model,
        (data, size),
        str(fp32),
        input_names=["images", "orig_target_sizes"],
        output_names=["labels", "boxes", "scores"],
        dynamic_axes={"images": {0: "N"}, "orig_target_sizes": {0: "N"}},
        opset_version=17,
        do_constant_folding=True,
    )
    # fp32: converted to fp16, D-FINE's distribution head overflows and finds nothing at all
    # (46 of 46 pets missed on the check photos). 42 MB is small beside CLIP's tower anyway.
    dst = OUT / "pet-dfine-s-coco.onnx"
    fp32.replace(dst)
    print(f"detector: {dst.name} {dst.stat().st_size / 1e6:.1f} MB; {check_detector(model, dst, images)}")


def check_detector(model, path, images):
    """Against PyTorch: the worst IoU of a pet PyTorch finds with the export's nearest box, and how many it misses."""
    sess = ort.InferenceSession(str(path), providers=["CPUExecutionProvider"])
    worst_iou, worst_score, missed, total = 1.0, 0.0, 0, 0
    for p in images:
        img = eval_pets.Image.open(p).convert("RGB")
        ref = eval_pets.detect_pets(model, img, 0.5)
        w, h = img.size
        labels, boxes, scores = sess.run(None, {"images": eval_pets.to_input(img).numpy(), "orig_target_sizes": np.array([[w, h]], dtype=np.int64)})
        species = {eval_pets.OUT_CAT: "cat", eval_pets.OUT_DOG: "dog"}
        got = [(b, s, species[int(l)]) for l, b, s in zip(labels[0], boxes[0], scores[0]) if int(l) in species and s >= 0.5]
        for r in ref:
            total += 1
            mine = [g for g in got if g[2] == r["species"]]
            best = max(mine, key=lambda g: eval_pets.iou(g[0], r["box"]), default=None)
            if best is None or eval_pets.iou(best[0], r["box"]) < 0.5:
                missed += 1
                continue
            worst_iou = min(worst_iou, eval_pets.iou(best[0], r["box"]))
            worst_score = max(worst_score, abs(best[1] - r["score"]))
    return f"on {len(images)} photos, {missed} of {total} pets missed; of the rest, worst box IoU {worst_iou:.3f}, worst score difference {worst_score:.4f}"


def export_embedder(crops):
    """DINOv2 ViT-S/14's CLS embedding of a 224×224 crop (ImageNet mean/std applied by the caller)."""
    dino = torch.hub.load(str(eval_pets.EVAL / "dinov2"), "dinov2_vits14", source="local", pretrained=False)
    dino.load_state_dict(torch.load(eval_pets.EVAL / "dinov2_vits14_pretrain.pth", map_location="cpu"))
    dino.eval()
    fp32 = OUT / "pet-dinov2-fp32.onnx"

    class Cls(torch.nn.Module):
        """One input: DINOv2's forward also takes `masks`, which would become a required input."""

        def __init__(self):
            super().__init__()
            self.dino = dino

        def forward(self, crops):
            return self.dino(crops)

    torch.onnx.export(Cls().eval(), torch.rand(1, 3, 224, 224), str(fp32), input_names=["crops"], output_names=["embedding"], dynamic_axes={"crops": {0: "N"}, "embedding": {0: "N"}}, opset_version=17, do_constant_folding=True)
    dst = OUT / "pet-dinov2-vits14-fp16.onnx"
    to_fp16(fp32, dst)
    fp32.unlink()
    sess = ort.InferenceSession(str(dst), providers=["CPUExecutionProvider"])
    mean, std = np.array([0.485, 0.456, 0.406]), np.array([0.229, 0.224, 0.225])
    worst = 1.0
    for c in crops:
        x = ((np.asarray(c.convert("RGB").resize((224, 224), eval_pets.Image.BICUBIC), dtype=np.float32) / 255.0 - mean) / std).astype(np.float32).transpose(2, 0, 1)[None]
        with torch.no_grad():
            ref = dino(torch.from_numpy(x))[0].numpy()
        got = sess.run(None, {"crops": x})[0][0]
        worst = min(worst, float(ref @ got / np.linalg.norm(ref) / np.linalg.norm(got)))
    print(f"embedder: {dst.name} {dst.stat().st_size / 1e6:.1f} MB; worst cosine to PyTorch on {len(crops)} crops {worst:.5f}")


# Round 4C: what a detected animal's crop is, as Round 4B's `face:*` (two phrasings each,
# averaged, with the same OpenAI CLIP ViT-B/32 text tower; keys only, nothing existing changes).
PET_KINDS = {
    "real": ["a photo of a real cat or dog", "a close-up photo of a pet animal"],
    "drawing": ["a drawing or cartoon of a cat or dog", "an illustration of an animal"],
    "toy": ["a stuffed toy animal or plush", "a teddy bear or a soft toy"],
    "screen": ["a cat or dog on a phone, TV or computer screen", "a photo of a screen showing an animal"],
    "print": ["a cat or dog printed on paper, a poster or a book", "a printed photo of an animal on paper"],
}

# Which animal a crop is, to overrule D-FINE when CLIP is sure (`pet_filter::species`): kept
# apart from `pet:*` so they never enter the crop check's softmax.
SPECIES = {
    "cat": ["a photo of a cat", "a photo of a kitten", "a close-up photo of a cat"],
    "dog": ["a photo of a dog", "a photo of a puppy", "a close-up photo of a dog"],
}


def export_prototypes(models_dir: Path):
    import json

    import open_clip

    model, _, _ = open_clip.create_model_and_transforms("ViT-B-32", pretrained="openai", cache_dir=str(models_dir))
    model.eval()
    tok = open_clip.get_tokenizer("ViT-B-32")
    path = Path(__file__).resolve().parents[2] / "src-tauri" / "crates" / "photos-ml" / "prototypes.json"
    protos = json.loads(path.read_text())
    with torch.no_grad():
        for kind, phrases in PET_KINDS.items():
            t = model.encode_text(tok(phrases))
            t = (t / t.norm(dim=-1, keepdim=True)).mean(dim=0)
            protos["prototypes"][f"pet:{kind}"] = [round(x, 6) for x in (t / t.norm()).tolist()]
        for kind, phrases in SPECIES.items():
            t = model.encode_text(tok(phrases))
            t = (t / t.norm(dim=-1, keepdim=True)).mean(dim=0)
            protos["prototypes"][f"species:{kind}"] = [round(x, 6) for x in (t / t.norm()).tolist()]
    path.write_text(json.dumps(protos))
    print("added", [f"pet:{k}" for k in PET_KINDS] + [f"species:{k}" for k in SPECIES])


if __name__ == "__main__":
    if "--prototypes" in sys.argv:
        export_prototypes(OUT)
        sys.exit(0)
    coco = eval_pets.COCO / "img"
    subset = eval_pets.json.loads((eval_pets.COCO / "subset.json").read_text())
    photos = [coco / f"{i:012d}.jpg" for i in subset["pets"][:40]]
    if "--embedder-only" not in sys.argv:
        export_detector(photos)
    det = eval_pets.load_dfine()
    crops = []
    for p in photos:
        img = eval_pets.Image.open(p).convert("RGB")
        crops += [eval_pets.crop_square(img, d["box"]) for d in eval_pets.detect_pets(det, img, 0.5)]
    export_embedder(crops)
