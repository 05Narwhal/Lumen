"""Evaluates a Retouch checkpoint.

    python -m lumen_train.retouch.eval --ckpt D:/lumen-train/runs/r1/best.pt --data D:/lumen-train --out D:/lumen-train/eval/r1 [--sources a b ...] [--marks marks.json]

A source the checkpoint trained on is scored on its validation split only (synthetic pairs
fixed by the seed); any other source (Julian's catalog, evaluation-only) on every photo.
Writes `report.json` / `report.md` — CIEDE2000 of Retouch against the target, beside the photo
left untouched and Auto (its measured prior, the app's Auto without its model, measured by
Rust on exactly the pixels Retouch saw); slider error per panel; crop overlap, crop on/off
accuracy and straighten error; and the gap between a RAW's and its JPEG's predictions on the
photos as prepared — overall and per source (`by_source`) — and `sheet.html` to mark by eye.
ΔE counts only the pixels a target covers: an aligned pair's target is the photographer's JPEG
warped back, absent outside his crop (`target_mask.u8`).

The sheet shows before | Auto | Retouch | the edit (a real pair's) or target (synthetic). Every
photo of a `--sheet-all` source is a row, first (Julian's own photos, `julian-pairs`: with
`--split test`, only the ones no run trains on); the other sources share `--rows`.

Ship bar (`passes`): lower ΔE than Auto (or untouched, where Auto is unknown), and Retouch
marked better or the same on at least 80 % of the sheet."""

import argparse
import json
import subprocess
import sys
import tempfile
from collections import defaultdict
from pathlib import Path
from typing import Callable

import numpy as np
import torch
from PIL import Image

from ..common import ckpt
from ..common.sheet import write as write_sheet
from . import params, render
from .dataset import PreparedDataset
from .model import RetouchNet
from .paths import example
from .prepared import decode_input, encode_input, read_meta, read_raw
from .train import iou, scored

SHIP_SHARE = 0.8

# Auto's Basic for each prepared input file (`input.u16` layout), None where it failed.
AutoBasic = Callable[[list[Path]], list[dict | None]]


def rust_auto(files: list[Path]) -> list[dict | None]:
    """`retouch_auto` over the files' folder, in one run."""
    if not files:
        return []
    out = subprocess.run([str(example("retouch_auto")), str(files[0].parent)], capture_output=True, text=True, check=True)
    got = {}
    for line in out.stdout.splitlines():
        row = json.loads(line)
        got[Path(row["file"]).name] = row.get("auto")
    return [got.get(f.name) for f in files]


def _image(lin: torch.Tensor, mask: torch.Tensor) -> Image.Image:
    w = int(mask[0, 0].sum())
    h = int(mask[0, :, 0].sum())
    shown = render.to_display(lin[:, :h, :w].clamp(min=0)).clamp(0, 1)
    return Image.fromarray((shown.permute(1, 2, 0).numpy() * 255).round().astype("uint8"))


def _batch(item: dict) -> dict:
    return {k: (v.unsqueeze(0) if isinstance(v, torch.Tensor) else torch.tensor([v])) for k, v in item.items()}


def _summary(v: torch.Tensor, aspect: float) -> str:
    s = params.value(v, "straighten")[0].item()
    crop = params.decode_crop(v, torch.tensor([aspect]))
    words = [f"straighten {s:+.1f}°"]
    if crop["on"][0] >= 0.5:
        x, y, w, h = crop["box"][0].tolist()
        words.append(f"crop {x:.2f},{y:.2f} {w:.2f}×{h:.2f}")
    words.append(f"exposure {params.value(v, 'exposure')[0].item():+.2f}")
    return ", ".join(words)


def _datasets(root: Path, sources: list[str], trained_on: list[str], split: str | None = None) -> list[tuple[str, PreparedDataset]]:
    """Each source's validation split if the checkpoint trained on it, else all of it; `split`
    (e.g. `test`, the photos no run ever trains on) overrides that for every source."""
    return [(s, PreparedDataset(root, [s], split=split or ("val" if s in trained_on else "all"), augment=False)) for s in sources]


