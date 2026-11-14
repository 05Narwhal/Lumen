"""Retouch's training data: folders that `retouch_prep` wrote, one folder per registry source
(`<root>/<source>/<photo>/`). A source of finished photos or RAWs becomes synthetic pairs
(`synth.py`), a fresh degradation each epoch. A folder with `target.json` (from
`build_pairs.py`) is a real pair: the original and a photographer's slider values, plus
optionally `target.u16`, the target rendered by Lumen.

A source of kind `pairs` or `raw+jpeg` contributes only its folders with `target.json`: its other photos
are originals nobody finished, never targets.

Splits are `data/splits.py`: a hash of each photo's stem and its source's shares, so a RAW,
its JPEG and its edited export (`IMG_1.CR3`, `IMG_1.JPG`, `IMG_1-Edit.jpg`) land on one side
and adding photos never moves an old one."""

import json
from pathlib import Path

import numpy as np
import torch
from torch.utils.data import Dataset, default_collate

from ..common import licence
from ..data import sources as sources_mod, splits
from . import params, synth
from .prepared import EDGE, decode_input, encode_input, read_meta, read_raw

CLIP_DIM = 512
CLIP_DROPOUT = 0.15
# `crop_ratio`'s unit value for a free crop (`photos-core::retouch::RATIO_FREE`).
RATIO_FREE = 0.5 / len(params.RATIOS)

# Slots whose meaning mirrors with the photo: x' = 1 - x.
_MIRROR = ["lin_fx", "lin_tx", "rad_cx"]
# Geometry, tone, colour, detail and effects come before the masks' slots.
_FIRST_MASK = params.index("lin_on")
# Sources whose photos are originals: only the ones with a target (`build_pairs.py`) train;
# an unedited RAW as a synthetic final would teach that a flat render is the goal.
PAIR_KINDS = ("pairs", "raw+jpeg")


def scene(folder_name: str) -> str:
    """`retouch_prep`'s `<stem>-<16 hex digits of the path>` → the photo's stem."""
    return splits.stem(folder_name)


def shares_of(entry: dict) -> splits.Shares:
    """A registry entry's `split = { val, test }`, or the default."""
    sp = entry.get("split") or {}
    d = splits.Shares()
    return splits.Shares(val=int(sp.get("val", d.val)), test=int(sp.get("test", d.test)))


def _unit(name: str, x: float) -> float:
    p = params.PARAMS[params.index(name)]
    return min(1.0, max(0.0, (x - p["min"]) / (p["max"] - p["min"])))


def _phys(name: str, u: float) -> float:
    p = params.PARAMS[params.index(name)]
    return p["min"] + u * (p["max"] - p["min"])


def _flip_content(t: torch.Tensor, w: int) -> torch.Tensor:
    out = t.clone()
    out[..., :w] = t[..., :w].flip(-1)
    return out


def collate(items: list[dict]) -> dict:
    """`default_collate` for a batch that may mix prepared pairs and lazy synthetic items: a key
    one kind lacks is filled with zeros shaped like the other kind's."""
    keys = {k for it in items for k in it}
    like = {k: next(it[k] for it in items if k in it) for k in keys}
    return default_collate([it if it.keys() == keys else {**{k: torch.zeros_like(like[k]) for k in keys - it.keys()}, **it} for it in items])


