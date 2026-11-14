"""Lumen Prism's few-shot kit: folds by stem, input-only augmentation, the 1:3 mix, the
leak-free warm start, cross-validation and its gate."""

import json
from pathlib import Path

import pytest
import torch

from lumen_train.common import ckpt, config as config_mod
from lumen_train.data import splits
from lumen_train.prism import augment, cv, fit, folds
from lumen_train.retouch import params, train
from lumen_train.retouch.dataset import PreparedDataset, collate
from lumen_train.retouch.model import RetouchNet
from lumen_train.retouch.prepared import decode_input, encode_input, read_raw
from lumen_train.tests.test_dataset import write_sample

GEOMETRY = ["straighten", "crop_on", "crop_x", "crop_y", "crop_w", "crop_h", "crop_ratio"]
SHARES = splits.Shares(val=12, test=18)
# img_0000 … img_0007: 2 and 6 fall in the test split, 4 in val, the rest in train.
STEMS = [f"img_{i:04d}" for i in range(8)]


def write_pair(folder: Path, w: int, h: int, level: float, gain: float = 1.4, known: list[str] | None = GEOMETRY) -> None:
    """A prepared photo, his edit of it (brighter by `gain`) and the sliders an aligned pair knows."""
    write_sample(folder, w, h, level)
    lin, _, _, _ = decode_input(read_raw(folder))
    target = encode_input(lin[:, :h, :w] * gain, w, h)
    (target.clamp(0, 1) * 65535).round().numpy().astype("<u2").tofile(folder / "target.u16")
    doc = {"v": params.neutral_vector().tolist()}
    if known is not None:
        doc["known"] = known
    (folder / "target.json").write_text(json.dumps(doc), encoding="utf-8")


@pytest.fixture
def pairs_root(tmp_path: Path) -> Path:
    return make_root(tmp_path / "data")


def make_root(root: Path) -> Path:
    root.mkdir()
    (root / "sources.toml").write_text(
        '[sources.julian-pairs]\nurl = ""\nlicence = "test"\ncommercial = true\nkind = "raw+jpeg"\nattribution = false\n'
        "[sources.julian-pairs.split]\nval = 12\ntest = 18\n",
        encoding="utf-8",
    )
    for i, s in enumerate(STEMS):
        write_pair(root / "julian-pairs" / f"{s}-{i:016x}", 96 if i % 2 else 64, 64 if i % 2 else 96, 0.05 + 0.02 * i)
    write_pair(root / "julian-pairs" / f"img_0003-edit-{99:016x}", 80, 64, 0.3)  # a second edit of one photo
    write_sample(root / "julian-pairs" / f"img_0001-{98:016x}", 96, 64, 0.1)  # an original nobody finished
    for i in range(12):
        write_sample(root / "synthetic-test" / f"p{i:02d}", 96 if i % 3 else 64, 64 if i % 3 else 96, 0.04 + 0.02 * i)
    return root


# --- folds -----------------------------------------------------------------------------------


def test_folds_are_deterministic_disjoint_and_even():
    stems = [f"s{i}" for i in range(23)]
    a = folds.of(stems, k=5)
    assert a == folds.of(list(reversed(stems)), k=5)
    assert len(a) == 5
    assert sorted(s for f in a for s in f) == sorted(stems)
    sizes = [len(f) for f in a]
    assert max(sizes) - min(sizes) <= 1


def test_folds_refuse_more_folds_than_photos():
    with pytest.raises(ValueError):
        folds.of(["a", "b"], k=3)


def test_pairs_lists_non_test_pairs_by_stem(pairs_root: Path):
    got = folds.pairs(pairs_root)
    assert set(got) == {s for s in STEMS if splits.split_of(s, SHARES) != "test"}
    assert {f.name for f in got["img_0003"]} == {f"img_0003-{3:016x}", f"img_0003-edit-{99:016x}"}
    assert [f.name for f in got["img_0001"]] == [f"img_0001-{1:016x}"]  # the unfinished original is no pair


