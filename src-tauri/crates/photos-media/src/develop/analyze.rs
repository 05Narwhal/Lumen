//! Numbers an AI assistant cannot judge from pixels alone (Round 16): tone, clipping, a
//! colour cast as the Temp/Tint correction that removes it, saturation, and each face's skin.
//! Measured on a small render of the current edit; luma is display-encoded (sRGB), 0–1.

use serde::Serialize;

use super::linear::{linear_to_srgb, luma, Linear};
use super::oklab::{lch, to_oklab};

/// A face to measure, 0–1 of the rendered frame.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct FaceBox {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Luma {
    pub mean: f32,
    pub p1: f32,
    pub p50: f32,
    pub p99: f32,
}

/// Percent of pixels by display luma: blacks < 0.1 ≤ shadows < 0.3 ≤ midtones < 0.7 ≤
/// highlights < 0.9 ≤ whites.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Zones {
    pub blacks: f32,
    pub shadows: f32,
    pub midtones: f32,
    pub highlights: f32,
    pub whites: f32,
}

/// Percent of pixels clipped: `shadow` black in every channel, `highlight` white in any,
/// and white per channel.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Clip {
    pub shadow: f32,
    pub highlight: f32,
    pub r: f32,
    pub g: f32,
    pub b: f32,
}

