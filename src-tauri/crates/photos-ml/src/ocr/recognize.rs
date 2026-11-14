//! Text recognition (Round 13): PaddleOCR's `get_rotate_crop_image`, `resize_norm_img` and
//! `CTCLabelDecode`, ported.

use super::{resize_bilinear, Quad};
use crate::manifest::OcrRecSpec;

/// A text line cut out of the image, upright, packed RGB.
#[derive(Debug, Clone)]
pub struct Crop {
    pub rgb: Vec<u8>,
    pub w: u32,
    pub h: u32,
}

/// PaddleOCR's `get_rotate_crop_image`: the quad warped to a `w × h` rectangle (sides
/// truncated like `int()`), bicubic with edges replicated; turned 90° counter-clockwise when
/// at least 1.5 times taller than wide.
pub fn crop_quad(rgb: &[u8], w: u32, h: u32, q: &Quad) -> Crop {
    let d = |a: [f32; 2], b: [f32; 2]| ((b[0] - a[0]) as f64).hypot((b[1] - a[1]) as f64);
    let cw = (d(q[0], q[1]).max(d(q[2], q[3])) as u32).max(1);
    let ch = (d(q[0], q[3]).max(d(q[1], q[2])) as u32).max(1);
    let to = [[0.0, 0.0], [cw as f64, 0.0], [cw as f64, ch as f64], [0.0, ch as f64]];
    let from = q.map(|p| [p[0] as f64, p[1] as f64]);
    let Some(m) = homography(&to, &from) else {
        return Crop { rgb: vec![0; (cw * ch * 3) as usize], w: cw, h: ch };
    };
    let mut out = Vec::with_capacity((cw * ch * 3) as usize);
    for y in 0..ch {
        for x in 0..cw {
            let (x, y) = (x as f64, y as f64);
            let z = m[6] * x + m[7] * y + 1.0;
            let (sx, sy) = ((m[0] * x + m[1] * y + m[2]) / z, (m[3] * x + m[4] * y + m[5]) / z);
            out.extend(bicubic(rgb, w, h, sx, sy));
        }
    }
    let crop = Crop { rgb: out, w: cw, h: ch };
    if ch as f64 >= 1.5 * cw as f64 { rot90_ccw(&crop) } else { crop }
}

/// PaddleX 3 never feeds the recogniser wider than this; a longer line is squeezed into it.
const MAX_REC_WIDTH: u32 = 3200;

/// PaddleX 3's `OCRReisizeNormImg.resize` for one crop: (resized width, padded width). The
/// padded width is `floor(height · max(base_width/height, w/h))`; the crop is resized to
/// `ceil(height · w/h)` unless that is wider, then to the padded width.
fn rec_widths(c: &Crop, spec: &OcrRecSpec) -> (u32, u32) {
    let hh = spec.height as f64;
    let r = c.w as f64 / c.h as f64;
    let padded = (hh * r.max(spec.base_width as f64 / hh)) as u32;
    if padded > MAX_REC_WIDTH {
        return (MAX_REC_WIDTH, MAX_REC_WIDTH);
    }
    let rw = (hh * r).ceil() as u32;
    (if rw > padded { padded } else { rw }.max(1), padded)
}

/// The recogniser's input for a batch, as PaddleX 3 builds it: each crop resized to
/// `spec.height` on its own ([`rec_widths`]), BGR, `(x/255 − 0.5)/0.5`, right-padded with 0.0
/// to the widest crop's padded width. Returns the NCHW tensor and that width.
pub fn rec_batch(crops: &[&Crop], spec: &OcrRecSpec) -> (Vec<f32>, u32) {
    let width = crops.iter().map(|c| rec_widths(c, spec).1).max().unwrap_or(spec.base_width);
    let (h, plane) = (spec.height, (spec.height * width) as usize);
    let mut t = vec![0f32; crops.len() * 3 * plane];
    for (n, c) in crops.iter().enumerate() {
        let rw = rec_widths(c, spec).0;
        let small = resize_bilinear(&c.rgb, c.w, c.h, rw, h);
        let base = n * 3 * plane;
        for (i, px) in small.as_chunks::<3>().0.iter().enumerate() {
            let (y, x) = (i / rw as usize, i % rw as usize);
            for ch in 0..3 {
                t[base + ch * plane + y * width as usize + x] = (px[2 - ch] as f32 / 255.0 - 0.5) / 0.5;
            }
        }
    }
    (t, width)
}

