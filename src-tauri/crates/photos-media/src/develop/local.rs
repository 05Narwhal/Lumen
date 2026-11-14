//! Stages that look at a pixel's neighbours: texture and clarity (local contrast at two
//! scales) and dehaze (a dark-channel estimate). Radii are fractions of the rendered frame's
//! long edge, so a preview and its export — cropped alike — see the same structure.

use rayon::prelude::*;

use super::blur::{gaussian, gaussian_coarse};
use super::linear::luma;
use super::{Ctx, Linear};

/// Blur radii as a fraction of the long edge.
pub const TEXTURE_SIGMA: f32 = 0.0015;
pub const CLARITY_SIGMA: f32 = 0.012;
/// The dark channel's neighbourhood, as a fraction of the long edge.
const DEHAZE_RADIUS: f32 = 0.01;

fn log_luma(img: &Linear) -> Vec<f32> {
    img.data.par_iter().map(|p| luma(*p).max(1e-5).log2()).collect()
}

/// Texture (fine) and clarity (coarse, midtones only) as a boost of the detail layer of
/// log luminance, applied as a ratio.
pub fn texture_clarity(img: &mut Linear, texture: f32, clarity: f32, ctx: &Ctx) {
    if texture == 0.0 && clarity == 0.0 {
        return;
    }
    let (w, h) = (img.width as usize, img.height as usize);
    let long = ctx.frame_long(img);
    let l = log_luma(img);
    let mut delta = vec![0f32; l.len()];
    if texture != 0.0 {
        let b = gaussian(&l, w, h, (TEXTURE_SIGMA * long).max(0.6));
        delta.par_iter_mut().zip(l.par_iter().zip(b.par_iter())).for_each(|(d, (v, bv))| *d += texture * 0.8 * (v - bv));
    }
    if clarity != 0.0 {
        let b = gaussian_coarse(&l, w, h, CLARITY_SIGMA * long);
        delta.par_iter_mut().zip(l.par_iter().zip(b.par_iter())).for_each(|(d, (v, bv))| {
            // Midtones: a bell over stops from middle grey (0.18 is log2 −2.47).
            let s = bv + 2.47;
            let mid = (-0.5 * (s / 2.5) * (s / 2.5)).exp();
            *d += clarity * 1.2 * mid * (v - bv);
        });
    }
    img.data.par_iter_mut().zip(delta.par_iter()).for_each(|(p, d)| {
        let k = 2f32.powf(*d);
        *p = [p[0] * k, p[1] * k, p[2] * k];
    });
}

/// The dark channel on a grid of `cell` pixels: each cell's minimum and mean colour.
struct Grid {
    w: usize,
    h: usize,
    min: Vec<f32>,
    mean: Vec<[f32; 3]>,
}

fn grid(img: &Linear, cell: usize) -> Grid {
    let (w, h) = (img.width as usize, img.height as usize);
    let (gw, gh) = (w.div_ceil(cell), h.div_ceil(cell));
    let rows: Vec<(Vec<f32>, Vec<[f32; 3]>)> = (0..gh)
        .into_par_iter()
        .map(|gy| {
            let mut min = vec![f32::INFINITY; gw];
            let mut sum = vec![[0f32; 3]; gw];
            let mut n = vec![0u32; gw];
            for y in gy * cell..((gy + 1) * cell).min(h) {
                for x in 0..w {
                    let p = img.data[y * w + x];
                    let g = x / cell;
                    min[g] = min[g].min(p[0].min(p[1]).min(p[2]));
                    sum[g] = [sum[g][0] + p[0], sum[g][1] + p[1], sum[g][2] + p[2]];
                    n[g] += 1;
                }
            }
            let mean = sum.iter().zip(&n).map(|(s, n)| s.map(|v| v / (*n).max(1) as f32)).collect();
            (min, mean)
        })
        .collect();
    let (min, mean): (Vec<Vec<f32>>, Vec<Vec<[f32; 3]>>) = rows.into_iter().unzip();
    Grid { w: gw, h: gh, min: min.concat(), mean: mean.concat() }
}

/// The mean colour of the haziest cells: the top 1 % by dark channel.
fn airlight_of(g: &Grid) -> [f32; 3] {
    let mut order: Vec<usize> = (0..g.min.len()).collect();
    order.sort_unstable_by(|a, b| g.min[*b].total_cmp(&g.min[*a]));
    let top = (g.min.len() / 100).max(1);
    let mut air = [0f32; 3];
    for &i in &order[..top] {
        for (a, v) in air.iter_mut().zip(g.mean[i]) {
            *a += v / top as f32;
        }
    }
    air
}

/// Dehaze's airlight for a whole frame, for rendering part of it later with `Ctx::airlight`.
pub fn airlight(img: &Linear) -> [f32; 3] {
    let cell = ((DEHAZE_RADIUS * img.long_edge() as f32).round() as usize).max(2);
    airlight_of(&grid(img, cell))
}

