"""Round 24 W2: GeoCLIP to ONNX for Lumen Wayfinder 1.0, plus its gallery and a parity reference.

Run in WSL, in a venv with torch (CPU is enough), geoclip, transformers, onnx, onnxruntime,
onnxconverter-common and blake3:

    ~/venv-wayfinder/bin/python scripts/ml/export_wayfinder.py --out /mnt/c/lumen-train/wayfinder \
        [--fixtures src-tauri/crates/photos-ml/tests/fixtures/wayfinder]

GeoCLIP (Vivanco et al., NeurIPS 2023; code and weights MIT, trained on MP-16) is the model;
nothing here trains anything. Three files come out:

- `wayfinder-image-fp16.onnx`: CLIP ViT-L/14 vision tower + visual projection + GeoCLIP's image
  MLP, then L2 normalisation. Input `pixel_values` [N,3,224,224] float32 (CLIP preprocessing:
  resize the short side to 224 bicubic, centre crop, (x/255-mean)/std with CLIP's mean and std),
  output `embedding` [N,512] float32, unit length. Weights fp16, input and output fp32.
- `wayfinder-location.onnx`: GeoCLIP's location encoder (fp32: the random Fourier features at
  sigma 256 lose too much in fp16). Input `latlon` [N,2] float32 degrees, output `embedding`
  [N,512] float32, unit length. Lets the plugin add gallery points of its own.
- `wayfinder-gallery.bin`: GeoCLIP's 100k-coordinate gallery run through the location encoder
  once. Little-endian: magic `LWVG`, u32 version (1), u32 rows, u32 dim, u32 dtype (1 = f16);
  then rows x dim f16 (unit-length embeddings); then rows x 2 f32 (lat, lon).

`wayfinder_reference.json` (with `--fixtures`) holds, for each image in that folder, PyTorch's
embedding and top-5 gallery places: what `tests/wayfinder_model.rs` holds the ONNX to. The
script prints sizes, SHA-256 and blake3 of each file, and GeoCLIP's logit scale (the softmax
temperature, exp(logit_scale) ~ 100). It only adds files.
"""

import argparse
import hashlib
import json
import math
import struct
from pathlib import Path

import geoclip
import numpy as np
import onnx
import onnxruntime as ort
import torch
import torch.nn.functional as F
from blake3 import blake3
from geoclip.model.image_encoder import _extract_image_features
from onnxconverter_common import float16
from PIL import Image

IMAGE_FILE = "wayfinder-image-fp16.onnx"
LOCATION_FILE = "wayfinder-location.onnx"
GALLERY_FILE = "wayfinder-gallery.bin"
MAGIC = b"LWVG"
OPSET = 17  # what the other exports use and ort 2.0.0-rc.10 loads everywhere


class ImageHead(torch.nn.Module):
    def __init__(self, enc):
        super().__init__()
        self.clip = enc.CLIP
        self.mlp = enc.mlp

    def forward(self, pixel_values):
        x = _extract_image_features(self.clip.get_image_features(pixel_values=pixel_values))
        return F.normalize(self.mlp(x), dim=1)


class LocationHead(torch.nn.Module):
    def __init__(self, enc):
        super().__init__()
        self.enc = enc

    def forward(self, latlon):
        return F.normalize(self.enc(latlon), dim=1)


def digests(path: Path) -> dict:
    data = path.read_bytes()
    return {
        "file": path.name,
        "bytes": len(data),
        "sha256": hashlib.sha256(data).hexdigest(),
        "blake3": blake3(data).hexdigest(),
    }


def drop_identity_casts(model: onnx.ModelProto):
    """The tracer leaves `Cast(to=float32)` on the float32 input. The fp16 converter merges that
    cast with the one it adds for `keep_io_types` and drops both, so the first Conv gets a float32
    input and fp16 weights. Removing the no-op first avoids it."""
    graph = model.graph
    inputs = {i.name for i in graph.input}
    rename = {}
    for node in list(graph.node):
        if node.op_type == "Cast" and node.input[0] in inputs:
            to = next(a.i for a in node.attribute if a.name == "to")
            if to == onnx.TensorProto.FLOAT:
                rename[node.output[0]] = node.input[0]
                graph.node.remove(node)
    for node in graph.node:
        for i, name in enumerate(node.input):
            if name in rename:
                node.input[i] = rename[name]
    assert rename, "expected the tracer's input cast; the export changed"


