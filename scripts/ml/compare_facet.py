"""P3b: EoMT-S vs EoMT-B on the same public photos, post-processed exactly as Rust does (eval_facet).

    python -I scripts/ml/compare_facet.py scripts/ml <s.onnx> <b.onnx> <out dir> <per_group>

(P3b, 2026-11-03: per_group 24 gives 120 photos; odd-*.jpg holds the photos where they disagree.)

Per photo: each model's rows (kind, instance, score); per kind IoU of S vs B where both find it;
a sheet per photo: photo | S subject | B subject | S people | B people | S sky | B sky.
"""
import json
import sys
from pathlib import Path

import numpy as np
from PIL import Image, ImageDraw

sys.path.insert(0, sys.argv[1])
import eval_facet as ef  # noqa: E402

s_path, b_path, out, per_group = sys.argv[2], sys.argv[3], Path(sys.argv[4]), int(sys.argv[5])
out.mkdir(parents=True, exist_ok=True)
photos = Path("C:/lumen-train/originals/wikimedia-cc0-pd")
names = {int(k): v for k, v in json.loads(Path(s_path).with_name("id2label.json").read_text()).items()}
sky_ids = {k for k, v in names.items() if v.startswith("sky")}
picks = ef.choose(photos, "C:/lumen-train/originals/manifest.jsonl", per_group=per_group)
sess = {"s": ef.session(s_path, True), "b": ef.session(b_path, True)}
cell = 300
palette = [(255, 80, 80), (80, 200, 255), (255, 210, 70), (120, 255, 120), (230, 120, 255), (255, 150, 60)]


def iou(a, b):
    a, b = a >= 0.5, b >= 0.5
    u = (a | b).sum()
    return float((a & b).sum() / u) if u else 1.0


report, sheets = [], []
for group, r in picks:
    img = Image.open(photos / f"{r['pageid']}.jpg").convert("RGB")
    x, used = ef.to_input(img)
    rows, segs = {}, {}
    for k, s in sess.items():
        seg, found, soft = ef.panoptic(*s.run(None, {"pixels": x}), used)
        rows[k] = ef.mask_rows(seg, found, names, soft, sky_ids)
        segs[k] = found
    entry = {"pageid": r["pageid"], "group": group, "title": r["title"][5:60]}
    for k in ("s", "b"):
        entry[k] = {
            "kinds": [(kind, inst, round(score, 3)) for kind, inst, score, _ in rows[k]],
            "sky_scores": [round(sc, 3) for l, sc in segs[k] if l in sky_ids],
            "things": sorted({names[l] for l, _ in segs[k] if l < 80}),
        }
    by = {k: {(kind, inst): w for kind, inst, _, w in rows[k]} for k in rows}
    entry["iou"] = {f"{kind}{inst}": round(iou(by["s"][(kind, inst)], by["b"][(kind, inst)]), 3)
                    for (kind, inst) in by["s"] if (kind, inst) in by["b"] and kind != "person"}
    entry["persons"] = [sum(1 for kind, *_ in rows[k] if kind == "person") for k in ("s", "b")]
    report.append(entry)

    w, h = img.size
    sc = cell / max(w, h)
    view = img.resize((max(1, round(w * sc)), max(1, round(h * sc))), Image.BILINEAR)
    vw, vh = view.size
    rgb = np.asarray(view, dtype=np.float32)
    guide = (0.2126 * rgb[..., 0] + 0.7152 * rgb[..., 1] + 0.0722 * rgb[..., 2]) / 255.0
    radius = max(2, round(max(vw, vh) / 64))
    zero = np.zeros((vh, vw), dtype=np.float32)
    ref = lambda m: ef.guided(guide, ef.upscale(m, (vw, vh)), radius, eps=2e-4)
    cols = [view]
    for kind, colour in (("subject", (255, 90, 90)), ("person", None), ("sky", (60, 140, 255))):
        for k in ("s", "b"):
            if kind == "person":
                pic = np.uint8(rgb.copy())
                n = 0
                for (kd, inst), m in by[k].items():
                    if kd == "person":
                        pic = ef.tint(pic, ref(m), palette[n % len(palette)])
                        n += 1
                cols.append(pic)
            else:
                m = by[k].get((kind, 0))
                cols.append(ef.tint(rgb, ref(m) if m is not None else zero, colour))
    canvas = Image.new("RGB", (len(cols) * (cell + 6) + 4, cell + 22), (24, 24, 24))
    d = ImageDraw.Draw(canvas)
    d.text((4, 4), f"{r['pageid']} [{group}] S/B: subjIoU={entry['iou'].get('subject0')} persons={entry['persons']} sky S={entry['s']['sky_scores']} B={entry['b']['sky_scores']}", fill=(235, 235, 235))
    for c, a in enumerate(cols):
        canvas.paste(a if isinstance(a, Image.Image) else Image.fromarray(a), (c * (cell + 6) + 4, 20))
    sheets.append(canvas)

def kinds(e, k, kind):
    return any(x[0] == kind for x in e[k]["kinds"])


odd = [c for e, c in zip(report, sheets)
       if e["iou"].get("subject0", 1) < 0.7 or kinds(e, "s", "subject") != kinds(e, "b", "subject") or kinds(e, "s", "sky") != kinds(e, "b", "sky")
       or abs(e["persons"][0] - e["persons"][1]) >= 2]
for p in range(0, len(odd), 6):
    part = odd[p:p + 6]
    sheet = Image.new("RGB", (part[0].width, sum(c.height + 4 for c in part)), (10, 10, 10))
    y = 0
    for c in part:
        sheet.paste(c, (0, y))
        y += c.height + 4
    sheet.save(out / f"odd-{p // 6 + 1:02d}.jpg", quality=85)
print("odd photos:", len(odd))

for p in range(0, len(sheets), 10):
    part = sheets[p:p + 10]
    sheet = Image.new("RGB", (part[0].width, sum(c.height + 4 for c in part)), (10, 10, 10))
    y = 0
    for c in part:
        sheet.paste(c, (0, y))
        y += c.height + 4
    sheet.save(out / f"sb-{p // 10 + 1:02d}.jpg", quality=85)
(out / "report.json").write_text(json.dumps(report, indent=1), encoding="utf-8")

ious = {}
for e in report:
    for k, v in e["iou"].items():
        ious.setdefault(k.rstrip("0123456789"), []).append(v)
for k, v in ious.items():
    v = np.array(v)
    print(f"{k}: n={len(v)} IoU median {np.median(v):.3f} p10 {np.percentile(v, 10):.3f} under 0.7: {(v < 0.7).sum()}")
for kind in ("sky", "subject"):
    only_s = [e["pageid"] for e in report if any(k[0] == kind for k in e["s"]["kinds"]) and not any(k[0] == kind for k in e["b"]["kinds"])]
    only_b = [e["pageid"] for e in report if any(k[0] == kind for k in e["b"]["kinds"]) and not any(k[0] == kind for k in e["s"]["kinds"])]
    print(f"{kind}: only S {len(only_s)} {only_s[:12]}; only B {len(only_b)} {only_b[:12]}")
print("persons S total", sum(e["persons"][0] for e in report), "B total", sum(e["persons"][1] for e in report))
print(f"{len(report)} photos; sheets in {out}")