def test_folders_of_folds_keep_a_photos_edits_together(pairs_root: Path):
    by_stem = folds.pairs(pairs_root)
    groups = folds.of(sorted(by_stem), k=3)
    for g in groups:
        names = {f.name for f in folds.folders(by_stem, g)}
        assert (f"img_0003-{3:016x}" in names) == (f"img_0003-edit-{99:016x}" in names)


# --- augmentation ----------------------------------------------------------------------------


def _pairs_ds(root: Path) -> PreparedDataset:
    return PreparedDataset(root, ["julian-pairs"], split="all", augment=False)


def test_photometric_changes_only_the_source_side(pairs_root: Path):
    item = _pairs_ds(pairs_root)[0]
    out = augment.photometric(item, torch.Generator().manual_seed(1))
    for k in ("target", "target_void", "mask", "v_target", "v_known"):
        assert torch.equal(out[k], item[k]), k
    assert not torch.allclose(out["aligned"], item["aligned"])
    assert torch.equal(out["input"][3], item["input"][3])
    w, h = int(item["mask"][0, 0].sum()), int(item["mask"][0, :, 0].sum())
    assert torch.allclose(out["input"], encode_input(out["aligned"][:, :h, :w], w, h), atol=1e-6)
    assert out["aligned"][:, h:, :].abs().sum() == 0 and out["aligned"][:, :, w:].abs().sum() == 0


def test_photometric_forgets_the_look_a_pair_knew(pairs_root: Path):
    folder = pairs_root / "julian-pairs" / f"img_0000-{0:016x}"
    write_pair(folder.with_name("full-" + folder.name), 64, 64, 0.2, known=None)  # a pair that knows every slider
    ds = _pairs_ds(pairs_root)
    i = next(j for j, f in enumerate(ds.samples) if f.name.startswith("full-"))
    out = augment.photometric(ds[i], torch.Generator().manual_seed(0))
    kept = {params.PARAMS[j]["name"] for j in range(len(params.PARAMS)) if out["v_known"][j] > 0}
    assert kept == set(GEOMETRY)


def test_apply_exposure_and_contrast():
    lin = torch.full((3, 4, 4), 0.18)
    assert torch.allclose(augment.apply(lin, augment.Look(ev=1.0)), lin * 2)
    assert torch.allclose(augment.apply(lin, augment.Look(contrast=1.2)), lin)  # 0.18 is the pivot
    brighter = augment.apply(torch.full((3, 1, 1), 0.36), augment.Look(contrast=1.2))
    assert brighter[0, 0, 0] > 0.36
    warm = augment.apply(lin, augment.Look(kelvin=800.0))
    assert warm[0, 0, 0] > 0.18 > warm[2, 0, 0]
    magenta = augment.apply(lin, augment.Look(tint=10.0))
    assert magenta[1, 0, 0] < 0.18


def test_draws_stay_in_their_ranges():
    for seed in range(200):
        look = augment.Look.draw(torch.Generator().manual_seed(seed))
        assert -1 <= look.ev <= 1 and -800 <= look.kelvin <= 800 and -10 <= look.tint <= 10 and 0.8 <= look.contrast <= 1.2


def test_prism_pairs_gives_many_views_per_pair(pairs_root: Path):
    base = _pairs_ds(pairs_root)
    pp = augment.PrismPairs(base, views=8, seed=0)
    assert len(pp) == len(base) * 8
    first = pp[0]["aligned"]
    assert not torch.equal(first, pp[1]["aligned"])  # another view of the same photo
    assert torch.equal(augment.PrismPairs(base, views=8, seed=0)[0]["aligned"], first)
    pp.set_epoch(1)
    assert not torch.equal(pp[0]["aligned"], first)


# --- the weighted loss, the mix and the warm start -------------------------------------------

W = {"param": 1.0, "de": 1.0, "iou": 0.5, "presence": 0.2}


