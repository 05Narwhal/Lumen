"""Phase 7b probe: export CLIP's text tower and compare precisions before choosing one.

    .venv-ml/Scripts/python scripts/ml/probe_text_tower.py
"""
import os
import pathlib

import numpy as np
import onnx
import onnxruntime as ort
import open_clip
import torch
from onnxconverter_common import float16
from onnxruntime.quantization import QuantType, quantize_dynamic

ort.set_default_logger_severity(3)
OUT = pathlib.Path("build/models")
PHRASES = ["a dog on the beach", "sunset over the sea", "birthday cake with candles", "a snowy mountain",
           "people dancing at a party", "a plate of pasta", "a cat sleeping on a sofa", "an old church"]


class TextTower(torch.nn.Module):
    def __init__(self, model):
        super().__init__()
        self.model = model

    def forward(self, tokens):
        # open_clip's encode_text, except the end-of-text position is found with an argmax
        # over floats: ONNX Runtime's CPU ArgMax has no int64 kernel.
        m = self.model
        x = m.token_embedding(tokens)
        x = x + m.positional_embedding
        x = m.transformer(x, attn_mask=m.attn_mask)
        x = m.ln_final(x)
        eot = tokens.to(torch.float32).argmax(dim=-1)
        x = x[torch.arange(x.shape[0]), eot] @ m.text_projection
        return x / x.norm(dim=-1, keepdim=True)


def main():
    model, _, _ = open_clip.create_model_and_transforms("ViT-B-32", pretrained="openai", cache_dir=str(OUT))
    model.eval()
    tok = open_clip.get_tokenizer("ViT-B-32")
    with torch.no_grad():
        mine = TextTower(model)(tok(PHRASES))
        theirs = model.encode_text(tok(PHRASES))
        theirs = theirs / theirs.norm(dim=-1, keepdim=True)
    print(f"rewritten forward vs encode_text: max diff {float((mine - theirs).abs().max()):.2e}")
    fp32 = OUT / "clip-vit-b32-text-fp32.onnx"
    torch.onnx.export(TextTower(model), tok(["a photo"]), fp32, input_names=["tokens"], output_names=["embedding"],
                      dynamic_axes={"tokens": {0: "batch"}, "embedding": {0: "batch"}}, opset_version=17)
    fp16 = OUT / "clip-vit-b32-text-fp16.onnx"
    onnx.save(float16.convert_float_to_float16(onnx.load(fp32), keep_io_types=True), fp16)
    int8 = OUT / "clip-vit-b32-text-int8.onnx"
    quantize_dynamic(str(fp32), str(int8), weight_type=QuantType.QInt8, op_types_to_quantize=["MatMul", "Gemm"])

    tokens = tok(PHRASES).numpy()
    ref = ort.InferenceSession(str(fp32)).run(None, {"tokens": tokens})[0]
    for path in (fp16, int8):
        got = ort.InferenceSession(str(path)).run(None, {"tokens": tokens})[0]
        cos = (ref * got).sum(1)
        # What search needs: does each phrase still rank the same against the others?
        same_rank = all(np.argsort(-(ref @ ref.T)[i]).tolist() == np.argsort(-(got @ ref.T)[i]).tolist() for i in range(len(PHRASES)))
        print(f"{path.name}: {os.path.getsize(path) / 1e6:.1f} MB, min cosine {cos.min():.4f}, same ranking {same_rank}")
    print("tokens of the first phrase:", tokens[0][:12].tolist())


if __name__ == "__main__":
    main()
