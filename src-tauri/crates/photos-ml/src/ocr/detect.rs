//! Text detection (Round 13): PaddleOCR's `DetResizeForTest` (`limit_type="max"`),
//! `NormalizeImage` and `DBPostProcess` (`box_type="quad"`, `score_mode="fast"`), ported.

use super::{resize_bilinear, Quad};
use crate::manifest::OcrDetSpec;

const MEAN: [f32; 3] = [0.485, 0.456, 0.406];
const STD: [f32; 3] = [0.229, 0.224, 0.225];

type P = [f64; 2];

/// The detector's input size: long side at most `side` (never upscaled), each side rounded
/// (half to even, like Python's `round`) to a multiple of 32, at least 32.
pub fn det_input_size(w: u32, h: u32, side: u32) -> (u32, u32) {
    let long = w.max(h);
    let ratio = if long > side { side as f64 / long as f64 } else { 1.0 };
    let snap = |v: u32| {
        let r = (v as f64 * ratio).trunc();
        (((r / 32.0).round_ties_even() as u32) * 32).max(32)
    };
    (snap(w), snap(h))
}

/// The image resized for the detector as an NCHW tensor. PaddleOCR feeds OpenCV's BGR, so
/// channel 0 is blue, normalised with ImageNet's *first* mean and std.
pub fn det_tensor(rgb: &[u8], w: u32, h: u32, side: u32) -> (Vec<f32>, u32, u32) {
    let (dw, dh) = det_input_size(w, h, side);
    let small = resize_bilinear(rgb, w, h, dw, dh);
    let plane = (dw * dh) as usize;
    let mut t = vec![0f32; 3 * plane];
    for (i, px) in small.as_chunks::<3>().0.iter().enumerate() {
        for c in 0..3 {
            t[c * plane + i] = (px[2 - c] as f32 / 255.0 - MEAN[c]) / STD[c];
        }
    }
    (t, dw, dh)
}

/// The detector's probability map (`mw × mh`, row-major) → text quads in pixels of the
/// `src_w × src_h` image, each with its box score. Unordered; see [`reading_order`].
pub fn boxes_from_map(prob: &[f32], mw: u32, mh: u32, src_w: u32, src_h: u32, spec: &OcrDetSpec) -> Vec<(Quad, f32)> {
    let bitmap: Vec<u8> = prob.iter().map(|&p| if p > spec.thresh { 255 } else { 0 }).collect();
    let Some(img) = image::GrayImage::from_raw(mw, mh, bitmap) else { return Vec::new() };
    let (sx, sy) = (src_w as f64 / mw as f64, src_h as f64 / mh as f64);
    let mut out = Vec::new();
    for contour in imageproc::contours::find_contours::<i32>(&img).into_iter().take(spec.max_candidates) {
        let pts: Vec<P> = contour.points.iter().map(|p| [p.x as f64, p.y as f64]).collect();
        let (quad, short) = mini_box(&min_area_rect(&pts));
        if short < spec.min_side as f64 {
            continue;
        }
        let score = box_score(prob, mw, mh, &quad);
        if spec.box_thresh > score {
            continue;
        }
        // Unclip: pyclipper offsets the integer-truncated box by d with round joins and
        // returns integer vertices; the min-area rect of that polygon is the new box.
        let perimeter: f64 = (0..4).map(|i| dist(quad[i], quad[(i + 1) % 4])).sum();
        let d = area(&quad).abs() * spec.unclip_ratio as f64 / perimeter;
        let ints: Vec<P> = quad.iter().map(|p| [p[0].trunc(), p[1].trunc()]).collect();
        let (quad, short) = mini_box(&min_area_rect(&round_offset(&ints, d)));
        if short < spec.min_side as f64 + 2.0 {
            continue;
        }
        let scale = |v: f64, s: f64, hi: u32| (v * s).round_ties_even().clamp(0.0, hi as f64) as f32;
        out.push((quad.map(|p| [scale(p[0], sx, src_w), scale(p[1], sy, src_h)]), score));
    }
    out
}

