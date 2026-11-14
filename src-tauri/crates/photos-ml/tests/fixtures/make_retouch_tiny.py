"""A tiny stand-in for Lumen's Retouch model, for `tests/retouch.rs` (Round 12-5).

Same ONNX contract as the real export (spec §4): inputs `image` (1,4,256,256), `clip` (1,512),
`clip_flag` (1,1); output `v` (1, len(params)). The output is
`sigmoid(linear(mean(image) ++ mean(clip) ++ flag))` (the clip pooled too, so the file
stays a few KB), fixed seed, opset 17.

Run once from the repo root with the training venv, then commit the .onnx:

    ../../.venv-train/Scripts/python src-tauri/crates/photos-ml/tests/fixtures/make_retouch_tiny.py

It writes the expected outputs for `image = 0.5` with `clip = 0`, flag 0 and with `clip = 0.1`,
flag 1 to `retouch-tiny.json`, which the Rust test reads.
"""

import json
from pathlib import Path

import torch

HERE = Path(__file__).resolve().parent
PARAMS_JSON = HERE.parents[2] / "photos-core" / "tests" / "retouch-params.json"
OUTPUTS = len(json.loads(PARAMS_JSON.read_text())["params"])


class Tiny(torch.nn.Module):
    def __init__(self):
        super().__init__()
        self.fc = torch.nn.Linear(4 + 1 + 1, OUTPUTS)

    def forward(self, image, clip, clip_flag):
        pooled = image.mean(dim=(2, 3))
        return torch.sigmoid(self.fc(torch.cat([pooled, clip.mean(dim=1, keepdim=True), clip_flag], dim=1)))


def main():
    torch.manual_seed(12)
    model = Tiny().eval()
    image = torch.full((1, 4, 256, 256), 0.5)
    zeros, flag0 = torch.zeros(1, 512), torch.zeros(1, 1)
    out = HERE / "retouch-tiny.onnx"
    torch.onnx.export(
        model, (image, zeros, flag0), str(out), opset_version=17, dynamo=False,
        input_names=["image", "clip", "clip_flag"], output_names=["v"],
    )
    with torch.no_grad():
        a = model(image, zeros, flag0)[0].tolist()
        b = model(image, torch.full((1, 512), 0.1), torch.ones(1, 1))[0].tolist()
    (HERE / "retouch-tiny.json").write_text(json.dumps({"outputs": OUTPUTS, "plain": a, "with_clip": b}))
    print(f"outputs = {OUTPUTS}, {out.stat().st_size} bytes")


if __name__ == "__main__":
    main()
