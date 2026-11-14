"""Many views of each of Julian's pairs: the photo he started from is shot a little darker,
warmer, flatter…, and his edit stays the target, so Prism learns to reach his look from any
starting point.

A view changes the source side only: `aligned` (what the predicted sliders render) and
`input` (what the model reads, re-encoded from it); `target`, `target_void` and `mask` stay
bit-identical. Once the source changed, the sliders a pair knew for his look no longer lead
from it to his edit, so a view keeps only the geometry it knew (an aligned pair knows only that
anyway). Geometry changes (the mirror) go through `PreparedDataset.flip`, which moves both
sides. No crop: an aligned pair knows its crop box and ratio, which a crop would have to remap."""

from dataclasses import dataclass
from pathlib import Path

import torch
from torch.utils.data import Dataset

from ..retouch import params
from ..retouch.dataset import PreparedDataset
from ..retouch.prepared import decode, encode_input

GEOMETRY = ("straighten", "crop_on", "crop_x", "crop_y", "crop_w", "crop_h", "crop_ratio")
_GEOMETRY = [params.index(n) for n in GEOMETRY]
# The brightest value `input.u16` holds; the app clamps there too.
_TOP = float(decode(torch.tensor(1.0)))
PIVOT = 0.18  # contrast turns around middle grey
# Gains per 800 K and per 10 tint: roughly what Lightroom's sliders do to R/B and G near daylight.
_KELVIN_GAIN = 0.12
_TINT_GAIN = 0.04


@dataclass(frozen=True)
class Look:
    """A change to the photo he started from: EV, white balance (K, tint) and contrast."""

    ev: float = 0.0
    kelvin: float = 0.0
    tint: float = 0.0
    contrast: float = 1.0

    @staticmethod
    def draw(gen: torch.Generator) -> "Look":
        u = torch.rand(4, generator=gen).tolist()
        return Look(ev=2 * u[0] - 1, kelvin=1600 * u[1] - 800, tint=20 * u[2] - 10, contrast=0.8 + 0.4 * u[3])


def apply(lin: torch.Tensor, look: Look) -> torch.Tensor:
    """(3, H, W) linear light with `look`; zero stays zero (the padding outside the photo)."""
    k, t = look.kelvin / 800.0, look.tint / 10.0
    gains = torch.tensor([_KELVIN_GAIN * k, -_TINT_GAIN * t, -_KELVIN_GAIN * k], dtype=lin.dtype).exp().view(3, 1, 1)
    x = lin.clamp(min=0.0) * gains * 2.0 ** look.ev
    x = PIVOT * (x / PIVOT).pow(look.contrast)
    return x.clamp(max=_TOP)


def photometric(item: dict, gen: torch.Generator) -> dict:
    """A pair item (`PreparedDataset._pair`) seen from another starting point."""
    mask = item["mask"]
    w, h = int(mask[0, 0].sum()), int(mask[0, :, 0].sum())
    aligned = apply(item["aligned"], Look.draw(gen)) * mask
    known = torch.zeros_like(item["v_known"])
    known[_GEOMETRY] = item["v_known"][_GEOMETRY]
    return {**item, "aligned": aligned, "input": encode_input(aligned[:, :h, :w], w, h), "v_known": known}


def pair_dataset(root: Path | str, folders: list[Path], source: str = "julian-pairs") -> PreparedDataset:
    """The source's pairs, only `folders`, unaugmented (views come from `PrismPairs`)."""
    ds = PreparedDataset(root, [source], split="all", augment=False)
    wanted = {Path(f).name for f in folders}
    keep = [i for i, f in enumerate(ds.samples) if f.name in wanted]
    if len(keep) != len(wanted):
        raise ValueError(f"{len(wanted) - len(keep)} of the folders are not prepared pairs of {source}")
    ds.samples = [ds.samples[i] for i in keep]
    ds.sources = [ds.sources[i] for i in keep]
    return ds


class PrismPairs(Dataset):
    """`views` augmented views of every pair of `base`, new ones each epoch, the same for the
    same seed and epoch. Each view is mirrored half the time."""

    def __init__(self, base: PreparedDataset, views: int = 8, seed: int = 0):
        self.base, self.views, self.seed, self.epoch = base, int(views), int(seed), 0

    def set_epoch(self, epoch: int) -> None:
        self.epoch = epoch

    def __len__(self) -> int:
        return len(self.base) * self.views

    def __getitem__(self, i: int) -> dict:
        gen = torch.Generator().manual_seed(self.seed * 1_000_003 + self.epoch * 10_007 + i)
        item = photometric(self.base[i // self.views], gen)
        if float(torch.rand(1, generator=gen)) < 0.5:
            item = self.base.flip(item)
        return item