def _autos(sets: list[tuple[str, PreparedDataset]], auto_basic: AutoBasic | None, tmp: Path) -> dict[tuple[str, int], dict]:
    """Auto's Basic for every item: a real pair's from `meta.json` (measured on the photo it
    shows) when prep recorded it, else measured now on the photo Retouch saw (a synthetic
    pair's degraded one)."""
    autos: dict[tuple[str, int], dict] = {}
    pending: list[tuple[tuple[str, int], Path]] = []
    for source, ds in sets:
        for i in range(len(ds)):
            folder = ds.samples[i]
            if (folder / "target.json").exists() and "auto" in (meta := read_meta(folder)):
                autos[(source, i)] = meta["auto"]
            elif auto_basic is not None:
                item = ds[i]
                w, h = (int(x) for x in render.photo_size(item["mask"].unsqueeze(0))[0].tolist())
                packed = encode_input(item["aligned"][:, :h, :w], w, h)
                f = tmp / f"{len(pending):06d}.u16"
                (packed.clamp(0, 1) * 65535).round().numpy().astype("<u2").tofile(f)
                pending.append(((source, i), f))
    if pending:
        for (key, _), basic in zip(pending, auto_basic([f for _, f in pending])):
            if basic is not None:
                autos[key] = basic
    return autos


def item_de(b: dict, v: torch.Tensor) -> tuple[float, torch.Tensor, torch.Tensor]:
    """One batched item's ΔE for predicted sliders `v`: (ΔE, Retouch's render, the target)."""
    aligned, size = b["aligned"], render.photo_size(b["mask"])
    if b["has_target"][0] > 0:
        target = b["target"]
    else:
        # A pair with no rendered target: its slider values rendered here (global stages).
        target = render.render(aligned, params.decode(b["v_target"], b["aspect"], hard=True), size)
    mine = render.render(aligned, params.decode(v, b["aspect"], hard=True), size)
    return float(render.delta_e(mine, target, scored(b))[0]), mine, target


def _scores(de_r: list[float], de_u: list[float], de_a: list[float], ious: list[float]) -> dict:
    mean = lambda xs: sum(xs) / len(xs) if xs else None
    return {"count": len(de_r), "de_retouch": mean(de_r), "de_auto": mean(de_a), "auto_count": len(de_a), "de_untouched": mean(de_u), "crop_iou": mean(ious)}


