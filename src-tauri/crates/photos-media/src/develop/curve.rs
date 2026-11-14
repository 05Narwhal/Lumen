//! The tone curve: a monotone cubic (Fritsch–Carlson) through the user's points, applied on
//! the sRGB-encoded value (a perceptual scale, as every curve editor draws it), master then
//! per channel. `src/lib/tone-curve.ts` implements the same maths for drawing, held to this
//! by `src/lib/curve-golden.json`, which a test here writes.

use photos_core::edits::Curve;
use rayon::prelude::*;

use super::linear::{linear_to_srgb, srgb_to_linear};
use super::Linear;

/// A curve through sorted points, (0, 0) and (1, 1) implied when absent. Beyond 0 and 1 it
/// continues along its end tangents, so unclamped values are neither clipped nor folded.
#[derive(Debug, Clone, PartialEq)]
pub struct Monotone {
    xs: Vec<f32>,
    ys: Vec<f32>,
    ms: Vec<f32>,
}

impl Monotone {
    pub fn new(points: &[[f32; 2]]) -> Self {
        let mut p: Vec<[f32; 2]> = points.to_vec();
        p.sort_by(|a, b| a[0].total_cmp(&b[0]));
        if p.first().is_none_or(|q| q[0] > 1e-4) {
            p.insert(0, [0.0, 0.0]);
        }
        if p.last().is_none_or(|q| q[0] < 1.0 - 1e-4) {
            p.push([1.0, 1.0]);
        }
        let xs: Vec<f32> = p.iter().map(|q| q[0]).collect();
        let ys: Vec<f32> = p.iter().map(|q| q[1]).collect();
        let n = xs.len();
        let d: Vec<f32> = (0..n - 1).map(|k| (ys[k + 1] - ys[k]) / (xs[k + 1] - xs[k]).max(1e-6)).collect();
        let mut ms = vec![0f32; n];
        ms[0] = d[0];
        ms[n - 1] = d[n - 2];
        for k in 1..n - 1 {
            ms[k] = if d[k - 1] * d[k] <= 0.0 { 0.0 } else { (d[k - 1] + d[k]) / 2.0 };
        }
        for k in 0..n - 1 {
            if d[k] == 0.0 {
                ms[k] = 0.0;
                ms[k + 1] = 0.0;
                continue;
            }
            let (a, b) = (ms[k] / d[k], ms[k + 1] / d[k]);
            let h = a * a + b * b;
            if h > 9.0 {
                let t = 3.0 / h.sqrt();
                ms[k] = t * a * d[k];
                ms[k + 1] = t * b * d[k];
            }
        }
        Self { xs, ys, ms }
    }

    pub fn eval(&self, x: f32) -> f32 {
        let n = self.xs.len();
        if x <= self.xs[0] {
            return self.ys[0] + (x - self.xs[0]) * self.ms[0];
        }
        if x >= self.xs[n - 1] {
            return self.ys[n - 1] + (x - self.xs[n - 1]) * self.ms[n - 1];
        }
        let k = self.xs.partition_point(|&v| v <= x).saturating_sub(1).min(n - 2);
        let h = self.xs[k + 1] - self.xs[k];
        let t = (x - self.xs[k]) / h;
        let (t2, t3) = (t * t, t * t * t);
        let h00 = 2.0 * t3 - 3.0 * t2 + 1.0;
        let h10 = t3 - 2.0 * t2 + t;
        let h01 = -2.0 * t3 + 3.0 * t2;
        let h11 = t3 - t2;
        h00 * self.ys[k] + h10 * h * self.ms[k] + h01 * self.ys[k + 1] + h11 * h * self.ms[k + 1]
    }
}

pub fn is_identity(c: &Curve) -> bool {
    c.master.is_empty() && c.r.is_empty() && c.g.is_empty() && c.b.is_empty()
}

/// Entries of the linear-in, linear-out table over 0-1, spaced by square root so the dark
/// end (where the sRGB encoding is steep) gets most of them.
const LINEAR_STEPS: usize = 16_384;

/// A curve made ready for pixels: the global stage runs it on the frame, a mask on the pixels
/// it covers.
pub struct Lut {
    master: Monotone,
    chans: [Monotone; 3],
    tables: Vec<Vec<f32>>,
}

impl Lut {
    /// `None` for the straight line.
    pub fn new(c: &Curve) -> Option<Self> {
        if is_identity(c) {
            return None;
        }
        let mut lut = Self { master: Monotone::new(&c.master), chans: [Monotone::new(&c.r), Monotone::new(&c.g), Monotone::new(&c.b)], tables: Vec::new() };
        lut.tables = (0..3).map(|i| (0..=LINEAR_STEPS).map(|k| lut.exact(i, (k as f32 / LINEAR_STEPS as f32).powi(2))).collect()).collect();
        Some(lut)
    }

    /// Master then channel, on the encoded value; linear in and out.
    fn exact(&self, i: usize, v: f32) -> f32 {
        let y = self.chans[i].eval(self.master.eval(linear_to_srgb(v)));
        if y < 0.0 {
            -srgb_to_linear(-y)
        } else {
            srgb_to_linear(y)
        }
    }

