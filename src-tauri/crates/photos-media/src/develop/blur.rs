//! A Gaussian blur of one channel by three box passes per axis (Kutskir's box sizes), linear
//! in the radius: a clarity blur on a 45 MP export costs what a small one does.

use rayon::prelude::*;

/// Box widths whose three passes approximate a Gaussian of `sigma`.
pub fn box_sizes(sigma: f32) -> [usize; 3] {
    let n = 3.0f32;
    let ideal = (12.0 * sigma * sigma / n + 1.0).sqrt();
    let mut wl = ideal.floor() as i64;
    if wl % 2 == 0 {
        wl -= 1;
    }
    let wl = wl.max(1);
    let wu = wl + 2;
    let wlf = wl as f32;
    let m_ideal = (12.0 * sigma * sigma - n * wlf * wlf - 4.0 * n * wlf - 3.0 * n) / (-4.0 * wlf - 4.0);
    let m = m_ideal.round() as i64;
    std::array::from_fn(|i| if (i as i64) < m { wl as usize } else { wu as usize })
}

/// One box pass along rows, edges clamped.
fn box_rows(src: &[f32], dst: &mut [f32], w: usize, r: usize) {
    if r == 0 {
        dst.copy_from_slice(src);
        return;
    }
    let norm = 1.0 / (2 * r + 1) as f32;
    dst.par_chunks_mut(w).zip(src.par_chunks(w)).for_each(|(out, row)| {
        let at = |i: isize| row[i.clamp(0, w as isize - 1) as usize];
        let mut acc: f32 = (-(r as isize)..=r as isize).map(at).sum();
        for (x, o) in out.iter_mut().enumerate() {
            *o = acc * norm;
            acc += at(x as isize + r as isize + 1) - at(x as isize - r as isize);
        }
    });
}

/// Columns handled by one task of a vertical pass: wide enough that every row it touches is
/// a contiguous run, narrow enough that the image splits across the cores.
const STRIP: usize = 64;

/// Where a vertical pass writes. Each task owns a distinct strip of columns, so no element is
/// written by two tasks and none is read while written.
struct Columns(*mut f32);
unsafe impl Sync for Columns {}
unsafe impl Send for Columns {}

/// One box pass along columns, edges clamped: a running sum per column, walking the rows in
/// order so memory is read row-contiguous (a transpose would scatter it).
fn box_cols(src: &[f32], dst: &mut [f32], w: usize, h: usize, r: usize) {
    if r == 0 {
        dst.copy_from_slice(src);
        return;
    }
    let norm = 1.0 / (2 * r + 1) as f32;
    let out = Columns(dst.as_mut_ptr());
    let out = &out;
    (0..w.div_ceil(STRIP)).into_par_iter().for_each(|strip| {
        let (x0, x1) = (strip * STRIP, ((strip + 1) * STRIP).min(w));
        let at = |y: isize, x: usize| src[y.clamp(0, h as isize - 1) as usize * w + x];
        let mut acc: Vec<f32> = (x0..x1).map(|x| (-(r as isize)..=r as isize).map(|y| at(y, x)).sum()).collect();
        for y in 0..h {
            for (i, x) in (x0..x1).enumerate() {
                // SAFETY: (y, x) lies in this task's own strip of columns and inside `dst`.
                unsafe { *out.0.add(y * w + x) = acc[i] * norm };
                acc[i] += at(y as isize + r as isize + 1, x) - at(y as isize - r as isize, x);
            }
        }
    });
}

/// Blur a `w × h` channel. `sigma < 0.5` returns it unchanged.
pub fn gaussian(ch: &[f32], w: usize, h: usize, sigma: f32) -> Vec<f32> {
    if sigma < 0.5 || w == 0 || h == 0 {
        return ch.to_vec();
    }
    let sizes = box_sizes(sigma);
    let mut a = ch.to_vec();
    let mut b = vec![0f32; a.len()];
    for s in sizes {
        box_rows(&a, &mut b, w, (s - 1) / 2);
        std::mem::swap(&mut a, &mut b);
    }
    for s in sizes {
        box_cols(&a, &mut b, w, h, (s - 1) / 2);
        std::mem::swap(&mut a, &mut b);
    }
    a
}

