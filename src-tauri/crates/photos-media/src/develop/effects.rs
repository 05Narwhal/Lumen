//! Effects on the cropped frame: motion blur, glow, halation, a vignette and film grain.

use photos_core::edits::{Glow, Grain, Halation, MotionBlur, Vignette};
use rayon::prelude::*;

use super::blur::{gaussian, gaussian_coarse};
use super::linear::luma;
use super::{Ctx, Linear};

fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// How much of the vignette a point gets, at distance `r` from the centre where 1 is the
/// middle of each edge (so the corners are √2).
pub fn vignette_weight(v: &Vignette, r: f32) -> f32 {
    let start = 0.3 + 0.9 * v.midpoint;
    let width = 0.1 + 1.0 * v.feather;
    smoothstep(start, start + width, r)
}

pub fn vignette(img: &mut Linear, v: &Vignette, ctx: &Ctx) {
    if v.amount == 0.0 {
        return;
    }
    let (ox, oy, fw, fh) = ctx.frame(img);
    let (w, h) = (fw as f32, fh as f32);
    let width = img.width as usize;
    img.data.par_chunks_mut(width).enumerate().for_each(|(y, row)| {
        let dy = ((y as u32 + oy) as f32 + 0.5 - h / 2.0) / (h / 2.0);
        for (x, p) in row.iter_mut().enumerate() {
            let dx = ((x as u32 + ox) as f32 + 0.5 - w / 2.0) / (w / 2.0);
            let f = vignette_weight(v, dx.hypot(dy)) * v.amount.abs();
            *p = if v.amount < 0.0 {
                [p[0] * (1.0 - f), p[1] * (1.0 - f), p[2] * (1.0 - f)]
            } else {
                std::array::from_fn(|c| p[c] + (1.0 - p[c]).max(0.0) * f)
            };
        }
    });
}

/// Bilinear read with the edges held, so a streak leaving the frame repeats its edge.
#[inline]
fn sample(img: &Linear, x: f32, y: f32) -> [f32; 3] {
    let (w, h) = (img.width as usize, img.height as usize);
    let x = x.clamp(0.0, (w - 1) as f32);
    let y = y.clamp(0.0, (h - 1) as f32);
    let (x0, y0) = (x.floor() as usize, y.floor() as usize);
    let (x1, y1) = ((x0 + 1).min(w - 1), (y0 + 1).min(h - 1));
    let (tx, ty) = (x - x0 as f32, y - y0 as f32);
    let d = &img.data;
    std::array::from_fn(|c| {
        let top = d[y0 * w + x0][c] * (1.0 - tx) + d[y0 * w + x1][c] * tx;
        let bot = d[y1 * w + x0][c] * (1.0 - tx) + d[y1 * w + x1][c] * tx;
        top * (1.0 - ty) + bot * ty
    })
}

/// The streak's length in render pixels for a frame whose long edge is `long`.
pub fn motion_blur_length(m: &MotionBlur, long: f32) -> f32 {
    m.amount.clamp(0.0, 1.0) * 0.06 * long
}

/// Motion blur: each pixel the mean of a line of samples through it along the angle, the
/// line sized by the frame so a preview streaks as far as its export.
pub fn motion_blur(img: &mut Linear, m: &MotionBlur, ctx: &Ctx) {
    let length = motion_blur_length(m, ctx.frame_long(img));
    if length < 1.0 || img.width < 2 || img.height < 2 {
        return;
    }
    let n = (length.ceil() as usize + 1).clamp(3, 96);
    let (sin, cos) = m.angle.to_radians().sin_cos();
    // Image rows grow downwards: a positive angle rises to the right.
    let (dx, dy) = (cos * length, -sin * length);
    let src = img.clone();
    let width = img.width as usize;
    img.data.par_chunks_mut(width).enumerate().for_each(|(y, row)| {
        for (x, p) in row.iter_mut().enumerate() {
            let mut acc = [0f32; 3];
            for i in 0..n {
                let t = i as f32 / (n - 1) as f32 - 0.5;
                let s = sample(&src, x as f32 + dx * t, y as f32 + dy * t);
                for c in 0..3 {
                    acc[c] += s[c];
                }
            }
            *p = acc.map(|v| v / n as f32);
        }
    });
}

