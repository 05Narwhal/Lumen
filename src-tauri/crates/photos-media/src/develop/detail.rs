//! Sharpening: an unsharp mask on luminance. The radius is in full-resolution pixels (scaled
//! by `Ctx::scale` for a preview); `detail` lets fine texture through where 0 sharpens edges
//! only, with haloes held back; `masking` leaves flat areas alone.

use photos_core::edits::Sharpen;
use rayon::prelude::*;

use super::blur::gaussian;
use super::linear::luma;
use super::{Ctx, Linear};

fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0).max(1e-9)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

pub fn sharpen(img: &mut Linear, s: &Sharpen, ctx: &Ctx) {
    sharpen_with(img, s, ctx, None);
}

/// `sharpen` with masks' local sharpness: `extra[i]` is added to the amount at pixel `i`.
pub fn sharpen_with(img: &mut Linear, s: &Sharpen, ctx: &Ctx, extra: Option<&[f32]>) {
    if s.amount == 0.0 && extra.is_none() {
        return;
    }
    let sigma = s.radius * ctx.scale;
    if sigma < 0.5 {
        // Finer than a preview pixel: the export sharpens, the preview cannot show it.
        return;
    }
    let (w, h) = (img.width as usize, img.height as usize);
    let y: Vec<f32> = img.data.par_iter().map(|p| luma(*p)).collect();
    let blurred = gaussian(&y, w, h, sigma);
    // How far a difference may go before it is treated as an edge's halo, relative to the
    // local brightness: small at detail 0, unbounded at detail 1.
    let halo = 0.02 + 2.0 * s.detail * s.detail;
    // The masking threshold, in stops per full-resolution pixel of the blurred luminance.
    let threshold = 0.04 * s.masking;
    let log_b: Vec<f32> = blurred.par_iter().map(|v| v.max(1e-5).log2()).collect();
    img.data.par_iter_mut().enumerate().for_each(|(i, p)| {
        let (x, yy) = (i % w, i / w);
        let base = blurred[i].max(1e-5);
        let d = y[i] - blurred[i];
        let limited = d / (1.0 + d.abs() / (halo * base));
        let mask = if threshold > 0.0 {
            let gx = log_b[yy * w + (x + 1).min(w - 1)] - log_b[yy * w + x.saturating_sub(1)];
            let gy = log_b[(yy + 1).min(h - 1) * w + x] - log_b[yy.saturating_sub(1) * w + x];
            // Per rendered pixel, back to per full-resolution pixel.
            let g = 0.5 * gx.hypot(gy) * ctx.scale;
            smoothstep(threshold * 0.5, threshold, g)
        } else {
            1.0
        };
        let amount = match extra {
            Some(e) => (s.amount + e[i]).clamp(-1.0, 2.5),
            None => s.amount,
        };
        let target = (y[i] + amount * limited * mask).max(0.0);
        if y[i] > 1e-6 {
            let k = target / y[i];
            *p = [p[0] * k, p[1] * k, p[2] * k];
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn step(w: u32, h: u32) -> Linear {
        let mut img = Linear::new(w, h, [0.1; 3]);
        for y in 0..h {
            for x in w / 2..w {
                img.data[(y * w + x) as usize] = [0.4; 3];
            }
        }
        img
    }

    #[test]
    fn sharpening_raises_edge_contrast() {
        let img = step(60, 20);
        let mut out = img.clone();
        sharpen(&mut out, &Sharpen { amount: 1.0, radius: 1.5, detail: 0.5, masking: 0.0 }, &Ctx::default());
        let row = |l: &Linear, x: u32| l.get(x, 10)[0];
        assert!(row(&out, 29) < row(&img, 29) && row(&out, 30) > row(&img, 30));
        assert!((row(&out, 5) - 0.1).abs() < 1e-4, "far from the edge nothing moves");
    }

    #[test]
    fn masking_leaves_flat_noise_alone() {
        let mut img = Linear::new(80, 80, [0.2; 3]);
        let mut seed = 1u32;
        for p in img.data.iter_mut() {
            seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12_345);
            let n = 0.2 * (1.0 + 0.01 * ((seed >> 16) as f32 / 32_768.0 - 1.0));
            *p = [n; 3];
        }
        let mut out = img.clone();
        sharpen(&mut out, &Sharpen { amount: 1.5, radius: 1.0, detail: 1.0, masking: 1.0 }, &Ctx::default());
        let worst = img.data.iter().zip(&out.data).map(|(a, b)| (b[0] / a[0] - 1.0).abs()).fold(0.0f32, f32::max);
        assert!(worst < 0.005, "{worst}");
        let mut unmasked = img.clone();
        sharpen(&mut unmasked, &Sharpen { amount: 1.5, radius: 1.0, detail: 1.0, masking: 0.0 }, &Ctx::default());
        assert!(unmasked.data.iter().zip(&img.data).any(|(b, a)| (b[0] / a[0] - 1.0).abs() > 0.005));
    }

    #[test]
    fn a_radius_finer_than_a_preview_pixel_is_left_to_the_export() {
        let img = step(30, 10);
        let mut out = img.clone();
        sharpen(&mut out, &Sharpen { amount: 1.0, radius: 1.0, detail: 0.5, masking: 0.0 }, &Ctx::new(0.3, 0));
        assert_eq!(out, img);
    }
}
