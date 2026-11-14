"""Round 13: PaddleOCR's PP-OCRv5 text models to ONNX, their Lumen dev entries, and a golden.

Run in WSL (paddle2onnx can't load on Windows), in a venv with paddlepaddle 3.2.2,
paddleocr 3.7, paddlex 3.7 (+ its paddle2onnx plugin), onnx and onnxruntime:

    PADDLE_PDX_DISABLE_MODEL_SOURCE_CHECK=True ~/venv-ocr/bin/python scripts/ml/export_13.py \
        --out /mnt/c/lumen-train/ocr-onnx \
        [--golden src-tauri/crates/photos-ml/tests/fixtures/ocr/signs.jpg] \
        [--install-dev /mnt/c/Users/<you>/AppData/Roaming/JL-Studios/Lumen/models/dev]

- `pp-ocrv5-mobile-det.onnx`: PP-OCRv5_mobile_det (Apache-2.0). Input `x` [N,3,H,W] (BGR,
  ImageNet mean/std, H and W multiples of 32), output [N,1,H,W] text probability.
- `pp-ocrv5-latin-mobile-rec.onnx`: latin_PP-OCRv5_mobile_rec (Apache-2.0). Input `x`
  [N,3,48,W] (BGR, (x/255-0.5)/0.5), output [N,T,classes] softmax over
  blank + dictionary + space.
- `ocr_detection.json` / `ocr_recognition.json`: dev-folder entries (roles `ocr_detection` /
  `ocr_recognition`) with bytes, sha256, licence, and the `ocr_det` / `ocr_rec` specs.

Each export is checked against Paddle on random inputs before it is written. Unused
initializers are stripped so ONNX Runtime does not warn about them on every session.
`--golden` runs PaddleOCR itself on an image (long side 1536) and writes `golden.json` beside
it: what `tests/ocr_model.rs` holds `OcrEngine` to. The script only adds files.
"""

import argparse
import hashlib
import json
import os
import shutil
import subprocess
import sys
from pathlib import Path

import numpy as np
import onnx
import onnxruntime as ort
import yaml

DET = "PP-OCRv5_mobile_det"
REC = "latin_PP-OCRv5_mobile_rec"
DET_FILE = "pp-ocrv5-mobile-det.onnx"
REC_FILE = "pp-ocrv5-latin-mobile-rec.onnx"
DET_ID = "pp-ocrv5-mobile-det@1"
REC_ID = "pp-ocrv5-latin-mobile-rec@1"
LICENCE = "Apache-2.0 (PaddlePaddle/PaddleOCR, PP-OCRv5)"
DET_SIDE = 960
GOLDEN_SIDE = 1536
REC_BATCH = 16


def paddle_dir(name: str) -> Path:
    """The official model's Paddle files, downloaded by PaddleX on first use."""
    home = Path(os.environ.get("PADDLE_PDX_CACHE_HOME", Path.home() / ".paddlex"))
    path = home / "official_models" / name
    if not (path / "inference.json").exists():
        from paddlex import create_model

        create_model(name)
    assert (path / "inference.json").exists(), f"{name} was not downloaded to {path}"
    return path


def convert(src: Path, work: Path) -> Path:
    out = work / src.name
    work.mkdir(parents=True, exist_ok=True)  # paddlex needs the parent to exist
    if not (out / "inference.onnx").exists():
        subprocess.run(
            ["paddlex", "--paddle2onnx", "--paddle_model_dir", str(src), "--onnx_model_dir", str(out)],
            check=True,
        )
    return out / "inference.onnx"


def strip_unused(src: Path, dst: Path):
    """Drops nodes no graph output depends on, then initializers no node reads (paddle2onnx
    leaves ~30 behind, and ONNX Runtime warns about each on every session it opens)."""
    model = onnx.load(str(src))
    needed = {o.name for o in model.graph.output}
    keep_nodes = []
    for n in reversed(model.graph.node):
        if any(o in needed for o in n.output):
            keep_nodes.append(n)
            needed |= set(n.input)
    keep_nodes.reverse()
    dead = len(model.graph.node) - len(keep_nodes)
    del model.graph.node[:]
    model.graph.node.extend(keep_nodes)
    used = {i for n in model.graph.node for i in n.input}
    for sub in (a.g for n in model.graph.node for a in n.attribute if a.type == onnx.AttributeProto.GRAPH):
        used |= {i for n in sub.node for i in n.input}
    keep = [t for t in model.graph.initializer if t.name in used]
    dropped = f"{dead} dead nodes and {len(model.graph.initializer) - len(keep)} initializers"
    del model.graph.initializer[:]
    model.graph.initializer.extend(keep)
    onnx.checker.check_model(model)
    onnx.save(model, str(dst))
    return dropped


def parity(paddle_model: Path, onnx_path: Path, shapes):
    """Max |Paddle − ONNX Runtime| over random inputs of the given shapes."""
    import paddle.inference as pi

    cfg = pi.Config(str(paddle_model / "inference.json"), str(paddle_model / "inference.pdiparams"))
    cfg.disable_gpu()
    cfg.disable_mkldnn()
    pred = pi.create_predictor(cfg)
    sess = ort.InferenceSession(str(onnx_path), providers=["CPUExecutionProvider"])
    worst = 0.0
    rng = np.random.default_rng(13)
    for shape in shapes:
        x = rng.standard_normal(shape).astype(np.float32)
        h = pred.get_input_handle(pred.get_input_names()[0])
        h.reshape(shape)
        h.copy_from_cpu(x)
        pred.run()
        want = pred.get_output_handle(pred.get_output_names()[0]).copy_to_cpu()
        got = sess.run(None, {sess.get_inputs()[0].name: x})[0]
        worst = max(worst, float(np.abs(want - got).max()))
    return worst


