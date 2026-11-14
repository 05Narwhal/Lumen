"""Read-only: each checkpoint's exposure answer on Wikimedia test photos damaged by a known
exposure e. A perfect model answers -e; a healthy run's offset at damage 0 is within +-0.05
(ML-TRAINING.md, "Why r1 is dark"). Also: does the answer on the untouched photo follow its
clipping or brightness?

    python -m lumen_train.retouch.probe_exposure [--data C:/lumen-train] RUN/best_de.pt [...]
"""
import sys
from pathlib import Path
import torch
from lumen_train.common import ckpt
from lumen_train.retouch import params, render
from lumen_train.retouch.model import RetouchNet
from lumen_train.retouch.dataset import PreparedDataset, CLIP_DIM
from lumen_train.retouch.prepared import encode_input, decode_input, read_raw

args = sys.argv[1:]
ROOT = Path("C:/lumen-train")
if args[:1] == ["--data"]:
    ROOT, args = Path(args[1]), args[2:]
EVS = [-1.0, -0.5, 0.0, 0.5, 1.0]
ds = PreparedDataset(ROOT, ["wikimedia-cc-by", "wikimedia-cc0-pd"], split="test", augment=False)
photos = []
for f in ds.samples[::2][:60]:
    lin, mask, w, h = decode_input(read_raw(f))
    photos.append((lin[:, :h, :w].unsqueeze(0), w, h))
print("photos", len(photos))
none, flag = torch.zeros(1, CLIP_DIM), torch.zeros(1, 1)


def corr(a, b):
    a, b = torch.tensor(a), torch.tensor(b)
    a, b = a - a.mean(), b - b.mean()
    return float((a * b).sum() / (a.norm() * b.norm()).clamp(min=1e-9))


@torch.no_grad()
def run(path):
    m = RetouchNet()
    st = ckpt.load(Path(path), m)
    m.eval()
    print(f"\n=== {path} (val photos {len(st.get('extra', {}).get('val', []) or [])})")
    clip0, ev0 = [], []
    for e in EVS:
        ans = []
        for img, w, h in photos:
            x = img if e == 0 else render.render(img, render.from_develop_json({"basic": {"exposure": e}}), torch.tensor([[w, h]], dtype=img.dtype))
            v = m(encode_input(x[0], w, h).unsqueeze(0), none, flag)
            a = float(params.value(v, "exposure")[0])
            ans.append(a)
            if e == 0:
                ev0.append(a)
                clip0.append(float((img.amax(dim=1) >= 0.95).float().mean()))
        t = torch.tensor(ans)
        print(f"damage {e:+.1f}: answer mean {t.mean():+.3f} (ideal {-e:+.1f}, offset {t.mean() + e:+.3f}) sd {t.std():.3f}")
    print(f"identity answer vs clipped share: corr {corr(ev0, clip0):+.2f}; clipped share mean {sum(clip0) / len(clip0):.3f}")
    lum = [float(torch.log2((0.2126 * i[0, 0] + 0.7152 * i[0, 1] + 0.0722 * i[0, 2]).clamp(min=1e-4)).mean()) for i, _, _ in photos]
    print(f"identity answer vs mean log2 luma: corr {corr(ev0, lum):+.2f}")


for p in args:
    run(p)
