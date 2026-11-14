"""Trains Retouch.

    python -m lumen_train.retouch.train --config lumen_train/retouch/config.toml data.root=D:/lumen-train run.dir=D:/lumen-train/runs/r1 [--ship]

Each step scores a prediction four ways: its slider values against those a sample knows (a
pair knows them all; a synthetic pair knows its straighten and crop, so straighten's angle
error is its L1 here), Lumen's render of it against the target (CIEDE2000), the crop's
overlap, and whether to crop at all. The best checkpoint by validation loss is kept as
`best.pt`, the best by validation CIEDE2000 as `best_de.pt`, the latest as `last.pt` (a run resumes from it, with the same sources and licence
mode only). `--ship` refuses non-commercial sources and marks the checkpoint as shippable."""

import argparse
import math
import sys
import time
from pathlib import Path
from types import SimpleNamespace

import torch
import torch.nn.functional as F
from torch.utils.checkpoint import checkpoint
from torch.utils.data import DataLoader

from ..common import ckpt, config as config_mod
from ..common.device import describe, pick_device, seed_everything
from ..common.licence import LicenceError
from ..common.logs import Logger
from ..data import sources as sources_mod
from . import params, render, synth
from .dataset import PreparedDataset, collate
from .model import RetouchNet

DEFAULT_CONFIG = Path(__file__).resolve().parent / "config.toml"


def _fmt_duration(seconds: float) -> str:
    """Format seconds as MM:SS or H:MM:SS for terminal progress."""
    seconds = max(0, int(round(seconds)))
    hours, remainder = divmod(seconds, 3600)
    minutes, seconds = divmod(remainder, 60)
    if hours:
        return f"{hours:d}:{minutes:02d}:{seconds:02d}"
    return f"{minutes:02d}:{seconds:02d}"


def _gpu_memory(device: torch.device) -> str:
    if device.type != "cuda":
        return ""
    allocated = torch.cuda.memory_allocated(device) / (1024 ** 3)
    peak = torch.cuda.max_memory_allocated(device) / (1024 ** 3)
    return f" | VRAM {allocated:.1f}G peak {peak:.1f}G"



def _box(v: torch.Tensor) -> torch.Tensor:
    return torch.stack([params.value(v, f"crop_{k}") for k in "xywh"], dim=1)


def iou(a: torch.Tensor, b: torch.Tensor) -> torch.Tensor:
    """Overlap of (B, 4) boxes as x, y, w, h."""
    x0 = torch.maximum(a[:, 0], b[:, 0])
    y0 = torch.maximum(a[:, 1], b[:, 1])
    x1 = torch.minimum(a[:, 0] + a[:, 2], b[:, 0] + b[:, 2])
    y1 = torch.minimum(a[:, 1] + a[:, 3], b[:, 1] + b[:, 3])
    inter = (x1 - x0).clamp(min=0) * (y1 - y0).clamp(min=0)
    union = a[:, 2] * a[:, 3] + b[:, 2] * b[:, 3] - inter
    return inter / union.clamp(min=1e-6)


METRICS = {"de2000": render.delta_e, "oklab": render.oklab_distance}


def train_metric(loss_cfg: dict, epoch: int, epochs: int) -> str:
    """The colour metric an epoch trains on: `loss.train_metric`, except that an OKLab run
    switches back to CIEDE2000 (what validation scores) for its last `de2000_tail` share."""
    metric = loss_cfg.get("train_metric", "de2000")
    if metric not in METRICS:
        raise ValueError(f"loss.train_metric = {metric!r}: use one of {sorted(METRICS)}")
    tail = math.ceil(float(loss_cfg.get("de2000_tail", 0.1)) * epochs)
    return "de2000" if metric == "oklab" and epoch >= epochs - tail else metric


def scored(batch: dict) -> torch.Tensor:
    """The pixels ΔE counts: the photo, less any part a pair's target does not cover
    (`target_void`, outside the photographer's crop). Synthetic rows have no void."""
    void = batch.get("target_void")
    return batch["mask"] if void is None else batch["mask"] * (1.0 - void.to(batch["mask"].dtype))


def _weighted_mean(x: torch.Tensor, rows: torch.Tensor) -> torch.Tensor:
    return (x * rows).sum() / rows.sum().clamp(min=1e-6)


