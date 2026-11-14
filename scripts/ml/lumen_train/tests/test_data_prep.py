"""Incremental prep and status, with a fake `retouch_prep`."""

import json
from pathlib import Path

import pytest

from lumen_train.data import prep, sources, status


def fake_runner(original: Path, dest: Path) -> Path:
    if "bad" in original.name:
        raise RuntimeError("decode failed: bad.jpg")
    folder = dest / f"{original.stem}-{abs(hash(str(original))) % 16**16:016x}"
    folder.mkdir(parents=True, exist_ok=True)
    (folder / "input.u16").write_bytes(b"\0\0")
    (folder / "meta.json").write_text(json.dumps({"source": str(original)}), encoding="utf-8")
    return folder


@pytest.fixture
def tree(tmp_path: Path):
    orig = tmp_path / "orig"
    (orig / "Ä Folder").mkdir(parents=True)
    (orig / "a.jpg").write_bytes(b"aaa")
    (orig / "Ä Folder" / "Été 01.jpg").write_bytes(b"bbb")
    return tmp_path / "root", orig


def test_second_run_skips_and_a_changed_file_is_redone(tree):
    root, orig = tree
    r = prep.run(root, "s", orig, runner=fake_runner)
    assert (r.prepared, r.skipped) == (2, 0)
    r = prep.run(root, "s", orig, runner=fake_runner)
    assert (r.prepared, r.skipped) == (0, 2)
    (orig / "a.jpg").write_bytes(b"new bytes")
    r = prep.run(root, "s", orig, runner=fake_runner)
    assert (r.prepared, r.skipped) == (1, 1)
    assert any("Été 01" in rel for rel in prep.read_index(root / "s"))


def test_failure_is_recorded_and_the_run_continues(tree):
    root, orig = tree
    (orig / "bad.jpg").write_bytes(b"x")
    r = prep.run(root, "s", orig, runner=fake_runner)
    assert r.prepared == 2 and r.failed == [("bad.jpg", "decode failed: bad.jpg")]
    assert prep.read_index(root / "s")["bad.jpg"]["ok"] is False


HE = "decoder failed: NEF compression Some(HighEfficencyStar) is not supported"


def he_runner(original: Path, dest: Path) -> Path:
    if original.suffix.lower() == ".nef":
        raise RuntimeError(HE)
    return fake_runner(original, dest)


def fake_embedded(raw: Path, out_dir: Path) -> Path:
    out_dir.mkdir(parents=True, exist_ok=True)
    out = out_dir / (raw.stem + ".jpg")
    out.write_bytes(b"camera jpeg of " + raw.read_bytes())
    return out


def test_a_high_efficiency_nef_is_prepared_from_its_embedded_jpeg(tree):
    """Nikon's HE / HE* NEFs (Z f, Z 8, Z 9…) have no open decoder: the photo is prepared from the
    full-size JPEG inside the NEF, written under the source's own folder, and marked as a stand-in."""
    root, orig = tree
    (orig / "DSC_5837.NEF").write_bytes(b"he")
    r = prep.run(root, "s", orig, runner=he_runner, embedded=fake_embedded, to_dng=lambda raw, out: None)
    assert r.failed == [] and r.prepared == 3
    row = prep.read_index(root / "s")["DSC_5837.NEF"]
    assert row["ok"] and row["via"] == "embedded JPEG"
    folder = root / "s" / row["folder"]
    stand_in = json.loads((folder / "stand_in.json").read_text(encoding="utf-8"))
    assert Path(stand_in["raw"]) == orig / "DSC_5837.NEF" and stand_in["via"] == "embedded JPEG"
    assert Path(json.loads((folder / "meta.json").read_text(encoding="utf-8"))["source"]).parent == root / "s" / "_embedded"
    assert not any(p.name.startswith("DSC_5837") and p.suffix == ".jpg" for p in orig.rglob("*"))  # originals untouched


def dng_runner(original: Path, dest: Path) -> Path:
    if original.suffix.lower() == ".nef":
        raise RuntimeError(HE)
    return fake_runner(original, dest)


def fake_dng(raw: Path, out_dir: Path) -> Path:
    out_dir.mkdir(parents=True, exist_ok=True)
    out = out_dir / (raw.stem + ".dng")
    out.write_bytes(b"sensor data")
    return out


def test_adobe_dng_converter_wins_over_the_embedded_jpeg(tree):
    """With Adobe DNG Converter installed (it licenses Nikon's codec), the HE NEF's own sensor
    data is prepared from a DNG; the embedded JPEG is only the fallback."""
    root, orig = tree
    (orig / "DSC_5837.NEF").write_bytes(b"he")
    r = prep.run(root, "s", orig, runner=dng_runner, embedded=fake_embedded, to_dng=fake_dng)
    row = prep.read_index(root / "s")["DSC_5837.NEF"]
    assert r.failed == [] and row["via"] == "Adobe DNG Converter"
    meta = json.loads((root / "s" / row["folder"] / "meta.json").read_text(encoding="utf-8"))
    assert meta["source"].endswith(".dng")


def test_without_the_converter_the_embedded_jpeg_is_used(tree):
    root, orig = tree
    (orig / "DSC_5837.NEF").write_bytes(b"he")
    prep.run(root, "s", orig, runner=dng_runner, embedded=fake_embedded, to_dng=lambda raw, out: None)
    assert prep.read_index(root / "s")["DSC_5837.NEF"]["via"] == "embedded JPEG"


def test_retouch_preps_reason_is_its_skipped_line():
    """`retouch_prep` ends with a progress line; the reason is on the `skipped` line before it."""
    out = "prepared 0 of 1 into C:/x\nskipped C:/a/DSC_5837.NEF: decode failed: raw: NEF compression Some(HighEfficencyStar) is not supported\n1 / 1\n"
    why = prep.failure_reason(out, 0)
    assert why.startswith("decode failed") and prep.no_open_decoder(Path("DSC_5837.NEF"), RuntimeError(why))
    assert prep.failure_reason("", 3) == "retouch_prep exited 3"
    assert prep.failure_reason("boom\n", 101) == "boom"


def test_other_raw_failures_are_not_rescued(tree):
    root, orig = tree
    (orig / "broken.NEF").write_bytes(b"x")

    def runner(original: Path, dest: Path) -> Path:
        if original.suffix.lower() == ".nef":
            raise RuntimeError("decoder failed: truncated file")
        return fake_runner(original, dest)

    called = []
    r = prep.run(root, "s", orig, runner=runner, embedded=lambda raw, out: called.append(raw) or raw)
    assert r.failed == [("broken.NEF", "decoder failed: truncated file")] and called == []


def test_status_counts_failed_and_orphaned(tree):
    root, orig = tree
    (orig / "bad.jpg").write_bytes(b"x")
    sources.add(root, "s", orig, "finals", "mine", True)
    prep.run(root, "s", orig, runner=fake_runner)
    (orig / "a.jpg").unlink()  # the original goes; its prepared item stays
    (row,) = [r for r in status.table(root) if r.source == "s"]
    assert row.failed == 1 and row.orphaned == 1
    assert row.train + row.val + row.test == 2