    #[inline]
    pub fn pixel(&self, mut p: [f32; 3]) -> [f32; 3] {
        for (i, c) in p.iter_mut().enumerate() {
            let v = *c;
            *c = if (0.0..=1.0).contains(&v) {
                let f = v.sqrt() * LINEAR_STEPS as f32;
                let k = (f as usize).min(LINEAR_STEPS - 1);
                let w = f - k as f32;
                self.tables[i][k] * (1.0 - w) + self.tables[i][k + 1] * w
            } else {
                self.exact(i, v)
            };
        }
        p
    }
}

pub fn apply(img: &mut Linear, c: &Curve) {
    let Some(lut) = Lut::new(c) else { return };
    img.data.par_iter_mut().for_each(|p| *p = lut.pixel(*p));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_curve_through_rising_points_never_falls() {
        // A deterministic scatter of rising points, then 10 000 samples.
        let mut seed = 7u32;
        let mut rnd = || {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (seed >> 8) as f32 / (1u32 << 24) as f32
        };
        for _ in 0..50 {
            let mut xs: Vec<f32> = (0..6).map(|_| rnd()).collect();
            let mut ys: Vec<f32> = (0..6).map(|_| rnd()).collect();
            xs.sort_by(f32::total_cmp);
            ys.sort_by(f32::total_cmp);
            let pts: Vec<[f32; 2]> = xs.iter().zip(&ys).map(|(x, y)| [*x, *y]).collect();
            let m = Monotone::new(&pts);
            let mut last = f32::NEG_INFINITY;
            for i in 0..=10_000 {
                let v = m.eval(i as f32 / 10_000.0);
                assert!(v >= last - 1e-5, "{pts:?} at {i}");
                last = v;
            }
        }
    }

    #[test]
    fn it_passes_through_its_points_and_continues_past_the_ends() {
        let m = Monotone::new(&[[0.25, 0.2], [0.75, 0.85]]);
        for (x, y) in [(0.0, 0.0), (0.25, 0.2), (0.75, 0.85), (1.0, 1.0)] {
            assert!((m.eval(x) - y).abs() < 1e-6, "{x}");
        }
        assert!(m.eval(1.2) > 1.0 && m.eval(-0.1) < 0.0);
    }

    #[test]
    fn no_points_is_the_identity_and_an_s_curve_adds_contrast() {
        let mut img = Linear::new(2, 1, [0.0; 3]);
        img.data = vec![[0.05, 0.2, 0.5], [0.8, 0.01, 1.3]];
        let before = img.clone();
        apply(&mut img, &Curve::default());
        assert_eq!(img, before);
        let s = Curve { master: vec![[0.25, 0.18], [0.75, 0.82]], ..Default::default() };
        let mut img = Linear::new(2, 1, [0.0; 3]);
        img.data = vec![[srgb_to_linear(0.25); 3], [srgb_to_linear(0.75); 3]];
        apply(&mut img, &s);
        assert!((linear_to_srgb(img.data[0][0]) - 0.18).abs() < 1e-3);
        assert!((linear_to_srgb(img.data[1][0]) - 0.82).abs() < 1e-3);
    }

    #[test]
    fn a_channel_curve_moves_only_its_channel() {
        let c = Curve { r: vec![[0.5, 0.7]], ..Default::default() };
        let mut img = Linear::new(1, 1, [0.2; 3]);
        apply(&mut img, &c);
        assert!(img.data[0][0] > 0.25 && (img.data[0][1] - 0.2).abs() < 1e-4 && (img.data[0][2] - 0.2).abs() < 1e-4);
    }

    /// The drawing in the curve editor must match the render: TypeScript is held to this.
    #[test]
    fn writes_the_golden_file_for_the_curve_editor() {
        let curves: Vec<Vec<[f32; 2]>> = vec![
            vec![],
            vec![[0.25, 0.18], [0.75, 0.82]],
            vec![[0.0, 0.1], [0.5, 0.5], [1.0, 0.9]],
            vec![[0.1, 0.0], [0.3, 0.6], [0.35, 0.62], [0.9, 1.0]],
            vec![[0.0, 1.0], [1.0, 0.0]],
            vec![[0.2, 0.5], [0.4, 0.5], [0.6, 0.9]],
        ];
        let xs: Vec<f32> = (0..50).map(|i| i as f32 / 49.0).collect();
        let cases: Vec<serde_json::Value> = curves
            .iter()
            .map(|pts| {
                let m = Monotone::new(pts);
                serde_json::json!({ "points": pts, "ys": xs.iter().map(|x| m.eval(*x)).collect::<Vec<_>>() })
            })
            .collect();
        let doc = serde_json::json!({ "xs": xs, "cases": cases });
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../src/lib/curve-golden.json");
        std::fs::write(path, serde_json::to_string_pretty(&doc).unwrap() + "\n").unwrap();
    }
}
