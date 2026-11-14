"""Save a trained Retouch checkpoint as a named Lumen model, and install it for testing in the app
(Round 12-4 Task 1, real tiers Round 14-4a; names in `common/naming.py`).

    python -m lumen_train.retouch.export --ckpt C:/lumen-train/runs/r3ft/best_de.pt --family ray --version 1.0.1-d --install-dev

writes `<out>/lumen-ray-1.0.1-d/`: `model.json` (each tier's manifest entry, the app's `ModelEntry`
plus `precision`, `tta` and its own `val_de`), the checkpoint (so the next version can warm-start
from it) and the tier files (`tiers.py`): `-high.onnx` fp32; `-low.onnx` int8, or fp16 when int8
costs more than 0.3 ΔE; `ultra` names high's file with `tta: 5`. Tiers are scored on the trained
sources' validation photos under `--data`. Saving a tier again replaces only that tier.
`--install-dev` copies every tier into `config_dir/JL-Studios/Lumen/models/dev/<family>/<id>/`;
`--install-dev <tier>` also writes the old `retouch.json` + that tier's file at the top. Settings →
Experimental picks them up. Nothing is uploaded."""

import argparse
import hashlib
import json
import os
import shutil
import sys
import time
import warnings
from pathlib import Path

import numpy as np
import onnxruntime as ort
import torch

from ..common import ckpt, naming, onnx_export
from ..data import sources as data_sources
from . import params
from . import tiers as tier_maths
from .model import RetouchNet
from .prepared import EDGE

INPUTS = ["image", "clip", "clip_flag"]
OUTPUT = "v"
CLIP = 512


class LicenceRefused(RuntimeError):
    pass


def dev_dir() -> Path:
    """The app's `models::dev_dir`: `$LUMEN_DEV_MODELS`, else `dirs::config_dir()` +
    `JL-Studios/Lumen/models/dev` (never Tauri's app dirs)."""
    if os.environ.get("LUMEN_DEV_MODELS"):
        return Path(os.environ["LUMEN_DEV_MODELS"])
    if sys.platform == "win32":
        base = Path(os.environ["APPDATA"])
    elif sys.platform == "darwin":
        base = Path.home() / "Library" / "Application Support"
    else:
        base = Path(os.environ.get("XDG_CONFIG_HOME") or Path.home() / ".config")
    return base / "JL-Studios" / "Lumen" / "models" / "dev"


def _sha256(path: Path) -> str:
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def _licence(trained_on: list[str], root: Path, dev_only: bool) -> str:
    known = data_sources.load(root)
    missing = [s for s in trained_on if s not in known]
    closed = [s for s in trained_on if s in known and not known[s].commercial]
    unknown = not trained_on  # a checkpoint that records no sources: nothing is cleared
    if (missing or closed or unknown) and not dev_only:
        why = ", ".join(missing + closed) if trained_on else "sources it does not record"
        raise LicenceRefused(f"trained on {why}: not cleared for commercial use; pass --dev-only to try it in Lumen anyway")
    parts = ", ".join(f"{s} ({known[s].licence})" if s in known else s for s in trained_on)
    text = f"Lumen (JL Studios); trained on {parts}"
    return f"not for shipping: {text}" if (missing or closed or unknown) else text


def _val_de(state: dict) -> float | None:
    """The checkpoint's validation ΔE2000: recorded by `best_de.pt`, else (an older `best.pt`,
    kept for the lowest validation loss) the ΔE of that epoch."""
    extra = state.get("extra", {})
    if extra.get("val_de") is not None:
        return round(float(extra["val_de"]), 3)
    history = [v for v in extra.get("val", []) if isinstance(v, dict) and "de" in v and "loss" in v]
    return round(float(min(history, key=lambda v: v["loss"])["de"]), 3) if history else None


