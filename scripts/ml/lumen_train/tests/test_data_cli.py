"""Sources, the merged registry and the CLI."""

from pathlib import Path

import pytest

from lumen_train.common import licence
from lumen_train.data import __main__ as cli
from lumen_train.data import prep, sources, splits


def test_add_writes_sources_and_load_merges(tmp_path: Path):
    folder = tmp_path / "contributor"
    folder.mkdir()
    s = sources.add(tmp_path / "root", "contributor-finals", folder, "finals", "permission", True, split=splits.Shares(12, 18))
    assert (tmp_path / "root" / "sources.toml").exists()
    got = sources.load(tmp_path / "root")
    assert got["contributor-finals"] == s and s.split == splits.Shares(12, 18) and s.local
    assert got["wikimedia-cc0-pd"].split == splits.Shares(15, 2) and not got["wikimedia-cc0-pd"].local


def test_name_in_both_registries_fails(tmp_path: Path):
    with pytest.raises(sources.SourceError, match="registry.toml"):
        sources.add(tmp_path, "wikimedia-cc0-pd", tmp_path, "finals", "x", True)


def test_add_refuses_a_library(tmp_path: Path):
    lib = tmp_path / "L"
    lib.mkdir()
    (lib / "library.db").write_bytes(b"")
    with pytest.raises(sources.SourceError):
        sources.add(tmp_path / "root", "x", lib, "finals", "x", True)
    with pytest.raises(sources.SourceError):
        sources.add(tmp_path / "root", "y", tmp_path / "Pictures" / "Lumen Library", "finals", "x", True)


def test_licence_gate_reads_the_merged_registry(tmp_path: Path):
    for name, commercial in (("yes", True), ("no", False)):
        f = tmp_path / name
        f.mkdir()
        sources.add(tmp_path / "root", name, f, "finals", "x", commercial)
    reg = sources.merged_entries(tmp_path / "root")
    licence.check(["yes"], ship=True, registry=reg)
    with pytest.raises(licence.LicenceError):
        licence.check(["no"], ship=True, registry=reg)


def test_cli_add_prep_status(tmp_path: Path, capsys, monkeypatch):
    orig = tmp_path / "o"
    orig.mkdir()
    (orig / "a.jpg").write_bytes(b"a")
    root = str(tmp_path / "root")

    def fake(original, dest):
        d = dest / "a-0000000000000000"
        d.mkdir(exist_ok=True)
        (d / "input.u16").write_bytes(b"")
        (d / "meta.json").write_text("{}")
        return d

    monkeypatch.setattr(prep, "run_retouch_prep", fake)
    assert cli.main(["--root", root, "add", "mine", str(orig), "--kind", "finals", "--licence", "L", "--commercial"]) == 0
    assert cli.main(["--root", root, "prep"]) == 0
    assert cli.main(["--root", root, "status"]) == 0
    assert "mine" in capsys.readouterr().out


def test_the_fathers_two_sources_keep_each_photo_on_one_side(tmp_path: Path):
    """Review: no test-split leakage. His finals and his RAW + JPEG pairs share stems; each stem
    lands in one split in both, training never sees a test stem, and a RAW whose pair was
    skipped (no `target.json`) is never a training photo."""
    import json

    from lumen_train.retouch import params
    from lumen_train.retouch.dataset import PreparedDataset
    from lumen_train.tests.test_dataset import write_sample

    root = tmp_path / "root"
    shares = splits.Shares(12, 18)
    for name, kind in (("contributor-finals", "finals"), ("contributor-pairs", "raw+jpeg")):
        (tmp_path / name).mkdir()
        sources.add(root, name, tmp_path / name, kind, "permission", True, split=shares)
    stems = [f"dsc_{i:04d}" for i in range(80)]
    for i, s in enumerate(stems):
        write_sample(root / "contributor-finals" / f"{s}-edit-{i:016x}", 32, 24, 0.2)
        pair = root / "contributor-pairs" / f"{s}-{i + 1000:016x}"
        write_sample(pair, 32, 24, 0.1)
        if i != 7:
            (pair / "target.json").write_text(json.dumps({"v": params.neutral_vector().tolist(), "known": ["straighten"]}), encoding="utf-8")

    def stems_of(split: str) -> dict[str, set[str]]:
        ds = PreparedDataset(root, ["contributor-finals", "contributor-pairs"], split=split, augment=False)
        out: dict[str, set[str]] = {"contributor-finals": set(), "contributor-pairs": set()}
        for folder, src in zip(ds.samples, ds.sources):
            out[src].add(splits.stem(folder.name))
        return out

    test_stems = {s for s in stems if splits.split_of(s, shares) == "test"}
    assert test_stems, "the fixture needs some test stems"
    seen = {}
    for split in ("train", "val", "test"):
        got = stems_of(split)
        assert got["contributor-pairs"] == got["contributor-finals"] - {stems[7]}, split
        seen[split] = got["contributor-finals"]
    assert seen["test"] == test_stems
    assert not (seen["train"] & test_stems)
    assert stems[7] not in stems_of("all")["contributor-pairs"]


def test_add_refuses_a_folder_inside_a_library(tmp_path: Path):
    """Review 7: a library stored anywhere, its `originals/` (or deeper) named directly."""
    lib = tmp_path / "D" / "MyLumenLib"
    (lib / "originals" / "2024").mkdir(parents=True)
    (lib / "library.db").write_bytes(b"")
    for folder in (lib / "originals", lib / "originals" / "2024"):
        with pytest.raises(sources.SourceError, match="Lumen library"):
            sources.add(tmp_path / "root", "x", folder, "finals", "x", True)


def test_add_stores_the_folder_absolute(tmp_path: Path, monkeypatch):
    """A relative folder would be read relative to wherever the next command runs."""
    (tmp_path / "contributor").mkdir()
    monkeypatch.chdir(tmp_path)
    src = sources.add(tmp_path / "root", "contributor-x", Path("contributor"), "finals", "x", True)
    assert Path(src.originals).is_absolute() and Path(src.originals) == (tmp_path / "contributor").resolve()