def sha256(path: Path) -> str:
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def dictionary(rec_dir: Path):
    cfg = yaml.safe_load((rec_dir / "inference.yml").read_text(encoding="utf-8"))
    chars = [str(c) for c in cfg["PostProcess"]["character_dict"]]
    return chars


def entry(role, ident, file, path, spec_key, spec):
    return {
        "id": ident,
        "role": role,
        "file": file,
        "bytes": path.stat().st_size,
        "sha256": sha256(path),
        "licence": LICENCE,
        spec_key: spec,
    }


def export(out: Path):
    out.mkdir(parents=True, exist_ok=True)
    work = out / "paddle2onnx"
    det_src, rec_src = paddle_dir(DET), paddle_dir(REC)

    det_onnx = out / DET_FILE
    dropped = strip_unused(convert(det_src, work), det_onnx)
    diff = parity(det_src, det_onnx, [(1, 3, 640, 960), (1, 3, 960, 736)])
    print(f"detector: {det_onnx.name} {det_onnx.stat().st_size / 1e6:.1f} MB, {dropped} dropped, max |Paddle − ORT| {diff:.2e}")
    assert diff < 1e-4, diff

    rec_onnx = out / REC_FILE
    dropped = strip_unused(convert(rec_src, work), rec_onnx)
    diff = parity(rec_src, rec_onnx, [(1, 3, 48, 320), (8, 3, 48, 640)])
    print(f"recogniser: {rec_onnx.name} {rec_onnx.stat().st_size / 1e6:.1f} MB, {dropped} dropped, max |Paddle − ORT| {diff:.2e}")
    assert diff < 1e-4, diff

    chars = dictionary(rec_src)
    classes = ort.InferenceSession(str(rec_onnx), providers=["CPUExecutionProvider"]).run(
        None, {"x": np.zeros((1, 3, 48, 320), np.float32)}
    )[0].shape[-1]
    assert classes == len(chars) + 2, f"{classes} classes for {len(chars)} characters + blank + space"

    det_spec = {"thresh": 0.3, "box_thresh": 0.6, "unclip_ratio": 1.5, "max_candidates": 1000, "min_side": 3.0}
    rec_spec = {"height": 48, "base_width": 320, "chars": chars}
    entries = {
        "ocr_detection.json": entry("ocr_detection", DET_ID, DET_FILE, det_onnx, "ocr_det", det_spec),
        "ocr_recognition.json": entry("ocr_recognition", REC_ID, REC_FILE, rec_onnx, "ocr_rec", rec_spec),
    }
    for name, e in entries.items():
        (out / name).write_text(json.dumps(e, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(f"entries: {', '.join(entries)} ({len(chars)} characters)")


def golden(image: Path):
    import cv2
    from paddleocr import PaddleOCR

    img = cv2.imread(str(image), cv2.IMREAD_COLOR)
    h, w = img.shape[:2]
    if max(w, h) != GOLDEN_SIDE:
        s = GOLDEN_SIDE / max(w, h)
        img = cv2.resize(img, (round(w * s), round(h * s)), interpolation=cv2.INTER_AREA)
        h, w = img.shape[:2]
    ocr = PaddleOCR(
        text_detection_model_name=DET,
        text_recognition_model_name=REC,
        use_doc_orientation_classify=False,
        use_doc_unwarping=False,
        use_textline_orientation=False,
        text_det_limit_side_len=DET_SIDE,
        text_det_limit_type="max",
        text_det_thresh=0.3,
        text_det_box_thresh=0.6,
        text_det_unclip_ratio=1.5,
        text_rec_score_thresh=0,
        text_recognition_batch_size=REC_BATCH,
        device="cpu",
        enable_mkldnn=False,
    )
    res = ocr.predict(img)[0]
    lines = [
        {"quad": [float(v) for v in np.asarray(q).reshape(-1)], "text": t, "conf": float(c)}
        for q, t, c in zip(res["rec_polys"], res["rec_texts"], res["rec_scores"])
    ]
    dst = image.parent / "golden.json"
    dst.write_text(json.dumps({"width": w, "height": h, "lines": lines}, ensure_ascii=False, indent=1) + "\n", encoding="utf-8")
    print(f"golden: {len(lines)} lines → {dst}")
    for line in lines:
        print(f"  {line['conf']:.3f} {line['text']!r}")


def install(out: Path, dev: Path):
    dev.mkdir(parents=True, exist_ok=True)
    for name in (DET_FILE, REC_FILE, "ocr_detection.json", "ocr_recognition.json"):
        shutil.copy2(out / name, dev / name)
    print(f"installed into {dev}")


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--out", type=Path, required=True)
    ap.add_argument("--golden", type=Path)
    ap.add_argument("--install-dev", type=Path)
    ap.add_argument("--skip-export", action="store_true", help="reuse the files already in --out")
    a = ap.parse_args()
    if not a.skip_export:
        export(a.out)
    if a.golden:
        golden(a.golden)
    if a.install_dev:
        install(a.out, a.install_dev)


if __name__ == "__main__":
    sys.exit(main())