def _onnx(state: dict, path: Path) -> None:
    net = RetouchNet()
    net.load_state_dict(state["model"])
    net.eval()
    g = torch.Generator().manual_seed(0)
    example = (torch.rand(1, 4, EDGE, EDGE, generator=g), torch.randn(1, CLIP, generator=g), torch.ones(1, 1))
    with warnings.catch_warnings():
        warnings.simplefilter("ignore", torch.jit.TracerWarning)  # the pooled size is fixed: EDGE in, 8 × 8 out
        onnx_export.export(net, example, path, INPUTS, [OUTPUT])
    # Parity again with CLIP missing, as the app sends it (zeros and flag 0).
    session = ort.InferenceSession(str(path), providers=["CPUExecutionProvider"])
    image, clip, flag = example[0], torch.zeros(1, CLIP), torch.zeros(1, 1)
    with torch.no_grad():
        want = net(image, clip, flag).numpy()
    (got,) = session.run([OUTPUT], {"image": image.numpy(), "clip": clip.numpy(), "clip_flag": flag.numpy()})
    if np.abs(got - want).max() > 1e-4:
        raise RuntimeError("onnxruntime differs from PyTorch without CLIP")


def _file_entry(path: Path) -> dict:
    return {"file": path.name, "bytes": path.stat().st_size, "sha256": _sha256(path)}


def _rounded(x: float | None) -> dict:
    return {} if x is None else {"val_de": round(x, 3)}


def _make_tiers(exported: Path, folder: Path, family: str, version: str, tiers: list[str], items: list[dict], calib: list[np.ndarray]) -> tuple[dict[str, dict], float | None]:
    """Write the tiers' files from the fp32 export; returns each tier's file entry, precision, tta
    and `val_de` (where validation photos exist), and low's ΔE over high. Scored in this order:
    high, low (int8, then fp16 if int8 costs more than `LOW_BOUND`), ultra."""
    made: dict[str, dict] = {}
    high_file = folder / f"{naming.model_id(family, version, 'high')}.onnx"
    high_de = None
    if "high" in tiers or "ultra" in tiers or "low" in tiers:
        shutil.copyfile(exported, high_file)
        high_de = tier_maths.mean_de(tier_maths.onnx_predict(high_file), items)
    if "high" in tiers:
        made["high"] = {**_file_entry(high_file), "precision": "fp32", "tta": 1, **_rounded(high_de)}
    low_gap = None
    if "low" in tiers:
        low_file = folder / f"{naming.model_id(family, version, 'low')}.onnx"
        tier_maths.quantize_int8(exported, low_file, calib)
        precision, low_de = "int8", tier_maths.mean_de(tier_maths.onnx_predict(low_file), items)
        if low_de is not None and high_de is not None and low_de - high_de > tier_maths.LOW_BOUND:
            tier_maths.to_fp16(exported, low_file)
            precision, low_de = "fp16", tier_maths.mean_de(tier_maths.onnx_predict(low_file), items)
        made["low"] = {**_file_entry(low_file), "precision": precision, "tta": 1, **_rounded(low_de)}
        if low_de is not None and high_de is not None:
            low_gap = round(low_de - high_de, 3)
    if "ultra" in tiers:
        ultra_de = tier_maths.mean_de(tier_maths.tta_predict(tier_maths.onnx_predict(high_file)), items)
        made["ultra"] = {**_file_entry(high_file), "precision": "fp32", "tta": tier_maths.TTA, **_rounded(ultra_de)}
        (folder / f"{naming.model_id(family, version, 'ultra')}.onnx").unlink(missing_ok=True)  # an older export's copy
    return made, low_gap


