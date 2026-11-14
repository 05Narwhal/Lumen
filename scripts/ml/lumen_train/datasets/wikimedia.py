"""Finished photos from Wikimedia Commons for Retouch's synthetic pairs.

    python -m lumen_train.datasets.wikimedia --out C:/lumen-train/originals --contact you@example.com [--featured 100000] [--quality 18000]

Reads two reviewed categories: *Featured pictures* (all of them, by default) and *Quality
images* (the most recently promoted, up to `--quality` kept). A file is kept only when:
- its licence allows training a model that ships (registry sources `wikimedia-cc0-pd` and
  `wikimedia-cc-by`): CC0 or public domain, or CC BY without SA/NC/ND, also when that is one
  of several licences the author offers. CC BY-SA, GFDL and everything else are refused;
- it is a photo: a JPEG with a camera model in its metadata, at least `MIN_EDGE` px on its
  long side, no wider than `MAX_RATIO` (no panoramas), and no category naming artwork,
  scans, maps or micrographs;
- it is in colour (black-and-white photos teach nothing about colour), checked after download.

Files land in `<out>/<source>/<pageid>.jpg` as Commons' 1280 px thumbnail (960 px wide for
portraits), with `<out>/<source>/credits.csv` (author, licence, file page) for the attribution
CC BY asks for. Every file looked at is recorded in `<out>/manifest.jsonl` and each
category's place in `<out>/state.json`, so a stopped run carries on where it was. Requests
are serial, about one a second, with the User-Agent and back-off Wikimedia's robot policy asks
for. Downloads never go anywhere but `--out`."""

import argparse
import csv
import html
import io
import json
import re
import sys
import time
import urllib.error
import urllib.parse
import urllib.request
from dataclasses import dataclass
from pathlib import Path
from typing import Callable

API = "https://commons.wikimedia.org/w/api.php"
FEATURED = "Category:Featured_pictures_on_Wikimedia_Commons"
QUALITY = "Category:Quality_images"
MIN_EDGE = 1600
MAX_RATIO = 2.5
THUMB = 1280
THUMB_PORTRAIT = 960
# Below this mean saturation (HSV, 0–1) a photo counts as black and white.
MIN_SATURATION = 0.06

CC0_PD = "wikimedia-cc0-pd"
CC_BY = "wikimedia-cc-by"
# Licence tracking categories templates add, for files offered under several licences.
_CC0_CATS = {"CC-Zero", "CC0"}
_CC_BY_CATS = {f"CC-BY-{v}" for v in ("1.0", "2.0", "2.5", "3.0", "4.0")}
_ARTWORK = re.compile(r"\b(paintings?|drawings?|engravings?|etchings?|lithographs?|woodcuts?|manuscripts?|maps?|illustrations?|scans?|scanned|\w*micrographs?|microscop\w*|artworks?|light painting|diagrams?|posters?|frescos?|frescoes|mosaics?|tapestr(y|ies)|stained glass|composites?|hdr)\b", re.I)


def classify(license_value: str, categories: list[str]) -> str | None:
    """The registry source a file's licence allows, or None."""
    lic = license_value.lower().strip()
    if lic.startswith("cc0") or lic.startswith("pd") or lic == "public domain":
        return CC0_PD
    if re.fullmatch(r"cc-by-\d(\.\d)?", lic):
        return CC_BY
    cats = set(categories)
    if cats & _CC0_CATS:
        return CC0_PD
    if cats & _CC_BY_CATS:
        return CC_BY
    return None


def photo_problem(info: dict, categories: list[str], title: str = "") -> str | None:
    """Why a file is not a photo to learn from, or None when it is (artwork, scans and
    micrographs show in the categories or the title)."""
    if info.get("mime") != "image/jpeg":
        return "not a JPEG"
    w, h = info.get("width", 0), info.get("height", 0)
    if max(w, h) < MIN_EDGE:
        return "too small"
    if max(w, h) > MAX_RATIO * max(1, min(w, h)):
        return "panorama"
    if not any(m.get("name") == "Model" and m.get("value") for m in info.get("commonmetadata", [])):
        return "no camera"
    for c in categories:
        if _ARTWORK.search(c):
            return f"category {c}"
    if _ARTWORK.search(title):
        return "title"
    return None


def thumb_url(info: dict) -> str | None:
    url = info.get("thumburl")
    if not url:
        return None
    if info.get("height", 0) > info.get("width", 0):
        url = url.replace(f"/{THUMB}px-", f"/{THUMB_PORTRAIT}px-")
    return url


