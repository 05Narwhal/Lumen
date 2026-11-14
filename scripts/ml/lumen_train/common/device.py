"""Where training runs, and repeatable randomness."""

import random

import numpy as np
import torch


def pick_device() -> torch.device:
    """The graphics card when there is one, else the CPU."""
    if torch.cuda.is_available():
        return torch.device("cuda")
    return torch.device("cpu")


def describe(device: torch.device) -> str:
    return torch.cuda.get_device_name(device) if device.type == "cuda" else "CPU"


def seed_everything(seed: int) -> None:
    random.seed(seed)
    np.random.seed(seed)
    torch.manual_seed(seed)
    torch.cuda.manual_seed_all(seed)
