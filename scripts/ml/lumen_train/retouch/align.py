"""A RAW and the JPEG a photographer made from it, without his sidecar: where his JPEG sits in
the RAW (Round 12-3).

`estimate` matches the two on gradient images, so his tone edits do not break the match:
ORB features, a ratio test, then a similarity transform (turn, scale, shift) by RANSAC. His
crop box and angle come from that transform, in the reference's frame. A pair with too few
consistent matches is a `Skip` with its reason, never a wrong pair.

Coordinates are continuous (0 at an image's top-left edge, `w` at its right edge). `matrix`
maps his JPEG's unit coordinates (0–1 across and down) to the reference's, so it holds at any
resolution of either."""

import math
from dataclasses import dataclass

import cv2
import numpy as np

from . import params

LONG_EDGE = 1024
FEATURES = 4000
RATIO_TEST = 0.75
RANSAC_PX = 3.0
MIN_INLIERS = 40
MIN_INLIER_RATIO = 0.3
SCALE = (0.2, 1.05)  # his crop's longer side, as a share of the reference's


@dataclass(frozen=True)
class Alignment:
    angle_deg: float  # his JPEG's content turned by this much in the reference (y down)
    box: tuple[float, float, float, float]  # x, y, w, h of the un-rotated crop, unit reference coordinates
    inliers: int
    ratio: float
    matrix: np.ndarray  # (2, 3): his unit coordinates → the reference's


@dataclass(frozen=True)
class Skip:
    reason: str


def _small(img: np.ndarray, long_edge: int = LONG_EDGE) -> tuple[np.ndarray, float]:
    h, w = img.shape[:2]
    f = min(1.0, long_edge / max(w, h))
    if f < 1.0:
        img = cv2.resize(img, (max(1, round(w * f)), max(1, round(h * f))), interpolation=cv2.INTER_AREA)
    return img, f


def _gradients(img: np.ndarray) -> np.ndarray:
    """Gradient magnitude, normalised to 8 bits: edges survive a change of tone."""
    g = cv2.GaussianBlur(img.astype(np.float32), (0, 0), 1.0)
    mag = cv2.magnitude(cv2.Sobel(g, cv2.CV_32F, 1, 0, ksize=3), cv2.Sobel(g, cv2.CV_32F, 0, 1, ksize=3))
    top = float(np.percentile(mag, 99.5))
    if top <= 1e-6:
        return np.zeros(mag.shape, np.uint8)
    return (np.clip(mag / top, 0.0, 1.0) * 255).astype(np.uint8)


def _features(img: np.ndarray, features: int = FEATURES):
    orb = cv2.ORB_create(nfeatures=features, nlevels=12, scaleFactor=1.2, edgeThreshold=15, patchSize=31)
    return orb.detectAndCompute(_gradients(img), None)


def to_unit(m: np.ndarray, ref_w: float, ref_h: float, ed_w: float, ed_h: float) -> np.ndarray:
    """A (2, 3) map between continuous pixel coordinates → the same map in unit coordinates."""
    a = np.diag([1 / ref_w, 1 / ref_h]) @ m[:, :2] @ np.diag([ed_w, ed_h])
    b = np.array([m[0, 2] / ref_w, m[1, 2] / ref_h])
    return np.hstack([a, b[:, None]])


def _pixels(m: np.ndarray, ref_w: float, ref_h: float, ed_w: float, ed_h: float) -> np.ndarray:
    a = np.diag([ref_w, ref_h]) @ m[:, :2] @ np.diag([1 / ed_w, 1 / ed_h])
    return np.hstack([a, np.array([m[0, 2] * ref_w, m[1, 2] * ref_h])[:, None]])


def _box(m_unit: np.ndarray, ref_w: float, ref_h: float, ed_w: float, ed_h: float) -> tuple[float, float, float, float]:
    """The crop before his turn: centred where his JPEG's centre lands, his JPEG's size times the
    transform's scale, in unit reference coordinates."""
    m = _pixels(m_unit, ref_w, ref_h, ed_w, ed_h)
    s = math.hypot(m[0, 0], m[1, 0])
    cx, cy = m @ np.array([ed_w / 2, ed_h / 2, 1.0])
    w, h = s * ed_w, s * ed_h
    return ((cx - w / 2) / ref_w, (cy - h / 2) / ref_h, w / ref_w, h / ref_h)