/// The box PaddleX 3 cuts a line from (`CropByPolys.get_minarea_rect_crop`): the min-area
/// rect of the quad's corners truncated to integers, corners ordered like [`mini_box`].
pub fn crop_box(q: &Quad) -> Quad {
    let ints: Vec<P> = q.iter().map(|p| [p[0].trunc() as f64, p[1].trunc() as f64]).collect();
    let (corners, _) = mini_box(&min_area_rect(&ints));
    corners.map(|p| [p[0] as f32, p[1] as f32])
}

/// PaddleOCR's `sorted_boxes`: by top-left (y, x), then a box within 10 px of the previous
/// one's top and to its left moves before it.
pub fn reading_order(quads: &mut [(Quad, f32)]) {
    quads.sort_by(|a, b| (a.0[0][1], a.0[0][0]).partial_cmp(&(b.0[0][1], b.0[0][0])).unwrap_or(std::cmp::Ordering::Equal));
    for i in 0..quads.len().saturating_sub(1) {
        for j in (0..=i).rev() {
            let (a, b) = (quads[j].0[0], quads[j + 1].0[0]);
            if (b[1] - a[1]).abs() < 10.0 && b[0] < a[0] {
                quads.swap(j, j + 1);
            } else {
                break;
            }
        }
    }
}

/// A rotated rectangle: centre, unit axis `u`, half extents along `u` and its normal.
#[derive(Clone, Copy)]
struct Rect {
    c: P,
    u: P,
    hw: f64,
    hh: f64,
}

impl Rect {
    fn corners(&self) -> [P; 4] {
        let n = [-self.u[1], self.u[0]];
        let at = |a: f64, b: f64| [self.c[0] + a * self.u[0] + b * n[0], self.c[1] + a * self.u[1] + b * n[1]];
        [at(-self.hw, -self.hh), at(self.hw, -self.hh), at(self.hw, self.hh), at(-self.hw, self.hh)]
    }

}

/// Clipper 6.4.2's `ClipperOffset` (what pyclipper runs) on a closed polygon with integer
/// vertices, `jtRound`, arc tolerance 0.25, growing by `delta > 0`: the offset polygon's
/// vertices, rounded half away from zero. Only its hull matters to the caller, so the
/// union Clipper runs afterwards is left out.
fn round_offset(src: &[P], delta: f64) -> Vec<P> {
    let mut src: Vec<P> = src.to_vec();
    src.dedup();
    while src.len() > 1 && src.first() == src.last() {
        src.pop();
    }
    if src.len() < 3 || delta <= 0.0 {
        return src;
    }
    if area(&src) < 0.0 {
        src.reverse();
    }
    let round = |v: f64| if v < 0.0 { (v - 0.5).trunc() } else { (v + 0.5).trunc() };
    let y = (delta * 0.25).min(0.25);
    let mut steps = std::f64::consts::PI / (1.0 - y / delta).acos();
    steps = steps.min(delta * std::f64::consts::PI);
    let (sin, cos) = ((std::f64::consts::TAU / steps).sin(), (std::f64::consts::TAU / steps).cos());
    let steps_per_rad = steps / std::f64::consts::TAU;
    let len = src.len();
    let normals: Vec<P> = (0..len)
        .map(|j| {
            let (a, b) = (src[j], src[(j + 1) % len]);
            let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
            let l = dx.hypot(dy);
            if l == 0.0 { [0.0, 0.0] } else { [dy / l, -dx / l] }
        })
        .collect();
    let at = |p: P, n: P| [round(p[0] + n[0] * delta), round(p[1] + n[1] * delta)];
    let mut out = Vec::new();
    let mut k = len - 1;
    for j in 0..len {
        let (nk, nj) = (normals[k], normals[j]);
        let mut sin_a = nk[0] * nj[1] - nj[0] * nk[1];
        let cos_a = nk[0] * nj[0] + nk[1] * nj[1];
        if (sin_a * delta).abs() < 1.0 {
            if cos_a > 0.0 {
                out.push(at(src[j], nk));
                k = j;
                continue;
            }
        } else {
            sin_a = sin_a.clamp(-1.0, 1.0);
        }
        if sin_a * delta < 0.0 {
            out.extend([at(src[j], nk), src[j], at(src[j], nj)]);
        } else {
            let a = sin_a.atan2(cos_a);
            let n = (round(steps_per_rad * a.abs()) as i64).max(1);
            let (mut x, mut yy) = (nk[0], nk[1]);
            for _ in 0..n {
                out.push(at(src[j], [x, yy]));
                (x, yy) = (x * cos - sin * yy, x * sin + yy * cos);
            }
            out.push(at(src[j], nj));
        }
        k = j;
    }
    out
}

