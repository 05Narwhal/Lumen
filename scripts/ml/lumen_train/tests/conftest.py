"""Fixtures shared by the export and tier tests."""

from pathlib import Path

import pytest
import torch

from lumen_train.common import ckpt
from lumen_train.retouch import params
from lumen_train.retouch.model import RetouchNet
from lumen_train.tests.test_dataset import write_sample


@pytest.fixture
def run(tmp_path: Path) -> Path:
    """A Wikimedia-trained checkpoint whose validation photos are not on disk."""
    torch.manual_seed(0)
    path = tmp_path / "runs" / "r1" / "best.pt"
    path.parent.mkdir(parents=True)
    ckpt.save(path, RetouchNet(), None, 90, 0.74, {"sources": ["wikimedia-cc0-pd", "wikimedia-cc-by"], "layout": params.LAYOUT, "clip_share": 0.0})
    return path


@pytest.fixture
def scored_run(tmp_path: Path) -> tuple[Path, Path]:
    """(data root, checkpoint): a checkpoint trained on `synthetic-test`, whose photos are prepared."""
    root = tmp_path / "data"
    for i in range(30):
        write_sample(root / "synthetic-test" / f"p{i:02d}", 96 if i % 3 else 64, 64 if i % 3 else 96, 0.04 + 0.02 * i)
    torch.manual_seed(0)
    path = tmp_path / "runs" / "s1" / "best.pt"
    path.parent.mkdir(parents=True)
    ckpt.save(path, RetouchNet(), None, 3, 1.0, {"sources": ["synthetic-test"], "layout": params.LAYOUT, "clip_share": 0.0})
    return root, path
