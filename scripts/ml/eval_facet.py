"""Phase 8f: look at Lumen Facet's masks and time it, on PUBLIC photos only.

    python scripts/ml/eval_facet.py <onnx> <photos dir> <manifest.jsonl> <out dir> [--timing-only]

Writes contact sheets (photo, sky, background, subject, people) and prints speed on DirectML
and on 4 CPU threads: 3 warm-ups, then 7 timed runs on a real image. Photos are chosen from the
Wikimedia CC0 / public-domain set by words in their titles, with a fixed seed.

The post-processing here is what Rust will do: per query, class score x sigmoid(mask); each pixel
goes to the query with the highest product; a query keeps its pixels if its score and the share of
its own confident pixels it keeps are both high enough. Sky is the union of `sky-other-merged`.
"""

import json
import random
import re
import sys
import time
from pathlib import Path

import numpy as np
import onnxruntime as ort
from PIL import Image, ImageDraw
from scipy.ndimage import uniform_filter

SIZE = 640
MEAN = np.array([0.485, 0.456, 0.406], dtype=np.float32)
STD = np.array([0.229, 0.224, 0.225], dtype=np.float32)
SCORE_MIN = 0.5
KEEP_MIN = 0.8
# False skies score lower than real ones: two portraits' backdrops 0.93 and 0.92, a dark workshop
# ceiling 0.78, against 0.98 and 0.99 for real skies with people in the photo and 0.99 or more for
# 49 of the 52 person-free photos where sky fired (2026-11-02, with the processor's black padding).
SKY_SCORE_MIN = 0.95
# Things that make a photo's subject before anything else (COCO names).
LIVING = {"person", "bird", "cat", "dog", "horse", "sheep", "cow", "elephant", "bear", "zebra", "giraffe"}

GROUPS = {
    "sky": r"landscape|mountain|sunset|panorama|skyline|view of|lighthouse|castle|bridge",
    "people": r"portrait|woman|man |girl|boy|people|children|festival|dancer",
    "animal": r"\bdog\b|\bcat\b|horse|bird|eagle|duck|cow|sheep|elephant|giraffe|zebra|bear\b",
    "object": r"still life|flower|car\b|motorcycle|bicycle|train|boat|fruit|cup|bottle",
    "indoor": r"interior|church|nave|library|hall|room|museum",
}


def to_input(img):
    w, h = img.size
    scale = SIZE / max(w, h)
    nw, nh = max(1, round(w * scale)), max(1, round(h * scale))
    arr = np.asarray(img.resize((nw, nh), Image.BILINEAR), dtype=np.float32) / 255.0
    arr = (arr - MEAN) / STD
    # Black padding, normalised like the photo: EomtImageProcessor pads before it normalises.
    out = np.ones((3, SIZE, SIZE), dtype=np.float32) * (-MEAN / STD)[:, None, None]
    out[:, :nh, :nw] = arr.transpose(2, 0, 1)
    return out[None], (nw, nh)


def panoptic(cls_logits, mask_logits, used):
    """Returns (segment map at mask size cropped to the photo, [(label, score, seg id)])."""
    e = np.exp(cls_logits[0] - cls_logits[0].max(axis=1, keepdims=True))
    probs = e / e.sum(axis=1, keepdims=True)
    labels = probs[:, :-1].argmax(axis=1)
    scores = probs[np.arange(len(labels)), labels]
    keep = np.where(scores >= SCORE_MIN)[0]
    h, w = mask_logits.shape[2:]
    uw, uh = max(1, round(used[0] * w / SIZE)), max(1, round(used[1] * h / SIZE))
    masks = 1.0 / (1.0 + np.exp(-np.clip(mask_logits[0][keep][:, :uh, :uw], -40, 40)))
    seg = np.full((uh, uw), -1, dtype=np.int32)
    found = []
    if len(keep) == 0:
        return seg, found, np.zeros((0, uh, uw))
    weighted = masks * scores[keep][:, None, None]
    owner = weighted.argmax(axis=0)
    soft = []
    for i, q in enumerate(keep):
        mine = owner == i
        confident = masks[i] >= 0.5
        if mine.sum() == 0 or confident.sum() == 0:
            continue
        kept = mine & confident
        if kept.sum() / confident.sum() < KEEP_MIN:
            continue
        seg[kept] = len(found)
        found.append((int(labels[q]), float(scores[q])))
        soft.append(masks[i])
    return seg, found, np.array(soft) if soft else np.zeros((0, uh, uw))


