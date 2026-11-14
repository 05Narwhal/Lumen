//! How well each look suits one photo (Round 18 D1): Lumen, not the assistant, scores every
//! look 0–100 by colour theory from the photo's *before* render, so the numbers are the same
//! on every run and edits don't move them.

use super::looks::{Key, Look};

/// What the score reads from a photo.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct PhotoColour {
    /// Share of the chromatic pixels per 30° OKLCh hue bin, bin i centred on 15 + 30i; sums
    /// to 1, or all 0 for a grey photo.
    pub hues: [f32; 12],
    /// Share of pixels with OKLCh chroma > 0.04.
    pub chroma_share: f32,
    /// Mean display luma, 0–1.
    pub key: f32,
    /// p99 − p1 of display luma.
    pub contrast: f32,
    /// Mean HSV saturation, 0–1.
    pub sat: f32,
    /// A face whose skin hue is 20–50°.
    pub skin: bool,
    /// Share of the frame Facet calls sky; 0 without Facet.
    pub sky: f32,
}

/// Pixels with less OKLCh chroma than this count as grey.
const CHROMA: f32 = 0.04;

/// Names for the hue bins, for the reasons.
const BIN_NAMES: [&str; 12] = ["pink", "red", "orange", "yellow", "lime", "green", "teal", "cyan", "blue", "violet", "purple", "magenta"];

/// The colour summary of a (small, ~512 px) render of the photo.
pub fn photo_colour(img: &image::RgbImage, faces_skin: bool, sky: f32) -> PhotoColour {
    use photos_media::develop::linear::{linear_to_srgb, luma, srgb_to_linear};
    use photos_media::develop::oklab::{lch, to_oklab};
    let n = (img.width() * img.height()).max(1) as f32;
    let mut bins = [0f32; 12];
    let mut hist = [0u32; 256];
    let (mut chromatic, mut key, mut sat) = (0u32, 0f64, 0f64);
    for p in img.pixels() {
        let e = p.0.map(|v| v as f32 / 255.0);
        let lin = e.map(srgb_to_linear);
        let y = linear_to_srgb(luma(lin).clamp(0.0, 1.0));
        key += f64::from(y);
        hist[(y * 255.0).round() as usize] += 1;
        let (hi, lo) = (e[0].max(e[1]).max(e[2]), e[0].min(e[1]).min(e[2]));
        sat += f64::from(if hi > 0.0 { (hi - lo) / hi } else { 0.0 });
        let (c, h) = lch(to_oklab(lin));
        if c > CHROMA {
            chromatic += 1;
            bins[((h / 30.0) as usize).min(11)] += 1.0;
        }
    }
    if chromatic > 0 {
        bins.iter_mut().for_each(|b| *b /= chromatic as f32);
    }
    let pct = |q: f32| {
        let target = (q * n).ceil().max(1.0) as u32;
        let mut seen = 0;
        hist.iter().position(|&c| {
            seen += c;
            seen >= target
        })
        .unwrap_or(255) as f32
            / 255.0
    };
    PhotoColour {
        hues: bins,
        chroma_share: chromatic as f32 / n,
        key: (key / f64::from(n)) as f32,
        contrast: pct(0.99) - pct(0.01),
        sat: (sat / f64::from(n)) as f32,
        skin: faces_skin,
        sky,
    }
}

/// What one Facet region of the frame looks like, each pixel weighed by the mask.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RegionStats {
    /// Share of the frame, 0–1.
    pub share: f32,
    /// Mean display luma, 0–1.
    pub luma: f32,
    /// Mean HSV saturation, 0–1.
    pub sat: f32,
    /// Mean OKLCh hue in degrees of its chromatic pixels; `None` when the region is grey.
    pub hue: Option<f32>,
}