def save(checkpoint: Path, family: str, version: str, tiers: list[str], out: Path, root: Path, dev_only: bool = False, data_root: Path | None = None, calib: int = 64) -> dict:
    """Export `checkpoint` as `tiers` of `lumen-<family>-<version>` under `out`; returns `model.json`.
    Each tier is scored on up to `calib` validation photos of the trained sources under
    `data_root` (default `root`), which also calibrate `low`'s int8."""
    if not tiers:
        raise ValueError("name at least one tier")
    for tier in tiers:
        naming.model_id(family, version, tier)  # checks the names
    role = naming.FAMILIES[family].role
    if role != "retouch":
        raise ValueError(f"{naming.FAMILIES[family].title} has no model yet; only Retouch models (Lumen Ray, Lumen Prism) can be saved")
    checkpoint = Path(checkpoint)
    state = ckpt.read(checkpoint)
    extra = state.get("extra", {})
    if int(extra.get("layout", params.LAYOUT)) != params.LAYOUT:
        raise ValueError(f"{checkpoint} was trained for slider layout {extra.get('layout')}, this Lumen uses {params.LAYOUT}")
    trained_on = list(extra.get("sources", []))
    licence = _licence(trained_on, Path(root), dev_only)

    folder = Path(out) / naming.model_id(family, version)
    folder.mkdir(parents=True, exist_ok=True)
    keep = folder / f"checkpoint-{_sha256(checkpoint)[:12]}.pt"
    if not keep.exists():
        shutil.copyfile(checkpoint, keep)
    exported = folder / f".export-{os.getpid()}.onnx"
    try:
        _onnx(state, exported)
        items = tier_maths.val_items(Path(data_root or root), trained_on, calib)
        made, low_gap = _make_tiers(exported, folder, family, version, tiers, items, [b["input"].numpy() for b in items])
        model_file = folder / "model.json"
        model = json.loads(model_file.read_text(encoding="utf-8")) if model_file.exists() else {}
        model.update({"name": naming.display_name(family, version), "family": family, "version": version, "purpose": naming.FAMILIES[family].purpose})
        model.setdefault("tiers", {})
        for tier in tiers:
            mid = naming.model_id(family, version, tier)
            model["tiers"][tier] = {
                "id": mid,
                "name": naming.display_name(family, version, tier),
                "role": role,
                **made[tier],
                "licence": licence,
                "retouch": {"size": EDGE, "layout": params.LAYOUT, "inputs": INPUTS, "output": OUTPUT},
                "trained": {"run": checkpoint.parent.name, "epoch": state.get("epoch"), "val_de": _val_de(state), "sources": trained_on, "clip_share": extra.get("clip_share"), "checkpoint": keep.name, "saved": time.strftime("%Y-%m-%d")},
            }
        for e in model["tiers"].values():  # high's file was written again: tiers sharing it keep its sha
            if e["file"].endswith("-high.onnx") and (folder / e["file"]).exists():
                e.update(_file_entry(folder / e["file"]))
        if low_gap is not None:
            model["low_vs_high_de"] = low_gap
        elif "low" in tiers:
            model.pop("low_vs_high_de", None)
        model["tiers"] = {t: model["tiers"][t] for t in naming.TIERS if t in model["tiers"]}
        tmp = model_file.with_suffix(".json.part")
        tmp.write_text(json.dumps(model, indent=2, ensure_ascii=False), encoding="utf-8")
        os.replace(tmp, model_file)
        return model
    finally:
        exported.unlink(missing_ok=True)


def _put(src: Path, dest: Path) -> None:
    """Copy as `*.part`, then rename: Lumen never reads half a file."""
    part = dest.with_name(dest.name + ".part")
    shutil.copyfile(src, part)
    os.replace(part, dest)


def _write(dest: Path, data: dict) -> None:
    part = dest.with_name(dest.name + ".part")
    part.write_text(json.dumps(data, indent=2, ensure_ascii=False), encoding="utf-8")
    os.replace(part, dest)


