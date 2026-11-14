"""The training kit's shared parts: config, licence gate, checkpoints, ONNX export."""

import json
from pathlib import Path

import pytest
import torch

from lumen_train.common import ckpt, config, licence, logs, manifest, onnx_export
from lumen_train.common.device import pick_device, seed_everything


def test_config_reads_toml_and_overrides_win(tmp_path: Path):
    p = tmp_path / "c.toml"
    p.write_text('[train]\nlr = 0.001\nbatch = 64\nname = "a"\n[loss]\nde = 1.0\n', encoding="utf-8")
    c = config.load(p, ["train.lr=3e-4", "loss.de=2", "train.name=b", "new.flag=true"])
    assert c["train"]["lr"] == pytest.approx(3e-4)
    assert c["train"]["batch"] == 64
    assert c["loss"]["de"] == 2
    assert c["train"]["name"] == "b"
    assert c["new"]["flag"] is True


def test_config_rejects_a_malformed_override(tmp_path: Path):
    p = tmp_path / "c.toml"
    p.write_text("", encoding="utf-8")
    with pytest.raises(ValueError):
        config.load(p, ["no-equals-sign"])


def registry(tmp_path: Path) -> Path:
    p = tmp_path / "registry.toml"
    p.write_text(
        '[sources.free]\nurl = "x"\nlicence = "CC0-1.0"\ncommercial = true\nkind = "finals"\nattribution = false\nchecked = "2026-10-06"\n'
        '[sources.nc]\nurl = "y"\nlicence = "CC-BY-NC-SA-4.0"\ncommercial = false\nkind = "pairs"\nattribution = true\nchecked = "2026-10-06"\n',
        encoding="utf-8",
    )
    return p


def test_the_licence_gate_lets_non_commercial_sources_ship_only_as_nc_weights(tmp_path: Path):
    reg = licence.load_registry(registry(tmp_path))
    licence.check(["free", "nc"], ship=True, registry=reg, terms="nc")
    licence.check(["free"], ship=True, registry=reg)
    with pytest.raises(licence.LicenceError, match="nc"):
        licence.check(["free", "nc"], ship=True, registry=reg, terms="open")
    assert licence.weights_licence("nc") == "CC-BY-NC-4.0"
    assert licence.weights_licence("open") == "MIT"


def test_the_licence_gate_refuses_a_non_commercial_source_when_shipping(tmp_path: Path):
    reg = licence.load_registry(registry(tmp_path))
    licence.check(["free"], ship=True, registry=reg)
    licence.check(["free", "nc"], ship=False, registry=reg)
    with pytest.raises(licence.LicenceError, match="nc"):
        licence.check(["free", "nc"], ship=True, registry=reg)


def test_an_unregistered_source_is_always_refused(tmp_path: Path):
    reg = licence.load_registry(registry(tmp_path))
    with pytest.raises(licence.LicenceError, match="unknown"):
        licence.check(["unknown"], ship=False, registry=reg)


def test_the_shipped_registry_loads():
    reg = licence.load_registry(licence.REGISTRY)
    assert reg["playraw-edits"]["commercial"] is False
    assert reg["wikimedia-cc0-pd"]["commercial"] is True


def test_seeding_repeats_random_numbers():
    seed_everything(7)
    a = torch.rand(3)
    seed_everything(7)
    assert torch.equal(a, torch.rand(3))
    assert pick_device().type in ("cpu", "cuda")


def test_a_checkpoint_resumes_model_optimiser_and_progress(tmp_path: Path):
    m = torch.nn.Linear(3, 2)
    opt = torch.optim.AdamW(m.parameters(), lr=0.1)
    m(torch.ones(1, 3)).sum().backward()
    opt.step()
    ckpt.save(tmp_path / "c.pt", m, opt, epoch=4, best=0.25, extra={"sources": ["free"]})
    m2 = torch.nn.Linear(3, 2)
    opt2 = torch.optim.AdamW(m2.parameters(), lr=0.1)
    state = ckpt.load(tmp_path / "c.pt", m2, opt2)
    assert state["epoch"] == 4 and state["best"] == 0.25 and state["extra"]["sources"] == ["free"]
    assert torch.equal(m.weight, m2.weight)
    assert opt2.state_dict()["state"], "optimiser moments came back"


def test_the_logger_writes_csv_rows(tmp_path: Path):
    log = logs.Logger(tmp_path, tensorboard=False)
    log.log(1, loss=0.5, de=3.0)
    log.log(2, loss=0.4)
    log.close()
    lines = (tmp_path / "log.csv").read_text(encoding="utf-8").splitlines()
    assert lines[0] == "step,de,loss"
    assert lines[2] == "2,,0.4"


class Toy(torch.nn.Module):
    def __init__(self):
        super().__init__()
        self.a = torch.nn.Conv2d(4, 8, 3, padding=1)
        self.b = torch.nn.Linear(8 + 5, 6)

    def forward(self, image, features):
        x = self.a(image).mean(dim=(2, 3))
        return torch.sigmoid(self.b(torch.cat([x, features], dim=1)))


def test_onnx_export_matches_pytorch_and_reports_its_hash(tmp_path: Path):
    m = Toy().eval()
    example = (torch.rand(2, 4, 32, 32), torch.rand(2, 5))
    out = onnx_export.export(m, example, tmp_path / "toy.onnx", ["image", "features"], ["params"])
    assert out["max_abs_diff"] < 1e-4
    assert len(out["sha256"]) == 64 and out["bytes"] == (tmp_path / "toy.onnx").stat().st_size
    e = manifest.entry("toy@1", "retouch", "toy.onnx", out, "Lumen's own", retouch={"size": 32})
    assert e["id"] == "toy@1" and e["sha256"] == out["sha256"] and e["retouch"] == {"size": 32}
    json.dumps(e)


def test_a_resumed_log_carries_on(tmp_path: Path):
    log = logs.Logger(tmp_path, tensorboard=False)
    log.log(1, loss=0.5)
    log.close()
    log = logs.Logger(tmp_path, tensorboard=False)
    log.log(2, loss=0.4, de=3.0)
    log.close()
    lines = (tmp_path / "log.csv").read_text(encoding="utf-8").splitlines()
    assert lines == ["step,de,loss", "1,,0.5", "2,3.0,0.4"]


def test_export_leaves_the_callers_model_as_it_was(tmp_path: Path):
    m = Toy().train()
    onnx_export.export(m, (torch.rand(1, 4, 8, 8), torch.rand(1, 5)), tmp_path / "t.onnx", ["image", "features"], ["params"])
    assert m.training