/// Each cell's minimum widened to its 3 x 3 neighbourhood, bilinearly read back per pixel.
fn dark_channel(g: &Grid, w: usize, h: usize, cell: usize) -> Vec<f32> {
    let (gw, gh) = (g.w, g.h);
    let wide: Vec<f32> = (0..gw * gh)
        .into_par_iter()
        .map(|i| {
            let (gx, gy) = ((i % gw) as isize, (i / gw) as isize);
            let mut m = f32::INFINITY;
            for dy in -1..=1 {
                for dx in -1..=1 {
                    let (x, y) = (gx + dx, gy + dy);
                    if x >= 0 && y >= 0 && (x as usize) < gw && (y as usize) < gh {
                        m = m.min(g.min[y as usize * gw + x as usize]);
                    }
                }
            }
            m
        })
        .collect();
    let mut out = vec![0f32; w * h];
    out.par_chunks_mut(w).enumerate().for_each(|(y, row)| {
        let fy = ((y as f32 + 0.5) / cell as f32 - 0.5).clamp(0.0, (gh - 1) as f32);
        let (y0, ty) = (fy.floor() as usize, fy.fract());
        let y1 = (y0 + 1).min(gh - 1);
        for (x, o) in row.iter_mut().enumerate() {
            let fx = ((x as f32 + 0.5) / cell as f32 - 0.5).clamp(0.0, (gw - 1) as f32);
            let (x0, tx) = (fx.floor() as usize, fx.fract());
            let x1 = (x0 + 1).min(gw - 1);
            let top = wide[y0 * gw + x0] * (1.0 - tx) + wide[y0 * gw + x1] * tx;
            let bot = wide[y1 * gw + x0] * (1.0 - tx) + wide[y1 * gw + x1] * tx;
            *o = top * (1.0 - ty) + bot * ty;
        }
    });
    out
}

/// Dehaze (He et al.'s dark-channel prior, simplified): the airlight is the mean colour of
/// the haziest cells (the top 1 % by dark channel); positive amounts divide the haze out,
/// negative ones mix toward it.
pub fn dehaze(img: &mut Linear, amount: f32, ctx: &Ctx) {
    if amount == 0.0 || img.data.is_empty() {
        return;
    }
    let (w, h) = (img.width as usize, img.height as usize);
    let cell = ((DEHAZE_RADIUS * ctx.frame_long(img)).round() as usize).max(2);
    let g = grid(img, cell);
    let air = ctx.airlight.unwrap_or_else(|| airlight_of(&g));
    let dark = dark_channel(&g, w, h, cell);
    let a_mean = ((air[0] + air[1] + air[2]) / 3.0).max(1e-4);
    img.data.par_iter_mut().zip(dark.par_iter()).for_each(|(p, d)| {
        if amount > 0.0 {
            let t = (1.0 - 0.95 * amount * (d / a_mean).clamp(0.0, 1.0)).max(0.1);
            *p = std::array::from_fn(|c| (p[c] - air[c]) / t + air[c]);
        } else {
            let k = -amount * 0.6;
            *p = std::array::from_fn(|c| p[c] + (air[c] - p[c]) * k);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn std_dev(v: &[f32]) -> f32 {
        let m = v.iter().sum::<f32>() / v.len() as f32;
        (v.iter().map(|x| (x - m) * (x - m)).sum::<f32>() / v.len() as f32).sqrt()
    }

    /// A soft pattern: a slow wave in brightness, with finer ripples on it.
    fn pattern(w: u32, h: u32) -> Linear {
        let mut img = Linear::new(w, h, [0.0; 3]);
        for y in 0..h {
            for x in 0..w {
                let v = 0.18 * 2f32.powf(0.8 * (x as f32 / 40.0).sin() + 0.3 * (y as f32 / 3.0).sin());
                img.data[(y * w + x) as usize] = [v, v * 0.9, v * 0.8];
            }
        }
        img
    }

    fn lumas(img: &Linear) -> Vec<f32> {
        img.data.iter().map(|p| luma(*p)).collect()
    }

    #[test]
    fn clarity_adds_local_contrast_and_keeps_the_mean() {
        let img = pattern(400, 300);
        let mut out = img.clone();
        texture_clarity(&mut out, 0.0, 1.0, &Ctx::default());
        let (a, b) = (lumas(&img), lumas(&out));
        assert!(std_dev(&b) > std_dev(&a) * 1.1, "{} → {}", std_dev(&a), std_dev(&b));
        let mean = |v: &[f32]| v.iter().sum::<f32>() / v.len() as f32;
        assert!((mean(&b) / mean(&a) - 1.0).abs() < 0.05);
    }

    #[test]
    fn texture_works_on_the_fine_scale() {
        let img = pattern(400, 300);
        let mut out = img.clone();
        texture_clarity(&mut out, -1.0, 0.0, &Ctx::default());
        assert!(std_dev(&lumas(&out)) < std_dev(&lumas(&img)), "negative texture smooths");
    }

    #[test]
    fn dehaze_restores_the_contrast_haze_took() {
        let clear = pattern(300, 200);
        let mut hazy = clear.clone();
        hazy.data.iter_mut().for_each(|p| *p = std::array::from_fn(|c| p[c] * 0.6 + 0.35));
        let mut fixed = hazy.clone();
        dehaze(&mut fixed, 0.6, &Ctx::default());
        assert!(std_dev(&lumas(&fixed)) > std_dev(&lumas(&hazy)) * 1.3);
        let mean = |img: &Linear| lumas(img).iter().sum::<f32>() / img.data.len() as f32;
        assert!(mean(&fixed) < mean(&hazy), "the veil comes off");
        let mut hazier = hazy.clone();
        dehaze(&mut hazier, -0.6, &Ctx::default());
        assert!(std_dev(&lumas(&hazier)) < std_dev(&lumas(&hazy)));
    }

    #[test]
    fn neutral_is_a_no_op() {
        let img = pattern(50, 40);
        let mut out = img.clone();
        texture_clarity(&mut out, 0.0, 0.0, &Ctx::default());
        dehaze(&mut out, 0.0, &Ctx::default());
        assert_eq!(out, img);
    }
}
