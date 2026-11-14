"""Saving a trained model under Lumen's model names (Ray / Prism / Spectrum, low / high / ultra)
and installing one tier into the app's dev folder (Round 12-4 Task 1)."""

import hashlib
import json
from pathlib import Path

import numpy as np
import onnxruntime as ort
import pytest
import torch

from lumen_train.common import ckpt, naming
from lumen_train.retouch import export, params, tiers
from lumen_train.retouch.model import RetouchNet


def test_names():
    assert naming.model_id("ray", "1.0", "high") == "lumen-ray-1.0-high"
    assert naming.display_name("ray", "1.0", "ultra") == "Lumen Ray-ultra 1.0"
    assert naming.display_name("spectrum", "2.1") == "Lumen Spectrum 2.1"
    assert [naming.FAMILIES[f].title for f in ("ray", "prism", "spectrum")] == ["Lumen Ray", "Lumen Prism", "Lumen Spectrum"]
    assert naming.TIERS == ("low", "high", "ultra")
    with pytest.raises(ValueError):
        naming.model_id("index", "1.0", "high")  # it is Spectrum
    with pytest.raises(ValueError):
        naming.model_id("ray", "1.0", "medium")
    with pytest.raises(ValueError):
        naming.model_id("ray", "1.0/../x", "high")


def test_pre_release_versions_carry_a_stage():
    assert naming.model_id("ray", "1.0.1-d", "high") == "lumen-ray-1.0.1-d-high"
    assert naming.display_name("ray", "1.0.1-d", "high") == "Lumen Ray-high 1.0.1-d"
    assert [naming.is_release(v) for v in ("2.1", "1.0.1-d", "1.0.4-r")] == [True, False, False]
    for bad in ("1.0.1", "1.0-d", "1.0.1-x", "1", "1.0.1-dev"):
        with pytest.raises(ValueError):
            naming.model_id("ray", bad, "high")
    assert naming.FAMILIES["iris"].purpose == "faces" and naming.FAMILIES["facet"].title == "Lumen Facet"


def test_save_writes_every_tier_with_its_own_entry(run: Path, tmp_path: Path):
    entry = export.save(run, "ray", "1.0", ["low", "high", "ultra"], out=tmp_path / "models", root=tmp_path)
    folder = tmp_path / "models" / "lumen-ray-1.0"
    model = json.loads((folder / "model.json").read_text(encoding="utf-8"))
    assert model["name"] == "Lumen Ray 1.0" and model["family"] == "ray" and model["version"] == "1.0" and model == entry
    assert set(model["tiers"]) == {"low", "high", "ultra"}
    for tier, e in model["tiers"].items():
        f = folder / e["file"]
        assert e["id"] == f"lumen-ray-1.0-{tier}" and e["name"] == f"Lumen Ray-{tier} 1.0" and e["role"] == "retouch"
        assert e["bytes"] == f.stat().st_size and e["sha256"] == hashlib.sha256(f.read_bytes()).hexdigest()
        assert e["retouch"] == {"size": 256, "layout": params.LAYOUT, "inputs": ["image", "clip", "clip_flag"], "output": "v"}
        assert e["trained"]["run"] == "r1" and e["trained"]["sources"] == ["wikimedia-cc0-pd", "wikimedia-cc-by"]
        assert "/" not in e["file"] and "\\" not in e["file"]
        assert (folder / e["trained"]["checkpoint"]).exists()  # kept to warm-start the next version


def test_the_onnx_gives_what_pytorch_gives_with_and_without_clip(run: Path, tmp_path: Path):
    export.save(run, "ray", "1.0", ["high"], out=tmp_path / "models", root=tmp_path)
    net = RetouchNet()
    ckpt.load(run, net)
    net.eval()
    session = ort.InferenceSession(str(tmp_path / "models" / "lumen-ray-1.0" / "lumen-ray-1.0-high.onnx"), providers=["CPUExecutionProvider"])
    image = torch.rand(2, 4, 256, 256)
    for clip, flag in ((torch.randn(2, 512), torch.ones(2, 1)), (torch.zeros(2, 512), torch.zeros(2, 1))):
        want = net(image, clip, flag).detach().numpy()
        (got,) = session.run(["v"], {"image": image.numpy(), "clip": clip.numpy(), "clip_flag": flag.numpy()})
        assert np.abs(got - want).max() < 1e-4


def test_saving_one_tier_later_keeps_the_others(run: Path, tmp_path: Path):
    export.save(run, "ray", "1.0", ["low", "high"], out=tmp_path / "models", root=tmp_path)
    model = export.save(run, "ray", "1.0", ["ultra"], out=tmp_path / "models", root=tmp_path)
    assert set(model["tiers"]) == {"low", "high", "ultra"}


def test_tiers_are_real_and_scored(scored_run, tmp_path: Path):
    root, path = scored_run
    model = export.save(path, "ray", "1.0.1-d", ["low", "high", "ultra"], out=tmp_path / "models", root=tmp_path, dev_only=True, data_root=root, calib=8)
    low, high, ultra = (model["tiers"][t] for t in ("low", "high", "ultra"))
    assert (high["precision"], high["tta"], high["file"]) == ("fp32", 1, "lumen-ray-1.0.1-d-high.onnx")
    assert (low["precision"], low["tta"], low["file"]) == ("int8", 1, "lumen-ray-1.0.1-d-low.onnx")
    assert low["sha256"] != high["sha256"] and low["bytes"] < high["bytes"]
    assert (ultra["file"], ultra["sha256"], ultra["precision"], ultra["tta"]) == (high["file"], high["sha256"], "fp32", 5)
    assert all(isinstance(e["val_de"], float) for e in (low, high, ultra))
    assert model["low_vs_high_de"] == pytest.approx(low["val_de"] - high["val_de"], abs=2e-3)
    assert sorted(p.name for p in (tmp_path / "models" / "lumen-ray-1.0.1-d").glob("*.onnx")) == [high["file"], low["file"]]


