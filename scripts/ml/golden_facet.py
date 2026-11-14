"""Phase 8f: the fixtures Lumen Facet's Rust port is held to. PUBLIC photos only.

    python scripts/ml/golden_facet.py <onnx> <originals dir> <manifest.jsonl> <fixtures dir>

Writes into <fixtures dir>:
- `<name>.jpg`: three public photos, at most 800 px, for `tests/facet_model.rs` (needs the model).
- `preprocess.png` and `preprocess.json`: a lossless photo and pixels sampled from what
  `EomtImageProcessor` makes of it, for `facet.rs`'s unit test (no model). A JPEG would measure the
  two decoders instead of the resize.
- `synthetic.json`: small made-up logits with what `eval_facet.py` makes of them, for `facet.rs`'s
  unit tests (no model).
- `golden.json`: what the model and `eval_facet.py` make of the three photos on the CPU.
"""

import json
import sys
from pathlib import Path

import numpy as np
import onnxruntime as ort
from PIL import Image
from transformers import AutoImageProcessor

sys.path.insert(0, str(Path(__file__).parent))
import eval_facet as ef  # noqa: E402

REPO = "tue-mps/coco_panoptic_eomt_small_640_2x"
PHOTOS = {"crew": 130335077, "valley": 122455417, "car": 191655451}
SKY = 119
LABELS = {0: "person", 2: "car", 15: "cat", 119: "sky-other-merged", 125: "grass-merged"}
MEAN, STD = ef.MEAN, ef.STD


def weights_u8(mask):
    return np.rint(np.clip(mask, 0, 1) * 255).astype(np.uint8)


def rle(mask):
    """Run lengths of weights >= 128, row by row, starting with a run of 0s."""
    flat = (weights_u8(mask) >= 128).ravel()
    runs, value, n = [], False, 0
    for v in flat:
        if v == value:
            n += 1
        else:
            runs.append(n)
            value, n = v, 1
    runs.append(n)
    return runs


def check_names(names):
    assert names[0] == "person" and names[SKY].startswith("sky"), "person and sky moved"
    assert all(names[k] in ef.LIVING for k in [0, *range(14, 24)]), "the living labels moved"
    assert {k for k, v in names.items() if v in ef.LIVING} == {0, *range(14, 24)}, "the living labels moved"
    assert [k for k, v in names.items() if v.startswith("sky")] == [SKY], "more than one sky"
    assert names[79] == "toothbrush" and names[80] != "toothbrush", "things no longer end at 79"


def write_photos(originals, manifest, out):
    titles = {}
    with open(manifest, encoding="utf-8") as f:
        for line in f:
            r = json.loads(line)
            if r["pageid"] in PHOTOS.values():
                assert r.get("kept") and r["source"] == "wikimedia-cc0-pd", r
                titles[r["pageid"]] = r["title"]
    for name, pageid in PHOTOS.items():
        img = Image.open(originals / f"{pageid}.jpg").convert("RGB")
        img.thumbnail((800, 800), Image.LANCZOS)
        img.save(out / f"{name}.jpg", quality=88)
        print(f"{name}.jpg {img.size} {titles[pageid]}")
    return titles


def write_preprocess(originals, out):
    img = Image.open(originals / f"{PHOTOS['car']}.jpg").convert("RGB")
    img.thumbnail((720, 720), Image.LANCZOS)
    img.save(out / "preprocess.png", optimize=True)
    img = Image.open(out / "preprocess.png").convert("RGB")
    processor = AutoImageProcessor.from_pretrained(REPO)
    print("processor:", type(processor).__name__)
    x = processor(images=img, return_tensors="pt")["pixel_values"][0].numpy()
    ours, used = ef.to_input(img)
    diff = np.abs(x - ours[0]) * STD[:, None, None] * 255.0
    print(f"preprocess: processor vs eval_facet in pixel levels: max {diff.max():.2f}, mean {diff.mean():.3f}")
    assert x.shape == (3, ef.SIZE, ef.SIZE), x.shape
    # Back to 0..255, so the test's bar reads in pixel levels.
    levels = (x.transpose(1, 2, 0) * STD + MEAN) * 255.0
    rng = np.random.default_rng(8)
    ys = rng.integers(0, used[1], 1500)
    xs = rng.integers(0, used[0], 1500)
    samples = [[int(a), int(b), *[round(float(v), 2) for v in levels[b, a]]] for a, b in zip(xs, ys)]
    json_out = {"width": img.width, "height": img.height, "used": list(used), "samples": samples}
    (out / "preprocess.json").write_text(json.dumps(json_out, separators=(",", ":")), encoding="utf-8")


def blob(xx, yy, cx, cy, r, rng):
    d = np.hypot(xx - cx, yy - cy) / r
    return (6.0 * (1.0 - d) + rng.normal(0, 0.4, xx.shape)).round(2)


