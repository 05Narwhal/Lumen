"""Round 4C, Task 1: measure the pet models on public data (never on Julian's photos).

Data lives outside the repo (`PETS_EVAL`, the scratchpad), never shipped, never trained on:
COCO val2017 (annotations CC BY 4.0), DogFaceNet_large (CC BY 4.0), Cat Individual Images
(CC BY 4.0), Oxford-IIIT Pet (CC BY-SA 4.0).

    PETS_EVAL=<dir> python scripts/ml/eval_pets.py detect      # D-FINE-S on COCO cats, dogs, look-alikes
    PETS_EVAL=<dir> python scripts/ml/eval_pets.py identity    # CLIP vs DINOv2 on individual animals
    PETS_EVAL=<dir> python scripts/ml/eval_pets.py species     # CLIP overruling D-FINE's cat/dog

`detect` writes `detections.json` (every cat/dog box over 0.1, per image) for the thresholds;
`identity` crops each animal with the detector and compares embedders by same-vs-other cosines.
"""

import json
import os
import sys
import time
import types
from pathlib import Path

import numpy as np
import torch
from PIL import Image

EVAL = Path(os.environ["PETS_EVAL"])
DFINE = EVAL / "D-FINE"
COCO = EVAL / "coco"
CAT, DOG = 17, 18  # COCO category ids (the annotations)
# D-FINE's post-processor numbers classes contiguously (remap_mscoco_category is off): cat 15, dog 16.
OUT_CAT, OUT_DOG = 15, 16
torch.set_num_threads(max(1, (os.cpu_count() or 4) // 2))


def load_dfine():
    """D-FINE-S, COCO-only weights, in deploy mode with its post-processor (labels, boxes, scores)."""
    sys.path.insert(0, str(DFINE))
    from src.core import YAMLConfig

    cfg = YAMLConfig(str(DFINE / "configs/dfine/dfine_hgnetv2_s_coco.yml"))
    cfg.yaml_cfg["HGNetv2"]["pretrained"] = False
    state = torch.load(EVAL / "dfine_s_coco.pth", map_location="cpu")
    cfg.model.load_state_dict(state["ema"]["module"] if "ema" in state else state["model"])

    class Model(torch.nn.Module):
        def __init__(self):
            super().__init__()
            self.model = cfg.model.deploy()
            self.post = cfg.postprocessor.deploy()

        def forward(self, images, sizes):
            return self.post(self.model(images), sizes)

    return Model().eval()


def to_input(img: Image.Image) -> torch.Tensor:
    """D-FINE's eval transform: a plain resize to 640×640 (no letterbox), 0..1, no mean/std."""
    x = np.asarray(img.convert("RGB").resize((640, 640), Image.BILINEAR), dtype=np.float32) / 255.0
    return torch.from_numpy(x).permute(2, 0, 1)[None]


@torch.no_grad()
def detect_pets(model, img: Image.Image, floor=0.1):
    w, h = img.size
    labels, boxes, scores = model(to_input(img), torch.tensor([[w, h]]))
    out = []
    for l, b, s in zip(labels[0].tolist(), boxes[0].tolist(), scores[0].tolist()):
        if l in (OUT_CAT, OUT_DOG) and s >= floor:
            out.append({"species": "cat" if l == OUT_CAT else "dog", "box": b, "score": s})
    return out


def iou(a, b):
    ix = max(0, min(a[2], b[2]) - max(a[0], b[0]))
    iy = max(0, min(a[3], b[3]) - max(a[1], b[1]))
    inter = ix * iy
    union = (a[2] - a[0]) * (a[3] - a[1]) + (b[2] - b[0]) * (b[3] - b[1]) - inter
    return inter / union if union > 0 else 0.0


def detect():
    model = load_dfine()
    subset = json.loads((COCO / "subset.json").read_text())
    ann = json.loads((COCO / "instances_val2017.json").read_text())
    gt = {}
    for a in ann["annotations"]:
        if a["category_id"] in (CAT, DOG) and not a.get("iscrowd"):
            x, y, w, h = a["bbox"]
            # Tiny animals (under 48 px a side) are below what the scan keeps.
            gt.setdefault(a["image_id"], []).append({"species": "cat" if a["category_id"] == CAT else "dog", "box": [x, y, x + w, y + h], "small": min(w, h) < 48})
    results, took = {}, []
    ids = sorted(set(subset["pets"] + subset["others"] + subset["none"]))
    for n, i in enumerate(ids):
        img = Image.open(COCO / "img" / f"{i:012d}.jpg")
        t = time.perf_counter()
        results[i] = detect_pets(model, img)
        took.append(time.perf_counter() - t)
        if n % 100 == 0:
            print(f"{n}/{len(ids)}", flush=True)
    (EVAL / "detections.json").write_text(json.dumps({"results": results, "gt": gt, "subset": subset}))
    print(f"median {np.median(took) * 1000:.0f} ms per photo on the CPU ({torch.get_num_threads()} threads)")
    report_detect(results, gt, subset)


def report_detect(results, gt, subset):
    results = {int(k): v for k, v in results.items()}
    gt = {int(k): v for k, v in gt.items()}
    print("\nthreshold | cat recall | dog recall | precision (pets) | photos of other animals with a pet | photos with no animal with a pet")
    for t in (0.3, 0.4, 0.5, 0.6, 0.7):
        found = {"cat": [0, 0], "dog": [0, 0]}
        tp = fp = 0
        for i in subset["pets"]:
            dets = [d for d in results[i] if d["score"] >= t]
            truths = [g for g in gt.get(i, []) if not g["small"]]
            used = set()
            for g in truths:
                found[g["species"]][1] += 1
                hit = next((k for k, d in enumerate(dets) if k not in used and d["species"] == g["species"] and iou(d["box"], g["box"]) >= 0.5), None)
                if hit is not None:
                    used.add(hit)
                    found[g["species"]][0] += 1
            # A box on a small labelled animal is not a false positive.
            for k, d in enumerate(dets):
                if k in used:
                    tp += 1
                elif not any(iou(d["box"], g["box"]) >= 0.5 for g in gt.get(i, [])):
                    fp += 1
        other = sum(1 for i in subset["others"] if any(d["score"] >= t for d in results[i]))
        none_ = sum(1 for i in subset["none"] if any(d["score"] >= t for d in results[i]))
        r = lambda s: found[s][0] / max(1, found[s][1])
        print(f"{t:.1f} | {r('cat'):.3f} ({found['cat'][1]}) | {r('dog'):.3f} ({found['dog'][1]}) | {tp / max(1, tp + fp):.3f} | {other}/{len(subset['others'])} | {none_}/{len(subset['none'])}")


def crop_square(img: Image.Image, box, margin=0.15) -> Image.Image:
    """The scan's crop: a square around the box, `margin` wider on each side, inside the photo."""
    w, h = img.size
    x0, y0, x1, y1 = box
    side = max(x1 - x0, y1 - y0) * (1 + 2 * margin)
    side = min(side, w, h)
    cx, cy = (x0 + x1) / 2, (y0 + y1) / 2
    left = min(max(cx - side / 2, 0), w - side)
    top = min(max(cy - side / 2, 0), h - side)
    return img.crop((round(left), round(top), round(left + side), round(top + side)))


def load_embedders():
    """name -> (model, preprocess to a 224 tensor); each returns an L2-normalised vector."""
    import open_clip

    models_dir = Path(os.environ.get("LUMEN_ML_BUILD", "build/models"))
    clip, _, _ = open_clip.create_model_and_transforms("ViT-B-32", pretrained=str(models_dir / "ViT-B-32-openai.pt"))
    clip.eval()
    dino = torch.hub.load(str(EVAL / "dinov2"), "dinov2_vits14", source="local", pretrained=False)
    dino.load_state_dict(torch.load(EVAL / "dinov2_vits14_pretrain.pth", map_location="cpu"))
    dino.eval()
    clip_mean, clip_std = np.array([0.48145466, 0.4578275, 0.40821073]), np.array([0.26862954, 0.26130258, 0.27577711])
    imnet_mean, imnet_std = np.array([0.485, 0.456, 0.406]), np.array([0.229, 0.224, 0.225])

    def prep(crop, mean, std):
        x = (np.asarray(crop.convert("RGB").resize((224, 224), Image.BICUBIC), dtype=np.float32) / 255.0 - mean) / std
        return torch.from_numpy(x.astype(np.float32)).permute(2, 0, 1)[None]

    @torch.no_grad()
    def clip_embed(crop):
        return clip.encode_image(prep(crop, clip_mean, clip_std))[0].numpy()

    @torch.no_grad()
    def dino_cls(crop):
        return dino(prep(crop, imnet_mean, imnet_std))[0].numpy()

    @torch.no_grad()
    def dino_cls_mean(crop):
        f = dino.forward_features(prep(crop, imnet_mean, imnet_std))
        return np.concatenate([f["x_norm_clstoken"][0].numpy(), f["x_norm_patchtokens"][0].mean(0).numpy()])

    unit = lambda f: (lambda c: (lambda v: v / np.linalg.norm(v))(f(c)))
    return {"clip": unit(clip_embed), "dinov2": unit(dino_cls), "dinov2+patches": unit(dino_cls_mean)}


def identity():
    """Individual animals: `identity <folder of one-folder-per-animal> <cat|dog> [max animals]`."""
    root, species = Path(sys.argv[2]), sys.argv[3]
    limit = int(sys.argv[4]) if len(sys.argv) > 4 else 500
    rng = np.random.default_rng(7)
    animals = sorted(d for d in root.iterdir() if d.is_dir() and len(list(d.iterdir())) >= 3)
    animals = [animals[i] for i in sorted(rng.choice(len(animals), min(limit, len(animals)), replace=False))]
    det, emb = load_dfine(), load_embedders()
    vecs = {k: [] for k in emb}
    who, found, tried = [], 0, 0
    for n, a in enumerate(animals):
        for f in sorted(a.iterdir())[:6]:
            tried += 1
            try:
                img = Image.open(f).convert("RGB")
            except Exception:
                continue
            boxes = [d for d in detect_pets(det, img, 0.5) if d["species"] == species]
            if not boxes:
                continue
            found += 1
            crop = crop_square(img, max(boxes, key=lambda d: d["score"])["box"])
            for k, e in emb.items():
                vecs[k].append(e(crop))
            who.append(n)
        if n % 50 == 0:
            print(f"{n}/{len(animals)} animals, {found}/{tried} photos with a {species}", flush=True)
    who = np.array(who)
    out = EVAL / f"identity_{species}.npz"
    np.savez(out, who=who, **{k: np.array(v) for k, v in vecs.items()})
    print(f"\n{len(set(who))} {species}s, {len(who)} crops ({found}/{tried} photos had a {species} at 0.5); saved {out.name}")
    report_identity(out)


def report_identity(path):
    data = np.load(path)
    who = data["who"]
    same = who[:, None] == who[None, :]
    upper = np.triu(np.ones_like(same, dtype=bool), 1)
    print("embedder | AUC | overlap | TAR@FAR1% (cos) | TAR@FAR0.1% (cos) | nearest is the same animal")
    for k in [k for k in data.files if k != "who"]:
        v = data[k]
        cos = v @ v.T
        g, i = cos[same & upper], cos[~same & upper]
        # AUC: the chance a same-animal pair is closer than a different-animal pair.
        order = np.argsort(np.concatenate([g, i]))
        ranks = np.empty(len(order)); ranks[order] = np.arange(len(order))
        auc = (ranks[: len(g)].sum() - len(g) * (len(g) - 1) / 2) / (len(g) * len(i))
        bins = np.linspace(-1, 1, 201)
        hg, hi = np.histogram(g, bins, density=True)[0], np.histogram(i, bins, density=True)[0]
        overlap = np.minimum(hg, hi).sum() * (bins[1] - bins[0])
        t1, t01 = np.quantile(i, 0.99), np.quantile(i, 0.999)
        np.fill_diagonal(cos, -9)
        nn_same = (who[cos.argmax(1)] == who).mean()
        print(f"{k} | {auc:.3f} | {overlap:.1%} | {(g >= t1).mean():.1%} ({t1:.3f}) | {(g >= t01).mean():.1%} ({t01:.3f}) | {nn_same:.1%}")


def filter_check():
    """The crop check (`pet:*` prototypes, Round 4B's rule) on COCO: how many real cats and dogs
    it would reject, and how many pets reported on photos of other animals and toys it catches."""
    import open_clip

    models_dir = Path(os.environ.get("LUMEN_ML_BUILD", "build/models"))
    clip, _, _ = open_clip.create_model_and_transforms("ViT-B-32", pretrained=str(models_dir / "ViT-B-32-openai.pt"))
    clip.eval()
    protos = json.loads((Path(__file__).resolve().parents[2] / "src-tauri/crates/photos-ml/prototypes.json").read_text())["prototypes"]
    kinds = ["real", "drawing", "toy", "screen", "print"]
    P = np.array([protos[f"pet:{k}"] for k in kinds], dtype=np.float32)
    mean, std = np.array([0.48145466, 0.4578275, 0.40821073]), np.array([0.26862954, 0.26130258, 0.27577711])
    data = json.loads((EVAL / "detections.json").read_text())
    results = {int(k): v for k, v in data["results"].items()}
    gt = {int(k): v for k, v in data["gt"].items()}
    subset = data["subset"]
    real, false = [], []
    for group, ids in (("pets", subset["pets"]), ("others", subset["others"]), ("none", subset["none"])):
        for i in ids:
            dets = [d for d in results[i] if d["score"] >= 0.5]
            if not dets:
                continue
            img = Image.open(COCO / "img" / f"{i:012d}.jpg").convert("RGB")
            for d in dets:
                crop = crop_square(img, d["box"])
                x = ((np.asarray(crop.resize((224, 224), Image.BICUBIC), dtype=np.float32) / 255.0 - mean) / std).astype(np.float32)
                with torch.no_grad():
                    e = clip.encode_image(torch.from_numpy(x).permute(2, 0, 1)[None])[0].numpy()
                e /= np.linalg.norm(e)
                logits = P @ e / 0.01
                share = np.exp(logits - logits.max())
                share /= share.sum()
                is_real = group == "pets" and any(iou(d["box"], g["box"]) >= 0.5 and g["species"] == d["species"] for g in gt.get(i, []))
                (real if is_real else false).append(share)
    for reasons in (["drawing", "toy", "screen", "print"], ["drawing", "toy", "print"], ["drawing", "toy"]):
        idx = [kinds.index(k) for k in reasons]
        for margin in (0.1, 0.2, 0.3):
            rej = lambda s: max(s[j] for j in idx) - s[0] > margin
            print(f"{'+'.join(reasons)} margin {margin}: rejects {sum(map(rej, real))}/{len(real)} real cats and dogs, catches {sum(map(rej, false))}/{len(false)} false pets")
    from collections import Counter
    top = lambda rows: Counter(kinds[int(np.argmax(s))] for s in rows)
    print("winning kind on real pets:", dict(top(real)), "| on false pets:", dict(top(false)))
    print("mean share on real pets:", {k: round(float(np.mean([s[j] for s in real])), 3) for j, k in enumerate(kinds)})


def species_check():
    """Which animal (`pet_filter::species`): how often D-FINE's cat/dog is right on COCO's
    detected animals, and what overruling it with CLIP's `species:*` softmax does at each
    confidence. Detections are deduplicated as the app does (one reading per box)."""
    import open_clip

    models_dir = Path(os.environ.get("LUMEN_ML_BUILD", "build/models"))
    clip, _, _ = open_clip.create_model_and_transforms("ViT-B-32", pretrained=str(models_dir / "ViT-B-32-openai.pt"))
    clip.eval()
    protos = json.loads((Path(__file__).resolve().parents[2] / "src-tauri/crates/photos-ml/prototypes.json").read_text())["prototypes"]
    P = np.array([protos["species:cat"], protos["species:dog"]], dtype=np.float32)
    mean, std = np.array([0.48145466, 0.4578275, 0.40821073]), np.array([0.26862954, 0.26130258, 0.27577711])
    data = json.loads((EVAL / "detections.json").read_text())
    results = {int(k): v for k, v in data["results"].items()}
    gt = {int(k): v for k, v in data["gt"].items()}

    def one_reading(dets):
        kept = []
        for d in sorted(dets, key=lambda d: -d["score"]):
            if all(iou(k["box"], d["box"]) <= 0.7 for k in kept):
                kept.append(d)
        return kept

    rows = []
    for i in data["subset"]["pets"]:
        dets = [d for d in one_reading(results[i]) if d["score"] >= 0.5]
        if not dets:
            continue
        img = Image.open(COCO / "img" / f"{i:012d}.jpg").convert("RGB")
        for d in dets:
            g = max(gt.get(i, []), key=lambda g: iou(d["box"], g["box"]), default=None)
            if g is None or iou(d["box"], g["box"]) < 0.5:
                continue
            x = ((np.asarray(crop_square(img, d["box"]).resize((224, 224), Image.BICUBIC), dtype=np.float32) / 255.0 - mean) / std).astype(np.float32)
            with torch.no_grad():
                e = clip.encode_image(torch.from_numpy(x).permute(2, 0, 1)[None])[0].numpy()
            logits = P @ (e / np.linalg.norm(e)) / 0.01
            p = np.exp(logits - logits.max())
            rows.append((d["species"], g["species"], float(p[0] / p.sum())))
    n = len(rows)
    print(f"{n} animals; D-FINE right on {sum(d == t for d, t, _ in rows) / n:.1%}")
    for t in (0.8, 0.9, 0.95, 0.98):
        fix = brk = ok = 0
        for det, truth, p_cat in rows:
            sp = "cat" if det == "dog" and p_cat >= t else "dog" if det == "cat" and 1 - p_cat >= t else det
            ok += sp == truth
            fix += det != truth and sp == truth
            brk += det == truth and sp != truth
        print(f"overrule at {t}: right {ok / n:.1%}; fixes {fix}, breaks {brk}")


def export_bin():
    """`export-bin <identity_*.npz> <embedder>`: the vectors as Lumen stores them (i8, L2 × 127)
    for the Rust replay of the grouping (`photos-core --test pets_grouping_public`). Layout:
    u32 count, u32 dims, then per crop an i32 animal and `dims` i8."""
    data = np.load(sys.argv[2])
    v, who = data[sys.argv[3]], data["who"].astype(np.int32)
    q = np.clip(np.round(v / np.linalg.norm(v, axis=1, keepdims=True) * 127), -127, 127).astype(np.int8)
    out = Path(sys.argv[2]).with_suffix(f".{sys.argv[3]}.bin")
    with open(out, "wb") as f:
        f.write(np.array([len(who), q.shape[1]], dtype=np.uint32).tobytes())
        for w, row in zip(who, q):
            f.write(np.int32(w).tobytes())
            f.write(row.tobytes())
    print(f"{out.name}: {len(who)} crops of {q.shape[1]} dims")


if __name__ == "__main__":
    {"detect": detect, "identity": identity, "report": lambda: report_identity(Path(sys.argv[2])), "export-bin": export_bin, "filter": filter_check, "species": species_check}[sys.argv[1]]()