/// Motion blur that reads only what a mask holds: `weights` (one per pixel of `img`) is also
/// the source weight, so each pixel is `K(img·w) / K(w)` with `K` the streak, and what lies
/// outside the mask is never dragged in. Where the streak finds no weight the pixel is kept.
pub fn motion_blur_weighted(img: &mut Linear, m: &MotionBlur, ctx: &Ctx, weights: &[f32]) {
    let length = motion_blur_length(m, ctx.frame_long(img));
    if length < 1.0 || img.width < 2 || img.height < 2 {
        return;
    }
    let n = (length.ceil() as usize + 1).clamp(3, 96);
    let (sin, cos) = m.angle.to_radians().sin_cos();
    let (dx, dy) = (cos * length, -sin * length);
    let mut pre = img.clone();
    pre.data.par_iter_mut().zip(weights.par_iter()).for_each(|(p, w)| *p = p.map(|v| v * w));
    let mut wimg = img.clone();
    wimg.data.par_iter_mut().zip(weights.par_iter()).for_each(|(p, w)| *p = [*w; 3]);
    let width = img.width as usize;
    img.data.par_chunks_mut(width).enumerate().for_each(|(y, row)| {
        for (x, p) in row.iter_mut().enumerate() {
            let (mut acc, mut wsum) = ([0f32; 3], 0f32);
            for i in 0..n {
                let t = i as f32 / (n - 1) as f32 - 0.5;
                let (sx, sy) = (x as f32 + dx * t, y as f32 + dy * t);
                let s = sample(&pre, sx, sy);
                for c in 0..3 {
                    acc[c] += s[c];
                }
                wsum += sample(&wimg, sx, sy)[0];
            }
            // Both sums carry the same 1/n, so it cancels.
            if wsum / n as f32 >= 1e-4 {
                *p = acc.map(|v| v / wsum);
            }
        }
    });
}

/// The highlights above `threshold` (linear), blurred by `sigma` render pixels, per channel.
/// With `weights`, a pixel gives light in proportion to its weight (a mask's light is its own).
fn bloom(img: &Linear, threshold: f32, sigma: f32, weights: Option<&[f32]>) -> [Vec<f32>; 3] {
    let (w, h) = (img.width as usize, img.height as usize);
    let mut over: Vec<f32> = img.data.par_iter().map(|p| smoothstep(threshold, threshold * 2.0, luma(*p))).collect();
    if let Some(wt) = weights {
        over.par_iter_mut().zip(wt.par_iter()).for_each(|(k, w)| *k *= w);
    }
    std::array::from_fn(|c| {
        let ch: Vec<f32> = img.data.par_iter().zip(over.par_iter()).map(|(p, k)| p[c].max(0.0) * k).collect();
        if sigma > 8.0 {
            gaussian_coarse(&ch, w, h, sigma)
        } else {
            gaussian(&ch, w, h, sigma)
        }
    })
}

/// Glow's blur in render pixels for a frame whose long edge is `long`.
pub fn glow_sigma(g: &Glow, long: f32) -> f32 {
    (0.004 + 0.03 * g.size.clamp(0.0, 1.0)) * long
}

/// Halation's blur in render pixels for a frame whose long edge is `long`.
pub fn halation_sigma(hl: &Halation, long: f32) -> f32 {
    (0.002 + 0.015 * hl.size.clamp(0.0, 1.0)) * long
}

/// Glow: the bright parts, blurred and added back, so lights bloom softly as through a
/// diffusion filter.
pub fn glow(img: &mut Linear, g: &Glow, ctx: &Ctx) {
    glow_weighted(img, g, ctx, None);
}

/// [`glow`] whose light comes only from what `weights` holds (a mask's pixels).
pub fn glow_weighted(img: &mut Linear, g: &Glow, ctx: &Ctx, weights: Option<&[f32]>) {
    if g.amount <= 0.0 {
        return;
    }
    let sigma = glow_sigma(g, ctx.frame_long(img));
    let b = bloom(img, 0.35, sigma, weights);
    let k = g.amount.clamp(0.0, 1.0) * 1.2;
    img.data.par_iter_mut().enumerate().for_each(|(i, p)| *p = std::array::from_fn(|c| p[c] + b[c][i] * k));
}

/// Halation: light scattered back through the film base glows red-orange round the
/// brightest highlights, and only there.
pub fn halation(img: &mut Linear, hl: &Halation, ctx: &Ctx) {
    halation_weighted(img, hl, ctx, None);
}

/// [`halation`] whose light comes only from what `weights` holds (a mask's pixels).
pub fn halation_weighted(img: &mut Linear, hl: &Halation, ctx: &Ctx, weights: Option<&[f32]>) {
    if hl.amount <= 0.0 {
        return;
    }
    let sigma = halation_sigma(hl, ctx.frame_long(img));
    let b = bloom(img, 0.6, sigma, weights);
    let k = hl.amount.clamp(0.0, 1.0) * 1.5;
    const TINT: [f32; 3] = [1.0, 0.28, 0.06];
    img.data.par_iter_mut().enumerate().for_each(|(i, p)| {
        let lum = (b[0][i] + b[1][i] + b[2][i]) / 3.0;
        *p = std::array::from_fn(|c| p[c] + lum * TINT[c] * k);
    });
}

