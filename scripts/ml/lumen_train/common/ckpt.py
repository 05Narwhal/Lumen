"""Checkpoints: the model, its optimiser, how far training got, and what it trained on."""

from pathlib import Path

import torch


def save(path: Path | str, model: torch.nn.Module, optimiser: torch.optim.Optimizer | None, epoch: int, best: float, extra: dict | None = None) -> None:
    path = Path(path)
    path.parent.mkdir(parents=True, exist_ok=True)
    tmp = path.with_suffix(path.suffix + ".tmp")
    torch.save(
        {
            "model": model.state_dict(),
            "optimiser": optimiser.state_dict() if optimiser is not None else None,
            "epoch": epoch,
            "best": best,
            "extra": extra or {},
        },
        tmp,
    )
    tmp.replace(path)


def read(path: Path | str, map_location: str | torch.device = "cpu") -> dict:
    """A checkpoint's contents, without a model. Tensors and plain values only: loading runs
    no code from the file."""
    return torch.load(path, map_location=map_location, weights_only=True)


def load(path: Path | str, model: torch.nn.Module, optimiser: torch.optim.Optimizer | None = None, map_location: str | torch.device = "cpu") -> dict:
    state = read(path, map_location)
    model.load_state_dict(state["model"])
    if optimiser is not None and state.get("optimiser") is not None:
        optimiser.load_state_dict(state["optimiser"])
    return state