fn dist(a: P, b: P) -> f64 {
    (b[0] - a[0]).hypot(b[1] - a[1])
}

fn cross(o: P, a: P, b: P) -> f64 {
    (a[0] - o[0]) * (b[1] - o[1]) - (a[1] - o[1]) * (b[0] - o[0])
}

/// Signed shoelace area.
fn area(poly: &[P]) -> f64 {
    (0..poly.len()).map(|i| cross([0.0, 0.0], poly[i], poly[(i + 1) % poly.len()])).sum::<f64>() / 2.0
}

/// Andrew's monotone chain; collinear points dropped.
fn hull(pts: &[P]) -> Vec<P> {
    let mut p = pts.to_vec();
    p.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    p.dedup();
    if p.len() < 3 {
        return p;
    }
    let mut h: Vec<P> = Vec::with_capacity(2 * p.len());
    for pass in [false, true] {
        let start = h.len();
        let iter: Box<dyn Iterator<Item = &P>> = if pass { Box::new(p.iter().rev()) } else { Box::new(p.iter()) };
        for &q in iter {
            while h.len() >= start + 2 && cross(h[h.len() - 2], h[h.len() - 1], q) <= 0.0 {
                h.pop();
            }
            h.push(q);
        }
        h.pop();
    }
    h
}

/// `cv2.minAreaRect`: rotating calipers over the hull's edges.
fn min_area_rect(pts: &[P]) -> Rect {
    let h = hull(pts);
    let mut best = Rect { c: h.first().copied().unwrap_or([0.0, 0.0]), u: [1.0, 0.0], hw: 0.0, hh: 0.0 };
    let mut best_area = f64::INFINITY;
    for i in 0..h.len() {
        let (a, b) = (h[i], h[(i + 1) % h.len()]);
        let len = dist(a, b);
        if len == 0.0 {
            continue;
        }
        let u = [(b[0] - a[0]) / len, (b[1] - a[1]) / len];
        let n = [-u[1], u[0]];
        let (mut u0, mut u1, mut n0, mut n1) = (f64::INFINITY, f64::NEG_INFINITY, f64::INFINITY, f64::NEG_INFINITY);
        for p in &h {
            let (pu, pn) = (p[0] * u[0] + p[1] * u[1], p[0] * n[0] + p[1] * n[1]);
            (u0, u1, n0, n1) = (u0.min(pu), u1.max(pu), n0.min(pn), n1.max(pn));
        }
        let a = (u1 - u0) * (n1 - n0);
        if a < best_area - 1e-9 {
            let (cu, cn) = ((u0 + u1) / 2.0, (n0 + n1) / 2.0);
            best_area = a;
            best = Rect { c: [cu * u[0] + cn * n[0], cu * u[1] + cn * n[1]], u, hw: (u1 - u0) / 2.0, hh: (n1 - n0) / 2.0 };
        }
    }
    best
}

/// PaddleOCR's `get_mini_boxes`: corners sorted by x, then top-left, top-right,
/// bottom-right, bottom-left by y within each pair; and the short side.
fn mini_box(r: &Rect) -> ([P; 4], f64) {
    let mut p = r.corners();
    p.sort_by(|a, b| a[0].total_cmp(&b[0]));
    let (i1, i4) = if p[1][1] > p[0][1] { (0, 1) } else { (1, 0) };
    let (i2, i3) = if p[3][1] > p[2][1] { (2, 3) } else { (3, 2) };
    ([p[i1], p[i2], p[i3], p[i4]], 2.0 * r.hw.min(r.hh))
}