def is_colour(jpeg: bytes) -> bool:
    from PIL import Image, ImageStat

    img = Image.open(io.BytesIO(jpeg)).convert("RGB")
    img.thumbnail((128, 128))
    return ImageStat.Stat(img.convert("HSV").getchannel(1)).mean[0] / 255.0 >= MIN_SATURATION


def _plain(text: str) -> str:
    return " ".join(html.unescape(re.sub(r"<[^>]+>", " ", text or "")).split())


class Fetcher:
    """Serial HTTP with a minimum gap between requests and back-off on 429, 5xx and network
    errors (honouring Retry-After)."""

    def __init__(self, contact: str, gap: float = 1.0, tries: int = 8, sleep: Callable[[float], None] = time.sleep, opener: Callable | None = None):
        self.headers = {"User-Agent": f"LumenRetouchDataset/1.0 (JL Studios Lumen photo app; {contact}) python-urllib", "Accept-Encoding": "identity"}
        self.gap, self.tries, self.sleep = gap, tries, sleep
        self.opener = opener or (lambda req: urllib.request.urlopen(req, timeout=60))
        self.last = 0.0

    def get(self, url: str) -> bytes:
        wait = 5.0
        for attempt in range(self.tries):
            pause = self.gap - (time.monotonic() - self.last)
            if pause > 0:
                self.sleep(pause)
            self.last = time.monotonic()
            try:
                with self.opener(urllib.request.Request(url, headers=self.headers)) as r:
                    return r.read()
            except urllib.error.HTTPError as e:
                if e.code not in (429, 500, 502, 503, 504) or attempt == self.tries - 1:
                    raise
                retry = e.headers.get("Retry-After") if e.headers else None
                self.sleep(float(retry) if retry and retry.isdigit() else wait)
            except (urllib.error.URLError, TimeoutError, ConnectionError):
                if attempt == self.tries - 1:
                    raise
                self.sleep(wait)
            wait = min(wait * 2, 300.0)
        raise RuntimeError("unreachable")

    def api(self, params: dict) -> dict:
        query = {"action": "query", "format": "json", "formatversion": "2", "maxlag": "5", **params}
        while True:
            data = json.loads(self.get(API + "?" + urllib.parse.urlencode(query)))
            if data.get("error", {}).get("code") == "maxlag":  # the servers are busy: wait, ask again
                self.sleep(10.0)
                continue
            if "error" in data:
                raise RuntimeError(f"Commons API: {data['error']}")
            return data


@dataclass
class Counts:
    kept: int = 0
    looked: int = 0


