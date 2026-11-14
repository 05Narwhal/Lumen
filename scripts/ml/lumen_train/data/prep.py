"""Incremental prep: `retouch_prep` runs only on originals whose content hash is new, and each
result is a line in `<root>/<source>/index.jsonl` (the newest line per file wins). Originals are
only read; the destination is always under `<root>`."""

import hashlib
import json
import os
import shutil
import subprocess
import time
from concurrent.futures import ThreadPoolExecutor
from dataclasses import dataclass, field
from pathlib import Path
from typing import Callable

EXTENSIONS = {".jpg", ".jpeg", ".png", ".tif", ".tiff", ".webp", ".heic", ".jxl", ".dng", ".cr2", ".cr3", ".nef", ".arw", ".raf", ".orf", ".rw2"}

# (original, destination folder holding the prepared `<stem>-<hash16>/`) -> the prepared folder.
Runner = Callable[[Path, Path], Path]
# (a RAW, a folder to write into) -> the full-size JPEG the camera stored inside it.
Embedded = Callable[[Path, Path], Path]
# (a RAW, a folder to write into) -> the RAW as a DNG, or None when no converter is installed.
ToDng = Callable[[Path, Path], Path | None]

EMBEDDED = "embedded JPEG"
ADOBE = "Adobe DNG Converter"
_ADOBE_PATHS = (
    "C:/Program Files/Adobe/Adobe DNG Converter/Adobe DNG Converter.exe",
    "/Applications/Adobe DNG Converter.app/Contents/MacOS/Adobe DNG Converter",
)


def no_open_decoder(original: Path, error: Exception) -> bool:
    """Nikon's High Efficiency (HE / HE*) NEFs, the Z f's and the Z 8's / Z 9's option: their
    codec (TicoRAW) is licensed and no open decoder reads it, so rawler refuses them."""
    return original.suffix.lower() == ".nef" and "highefficen" in str(error).lower().replace(" ", "")


def _exiftool() -> str:
    from ..retouch.paths import SRC_TAURI

    found = shutil.which("exiftool") or next((str(p) for p in sorted((SRC_TAURI / "binaries").glob("exiftool*")) if p.is_file()), None)
    if found is None:
        raise RuntimeError("ExifTool not found (PATH or src-tauri/binaries); it reads the JPEG inside a RAW Lumen cannot decode")
    return found


def _private(raw: Path, out_dir: Path) -> Path:
    """`<out_dir>/<hash of the RAW's path>/`: the stem (and so the split) stays the RAW's, and two
    RAWs of one name never share a file."""
    d = out_dir / hashlib.sha1(str(raw).encode("utf-8")).hexdigest()[:12]
    d.mkdir(parents=True, exist_ok=True)
    return d


def adobe_dng(raw: Path, out_dir: Path) -> Path | None:
    """`raw` as a lossless DNG by Adobe DNG Converter (free; it licenses Nikon's HE codec), which
    rawler decodes: the photo's own sensor data. None when the converter is not installed
    (`LUMEN_DNG_CONVERTER` names it if it is somewhere else)."""
    tool = os.environ.get("LUMEN_DNG_CONVERTER") or next((p for p in _ADOBE_PATHS if Path(p).is_file()), None)
    if tool is None:
        return None
    d = _private(raw, out_dir)
    out = d / (raw.stem + ".dng")
    out.unlink(missing_ok=True)
    # -c lossless compression, -p0 a small preview, -d the folder; it writes <stem>.dng there.
    subprocess.run([tool, "-c", "-p0", "-d", str(d), str(raw)], capture_output=True, timeout=600)
    if not out.is_file():
        raise RuntimeError(f"{ADOBE} made no DNG from {raw.name}")
    return out


def extract_embedded_jpeg(raw: Path, out_dir: Path) -> Path:
    """The camera's full-size JPEG from inside `raw`, with the RAW's orientation and EXIF: its
    rendering, not its sensor data, so only the fallback when no DNG converter is installed."""
    out = _private(raw, out_dir) / (raw.stem + ".jpg")
    tool = _exiftool()
    for tag in ("-JpgFromRaw", "-PreviewImage"):
        data = subprocess.run([tool, "-b", tag, str(raw)], capture_output=True).stdout
        if len(data) > 100_000 and data[:2] == b"\xff\xd8":
            break
    else:
        raise RuntimeError(f"{raw.name}: no full-size JPEG inside")
    out.write_bytes(data)
    subprocess.run([tool, "-q", "-overwrite_original", "-TagsFromFile", str(raw), "-exif:all", "--PreviewImage", "--JpgFromRaw", "--ThumbnailImage", str(out)], capture_output=True, check=True)
    return out


@dataclass
class PrepReport:
    prepared: int = 0
    skipped: int = 0
    failed: list[tuple[str, str]] = field(default_factory=list)


