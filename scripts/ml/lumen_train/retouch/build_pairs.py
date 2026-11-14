"""Real pairs from a folder of originals with Lightroom sidecars (`photo.xmp` or
`photo.ext.xmp`): each is prepared as the app would (`retouch_prep`), its sidecar converted
to Lumen's sliders by 11A's converter (`photos-core` example `lr2lumen`, the same mapping the
app uses), and the target rendered by Lumen with what training's render scores
(`global_look`): no geometry (crop and straighten are learnt from their slider values), no
stage that needs neighbours, and only the gradients the layout holds.

    python -m lumen_train.retouch.build_pairs <photos> <out>/<registry source>

A RAW with the JPEG a photographer made from it and no sidecar (a source of kind `raw+jpeg`,
prepared by `data prep`) becomes a pair with `--align`: his JPEG is found in Lumen's render of
the RAW (`align.py`), his crop and angle become the geometry sliders, the only ones known, and
his JPEG warped back onto the RAW is the target, with a mask of the pixels his crop kept.

    python -m lumen_train.retouch.build_pairs --align <root>/<raw+jpeg source>

Originals are only read; nothing is written next to them."""

import argparse
import copy
import json
import os
import re
import subprocess
import sys
import tempfile
from pathlib import Path
from typing import Callable

import numpy as np
import torch
from PIL import Image, ImageOps

from ..common import icc, licence
from ..data import sources as sources_mod, splits
from . import align, params
from .paths import example
from .prepared import EDGE, encode_input, read_meta

Converter = Callable[[Path], dict]
Renderer = Callable[[Path, dict, Path], None]


def sidecar(photo: Path) -> Path | None:
    for c in (photo.with_suffix(".xmp"), photo.with_name(photo.name + ".xmp"), photo.with_suffix(".XMP")):
        if c.exists():
            return c
    return None


def global_look(develop: dict) -> dict:
    """`develop` as training's render can show it. Texture, clarity, dehaze, sharpening,
    denoise and grain need neighbours, so they train on their slider values alone; brush masks,
    extra gradients and masks' local texture/clarity/dehaze/sharpness have no slot. Leaving them
    in the target would make ΔE ask the global sliders to imitate them."""
    d = copy.deepcopy(develop)
    basic = d.setdefault("basic", {})
    for k in ("texture", "clarity", "dehaze"):
        basic[k] = 0.0
    detail = d.setdefault("detail", {})
    detail["sharpen"] = {**(detail.get("sharpen") or {}), "amount": 0.0}
    detail["denoise"] = None
    effects = d.setdefault("effects", {})
    effects["grain"] = {**(effects.get("grain") or {}), "amount": 0.0}
    kept = []
    for m in params.layout_masks(d):
        if m is not None:
            m = copy.deepcopy(m)
            m["adjust"] = {**m.get("adjust", {}), "texture": 0.0, "clarity": 0.0, "dehaze": 0.0, "sharpness": 0.0}
            kept.append(m)
    d["masks"] = kept
    return d


def build(prepared_root: Path, convert: Converter, render_target: Renderer) -> int:
    """Writes `target.json` (and through `render_target`, `target.u16`) for every prepared
    photo with a sidecar. Returns how many pairs were made."""
    made = 0
    for folder in sorted(p.parent for p in Path(prepared_root).glob("*/meta.json")):
        meta = read_meta(folder)
        photo = Path(meta.get("source", ""))
        xmp = sidecar(photo)
        if xmp is None:
            continue
        try:
            c = convert(xmp)
        except Exception as e:  # a bad sidecar skips its photo, never the run
            print(f"skipped {photo}: {e}", file=sys.stderr)
            continue
        geometry = c.get("geometry") or {}
        v = params.encode_develop_json(c["develop"], geometry.get("crop"), geometry.get("straighten", 0.0), float(meta.get("aspect", 1.0)))
        render_target(photo, global_look(c["develop"]), folder)
        (folder / "target.json").write_text(json.dumps({"v": v, "skipped": c.get("skipped", []), "xmp": str(xmp)}), encoding="utf-8")
        made += 1
    return made