def guided(guide, src, radius, eps=1e-3):
    size = 2 * radius + 1
    box = lambda a: uniform_filter(a, size=size, mode="nearest")
    mg, ms = box(guide), box(src)
    var = box(guide * guide) - mg * mg
    cov = box(guide * src) - mg * ms
    a = cov / (var + eps)
    b = ms - a * mg
    return np.clip(box(a) * guide + box(b), 0, 1)


def upscale(mask, size):
    img = Image.fromarray((np.clip(mask, 0, 1) * 255).astype(np.uint8))
    return np.asarray(img.resize(size, Image.BICUBIC), dtype=np.float32) / 255.0


def prominent(seg, found, names, soft):
    """The subject: most prominent thing, weighted for size and centre, living things first."""
    if not found:
        return None, [], 0.0
    h, w = seg.shape
    ys, xs = np.mgrid[0:h, 0:w]
    ranked = []
    for i, (label, score) in enumerate(found):
        name = names[label]
        if label >= 80:  # stuff
            continue
        m = seg == i
        area = m.sum() / m.size
        if area < 0.003:
            continue
        cx, cy = xs[m].mean() / w, ys[m].mean() / h
        off = min(1.0, np.hypot(cx - 0.5, cy - 0.5) / 0.7071)
        weight = 2.0 if name in LIVING else 1.0
        ranked.append((area * (1 - 0.6 * off) * weight * score, i, name))
    if not ranked:
        return None, [], 0.0
    ranked.sort(reverse=True)
    top, _, top_name = ranked[0]
    # A quarter, not a half: at a half, one of a group portrait's people fell into Background.
    chosen = [i for s, i, n in ranked if s >= top * 0.25 and (n == top_name)]
    out = np.zeros_like(soft[0])
    for i in chosen:
        out = np.maximum(out, soft[i] * (seg == i).astype(np.float32) + soft[i] * (seg == -1))
    return out, [top_name] * len(chosen), max(found[i][1] for i in chosen)


def mask_rows(seg, found, names, soft, sky_ids):
    """What is stored for a photo: [(kind, instance, score, weights 0..1)]. Rust's `facet::segments`."""
    rows = []
    skies = [i for i, (label, score) in enumerate(found) if label in sky_ids and score >= SKY_SCORE_MIN]
    if skies:
        rows.append(("sky", 0, max(found[i][1] for i in skies), np.max([soft[i] for i in skies], axis=0)))
    subj, _, score = prominent(seg, found, names, soft)
    if subj is not None:
        rows.append(("subject", 0, score, subj))
        # Background is the subject's inverse; a photo with no subject has none.
        rows.append(("background", 0, score, 1.0 - subj))
    people = [i for i, (label, _) in enumerate(found) if names[label] == "person"]
    for n, i in enumerate(people):
        rows.append(("person", n, found[i][1], soft[i] * (seg == i)))
    return rows


def tint(rgb, mask, colour):
    c = np.array(colour, dtype=np.float32)
    a = (mask * 0.6)[..., None]
    return np.uint8(rgb * (1 - a) + c * a)


def choose(photos, manifest, per_group, seed=8):
    random.seed(seed)
    rows = []
    with open(manifest, encoding="utf-8") as f:
        for line in f:
            r = json.loads(line)
            if r.get("kept") and (photos / f"{r['pageid']}.jpg").exists():
                rows.append(r)
    rows.sort(key=lambda r: r["pageid"])
    random.shuffle(rows)
    picked, seen = [], set()
    for group, pat in GROUPS.items():
        n = 0
        for r in rows:
            if n >= per_group:
                break
            if r["pageid"] not in seen and re.search(pat, r["title"], re.I):
                picked.append((group, r))
                seen.add(r["pageid"])
                n += 1
    return picked


def session(path, gpu):
    so = ort.SessionOptions()
    if gpu:
        so.enable_mem_pattern = False
        so.execution_mode = ort.ExecutionMode.ORT_SEQUENTIAL
        return ort.InferenceSession(path, so, providers=["DmlExecutionProvider"])
    so.intra_op_num_threads = 4
    return ort.InferenceSession(path, so, providers=["CPUExecutionProvider"])


def timing(path, x):
    for gpu, name in ((True, "DirectML"), (False, "CPU 4 threads")):
        t0 = time.perf_counter()
        s = session(path, gpu)
        load = time.perf_counter() - t0
        for _ in range(3):
            s.run(None, {"pixels": x})
        ts = []
        for _ in range(7):
            t0 = time.perf_counter()
            s.run(None, {"pixels": x})
            ts.append((time.perf_counter() - t0) * 1000)
        ts.sort()
        print(f"{Path(path).name} {name}: load {load:.2f} s, median {ts[3]:.0f} ms, min {ts[0]:.0f}, max {ts[-1]:.0f} (provisional: machine busy)")