def _batch(root: Path) -> tuple[dict, torch.Tensor]:
    ds = _pairs_ds(root)
    batch = collate([ds[i] for i in range(4)])
    v = torch.rand(4, len(params.PARAMS), generator=torch.Generator().manual_seed(3))
    return batch, v


def test_loss_with_unit_weights_is_the_plain_loss(pairs_root: Path):
    batch, v = _batch(pairs_root)
    plain, _ = train.loss_of(v, batch, W, hard=True)
    weighted, _ = train.loss_of(v, {**batch, "weight": torch.ones(4)}, W, hard=True)
    assert torch.allclose(plain, weighted, atol=1e-6)


def test_loss_weights_rows(pairs_root: Path):
    batch, v = _batch(pairs_root)
    alone, _ = train.loss_of(v[:1], {k: x[:1] if isinstance(x, torch.Tensor) else x for k, x in batch.items()}, W, hard=True)
    only_first, _ = train.loss_of(v, {**batch, "weight": torch.tensor([1.0, 0.0, 0.0, 0.0])}, W, hard=True)
    assert torch.allclose(alone, only_first, atol=1e-5)


def test_mix_sampler_gives_one_pair_to_three_others():
    s = fit.MixSampler(n_pairs=10, n_wiki=50, batch=8, seed=0)
    batches = list(s)
    assert len(batches) == len(s) == 5
    for b in batches:
        assert len(b) == 8 and sum(i < 10 for i in b) == 2 and all(i < 60 for i in b)
    assert sorted(i for b in batches for i in b if i < 10) == list(range(10))
    assert batches == list(fit.MixSampler(n_pairs=10, n_wiki=50, batch=8, seed=0))
    s.set_epoch(1)
    assert list(s) != batches
    with pytest.raises(ValueError):
        fit.MixSampler(n_pairs=10, n_wiki=50, batch=3)


def _ckpt(path: Path, sources: list[str] | None, init: Path | None = None) -> Path:
    extra = {"layout": params.LAYOUT, "config": {"train": {"init": str(init) if init else ""}}}
    if sources is not None:
        extra["sources"] = sources
    torch.manual_seed(0)
    ckpt.save(path, RetouchNet(), None, 1, 1.0, extra)
    return path


def test_leak_free_follows_the_warm_start_chain(tmp_path: Path):
    wiki = _ckpt(tmp_path / "a.pt", ["wikimedia-cc0-pd"])
    on_wiki = _ckpt(tmp_path / "b.pt", ["wikimedia-cc-by"], init=wiki)
    assert fit.leak_free(on_wiki) == [on_wiki, wiki]
    with pytest.raises(fit.LeakError):
        fit.leak_free(_ckpt(tmp_path / "c.pt", ["julian-pairs", "wikimedia-cc-by"]))
    with pytest.raises(fit.LeakError):
        fit.leak_free(_ckpt(tmp_path / "d.pt", ["wikimedia-cc-by"], init=_ckpt(tmp_path / "e.pt", ["julian-pairs"])))
    with pytest.raises(fit.LeakError):
        fit.leak_free(_ckpt(tmp_path / "f.pt", None))
    with pytest.raises(fit.LeakError):
        fit.leak_free(_ckpt(tmp_path / "g.pt", ["wikimedia-cc-by"], init=tmp_path / "gone.pt"))


def test_encoder_is_frozen_for_the_first_third():
    factors = [fit.encoder_factor(e, epochs=9, freeze=1 / 3, after=0.1) for e in range(9)]
    assert factors == [0.0, 0.0, 0.0] + [0.1] * 6


# --- cross-validation ------------------------------------------------------------------------


def _auto(files: list[Path]) -> list[dict | None]:
    return [{"exposure": 0.3} for _ in files]


def make_cfg(root: Path, tmp: Path) -> dict:
    base = _ckpt(tmp / "base" / "best_de.pt", ["synthetic-test"])
    return config_mod.load(cv.DEFAULT_CONFIG, [
        f"data.root={root.as_posix()}", 'data.wiki=["synthetic-test"]', "data.views=1", "data.workers=0",
        f"train.init={base.as_posix()}", "train.epochs=2", "train.batch=4", "train.device=cpu", "train.amp=false",
        f"run.dir={(tmp / 'prism').as_posix()}",
    ])