/// `weights` (0–1) are one per pixel of `img`, row by row, drawn in the same frame.
pub fn region_stats(img: &image::RgbImage, weights: &[f32]) -> RegionStats {
    use photos_media::develop::linear::{linear_to_srgb, luma, srgb_to_linear};
    use photos_media::develop::oklab::{lch, to_oklab};
    let (mut total, mut y_sum, mut s_sum, mut chromatic, mut hx, mut hy) = (0f64, 0f64, 0f64, 0f64, 0f64, 0f64);
    for (p, &w) in img.pixels().zip(weights) {
        if w <= 0.0 || !w.is_finite() {
            continue;
        }
        let w = f64::from(w.min(1.0));
        let e = p.0.map(|v| v as f32 / 255.0);
        let lin = e.map(srgb_to_linear);
        total += w;
        y_sum += w * f64::from(linear_to_srgb(luma(lin).clamp(0.0, 1.0)));
        let (hi, lo) = (e[0].max(e[1]).max(e[2]), e[0].min(e[1]).min(e[2]));
        s_sum += w * f64::from(if hi > 0.0 { (hi - lo) / hi } else { 0.0 });
        let (c, h) = lch(to_oklab(lin));
        if c > CHROMA {
            let r = f64::from(h).to_radians();
            (chromatic, hx, hy) = (chromatic + w, hx + w * r.cos(), hy + w * r.sin());
        }
    }
    let n = (img.width() * img.height()).max(1) as f64;
    let mean = |v: f64| if total > 0.0 { (v / total) as f32 } else { 0.0 };
    // A hue only when a tenth of the region is chromatic and the hues don't cancel out.
    let hue = (chromatic > 0.1 * total && hx.hypot(hy) > 0.2 * chromatic).then(|| (hy.atan2(hx).to_degrees().rem_euclid(360.0)) as f32);
    RegionStats { share: (total / n) as f32, luma: mean(y_sum), sat: mean(s_sum), hue }
}

/// Share of the frame (0–1) that Facet's sky region covers; 0 without one.
pub fn sky_share(regions: &[(String, RegionStats)]) -> f32 {
    regions.iter().filter(|(name, _)| name == "sky").map(|(_, r)| r.share).sum::<f32>().min(1.0)
}

fn angle(a: f32, b: f32) -> f32 {
    let d = (a - b).rem_euclid(360.0);
    d.min(360.0 - d)
}

/// How a grade toward hue `look` sits with colour at `bin`: the same or complementary
/// hue 1, a triad 0.5, else 0.
fn affinity(look: f32, bin: f32) -> f32 {
    let d = angle(look, bin);
    if d <= 30.0 || (d - 180.0).abs() <= 30.0 {
        1.0
    } else if (d - 120.0).abs() <= 20.0 {
        0.5
    } else {
        0.0
    }
}

fn centre(i: usize) -> f32 {
    15.0 + 30.0 * i as f32
}

#[derive(Debug, Clone, PartialEq)]
pub struct Fit {
    pub id: String,
    pub score: u8,
    /// Short phrases, e.g. "complements the photo's blue".
    pub reasons: Vec<String>,
}

/// The look's score for this photo: 45 % colour harmony, 30 % tonal key, 25 % content (skin
/// and sky).
pub fn score(look: &Look, c: &PhotoColour) -> Fit {
    let clamp = |v: f32| if v.is_finite() { v.clamp(0.0, 1.0) } else { 0.0 };
    let mut reasons = Vec::new();
    let dominant = (0..12).max_by(|&a, &b| c.hues[a].total_cmp(&c.hues[b])).filter(|&i| c.hues[i] > 0.0);
    let h = if look.safe {
        0.7
    } else if look.mono {
        // Mono throws colour away: the richer the photo's colour (how much of it is chromatic,
        // how saturated), the more it loses.
        let richness = 0.5 * (c.sat / 0.25).min(1.0) + 0.5 * clamp(c.chroma_share / 0.6);
        let h = clamp(1.0 - richness + c.contrast.min(1.0) * 0.1);
        if h >= 0.7 {
            reasons.push(if c.sat < 0.15 { "the photo's colour is muted; tone and shape carry it".into() } else { "strong tonal range for monochrome".into() });
        }
        h
    } else if look.hues.is_empty() {
        // A look that pushes every colour alike suits a photo that already has colour.
        let h = clamp(c.chroma_share * 0.6 + (c.sat / 0.4).min(1.0) * 0.4);
        if h >= 0.6 {
            reasons.push("already colourful".into());
        }
        h
    } else {
        let per_hue = |lh: f32| (0..12).map(|i| c.hues[i] * affinity(lh, centre(i))).sum::<f32>();
        let h = clamp(look.hues.iter().map(|&lh| per_hue(lh)).sum::<f32>() / look.hues.len() as f32);
        if let (true, Some(i)) = (h >= 0.5, dominant) {
            let same = look.hues.iter().any(|&lh| angle(lh, centre(i)) <= 30.0);
            reasons.push(format!("{} the photo's {}", if same { "matches" } else { "complements" }, BIN_NAMES[i]));
        }
        h
    };
    let k = match look.key {
        Key::Low => 1.0 - clamp((c.key - 0.25) / 0.35),
        Key::High => clamp((c.key - 0.4) / 0.3),
        Key::Mid => 1.0 - clamp((c.key - 0.47).abs() / 0.3),
    };
    if k >= 0.7 {
        reasons.push(match look.key {
            Key::Low => "suits its dark key",
            Key::High => "suits its bright key",
            Key::Mid => "suits its middle key",
        }
        .into());
    }
    let mut t = 1.0f32;
    if c.skin && !look.skin_safe {
        t *= 0.3;
        reasons.push("would push skin colour".into());
    }
    if look.likes_sky && c.sky > 0.15 {
        t = (t + 0.2).min(1.0);
        reasons.push("works with the sky".into());
    }
    // Stretched around 65 so a good fit and a poor one are far apart, not 80 against 75.
    let s = 65.0 + (100.0 * (0.45 * h + 0.30 * k + 0.25 * t) - 65.0) * SPREAD;
    Fit { id: look.id.clone(), score: if s.is_finite() { s.round().clamp(0.0, 100.0) as u8 } else { 0 }, reasons }
}