def estimate(reference: np.ndarray, edited: np.ndarray, long_edge: int = LONG_EDGE) -> Alignment | Skip:
    """`reference` (H, W) and `edited` (h, w): luminance, float32 in [0, 1], matched at most
    `long_edge` px (a finer size keeps a tight crop's detail; the bars stay the same)."""
    ref_h, ref_w = reference.shape[:2]
    ed_h, ed_w = edited.shape[:2]
    ref_s, f_ref = _small(reference, long_edge)
    ed_s, f_ed = _small(edited, long_edge)
    features = round(FEATURES * long_edge / LONG_EDGE)
    kr, dr = _features(ref_s, features)
    ke, de = _features(ed_s, features)
    if dr is None or de is None or len(kr) < MIN_INLIERS or len(ke) < MIN_INLIERS:
        return Skip(f"too few features ({0 if de is None else len(ke)} in the JPEG, {0 if dr is None else len(kr)} in the RAW)")
    pairs = cv2.BFMatcher(cv2.NORM_HAMMING).knnMatch(de, dr, k=2)
    good = [p[0] for p in pairs if len(p) == 2 and p[0].distance < RATIO_TEST * p[1].distance]
    if len(good) < MIN_INLIERS:
        return Skip(f"{len(good)} matches after the ratio test (need {MIN_INLIERS})")
    # His points in his full-size pixels, the reference's in its small pixels (RANSAC's threshold).
    src = np.float32([(np.array(ke[g.queryIdx].pt) + 0.5) / f_ed for g in good])
    dst = np.float32([np.array(kr[g.trainIdx].pt) + 0.5 for g in good])
    m, keep = cv2.estimateAffinePartial2D(src, dst, method=cv2.RANSAC, ransacReprojThreshold=RANSAC_PX, maxIters=5000, confidence=0.999)
    if m is None:
        return Skip("no transform fits the matches")
    inliers = int(keep.sum())
    ratio = inliers / len(good)
    if inliers < MIN_INLIERS or ratio < MIN_INLIER_RATIO:
        return Skip(f"{inliers} inliers of {len(good)} matches (need {MIN_INLIERS} and {MIN_INLIER_RATIO:.0%})")
    m = m.astype(np.float64) / f_ref  # → the reference's full-size pixels
    unit = to_unit(m, ref_w, ref_h, ed_w, ed_h)
    box = _box(unit, ref_w, ref_h, ed_w, ed_h)
    side = max(box[2], box[3])
    if not SCALE[0] <= side <= SCALE[1]:
        return Skip(f"his crop would be {side:.2f} of the RAW's side (allowed {SCALE[0]}–{SCALE[1]})")
    angle = math.degrees(math.atan2(m[1, 0], m[0, 0]))
    return Alignment(angle_deg=angle, box=box, inliers=inliers, ratio=ratio, matrix=unit)


def warp_back(edited_rgb: np.ndarray, alignment: Alignment, ref_w: int, ref_h: int) -> tuple[np.ndarray, np.ndarray]:
    """His JPEG drawn in the reference's frame at `ref_w × ref_h`, and where it covers it."""
    ed_h, ed_w = edited_rgb.shape[:2]
    m = _pixels(alignment.matrix, ref_w, ref_h, ed_w, ed_h)
    scale = math.hypot(m[0, 0], m[1, 0])
    src = edited_rgb.astype(np.float32)
    if scale < 1.0:  # shrink first, so the warp does not alias
        nw, nh = max(1, round(ed_w * scale)), max(1, round(ed_h * scale))
        src = cv2.resize(src, (nw, nh), interpolation=cv2.INTER_AREA)
        m = m @ np.diag([ed_w / nw, ed_h / nh, 1.0])
        ed_w, ed_h = nw, nh
    # Pixel indices: index = continuous − 0.5 on both sides.
    a, b = m[:, :2], m[:, 2]
    idx = np.hstack([a, (b + a @ np.array([0.5, 0.5]) - 0.5)[:, None]])
    rgb = cv2.warpAffine(src, idx, (ref_w, ref_h), flags=cv2.INTER_LINEAR, borderMode=cv2.BORDER_CONSTANT, borderValue=0)
    cover = cv2.warpAffine(np.ones((ed_h, ed_w), np.float32), idx, (ref_w, ref_h), flags=cv2.INTER_LINEAR, borderMode=cv2.BORDER_CONSTANT, borderValue=0)
    valid = cover > 0.999  # a pixel half outside his crop is not his
    if rgb.ndim == 2:
        rgb = rgb[..., None]
    return rgb * valid[..., None], valid


def lumen_geometry(alignment: Alignment, aspect: float) -> tuple[float, tuple[float, float, float, float]]:
    """His crop as Lumen's sliders: `straighten` (degrees) and the crop box (unit x, y, w, h of
    the straightened frame). Lumen turns the photo about its centre and zooms it to fill the
    frame (`geometry::straighten_source`), then crops; a crop the zoom cut off is clamped."""
    w, h = aspect, 1.0  # unit height; only ratios matter
    m = _pixels(alignment.matrix, w, h, 1.0, 1.0)  # his JPEG as a unit square
    theta = math.atan2(m[1, 0], m[0, 0])
    sx, sy = math.hypot(m[0, 0], m[1, 0]), math.hypot(m[0, 1], m[1, 1])
    s, c = math.sin(theta), math.cos(theta)
    z = abs(c) + abs(s) * max(w / h, h / w)
    ex, ey = m @ np.array([0.5, 0.5, 1.0])
    dx, dy = ex - w / 2, ey - h / 2
    # Lumen samples C + R(θ)·(p − C)/z, so p = C + z·R(−θ)·(e − C).
    px, py = w / 2 + z * (c * dx + s * dy), h / 2 + z * (-s * dx + c * dy)
    bw, bh = z * sx, z * sy
    x0, y0 = max(0.0, px - bw / 2), max(0.0, py - bh / 2)
    x1, y1 = min(w, px + bw / 2), min(h, py + bh / 2)
    return -math.degrees(theta), (x0 / w, y0 / h, max(0.0, x1 - x0) / w, max(0.0, y1 - y0) / h)


def ratio_bin(pixels: float, aspect: float, tol: float = 0.01) -> int:
    """The crop-ratio bin (`params.RATIOS`, following the photo's orientation) within `tol` of
    his JPEG's shape, the nearest if several, the first on a tie; 0 (free) if none."""
    best, err = 0, tol
    for b in range(1, len(params.RATIOS)):
        r = params._ratio_of(b, aspect)
        if r is None:
            continue
        e = abs(r - pixels) / r
        if e <= err and (best == 0 or e < err):
            best, err = b, e
    return best
