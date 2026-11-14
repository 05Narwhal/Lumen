"""`python -m lumen_train.data {add,prep,status,download}` - see docs/ML-TRAINING.md, *Adding data*."""

import argparse
import os
import sys
from pathlib import Path

from . import prep, sources, splits, status


def originals_of(root: Path, src: sources.Source) -> Path | None:
    """A local source's folder, or where `download` put a public source's photos."""
    if src.originals is not None:
        return src.originals
    d = Path(root) / "originals" / src.name
    return d if d.is_dir() else None


def _root(a) -> Path:
    r = a.root or os.environ.get("LUMEN_TRAIN_ROOT")
    if not r:
        raise SystemExit("give --root (the training folder, e.g. C:/lumen-train) or set LUMEN_TRAIN_ROOT")
    return Path(r)


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(prog="lumen_train.data", description=__doc__)
    ap.add_argument("--root", help="the training folder (default: $LUMEN_TRAIN_ROOT)")
    sub = ap.add_subparsers(dest="cmd", required=True)

    p = sub.add_parser("add", help="register a folder of photos as a local source")
    p.add_argument("name")
    p.add_argument("folder", type=Path)
    p.add_argument("--kind", required=True, choices=sources.KINDS)
    p.add_argument("--licence", required=True)
    p.add_argument("--commercial", action="store_true")
    p.add_argument("--attribution", action="store_true")
    p.add_argument("--val", type=int, default=splits.Shares().val)
    p.add_argument("--test", type=int, default=splits.Shares().test)

    p = sub.add_parser("prep", help="prepare new or changed photos only")
    p.add_argument("names", nargs="*")
    p.add_argument("--clip", action="store_true", help="accepted; retouch_prep has no CLIP stage yet")
    p.add_argument("--jobs", type=int, default=8)

    sub.add_parser("status", help="items per source x split, CLIP coverage, licence")

    p = sub.add_parser("download", help="fetch public photos into <root>/originals/")
    p.add_argument("what", choices=["wikimedia"])
    p.add_argument("--limit", type=int, required=True, help="photos to keep per category")
    p.add_argument("--contact", default=os.environ.get("LUMEN_CONTACT"), help="an email or URL for the User-Agent (or $LUMEN_CONTACT)")

    a = ap.parse_args(argv)
    root = _root(a)
    try:
        if a.cmd == "add":
            s = sources.add(root, a.name, a.folder, a.kind, a.licence, a.commercial, a.attribution, splits.Shares(a.val, a.test))
            print(f"added {s.name} ({s.kind}, val {s.split.val} %, test {s.split.test} %) -> {sources.local_file(root)}")
        elif a.cmd == "prep":
            known = sources.load(root)
            names = a.names or [n for n, s in known.items() if originals_of(root, s) is not None]
            for n in names:
                if n not in known:
                    raise sources.SourceError(f"unknown source {n}")
                src = known[n]
                folder = originals_of(root, src)
                if folder is None:
                    raise sources.SourceError(f"{n} has no originals folder (data add it, or data download first)")
                r = prep.run(root, n, folder, clip=a.clip, jobs=a.jobs)
                print(f"{n}: {r.prepared} prepared, {r.skipped} unchanged, {len(r.failed)} failed")
                for rel, why in r.failed:
                    print(f"  failed {rel}: {why}")
        elif a.cmd == "status":
            print(status.render(status.table(root)))
        elif a.cmd == "download":
            if not a.contact:
                raise sources.SourceError("give --contact (an email or URL for Wikimedia's User-Agent) or set LUMEN_CONTACT")
            from ..datasets import wikimedia

            wikimedia.download(root / "originals", a.contact, a.limit, a.limit)
    except sources.SourceError as e:
        print(f"error: {e}", file=sys.stderr)
        return 2
    return 0


if __name__ == "__main__":
    sys.exit(main())