/// A wide blur computed on a grid `σ/4` coarser and read back bilinearly: the same answer
/// to within what a wide Gaussian leaves of the fine grid, at a fraction of the cost.
pub fn gaussian_coarse(ch: &[f32], w: usize, h: usize, sigma: f32) -> Vec<f32> {
    let f = ((sigma / 4.0).floor() as usize).max(1);
    if f == 1 || w < 2 * f || h < 2 * f {
        return gaussian(ch, w, h, sigma);
    }
    let (gw, gh) = (w.div_ceil(f), h.div_ceil(f));
    let mut small = vec![0f32; gw * gh];
    small.par_chunks_mut(gw).enumerate().for_each(|(gy, row)| {
        for (gx, o) in row.iter_mut().enumerate() {
            let (mut acc, mut n) = (0f32, 0u32);
            for y in gy * f..((gy + 1) * f).min(h) {
                for x in gx * f..((gx + 1) * f).min(w) {
                    acc += ch[y * w + x];
                    n += 1;
                }
            }
            *o = acc / n as f32;
        }
    });
    let blurred = gaussian(&small, gw, gh, sigma / f as f32);
    let mut out = vec![0f32; w * h];
    out.par_chunks_mut(w).enumerate().for_each(|(y, row)| {
        let fy = ((y as f32 + 0.5) / f as f32 - 0.5).clamp(0.0, (gh - 1) as f32);
        let (y0, ty) = (fy.floor() as usize, fy.fract());
        let y1 = (y0 + 1).min(gh - 1);
        for (x, o) in row.iter_mut().enumerate() {
            let fx = ((x as f32 + 0.5) / f as f32 - 0.5).clamp(0.0, (gw - 1) as f32);
            let (x0, tx) = (fx.floor() as usize, fx.fract());
            let x1 = (x0 + 1).min(gw - 1);
            let top = blurred[y0 * gw + x0] * (1.0 - tx) + blurred[y0 * gw + x1] * tx;
            let bot = blurred[y1 * gw + x0] * (1.0 - tx) + blurred[y1 * gw + x1] * tx;
            *o = top * (1.0 - ty) + bot * ty;
        }
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_constant_stays_constant() {
        let out = gaussian(&vec![0.4; 40 * 30], 40, 30, 5.0);
        assert!(out.iter().all(|v| (v - 0.4).abs() < 1e-5));
    }

    #[test]
    fn the_coarse_blur_agrees_with_the_fine_one() {
        let (w, h) = (400usize, 300usize);
        let ch: Vec<f32> = (0..w * h).map(|i| ((i % w) as f32 / 23.0).sin() + ((i / w) as f32 / 31.0).cos()).collect();
        let fine = gaussian(&ch, w, h, 20.0);
        let coarse = gaussian_coarse(&ch, w, h, 20.0);
        // Away from the borders, where the two clamp at different granularity.
        let m = 60;
        let worst = (m..h - m).flat_map(|y| (m..w - m).map(move |x| y * w + x)).map(|i| (fine[i] - coarse[i]).abs()).fold(0.0f32, f32::max);
        assert!(worst < 0.03, "{worst}");
    }

    #[test]
    fn small_sigma_is_the_identity() {
        let ch: Vec<f32> = (0..100).map(|i| i as f32).collect();
        assert_eq!(gaussian(&ch, 10, 10, 0.3), ch);
    }

    #[test]
    fn an_impulse_spreads_like_the_gaussian_asked_for() {
        let (w, h, sigma) = (201usize, 1usize, 8.0f32);
        let mut ch = vec![0f32; w * h];
        ch[100] = 1.0;
        // Along one axis only: blur a single row by passing its transposed twin through rows.
        let out = gaussian(&ch, w, h, sigma);
        let mass: f32 = out.iter().sum();
        let var: f32 = out.iter().enumerate().map(|(i, v)| v * (i as f32 - 100.0).powi(2)).sum::<f32>() / mass;
        assert!((mass - 1.0).abs() < 1e-3);
        assert!((var.sqrt() / sigma - 1.0).abs() < 0.05, "σ measured {}", var.sqrt());
    }
}