def loss_of(v: torch.Tensor, batch: dict, w: dict, hard: bool = False, metric: str = "de2000") -> tuple[torch.Tensor, dict]:
    """The loss of a batch; `batch["weight"]` (one per row, absent = 1) weighs rows against
    each other in every part (Prism: his pairs ×3)."""
    v = v.float()
    known, target = batch["v_known"], batch["v_target"]
    rows = batch.get("weight")
    rows = torch.ones(v.shape[0], device=v.device) if rows is None else rows.to(v.device, torch.float32)
    known = known * rows[:, None]
    param = ((v - target).abs() * known).sum() / known.sum().clamp(min=1.0)

    dev = params.decode(v, batch["aspect"], hard=hard)
    shown = render.render(batch["aligned"].float(), dev, render.photo_size(batch["mask"]), recompute=True)
    seen = scored(batch)
    de = checkpoint(METRICS[metric], shown, batch["target"].float(), seen, use_reentrant=False) if torch.is_grad_enabled() else METRICS[metric](shown, batch["target"].float(), seen)
    has = batch["has_target"].float() * rows
    de = (de * has).sum() / has.sum().clamp(min=1.0)

    on_i = params.index("crop_on")
    on_known = batch["v_known"][:, on_i] > 0
    p_on = v[:, on_i].clamp(1e-4, 1 - 1e-4)
    presence = _weighted_mean(F.binary_cross_entropy(p_on[on_known], target[on_known, on_i], reduction="none"), rows[on_known]) if on_known.any() else v.sum() * 0
    cropped = on_known & (target[:, on_i] > 0.5) & (batch["v_known"][:, params.index("crop_w")] > 0)
    overlap = _weighted_mean(1 - iou(_box(v[cropped]), _box(target[cropped])), rows[cropped]) if cropped.any() else v.sum() * 0

    parts = {"param": param, "de": de, "iou": overlap, "presence": presence}
    total = w["param"] * param + w["de"] * de / 10.0 + w["iou"] * overlap + w["presence"] * presence
    return total, parts


def _to(batch: dict, device) -> dict:
    return {k: (t.to(device, non_blocking=True) if isinstance(t, torch.Tensor) else t) for k, t in batch.items()}


@torch.no_grad()
def evaluate(model, loader, w: dict, device) -> dict:
    model.eval()
    sums, n, neutral = {}, 0, 0.0
    for batch in loader:
        batch = synth.realise(_to(batch, device))
        v = model(batch["input"], batch["clip"], batch["clip_flag"])
        loss, parts = loss_of(v, batch, w, hard=True)
        if not torch.isfinite(loss).item():
            raise FloatingPointError(f"Non-finite validation loss: {loss.detach().item()}")
        b = v.shape[0]
        for k, x in {"loss": loss, **parts}.items():
            sums[k] = sums.get(k, 0.0) + x.detach().item() * b
        neutral += render.delta_e(batch["aligned"], batch["target"], scored(batch)).mean().item() * b
        n += b
    out = {k: s / max(n, 1) for k, s in sums.items()}
    out["de_untouched"] = neutral / max(n, 1)
    return out


def check_resume(extra: dict, sources: list[str], ship: bool) -> None:
    """A run resumes only as what it started as: a checkpoint that saw a non-commercial
    source never becomes shippable by resuming it with `--ship`."""
    if extra.get("layout") != params.LAYOUT:
        raise ValueError(f"last.pt predicts layout {extra.get('layout')}, this kit {params.LAYOUT}: start a new run.dir")
    if ship and not extra.get("ship", False):
        raise LicenceError("last.pt trained without --ship (its sources were not all checked for shipping): start a new run.dir")
    if sorted(extra.get("sources", [])) != sorted(sources) or bool(extra.get("ship", False)) != ship:
        raise ValueError(f"last.pt trained on {extra.get('sources')} (ship={extra.get('ship')}), not {sources} (ship={ship}): start a new run.dir or set run.resume=false")


def resolve_sources(root: Path, names: list[str], ship: bool, resume_from: Path | None = None) -> list[str]:
    """`["all"]` is every source with prepared photos under `root` (only commercial ones when
    shipping), never one of unedited RAWs (kind `raw`: a flat render is no goal); any other
    list is taken as written. A run resumed from `resume_from` keeps the list it started with,
    so a source prepared since does not stop it."""
    if names != ["all"]:
        return list(names)
    if resume_from is not None and resume_from.exists():
        return list(ckpt.read(resume_from)["extra"]["sources"])
    reg = sources_mod.merged_entries(root)
    found = sorted(
        n for n in reg
        if reg[n].get("kind") != "raw"
        and n in {p.parent.parent.name for p in (root / n).glob("*/input.u16")}
        and (not ship or reg[n].get("commercial", False))
    )
    if not found:
        raise ValueError(f"no prepared source under {root}: run `python -m lumen_train.data prep`")
    return found