@torch.no_grad()
def run(ckpt_path: Path, root: Path, sources: list[str], out: Path, sheet_rows: int = 60, auto_basic: AutoBasic | None = rust_auto, split: str | None = None, sheet_all: list[str] | None = None) -> dict:
    model = RetouchNet()
    state = ckpt.load(ckpt_path, model)
    model.eval()
    trained_on = state.get("extra", {}).get("sources") or []
    out.mkdir(parents=True, exist_ok=True)
    sets = _datasets(root, sources, trained_on, split)
    with tempfile.TemporaryDirectory() as tmp:
        autos = _autos(sets, auto_basic, Path(tmp))
    de_r, de_u, de_a = [], [], []
    group_err: dict[str, list[float]] = defaultdict(list)
    ious, angles, presence = [], [], []
    by_photo: dict[tuple[str, str], list[tuple[str, torch.Tensor]]] = defaultdict(list)
    sheet: list[dict] = []
    own: list[dict] = []  # rows of `sheet_all` sources: every one, first
    per: dict[str, dict[str, list[float]]] = defaultdict(lambda: defaultdict(list))
    groups = [p["group"] for p in params.PARAMS]
    for source, ds in sets:
        for i in range(len(ds)):
            folder, b = ds.samples[i], _batch(ds[i])
            v = model(b["input"], b["clip"], b["clip_flag"])
            aligned, mask = b["aligned"], b["mask"]
            size = render.photo_size(mask)
            de, mine, target = item_de(b, v)
            seen = scored(b)
            de_r.append(de)
            de_u.append(float(render.delta_e(aligned, target, seen)[0]))
            per[source]["r"].append(de_r[-1])
            per[source]["u"].append(de_u[-1])
            auto_img = None
            if (source, i) in autos:
                auto_img = render.render(aligned, render.from_develop_json({"basic": autos[(source, i)]}), size)
                de_a.append(float(render.delta_e(auto_img, target, seen)[0]))
                per[source]["a"].append(de_a[-1])
            known, t = b["v_known"][0], b["v_target"][0]
            for j, g in enumerate(groups):
                if known[j] > 0:
                    group_err[g].append(abs(float(v[0, j] - t[j])))
            on = params.index("crop_on")
            if known[on] > 0:
                presence.append(float((v[0, on] >= 0.5) == (t[on] >= 0.5)))
                if t[on] >= 0.5 and known[params.index("crop_w")] > 0:
                    box = lambda x: torch.stack([params.value(x, f"crop_{k}") for k in "xywh"], dim=1)
                    ious.append(float(iou(box(v), box(t.unsqueeze(0)))[0]))
                    per[source]["iou"].append(ious[-1])
            s = params.index("straighten")
            if known[s] > 0:
                angles.append(abs(float(params.value(v, "straighten")[0] - params.value(t.unsqueeze(0), "straighten")[0])))
            meta = read_meta(folder)
            if meta.get("source"):
                # The RAW and its JPEG are one scene: compare what Retouch makes of each as
                # prepared (a synthetic item's random degradation would differ between them).
                src = Path(meta["source"])
                lin, m, w, h = decode_input(read_raw(folder))
                plain = encode_input(lin[:, :h, :w], w, h).unsqueeze(0)
                none = torch.zeros(1, b["clip"].shape[1])
                by_photo[(str(src.parent).lower(), src.stem.lower())].append((src.suffix.lower(), model(plain, none, torch.zeros(1, 1))[0]))
            mine_row = source in (sheet_all or [])
            if mine_row or len(sheet) < sheet_rows:
                images = [("before", _image(aligned[0], mask[0]))]
                if auto_img is not None:
                    images.append(("Auto", _image(auto_img[0], mask[0])))
                last = "edit" if int(b["kind"][0]) == 1 else "target"
                images += [("Retouch", _image(mine[0], mask[0])), (last, _image(target[0], mask[0]))]
                (own if mine_row else sheet).append({"id": f"{source}/{folder.name}", "images": images, "caption": _summary(v, float(b["aspect"][0]))})
    gaps = []
    for versions in by_photo.values():
        firsts = {}
        for ext, x in versions:
            firsts.setdefault(ext, x)
        if len(firsts) > 1:
            vs = list(firsts.values())
            gaps.append(float((vs[0] - vs[1]).abs().mean()))
    mean = lambda xs: sum(xs) / len(xs) if xs else None
    report = {
        "checkpoint": str(ckpt_path),
        "trained_on": trained_on,
        "shippable": state.get("extra", {}).get("ship", False),
        "sources": sources,
        "count": len(de_r),
        "de_retouch": mean(de_r),
        "de_retouch_median": float(np.median(de_r)) if de_r else None,
        "de_untouched": mean(de_u),
        "de_auto": mean(de_a),
        "auto_count": len(de_a),
        "param_error": {g: mean(e) for g, e in group_err.items()},
        "crop_iou": mean(ious),
        "crop_presence_accuracy": mean(presence),
        "angle_error": mean(angles),
        "raw_jpeg_gap": mean(gaps),
        "raw_jpeg_pairs": len(gaps),
        "by_source": {s: _scores(per[s]["r"], per[s]["u"], per[s]["a"], per[s]["iou"]) for s, _ in sets},
    }
    (out / "report.json").write_text(json.dumps(report, indent=2), encoding="utf-8")
    (out / "report.md").write_text(_markdown(report), encoding="utf-8")
    write_sheet(out / "sheet.html", f"Retouch — {Path(ckpt_path).parent.name}", own + sheet, key=str(Path(ckpt_path).resolve()))
    return report


