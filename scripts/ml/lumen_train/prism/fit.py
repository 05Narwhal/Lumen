"""One Prism fit: Retouch started from a checkpoint that never saw `julian-pairs`, then trained
on his pairs (many views each, `augment.PrismPairs`) mixed 1 : 3 with Wikimedia's synthetic
pairs, his rows weighted ×3 in the loss.

The encoder (`stem`, `stages`) is frozen for the first `train.freeze` share of the epochs, then
trains at `train.encoder_lr` × the heads' rate; the rate falls on a cosine. The weights' EMA is
what is scored (on the held-out folders, each epoch) and saved."""

import math
from pathlib import Path
from types import SimpleNamespace

import torch
from torch.optim.swa_utils import AveragedModel, get_ema_multi_avg_fn
from torch.utils.data import ConcatDataset, DataLoader, Dataset, Sampler

from ..common import ckpt
from ..common.device import describe, pick_device, seed_everything
from ..retouch import params, train
from ..retouch.dataset import PreparedDataset, collate
from ..retouch.model import RetouchNet
from . import augment
from .folds import PAIRS


class LeakError(ValueError):
    """A warm start that saw (or may have seen) Julian's pairs: cross-validation would score
    photos the model already knew."""


def leak_free(path: Path | str) -> list[Path]:
    """The warm-start chain from `path` back through each checkpoint's `config.train.init`,
    refused if any link trained on `julian-pairs`, records no sources, or is gone."""
    chain: list[Path] = []
    p = Path(path)
    while True:
        if not p.exists():
            raise LeakError(f"{p} is gone: the warm start's history cannot be checked")
        if any(p.resolve() == q.resolve() for q in chain):
            return chain
        extra = ckpt.read(p).get("extra", {})
        sources = extra.get("sources")
        if not sources:
            raise LeakError(f"{p} records no sources: start from a run that does (r2-pre's best_de.pt)")
        if PAIRS in sources:
            raise LeakError(f"{p} trained on {PAIRS}: start from a run that never saw them (r2-pre's best_de.pt)")
        chain.append(p)
        init = ((extra.get("config") or {}).get("train") or {}).get("init")
        if not init:
            return chain
        p = Path(init)


def encoder_factor(epoch: int, epochs: int, freeze: float, after: float) -> float:
    """The encoder's share of the heads' learning rate in `epoch`: 0 while frozen."""
    return 0.0 if epoch < round(freeze * epochs) else after


class Weighted(Dataset):
    """A dataset whose items carry `weight` (the loss's per-row weight)."""

    def __init__(self, ds, weight: float):
        self.ds, self.weight = ds, torch.tensor(float(weight))

    def set_epoch(self, epoch: int) -> None:
        self.ds.set_epoch(epoch)

    def __len__(self) -> int:
        return len(self.ds)

    def __getitem__(self, i: int) -> dict:
        return {**self.ds[i], "weight": self.weight}