def load_init(model: torch.nn.Module, path: Path | str, device) -> None:
    """Warm start: the weights of an earlier run, nothing else (a fresh optimiser and schedule)."""
    state = ckpt.read(path, device)
    layout = state.get("extra", {}).get("layout")
    if layout != params.LAYOUT:
        raise ValueError(f"{path} predicts layout {layout}, this kit {params.LAYOUT}: it cannot start this run")
    model.load_state_dict(state["model"])


def _build(cfg: dict) -> SimpleNamespace:
    """The pieces of a run (data, model, optimiser, schedule, scaler), shared by `run` and the bench."""
    t = cfg["train"]
    seed_everything(int(t["seed"]))
    device = pick_device() if t.get("device", "auto") == "auto" else torch.device(t["device"])
    ship = bool(cfg["run"].get("ship", False))
    root = Path(cfg["data"]["root"])
    last = Path(cfg["run"]["dir"]) / "last.pt" if cfg["run"].get("resume", True) else None
    sources = resolve_sources(root, list(cfg["data"]["sources"]), ship, resume_from=last)
    # Lazy: synthetic damage is rendered for the whole batch on the device (`synth.realise`).
    cache = bool(cfg["data"].get("cache", False))
    # Already-fine samples train only: validation stays comparable with earlier runs.
    identity_p = float(cfg.get("synth", {}).get("identity_p", 0.0))
    train_ds = PreparedDataset(root, sources, split="train", seed=int(t["seed"]), ship=ship, lazy=True, cache=cache, identity_p=identity_p)
    val_ds = PreparedDataset(root, sources, split="val", seed=int(t["seed"]), ship=ship, lazy=True, cache=cache)
    if len(train_ds) == 0 or len(val_ds) == 0:
        raise ValueError(f"{len(train_ds)} training and {len(val_ds)} validation photos under {root} for {sources}: prepare more (retouch_prep)")
    workers = int(cfg["data"].get("workers", 0))
    loader = DataLoader(train_ds, batch_size=int(t["batch"]), shuffle=True, num_workers=workers, drop_last=len(train_ds) > int(t["batch"]), persistent_workers=False, pin_memory=device.type == "cuda", collate_fn=collate)
    val_loader = DataLoader(val_ds, batch_size=int(t["batch"]), num_workers=workers, collate_fn=collate)

    model = RetouchNet().to(device)
    if t.get("init"):
        load_init(model, t["init"], device)
    opt = torch.optim.AdamW(model.parameters(), lr=float(t["lr"]), weight_decay=float(t["weight_decay"]))
    epochs = int(t["epochs"])
    steps_per_epoch = max(1, len(loader))
    warmup = int(t.get("warmup", 0)) * steps_per_epoch
    total = epochs * steps_per_epoch
    sched = torch.optim.lr_scheduler.LambdaLR(opt, lambda s: (s + 1) / warmup if s < warmup else 0.5 * (1 + math.cos(math.pi * min(1.0, (s - warmup) / max(1, total - warmup)))))
    amp = bool(t.get("amp", False)) and device.type == "cuda"
    scaler = torch.amp.GradScaler("cuda", enabled=amp)
    return SimpleNamespace(
        device=device, ship=ship, sources=sources, train_ds=train_ds, val_ds=val_ds, loader=loader, val_loader=val_loader,
        model=model, opt=opt, sched=sched, scaler=scaler, amp=amp, epochs=epochs, steps_per_epoch=steps_per_epoch,
    )


def _forward(b: SimpleNamespace, batch: dict, w: dict, metric: str = "de2000") -> tuple[torch.Tensor, dict]:
    """One training batch's loss: the synthetic rows made into pairs on the device, the model
    under autocast, the render and ΔE in float32."""
    batch = synth.realise(_to(batch, b.device))
    with torch.autocast(device_type=b.device.type, dtype=torch.float16, enabled=b.amp):
        v = b.model(batch["input"], batch["clip"], batch["clip_flag"])
    return loss_of(v, batch, w, metric=metric)


