"""Lumen Prism's cross-validation, final fit and gate.

    python -m lumen_train.prism.cv folds  [--fold i] --config lumen_train/prism/config.toml data.root=C:/lumen-train train.init=C:/lumen-train/runs/r2-pre/best_de.pt run.dir=C:/lumen-train/runs/prism-1
    python -m lumen_train.prism.cv final  (same overrides)
    python -m lumen_train.prism.cv report (same overrides)

`folds` trains one model per fold of his non-test pairs (grouped by photo) and scores it on the
held-out fold each epoch, beside the warm start ("base") and Auto on the same photos; a fold
already finished (`fold-<i>/fold.json`) is skipped, so a stopped run picks up where it was.
`cv.json` holds every fold, the median best epoch and each method's mean ± std ΔE. Prism's
number per fold is its ΔE at that median epoch (the best epoch of each fold, chosen on the fold
itself, is reported as `prism_best` but would flatter it). `final` trains on every non-test pair
for the median epoch count into `final/best_de.pt`; `report` prints the table and the gate."""

import argparse
import json
import statistics
import subprocess
import sys
import tempfile
from pathlib import Path

from ..common import config as config_mod
from ..retouch import eval as ev, render, train
from . import augment, fit, folds

DEFAULT_CONFIG = Path(__file__).resolve().parent / "config.toml"


def _source(cfg: dict) -> str:
    return cfg["data"].get("pairs", folds.PAIRS)


def _auto_de(cfg: dict, out: Path, by_stem: dict[str, list[Path]], auto_basic: ev.AutoBasic | None) -> dict[str, float]:
    """Auto's ΔE per pair folder (cached in `auto.json`); empty when Auto cannot run."""
    cache = out / "auto.json"
    if cache.exists():
        return json.loads(cache.read_text(encoding="utf-8"))
    source = _source(cfg)
    ds = augment.pair_dataset(cfg["data"]["root"], folds.folders(by_stem, sorted(by_stem)), source)
    try:
        with tempfile.TemporaryDirectory() as tmp:
            autos = ev._autos([(source, ds)], auto_basic, Path(tmp))
    except (OSError, RuntimeError, ValueError, subprocess.SubprocessError) as e:  # retouch_auto cannot be built or run
        print(f"Auto could not run ({e}): its column stays empty and the gate cannot pass", flush=True)
        return {}
    got: dict[str, float] = {}
    for i, folder in enumerate(ds.samples):
        if (source, i) in autos:
            b = ev._batch(ds[i])
            shown = render.render(b["aligned"], render.from_develop_json({"basic": autos[(source, i)]}), render.photo_size(b["mask"]))
            got[folder.name] = float(render.delta_e(shown, b["target"], train.scored(b))[0])
    out.mkdir(parents=True, exist_ok=True)
    cache.write_text(json.dumps(got, indent=1), encoding="utf-8")
    return got


def _spread(xs: list[float]) -> dict:
    if not xs:
        return {"mean": None, "std": None}
    return {"mean": statistics.fmean(xs), "std": statistics.stdev(xs) if len(xs) > 1 else 0.0}


def summarise(results: list[dict], k: int) -> dict:
    """The folds with Prism's ΔE at the median best epoch, and each method's mean ± std."""
    median = int(statistics.median_low([r["best_epoch"] for r in results])) if results else 0
    rows = [{**r, "prism": r["history"][min(median, len(r["history"]) - 1)]} for r in results]
    return {
        "k": k,
        "finished": len(rows),
        "median_epoch": median,
        "folds": rows,
        "prism": _spread([r["prism"] for r in rows]),
        "prism_best": _spread([r["prism_best"] for r in rows]),
        "base": _spread([r["base"] for r in rows]),
        "auto": _spread([r["auto"] for r in rows if r["auto"] is not None]),
    }


