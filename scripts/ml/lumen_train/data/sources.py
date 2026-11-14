"""Sources: the repo's public `datasets/registry.toml` plus the local `<root>/sources.toml`
that `data add` writes. A name may live in only one of them."""

import tomllib
from dataclasses import dataclass
from pathlib import Path

import tomli_w

from ..common import licence
from .splits import Shares

KINDS = ("finals", "pairs", "raw", "raw+jpeg")


class SourceError(RuntimeError):
    pass


@dataclass(frozen=True)
class Source:
    name: str
    url: str
    licence: str
    commercial: bool
    kind: str
    attribution: bool
    split: Shares
    local: bool
    originals: Path | None


def local_file(root: Path) -> Path:
    return Path(root) / "sources.toml"


def _read(path: Path) -> dict:
    if not path.exists():
        return {}
    with open(path, "rb") as f:
        return tomllib.load(f).get("sources", {})


def _make(name: str, e: dict, local: bool) -> Source:
    sp = e.get("split") or {}
    d = Shares()
    return Source(
        name=name,
        url=e.get("url", ""),
        licence=e.get("licence", ""),
        commercial=bool(e.get("commercial", False)),
        kind=e["kind"],
        attribution=bool(e.get("attribution", False)),
        split=Shares(val=int(sp.get("val", d.val)), test=int(sp.get("test", d.test))),
        local=local,
        originals=Path(e["originals"]) if e.get("originals") else None,
    )


def merged_entries(root: Path) -> dict:
    """Both registries as raw entries (what `licence.check` and the dataset read)."""
    public, mine = licence.load_registry(), _read(local_file(root))
    both = sorted(set(public) & set(mine))
    if both:
        raise SourceError(f"{', '.join(both)} is in both {licence.REGISTRY} and {local_file(root)}")
    return {**public, **mine}


def load(root: Path) -> dict[str, Source]:
    mine = _read(local_file(root))
    return {n: _make(n, e, n in mine) for n, e in merged_entries(root).items()}


def _forbidden(folder: Path) -> str | None:
    full = folder.resolve()
    s = str(full).replace("\\", "/").lower()
    inside = any((p / "library.db").exists() for p in full.parents)  # e.g. a library's originals/
    if "pictures/lumen library" in s or inside or (folder.exists() and any(folder.rglob("library.db"))):
        return "that folder is (or holds) a Lumen library; never train from it"
    return None


def add(root: Path, name: str, folder: Path, kind: str, licence_text: str, commercial: bool, attribution: bool = False, split: Shares | None = None) -> Source:
    root, folder = Path(root), Path(folder).resolve()  # read later from any working folder
    if kind not in KINDS:
        raise SourceError(f"kind must be one of {', '.join(KINDS)}")
    if (why := _forbidden(folder)) is not None:
        raise SourceError(why)
    if not folder.is_dir():
        raise SourceError(f"not a folder: {folder}")
    if name in licence.load_registry():
        raise SourceError(f"{name} is already in {licence.REGISTRY}")
    path = local_file(root)
    mine = _read(path)
    if name in mine:
        raise SourceError(f"{name} is already in {path}")
    sp = split or Shares()
    mine[name] = {
        "url": "",
        "licence": licence_text,
        "commercial": commercial,
        "kind": kind,
        "attribution": attribution,
        "originals": str(folder),
        "split": {"val": sp.val, "test": sp.test},
    }
    root.mkdir(parents=True, exist_ok=True)
    path.write_text(tomli_w.dumps({"sources": mine}), encoding="utf-8")
    return load(root)[name]
