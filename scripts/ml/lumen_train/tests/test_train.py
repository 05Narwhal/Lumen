"""The training loop, end to end on a few synthetic photos on the CPU."""

from pathlib import Path

import pytest
import torch

from lumen_train.common import ckpt
from lumen_train.common.licence import LicenceError
from lumen_train.retouch import train
from lumen_train.retouch.model import RetouchNet
from lumen_train.tests.test_dataset import write_sample


def config(root: Path, run: Path, sources: list[str], epochs: int = 4) -> dict:
    return {
        "data": {"root": str(root), "sources": sources, "workers": 0},
        "run": {"dir": str(run), "ship": False, "resume": True},
        "train": {"epochs": epochs, "batch": 4, "lr": 1e-3, "weight_decay": 0.0, "warmup": 0, "amp": False, "seed": 0, "device": "cpu"},
        "loss": {"param": 1.0, "de": 1.0, "iou": 0.5, "presence": 0.2},
    }


@pytest.fixture
def root(tmp_path: Path) -> Path:
    for i in range(24):
        write_sample(tmp_path / "data" / "synthetic-test" / f"p{i:02d}", 96, 64, 0.04 + 0.02 * i)
    return tmp_path / "data"


def test_training_lowers_the_validation_error_and_keeps_its_checkpoints(root: Path, tmp_path: Path):
    out = train.run(config(root, tmp_path / "run", ["synthetic-test"]))
    assert out["epochs"] == 4
    losses = [v["loss"] for v in out["val"]]
    assert min(losses[1:]) < losses[0], losses  # the best epoch (best.pt) beats the start
    assert (tmp_path / "run" / "best.pt").exists() and (tmp_path / "run" / "last.pt").exists()
    state = ckpt.load(tmp_path / "run" / "best.pt", RetouchNet())
    assert state["extra"]["sources"] == ["synthetic-test"]
    assert state["extra"]["ship"] is False
    assert (tmp_path / "run" / "log.csv").exists()


def test_a_finished_run_resumes_instead_of_starting_over(root: Path, tmp_path: Path):
    train.run(config(root, tmp_path / "run", ["synthetic-test"], epochs=1))
    out = train.run(config(root, tmp_path / "run", ["synthetic-test"], epochs=2))
    assert out["started_at"] == 1 and out["epochs"] == 2


def test_shipping_refuses_a_run_with_a_non_commercial_source(root: Path, tmp_path: Path):
    (root / "julian-catalog").mkdir()
    cfg = config(root, tmp_path / "run", ["synthetic-test", "julian-catalog"], epochs=1)
    cfg["run"]["ship"] = True
    with pytest.raises(LicenceError):
        train.run(cfg)


def test_the_loss_has_every_part():
    m = RetouchNet()
    b = 2
    batch = {
        "input": torch.rand(b, 4, 64, 64),
        "aligned": torch.rand(b, 3, 64, 64) * 0.5,
        "target": torch.rand(b, 3, 64, 64) * 0.5,
        "mask": torch.ones(b, 1, 64, 64),
        "has_target": torch.ones(b),
        "v_target": torch.rand(b, len(train.params.PARAMS)),
        "v_known": torch.ones(b, len(train.params.PARAMS)),
        "aspect": torch.ones(b),
        "clip": torch.zeros(b, 512),
        "clip_flag": torch.zeros(b, 1),
    }
    batch["v_target"][:, train.params.index("crop_on")] = 1.0
    v = m(batch["input"], batch["clip"], batch["clip_flag"])
    loss, parts = train.loss_of(v, batch, {"param": 1.0, "de": 1.0, "iou": 0.5, "presence": 0.2})
    assert set(parts) == {"param", "de", "iou", "presence"}
    assert torch.isfinite(loss)
    loss.backward()


def test_a_run_never_becomes_shippable_by_resuming_it(root: Path, tmp_path: Path):
    """Epochs trained without the gate stay in the weights: `--ship` cannot pick them up."""
    train.run(config(root, tmp_path / "run", ["synthetic-test"], epochs=1))
    cfg = config(root, tmp_path / "run", ["synthetic-test"], epochs=2)
    cfg["run"]["ship"] = True
    with pytest.raises(LicenceError, match="new run.dir"):
        train.run(cfg)


def test_a_run_resumes_only_on_the_sources_it_started_with(root: Path, tmp_path: Path):
    for i in range(6):
        write_sample(root / "wikimedia-cc0-pd" / f"q{i}", 64, 64, 0.1)
    train.run(config(root, tmp_path / "run", ["synthetic-test"], epochs=1))
    with pytest.raises(ValueError, match="trained on"):
        train.run(config(root, tmp_path / "run", ["synthetic-test", "wikimedia-cc0-pd"], epochs=2))


def test_a_resumed_run_keeps_its_log_and_schedule(root: Path, tmp_path: Path):
    train.run(config(root, tmp_path / "run", ["synthetic-test"], epochs=1))
    before = (tmp_path / "run" / "log.csv").read_text(encoding="utf-8").splitlines()
    steps = ckpt.read(tmp_path / "run" / "last.pt")["extra"]["sched"]["last_epoch"]
    train.run(config(root, tmp_path / "run", ["synthetic-test"], epochs=2))
    after = (tmp_path / "run" / "log.csv").read_text(encoding="utf-8").splitlines()
    assert after[1 : len(before)] == before[1:] and len(after) > len(before), "the first epoch's rows are still there"
    assert ckpt.read(tmp_path / "run" / "last.pt")["extra"]["sched"]["last_epoch"] == 2 * steps, "the schedule counted on, not from zero"


