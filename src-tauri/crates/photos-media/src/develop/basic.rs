//! The Basic panel, per pixel in linear light: white balance, exposure, the four tone
//! ranges, contrast, vibrance and saturation. (Texture, clarity and dehaze need their
//! neighbours and live in `local`.)
//!
//! Tone work happens on luminance in stops about middle grey (0.18) and is applied as a
//! ratio to all three channels, so hue holds while brightness moves.

use photos_core::edits::Basic;
use rayon::prelude::*;

use super::linear::luma;
use super::Linear;

pub const MID: f32 = 0.18;

/// White-balance gains: in core since Phase 8d, where Auto inverts them.
pub use photos_core::edits::wb_gains;

/// A bell over stops: 1 at `centre`, falling with width `sigma`.
#[inline]
fn bell(s: f32, centre: f32, sigma: f32) -> f32 {
    let d = (s - centre) / sigma;
    (-0.5 * d * d).exp()
}

/// Stops added to a pixel `s` stops from middle grey by the four tone sliders. Whites sit at
/// clipping (+2.47 stops is 1.0), blacks far below.
pub fn tone_stops(b: &Basic, s: f32) -> f32 {
    let mut d = 0.0;
    if b.whites != 0.0 {
        d += b.whites * 1.0 * bell(s, 2.3, 0.7);
    }
    if b.highlights != 0.0 {
        d += b.highlights * 1.5 * bell(s, 1.2, 1.0);
    }
    if b.shadows != 0.0 {
        d += b.shadows * 1.5 * bell(s, -2.5, 1.3);
    }
    if b.blacks != 0.0 {
        d += b.blacks * 1.5 * bell(s, -4.5, 1.3);
    }
    d
}

/// Contrast as an S-curve on stops: 0 stays put, the extremes move least.
#[inline]
pub fn contrast_stops(c: f32, s: f32) -> f32 {
    c * 1.2 * (s / 1.5).tanh()
}

/// A colour's saturation, 0 (grey) to 1 (a primary).
#[inline]
fn chroma(p: [f32; 3]) -> f32 {
    let max = p[0].max(p[1]).max(p[2]);
    let min = p[0].min(p[1]).min(p[2]);
    if max <= 1e-6 {
        0.0
    } else {
        ((max - min) / max).clamp(0.0, 1.0)
    }
}

/// White balance and exposure as one gain per channel, or `None` when they are neutral.
pub fn gains_of(b: &Basic) -> Option<[f32; 3]> {
    if b.temperature == 0.0 && b.tint == 0.0 && b.exposure == 0.0 {
        return None;
    }
    let wb = wb_gains(b.temperature, b.tint);
    let ev = 2f32.powf(b.exposure);
    Some([wb[0] * ev, wb[1] * ev, wb[2] * ev])
}

/// White balance and exposure: plain gains, run before any local stage.
pub fn apply_gains(img: &mut Linear, b: &Basic) {
    if let Some(g) = gains_of(b) {
        img.data.par_iter_mut().for_each(|p| *p = [p[0] * g[0], p[1] * g[1], p[2] * g[2]]);
    }
}

/// The tone ranges, contrast, vibrance and saturation for one pixel: what `apply_tones` does
/// to every pixel, and the mask stage to the pixels under a mask.
#[derive(Debug, Clone, Copy)]
pub struct Tones {
    b: Basic,
    tones: bool,
    colour: bool,
}

impl Tones {
    /// `None` when the settings change nothing.
    pub fn new(b: &Basic) -> Option<Self> {
        let tones = b.whites != 0.0 || b.highlights != 0.0 || b.shadows != 0.0 || b.blacks != 0.0 || b.contrast != 0.0;
        let colour = b.vibrance != 0.0 || b.saturation != 0.0;
        (tones || colour).then_some(Self { b: *b, tones, colour })
    }

    #[inline]
    pub fn pixel(&self, mut c: [f32; 3]) -> [f32; 3] {
        let b = &self.b;
        if self.tones {
            let y = luma(c);
            if y > 1e-7 {
                let s = (y / MID).log2();
                let s2 = s + tone_stops(b, s);
                let s3 = s2 + contrast_stops(b.contrast, s2);
                let k = 2f32.powf(s3 - s);
                c = [c[0] * k, c[1] * k, c[2] * k];
            }
        }
        if self.colour {
            let y = luma(c);
            let f = (1.0 + b.vibrance * (1.0 - chroma(c))) * (1.0 + b.saturation);
            c = [y + (c[0] - y) * f, y + (c[1] - y) * f, y + (c[2] - y) * f];
        }
        c
    }
}