def test_low_is_fp16_when_int8_costs_too_much(scored_run, tmp_path: Path, monkeypatch):
    root, path = scored_run
    scores = iter([1.0, 1.0 + tiers.LOW_BOUND + 0.01, 1.02, 0.95])  # high, int8, fp16, ultra
    monkeypatch.setattr(tiers, "mean_de", lambda predict, items: next(scores))
    model = export.save(path, "ray", "1.0.1-d", ["low", "high", "ultra"], out=tmp_path / "models", root=tmp_path, dev_only=True, data_root=root, calib=4)
    assert model["tiers"]["low"]["precision"] == "fp16" and model["tiers"]["low"]["val_de"] == 1.02
    assert model["low_vs_high_de"] == pytest.approx(0.02)


def test_ultra_alone_writes_highs_file_and_drops_a_stale_ultra_file(run: Path, tmp_path: Path):
    folder = tmp_path / "models" / "lumen-ray-1.0"
    folder.mkdir(parents=True)
    (folder / "lumen-ray-1.0-ultra.onnx").write_bytes(b"old copy")
    model = export.save(run, "ray", "1.0", ["ultra"], out=tmp_path / "models", root=tmp_path)
    assert model["tiers"]["ultra"]["file"] == "lumen-ray-1.0-high.onnx" and (folder / "lumen-ray-1.0-high.onnx").exists()
    assert not (folder / "lumen-ray-1.0-ultra.onnx").exists()


def test_without_validation_photos_val_de_is_absent(run: Path, tmp_path: Path):
    model = export.save(run, "ray", "1.0", ["low", "high", "ultra"], out=tmp_path / "models", root=tmp_path, calib=4)
    for e in model["tiers"].values():
        assert "val_de" not in e and e["precision"] in ("int8", "fp32")
    assert "low_vs_high_de" not in model


def test_install_dev_copies_every_tier_into_its_family_folder(run: Path, tmp_path: Path, monkeypatch):
    dev = tmp_path / "dev"
    monkeypatch.setenv("LUMEN_DEV_MODELS", str(dev))
    other = dev / "ray" / "lumen-ray-0.9.1-d"
    other.mkdir(parents=True)
    (other / "model.json").write_text("{}", encoding="utf-8")
    export.save(run, "ray", "1.0", ["low", "high", "ultra"], out=tmp_path / "models", root=tmp_path, calib=4)
    for _ in range(2):  # installing again replaces the files
        model = export.install_dev(tmp_path / "models" / "lumen-ray-1.0")
    there = dev / "ray" / "lumen-ray-1.0"
    assert json.loads((there / "model.json").read_text(encoding="utf-8")) == model
    assert sorted(p.name for p in there.iterdir()) == ["lumen-ray-1.0-high.onnx", "lumen-ray-1.0-low.onnx", "model.json"]
    for e in model["tiers"].values():
        assert hashlib.sha256((there / e["file"]).read_bytes()).hexdigest() == e["sha256"]
    assert not list(dev.rglob("*.part")) and (other / "model.json").read_text(encoding="utf-8") == "{}"
    assert not (dev / "retouch.json").exists()


def test_install_dev_with_a_tier_also_writes_the_legacy_entry(run: Path, tmp_path: Path, monkeypatch):
    dev = tmp_path / "dev"
    monkeypatch.setenv("LUMEN_DEV_MODELS", str(dev))
    export.save(run, "ray", "1.0", ["low", "high", "ultra"], out=tmp_path / "models", root=tmp_path, calib=4)
    model = export.install_dev(tmp_path / "models" / "lumen-ray-1.0", "high")
    entry = json.loads((dev / "retouch.json").read_text(encoding="utf-8"))
    assert entry == model["tiers"]["high"] and entry["id"] == "lumen-ray-1.0-high"
    assert hashlib.sha256((dev / entry["file"]).read_bytes()).hexdigest() == entry["sha256"]
    assert (dev / "ray" / "lumen-ray-1.0" / "model.json").exists()
    assert not list(dev.rglob("*.part"))


def test_a_non_commercial_source_is_dev_only(run: Path, tmp_path: Path):
    (tmp_path / "sources.toml").write_text('[sources.mine]\nurl = ""\nlicence = "NC"\ncommercial = false\nkind = "finals"\noriginals = "x"\n', encoding="utf-8")
    state = ckpt.read(run)
    ckpt.save(run, _net(state), None, 1, 1.0, {**state["extra"], "sources": ["mine"]})
    with pytest.raises(export.LicenceRefused):
        export.save(run, "ray", "1.0", ["high"], out=tmp_path / "models", root=tmp_path)
    e = export.save(run, "ray", "1.0", ["high"], out=tmp_path / "models", root=tmp_path, dev_only=True)
    assert "not for shipping" in e["tiers"]["high"]["licence"]


def _net(state: dict) -> RetouchNet:
    net = RetouchNet()
    net.load_state_dict(state["model"])
    return net


def test_a_checkpoint_without_its_sources_is_dev_only(tmp_path: Path):
    """Nothing recorded is nothing cleared: such a model never ships."""
    with pytest.raises(export.LicenceRefused):
        export._licence([], tmp_path, dev_only=False)
    assert export._licence([], tmp_path, dev_only=True).startswith("not for shipping")