def run(cfg: dict, k: int | None = None, auto_basic: ev.AutoBasic | None = ev.rust_auto, only: int | None = None) -> dict:
    """Every fold (or only fold `only`), skipping finished ones; writes `cv.json` once all are in."""
    out = Path(cfg["run"]["dir"])
    k = int(k or cfg["train"].get("folds", 5))
    by_stem = folds.pairs(cfg["data"]["root"], _source(cfg))
    groups = folds.of(sorted(by_stem), k)
    autos = _auto_de(cfg, out, by_stem, auto_basic)
    results = []
    for i, held in enumerate(groups):
        done = out / f"fold-{i}" / "fold.json"
        if done.exists():
            r = json.loads(done.read_text(encoding="utf-8"))
            if r["stems"] != held:
                raise ValueError(f"{done} holds other photos than fold {i} has now: start a new run.dir")
            results.append(r)
            continue
        if only is not None and i != only:
            continue
        rest = [s for j, g in enumerate(groups) if j != i for s in g]
        res = fit.fit(cfg, folds.folders(by_stem, rest), folds.folders(by_stem, held), out / f"fold-{i}", fold=i)
        hist = res["history"]
        held_autos = [autos[f.name] for f in folds.folders(by_stem, held) if f.name in autos]
        r = {
            "fold": i, "stems": held, "history": hist, "base": hist[0], "prism_best": res["best_de"], "best_epoch": res["best_epoch"],
            "auto": statistics.fmean(held_autos) if held_autos else None, "auto_count": len(held_autos),
        }
        done.write_text(json.dumps(r, indent=1), encoding="utf-8")
        results.append(r)
    report = summarise(results, k)
    if len(results) == k:
        (out / "cv.json").write_text(json.dumps(report, indent=1), encoding="utf-8")
    return report


def final(cfg: dict) -> dict:
    """Every non-test pair, for the CV's median best epoch, into `final/`."""
    out = Path(cfg["run"]["dir"])
    report_file = out / "cv.json"
    if not report_file.exists():
        raise FileNotFoundError(f"{report_file} is missing: run `folds` to the end first")
    report = json.loads(report_file.read_text(encoding="utf-8"))
    by_stem = folds.pairs(cfg["data"]["root"], _source(cfg))
    return fit.fit(cfg, folds.folders(by_stem, sorted(by_stem)), [], out / "final", fold=None, epochs=report["median_epoch"])


def gate(report: dict) -> tuple[bool, str]:
    """Whether Prism may be exported: its CV ΔE below the warm start's and Auto's."""
    p, b, a = (report[k]["mean"] for k in ("prism", "base", "auto"))
    if report.get("finished", 0) < report.get("k", 0):
        return False, f"only {report['finished']} of {report['k']} folds have finished"
    if a is None:
        return False, "Auto has no score (build the retouch_auto example and rerun `folds`)"
    if p >= b:
        return False, f"Prism {p:.2f} is no better than its warm start {b:.2f}"
    if p >= a:
        return False, f"Prism {p:.2f} is no better than Auto {a:.2f}"
    return True, f"Prism {p:.2f} beats its warm start {b:.2f} and Auto {a:.2f}"


def markdown(report: dict) -> str:
    f = lambda x: "—" if x is None else f"{x:.2f}"
    s = lambda d: f"{f(d['mean'])} ± {f(d['std'])}"
    lines = [
        f"# Lumen Prism cross-validation ({report['finished']}/{report['k']} folds, median best epoch {report['median_epoch']})",
        "",
        "| Fold | Photos | Prism ΔE | (best epoch) | Warm start ΔE | Auto ΔE |",
        "|---|---|---|---|---|---|",
        *(f"| {r['fold']} | {len(r['stems'])} | {f(r['prism'])} | {f(r['prism_best'])} ({r['best_epoch']}) | {f(r['base'])} | {f(r['auto'])} |" for r in report["folds"]),
        f"| **All** | | **{s(report['prism'])}** | {s(report['prism_best'])} | {s(report['base'])} | {s(report['auto'])} |",
        "",
        "Gate: " + ("pass — " if gate(report)[0] else "fail — ") + gate(report)[1],
    ]
    return "\n".join(lines) + "\n"


def main() -> None:
    sys.stdout.reconfigure(encoding="utf-8", errors="replace")
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("command", choices=["folds", "final", "report"])
    ap.add_argument("--config", type=Path, default=DEFAULT_CONFIG)
    ap.add_argument("--fold", type=int, help="folds: only this fold")
    ap.add_argument("overrides", nargs="*", help="section.key=value")
    a = ap.parse_intermixed_args()  # `folds --fold 2 data.root=…`: overrides after an option
    cfg = config_mod.load(a.config, a.overrides)
    if a.command == "folds":
        report = run(cfg, only=a.fold)
        print(markdown(report))
    elif a.command == "final":
        final(cfg)
    else:
        report = json.loads((Path(cfg["run"]["dir"]) / "cv.json").read_text(encoding="utf-8"))
        text = markdown(report)
        (Path(cfg["run"]["dir"]) / "cv.md").write_text(text, encoding="utf-8")
        print(text)
        sys.exit(0 if gate(report)[0] else 1)


if __name__ == "__main__":
    main()