class Downloader:
    def __init__(self, out: Path, fetch: Fetcher, log=print):
        self.out = Path(out)
        self.fetch = fetch
        self.log = log
        self.out.mkdir(parents=True, exist_ok=True)
        self.manifest = self.out / "manifest.jsonl"
        self.state_path = self.out / "state.json"
        self.seen: dict[int, dict] = {}
        if self.manifest.exists():
            for line in self.manifest.read_text(encoding="utf-8").splitlines():
                if line.strip():
                    row = json.loads(line)
                    self.seen[row["pageid"]] = row
        self.state = json.loads(self.state_path.read_text(encoding="utf-8")) if self.state_path.exists() else {}

    def kept(self, category: str) -> int:
        return sum(1 for r in self.seen.values() if r.get("category") == category and r.get("kept"))

    def _record(self, row: dict) -> None:
        self.seen[row["pageid"]] = row
        with open(self.manifest, "a", encoding="utf-8") as f:
            f.write(json.dumps(row, ensure_ascii=False) + "\n")

    def _credit(self, source: str, row: dict) -> None:
        path = self.out / source / "credits.csv"
        new = not path.exists()
        with open(path, "a", newline="", encoding="utf-8") as f:
            w = csv.writer(f)
            if new:
                w.writerow(["file", "title", "author", "licence", "licence_url", "page"])
            w.writerow([row["file"], row["title"], row["author"], row["licence"], row["licence_url"], row["page"]])

    def _save_state(self) -> None:
        tmp = self.state_path.with_suffix(".tmp")
        tmp.write_text(json.dumps(self.state), encoding="utf-8")
        tmp.replace(self.state_path)

    def _consider(self, page: dict, category: str) -> bool:
        pid = page["pageid"]
        if pid in self.seen or not page.get("imageinfo"):
            # Seen before, or its details come in a later response (`iicontinue`).
            return False
        info = page["imageinfo"][0]
        meta = info.get("extmetadata", {})
        cats = [c for c in meta.get("Categories", {}).get("value", "").split("|") if c]
        row = {"pageid": pid, "title": page.get("title", ""), "category": category, "kept": False}
        source = classify(meta.get("License", {}).get("value", ""), cats)
        why = "licence" if source is None else photo_problem(info, cats, page.get("title", ""))
        url = thumb_url(info) if why is None else None
        if why is None and url is None:
            why = "no thumbnail"
        if why is None:
            try:
                jpeg = self.fetch.get(url)
            except urllib.error.HTTPError as e:
                why = f"download failed ({e.code})"
            else:
                if not is_colour(jpeg):
                    why = "black and white"
                else:
                    folder = self.out / source
                    folder.mkdir(parents=True, exist_ok=True)
                    name = f"{pid}.jpg"
                    tmp = folder / (name + ".part")
                    tmp.write_bytes(jpeg)
                    tmp.replace(folder / name)
                    row.update(kept=True, source=source, file=f"{source}/{name}")
                    self._credit(source, {
                        "file": name,
                        "title": page.get("title", ""),
                        "author": _plain(meta.get("Artist", {}).get("value", "")) or _plain(meta.get("Credit", {}).get("value", "")),
                        "licence": meta.get("LicenseShortName", {}).get("value", ""),
                        "licence_url": meta.get("LicenseUrl", {}).get("value", ""),
                        "page": info.get("descriptionurl", ""),
                    })
        if why is not None:
            row["skipped"] = why
        self._record(row)
        return row["kept"]

    def run(self, category: str, want: int) -> Counts:
        """Keeps up to `want` files of `category` (counting earlier runs), newest first."""
        counts = Counts(kept=self.kept(category))
        st = self.state.setdefault(category, {})
        while counts.kept < want and not st.get("done"):
            params = {
                "generator": "categorymembers", "gcmtitle": category, "gcmtype": "file", "gcmlimit": "50",
                "gcmsort": "timestamp", "gcmdir": "desc",
                "prop": "imageinfo", "iiprop": "url|size|mime|extmetadata|commonmetadata", "iiurlwidth": str(THUMB),
                "iiextmetadatafilter": "License|LicenseShortName|LicenseUrl|Artist|Credit|Categories",
                **st.get("continue", {}),
            }
            data = self.fetch.api(params)
            finished = True
            for page in sorted(data.get("query", {}).get("pages", []), key=lambda p: p.get("pageid", 0)):
                if counts.kept >= want:
                    # Stopped inside this batch: the next run asks for it again (the files
                    # already looked at are in the manifest), never skips the rest.
                    finished = False
                    break
                counts.looked += 1
                if self._consider(page, category):
                    counts.kept += 1
            if not finished:
                break
            if "continue" in data:
                st["continue"] = data["continue"]
            else:
                st["done"] = True
            self._save_state()
            self.log(f"{category}: {counts.kept} kept, {counts.looked} looked at this run")
        return counts


def download(out: Path, contact: str, featured: int, quality: int, log=print) -> None:
    """Both categories into `out/<source>/`, resumable (`state.json`, `manifest.jsonl`)."""
    d = Downloader(out, Fetcher(contact), log=lambda m: log(time.strftime("%H:%M:%S") + " " + m))
    d.run(FEATURED, featured)
    d.run(QUALITY, quality)
    for source in (CC0_PD, CC_BY):
        n = len(list((out / source).glob("*.jpg"))) if (out / source).exists() else 0
        log(f"{source}: {n} photos in {out / source}")


def main() -> None:
    sys.stdout.reconfigure(encoding="utf-8", errors="replace")
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--out", type=Path, required=True, help="e.g. C:/lumen-train/originals (never the Lumen library)")
    ap.add_argument("--contact", required=True, help="an email or URL for the User-Agent (Wikimedia's robot policy)")
    ap.add_argument("--featured", type=int, default=100_000, help="featured pictures to keep (default: all)")
    ap.add_argument("--quality", type=int, default=18_000, help="quality images to keep")
    a = ap.parse_args()
    download(a.out, a.contact, a.featured, a.quality, log=lambda m: print(m, flush=True))


if __name__ == "__main__":
    main()
