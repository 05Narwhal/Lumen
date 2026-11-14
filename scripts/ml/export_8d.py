"""Phase 8d exports. Adds keys to prototypes.json only; nothing existing changes.

    .venv-ml/Scripts/python scripts/ml/export_8d.py --prototypes-only --models build/models
    .venv-ml/Scripts/python scripts/ml/export_8d.py --models build/models      # + the denoise model

Denoise: NAFNet width 32 trained on SIDD (megvii-research/NAFNet, MIT; SIDD, MIT), chosen
over SCUNet in Task 5 (docs/THIRD-PARTY-MODELS.md has the numbers). The architecture is
fetched at a pinned commit, the checkpoint by its SHA-256; exported to ONNX (opset 17,
dynamic H/W), weights fp16 with fp32 input and output, and checked against PyTorch.

`scene:*`: what Auto reads a photo as (develop spec §4 step 2), which bends its prior — a
sunset stays warm, a night stays dark. `scene:daylight` gives an ordinary photo somewhere to
go in the softmax. Embedded with the same OpenAI CLIP ViT-B/32 text tower as the 7a/7b
prototypes (`t / t.norm()`, six decimals).
"""
import argparse
import hashlib
import json
import pathlib
import sys
import urllib.request

import numpy as np
import onnx
import onnxruntime as ort
import open_clip
import torch
from onnxconverter_common import float16

REPO = pathlib.Path(__file__).resolve().parents[2]
ML = REPO / "src-tauri" / "crates" / "photos-ml"

SCENES = {
    "sunset": "a photo of a sunset or sunrise with warm golden light",
    "night": "a photo taken at night in the dark",
    "snow": "a photo of a snowy landscape",
    "beach": "a photo of a sunny beach",
    "backlit": "a backlit photo against a bright sky",
    "portrait": "a portrait photo of a person",
    "foggy": "a photo of a foggy or hazy scene",
    "indoor": "a photo taken indoors in warm artificial light or candlelight",
    "daylight": "an ordinary photo in daylight",
}


def prototypes(models: str) -> None:
    model, _, _ = open_clip.create_model_and_transforms("ViT-B-32", pretrained="openai", cache_dir=models)
    model.eval()
    tok = open_clip.get_tokenizer("ViT-B-32")
    path = ML / "prototypes.json"
    protos = json.loads(path.read_text())
    with torch.no_grad():
        for name, phrase in SCENES.items():
            t = model.encode_text(tok([phrase]))[0]
            protos["prototypes"][f"scene:{name}"] = [round(x, 6) for x in (t / t.norm()).tolist()]
    path.write_text(json.dumps(protos))
    print("added", [f"scene:{k}" for k in SCENES])


NAFNET_COMMIT = "2b4af71ebe098a92a75910c233a3965a3e93ede4"
NAFNET_ARCH = ["NAFNet_arch.py", "arch_util.py", "local_arch.py"]
NAFNET_WEIGHTS = "https://huggingface.co/nyanko7/nafnet-models/resolve/main/NAFNet-SIDD-width32.pth"
NAFNET_SHA = "89c70e808d1783b6c07911306e106aaf0d4f7f3da8c61078b99ff7f8929a26f4"
DENOISE_ID = "nafnet-sidd-w32@1"
DENOISE_FILE = "denoise-nafnet-sidd-w32-fp16.onnx"


def fetch(url: str, to: pathlib.Path) -> None:
    if not to.exists():
        print("fetching", url)
        urllib.request.urlretrieve(url, to)  # big files: fetch with `curl -C -` on a flaky network


def sha256(p: pathlib.Path) -> str:
    h = hashlib.sha256()
    with open(p, "rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def denoise(models: str) -> None:
    out = pathlib.Path(models)
    src = out / "nafnet-src"
    src.mkdir(parents=True, exist_ok=True)
    for name in NAFNET_ARCH:
        fetch(f"https://raw.githubusercontent.com/megvii-research/NAFNet/{NAFNET_COMMIT}/basicsr/models/archs/{name}", src / name)
    # The repo imports through its `basicsr` package; the three files work side by side.
    for name in NAFNET_ARCH:
        p = src / name
        t = p.read_text()
        t = t.replace("from basicsr.models.archs.arch_util import", "from arch_util import")
        t = t.replace("from basicsr.models.archs.local_arch import", "from local_arch import")
        t = t.replace("from basicsr.utils import get_root_logger", "import logging\nget_root_logger = logging.getLogger")
        p.write_text(t)
    sys.path.insert(0, str(src))
    from NAFNet_arch import NAFNet

    ckpt = out / "NAFNet-SIDD-width32.pth"
    fetch(NAFNET_WEIGHTS, ckpt)
    assert sha256(ckpt) == NAFNET_SHA, "unexpected checkpoint"
    model = NAFNet(img_channel=3, width=32, middle_blk_num=12, enc_blk_nums=[2, 2, 4, 8], dec_blk_nums=[2, 2, 2, 2])
    sd = torch.load(ckpt, map_location="cpu", weights_only=True)
    model.load_state_dict(sd.get("params", sd))
    model.eval()

    fp32, half = out / "denoise-fp32.onnx", out / DENOISE_FILE
    x = torch.rand(1, 3, 512, 512)
    torch.onnx.export(model, x, fp32, opset_version=17, input_names=["image"], output_names=["denoised"],
                      dynamic_axes={"image": {2: "h", 3: "w"}, "denoised": {2: "h", 3: "w"}})
    onnx.save(float16.convert_float_to_float16(onnx.load(fp32), keep_io_types=True), half)
    fp32.unlink()

    probe = torch.rand(1, 3, 256, 320)
    with torch.no_grad():
        want = model(probe).numpy()
    got = ort.InferenceSession(str(half), providers=["CPUExecutionProvider"]).run(None, {"image": probe.numpy()})[0]
    print(f"denoise: fp16 ONNX vs PyTorch max diff {float(np.abs(got - want).max()):.5f}")
    entry = {
        "id": DENOISE_ID,
        "role": "denoise",
        "file": DENOISE_FILE,
        "bytes": half.stat().st_size,
        "sha256": sha256(half),
        "licence": "MIT (megvii-research/NAFNet code and weights; trained on SIDD, MIT)",
        "denoise": {"tile": 512, "overlap": 32, "multiple": 16, "input": "srgb"},
    }
    print(json.dumps(entry, indent=2))


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--models", default="build/models")
    ap.add_argument("--prototypes-only", action="store_true")
    args = ap.parse_args()
    prototypes(args.models)
    if args.prototypes_only:
        return
    denoise(args.models)


if __name__ == "__main__":
    main()