RAW_EXTENSIONS = {".dng", ".cr2", ".cr3", ".nef", ".arw", ".raf", ".orf", ".rw2"}
JPEG_EXTENSIONS = {".jpg", ".jpeg"}
LOSSLESS_EXTENSIONS = {".png", ".tif", ".tiff"}
EDIT_EXTENSIONS = JPEG_EXTENSIONS | LOSSLESS_EXTENSIONS
GEOMETRY = ["straighten", "crop_on", "crop_x", "crop_y", "crop_w", "crop_h", "crop_ratio"]
_TARGET_FILES = ("target.json", "target.u16", "target_mask.u8")
_STILL = 0.1  # degrees: a smaller angle is the match's noise, not his straightening


def _srgb_to_linear(x: np.ndarray) -> np.ndarray:
    return np.where(x <= 0.04045, x / 12.92, ((x + 0.055) / 1.055) ** 2.4)


def _luminance(rgb: np.ndarray) -> np.ndarray:
    return (rgb[..., :3] @ np.array([0.2126, 0.7152, 0.0722])).astype(np.float32)


def read_edit(path: Path) -> np.ndarray:
    """His export, (h, w, 3) sRGB in [0, 1], upright: a JPEG, or a PNG / TIFF at its own depth
    (8 or 16 bits, or float), converted to sRGB from the colour profile it carries (Lightroom's
    TIFFs are ProPhoto, many JPEGs Adobe RGB); untagged means sRGB."""
    path = Path(path)
    try:
        with Image.open(path) as im:  # the header only: its profile
            profile = im.info.get("icc_profile")
    except Exception:  # a TIFF Pillow cannot open: OpenCV may still read it, untagged
        profile = None
    rgb = _read_pixels(path)
    return icc.to_srgb(rgb, profile) if profile else rgb


# EXIF Software of a camera, not an editor: missing, a firmware version (`Ver.1.03`, `17.5.1`),
# or naming the camera or its firmware.
_FIRMWARE = re.compile(r"(?:ver(?:sion)?\.?\s*)?v?\d+(?:\.\d+)*[a-z]?", re.IGNORECASE)


def camera_jpeg(path: Path) -> bool:
    """A JPEG as the camera wrote it (RAW+JPEG shooting), never an edit: EXIF names a camera
    model and its Software is missing or the camera's firmware. An export keeps the model but
    names its editor (Lightroom, Photoshop, …); a file with no camera model, or named as an
    edit (`-Edit`, `-Enhanced-NR`, ` copy`: an editor that kept the camera's tags), is an export."""
    if Path(path).suffix.lower() not in JPEG_EXTENSIONS or splits.edited_name(Path(path).name):
        return False
    try:
        with Image.open(path) as im:
            exif = im.getexif()
    except Exception:  # unreadable: left to the matching, which skips it with its reason
        return False
    model = str(exif.get(0x0110) or "").strip().strip("\x00").lower()
    software = str(exif.get(0x0131) or "").strip().strip("\x00").lower()
    if not model:
        return False
    return not software or bool(_FIRMWARE.fullmatch(software)) or model in software or "firmware" in software


def _read_pixels(path: Path) -> np.ndarray:
    if path.suffix.lower() in JPEG_EXTENSIONS:
        with Image.open(path) as im:
            return np.asarray(ImageOps.exif_transpose(im).convert("RGB"), dtype=np.float32) / 255.0
    import cv2

    # imdecode, not imread: imread cannot open a non-ASCII path on Windows. IMREAD_COLOR turns it
    # upright by its orientation tag and drops alpha; ANYDEPTH keeps 16 bits.
    bgr = cv2.imdecode(np.fromfile(path, dtype=np.uint8), cv2.IMREAD_COLOR | cv2.IMREAD_ANYDEPTH)
    if bgr is None:
        raise ValueError(f"cannot read {Path(path).name}")
    scale = {np.dtype(np.uint8): 255.0, np.dtype(np.uint16): 65535.0}.get(bgr.dtype, 1.0)
    return np.ascontiguousarray(bgr[..., ::-1], dtype=np.float32) / np.float32(scale)