class PreparedDataset(Dataset):
    """Prepared photos as training items. A synthetic item is a pair made from the photo now
    (`synth.make_pair`), or with `lazy` the photo, its size and the damage drawn for it, made
    into a pair later for the whole batch on the GPU (`synth.realise`); the draws are the same.
    `cache` keeps each decoded photo in RAM (float16) after its first read."""

    def __init__(self, root: Path | str, sources: list[str], split: str = "train", seed: int = 0, augment: bool = True, ship: bool = False, registry: dict | None = None, lazy: bool = False, cache: bool = False, identity_p: float = 0.0, terms: str = "open"):
        reg =sources_mod.merged_entries(Path(root)) if registry is None else registry
        licence.check(sources, ship=ship, registry=reg, terms=terms)
        self.root = Path(root)
        self.split = split
        self.seed = seed
        self.epoch = 0
        self.augment = augment and split == "train"
        self.lazy = lazy
        self.identity_p = identity_p
        self.cache: dict[int, tuple] | None = {} if cache else None
        self.samples: list[Path] = []
        self.kinds: dict[str, str] = {s: reg[s]["kind"] for s in sources}
        self.shares = {s: shares_of(reg[s]) for s in sources}
        self.sources: list[str] = []
        for s in sources:
            for d in sorted((self.root / s).glob("*/input.u16")):
                folder = d.parent
                if self.kinds[s] in PAIR_KINDS and not (folder / "target.json").exists():
                    continue
                if split == "all" or splits.split_of(scene(folder.name), self.shares[s]) == split:
                    self.samples.append(folder)
                    self.sources.append(s)

    def set_epoch(self, epoch: int) -> None:
        self.epoch = epoch

    def clip_share(self) -> float:
        """The share of photos with a CLIP embedding in `meta.json`."""
        if not self.samples:
            return 0.0
        return sum(1 for f in self.samples if read_meta(f).get("clip") is not None) / len(self.samples)

    def __len__(self) -> int:
        return len(self.samples)

    def _generator(self, i: int) -> torch.Generator:
        epoch = self.epoch if self.split == "train" else 0
        return torch.Generator().manual_seed(self.seed * 1_000_003 + epoch * 10_007 + i)

    def _read(self, i: int) -> tuple[torch.Tensor, torch.Tensor, int, int]:
        if self.cache is not None and i in self.cache:
            lin, mask, w, h = self.cache[i]
            return lin.float(), mask.float(), w, h
        lin, mask, w, h = decode_input(read_raw(self.samples[i]))
        if self.cache is not None:
            self.cache[i] = (lin.half(), mask.bool(), w, h)
        return lin, mask, w, h

    def __getitem__(self, i: int) -> dict:
        folder = self.samples[i]
        gen = self._generator(i)
        lin, mask, w, h = self._read(i)
        meta = read_meta(folder)
        target_json = folder / "target.json"
        n = len(params.PARAMS)
        v_target = params.neutral_vector()
        v_known = torch.zeros(n)
        if target_json.exists():
            item = self._pair(folder, lin, mask, w, h, json.loads(target_json.read_text(encoding="utf-8")), v_target, v_known)
        else:
            item = self._synthetic(lin, w, h, gen, v_target, v_known)
        clip = meta.get("clip")
        keep_clip = clip is not None and not (self.augment and float(torch.rand(1, generator=gen)) < CLIP_DROPOUT)
        item["clip"] = torch.tensor(clip, dtype=torch.float32) if keep_clip else torch.zeros(CLIP_DIM)
        item["clip_flag"] = torch.tensor([1.0 if keep_clip else 0.0])
        if self.augment and float(torch.rand(1, generator=gen)) < 0.5:
            item = self.flip(item)
        return item

    def _synthetic(self, lin, w, h, gen, v_target, v_known) -> dict:
        d = synth.Degradation.sample(gen)
        # The coin comes after the damage, and only when there is one, so 12-1's draws stand.
        fine = self.identity_p > 0 and float(torch.rand(1, generator=gen)) < self.identity_p
        if fine:
            # "Already fine": the photo as it is, and every global slider known at neutral.
            d = synth.Degradation.identity()
            v_known[:_FIRST_MASK] = 1.0
            for name in ("lin_on", "rad_on"):
                v_target[params.index(name)] = 0.0
                v_known[params.index(name)] = 1.0
        if self.lazy:
            size, deg = torch.tensor([w, h]), d.to_tensor()
            g = {k: x[0] for k, x in synth.geometry(size.unsqueeze(0), deg.unsqueeze(0)).items()}
            pair = {"straighten": float(g["straighten"]), "crop_on": float(g["crop_on"]), "crop_box": g["crop_box"].tolist(), "aspect": float(g["aspect"])}
        else:
            pair = synth.make_pair(lin[:, :h, :w], d)
        v_target[params.index("straighten")] = _unit("straighten", pair["straighten"])
        v_known[params.index("straighten")] = 1.0
        v_target[params.index("crop_on")] = pair["crop_on"]
        v_known[params.index("crop_on")] = 1.0
        if pair["crop_on"]:
            for k, x in zip("xywh", pair["crop_box"]):
                v_target[params.index(f"crop_{k}")] = _unit(f"crop_{k}", x)
                v_known[params.index(f"crop_{k}")] = 1.0
            # The box finds the photo again at its own shape, which is a free crop of the
            # looser frame; untrained, the ratio would pick a shape at random in the app.
            v_target[params.index("crop_ratio")] = RATIO_FREE
            v_known[params.index("crop_ratio")] = 1.0
        images = {"final": lin.float(), "size": size, "deg": deg} if self.lazy else {k: pair[k].float() for k in ["input", "aligned", "target", "mask"]}
        return {
            **images,
            "has_target": torch.tensor(1.0),
            "v_target": v_target,
            "v_known": v_known,
            "aspect": torch.tensor(pair["aspect"], dtype=torch.float32),
            "kind": 0,
        }

    def _pair(self, folder, lin, mask, w, h, target, v_target, v_known) -> dict:
        v = torch.tensor(target["v"], dtype=torch.float32)
        v_target[:] = v
        if "known" in target:  # an aligned pair: only the geometry is his, the look is a JPEG
            for name in target["known"]:
                v_known[params.index(name)] = 1.0
        else:
            v_known[:] = 1.0
        raw_target = folder / "target.u16"
        if raw_target.exists():
            tgt, _, _, _ = decode_input(np.fromfile(raw_target, dtype="<u2"))
            has = 1.0
        else:
            tgt, has = torch.zeros(3, EDGE, EDGE), 0.0
        # Pixels of the photo his edit does not cover (outside his crop): never scored. A
        # separate key, not a narrower `mask`, because `mask` also gives the photo's size.
        void = torch.zeros(1, EDGE, EDGE)
        target_mask = folder / "target_mask.u8"
        if target_mask.exists():
            valid = torch.from_numpy(np.fromfile(target_mask, dtype=np.uint8).reshape(1, EDGE, EDGE).astype(np.float32))
            void = mask.float() * (1.0 - valid)
        return {
            "input": encode_input(lin[:, :h, :w], w, h).float(),
            "aligned": lin.float(),
            "target": tgt.float(),
            "mask": mask.float(),
            "target_void": void,
            "has_target": torch.tensor(has),
            "v_target": v_target,
            "v_known": v_known,
            "aspect": torch.tensor(w / h, dtype=torch.float32),
            "kind": 1,
        }

    def flip(self, item: dict) -> dict:
        """The photo mirrored left to right, and every slot that has a side with it."""
        out = dict(item)
        if "final" in item:  # lazy: mirror the photo and the damage, render later
            out["final"] = _flip_content(item["final"], int(item["size"][0]))
            deg = item["deg"].clone()
            deg[synth.TILT] = -deg[synth.TILT]
            pad = deg[synth.PAD].clone()
            deg[synth.PAD] = pad[[2, 1, 0, 3]]  # left and right swap
            out["deg"] = deg
            out["v_target"] = self._flip_target(item)
            return out
        w_in = int(item["input"][3, 0].sum().item())
        w_t = int(item["mask"][0, 0].sum().item())
        out["input"] = _flip_content(item["input"], w_in)
        out["aligned"] = _flip_content(item["aligned"], w_t)
        out["target"] = _flip_content(item["target"], w_t)
        if "target_void" in item:
            out["target_void"] = _flip_content(item["target_void"], w_t)
        out["v_target"] = self._flip_target(item)
        return out

    @staticmethod
    def _flip_target(item: dict) -> torch.Tensor:
        v = item["v_target"].clone()
        s = params.index("straighten")
        v[s] = 1.0 - v[s]
        for name in _MIRROR:
            j = params.index(name)
            v[j] = 1.0 - v[j]
        cx, cw = params.index("crop_x"), params.index("crop_w")
        if item["v_known"][cx] > 0:
            x, w = _phys("crop_x", float(v[cx])), _phys("crop_w", float(v[cw]))
            v[cx] = _unit("crop_x", 1.0 - x - w)
        return v