def install_dev(folder: Path, tier: str | None = None) -> dict:
    """Copy a saved model into the app's dev folder as `<family>/<id>/` (every distinct tier file,
    then `model.json`), touching no other model's folder. With `tier`, also the old layout: that
    tier's file + `<role>.json` at the top (read by Lumen as one model). Returns `model.json`."""
    folder = Path(folder)
    model = json.loads((folder / "model.json").read_text(encoding="utf-8"))
    entries = model.get("tiers", {})
    if not entries or (tier is not None and tier not in entries):
        raise ValueError(f"{model.get('name')} has no {tier or 'saved'!r} tier (saved: {', '.join(entries) or 'none'})")
    dev = dev_dir()
    there = dev / model["family"] / folder.name
    there.mkdir(parents=True, exist_ok=True)
    shas = {e["file"]: e["sha256"] for e in entries.values()}
    try:
        for name in shas:
            _put(folder / name, there / name)
        _write(there / "model.json", model)
        if tier is not None:
            entry = entries[tier]
            _put(folder / entry["file"], dev / entry["file"])
            _write(dev / f"{entry['role']}.json", entry)
    except PermissionError as e:
        raise RuntimeError(f"Lumen has the model open ({e.filename}): close Lumen and try again") from e
    for name, sha in shas.items():
        if _sha256(there / name) != sha:
            raise RuntimeError(f"the installed {name} does not match its entry's SHA-256")
    if tier is not None and _sha256(dev / entries[tier]["file"]) != entries[tier]["sha256"]:
        raise RuntimeError("the installed legacy file does not match its entry's SHA-256")
    return model


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--ckpt", type=Path, required=True, help="a run's best_de.pt (or best.pt)")
    ap.add_argument("--family", default="ray", choices=list(naming.FAMILIES))
    ap.add_argument("--version", required=True, help="a build like 1.0.1-d (stages d, a, b, r), or a release like 1.1 with --release")
    ap.add_argument("--release", action="store_true", help="allow a release version (x.y): only when Julian says so")
    ap.add_argument("--tiers", default=",".join(naming.TIERS), help="comma-separated: low,high,ultra")
    ap.add_argument("--root", type=Path, default=Path(os.environ.get("LUMEN_TRAIN_ROOT", "C:/lumen-train")), help="the training root (its sources.toml)")
    ap.add_argument("--out", type=Path, help="where saved models live (default <root>/models)")
    ap.add_argument("--data", type=Path, help="prepared photos to score the tiers on and calibrate int8 (default --root)")
    ap.add_argument("--calib", type=int, default=64, help="validation photos used to score and calibrate (default 64)")
    ap.add_argument("--install-dev", nargs="?", const="", metavar="TIER", help="also install every tier into Lumen's dev models folder; naming a tier also writes the old retouch.json")
    ap.add_argument("--dev-only", action="store_true", help="allow sources not cleared for commercial use; the entry says not for shipping")
    a = ap.parse_args()
    sys.stdout.reconfigure(encoding="utf-8")  # "ΔE" on a piped Windows console
    naming.model_id(a.family, a.version)  # checks the version
    if naming.is_release(a.version) and not a.release:
        ap.error(f"{a.version} is a release version; name a build like {a.version}.1-d, or pass --release when Julian says to ship")
    out = a.out or a.root / "models"
    model = save(a.ckpt, a.family, a.version, [t.strip() for t in a.tiers.split(",") if t.strip()], out, a.root, a.dev_only, a.data, a.calib)
    folder = out / naming.model_id(a.family, a.version)
    for tier, e in model["tiers"].items():
        print(f"{e['name']:<26} {e['file']:<32} {e['precision']} tta {e['tta']}  val ΔE {e.get('val_de', '—')}  sha {e['sha256'][:12]}")
    if model.get("low_vs_high_de") is not None:
        print(f"low costs {model['low_vs_high_de']:+.3f} ΔE over high")
    if a.install_dev is not None:
        install_dev(folder, a.install_dev or None)
        print(f"installed {model['name']} into {dev_dir() / a.family}: turn on Settings → Smart features → Experimental in Lumen")


if __name__ == "__main__":
    main()