def _chroma(rgb: np.ndarray) -> float:
    return float(np.abs(np.diff(rgb[..., :3], axis=-1)).mean())


def black_and_white(edited: np.ndarray, reference: np.ndarray) -> bool:
    """His JPEG has no colour where the RAW has some: a creative choice Auto must not learn."""
    return _chroma(edited) < 0.01 and _chroma(reference) > 0.02


def edits_by_stem(folder: Path) -> dict[str, list[Path]]:
    found: dict[str, list[Path]] = {}
    for p in sorted(Path(folder).rglob("*")):
        if p.is_file() and p.suffix.lower() in EDIT_EXTENSIONS:
            found.setdefault(splits.stem(p.name), []).append(p)
    return found


def _his_edits(raw: Path, candidates: list[Path]) -> list[Path]:
    """Every edit of this RAW, the likeliest first: his export over a camera JPEG of the same name
    (`DSC_1-Edit.jpg` over `DSC_1.JPG`), a lossless export over a JPEG one, then the newest."""
    return sorted(candidates, key=lambda p: (p.stem.lower() == raw.stem.lower(), p.suffix.lower() not in LOSSLESS_EXTENSIONS, -p.stat().st_mtime))


def geometry_vector(a: "align.Alignment", aspect: float, jpeg_ratio: float) -> list[float]:
    """Neutral sliders with his straighten and crop: the crop's ratio snapped to a bin within
    1 % of his JPEG's shape (centre kept), or free."""
    straighten, (x, y, w, h) = align.lumen_geometry(a, aspect)
    if abs(straighten) < _STILL:
        straighten = 0.0
    b = align.ratio_bin(jpeg_ratio, aspect)
    if b:
        r = params._ratio_of(b, aspect)
        cx, cy = x + w / 2, y + h / 2
        h = w * aspect / r
        if h > 1.0:
            h = 1.0
            w = h * r / aspect
            x = min(max(0.0, cx - w / 2), 1.0 - w)
        y = min(max(0.0, cy - h / 2), 1.0 - h)
    whole = x < 5e-3 and y < 5e-3 and x + w > 1 - 5e-3 and y + h > 1 - 5e-3
    crop = None if whole else {"x": x, "y": y, "w": w, "h": h}
    return params.encode_develop_json({}, crop, straighten, aspect)


FINE_EDGE = 2048


def _first_match(photo: Path, edits: list[Path], render_reference: Callable[[Path], np.ndarray], render_fine: Callable[[Path], np.ndarray] | None):
    """The first of his edits found in the photo: `(alignment, edit, edited, reference, pass)`,
    or `(Skip, first edit)` with the first edit's reason. Each edit is matched at 1024 px, then,
    if that finds too little, against `render_fine` at `FINE_EDGE` px with the same bars."""
    ref = render_reference(photo)
    fine = None
    first: tuple[align.Skip, Path] | None = None
    for edit in edits:
        try:
            edited = read_edit(edit)
        except Exception as e:  # one unreadable edit (a broken TIFF, a CMYK profile): try the next
            first = first or (align.Skip(f"{type(e).__name__}: {e}"), edit)
            continue
        if black_and_white(edited, ref):
            first = first or (align.Skip("his edit is black and white"), edit)
            continue
        a = align.estimate(_luminance(ref), _luminance(edited))
        if isinstance(a, align.Alignment):
            return a, edit, edited, ref, "coarse"
        if render_fine is not None:
            fine = render_fine(photo) if fine is None else fine
            b = align.estimate(_luminance(fine), _luminance(edited), long_edge=FINE_EDGE)
            if isinstance(b, align.Alignment):
                return b, edit, edited, fine, "fine"
        first = first or (a, edit)
    return first


def _depth(a: Path, b: Path) -> int:
    """How many folders `a` and `b` share from the top (0 on different drives)."""
    try:
        return len(Path(os.path.commonpath([a.parent, b.parent])).parts)
    except ValueError:
        return 0