class MixSampler(Sampler):
    """Batches over `ConcatDataset([pairs, others])`: a quarter of each batch his pairs (every
    pair once an epoch, leftovers skipped), the rest the others, drawn in turn."""

    def __init__(self, n_pairs: int, n_wiki: int, batch: int, seed: int = 0):
        if batch < 4 or n_pairs < 1 or n_wiki < 1:
            raise ValueError(f"batch {batch} over {n_pairs} pairs and {n_wiki} others: needs batch ≥ 4 and both kinds")
        self.n_pairs, self.n_wiki, self.batch, self.seed, self.epoch = n_pairs, n_wiki, batch, seed, 0
        self.per = batch // 4

    def set_epoch(self, epoch: int) -> None:
        self.epoch = epoch

    def __len__(self) -> int:
        return max(1, self.n_pairs // self.per)

    def __iter__(self):
        gen = torch.Generator().manual_seed(self.seed * 1_000_003 + self.epoch)
        pairs = torch.randperm(self.n_pairs, generator=gen).tolist()
        others: list[int] = []
        for b in range(len(self)):
            need = self.batch - self.per
            while len(others) < need:
                others += torch.randperm(self.n_wiki, generator=gen).tolist()
            mine, others = others[:need], others[need:]
            yield pairs[b * self.per:(b + 1) * self.per] + [self.n_pairs + i for i in mine]


def fit(cfg: dict, train_folders: list[Path], heldout_folders: list[Path], run_dir: Path | str, fold: int | None = None, epochs: int | None = None) -> dict:
    """Trains on `train_folders` (his pairs) + the Wikimedia sources; scores the EMA on
    `heldout_folders` each epoch (none: the final fit, saved at its last epoch). Returns
    `{"history": [the warm start's ΔE, then each epoch's], "best_epoch", "best_de"}`."""
    t, d = cfg["train"], cfg["data"]
    init = t.get("init")
    if not init:
        raise LeakError("train.init is empty: Prism starts from a run that never saw his pairs (r2-pre's best_de.pt)")
    leak_free(init)
    seed = int(t["seed"])
    seed_everything(seed)
    device = pick_device() if t.get("device", "auto") == "auto" else torch.device(t["device"])
    root, source, wiki = Path(d["root"]), d.get("pairs", PAIRS), list(d["wiki"])
    epochs = int(epochs or t["epochs"])
    run_dir = Path(run_dir)

    pairs = augment.PrismPairs(augment.pair_dataset(root, train_folders, source), views=int(d.get("views", 8)), seed=seed)
    others = PreparedDataset(root, wiki, split="train", seed=seed, lazy=True, cache=bool(d.get("cache", False)), identity_p=float(cfg.get("synth", {}).get("identity_p", 0.0)))
    sampler = MixSampler(len(pairs), len(others), int(t["batch"]), seed)
    workers = int(d.get("workers", 0))
    loader = DataLoader(ConcatDataset([Weighted(pairs, float(t.get("pair_weight", 3.0))), Weighted(others, 1.0)]), batch_sampler=sampler, num_workers=workers, collate_fn=collate, pin_memory=device.type == "cuda")
    held = DataLoader(augment.pair_dataset(root, heldout_folders, source), batch_size=int(t["batch"]), num_workers=workers, collate_fn=collate) if heldout_folders else None

    model = RetouchNet().to(device)
    train.load_init(model, init, device)
    encoder = [*model.stem.parameters(), *model.stages.parameters()]
    mine = {id(p) for p in encoder}
    heads = [p for p in model.parameters() if id(p) not in mine]
    lr = float(t["lr"])
    opt = torch.optim.AdamW([{"params": encoder, "lr": lr}, {"params": heads, "lr": lr}], weight_decay=float(t["weight_decay"]))
    steps = len(sampler)
    total = epochs * steps
    cosine = lambda s: 0.5 * (1 + math.cos(math.pi * min(1.0, s / max(1, total))))
    factor = lambda e: encoder_factor(e, epochs, float(t.get("freeze", 1 / 3)), float(t.get("encoder_lr", 0.1)))
    sched = torch.optim.lr_scheduler.LambdaLR(opt, [lambda s: cosine(s) * factor(s // steps), cosine])
    amp = bool(t.get("amp", False)) and device.type == "cuda"
    b = SimpleNamespace(device=device, amp=amp, model=model, opt=opt, sched=sched, scaler=torch.amp.GradScaler("cuda", enabled=amp))
    ema = AveragedModel(model, multi_avg_fn=get_ema_multi_avg_fn(float(t.get("ema", 0.999))), use_buffers=True)

    n_clip = len(pairs.base) + len(others)
    clip_share = (pairs.base.clip_share() * len(pairs.base) + others.clip_share() * len(others)) / max(1, n_clip)
    history: list[float] = []
    extra = lambda: {
        "sources": [source, *wiki], "ship": False, "layout": params.LAYOUT, "clip_share": clip_share, "config": cfg,
        "val": history, "prism": {"fold": fold, "epochs": epochs, "pairs": len(train_folders)},
    }
    if held is not None:
        history.append(train.evaluate(model, held, cfg["loss"], device)["de"])
        print(f"Prism fold {fold}: warm start ΔE {history[0]:.2f} on {len(heldout_folders)} held-out pairs", flush=True)
    print(f"Prism: {len(train_folders)} pairs × {pairs.views} views + {len(others)} others on {describe(device)}; {epochs} epochs of {steps} batches", flush=True)

    best_de, best_epoch = float("inf"), 0
    for epoch in range(epochs):
        frozen = factor(epoch) == 0.0
        for p in encoder:
            p.requires_grad_(not frozen)
        for ds in (pairs, others, sampler):
            ds.set_epoch(epoch)
        model.train()
        metric = train.train_metric(cfg["loss"], epoch, epochs)
        for i, batch in enumerate(loader):
            loss, _ = train._forward(b, batch, cfg["loss"], metric)
            if not torch.isfinite(loss).item():
                raise FloatingPointError(f"Non-finite training loss at epoch={epoch}, batch={i}: {loss.detach().item()}")
            train._update(b, loss)
            ema.update_parameters(model)
        line = f"Epoch {epoch + 1}/{epochs}{' (encoder frozen)' if frozen else ''} | loss {loss.detach().item():.4f}"
        if held is not None:
            de = train.evaluate(ema.module, held, cfg["loss"], device)["de"]
            history.append(de)
            line += f" | held-out ΔE {de:.2f}"
            if de < best_de:
                best_de, best_epoch = de, epoch + 1
                ckpt.save(run_dir / "best_de.pt", ema.module, None, epoch, de, {**extra(), "val_de": de})
        print(line, flush=True)
    ckpt.save(run_dir / "last.pt", ema.module, None, epochs - 1, best_de, extra())
    if held is None:
        best_epoch = epochs
        ckpt.save(run_dir / "best_de.pt", ema.module, None, epochs - 1, best_de, extra())
    return {"history": history, "best_epoch": best_epoch, "best_de": best_de}