/// CTC greedy decoding over `steps × classes` probabilities, classes `blank, chars…, " "`:
/// a step counts when it is not blank and differs from the step before; confidence is the
/// mean of the counted steps' best probability (0.0 when none counts).
pub fn ctc_greedy(probs: &[f32], steps: usize, classes: usize, chars: &[String]) -> (String, f32) {
    let (mut text, mut sum, mut n, mut prev) = (String::new(), 0f32, 0u32, usize::MAX);
    for row in probs.chunks(classes).take(steps) {
        let (best, p) = row.iter().enumerate().fold((0, f32::MIN), |b, (i, &p)| if p > b.1 { (i, p) } else { b });
        if best != 0 && best != prev {
            text.push_str(chars.get(best - 1).map_or(" ", String::as_str));
            sum += p;
            n += 1;
        }
        prev = best;
    }
    (text, if n == 0 { 0.0 } else { sum / n as f32 })
}

/// `cv2.getPerspectiveTransform` from `src` to `dst` (8 unknowns, h33 = 1).
fn homography(src: &[[f64; 2]; 4], dst: &[[f64; 2]; 4]) -> Option<[f64; 8]> {
    let mut a = [[0f64; 9]; 8];
    for i in 0..4 {
        let ([x, y], [u, v]) = (src[i], dst[i]);
        a[2 * i] = [x, y, 1.0, 0.0, 0.0, 0.0, -x * u, -y * u, u];
        a[2 * i + 1] = [0.0, 0.0, 0.0, x, y, 1.0, -x * v, -y * v, v];
    }
    for col in 0..8 {
        let pivot = (col..8).max_by(|&i, &j| a[i][col].abs().total_cmp(&a[j][col].abs()))?;
        if a[pivot][col].abs() < 1e-12 {
            return None;
        }
        a.swap(col, pivot);
        let p = a[col];
        for (r, row) in a.iter_mut().enumerate() {
            if r != col {
                let f = row[col] / p[col];
                for (v, pv) in row.iter_mut().zip(p).skip(col) {
                    *v -= f * pv;
                }
            }
        }
    }
    Some(std::array::from_fn(|i| a[i][8] / a[i][i]))
}

/// OpenCV's bicubic (A = −0.75) at `(x, y)` with `BORDER_REPLICATE`.
fn bicubic(rgb: &[u8], w: u32, h: u32, x: f64, y: f64) -> [u8; 3] {
    const A: f64 = -0.75;
    let weights = |t: f64| {
        let w0 = ((A * (t + 1.0) - 5.0 * A) * (t + 1.0) + 8.0 * A) * (t + 1.0) - 4.0 * A;
        let w1 = ((A + 2.0) * t - (A + 3.0)) * t * t + 1.0;
        let w2 = ((A + 2.0) * (1.0 - t) - (A + 3.0)) * (1.0 - t) * (1.0 - t) + 1.0;
        [w0, w1, w2, 1.0 - w0 - w1 - w2]
    };
    let (x0, y0) = (x.floor(), y.floor());
    let (wx, wy) = (weights(x - x0), weights(y - y0));
    let clamp = |v: f64, hi: u32| (v as i64).clamp(0, hi as i64 - 1) as usize;
    let mut px = [0f64; 3];
    for (j, wyj) in wy.iter().enumerate() {
        let yy = clamp(y0 - 1.0 + j as f64, h);
        for (i, wxi) in wx.iter().enumerate() {
            let xx = clamp(x0 - 1.0 + i as f64, w);
            let at = (yy * w as usize + xx) * 3;
            for (c, v) in px.iter_mut().enumerate() {
                *v += wyj * wxi * rgb[at + c] as f64;
            }
        }
    }
    px.map(|v| v.round().clamp(0.0, 255.0) as u8)
}