def _nearest(edits: list[Path], raw: Path, raws: list[Path]) -> list[Path]:
    """The edits as near this RAW as any other RAW of its name: `a/DSC_0001.NEF` never takes
    `b/DSC_0001-Edit.jpg`, another card's photo."""
    others = [r for r in raws if r != raw]
    return [e for e in edits if all(_depth(e, raw) >= _depth(e, o) for o in others)]


def build_aligned(prepared_root: Path, originals: Path, render_reference: Callable[[Path], np.ndarray], render_fine: Callable[[Path], np.ndarray] | None = None) -> int:
    """`target.json` (geometry known), `target.u16` and `target_mask.u8` for every prepared RAW
    whose edit is found in it; one line per RAW in `pairs.jsonl`. `render_reference(raw)` is
    Lumen's render of the RAW with its defaults, (H, W, 3) sRGB in [0, 1]; `render_fine` the same
    at `FINE_EDGE` px for a second pass. Returns the pairs made."""
    prepared_root = Path(prepared_root)
    edits = edits_by_stem(originals)
    lines, made = [], 0
    photos = []
    for folder in sorted(p.parent for p in prepared_root.glob("*/meta.json")):
        meta = read_meta(folder)
        # `photo` is what was prepared: the RAW, or (`stand_in.json`, written by `data prep`) the
        # full-size JPEG inside a RAW Lumen cannot decode.
        photo = raw = Path(meta.get("source", ""))
        stand_in = json.loads((folder / "stand_in.json").read_text(encoding="utf-8")) if (folder / "stand_in.json").exists() else None
        if stand_in:
            raw = Path(stand_in["raw"])
        if raw.suffix.lower() in RAW_EXTENSIONS:
            photos.append((folder, meta, photo, raw, stand_in))
    raws_by_stem: dict[str, list[Path]] = {}
    for _, _, _, raw, _ in photos:
        raws_by_stem.setdefault(splits.stem(raw.name), []).append(raw)
    for folder, meta, photo, raw, stand_in in photos:
        for name in _TARGET_FILES:  # a pair that no longer matches never keeps an old target
            (folder / name).unlink(missing_ok=True)
        line = {"raw": str(raw), "folder": folder.name, **({"via": stand_in["via"]} if stand_in else {})}
        if not raw.exists():  # deleted or replaced (an HE NEF by its DNG): never trains twice
            lines.append({**line, "status": "skipped", "reason": "its RAW is gone"})
            continue
        candidates = _nearest(edits.get(splits.stem(raw.name), []), raw, raws_by_stem[splits.stem(raw.name)])
        if not candidates:
            lines.append({**line, "status": "skipped", "reason": "no edited JPEG, PNG or TIFF with this stem"})
            continue
        exports = [p for p in candidates if not camera_jpeg(p)]
        if not exports:  # RAW+JPEG shooting, never edited: the camera's look is not his
            lines.append({**line, "jpeg": str(candidates[0]), "status": "skipped", "reason": "only the camera's own JPEG (EXIF: camera firmware), no edit"})
            continue
        candidates = exports
        try:
            found = _first_match(photo, _his_edits(raw, candidates), render_reference, render_fine)
        except Exception as e:  # one bad file skips its photo, never the run
            lines.append({**line, "jpeg": str(candidates[0]), "status": "skipped", "reason": f"{type(e).__name__}: {e}"})
            continue
        if isinstance(found[0], align.Skip):
            lines.append({**line, "jpeg": str(found[1]), "status": "skipped", "reason": found[0].reason})
            continue
        a, jpeg, edited, ref, how = found
        line = {**line, "jpeg": str(jpeg), **({"pass": how} if how != "coarse" else {})}
        w, h = int(meta["width"]), int(meta["height"])
        rgb, valid = align.warp_back(edited, a, w, h)
        lin = torch.from_numpy(_srgb_to_linear(rgb).astype(np.float32)).permute(2, 0, 1)
        (encode_input(lin, w, h).clamp(0, 1) * 65535).round().numpy().astype("<u2").tofile(folder / "target.u16")
        mask = np.zeros((EDGE, EDGE), np.uint8)
        mask[:h, :w] = valid
        mask.tofile(folder / "target_mask.u8")
        aspect = ref.shape[1] / ref.shape[0]
        v = geometry_vector(a, aspect, edited.shape[1] / edited.shape[0])
        aligned = {"angle": a.angle_deg, "box": list(a.box), "inliers": a.inliers, "ratio": a.ratio}
        (folder / "target.json").write_text(json.dumps({"v": v, "known": GEOMETRY, "aligned": aligned, "jpeg": str(jpeg)}), encoding="utf-8")
        lines.append({**line, "status": "ok", **aligned})
        made += 1
    (prepared_root / "pairs.jsonl").write_text("".join(json.dumps(x) + "\n" for x in lines), encoding="utf-8")
    return made


