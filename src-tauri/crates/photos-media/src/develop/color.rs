//! HSL (eight overlapping hue bands) and colour grading (three tinted luminance ranges), in
//! one pass through OKLab so a pixel is converted once.

use photos_core::edits::{Grading, Hsl, Wheel};
use rayon::prelude::*;

use super::oklab::{from_oklab, lch, to_oklab};
use super::Linear;

/// Band centres as OKLCh hues: red, orange, yellow, green, aqua, blue, purple, magenta.
pub const BAND_HUES: [f32; 8] = [25.0, 60.0, 100.0, 140.0, 195.0, 262.0, 300.0, 330.0];
/// Half the width of a band's raised cosine, in degrees.
const HALF_WIDTH: f32 = 45.0;
/// The full swing of a band's hue slider, in degrees.
const HUE_SWING: f32 = 30.0;

fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Each band's share of a hue; the shares sum to 1.
pub fn band_weights(hue: f32) -> [f32; 8] {
    let mut w = BAND_HUES.map(|c| {
        let d = ((hue - c + 540.0).rem_euclid(360.0) - 180.0).abs();
        if d >= HALF_WIDTH {
            0.0
        } else {
            let t = (d / HALF_WIDTH) * std::f32::consts::FRAC_PI_2;
            t.cos() * t.cos()
        }
    });
    let sum: f32 = w.iter().sum();
    if sum > 0.0 {
        w.iter_mut().for_each(|v| *v /= sum);
    }
    w
}

/// `band_weights` tabulated every half degree, for the per-pixel pass.
fn weight_table() -> &'static [[f32; 8]] {
    static T: std::sync::OnceLock<Vec<[f32; 8]>> = std::sync::OnceLock::new();
    T.get_or_init(|| (0..=720).map(|i| band_weights(i as f32 / 2.0)).collect())
}

#[inline]
fn table_weights(hue: f32) -> [f32; 8] {
    let t = weight_table();
    let f = hue.rem_euclid(360.0) * 2.0;
    let i = (f as usize).min(719);
    let k = f - i as f32;
    std::array::from_fn(|b| t[i][b] * (1.0 - k) + t[i + 1][b] * k)
}

fn hsl_neutral(h: &[Hsl; 8]) -> bool {
    h.iter().all(|b| b.h == 0.0 && b.s == 0.0 && b.l == 0.0)
}

fn wheel_neutral(w: &Wheel) -> bool {
    w.sat == 0.0 && w.lum == 0.0
}

pub fn is_identity(hsl: &[Hsl; 8], g: &Grading) -> bool {
    hsl_neutral(hsl) && wheel_neutral(&g.shadows) && wheel_neutral(&g.midtones) && wheel_neutral(&g.highlights)
}

/// Shadow, midtone and highlight weights for an OKLab lightness. `balance` moves the split
/// points together (−1 favours highlights, +1 shadows); `blending` widens the overlaps.
pub fn range_weights(l: f32, g: &Grading) -> [f32; 3] {
    let width = 0.1 + 0.5 * g.blending;
    let shift = 0.15 * g.balance;
    let (lo, hi) = (0.33 + shift, 0.66 + shift);
    let s = 1.0 - smoothstep(lo - width / 2.0, lo + width / 2.0, l);
    let h = smoothstep(hi - width / 2.0, hi + width / 2.0, l);
    [s, (1.0 - s - h).max(0.0), h]
}

/// HSL and grading made ready for pixels: the global stage runs it on the frame, a mask on
/// the pixels it covers.
pub struct Colour {
    hsl: [Hsl; 8],
    grading: Grading,
    do_hsl: bool,
    do_grade: bool,
    tints: [(f32, f32, f32); 3],
}

impl Colour {
    /// `None` when neither changes a pixel.
    pub fn new(hsl: &[Hsl; 8], g: &Grading) -> Option<Self> {
        if is_identity(hsl, g) {
            return None;
        }
        let tints = [g.shadows, g.midtones, g.highlights].map(|w| {
            let (s, c) = w.hue.to_radians().sin_cos();
            (w.sat * 0.08 * c, w.sat * 0.08 * s, w.lum * 0.15)
        });
        let do_grade = !(wheel_neutral(&g.shadows) && wheel_neutral(&g.midtones) && wheel_neutral(&g.highlights));
        Some(Self { hsl: *hsl, grading: *g, do_hsl: !hsl_neutral(hsl), do_grade, tints })
    }

