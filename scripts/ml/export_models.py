"""One-off export of Lumen's Phase 7a models (docs/superpowers/plans/2026-09-22-phase-7a-ml-runtime-and-creative.md, Task 1).

    py -3.11 -m venv .venv-ml
    .venv-ml/Scripts/pip install -r scripts/ml/requirements.txt   # see the note there about torch
    .venv-ml/Scripts/pip install --no-deps -e build/beat_this       # git clone https://github.com/CPJKU/beat_this build/beat_this
    .venv-ml/Scripts/python scripts/ml/export_models.py --out build/models [--release URL]

Writes the ONNX files to --out; the manifest, prototypes and test fixtures into the repo.
Put LAION's `sa_0_4_vit_b_32_linear.pth` (github.com/LAION-AI/aesthetic-predictor, MIT) in --out first.
"""
import argparse
import hashlib
import json
import math
import pathlib

import numpy as np
import onnxruntime as ort
import open_clip
import torch
import onnx
from onnxconverter_common import float16

REPO = pathlib.Path(__file__).resolve().parents[2]
ML = REPO / "src-tauri" / "crates" / "photos-ml"
FIXTURES = ML / "tests" / "fixtures"

CLIP_ID = "clip-vit-b32-fp16@1"
BEAT_ID = "beat-this-small@1"
CLIP_MEAN = [0.48145466, 0.4578275, 0.40821073]
CLIP_STD = [0.26862954, 0.26130258, 0.27577711]

# Verified against beat_this/preprocessing.py LogMelSpect (torchaudio MelSpectrogram with
# mel_scale="slaney", norm=None, normalized="frame_length", power=1, centred reflect-padded
# periodic Hann frames) and inference.py (chunks of 1500 frames, border 6).
BEAT_AUDIO = {
    "sample_rate": 22050, "n_fft": 1024, "hop": 441, "n_mels": 128,
    "fmin": 30.0, "fmax": 11000.0, "mel_scale": "slaney", "norm": "none", "frame_length_norm": True,
    "power": 1.0, "log_multiplier": 1000.0,
    "chunk_frames": 1500, "border_frames": 6,
    "beat_output": "beat", "downbeat_output": "downbeat",
}

# The phrases 7a compares photographs against (spec §7, §8). Several phrasings per prototype
# are averaged: one sentence is a noisy sample of what a concept looks like to CLIP.
PROTOTYPES = {
    "mood:happy": ["a photo that feels happy", "a joyful moment", "people laughing and having fun"],
    "mood:calm": ["a photo that feels calm", "a peaceful quiet scene", "a serene landscape"],
    "mood:romantic": ["a photo that feels romantic", "a couple in love", "a romantic evening"],
    "mood:epic": ["a photo that feels epic", "a vast dramatic landscape", "a breathtaking view"],
    "mood:melancholic": ["a photo that feels melancholic", "a lonely rainy day", "a wistful moment"],
    "mood:energetic": ["a photo that feels energetic", "people dancing at a party", "an action sport"],
    "mood:dreamy": ["a photo that feels dreamy", "soft golden light", "a hazy dreamlike scene"],
    "mood:playful": ["a photo that feels playful", "children playing", "a funny moment with a pet"],
    "junk:screenshot": ["a screenshot of a phone screen", "a screenshot of a computer"],
    "junk:document": ["a photo of a document", "a photo of a receipt", "a photo of a page of text"],
    "junk:whiteboard": ["a photo of a whiteboard", "a photo of slides on a projector"],
    "junk:blurry": ["a blurry photo", "an out of focus photo", "a badly blurred picture"],
    "junk:accidental": ["an accidental photo of the ground", "a photo taken in a pocket", "a black photo"],
    "good": ["a good photograph", "a beautiful photograph", "a well composed photo"],
}