def _update(b: SimpleNamespace, loss: torch.Tensor) -> None:
    b.opt.zero_grad(set_to_none=True)
    b.scaler.scale(loss).backward()
    b.scaler.unscale_(b.opt)
    torch.nn.utils.clip_grad_norm_(b.model.parameters(), 1.0)
    b.scaler.step(b.opt)
    b.scaler.update()
    b.sched.step()


def run(cfg: dict) -> dict:
    b = _build(cfg)
    device, ship, sources, train_ds, val_ds = b.device, b.ship, b.sources, b.train_ds, b.val_ds
    loader, val_loader, model, opt, sched, scaler = b.loader, b.val_loader, b.model, b.opt, b.sched, b.scaler
    amp, epochs, steps_per_epoch = b.amp, b.epochs, b.steps_per_epoch

    run_dir = Path(cfg["run"]["dir"])
    start, best, best_de, history = 0, float("inf"), float("inf"), []
    last = run_dir / "last.pt"
    if cfg["run"].get("resume", True) and last.exists():
        state = ckpt.load(last, model, opt, map_location=device)
        check_resume(state["extra"], sources, ship)
        start, best = int(state["epoch"]) + 1, float(state["best"])
        best_de = float(state["extra"].get("best_de", float("inf")))  # absent before Round 12-1
        history = state["extra"].get("val", [])
        sched.load_state_dict(state["extra"]["sched"])
        if amp and state["extra"].get("scaler"):  # empty when the run started without AMP
            scaler.load_state_dict(state["extra"]["scaler"])
    started_at = start
    log = Logger(run_dir, tensorboard=device.type == "cuda")
    # Whether the model ever saw a CLIP embedding: without one, the app must pass zeros and
    # flag 0 (the export records it).
    clip_share = train_ds.clip_share()
    extra = lambda: {"sources": sources, "ship": ship, "layout": params.LAYOUT, "clip_share": clip_share, "config": cfg, "val": history}
    baseline_val_seconds = 0.0
    if start == 0:
        print("Baseline validation...", end="", flush=True)
        baseline_started = time.perf_counter()
        baseline = evaluate(model, val_loader, cfg["loss"], device)
        baseline_val_seconds = time.perf_counter() - baseline_started
        history.append(baseline)
        print(
            f"\rBaseline: val loss {baseline['loss']:.4f}, ΔE {baseline['de']:.2f} "
            f"(untouched {baseline['de_untouched']:.2f}) in {_fmt_duration(baseline_val_seconds)}",
            flush=True,
        )

    print(
        f"Retouch: {len(train_ds)} train / {len(val_ds)} val on {describe(device)}; "
        f"epochs {start}–{epochs - 1}; CLIP on {clip_share:.0%}; "
        f"{steps_per_epoch} batches/epoch",
        flush=True,
    )

    run_started = time.perf_counter()
    val_seconds_estimate = baseline_val_seconds
    completed_epoch_seconds: list[float] = []

    val_every = max(1, int(cfg["train"].get("val_every", 1)))
    for epoch in range(start, epochs):
        metric = train_metric(cfg["loss"], epoch, epochs)
        train_ds.set_epoch(epoch)
        model.train()
        epoch_started = time.perf_counter()
        if device.type == "cuda":
            torch.cuda.reset_peak_memory_stats(device)

        print(f"\nEpoch {epoch + 1}/{epochs}", flush=True)

        for i, batch in enumerate(loader):
            batch_started = time.perf_counter()
            loss, parts = _forward(b, batch, cfg["loss"], metric)

            if not torch.isfinite(loss).item():
                raise FloatingPointError(
                    f"Non-finite training loss at epoch={epoch}, batch={i}, "
                    f"step={epoch * steps_per_epoch + i}: {loss.detach().item()}"
                )

            _update(b, loss)

            step = epoch * steps_per_epoch + i
            loss_value = loss.detach().item()
            if step % 20 == 0:
                log.log(
                    step,
                    loss=loss_value,
                    **{k: x.detach().item() for k, x in parts.items()},
                    lr=sched.get_last_lr()[0],
                )

            now = time.perf_counter()
            done = i + 1
            train_elapsed = now - epoch_started
            run_elapsed = now - run_started
            batch_seconds = now - batch_started
            avg_batch_seconds = train_elapsed / max(done, 1)
            train_eta = avg_batch_seconds * max(steps_per_epoch - done, 0)
            epoch_eta = train_eta + val_seconds_estimate

            if completed_epoch_seconds:
                avg_epoch_seconds = sum(completed_epoch_seconds) / len(completed_epoch_seconds)
            else:
                avg_epoch_seconds = avg_batch_seconds * steps_per_epoch + val_seconds_estimate

            future_epochs = max(epochs - epoch - 1, 0)
            run_eta = epoch_eta + future_epochs * avg_epoch_seconds
            pct = 100.0 * done / max(steps_per_epoch, 1)
            lr = sched.get_last_lr()[0]

            line = (
                f"  {done:>3}/{steps_per_epoch:<3} {pct:5.1f}%"
                f" | loss {loss_value:.4f}"
                f" | batch {_fmt_duration(batch_seconds)}"
                f" | elapsed {_fmt_duration(run_elapsed)}"
                f" | epoch ETA {_fmt_duration(epoch_eta)}"
                f" | run ETA {_fmt_duration(run_eta)}"
                f" | lr {lr:.2e}"
                f"{_gpu_memory(device)}"
            )
            print("\r" + line, end="", flush=True)

        print(flush=True)
        train_seconds = time.perf_counter() - epoch_started

        if epoch % val_every != val_every - 1 and epoch != epochs - 1:
            ckpt.save(last, model, opt, epoch, best, {**extra(), "best_de": best_de, "sched": sched.state_dict(), "scaler": scaler.state_dict()})
            completed_epoch_seconds.append(time.perf_counter() - epoch_started)
            print(f"Epoch {epoch + 1}/{epochs} complete | train {_fmt_duration(train_seconds)} ({metric}, no validation)", flush=True)
            continue

        print("  validating...", end="", flush=True)
        val_started = time.perf_counter()
        val = evaluate(model, val_loader, cfg["loss"], device)
        val_seconds = time.perf_counter() - val_started
        if val_seconds_estimate <= 0:
            val_seconds_estimate = val_seconds
        else:
            val_seconds_estimate = 0.7 * val_seconds_estimate + 0.3 * val_seconds

        history.append(val)
        log.log((epoch + 1) * steps_per_epoch, **{f"val_{k}": x for k, x in val.items()})

        epoch_seconds = time.perf_counter() - epoch_started
        completed_epoch_seconds.append(epoch_seconds)
        remaining_epochs = max(epochs - epoch - 1, 0)
        avg_epoch_seconds = sum(completed_epoch_seconds) / len(completed_epoch_seconds)
        post_epoch_eta = remaining_epochs * avg_epoch_seconds

        print(
            f"\rEpoch {epoch + 1}/{epochs} complete"
            f" | val loss {val['loss']:.4f}"
            f" | ΔE {val['de']:.2f} (untouched {val['de_untouched']:.2f})"
            f" | train {_fmt_duration(train_seconds)}"
            f" | val {_fmt_duration(val_seconds)}"
            f" | total {_fmt_duration(epoch_seconds)}"
            f" | remaining ~{_fmt_duration(post_epoch_eta)}",
            flush=True,
        )

        if val["loss"] < best:
            best = val["loss"]
            ckpt.save(run_dir / "best.pt", model, None, epoch, best, extra())
        if val["de"] < best_de:
            best_de = val["de"]
            ckpt.save(run_dir / "best_de.pt", model, None, epoch, best, {**extra(), "val_de": best_de})
        ckpt.save(last, model, opt, epoch, best, {**extra(), "best_de": best_de, "sched": sched.state_dict(), "scaler": scaler.state_dict()})

    print(f"\nTraining finished in {_fmt_duration(time.perf_counter() - run_started)}", flush=True)
    log.close()
    return {"epochs": epochs, "started_at": started_at, "best": best, "best_de": best_de, "val": history}


def main() -> None:
    # Progress lines hold ΔE and dashes: redirected to a file on Windows, stdout would be
    # cp1252 and the first epoch's line would end the run.
    sys.stdout.reconfigure(encoding="utf-8", errors="replace")
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--config", type=Path, default=DEFAULT_CONFIG)
    ap.add_argument("--ship", action="store_true", help="refuse non-commercial sources; the result may ship")
    ap.add_argument("overrides", nargs="*", help="section.key=value")
    a = ap.parse_args()
    cfg = config_mod.load(a.config, a.overrides)
    if a.ship:
        cfg["run"]["ship"] = True
    run(cfg)


if __name__ == "__main__":
    main()