def export_image(model, out: Path) -> Path:
    head = ImageHead(model.image_encoder).eval()
    fp32 = out / "wayfinder-image-fp32.onnx"
    x = torch.randn(2, 3, 224, 224)
    with torch.no_grad():
        torch.onnx.export(
            head, x, str(fp32), opset_version=OPSET, input_names=["pixel_values"],
            output_names=["embedding"], dynamic_axes={"pixel_values": {0: "batch"}, "embedding": {0: "batch"}},
            dynamo=False,
        )
    half = out / IMAGE_FILE
    graph = onnx.load(str(fp32), load_external_data=True)
    drop_identity_casts(graph)
    onnx.save(float16.convert_float_to_float16(graph, keep_io_types=True), str(half))
    # Parity with PyTorch on random input before it counts as exported.
    sess = ort.InferenceSession(str(half), providers=["CPUExecutionProvider"])
    with torch.no_grad():
        want = head(x).numpy()
    got = sess.run(None, {"pixel_values": x.numpy()})[0]
    cos = (want * got).sum(1)
    assert cos.min() > 0.995, f"image export drifted: cosine {cos}"
    print(f"image encoder: min cosine to PyTorch {cos.min():.5f}")
    for side in out.glob("wayfinder-image-fp32*"):
        side.unlink()
    return half


def export_location(model, out: Path) -> Path:
    head = LocationHead(model.location_encoder).eval()
    path = out / LOCATION_FILE
    x = torch.tensor([[48.8584, 2.2945], [-33.8568, 151.2153]], dtype=torch.float32)
    real_deg2rad = torch.deg2rad
    torch.deg2rad = lambda t: t * (math.pi / 180.0)  # opset 17 has no deg2rad
    try:
        with torch.no_grad():
            torch.onnx.export(
                head, x, str(path), opset_version=OPSET, input_names=["latlon"], output_names=["embedding"],
                dynamic_axes={"latlon": {0: "batch"}, "embedding": {0: "batch"}}, dynamo=False,
            )
    finally:
        torch.deg2rad = real_deg2rad
    sess = ort.InferenceSession(str(path), providers=["CPUExecutionProvider"])
    pts = model.gps_gallery[:512]
    with torch.no_grad():
        want = head(pts).numpy()
    got = sess.run(None, {"latlon": pts.numpy()})[0]
    cos = (want * got).sum(1)
    assert cos.min() > 0.9999, f"location export drifted: cosine {cos.min()}"
    print(f"location encoder: min cosine to PyTorch {cos.min():.6f}")
    return path


def write_gallery(model, out: Path) -> tuple[Path, np.ndarray]:
    head = LocationHead(model.location_encoder).eval()
    gps = model.gps_gallery
    chunks = []
    with torch.no_grad():
        for i in range(0, len(gps), 4096):
            chunks.append(head(gps[i : i + 4096]).numpy())
    emb = np.concatenate(chunks).astype(np.float32)
    rows, dim = emb.shape
    path = out / GALLERY_FILE
    with open(path, "wb") as f:
        f.write(MAGIC)
        f.write(struct.pack("<IIII", 1, rows, dim, 1))
        f.write(emb.astype("<f2").tobytes())
        f.write(gps.numpy().astype("<f4").tobytes())
    print(f"gallery: {rows} x {dim}")
    return path, emb


def reference(model, emb: np.ndarray, fixtures: Path, out: Path):
    gps = model.gps_gallery.numpy()
    items = []
    for img in sorted(p for p in fixtures.iterdir() if p.suffix.lower() in {".jpg", ".jpeg", ".png"}):
        with Image.open(img) as im:
            x = model.image_encoder.preprocess_image(im.convert("RGB"))
        with torch.no_grad():
            e = ImageHead(model.image_encoder).eval()(x).numpy()[0]
        sims = emb @ e
        top = np.argsort(-sims)[:5]
        items.append({
            "file": img.name,
            "embedding": [float(v) for v in e],
            "top": [{"lat": float(gps[i][0]), "lon": float(gps[i][1]), "cosine": float(sims[i])} for i in top],
        })
    ref = {"logit_scale": float(model.logit_scale.exp()), "images": items}
    (out / "wayfinder_reference.json").write_text(json.dumps(ref))
    print(f"reference: {len(items)} images -> {out / 'wayfinder_reference.json'}")


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--out", required=True)
    ap.add_argument("--fixtures")
    args = ap.parse_args()
    out = Path(args.out)
    out.mkdir(parents=True, exist_ok=True)

    model = geoclip.GeoCLIP().eval()
    print("logit scale exp:", float(model.logit_scale.exp()))
    files = [export_image(model, out), export_location(model, out)]
    gallery, emb = write_gallery(model, out)
    files.append(gallery)
    if args.fixtures:
        reference(model, emb, Path(args.fixtures), out)
    for f in files:
        print(json.dumps(digests(f)))


if __name__ == "__main__":
    main()