def synthetic_case(name, rng, queries):
    """`queries`: (label or None for no object, class logit, centre x, centre y, radius)."""
    q, c, side = len(queries), 134, 24
    used = [640, 400]
    cls = np.full((q, c), -8.0, dtype=np.float32)
    yy, xx = np.mgrid[0:side, 0:side].astype(np.float32)
    masks = np.zeros((q, side, side), dtype=np.float32)
    for i, (label, logit, cx, cy, r) in enumerate(queries):
        # Against a "no object" logit of 0, so a score is about sigmoid(logit).
        cls[i, c - 1] = 0.0
        cls[i, c - 1 if label is None else label] = logit
        masks[i] = blob(xx, yy, cx, cy, r, rng)
    seg, found, soft = ef.panoptic(cls[None], masks[None], used)
    names = {k: LABELS.get(k, f"other{k}") for k in range(c)}
    rows = ef.mask_rows(seg, found, names, soft, {SKY}) if found else []
    return {
        "name": name,
        "queries": q,
        "classes": c,
        "side": side,
        "used": used,
        "cls": [round(float(v), 3) for v in cls.ravel()],
        "masks": [round(float(v), 3) for v in masks.ravel()],
        "seg": seg.ravel().tolist(),
        "found": [[lab, round(score, 5)] for lab, score in found],
        "rows": [
            {"kind": k, "instance": n, "score": round(float(s), 5), "width": m.shape[1], "height": m.shape[0], "weights": weights_u8(m).ravel().tolist()}
            for k, n, s, m in rows
        ],
    }


def write_synthetic(out):
    rng = np.random.default_rng(8)
    cases = [
        # Two people (the second well under half the first's prominence), a car, real sky, grass and
        # queries the model calls nothing. An overlap makes one of them lose pixels to another.
        synthetic_case("group", rng, [
            (0, 6.0, 12, 9, 3.0), (0, 6.0, 4, 9, 2.0), (2, 5.0, 20, 11, 2.0),
            (SKY, 7.0, 12, -3, 8.0), (125, 6.0, 12, 19, 5.0), (None, 6.0, 3, 3, 3.0),
            (None, 6.0, 20, 3, 3.0), (0, 1.0, 18, 5, 1.5),
        ]),
        # A false sky: a score of about 0.93 is under the bar. The cat outranks the car (living x2).
        synthetic_case("backdrop", rng, [
            (SKY, 2.6, 12, 0, 6.0), (2, 6.0, 5, 8, 3.2), (15, 6.0, 18, 9, 3.0),
            (125, 6.0, 12, 16, 6.0), (None, 6.0, 2, 2, 2.0),
        ]),
        # Nothing the subject could be: sky and grass only. Query 2 keeps too little of its own area.
        synthetic_case("landscape", rng, [
            (SKY, 7.0, 12, 2, 9.0), (125, 7.0, 12, 13, 7.0), (125, 1.0, 12, 11, 5.0),
        ]),
        # The model sees nothing.
        synthetic_case("empty", rng, [(None, 6.0, 12, 8, 5.0), (None, 6.0, 4, 4, 5.0)]),
    ]
    (out / "synthetic.json").write_text(json.dumps(cases, separators=(",", ":")), encoding="utf-8")
    for c in cases:
        print(c["name"], "found", c["found"], "rows", [(r["kind"], r["instance"]) for r in c["rows"]])


def write_golden(onnx_path, out, names, titles):
    s = ef.session(onnx_path, False)
    photos = []
    for name, pageid in PHOTOS.items():
        img = Image.open(out / f"{name}.jpg").convert("RGB")
        x, used = ef.to_input(img)
        cls, masks = s.run(None, {"pixels": x})
        seg, found, soft = ef.panoptic(cls, masks, used)
        rows = ef.mask_rows(seg, found, names, soft, {SKY})
        photos.append({
            "file": f"{name}.jpg",
            "pageid": pageid,
            "title": titles[pageid],
            "used": list(used),
            "found": [[lab, round(score, 4)] for lab, score in found],
            "rows": [{"kind": k, "instance": n, "score": round(float(v), 4), "width": m.shape[1], "height": m.shape[0], "rle": rle(m)} for k, n, v, m in rows],
        })
        print(name, [names[lab] for lab, _ in found], [(k, n) for k, n, _, _ in rows])
    (out / "golden.json").write_text(json.dumps(photos, separators=(",", ":")), encoding="utf-8")


def main():
    onnx_path, originals, manifest, out = sys.argv[1], Path(sys.argv[2]), sys.argv[3], Path(sys.argv[4])
    out.mkdir(parents=True, exist_ok=True)
    names = {int(k): v for k, v in json.loads(Path(onnx_path).with_name("id2label.json").read_text()).items()}
    check_names(names)
    titles = write_photos(originals, manifest, out)
    write_preprocess(originals, out)
    write_synthetic(out)
    write_golden(onnx_path, out, names, titles)


if __name__ == "__main__":
    main()