const SPREAD: f32 = 1.35;
pub const THRESHOLD: u8 = 60;
pub const MAX_OFFERED: usize = 4;

/// Natural first, then every look scoring ≥ 60 by score (ties in the order given), at most 4.
pub fn offer(fits: &[Fit]) -> Vec<String> {
    let mut rest: Vec<&Fit> = fits.iter().filter(|f| f.id != "natural" && f.score >= THRESHOLD).collect();
    rest.sort_by_key(|f| std::cmp::Reverse(f.score));
    std::iter::once("natural".to_string()).chain(rest.into_iter().map(|f| f.id.clone())).take(MAX_OFFERED).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mcp::looks::{by_id, catalogue};

    fn fit(id: &str, c: &PhotoColour) -> u8 {
        score(by_id(id).unwrap(), c).score
    }

    fn all(c: &PhotoColour) -> Vec<Fit> {
        catalogue().iter().map(|l| score(l, c)).collect()
    }

    #[test]
    fn a_dark_snowy_forest_suits_mono_and_low_key() {
        let mut hues = [0.0; 12];
        (hues[7], hues[8], hues[0]) = (0.7, 0.2, 0.1);
        let c = PhotoColour { hues, chroma_share: 0.1, key: 0.35, contrast: 0.85, sat: 0.05, skin: false, sky: 0.0 };
        assert!(fit("bw-contrast", &c) >= 60, "{:?}", all(&c));
        assert!(fit("high-key-airy", &c) < 60, "{:?}", all(&c));
    }

    #[test]
    fn a_colour_rich_photo_does_not_suit_mono() {
        // Red and yellow scarves, green grass, blue sky: every hue busy, bright and contrasty.
        let mut hues = [0.0; 12];
        (hues[0], hues[1], hues[3], hues[7]) = (0.3, 0.25, 0.2, 0.25);
        let c = PhotoColour { hues, chroma_share: 0.8, key: 0.5, contrast: 0.85, sat: 0.3, skin: false, sky: 0.0 };
        for id in ["bw-contrast", "bw-soft"] {
            assert!(fit(id, &c) < 60, "{id}: {:?}", all(&c));
        }
        assert!(fit("vivid-pop", &c) >= 60, "{:?}", all(&c));
        // Muted, the same frame is fine in mono.
        let muted = PhotoColour { chroma_share: 0.1, sat: 0.04, ..c };
        assert!(fit("bw-contrast", &muted) >= 60, "{:?}", all(&muted));
    }

    #[test]
    fn scores_spread_over_a_wide_range() {
        let mut hues = [0.0; 12];
        (hues[0], hues[1], hues[3], hues[7]) = (0.3, 0.25, 0.2, 0.25);
        let c = PhotoColour { hues, chroma_share: 0.8, key: 0.5, contrast: 0.85, sat: 0.3, skin: false, sky: 0.0 };
        let scores: Vec<u8> = all(&c).iter().map(|f| f.score).collect();
        let (lo, hi) = (*scores.iter().min().unwrap(), *scores.iter().max().unwrap());
        assert!(hi - lo >= 45, "{:?}", all(&c));
    }

    #[test]
    fn warm_skin_under_blue_sky_suits_teal_orange() {
        let mut hues = [0.0; 12];
        (hues[1], hues[7], hues[4]) = (0.4, 0.4, 0.2);
        let c = PhotoColour { hues, chroma_share: 0.6, key: 0.5, contrast: 0.8, sat: 0.35, skin: true, sky: 0.3 };
        let teal = score(by_id("teal-orange").unwrap(), &c);
        assert!(teal.score >= 60, "{:?}", all(&c));
        assert!(teal.score > fit("cool-moody", &c), "{:?}", all(&c));
        assert!(!teal.reasons.is_empty());
    }

    #[test]
    fn a_grey_photo_is_safe() {
        let c = PhotoColour { key: 0.5, contrast: 0.6, ..Default::default() };
        let fits = all(&c);
        assert!(fits.iter().all(|f| f.score <= 100), "{fits:?}");
        assert!(fit("bw-soft", &c) >= 60, "{fits:?}");
        assert_eq!(offer(&fits)[0], "natural");
    }

    #[test]
    fn offer_caps_at_four() {
        let f = |id: &str, score| Fit { id: id.to_string(), score, reasons: Vec::new() };
        let fits = [f("golden-film", 90), f("natural", 40), f("teal-orange", 95), f("cool-moody", 91), f("low-key", 94), f("bw-soft", 92), f("night-neon", 93), f("faded-matte", 59)];
        assert_eq!(offer(&fits), ["natural", "teal-orange", "low-key", "night-neon"]);
        assert_eq!(offer(&[f("natural", 10), f("bw-soft", 59)]), ["natural"]);
    }

    #[test]
    fn region_stats_weigh_the_pixels_under_the_mask() {
        // Left half blue, right half grey; the mask covers the left half.
        let img = image::RgbImage::from_fn(20, 10, |x, _| image::Rgb(if x < 10 { [40, 90, 220] } else { [128, 128, 128] }));
        let weights: Vec<f32> = (0..200).map(|i| if i % 20 < 10 { 1.0 } else { 0.0 }).collect();
        let r = region_stats(&img, &weights);
        assert!((r.share - 0.5).abs() < 1e-3, "{r:?}");
        assert!(r.sat > 0.7, "only the blue counts: {r:?}");
        let hue = r.hue.unwrap();
        assert!((230.0..290.0).contains(&hue), "blue: {hue}");
        let grey = region_stats(&img, &weights.iter().map(|w| 1.0 - w).collect::<Vec<_>>());
        assert_eq!(grey.hue, None, "{grey:?}");
        let none = region_stats(&img, &[0.0; 200]);
        assert!(none.share == 0.0 && none.luma.is_finite() && none.hue.is_none(), "{none:?}");
        assert!((sky_share(&[("subject".into(), r), ("sky".into(), grey)]) - 0.5).abs() < 1e-3);
    }

    #[test]
    fn photo_colour_reads_a_flat_red_image_as_one_bin() {
        use photos_media::develop::linear::{linear_to_srgb, luma, srgb_to_linear};
        let px = [200u8, 30, 30];
        let img = image::RgbImage::from_pixel(64, 48, image::Rgb(px));
        let c = photo_colour(&img, false, 0.0);
        assert!(c.hues.iter().any(|&h| h >= 0.95), "{c:?}");
        assert!((c.hues.iter().sum::<f32>() - 1.0).abs() < 1e-3);
        assert!(c.chroma_share > 0.99);
        let y = linear_to_srgb(luma(px.map(|v| srgb_to_linear(v as f32 / 255.0))));
        assert!((c.key - y).abs() < 0.02, "{} vs {y}", c.key);
        assert!(c.contrast < 0.02 && c.sat > 0.8, "{c:?}");
    }
}