def main():
    onnx_path, photos, manifest, out = sys.argv[1], Path(sys.argv[2]), sys.argv[3], Path(sys.argv[4])
    out.mkdir(parents=True, exist_ok=True)
    names = {int(k): v for k, v in json.loads(Path(onnx_path).with_name("id2label.json").read_text()).items()}
    sky_ids = {k for k, v in names.items() if v.startswith("sky")}
    print("sky classes:", {k: names[k] for k in sky_ids}, "first stuff:", names[80], "person:", names[0])

    picks = choose(photos, manifest, per_group=5)
    first = Image.open(photos / f"{picks[0][1]['pageid']}.jpg").convert("RGB")
    timing(onnx_path, to_input(first)[0])
    if "--timing-only" in sys.argv:
        return

    sess = session(onnx_path, True)
    cell = 380
    rows = []
    report = []
    for group, r in picks:
        img = Image.open(photos / f"{r['pageid']}.jpg").convert("RGB")
        x, used = to_input(img)
        cls, masks = sess.run(None, {"pixels": x})
        seg, found, soft = panoptic(cls, masks, used)

        w, h = img.size
        s = cell / max(w, h)
        view = img.resize((max(1, round(w * s)), max(1, round(h * s))), Image.BILINEAR)
        vw, vh = view.size
        rgb = np.asarray(view, dtype=np.float32)
        guide = (0.2126 * rgb[..., 0] + 0.7152 * rgb[..., 1] + 0.0722 * rgb[..., 2]) / 255.0
        radius = max(2, round(max(vw, vh) / 64))

        sky_soft = np.zeros(seg.shape, dtype=np.float32)
        for i, (label, _) in enumerate(found):
            if label in sky_ids and found[i][1] >= SKY_SCORE_MIN:
                sky_soft = np.maximum(sky_soft, soft[i])
        sky_ref = guided(guide, upscale(sky_soft, (vw, vh)), radius)

        subj, subj_names, _ = prominent(seg, found, names, soft)
        subj_ref = guided(guide, upscale(subj, (vw, vh)), radius) if subj is not None else np.zeros((vh, vw), dtype=np.float32)
        # Background is the subject's inverse; a photo with no subject has none.
        back_ref = 1.0 - subj_ref if subj is not None else np.zeros((vh, vw), dtype=np.float32)

        people = np.uint8(rgb.copy())
        palette = [(255, 80, 80), (80, 200, 255), (255, 210, 70), (120, 255, 120), (230, 120, 255), (255, 150, 60)]
        n_people = 0
        for i, (label, _) in enumerate(found):
            if names[label] == "person":
                m = guided(guide, upscale(soft[i] * (seg == i), (vw, vh)), radius)
                people = tint(people, m, palette[n_people % len(palette)])
                n_people += 1

        canvas = Image.new("RGB", (cell * 5 + 40, cell + 22), (24, 24, 24))
        d = ImageDraw.Draw(canvas)
        things = sorted({names[l] for l, _ in found if l < 80})
        d.text((4, 4), f"{r['pageid']} [{group}] subject={subj_names[:1]}x{len(subj_names)} things={things[:7]} sky={'yes' if sky_soft.max() > 0 else 'no'}", fill=(235, 235, 235))
        for c, a in enumerate((
            view,
            tint(rgb, sky_ref, (60, 140, 255)),
            tint(rgb, back_ref, (70, 210, 130)),
            tint(rgb, subj_ref, (255, 90, 90)),
            people,
        )):
            canvas.paste(Image.fromarray(a) if not isinstance(a, Image.Image) else a, (c * (cell + 8) + 4, 20))
        rows.append(canvas)
        report.append({"pageid": r["pageid"], "group": group, "title": r["title"], "things": things, "sky": bool(sky_soft.max() > 0), "subject": subj_names[:1], "people": n_people})

    per_sheet = 12
    for p in range(0, len(rows), per_sheet):
        part = rows[p : p + per_sheet]
        sheet = Image.new("RGB", (part[0].width, sum(c.height + 4 for c in part)), (10, 10, 10))
        y = 0
        for c in part:
            sheet.paste(c, (0, y))
            y += c.height + 4
        sheet.save(out / f"facet-sheet-{p // per_sheet + 1}.jpg", quality=88)
    (out / "report.json").write_text(json.dumps(report, indent=1), encoding="utf-8")
    print(f"{len(rows)} photos; sheets in {out}")


if __name__ == "__main__":
    main()
