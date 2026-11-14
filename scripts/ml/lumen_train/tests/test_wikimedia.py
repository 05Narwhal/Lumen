"""The Commons downloader, against a fake Wikimedia: licences, photo filters, credits, resuming,
back-off. No network."""

import csv
import io
import json
import urllib.error
import urllib.parse
from pathlib import Path

import pytest
from PIL import Image

from lumen_train.datasets import wikimedia as wm


def jpeg(colour: bool) -> bytes:
    img = Image.new("RGB", (64, 48))
    for x in range(64):
        for y in range(48):
            img.putpixel((x, y), (200, 80 + x, 40) if colour else (x * 3, x * 3, x * 3))
    buf = io.BytesIO()
    img.save(buf, "JPEG")
    return buf.getvalue()


def page(pid: int, licence: str = "cc0", cats: str = "Taken with a camera", mime: str = "image/jpeg", w: int = 4000, h: int = 3000, camera: bool = True, colour: bool = True) -> dict:
    return {
        "pageid": pid,
        "title": f"File:P{pid}.jpg",
        "imageinfo": [
            {
                "mime": mime,
                "width": w,
                "height": h,
                "thumburl": f"https://upload.example/thumb/{pid}.jpg/1280px-{'c' if colour else 'g'}{pid}.jpg",
                "descriptionurl": f"https://commons.example/File:P{pid}.jpg",
                "commonmetadata": [{"name": "Model", "value": "X-T5"}] if camera else [],
                "extmetadata": {
                    "License": {"value": licence},
                    "LicenseShortName": {"value": licence.upper()},
                    "LicenseUrl": {"value": "https://creativecommons.org/x"},
                    "Artist": {"value": '<a href="//commons">Ana <b>B</b></a>'},
                    "Categories": {"value": cats},
                },
            }
        ],
    }


class FakeWiki:
    """Category pages served in batches; thumbnails by their URL (`c…` colour, `g…` grey)."""

    def __init__(self, batches: dict[str, list[list[dict]]]):
        self.batches = batches
        self.api_calls = 0
        self.downloads: list[str] = []
        self.fail_next = 0

    def get(self, url: str) -> bytes:
        if self.fail_next:
            self.fail_next -= 1
            raise urllib.error.HTTPError(url, 429, "slow down", {"Retry-After": "2"}, None)
        if url.startswith(wm.API):
            self.api_calls += 1
            q = dict(urllib.parse.parse_qsl(urllib.parse.urlparse(url).query))
            batches = self.batches[q["gcmtitle"]]
            i = int(q.get("gcmcontinue", "0"))
            out = {"query": {"pages": batches[i]}}
            if i + 1 < len(batches):
                out["continue"] = {"gcmcontinue": str(i + 1), "continue": "gcmcontinue||"}
            return json.dumps(out).encode()
        self.downloads.append(url)
        return jpeg(colour="/1280px-c" in url or "/960px-c" in url)


class Opener:
    def __init__(self, wiki: FakeWiki):
        self.wiki = wiki

    def __call__(self, req):
        data = self.wiki.get(req.full_url)
        assert "LumenRetouchDataset" in req.headers["User-agent"] and "me@example.com" in req.headers["User-agent"]
        return io.BytesIO(data)


def downloader(tmp_path: Path, wiki: FakeWiki, sleeps: list | None = None) -> wm.Downloader:
    fetch = wm.Fetcher("me@example.com", gap=0.0, sleep=(sleeps.append if sleeps is not None else lambda s: None), opener=Opener(wiki))
    return wm.Downloader(tmp_path / "originals", fetch, log=lambda m: None)


def test_licences():
    assert wm.classify("cc0", []) == wm.CC0_PD
    assert wm.classify("pd", []) == wm.CC0_PD
    assert wm.classify("cc-by-4.0", []) == wm.CC_BY
    assert wm.classify("cc-by-2.0", []) == wm.CC_BY
    for refused in ["cc-by-sa-4.0", "cc-by-nc-2.0", "cc-by-nd-4.0", "gfdl", "", "cc-by-sa-3.0-de"]:
        assert wm.classify(refused, []) is None, refused
    assert wm.classify("cc-by-sa-4.0", ["CC-BY-SA-4.0", "CC-BY-4.0"]) == wm.CC_BY, "offered under CC BY too"
    assert wm.classify("gfdl", ["GFDL", "CC-Zero"]) == wm.CC0_PD


