"""Julian's pairs by photo, and k folds over them that never split a photo: a RAW's edits
(`IMG_1`, `IMG_1-Edit`) share one stem and so one fold.

Only the pairs outside the source's test split take part; the test split stays for the final
comparison with Ray."""

import hashlib
from pathlib import Path

from ..data import sources as sources_mod, splits
from ..retouch.dataset import scene, shares_of

PAIRS = "julian-pairs"


def pairs(root: Path | str, source: str = PAIRS) -> dict[str, list[Path]]:
    """Stem → the source's pair folders (those with `target.json`) outside its test split."""
    root = Path(root)
    shares = shares_of(sources_mod.merged_entries(root)[source])
    out: dict[str, list[Path]] = {}
    for f in sorted((root / source).glob("*/target.json")):
        s = scene(f.parent.name)
        if splits.split_of(s, shares) != "test":
            out.setdefault(s, []).append(f.parent)
    return out


def of(stems: list[str], k: int = 5) -> list[list[str]]:
    """`k` folds of stems, sizes within one of each other, the same for the same stems in any
    order (dealt round-robin in the order of a hash of each stem)."""
    if k < 2 or k > len(stems):
        raise ValueError(f"{k} folds over {len(stems)} photos: use 2 ≤ k ≤ the number of photos")
    order = sorted(set(stems), key=lambda s: (hashlib.sha1(s.encode("utf-8")).hexdigest(), s))
    return [sorted(order[i::k]) for i in range(k)]


def folders(by_stem: dict[str, list[Path]], stems: list[str]) -> list[Path]:
    """Every folder of the given stems."""
    return [f for s in stems for f in by_stem[s]]