/// A value in [−1, 1] for a lattice point: a hash of the seed and the coordinates.
#[inline]
fn noise(seed: u64, x: i64, y: i64) -> f32 {
    let mut z = seed ^ (x as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ (y as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^= z >> 31;
    (z >> 40) as f32 / (1u64 << 23) as f32 - 1.0
}

/// Grain: value noise on a lattice of `0.5 + 3·size` full-resolution pixels, read bilinearly,
/// strongest in the midtones. Where a lattice cell is smaller than a rendered pixel the noise
/// is quietened as averaging would, so a preview's grain looks like its export's.
pub fn grain(img: &mut Linear, g: &Grain, ctx: &Ctx) {
    if g.amount == 0.0 {
        return;
    }
    let cell = (0.5 + 3.0 * g.size) * ctx.scale;
    let (step, gain) = if cell >= 1.0 { (cell, 1.0) } else { (1.0, cell) };
    let amp = g.amount * 0.16 * gain;
    let seed = ctx.seed;
    let (ox, oy, _, _) = ctx.frame(img);
    let width = img.width as usize;
    img.data.par_chunks_mut(width).enumerate().for_each(|(y, row)| {
        let fy = (y as u32 + oy) as f32 / step;
        let (y0, ty) = (fy.floor() as i64, fy.fract());
        for (x, p) in row.iter_mut().enumerate() {
            let fx = (x as u32 + ox) as f32 / step;
            let (x0, tx) = (fx.floor() as i64, fx.fract());
            let top = noise(seed, x0, y0) * (1.0 - tx) + noise(seed, x0 + 1, y0) * tx;
            let bot = noise(seed, x0, y0 + 1) * (1.0 - tx) + noise(seed, x0 + 1, y0 + 1) * tx;
            let n = top * (1.0 - ty) + bot * ty;
            // Strongest around middle grey, fading in deep shadow and bright highlight.
            let l = luma(*p).max(1e-5);
            let s = (l / 0.18).log2();
            let mid = (-0.5 * (s / 2.0) * (s / 2.0)).exp();
            let k = 1.0 + amp * n * mid;
            *p = [p[0] * k, p[1] * k, p[2] * k];
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_dark_vignette_darkens_corners_not_the_centre() {
        let mut img = Linear::new(100, 60, [0.5; 3]);
        vignette(&mut img, &Vignette { amount: -1.0, midpoint: 0.5, feather: 0.5 }, &Ctx::default());
        assert!((img.get(50, 30)[0] - 0.5).abs() < 1e-4);
        assert!(img.get(0, 0)[0] < 0.2);
        let mut light = Linear::new(100, 60, [0.5; 3]);
        vignette(&mut light, &Vignette { amount: 0.8, midpoint: 0.5, feather: 0.5 }, &Ctx::default());
        assert!(light.get(0, 0)[0] > 0.8);
    }

    #[test]
    fn motion_blur_streaks_along_its_angle_only() {
        // A bright vertical line: a horizontal blur spreads it sideways, a vertical one not.
        let line = || {
            let mut img = Linear::new(100, 40, [0.0; 3]);
            for y in 0..40 {
                img.data[y * 100 + 50] = [1.0; 3];
            }
            img
        };
        let mut h = line();
        motion_blur(&mut h, &MotionBlur { amount: 1.0, angle: 0.0 }, &Ctx::default());
        assert!(h.get(52, 20)[0] > 0.02);
        assert!(h.get(50, 20)[0] < 0.5);
        let mut v = line();
        motion_blur(&mut v, &MotionBlur { amount: 1.0, angle: 90.0 }, &Ctx::default());
        assert!(v.get(52, 20)[0] < 1e-4);
        assert!((v.get(50, 20)[0] - 1.0).abs() < 1e-3);
        let mut off = line();
        motion_blur(&mut off, &MotionBlur::default(), &Ctx::default());
        assert_eq!(off, line());
    }

    #[test]
    fn glow_and_halation_light_round_highlights_and_leave_shadows() {
        let spot = || {
            let mut img = Linear::new(80, 80, [0.02; 3]);
            for y in 36..44 {
                for x in 36..44 {
                    img.data[y * 80 + x] = [4.0; 3];
                }
            }
            img
        };
        let mut g = spot();
        glow(&mut g, &Glow { amount: 1.0, size: 0.5 }, &Ctx::default());
        assert!(g.get(45, 40)[0] > 0.05, "{:?}", g.get(45, 40));
        assert!((g.get(2, 2)[0] - 0.02).abs() < 0.01);
        let mut h = spot();
        halation(&mut h, &Halation { amount: 1.0, size: 0.5 }, &Ctx::default());
        let near = h.get(44, 40);
        assert!(near[0] > near[1] && near[1] > near[2], "{near:?}");
    }

    #[test]
    fn grain_is_the_same_for_a_seed_and_differs_for_another() {
        let g = Grain { amount: 0.8, size: 0.3 };
        let render = |seed| {
            let mut img = Linear::new(64, 64, [0.18; 3]);
            grain(&mut img, &g, &Ctx::new(1.0, seed));
            img
        };
        assert_eq!(render(3), render(3));
        assert_ne!(render(3), render(4));
        let img = render(3);
        let spread = img.data.iter().map(|p| (p[0] - 0.18).abs()).fold(0.0f32, f32::max);
        assert!(spread > 0.005 && spread < 0.05, "{spread}");
    }
}