/// PaddleOCR's `box_score_fast`: the mean probability inside the box (its integer-truncated
/// polygon filled over its integer bounding box, edges included).
fn box_score(prob: &[f32], mw: u32, mh: u32, q: &[P; 4]) -> f32 {
    let lim = |v: f64, hi: u32| (v as i64).clamp(0, hi as i64 - 1);
    let (xs, ys) = (q.map(|p| p[0]), q.map(|p| p[1]));
    let fold = |v: [f64; 4], f: fn(f64, f64) -> f64| v.into_iter().reduce(f).unwrap_or(0.0);
    let (x0, x1) = (lim(fold(xs, f64::min).floor(), mw), lim(fold(xs, f64::max).ceil(), mw));
    let (y0, y1) = (lim(fold(ys, f64::min).floor(), mh), lim(fold(ys, f64::max).ceil(), mh));
    let poly = q.map(|p| [(p[0] - x0 as f64).trunc(), (p[1] - y0 as f64).trunc()]);
    let sign = if area(&poly) >= 0.0 { 1.0 } else { -1.0 };
    let (mut sum, mut n) = (0f64, 0u32);
    for y in 0..=(y1 - y0) {
        for x in 0..=(x1 - x0) {
            let pt = [x as f64, y as f64];
            let inside = (0..4).all(|i| {
                let (a, b) = (poly[i], poly[(i + 1) % 4]);
                let len = dist(a, b);
                len == 0.0 || sign * cross(a, b, pt) / len >= -0.5
            });
            if inside {
                sum += prob[((y + y0) as u32 * mw + (x + x0) as u32) as usize] as f64;
                n += 1;
            }
        }
    }
    if n == 0 { 0.0 } else { (sum / n as f64) as f32 }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::OcrDetSpec;

    fn map_with(mw: u32, mh: u32, f: impl Fn(u32, u32) -> f32) -> Vec<f32> {
        (0..mh).flat_map(|y| (0..mw).map(move |x| (x, y))).map(|(x, y)| f(x, y)).collect()
    }

    fn close(q: &Quad, want: [[f32; 2]; 4], tol: f32) {
        for (p, w) in q.iter().zip(want) {
            assert!((p[0] - w[0]).abs() <= tol && (p[1] - w[1]).abs() <= tol, "{q:?} vs {want:?}");
        }
    }

    #[test]
    fn input_sizes_are_multiples_of_32() {
        assert_eq!(det_input_size(1536, 1024, 960), (960, 640));
        assert_eq!(det_input_size(4000, 30, 960), (960, 32));
        assert_eq!(det_input_size(500, 300, 960), (512, 288));
    }

    #[test]
    fn tensor_is_bgr_normalised() {
        let red: Vec<u8> = (0..32 * 32).flat_map(|_| [255, 0, 0]).collect();
        let (t, w, h) = det_tensor(&red, 32, 32, 960);
        assert_eq!((w, h, t.len()), (32, 32, 3 * 32 * 32));
        let plane = 32 * 32;
        assert!((t[0] - (0.0 - 0.485) / 0.229).abs() < 1e-5, "channel 0 is blue");
        assert!((t[2 * plane] - (1.0 - 0.406) / 0.225).abs() < 1e-5, "channel 2 is red");
    }

    #[test]
    fn one_rect_gives_one_unclipped_quad() {
        let prob = map_with(100, 60, |x, y| if (20..60).contains(&x) && (20..30).contains(&y) { 0.9 } else { 0.0 });
        let boxes = boxes_from_map(&prob, 100, 60, 100, 60, &OcrDetSpec::default());
        assert_eq!(boxes.len(), 1);
        // Pixel centres 20..59 × 20..29: a 39 × 9 rect, d = 351 · 1.5 / 96 = 5.48 → 14.52..64.48 × 14.52..34.48, rounded.
        close(&boxes[0].0, [[15.0, 15.0], [64.0, 15.0], [64.0, 34.0], [15.0, 34.0]], 0.01);
        assert!((boxes[0].1 - 0.9).abs() < 1e-4);
    }

    #[test]
    fn faint_and_tiny_regions_are_dropped() {
        let prob = map_with(100, 60, |x, y| {
            if (10..50).contains(&x) && (10..20).contains(&y) {
                0.4
            } else if (70..72).contains(&x) && (40..42).contains(&y) {
                0.9
            } else {
                0.0
            }
        });
        assert!(boxes_from_map(&prob, 100, 60, 100, 60, &OcrDetSpec::default()).is_empty());
    }

    #[test]
    fn rotated_rect_keeps_its_angle() {
        let (s, c) = 30f32.to_radians().sin_cos();
        let prob = map_with(120, 100, |x, y| {
            let (dx, dy) = (x as f32 - 60.0, y as f32 - 50.0);
            let (u, v) = (dx * c + dy * s, -dx * s + dy * c);
            if u.abs() <= 30.0 && v.abs() <= 8.0 { 0.9 } else { 0.0 }
        });
        let boxes = boxes_from_map(&prob, 120, 100, 120, 100, &OcrDetSpec::default());
        assert_eq!(boxes.len(), 1);
        let q = boxes[0].0;
        let angle = (q[1][1] - q[0][1]).atan2(q[1][0] - q[0][0]).to_degrees();
        assert!((angle - 30.0).abs() <= 2.0, "{angle}");
    }

    #[test]
    fn quads_scale_to_source() {
        let prob = map_with(100, 60, |x, y| if (20..60).contains(&x) && (20..30).contains(&y) { 0.9 } else { 0.0 });
        let boxes = boxes_from_map(&prob, 100, 60, 1000, 600, &OcrDetSpec::default());
        assert_eq!(boxes.len(), 1);
        // Clipper rounds the unclipped box in map pixels (15..64 × 15..34) before it is scaled.
        close(&boxes[0].0, [[150.0, 150.0], [640.0, 150.0], [640.0, 340.0], [150.0, 340.0]], 0.01);
    }

    #[test]
    fn reading_order_is_rows_then_columns() {
        let at = |x: f32, y: f32| ([[x, y], [x + 50.0, y], [x + 50.0, y + 10.0], [x, y + 10.0]], 0.9);
        let mut boxes = vec![at(100.0, 50.0), at(10.0, 55.0), at(10.0, 200.0)];
        reading_order(&mut boxes);
        let tops: Vec<_> = boxes.iter().map(|b| b.0[0]).collect();
        assert_eq!(tops, [[10.0, 55.0], [100.0, 50.0], [10.0, 200.0]]);
    }

    #[test]
    fn round_offset_grows_by_d_with_integer_vertices() {
        // A 10 × 4 box grown by 2.3: edges at round(−2.3) = −2 and round(12.3) = 12, corners
        // rounded off (no vertex near the grown rect's corner), either winding.
        for poly in [[[0.0, 0.0], [10.0, 0.0], [10.0, 4.0], [0.0, 4.0]], [[0.0, 4.0], [10.0, 4.0], [10.0, 0.0], [0.0, 0.0]]] {
            let out = round_offset(&poly, 2.3);
            assert!(out.iter().all(|p| p[0].fract() == 0.0 && p[1].fract() == 0.0), "{out:?}");
            let fold = |i: usize, f: fn(f64, f64) -> f64| out.iter().map(|p| p[i]).reduce(f).unwrap();
            assert_eq!((fold(0, f64::min), fold(0, f64::max), fold(1, f64::min), fold(1, f64::max)), (-2.0, 12.0, -2.0, 6.0));
            assert!(!out.contains(&[-2.0, -2.0]), "round join, not a mitre");
        }
    }

    #[test]
    fn crop_box_is_the_truncated_quads_min_rect_from_top_left() {
        // An axis-aligned quad with fractional corners: truncated, same order.
        let q = [[10.7, 20.2], [60.9, 20.2], [60.9, 30.8], [10.7, 30.8]];
        close(&crop_box(&q), [[10.0, 20.0], [60.0, 20.0], [60.0, 30.0], [10.0, 30.0]], 1e-3);
        // A slanted line starting at a corner other than top-left comes back top-left first.
        let q = [[1302.0, 572.0], [1167.0, 588.0], [1164.0, 564.0], [1299.0, 547.0]];
        let b = crop_box(&q);
        assert!(b[0][0] < b[1][0] && b[0][1] < b[3][1], "{b:?}");
    }
}