def read_index(source_dir: Path) -> dict[str, dict]:
    """rel -> its newest line."""
    out: dict[str, dict] = {}
    f = source_dir / "index.jsonl"
    if f.exists():
        for line in f.read_text(encoding="utf-8").splitlines():
            if line.strip():
                row = json.loads(line)
                out[row["rel"]] = row
    return out


def sha256(path: Path) -> str:
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def failure_reason(output: str, code: int) -> str:
    """Why `retouch_prep` made nothing: its `skipped <path>: <reason>` line (it ends with a
    progress line), else its last line, else its exit code."""
    lines = [x.strip() for x in output.splitlines() if x.strip()]
    for line in lines:
        if line.startswith("skipped ") and ": " in line:
            return line.split(": ", 1)[1]
    return lines[-1] if lines else f"retouch_prep exited {code}"


def run_retouch_prep(original: Path, dest: Path) -> Path:
    """The app's own preparation (`retouch_prep`), one file at a time into a scratch folder
    that is then moved into `dest`, so concurrent files never confuse which folder is whose."""
    from ..retouch.paths import example

    scratch = dest / f".tmp-{hashlib.sha1(str(original).encode('utf-8')).hexdigest()[:12]}"
    shutil.rmtree(scratch, ignore_errors=True)
    scratch.mkdir(parents=True)
    try:
        r = subprocess.run([str(example("retouch_prep")), str(original), str(scratch), "--jobs", "1"], capture_output=True, text=True)
        made = [p for p in scratch.iterdir() if (p / "input.u16").exists()]
        if r.returncode != 0 or len(made) != 1:
            raise RuntimeError(failure_reason(f"{r.stdout}\n{r.stderr}", r.returncode))
        final = dest / made[0].name
        shutil.rmtree(final, ignore_errors=True)  # our own prepared copy of a changed photo
        made[0].rename(final)
        return final
    finally:
        shutil.rmtree(scratch, ignore_errors=True)


def originals_in(folder: Path) -> list[Path]:
    return sorted(p for p in folder.rglob("*") if p.is_file() and p.suffix.lower() in EXTENSIONS)


def run(root: Path, source: str, originals: Path, clip: bool = False, jobs: int = 4, runner: Runner | None = None, embedded: Embedded | None = None, to_dng: ToDng | None = None) -> PrepReport:
    # `clip` is accepted for the CLI; retouch_prep has no CLIP stage yet (see the plan's As built).
    runner = runner or run_retouch_prep
    embedded = embedded or extract_embedded_jpeg
    to_dng = to_dng or adobe_dng

    def stand_in(p: Path) -> tuple[Path, str]:
        """A RAW Lumen cannot decode, prepared from its DNG, else its embedded JPEG, both made
        under the source's own folder, never next to the original."""
        try:
            dng = to_dng(p, dest / "_converted")
            if dng is not None:
                return Path(runner(dng, dest)), ADOBE
        except Exception:
            pass  # a DNG that fails falls back like a missing converter
        return Path(runner(embedded(p, dest / "_embedded"), dest)), EMBEDDED
    dest = Path(root) / source
    dest.mkdir(parents=True, exist_ok=True)
    index = read_index(dest)
    report = PrepReport()
    todo: list[tuple[str, Path, str]] = []
    for p in originals_in(Path(originals)):
        rel = p.relative_to(originals).as_posix()
        digest = sha256(p)
        row = index.get(rel)
        if row and row["ok"] and row["sha256"] == digest:
            report.skipped += 1
        else:
            todo.append((rel, p, digest))

    def one(item: tuple[str, Path, str]) -> dict:
        rel, p, digest = item
        try:
            via = None
            try:
                folder = Path(runner(p, dest))
            except Exception as e:
                if not no_open_decoder(p, e):
                    raise
                folder, via = stand_in(p)
                (folder / "stand_in.json").write_text(json.dumps({"raw": str(p), "via": via, "why": str(e)}, ensure_ascii=False), encoding="utf-8")
            return {"rel": rel, "sha256": digest, "folder": folder.name, "ok": True, "error": None, "via": via, "at": int(time.time())}
        except Exception as e:  # one bad file never stops the run; its reason goes in the index
            return {"rel": rel, "sha256": digest, "folder": None, "ok": False, "error": str(e), "at": int(time.time())}

    with ThreadPoolExecutor(max_workers=max(1, jobs)) as pool, open(dest / "index.jsonl", "a", encoding="utf-8") as out:
        for row in pool.map(one, todo):
            out.write(json.dumps(row, ensure_ascii=False) + "\n")
            out.flush()
            if row["ok"]:
                report.prepared += 1
            else:
                report.failed.append((row["rel"], row["error"]))
    return report
