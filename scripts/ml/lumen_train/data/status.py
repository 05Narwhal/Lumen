"""`data status`: items per source x split, CLIP coverage, licence, failures and orphans."""

import json
from dataclasses import dataclass
from pathlib import Path

from . import prep, sources, splits


@dataclass
class Row:
    source: str
    licence: str
    commercial: bool
    train: int
    val: int
    test: int
    clip_share: float
    failed: int
    orphaned: int


def table(root: Path) -> list[Row]:
    rows = []
    for name, src in sorted(sources.load(Path(root)).items()):
        d = Path(root) / name
        if not d.is_dir():
            continue
        count = {"train": 0, "val": 0, "test": 0}
        folders = [p.parent for p in d.glob("*/input.u16")]
        clips = 0
        for f in folders:
            count[splits.split_of(splits.stem(f.name), src.split)] += 1
            try:
                clips += json.loads((f / "meta.json").read_text(encoding="utf-8")).get("clip") is not None
            except (OSError, ValueError):
                pass
        index = prep.read_index(d)
        failed = sum(1 for r in index.values() if not r["ok"])
        orphaned = 0
        if src.originals is not None:
            orphaned = sum(1 for rel, r in index.items() if r["ok"] and not (src.originals / rel).exists())
        rows.append(Row(name, src.licence, src.commercial, count["train"], count["val"], count["test"], clips / len(folders) if folders else 0.0, failed, orphaned))
    return rows


def render(rows: list[Row]) -> str:
    lines = [f"{'source':<28}{'train':>7}{'val':>6}{'test':>6}{'clip':>6}{'failed':>7}{'orphan':>7}  licence"]
    for r in rows:
        lic = ("" if r.commercial else "NON-COMMERCIAL ") + r.licence
        lines.append(f"{r.source:<28}{r.train:>7}{r.val:>6}{r.test:>6}{r.clip_share:>6.0%}{r.failed:>7}{r.orphaned:>7}  {lic}")
    return "\n".join(lines)