    #[inline]
    pub fn pixel(&self, p: [f32; 3]) -> [f32; 3] {
        let (hsl, g, tints) = (&self.hsl, &self.grading, &self.tints);
        let mut lab = to_oklab(p);
        if self.do_hsl {
            let (c, h) = lch(lab);
            // Neutrals have no hue to belong to: they never change.
            let n = smoothstep(0.0, 0.04, c);
            if n > 0.0 {
                let w = table_weights(h);
                let (mut dh, mut ds, mut dl) = (0.0, 0.0, 0.0);
                for i in 0..8 {
                    dh += w[i] * hsl[i].h;
                    ds += w[i] * hsl[i].s;
                    dl += w[i] * hsl[i].l;
                }
                let hue = (h + n * dh * HUE_SWING).to_radians();
                let chroma = c * (1.0 + n * ds).max(0.0);
                lab = [lab[0] * 2f32.powf(0.5 * n * dl), chroma * hue.cos(), chroma * hue.sin()];
            }
        }
        if self.do_grade {
            let w = range_weights(lab[0], g);
            for (k, (da, db, dl)) in tints.iter().enumerate() {
                lab[1] += w[k] * da;
                lab[2] += w[k] * db;
                lab[0] += w[k] * dl;
            }
        }
        from_oklab(lab)
    }
}

pub fn apply(img: &mut Linear, hsl: &[Hsl; 8], g: &Grading) {
    let Some(colour) = Colour::new(hsl, g) else { return };
    img.data.par_iter_mut().for_each(|p| *p = colour.pixel(*p));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn one(c: [f32; 3], hsl: [Hsl; 8], g: Grading) -> [f32; 3] {
        let mut img = Linear::new(1, 1, c);
        apply(&mut img, &hsl, &g);
        img.data[0]
    }

    fn band(i: usize, b: Hsl) -> [Hsl; 8] {
        let mut h = [Hsl::default(); 8];
        h[i] = b;
        h
    }

    const RED: [f32; 3] = [0.7, 0.08, 0.06];
    const BLUE: [f32; 3] = [0.05, 0.1, 0.7];

    #[test]
    fn weights_cover_every_hue() {
        for h in 0..360 {
            let s: f32 = band_weights(h as f32).iter().sum();
            assert!((s - 1.0).abs() < 1e-5, "{h}");
        }
    }

    #[test]
    fn neutral_settings_change_nothing() {
        assert_eq!(one(RED, [Hsl::default(); 8], Grading::default()), RED);
    }

    #[test]
    fn a_red_hue_shift_moves_reds_and_leaves_blues() {
        let shift = band(0, Hsl { h: 1.0, ..Default::default() });
        let (_, before) = lch(to_oklab(RED));
        let (_, after) = lch(to_oklab(one(RED, shift, Grading::default())));
        let moved = (after - before + 540.0).rem_euclid(360.0) - 180.0;
        assert!((20.0..=30.0).contains(&moved), "{moved}");
        let (_, b0) = lch(to_oklab(BLUE));
        let (_, b1) = lch(to_oklab(one(BLUE, shift, Grading::default())));
        assert!((b1 - b0).abs() < 0.5, "{b0} → {b1}");
    }

    #[test]
    fn desaturating_blue_greys_blue_but_not_red() {
        let grey = band(5, Hsl { s: -1.0, ..Default::default() });
        assert!(lch(to_oklab(one(BLUE, grey, Grading::default()))).0 < 0.3 * lch(to_oklab(BLUE)).0);
        let r = one(RED, grey, Grading::default());
        assert!((0..3).all(|i| (r[i] - RED[i]).abs() < 1e-3), "{r:?}");
    }

    #[test]
    fn greys_never_change_under_hsl() {
        let all = [Hsl { h: 1.0, s: 1.0, l: 1.0 }; 8];
        for v in [0.02, 0.18, 0.9] {
            let p = one([v; 3], all, Grading::default());
            assert!((0..3).all(|i| (p[i] - v).abs() < 1e-4), "{v}: {p:?}");
        }
    }

    #[test]
    fn the_shadows_wheel_tints_the_shadows_only() {
        let g = Grading { shadows: Wheel { hue: 250.0, sat: 1.0, lum: 0.0 }, ..Default::default() };
        let dark = lch(to_oklab(one([0.02; 3], [Hsl::default(); 8], g)));
        let light = lch(to_oklab(one([0.8; 3], [Hsl::default(); 8], g)));
        assert!(dark.0 > 0.03 && (dark.1 - 250.0).abs() < 2.0, "{dark:?}");
        assert!(light.0 < 0.005, "{light:?}");
    }
}