/// The Temp and Tint to *add* so the near-neutral midtones turn grey (a warm cast gives a
/// negative temperature).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Cast {
    pub temperature: f32,
    pub tint: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Sat {
    pub mean: f32,
    pub p95: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Face {
    #[serde(rename = "box")]
    pub bounds: FaceBox,
    /// OKLCh hue of the skin, degrees; natural skin sits around 20–50°.
    pub skin_hue: f32,
    pub luma: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Analysis {
    pub luma: Luma,
    pub zones: Zones,
    pub clip: Clip,
    /// `None` when too few pixels are near neutral to tell.
    pub cast: Option<Cast>,
    pub sat: Sat,
    pub faces: Vec<Face>,
}

const BINS: usize = 1024;

fn round(v: f32, dp: i32) -> f32 {
    let k = 10f32.powi(dp);
    (v * k).round() / k
}

/// The value below which `q` of a histogram over 0–1 falls.
fn percentile(hist: &[u32], total: u32, q: f32) -> f32 {
    let target = (q * total as f32).ceil().max(1.0) as u32;
    let mut seen = 0;
    for (i, n) in hist.iter().enumerate() {
        seen += n;
        if seen >= target {
            return i as f32 / (hist.len() - 1) as f32;
        }
    }
    1.0
}

fn bin(v: f32) -> usize {
    (v.clamp(0.0, 1.0) * (BINS - 1) as f32).round() as usize
}

/// Measure a rendered frame (any size; a 512 px render is plenty) and the faces in it.
pub fn analyze(img: &Linear, faces: &[FaceBox]) -> Analysis {
    let n = img.data.len().max(1) as u32;
    let pct = |c: u32| round(100.0 * c as f32 / n as f32, 2);
    let mut luma_hist = vec![0u32; BINS];
    let mut sat_hist = vec![0u32; BINS];
    let (mut sum, mut sat_sum) = (0f64, 0f64);
    let mut zones = [0u32; 5];
    let (mut shadow, mut highlight, mut ch) = (0u32, 0u32, [0u32; 3]);
    let (mut grey, mut grey_n) = ([0f64; 3], 0u32);
    for &p in &img.data {
        let e = p.map(|v| linear_to_srgb(v.clamp(0.0, 1.0)));
        let y = linear_to_srgb(luma(p).clamp(0.0, 1.0));
        sum += f64::from(y);
        luma_hist[bin(y)] += 1;
        zones[match y {
            y if y < 0.1 => 0,
            y if y < 0.3 => 1,
            y if y < 0.7 => 2,
            y if y < 0.9 => 3,
            _ => 4,
        }] += 1;
        if e.iter().all(|&v| v < 0.5 / 255.0) {
            shadow += 1;
        }
        if e.iter().any(|&v| v > 254.5 / 255.0) {
            highlight += 1;
        }
        for c in 0..3 {
            if e[c] > 254.5 / 255.0 {
                ch[c] += 1;
            }
        }
        let (hi, lo) = (e[0].max(e[1]).max(e[2]), e[0].min(e[1]).min(e[2]));
        let s = if hi > 0.0 { (hi - lo) / hi } else { 0.0 };
        sat_sum += f64::from(s);
        sat_hist[bin(s)] += 1;
        let (chroma, _) = lch(to_oklab(p));
        if (0.2..0.8).contains(&y) && chroma < 0.06 {
            for c in 0..3 {
                grey[c] += f64::from(p[c]);
            }
            grey_n += 1;
        }
    }
    let cast = (grey_n * 100 >= n).then(|| {
        let mean = grey.map(|v| (v / f64::from(grey_n)) as f32);
        let (t, m) = photos_core::edits::wb_from_grey(mean);
        Cast { temperature: t.round(), tint: m.round() }
    });
    Analysis {
        luma: Luma {
            mean: round((sum / f64::from(n)) as f32, 3),
            p1: round(percentile(&luma_hist, n, 0.01), 3),
            p50: round(percentile(&luma_hist, n, 0.5), 3),
            p99: round(percentile(&luma_hist, n, 0.99), 3),
        },
        zones: Zones { blacks: pct(zones[0]), shadows: pct(zones[1]), midtones: pct(zones[2]), highlights: pct(zones[3]), whites: pct(zones[4]) },
        clip: Clip { shadow: pct(shadow), highlight: pct(highlight), r: pct(ch[0]), g: pct(ch[1]), b: pct(ch[2]) },
        cast,
        sat: Sat { mean: round((sat_sum / f64::from(n)) as f32, 3), p95: round(percentile(&sat_hist, n, 0.95), 3) },
        faces: faces.iter().filter_map(|f| face(img, *f)).collect(),
    }
}

/// The skin of one face: the inner ellipse of its box (cheeks and forehead, little hair or
/// background), its mean OKLab hue and display luma.
fn face(img: &Linear, f: FaceBox) -> Option<Face> {
    let (w, h) = (img.width as f32, img.height as f32);
    let (cx, cy, rx, ry) = ((f.x + f.w / 2.0) * w, (f.y + f.h / 2.0) * h, f.w * w * 0.35, f.h * h * 0.35);
    let (x0, x1) = (((cx - rx).floor().max(0.0)) as u32, ((cx + rx).ceil().min(w - 1.0)) as u32);
    let (y0, y1) = (((cy - ry).floor().max(0.0)) as u32, ((cy + ry).ceil().min(h - 1.0)) as u32);
    let (mut lab, mut lin, mut n) = ([0f64; 3], [0f64; 3], 0u32);
    for y in y0..=y1 {
        for x in x0..=x1 {
            let (dx, dy) = ((x as f32 + 0.5 - cx) / rx.max(0.5), (y as f32 + 0.5 - cy) / ry.max(0.5));
            if dx * dx + dy * dy > 1.0 {
                continue;
            }
            let p = img.get(x, y);
            let o = to_oklab(p);
            for c in 0..3 {
                lab[c] += f64::from(o[c]);
                lin[c] += f64::from(p[c]);
            }
            n += 1;
        }
    }
    if n == 0 {
        return None;
    }
    let mean = |v: [f64; 3]| v.map(|s| (s / f64::from(n)) as f32);
    let (_, hue) = lch(mean(lab));
    let bounds = FaceBox { x: round(f.x, 3), y: round(f.y, 3), w: round(f.w, 3), h: round(f.h, 3) };
    Some(Face { bounds, skin_hue: hue.round(), luma: round(linear_to_srgb(luma(mean(lin)).clamp(0.0, 1.0)), 3) })
}

#[cfg(test)]
mod tests {
    use super::super::linear::srgb_to_linear;
    use super::*;

    fn grey(v: f32) -> [f32; 3] {
        [srgb_to_linear(v); 3]
    }

    #[test]
    fn a_grey_ramp_fills_the_zones_by_width() {
        // Ten columns at display 0.05, 0.15 … 0.95: one in blacks, two in shadows, four in
        // midtones, two in highlights, one in whites.
        let mut img = Linear::new(10, 4, [0.0; 3]);
        for y in 0..4 {
            for x in 0..10 {
                img.data[(y * 10 + x) as usize] = grey(0.05 + 0.1 * x as f32);
            }
        }
        let a = analyze(&img, &[]);
        assert_eq!(a.zones, Zones { blacks: 10.0, shadows: 20.0, midtones: 40.0, highlights: 20.0, whites: 10.0 });
        assert!((a.luma.mean - 0.5).abs() < 0.01, "{}", a.luma.mean);
        assert!((a.luma.p1 - 0.05).abs() < 0.01 && (a.luma.p99 - 0.95).abs() < 0.01, "{:?}", a.luma);
        assert_eq!(a.cast, Some(Cast { temperature: 0.0, tint: 0.0 }));
        assert_eq!(a.clip.highlight, 0.0);
    }

    #[test]
    fn a_clipped_patch_counts_its_share() {
        let mut img = Linear::new(10, 10, grey(0.5));
        for i in 0..5 {
            img.data[i] = [1.2, 0.9, 0.4];
        }
        for i in 5..8 {
            img.data[i] = [0.0; 3];
        }
        let a = analyze(&img, &[]);
        assert_eq!((a.clip.highlight, a.clip.r, a.clip.g, a.clip.shadow), (5.0, 5.0, 0.0, 3.0));
    }

    #[test]
    fn a_warm_cast_asks_for_a_cooler_temperature() {
        let img = Linear::new(8, 8, [srgb_to_linear(0.55), srgb_to_linear(0.5), srgb_to_linear(0.45)]);
        let cast = analyze(&img, &[]).cast.expect("a near-neutral frame");
        assert!(cast.temperature < -5.0, "{cast:?}");
        // Applying it makes the frame grey.
        let g = photos_core::edits::wb_gains(cast.temperature, cast.tint);
        let p = img.data[0];
        let fixed: Vec<f32> = (0..3).map(|c| p[c] * g[c]).collect();
        assert!((fixed[0] - fixed[2]).abs() < 0.01, "{fixed:?}");
    }

    #[test]
    fn a_face_box_reads_the_skin_inside_it() {
        // Skin-coloured square in the middle of a blue frame; the face box covers it.
        let skin = [srgb_to_linear(0.85), srgb_to_linear(0.62), srgb_to_linear(0.5)];
        let mut img = Linear::new(100, 100, [0.02, 0.05, 0.4]);
        for y in 30..70 {
            for x in 30..70 {
                img.data[y * 100 + x] = skin;
            }
        }
        let a = analyze(&img, &[FaceBox { x: 0.3, y: 0.3, w: 0.4, h: 0.4 }, FaceBox { x: 2.0, y: 2.0, w: 0.1, h: 0.1 }]);
        assert_eq!(a.faces.len(), 1, "a box outside the frame is dropped");
        let f = &a.faces[0];
        assert!((20.0..=60.0).contains(&f.skin_hue), "{f:?}");
        assert!(f.luma > 0.6, "{f:?}");
    }
}