def render_default(photo: Path, edge: int = align.LONG_EDGE) -> np.ndarray:
    """Lumen's render of `photo` with no edits, `edge` px on its long side."""
    with tempfile.TemporaryDirectory() as tmp:
        recipe = Path(tmp) / "develop.json"
        recipe.write_text("{}", encoding="utf-8")
        png = Path(tmp) / "reference.png"
        subprocess.run([str(example("render_recipe")), str(photo), str(recipe), str(png), "--edge", str(edge)], check=True)
        with Image.open(png) as im:
            return np.asarray(im.convert("RGB"), dtype=np.float32) / 255.0


def lr2lumen(xmp: Path) -> dict:
    out = subprocess.run([str(example("lr2lumen")), str(xmp)], capture_output=True, text=True, check=True)
    return json.loads(out.stdout)


def render_with_lumen(photo: Path, develop: dict, folder: Path) -> None:
    meta = read_meta(folder)
    w, h = int(meta["width"]), int(meta["height"])
    with tempfile.TemporaryDirectory() as tmp:
        recipe = Path(tmp) / "develop.json"
        recipe.write_text(json.dumps(develop), encoding="utf-8")
        png = Path(tmp) / "target.png"
        subprocess.run([str(example("render_recipe")), str(photo), str(recipe), str(png), "--edge", str(max(w, h))], check=True)
        img = Image.open(png).convert("RGB")
    if img.size != (w, h):
        img = img.resize((w, h), Image.BILINEAR)
    srgb = torch.from_numpy(np.asarray(img, dtype=np.float32) / 255.0).permute(2, 0, 1)
    lin = torch.where(srgb <= 0.04045, srgb / 12.92, ((srgb + 0.055) / 1.055) ** 2.4)
    packed = encode_input(lin, w, h)
    (packed.clamp(0, 1) * 65535).round().numpy().astype("<u2").tofile(folder / "target.u16")


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("photos", type=Path, nargs="?")
    ap.add_argument("out", type=Path, nargs="?", help="<root>/<registry source>")
    ap.add_argument("--jobs", type=int, default=4)
    ap.add_argument("--align", type=Path, metavar="SOURCE_DIR", help="<root>/<raw+jpeg source>, prepared by `data prep`")
    a = ap.parse_args()
    if a.align is not None:
        src = sources_mod.load(a.align.parent).get(a.align.name)
        if src is None or src.kind != "raw+jpeg" or src.originals is None:
            sys.exit(f"{a.align.name!r} is not a local source of kind \"raw+jpeg\" under {a.align.parent} (data add … --kind raw+jpeg)")
        made = build_aligned(a.align, src.originals, render_default, render_fine=lambda p: render_default(p, FINE_EDGE))
        print(f"{made} pairs in {a.align}; skipped photos and their reasons in {a.align / 'pairs.jsonl'}")
        return
    if a.photos is None or a.out is None:
        ap.error("give <photos> <out>, or --align <root>/<source>")
    reg = licence.load_registry()
    if reg.get(a.out.name, {}).get("kind") != "pairs":
        sys.exit(f"{a.out.name!r} is not a registered source of kind \"pairs\" (datasets/registry.toml): name the folder after it")
    subprocess.run([str(example("retouch_prep")), str(a.photos), str(a.out), "--auto", "--jobs", str(a.jobs)], check=True)
    print(f"{build(a.out, lr2lumen, render_with_lumen)} pairs in {a.out}")


if __name__ == "__main__":
    main()