def test_only_photos_pass():
    ok = page(1)["imageinfo"][0]
    assert wm.photo_problem(ok, ["Taken with Fujifilm X-T5"]) is None
    assert wm.photo_problem({**ok, "mime": "image/png"}, []) == "not a JPEG"
    assert wm.photo_problem({**ok, "width": 1200, "height": 900}, []) == "too small"
    assert wm.photo_problem({**ok, "width": 9000, "height": 3000}, []) == "panorama"
    assert wm.photo_problem({**ok, "commonmetadata": []}, []) == "no camera"
    for cat in ["Paintings by Monet", "Scanned photographs", "HDR images", "Maps of Paris", "Photomicrographs"]:
        assert wm.photo_problem(ok, [cat]) is not None, cat
    assert wm.photo_problem(ok, ["Mapplethorpe Street"]) is None, "words, not letters"
    # What the first live run let through.
    for cat in ["Artworks without Wikidata item", "Microscopic images of Crustacea", "Electron microscope images"]:
        assert wm.photo_problem(ok, [cat]) is not None, cat
    assert wm.photo_problem(ok, [], "File:Water mite (Hydrachnidia) dorsal view under microscope.jpg") == "title"
    assert wm.photo_problem(ok, [], "File:Micrograph of a Simocephalus water flea.jpg") == "title"
    assert wm.photo_problem(ok, ["Focus stacking images of Arachnida"], "File:Jumping spider.jpg") is None


def test_portraits_ask_for_a_narrower_thumbnail():
    info = page(1, w=3000, h=4000)["imageinfo"][0]
    assert wm.thumb_url(info).endswith("/960px-c1.jpg")
    assert wm.thumb_url(page(2)["imageinfo"][0]).endswith("/1280px-c2.jpg")


def test_a_run_keeps_the_good_files_and_says_why_it_left_the_others(tmp_path: Path):
    wiki = FakeWiki({wm.FEATURED: [[page(1), page(2, licence="cc-by-sa-4.0"), page(3, licence="cc-by-4.0")], [page(4, colour=False), page(5, cats="Paintings by X"), page(6, camera=False)]]})
    d = downloader(tmp_path, wiki)
    counts = d.run(wm.FEATURED, 100)
    out = tmp_path / "originals"
    assert counts.kept == 2 and counts.looked == 6
    assert (out / wm.CC0_PD / "1.jpg").exists() and (out / wm.CC_BY / "3.jpg").exists()
    assert not list(out.rglob("*.part"))
    why = {json.loads(l)["pageid"]: json.loads(l).get("skipped") for l in (out / "manifest.jsonl").read_text(encoding="utf-8").splitlines()}
    assert why == {1: None, 2: "licence", 3: None, 4: "black and white", 5: "category Paintings by X", 6: "no camera"}
    rows = list(csv.DictReader(open(out / wm.CC_BY / "credits.csv", encoding="utf-8")))
    assert rows == [{"file": "3.jpg", "title": "File:P3.jpg", "author": "Ana B", "licence": "CC-BY-4.0", "licence_url": "https://creativecommons.org/x", "page": "https://commons.example/File:P3.jpg"}]
    assert len(wiki.downloads) == 3, "refused licences and non-photos are never downloaded"


def test_a_stopped_run_carries_on_where_it_was(tmp_path: Path):
    wiki = FakeWiki({wm.QUALITY: [[page(i) for i in range(1, 4)], [page(i) for i in range(4, 7)], [page(i) for i in range(7, 10)]]})
    assert downloader(tmp_path, wiki).run(wm.QUALITY, 4).kept == 4
    calls, downloads = wiki.api_calls, len(wiki.downloads)
    again = downloader(tmp_path, wiki).run(wm.QUALITY, 4)
    assert again.kept == 4 and wiki.api_calls == calls and len(wiki.downloads) == downloads, "nothing asked twice"
    more = downloader(tmp_path, wiki).run(wm.QUALITY, 100)
    assert more.kept == 9
    assert len(wiki.downloads) == 9, "every file downloaded once"
    assert json.loads((tmp_path / "originals" / "state.json").read_text(encoding="utf-8"))[wm.QUALITY]["done"]


def test_a_file_seen_in_one_category_is_not_fetched_again_for_another(tmp_path: Path):
    wiki = FakeWiki({wm.FEATURED: [[page(1)]], wm.QUALITY: [[page(1), page(2)]]})
    d = downloader(tmp_path, wiki)
    d.run(wm.FEATURED, 10)
    assert d.run(wm.QUALITY, 10).kept == 1
    assert len(wiki.downloads) == 2


def test_a_page_without_its_details_yet_is_left_for_later(tmp_path: Path):
    bare = {"pageid": 7, "title": "File:P7.jpg"}
    wiki = FakeWiki({wm.FEATURED: [[bare], [page(7)]]})
    assert downloader(tmp_path, wiki).run(wm.FEATURED, 10).kept == 1


def test_too_many_requests_backs_off_as_asked(tmp_path: Path):
    wiki = FakeWiki({wm.FEATURED: [[page(1)]]})
    sleeps: list[float] = []
    wiki.fail_next = 2
    assert downloader(tmp_path, wiki, sleeps).run(wm.FEATURED, 10).kept == 1
    assert sleeps.count(2.0) == 2, sleeps


def test_a_refused_request_that_never_recovers_is_an_error(tmp_path: Path):
    wiki = FakeWiki({wm.FEATURED: [[page(1)]]})
    wiki.fail_next = 100
    with pytest.raises(urllib.error.HTTPError):
        downloader(tmp_path, wiki).run(wm.FEATURED, 10)