def sha256(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for block in iter(lambda: f.read(1 << 20), b""):
            h.update(block)
    return h.hexdigest()


def gradient_image():
    img = np.zeros((224, 224, 3), dtype=np.uint8)
    img[..., 0] = np.arange(224)[None, :]
    img[..., 1] = np.arange(224)[:, None]
    img[..., 2] = 128
    return img


class ImageTower(torch.nn.Module):
    def __init__(self, model):
        super().__init__()
        self.model = model

    def forward(self, pixels):
        v = self.model.encode_image(pixels)
        return v / v.norm(dim=-1, keepdim=True)


def export_clip(out):
    # A pre-fetched ViT-B-32.pt in --out is used as the cache (open_clip checks its SHA-256).
    model, _, _ = open_clip.create_model_and_transforms("ViT-B-32", pretrained="openai", cache_dir=str(out))
    model.eval()
    fp32 = out / "clip-vit-b32-image-fp32.onnx"
    half = out / "clip-vit-b32-image-fp16.onnx"
    torch.onnx.export(ImageTower(model), torch.zeros(1, 3, 224, 224), fp32, input_names=["pixels"],
                      output_names=["embedding"], dynamic_axes={"pixels": {0: "batch"}, "embedding": {0: "batch"}},
                      opset_version=17)
    # fp16, not int8: dynamic int8 moved embeddings to cosine 0.970 of fp32 (per-channel: 0.39),
    # too far for thresholds at 0.85 and 0.96. fp16 is exact to four places at half the size.
    # Measured on the dev machine: 18.8 img/s in batches of 16 on 4 threads.
    onnx.save(float16.convert_float_to_float16(onnx.load(fp32), keep_io_types=True), half)

    tok = open_clip.get_tokenizer("ViT-B-32")
    protos = {}
    with torch.no_grad():
        for name, phrases in PROTOTYPES.items():
            t = model.encode_text(tok(phrases))
            t = t / t.norm(dim=-1, keepdim=True)
            m = t.mean(dim=0)
            protos[name] = [round(x, 6) for x in (m / m.norm()).tolist()]
    (ML / "prototypes.json").write_text(json.dumps({"model": CLIP_ID, "prototypes": protos}))

    # How far quantisation moved the embedding, and the reference the Rust test is held to.
    x = ((gradient_image() / 255.0 - np.array(CLIP_MEAN)) / np.array(CLIP_STD)).transpose(2, 0, 1)[None].astype(np.float32)
    e32 = ort.InferenceSession(str(fp32)).run(None, {"pixels": x})[0][0]
    e16 = ort.InferenceSession(str(half)).run(None, {"pixels": x})[0][0]
    print(f"clip: fp16 vs fp32 cosine {float(np.dot(e32, e16)):.4f}")
    (FIXTURES / "clip_reference.json").write_text(json.dumps({"embedding": [round(float(v), 6) for v in e16]}))
    return {"id": CLIP_ID, "role": "image_embedding", "file": half.name, "bytes": half.stat().st_size,
            "sha256": sha256(half), "licence": "MIT (OpenAI CLIP weights; OpenCLIP export)",
            "input": {"size": 224, "mean": CLIP_MEAN, "std": CLIP_STD}}


def aesthetic_head(out):
    head = out / "sa_0_4_vit_b_32_linear.pth"
    if head.exists():
        sd = torch.load(head, map_location="cpu")
        # LAION scores run roughly 1..10; offset/scale map 3..8 onto 0..1.
        return {"weights": [round(float(v), 7) for v in sd["weight"][0]], "bias": float(sd["bias"][0]),
                "offset": 3.0, "scale": 5.0, "source": "LAION aesthetic-predictor sa_0_4_vit_b_32_linear (MIT)"}
    p = json.loads((ML / "prototypes.json").read_text())["prototypes"]
    d = np.array(p["good"]) - np.array(p["junk:blurry"])
    return {"weights": d.tolist(), "bias": 0.0, "offset": -0.1, "scale": 0.2, "source": "prototype difference"}


def reference_signal():
    """A 440 Hz tone with a click every half second, 3 s. Rust regenerates it from this formula."""
    sr = BEAT_AUDIO["sample_rate"]
    n = np.arange(sr * 3)
    y = 0.3 * np.sin(2 * math.pi * 440 * n / sr)
    y[(n % (sr // 2)) < 64] += 0.8
    return y.astype(np.float32)


class BeatWrap(torch.nn.Module):
    def __init__(self, m):
        super().__init__()
        self.m = m

    def forward(self, spect):
        o = self.m(spect)
        return torch.sigmoid(o["beat"]), torch.sigmoid(o["downbeat"])


def export_beat(out):
    from beat_this.inference import load_model
    from beat_this.preprocessing import LogMelSpect

    local = out / "small0.ckpt"
    model = load_model(str(local) if local.exists() else "small0", device="cpu").eval()
    wrap = BeatWrap(model).eval()
    path = out / "beat-this-small.onnx"
    dummy = torch.zeros(1, BEAT_AUDIO["chunk_frames"], BEAT_AUDIO["n_mels"])
    torch.onnx.export(wrap, dummy, path, input_names=["spect"], output_names=["beat", "downbeat"],
                      dynamic_axes={"spect": {1: "frames"}, "beat": {1: "frames"}, "downbeat": {1: "frames"}},
                      opset_version=17)

    spect = LogMelSpect()(torch.tensor(reference_signal()))
    with torch.no_grad():
        b_torch, _ = wrap(spect[None])
    b_onnx, d_onnx = ort.InferenceSession(str(path)).run(None, {"spect": spect[None].numpy()})
    print(f"beat: max |torch - onnx| {float(np.abs(b_torch.numpy() - b_onnx).max()):.2e}, frames {spect.shape[0]}")
    (FIXTURES / "mel_reference.json").write_text(json.dumps({
        "log_mel": [[round(float(v), 5) for v in row] for row in spect.numpy()],
        "beat": [round(float(v), 5) for v in b_onnx[0]],
        "downbeat": [round(float(v), 5) for v in d_onnx[0]],
    }))
    return {"id": BEAT_ID, "role": "beat_activation", "file": path.name, "bytes": path.stat().st_size,
            "sha256": sha256(path), "licence": "MIT (Beat This!, CPJKU; code and published weights)",
            "audio": BEAT_AUDIO}


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--out", default="build/models")
    ap.add_argument("--release", default="https://github.com/05Narwhal/lumen-models/releases/download/models-v1/")
    args = ap.parse_args()
    out = pathlib.Path(args.out)
    out.mkdir(parents=True, exist_ok=True)
    FIXTURES.mkdir(parents=True, exist_ok=True)
    clip = export_clip(out)
    clip["aesthetic"] = aesthetic_head(out)
    beat = export_beat(out)
    manifest = {"version": 1, "release": args.release, "models": [clip, beat]}
    (REPO / "src-tauri" / "crates" / "photos-models" / "models.json").write_text(json.dumps(manifest, indent=2))
    total = sum(m["bytes"] for m in manifest["models"])
    print(f"models: {total / 1e6:.1f} MB in {len(manifest['models'])} files (budget 250 MB)")


if __name__ == "__main__":
    main()