@pytest.fixture
def cfg(pairs_root: Path, tmp_path: Path) -> dict:
    return make_cfg(pairs_root, tmp_path)


@pytest.fixture(scope="module")
def cv_done(tmp_path_factory) -> tuple[dict, dict]:
    """One 3-fold cross-validation (2 epochs on CPU), shared: it is the slow part."""
    tmp = tmp_path_factory.mktemp("cv")
    cfg = make_cfg(make_root(tmp / "data"), tmp)
    return cfg, cv.run(cfg, k=3, auto_basic=_auto)


def test_cv_scores_every_fold_against_the_base_and_auto(cv_done):
    cfg, report = cv_done
    out = Path(cfg["run"]["dir"])
    assert json.loads((out / "cv.json").read_text(encoding="utf-8")) == report
    assert [f["fold"] for f in report["folds"]] == [0, 1, 2]
    assert sorted(s for f in report["folds"] for s in f["stems"]) == sorted(folds.pairs(cfg["data"]["root"]))
    for f in report["folds"]:
        assert set(f) >= {"prism", "prism_best", "best_epoch", "base", "auto", "history"}
        assert 1 <= f["best_epoch"] <= 2 and len(f["history"]) == 3  # the base, then each epoch
        assert f["base"] == f["history"][0]
        state = ckpt.read(out / f"fold-{f['fold']}" / "best_de.pt")
        assert state["extra"]["sources"] == ["julian-pairs", "synthetic-test"]
        assert state["extra"]["prism"]["fold"] == f["fold"]
    assert report["median_epoch"] in (1, 2)
    for k in ("prism", "base", "auto"):
        assert set(report[k]) == {"mean", "std"}


def test_cv_skips_finished_folds(cv_done, monkeypatch):
    cfg, first = cv_done
    monkeypatch.setattr(cv.fit, "fit", lambda *a, **k: pytest.fail("a finished fold trained again"))
    assert cv.run(cfg, k=3, auto_basic=_auto) == first


def test_final_fits_every_pair_for_the_median_epoch(cv_done):
    cfg, report = cv_done
    cv.final(cfg)
    state = ckpt.read(Path(cfg["run"]["dir"]) / "final" / "best_de.pt")
    n = sum(len(fs) for fs in folds.pairs(cfg["data"]["root"]).values())
    assert state["extra"]["prism"] == {"fold": None, "epochs": report["median_epoch"], "pairs": n}


def test_gate_needs_prism_below_the_base_and_auto():
    r = lambda p, b, a: {"prism": {"mean": p, "std": 0.1}, "base": {"mean": b, "std": 0.1}, "auto": {"mean": a, "std": 0.1}}
    assert cv.gate(r(2.0, 3.0, 4.0))[0]
    assert not cv.gate(r(3.5, 3.0, 4.0))[0]
    assert not cv.gate(r(3.5, 4.0, 3.0))[0]
    assert not cv.gate({**r(2.0, 3.0, 4.0), "auto": {"mean": None, "std": None}})[0]
    assert not cv.gate({**r(2.0, 3.0, 4.0), "k": 5, "finished": 1})[0]  # a partial run never passes


def test_prism_is_a_retouch_model():
    from lumen_train.common import naming

    assert naming.FAMILIES["prism"].role == "retouch"
    assert "Retouch" in naming.FAMILIES["prism"].purpose


def test_fit_refuses_a_warm_start_that_saw_his_pairs(cfg: dict, tmp_path: Path):
    leaky = _ckpt(tmp_path / "leaky.pt", ["julian-pairs"])
    by_stem = folds.pairs(cfg["data"]["root"])
    with pytest.raises(fit.LeakError):
        fit.fit({**cfg, "train": {**cfg["train"], "init": str(leaky)}}, folds.folders(by_stem, sorted(by_stem)), [], tmp_path / "x")
