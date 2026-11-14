"""P3b: the sky bar on photos with people. Every kept public photo runs through the model; those
where a person fills >= 4 % and sky fires at any score are drawn with the sky tinted and the
score written, for a by-eye real/false call. `new` marks photos outside the 120 the bar was set on.

    python -I scripts/ml/sky_facet.py scripts/ml <onnx> <out dir>
"""
import json
import sys
from pathlib import Path

import numpy as np
from PIL import Image, ImageDraw

sys.path.insert(0, sys.argv[1])
import eval_facet as ef  # noqa: E402

onnx, out = sys.argv[2], Path(sys.argv[3])
out.mkdir(parents=True, exist_ok=True)
photos = Path("C:/lumen-train/originals/wikimedia-cc0-pd")
# The 120 photos the 0.95 bar was set on (the design session's sky probe), to count new evidence.
_groups = ef.GROUPS
ef.GROUPS = {k: v for k, v in _groups.items() if k in ("people", "sky")}
skip = {r["pageid"] for _, r in ef.choose(photos, "C:/lumen-train/originals/manifest.jsonl", per_group=60)}
ef.GROUPS = _groups
names = {int(k): v for k, v in json.loads(Path(onnx).with_name("id2label.json").read_text()).items()}
sky_ids = {k for k, v in names.items() if v.startswith("sky")}
rows = []
with open("C:/lumen-train/originals/manifest.jsonl", encoding="utf-8") as f:
    for line in f:
        r = json.loads(line)
        if r.get("kept") and (photos / f"{r['pageid']}.jpg").exists():
            rows.append(r)
rows.sort(key=lambda r: r["pageid"])
s = ef.session(onnx, True)
cells, found_rows = [], []
for r in rows:
    img = Image.open(photos / f"{r['pageid']}.jpg").convert("RGB")
    x, used = ef.to_input(img)
    seg, found, soft = ef.panoptic(*s.run(None, {"pixels": x}), used)
    person = np.isin(seg, [i for i, (l, _) in enumerate(found) if l == 0]).mean()
    sky = [i for i, (l, _) in enumerate(found) if l in sky_ids]
    if person < 0.04 or not sky:
        continue
    score = max(found[i][1] for i in sky)
    area = float(np.isin(seg, sky).mean())
    found_rows.append({"pageid": r["pageid"], "score": round(score, 3), "sky_area": round(area, 3), "person_area": round(float(person), 3), "new": r["pageid"] not in skip, "title": r["title"][5:70]})
    w, h = img.size
    sc = 260 / max(w, h)
    view = img.resize((max(1, round(w * sc)), max(1, round(h * sc))), Image.BILINEAR)
    rgb = np.asarray(view, dtype=np.float32)
    m = ef.upscale(np.max([soft[i] for i in sky], axis=0) * np.isin(seg, sky), view.size)
    cell = Image.new("RGB", (540, 284), (24, 24, 24))
    cell.paste(view, (4, 20))
    cell.paste(Image.fromarray(ef.tint(rgb, m, (60, 140, 255))), (276, 20))
    ImageDraw.Draw(cell).text((4, 4), f"#{len(found_rows)} {r['pageid']} sky {score:.3f} area {area:.2f} person {person:.2f}", fill=(235, 235, 235))
    cells.append(cell)

for p in range(0, len(cells), 12):
    part = cells[p:p + 12]
    sheet = Image.new("RGB", (540 * 3, 284 * ((len(part) + 2) // 3)), (10, 10, 10))
    for i, c in enumerate(part):
        sheet.paste(c, ((i % 3) * 540, (i // 3) * 284))
    sheet.save(out / f"sky-{p // 12 + 1:02d}.jpg", quality=85)
(out / "sky.json").write_text(json.dumps(found_rows, indent=1), encoding="utf-8")
print(f"{len(rows)} photos; {len(found_rows)} with a person >= 4 % and sky firing; new {sum(r['new'] for r in found_rows)}")
for r in sorted(found_rows, key=lambda r: r["score"]):
    print(r["pageid"], r["score"], r["sky_area"], r["person_area"], "new" if r["new"] else "seen", r["title"][:40])
