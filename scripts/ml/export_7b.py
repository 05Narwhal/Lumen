"""Phase 7b models, added to what Phase 7a exported (never re-exporting 7a's files, whose
hashes the published release and the app already carry).

    .venv-ml/Scripts/python scripts/ml/export_7b.py --models build/models --faces build/faces

Needs, already in place: build/models/ViT-B-32.pt (see export_models.py),
build/models/clip-tokenizer.json (huggingface.co/openai/clip-vit-base-patch32, MIT),
build/faces/face_detection_yunet_2026may.onnx (OpenCV Zoo, MIT) and
build/faces/face_recognition_sface_2021dec.onnx (OpenCV Zoo, Apache 2.0).
"""
import argparse
import hashlib
import json
import pathlib
import shutil

import numpy as np
import onnx
import onnxruntime as ort
import open_clip
import torch
from onnxconverter_common import float16

ort.set_default_logger_severity(3)
REPO = pathlib.Path(__file__).resolve().parents[2]
ML = REPO / "src-tauri" / "crates" / "photos-ml"
FIXTURES = ML / "tests" / "fixtures"
CLIP_ID = "clip-vit-b32-fp16@1"

TAGS = [
    "beach", "sea", "lake", "river", "waterfall", "mountain", "snow", "forest", "desert", "field", "garden",
    "flowers", "sunset", "sunrise", "night sky", "city", "street", "building", "church", "castle", "bridge",
    "interior", "kitchen", "restaurant", "food", "cake", "coffee", "drinks", "party", "wedding", "concert", "stage",
    "sport", "football", "swimming", "skiing", "cycling", "hiking", "camping", "boat", "airplane", "train", "car",
    "bicycle", "dog", "cat", "horse", "bird", "baby", "child", "group of people", "selfie", "portrait", "museum",
    "painting", "fireworks", "christmas", "rain", "fog", "autumn leaves",
]
REFERENCE_PHRASES = ["a dog on the beach", "sunset over the sea", "birthday cake with candles", "Anna's first steps!"]


def sha256(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for block in iter(lambda: f.read(1 << 20), b""):
            h.update(block)
    return h.hexdigest()


def entry(path, **fields):
    return {"file": path.name, "bytes": path.stat().st_size, "sha256": sha256(path), **fields}


class TextTower(torch.nn.Module):
    """open_clip's encode_text with the end-of-text position found by an argmax over floats:
    ONNX Runtime's CPU ArgMax has no int64 kernel."""

    def __init__(self, model):
        super().__init__()
        self.model = model

    def forward(self, tokens):
        m = self.model
        x = m.token_embedding(tokens) + m.positional_embedding
        x = m.ln_final(m.transformer(x, attn_mask=m.attn_mask))
        x = x[torch.arange(x.shape[0]), tokens.to(torch.float32).argmax(dim=-1)] @ m.text_projection
        return x / x.norm(dim=-1, keepdim=True)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--models", default="build/models")
    ap.add_argument("--faces", default="build/faces")
    args = ap.parse_args()
    out, faces = pathlib.Path(args.models), pathlib.Path(args.faces)

    model, _, _ = open_clip.create_model_and_transforms("ViT-B-32", pretrained="openai", cache_dir=str(out))
    model.eval()
    tok = open_clip.get_tokenizer("ViT-B-32")
    tower = TextTower(model).eval()
    fp32 = out / "clip-vit-b32-text-fp32.onnx"
    torch.onnx.export(tower, tok(["a photo"]), fp32, input_names=["tokens"], output_names=["embedding"],
                      dynamic_axes={"tokens": {0: "batch"}, "embedding": {0: "batch"}}, opset_version=17)
    text = out / "clip-vit-b32-text-fp16.onnx"
    onnx.save(float16.convert_float_to_float16(onnx.load(fp32), keep_io_types=True), text)

    # Scene tags join the 7a prototypes; the existing ones are left exactly as they were.
    protos_path = ML / "prototypes.json"
    protos = json.loads(protos_path.read_text())
    with torch.no_grad():
        for tag in TAGS:
            t = tower(tok([f"a photo of {tag}", f"a picture of {tag}"])).mean(dim=0)
            protos["prototypes"][f"tag:{tag}"] = [round(x, 6) for x in (t / t.norm()).tolist()]
    protos_path.write_text(json.dumps(protos))

    tokens = tok(REFERENCE_PHRASES).numpy()
    emb = ort.InferenceSession(str(text)).run(None, {"tokens": tokens})[0]
    (FIXTURES / "text_reference.json").write_text(json.dumps({
        "phrases": REFERENCE_PHRASES,
        "tokens": tokens.tolist(),
        "embeddings": [[round(float(v), 6) for v in e] for e in emb],
    }))
    shutil.copy(out / "clip-tokenizer.json", FIXTURES / "clip-tokenizer.json")

    yunet = out / "face-yunet-2026may.onnx"
    sface = out / "face-sface-2021dec.onnx"
    shutil.copy(faces / "face_detection_yunet_2026may.onnx", yunet)
    shutil.copy(faces / "face_recognition_sface_2021dec.onnx", sface)

    manifest_path = REPO / "src-tauri" / "crates" / "photos-models" / "models.json"
    manifest = json.loads(manifest_path.read_text())
    manifest["models"] = [m for m in manifest["models"] if m["role"] in ("image_embedding", "beat_activation")]
    manifest["models"] += [
        entry(text, id="clip-vit-b32-text-fp16@1", role="text_embedding", licence="MIT (OpenAI CLIP weights; OpenCLIP export)",
              text={"context": 77, "pair_of": CLIP_ID}),
        entry(out / "clip-tokenizer.json", id="clip-tokenizer@1", role="tokenizer", licence="MIT (OpenAI CLIP)"),
        entry(yunet, id="yunet-2026may@1", role="face_detection", licence="MIT (YuNet, Shiqi Yu; OpenCV Zoo)",
              faces={"score": 0.8, "nms": 0.3, "min_side_px": 24}),
        entry(sface, id="sface-2021dec@1", role="face_embedding", licence="Apache-2.0 (SFace; OpenCV Zoo)",
              faces={"size": 112, "same_person": 0.363}),
    ]
    manifest_path.write_text(json.dumps(manifest, indent=2))
    total = sum(m["bytes"] for m in manifest["models"])
    print(f"text tower {text.stat().st_size / 1e6:.1f} MB; {len(TAGS)} tags; models {total / 1e6:.1f} MB in {len(manifest['models'])} files")
    print("reference tokens:", tokens[3][:10].tolist())


if __name__ == "__main__":
    main()