/// `np.rot90`: the top row becomes the left column, read bottom to top.
fn rot90_ccw(c: &Crop) -> Crop {
    let (w, h) = (c.h, c.w);
    let mut rgb = Vec::with_capacity(c.rgb.len());
    for y in 0..h {
        for x in 0..w {
            let at = ((x * c.w + (c.w - 1 - y)) * 3) as usize;
            rgb.extend_from_slice(&c.rgb[at..at + 3]);
        }
    }
    Crop { rgb, w, h }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::OcrRecSpec;

    fn image(w: u32, h: u32, f: impl Fn(u32, u32) -> [u8; 3]) -> Vec<u8> {
        (0..h).flat_map(|y| (0..w).map(move |x| (x, y))).flat_map(|(x, y)| f(x, y)).collect()
    }

    fn spec(chars: &[&str]) -> OcrRecSpec {
        OcrRecSpec { height: 48, base_width: 320, chars: chars.iter().map(|c| c.to_string()).collect() }
    }

    /// One row of `classes` probabilities per step, `p` on the argmax.
    fn steps(argmax: &[usize], p: &[f32], classes: usize) -> Vec<f32> {
        argmax.iter().zip(p).flat_map(|(&a, &p)| (0..classes).map(move |c| if c == a { p } else { 0.0 })).collect()
    }

    #[test]
    fn axis_aligned_quad_crops_exactly() {
        let img = image(10, 10, |x, y| if (3..7).contains(&x) && (4..6).contains(&y) { [255; 3] } else { [0; 3] });
        let c = crop_quad(&img, 10, 10, &[[3.0, 4.0], [7.0, 4.0], [7.0, 6.0], [3.0, 6.0]]);
        assert_eq!((c.w, c.h), (4, 2));
        assert!(c.rgb.iter().all(|&v| v == 255), "{:?}", c.rgb);
    }

    #[test]
    fn tall_crops_are_rotated_ccw() {
        let img = image(12, 32, |_, y| if y == 1 { [255, 0, 0] } else { [0, 0, 255] });
        let c = crop_quad(&img, 12, 32, &[[1.0, 1.0], [11.0, 1.0], [11.0, 31.0], [1.0, 31.0]]);
        assert_eq!((c.w, c.h), (30, 10));
        for y in 0..10 {
            let i = (y * 30 * 3) as usize;
            assert_eq!(&c.rgb[i..i + 3], &[255, 0, 0], "row {y} of the left column");
        }
        assert_eq!(&c.rgb[5 * 90 + 15 * 3..5 * 90 + 15 * 3 + 3], &[0, 0, 255]);
    }

    #[test]
    fn batch_pads_to_the_widest() {
        let a = Crop { rgb: vec![128; 96 * 48 * 3], w: 96, h: 48 };
        let b = Crop { rgb: vec![128; 480 * 48 * 3], w: 480, h: 48 };
        let (t, w) = rec_batch(&[&a, &b], &spec(&["a"]));
        assert_eq!(w, 480);
        assert_eq!(t.len(), 2 * 3 * 48 * 480);
        assert_eq!(t[200], 0.0, "padding");
        assert!((t[50] - (128.0 / 255.0 - 0.5) / 0.5).abs() < 1e-6);
        assert!((t[3 * 48 * 480 + 400] - (128.0 / 255.0 - 0.5) / 0.5).abs() < 1e-6);
    }

    #[test]
    fn each_crop_is_sized_on_its_own_like_paddlex() {
        // 481×50: 48·9.62 = 461.76 → padded to 461, so resized to 461 (not ceil = 462), even
        // in a batch with a wider crop that leaves room for 462.
        let a = Crop { rgb: vec![255; 481 * 50 * 3], w: 481, h: 50 };
        let b = Crop { rgb: vec![255; 960 * 48 * 3], w: 960, h: 48 };
        let (t, w) = rec_batch(&[&a, &b], &spec(&["a"]));
        assert_eq!(w, 960);
        assert_eq!((t[460], t[461]), (1.0, 0.0));
        // Wider than 3200 is squeezed into 3200.
        let long = Crop { rgb: vec![255; 4000 * 40 * 3], w: 4000, h: 40 };
        assert_eq!(rec_batch(&[&long], &spec(&["a"])).1, 3200);
    }

    #[test]
    fn ctc_collapses_repeats_and_blanks() {
        let chars = spec(&["a", "b"]).chars;
        let probs = steps(&[1, 1, 0, 2, 2, 0, 2], &[0.9, 0.8, 0.99, 0.7, 0.6, 0.99, 0.5], 4);
        let (text, conf) = ctc_greedy(&probs, 7, 4, &chars);
        assert_eq!(text, "abb");
        assert!((conf - (0.9 + 0.7 + 0.5) / 3.0).abs() < 1e-6);
        assert_eq!(ctc_greedy(&steps(&[3], &[0.8], 4), 1, 4, &chars).0, " ");
        assert_eq!(ctc_greedy(&steps(&[0, 0], &[0.8, 0.9], 4), 2, 4, &chars), (String::new(), 0.0));
    }

    #[test]
    fn ctc_reads_diacritics() {
        let chars = spec(&["č", "ć", "š", "ž", "đ"]).chars;
        let (text, _) = ctc_greedy(&steps(&[1, 0, 5], &[0.9, 0.9, 0.9], 7), 3, 7, &chars);
        assert_eq!(text, "čđ");
    }
}