def test_an_empty_split_is_a_clear_error(tmp_path: Path):
    (tmp_path / "data" / "synthetic-test").mkdir(parents=True)
    with pytest.raises(ValueError, match="prepare more"):
        train.run(config(tmp_path / "data", tmp_path / "run", ["synthetic-test"], epochs=1))


def test_the_checkpoint_records_whether_clip_was_seen(root: Path, tmp_path: Path):
    train.run(config(root, tmp_path / "run", ["synthetic-test"], epochs=1))
    assert ckpt.read(tmp_path / "run" / "best.pt")["extra"]["clip_share"] == 0.0


def test_sources_all_is_every_prepared_source_and_ship_keeps_the_commercial_ones(root: Path, tmp_path: Path):
    for i in range(3):
        write_sample(root / "julian-catalog" / f"x{i}", 96, 64, 0.2)  # non-commercial in the repo registry
    (root / "wikimedia-cc0-pd").mkdir()  # in the registry but nothing prepared: not listed
    assert train.resolve_sources(root, ["all"], ship=False) == ["julian-catalog", "synthetic-test"]
    assert train.resolve_sources(root, ["all"], ship=True) == ["synthetic-test"]
    assert train.resolve_sources(root, ["synthetic-test"], ship=False) == ["synthetic-test"]


def test_init_starts_at_epoch_zero_from_the_checkpoints_weights(root: Path, tmp_path: Path):
    train.run(config(root, tmp_path / "first", ["synthetic-test"], epochs=1))
    init = tmp_path / "first" / "best.pt"
    cfg = config(root, tmp_path / "second", ["synthetic-test"], epochs=1)
    cfg["train"]["init"] = str(init)
    b = train._build(cfg)
    want = ckpt.read(init)["model"]
    assert all(torch.equal(v.cpu(), want[k]) for k, v in b.model.state_dict().items())
    assert train.run(cfg)["started_at"] == 0


def test_init_from_another_layout_is_a_clear_error(root: Path, tmp_path: Path):
    bad = tmp_path / "old.pt"
    ckpt.save(bad, RetouchNet(), None, 0, 1.0, {"layout": "something-else"})
    cfg = config(root, tmp_path / "run", ["synthetic-test"], epochs=1)
    cfg["train"]["init"] = str(bad)
    with pytest.raises(ValueError, match="layout"):
        train.run(cfg)


def test_val_every_skips_validation_but_always_scores_the_last_epoch(root: Path, tmp_path: Path):
    cfg = config(root, tmp_path / "run", ["synthetic-test"], epochs=5)
    cfg["train"]["val_every"] = 2
    cfg["loss"]["train_metric"] = "oklab"
    out = train.run(cfg)
    assert len(out["val"]) == 1 + 3  # baseline, epochs 2 and 4, the last (5)


def _val_des(run: Path) -> list[float]:
    import csv

    with open(run / "log.csv", encoding="utf-8", newline="") as f:
        return [float(r["val_de"]) for r in csv.DictReader(f) if r.get("val_de")]


def test_the_best_checkpoint_by_delta_e_is_kept(root: Path, tmp_path: Path):
    run = tmp_path / "run"
    train.run(config(root, run, ["synthetic-test"], epochs=3))
    state = ckpt.read(run / "best_de.pt")
    assert state["extra"]["val_de"] == pytest.approx(min(_val_des(run)), abs=1e-9)
    assert state.get("optimiser") is None  # model only, like best.pt
    assert ckpt.read(run / "last.pt")["extra"]["best_de"] == pytest.approx(min(_val_des(run)), abs=1e-9)


def test_a_last_checkpoint_from_before_best_de_still_resumes(root: Path, tmp_path: Path):
    """Review focus 3: r1's last.pt has no `best_de`; it counts as infinity."""
    run = tmp_path / "run"
    train.run(config(root, run, ["synthetic-test"], epochs=2))
    state = ckpt.read(run / "last.pt")
    state["extra"].pop("best_de")
    torch.save(state, run / "last.pt")
    (run / "best_de.pt").unlink()
    out = train.run(config(root, run, ["synthetic-test"], epochs=3))
    assert out["started_at"] == 2
    assert ckpt.read(run / "best_de.pt")["extra"]["val_de"] == pytest.approx(_val_des(run)[-1], abs=1e-9)


def test_sources_all_never_takes_unedited_raws(root: Path):
    """Review 2: a prepared source of kind `raw` would train flat RAW renders as finished photos."""
    for i in range(3):
        write_sample(root / "raw-pixls-cc0" / f"r{i}", 96, 64, 0.2)  # kind "raw" in the registry
    assert "raw-pixls-cc0" not in train.resolve_sources(root, ["all"], ship=False)
    assert train.resolve_sources(root, ["raw-pixls-cc0"], ship=False) == ["raw-pixls-cc0"], "named, it is a choice"


def test_a_run_on_all_resumes_after_a_new_source_is_prepared(root: Path, tmp_path: Path):
    """Review 6: `all` is worked out once, when the run starts; resuming keeps that list."""
    train.run(config(root, tmp_path / "run", ["all"], epochs=1))
    for i in range(3):
        write_sample(root / "julian-catalog" / f"x{i}", 96, 64, 0.2)
    train.run(config(root, tmp_path / "run", ["all"], epochs=2))
    assert ckpt.read(tmp_path / "run" / "last.pt")["extra"]["sources"] == ["synthetic-test"]
