"""The Retouch network, trained from scratch (no third-party weights to license).

Input: the prepared photo (4 × 256 × 256: log-encoded RGB and the photo's mask) and its CLIP
embedding with a flag (zeros and 0 when the app has none). Output: one value in [0, 1] per
`params.PARAMS` slot. The photo's global statistics (a soft histogram, channel means and
spreads, clipping) are computed in the graph from the masked pixels, so the app passes nothing
else and synthetic pairs need no Rust round trip."""

import math

import torch
import torch.nn as nn
import torch.nn.functional as F

from . import params

CLIP_DIM = 512
HIST_BINS = 32
STATS = HIST_BINS + 3 + 3 + 2
# `prepared.encode(1.0)`: where white sits in the encoded range.
WHITE = (math.log2(1.0 + 1.0 / 64.0) + 6.0) / 8.0


def _norm(c: int) -> nn.GroupNorm:
    """Per-photo normalisation: unlike batch norm it does not depend on how much padding the
    other photos in a batch have, nor on the batch size."""
    return nn.GroupNorm(8, c)


class MBConv(nn.Module):
    """An inverted residual block (expand, depthwise, project)."""

    def __init__(self, cin: int, cout: int, stride: int, expand: int = 4):
        super().__init__()
        mid = cin * expand
        self.body = nn.Sequential(
            nn.Conv2d(cin, mid, 1, bias=False),
            _norm(mid),
            nn.SiLU(),
            nn.Conv2d(mid, mid, 3, stride=stride, padding=1, groups=mid, bias=False),
            _norm(mid),
            nn.SiLU(),
            nn.Conv2d(mid, cout, 1, bias=False),
            _norm(cout),
        )
        self.skip = stride == 1 and cin == cout

    def forward(self, x):
        y = self.body(x)
        return x + y if self.skip else y


def _stage(cin: int, cout: int, n: int) -> nn.Sequential:
    return nn.Sequential(MBConv(cin, cout, 2), *[MBConv(cout, cout, 1) for _ in range(n - 1)])


class RetouchNet(nn.Module):
    def __init__(self):
        super().__init__()
        n = len(params.PARAMS)
        self.stem = nn.Sequential(nn.Conv2d(4, 32, 3, stride=2, padding=1, bias=False), _norm(32), nn.SiLU())
        self.stages = nn.Sequential(_stage(32, 48, 2), _stage(48, 96, 3), _stage(96, 160, 3), _stage(160, 256, 2))
        # Where things are (crop, gradients): an 8 × 8 map, 16 channels.
        self.where = nn.Conv2d(256, 16, 1)
        self.clip = nn.Sequential(nn.Linear(CLIP_DIM + 1, 128), nn.SiLU())
        width = 256 + 16 * 64 + STATS + 128
        self.head = nn.Sequential(nn.Linear(width, 512), nn.SiLU(), nn.Dropout(0.1), nn.Linear(512, n))
        centres = torch.linspace(0.0, 1.0, HIST_BINS)
        self.register_buffer("centres", centres.view(1, HIST_BINS, 1, 1), persistent=False)
        # Start at neutral: a fresh model changes (almost) nothing.
        last = self.head[-1]
        nn.init.zeros_(last.weight)
        u = params.neutral_vector().clamp(0.02, 0.98)
        with torch.no_grad():
            last.bias.copy_(torch.log(u / (1 - u)))

    def statistics(self, x: torch.Tensor) -> torch.Tensor:
        """Global statistics of the photo's own pixels (the mask leaves out the padding)."""
        m = x[:, 3:4]
        enc = x[:, :3] * m
        total = m.sum(dim=(2, 3)).clamp(min=1.0)
        lum = 0.2126 * enc[:, 0:1] + 0.7152 * enc[:, 1:2] + 0.0722 * enc[:, 2:3]
        sigma = 1.0 / HIST_BINS
        w = torch.exp(-0.5 * ((lum - self.centres) / sigma) ** 2) * m
        hist = w.sum(dim=(2, 3))
        hist = hist / hist.sum(dim=1, keepdim=True).clamp(min=1e-6)
        mean = enc.sum(dim=(2, 3)) / total
        var = ((enc - mean[:, :, None, None]) ** 2 * m).sum(dim=(2, 3)) / total
        clipped = (torch.sigmoid((lum - WHITE) * 200.0) * m).sum(dim=(2, 3)) / total
        dark = (torch.sigmoid((0.1 - lum) * 200.0) * m).sum(dim=(2, 3)) / total
        return torch.cat([hist, mean, torch.sqrt(var + 1e-8), clipped, dark], dim=1)

    def forward(self, image: torch.Tensor, clip: torch.Tensor, clip_flag: torch.Tensor) -> torch.Tensor:
        m = image[:, 3:4]
        f = self.stages(self.stem(image))
        mm = F.adaptive_avg_pool2d(m, (int(f.shape[-2]), int(f.shape[-1])))  # ints: ONNX export needs a static size
        pooled = (f * mm).sum(dim=(2, 3)) / mm.sum(dim=(2, 3)).clamp(min=1e-6)
        where = self.where(F.adaptive_avg_pool2d(f, 8)).flatten(1)
        c = self.clip(torch.cat([clip * clip_flag, clip_flag], dim=1))
        z = torch.cat([pooled, where, self.statistics(image), c], dim=1)
        return torch.sigmoid(self.head(z))
