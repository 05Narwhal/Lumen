"""The files `retouch_prep` writes (`photos-media::retouch_input`): `input.u16` is
4 × EDGE × EDGE little-endian, 0–65535 for 0–1 — R, G, B log-encoded, then 1 where the photo
is — and `meta.json` holds its size, aspect and features."""

import json
from pathlib import Path

import numpy as np
import torch

EDGE = 256


def encode(x: torch.Tensor) -> torch.Tensor:
    """Linear light → the model's range (`retouch_input::encode`)."""
    pos = x.clamp(min=0.0)
    e = (torch.log2(pos + 1.0 / 64.0) + 6.0) / 8.0
    return torch.where(x > 0, e, torch.zeros_like(e))


def decode(e: torch.Tensor) -> torch.Tensor:
    return torch.pow(2.0, e * 8.0 - 6.0) - 1.0 / 64.0


def encode_input(lin: torch.Tensor, w: int, h: int) -> torch.Tensor:
    """(3, h, w) linear → (4, EDGE, EDGE) as the model reads it, from the top-left corner."""
    if h > EDGE or w > EDGE:
        raise ValueError(f"{w}x{h} does not fit in {EDGE}")
    out = torch.zeros(4, EDGE, EDGE, dtype=lin.dtype)
    out[:3, :h, :w] = encode(lin[:, :h, :w])
    out[3, :h, :w] = 1.0
    return out


def decode_input(raw) -> tuple[torch.Tensor, torch.Tensor, int, int]:
    """`input.u16` → (linear (3, E, E), mask (1, E, E), width, height)."""
    a = np.asarray(raw, dtype=np.uint16).reshape(4, EDGE, EDGE).astype(np.float32) / 65535.0
    t = torch.from_numpy(a)
    mask = (t[3:4] > 0.5).to(t.dtype)
    w = int(mask[0, 0].sum().item())
    h = int(mask[0, :, 0].sum().item())
    lin = decode(t[:3]) * mask
    return lin, mask, w, h


def read_raw(folder: Path) -> np.ndarray:
    return np.fromfile(folder / "input.u16", dtype="<u2")


def read_meta(folder: Path) -> dict:
    return json.loads((folder / "meta.json").read_text(encoding="utf-8"))