def _markdown(r: dict) -> str:
    f = lambda x, fmt="{:.2f}": "—" if x is None else fmt.format(x)
    lines = [
        f"# Retouch evaluation — {r['count']} photos",
        "",
        f"Checkpoint `{r['checkpoint']}`, trained on {', '.join(r['trained_on'] or [])} ({'shippable' if r['shippable'] else 'not for shipping'}); scored on {', '.join(r['sources'])}.",
        "",
        "| Measure | Retouch | Auto | Untouched |",
        "|---|---|---|---|",
        f"| Mean ΔE (CIEDE2000) to the target | {f(r['de_retouch'])} | {f(r['de_auto'])} ({r['auto_count']} photos) | {f(r['de_untouched'])} |",
        f"| Median ΔE | {f(r['de_retouch_median'])} | | |",
        "",
        f"Crop overlap (IoU) {f(r['crop_iou'])}, crop on/off right {f(r['crop_presence_accuracy'], '{:.0%}')}, straighten error {f(r['angle_error'])}°, RAW vs JPEG gap {f(r['raw_jpeg_gap'], '{:.3f}')} over {r['raw_jpeg_pairs']} pairs.",
        "",
        "Slider error per panel (0–1 units): " + ", ".join(f"{g} {e:.3f}" for g, e in sorted(r["param_error"].items())),
        "",
        "| Source | Photos | Retouch ΔE | Auto ΔE | Untouched ΔE | Crop IoU |",
        "|---|---|---|---|---|---|",
    ]
    for s, x in (r.get("by_source") or {}).items():
        lines.append(f"| {s} | {x['count']} | {f(x['de_retouch'])} | {f(x['de_auto'])} ({x['auto_count']}) | {f(x['de_untouched'])} | {f(x['crop_iou'])} |")
    lines.append("")
    return "\n".join(lines)


def passes(report: dict, marks: dict) -> bool:
    if not marks or report.get("de_retouch") is None:
        return False
    base = report["de_auto"] if report.get("de_auto") is not None else report["de_untouched"]
    good = sum(1 for m in marks.values() if m in ("better", "same")) / len(marks)
    return report["de_retouch"] < base and good >= SHIP_SHARE


def main() -> None:
    # Progress lines hold ΔE and dashes: redirected to a file on Windows, stdout would be
    # cp1252 and the first epoch's line would end the run.
    sys.stdout.reconfigure(encoding="utf-8", errors="replace")
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--ckpt", type=Path, required=True)
    ap.add_argument("--data", type=Path, required=True)
    ap.add_argument("--out", type=Path, required=True)
    ap.add_argument("--sources", nargs="*", help="default: those the checkpoint trained on")
    ap.add_argument("--split", choices=["train", "val", "test", "all"], help="score this split of every source (default: val of trained sources, all of the others)")
    ap.add_argument("--rows", type=int, default=60, help="sheet rows from the sources not in --sheet-all")
    ap.add_argument("--sheet-all", nargs="*", default=["julian-pairs"], help="sources whose every scored photo is a sheet row, first (default: julian-pairs, Julian's own photos)")
    ap.add_argument("--marks", type=Path, help="marks.json from the sheet: say whether it passes the ship bar")
    a = ap.parse_args()
    sources = a.sources or ckpt.read(a.ckpt)["extra"]["sources"]
    report = run(a.ckpt, a.data, sources, a.out, a.rows, split=a.split, sheet_all=a.sheet_all)
    print((a.out / "report.md").read_text(encoding="utf-8"))
    if a.marks:
        print("Passes the ship bar." if passes(report, json.loads(a.marks.read_text(encoding="utf-8"))) else "Does not pass the ship bar.")


if __name__ == "__main__":
    main()