/// The tone ranges, contrast, vibrance and saturation.
pub fn apply_tones(img: &mut Linear, b: &Basic) {
    if let Some(t) = Tones::new(b) {
        img.data.par_iter_mut().for_each(|p| *p = t.pixel(*p));
    }
}
/// Both halves, for callers without local stages between them.
pub fn apply(img: &mut Linear, b: &Basic) {
    apply_gains(img, b);
    apply_tones(img, b);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn one(p: [f32; 3], b: Basic) -> [f32; 3] {
        let mut img = Linear::new(1, 1, p);
        apply(&mut img, &b);
        img.data[0]
    }

    fn close(a: [f32; 3], b: [f32; 3], eps: f32) -> bool {
        (0..3).all(|i| (a[i] - b[i]).abs() <= eps)
    }

    #[test]
    fn zero_is_the_identity() {
        for p in [[0.18; 3], [0.9, 0.2, 0.05], [1.4, -0.02, 0.3], [0.0; 3]] {
            assert_eq!(one(p, Basic::default()), p);
        }
    }

    #[test]
    fn one_stop_doubles_linear_light() {
        assert!(close(one([0.1, 0.2, 0.3], Basic { exposure: 1.0, ..Default::default() }), [0.2, 0.4, 0.6], 1e-6));
    }

    #[test]
    fn contrast_pivots_on_middle_grey() {
        let b = Basic { contrast: 0.5, ..Default::default() };
        assert!(close(one([MID; 3], b), [MID; 3], 1e-6));
        assert!(one([0.05; 3], b)[0] < 0.05, "dark gets darker");
        assert!(one([0.6; 3], b)[0] > 0.6, "light gets lighter");
        let flat = Basic { contrast: -0.5, ..Default::default() };
        assert!(one([0.05; 3], flat)[0] > 0.05 && one([0.6; 3], flat)[0] < 0.6);
    }

    #[test]
    fn saturation_minus_one_is_grey_and_keeps_luminance() {
        let p = one([0.8, 0.2, 0.05], Basic { saturation: -1.0, ..Default::default() });
        assert!(close(p, [p[0]; 3], 1e-6));
        assert!((p[0] - luma([0.8, 0.2, 0.05])).abs() < 1e-6);
    }

    #[test]
    fn vibrance_moves_a_dull_colour_more_than_a_vivid_one() {
        let b = Basic { vibrance: 0.8, ..Default::default() };
        let gain = |p: [f32; 3]| chroma(one(p, b)) - chroma(p);
        assert!(gain([0.4, 0.35, 0.3]) > 0.0);
        assert!(gain([0.4, 0.35, 0.3]) / chroma([0.4, 0.35, 0.3]) > gain([0.8, 0.1, 0.05]) / chroma([0.8, 0.1, 0.05]));
    }

    #[test]
    fn highlights_lower_the_brights_and_leave_the_darks() {
        let b = Basic { highlights: -1.0, ..Default::default() };
        assert!(one([0.9; 3], b)[0] < 0.7);
        assert!((one([0.02; 3], b)[0] / 0.02 - 1.0).abs() < 0.01);
    }

    #[test]
    fn shadows_lift_the_darks_and_leave_the_brights() {
        let b = Basic { shadows: 0.8, ..Default::default() };
        assert!(one([0.013; 3], b)[0] > 0.018);
        assert!((one([0.8; 3], b)[0] / 0.8 - 1.0).abs() < 0.02);
    }

    #[test]
    fn blacks_lift_deep_shadows_more_than_midtones() {
        let b = Basic { blacks: 1.0, ..Default::default() };
        let lift = |v: f32| one([v; 3], b)[0] / v;
        assert!(lift(0.01) > 1.5 && lift(0.01) > lift(0.5) + 0.4, "{} {}", lift(0.01), lift(0.5));
    }

    #[test]
    fn whites_pull_down_what_would_clip() {
        let b = Basic { whites: -1.0, ..Default::default() };
        assert!(one([1.2; 3], b)[0] < 1.0);
        assert!((one([0.1; 3], b)[0] / 0.1 - 1.0).abs() < 0.01);
    }

    #[test]
    fn temperature_and_tint_move_the_right_channels_and_keep_white_bright() {
        let warm = one([0.5; 3], Basic { temperature: 50.0, ..Default::default() });
        assert!(warm[0] > 0.5 && warm[2] < 0.5, "{warm:?}");
        assert!((luma(warm) - 0.5).abs() < 1e-5);
        let magenta = one([0.5; 3], Basic { tint: 50.0, ..Default::default() });
        assert!(magenta[1] < 0.5 && magenta[0] > 0.5, "{magenta:?}");
    }

    #[test]
    fn a_ratio_keeps_the_hue() {
        let p = [0.6, 0.3, 0.1];
        let q = one(p, Basic { highlights: -0.7, shadows: 0.4, contrast: 0.3, ..Default::default() });
        assert!((q[1] / q[0] - 0.5).abs() < 1e-5 && (q[2] / q[0] - 1.0 / 6.0).abs() < 1e-5);
    }
}
