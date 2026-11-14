//! What Retouch's model reads (Round 11B): the photo's uncropped, display-oriented base at
//! `RETOUCH_EDGE` px, log-encoded, padded to a square with a fourth channel saying which
//! pixels are the photo; and `FEATURES` numbers measured on it. The app and the training
//! scripts (through `examples/retouch_prep.rs`) both call `prepare`, so they see the same.

use std::path::{Path, PathBuf};

use image::RgbImage;
use photos_core::develop_auto::measure;
use photos_core::edits::Recipe;

use crate::develop::Linear;
use crate::error::MediaError;

pub const RETOUCH_EDGE: u32 = 256;
/// Histogram bins over `HIST_STOPS` of luminance, in stops about middle grey.
pub const HIST_BINS: usize = 64;
pub const HIST_STOPS: (f32, f32) = (-10.0, 4.0);
/// 13 measures, 3 channel means, then the histogram.
pub const FEATURES: usize = 16 + HIST_BINS;

/// The model's input for one photo.
#[derive(Debug, Clone, PartialEq)]
pub struct Prepared {
    /// `4 × RETOUCH_EDGE × RETOUCH_EDGE`, channel-major: R, G, B (`encode`d), then 1 where
    /// the photo is and 0 in the padding.
    pub input: Vec<f32>,
    /// The photo's size inside the square, from its top-left corner.
    pub width: u32,
    pub height: u32,
    /// Width / height of the photo.
    pub aspect: f32,
    pub features: Vec<f32>,
}

/// Linear light to the model's range: log2(x + 1/64), 0 at black, 1 at four times white.
pub fn encode(x: f32) -> f32 {
    if x.is_nan() || x <= 0.0 {
        return 0.0;
    }
    ((x + 1.0 / 64.0).log2() + 6.0) / 8.0
}

/// Stops about middle grey, bounded so a black frame stays finite.
fn stops(v: f32) -> f32 {
    if v.is_finite() {
        (v.max(1e-6) / 0.18).log2().clamp(-20.0, 10.0)
    } else {
        0.0
    }
}

/// The model's input for `base`: the whole photo (no crop), as it is displayed.
pub fn prepare(base: &Linear) -> Prepared {
    let e = RETOUCH_EDGE as usize;
    let mut input = vec![0.0; 4 * e * e];
    if base.data.is_empty() || base.width == 0 || base.height == 0 {
        return Prepared { input, width: 0, height: 0, aspect: 1.0, features: features(&[], 1.0) };
    }
    let small = if base.long_edge() > RETOUCH_EDGE { base.resize_long_edge(RETOUCH_EDGE) } else { base.clone() };
    let (w, h) = (small.width as usize, small.height as usize);
    for y in 0..h {
        for x in 0..w {
            let p = small.data[y * w + x];
            for c in 0..3 {
                input[c * e * e + y * e + x] = encode(p[c]);
            }
            input[3 * e * e + y * e + x] = 1.0;
        }
    }
    let aspect = base.width as f32 / base.height as f32;
    Prepared { input, width: small.width, height: small.height, aspect, features: features(&small.data, aspect) }
}

/// `p` with every part that exists resolved (`..`, links, the disk's own spelling), so two
/// spellings of one folder compare equal; the parts that do not exist yet are kept as given.
fn resolved(p: &Path) -> std::io::Result<PathBuf> {
    let abs = std::path::absolute(p)?;
    let mut existing = abs.as_path();
    let mut rest = Vec::new();
    loop {
        if let Ok(c) = existing.canonicalize() {
            return Ok(rest.iter().rev().fold(c, |acc, part| acc.join(part)));
        }
        match (existing.parent(), existing.file_name()) {
            (Some(parent), Some(name)) => {
                rest.push(name.to_os_string());
                existing = parent;
            }
            _ => return Ok(abs),
        }
    }
}

/// Refuses `out` when it is `sources` or inside it, however either is spelt: prepared data
/// never lands among the photos it was made from.
pub fn check_destination(out: &Path, sources: &Path) -> Result<(), MediaError> {
    if resolved(out)?.starts_with(resolved(sources)?) {
        return Err(MediaError::Unsupported(format!("will not write inside the photos' folder: {}", out.display())));
    }
    Ok(())
}

/// Auto's Basic for a prepared base: the measured prior (the app's Auto without its model),
/// the baseline Retouch is evaluated against.
pub fn auto_basic(base: &Linear) -> photos_core::edits::Basic {
    let small = if base.long_edge() > RETOUCH_EDGE { base.resize_long_edge(RETOUCH_EDGE) } else { base.clone() };
    photos_core::develop_auto::prior(&measure(&small.data), &photos_core::develop_auto::Scene::default())
}

/// The input as `input.u16` stores it.
pub fn to_bytes(p: &Prepared) -> Vec<u8> {
    p.input.iter().flat_map(|&v| ((v.clamp(0.0, 1.0) * 65535.0).round() as u16).to_le_bytes()).collect()
}

/// The photo back from `input.u16` bytes (`to_bytes`), in linear light, at its prepared size.
pub fn read_input(bytes: &[u8]) -> Result<Linear, MediaError> {
    let e = RETOUCH_EDGE as usize;
    if bytes.len() != 4 * e * e * 2 {
        return Err(MediaError::Decode(format!("a prepared input is {} bytes, not {}", 4 * e * e * 2, bytes.len())));
    }
    let at = |i: usize| u16::from_le_bytes([bytes[2 * i], bytes[2 * i + 1]]) as f32 / 65535.0;
    let mask = 3 * e * e;
    let w = (0..e).take_while(|&x| at(mask + x) > 0.5).count();
    let h = (0..e).take_while(|&y| at(mask + y * e) > 0.5).count();
    let mut img = Linear::new(w as u32, h as u32, [0.0; 3]);
    for y in 0..h {
        for x in 0..w {
            img.data[y * w + x] = std::array::from_fn(|c| (2f32.powf(at(c * e * e + y * e + x) * 8.0 - 6.0) - 1.0 / 64.0).max(0.0));
        }
    }
    Ok(img)
}

/// Prepares one photo into `out/<stem>-<hash of its path>/`: `input.u16` (the input,
/// little-endian, 0–65535 for 0–1) and `meta.json` (source, size, aspect, features, and with
/// `auto` Auto's measured Basic). Never writes inside the photo's own folder.
pub fn prep_file(path: &Path, out: &Path, auto: bool) -> Result<PathBuf, MediaError> {
    check_destination(out, path.parent().unwrap_or(Path::new(".")))?;
    let base = load(path, RETOUCH_EDGE * 4)?.0;
    let p = prepare(&base);
    let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let dir = out.join(format!("{stem}-{:016x}", fnv(path.to_string_lossy().as_bytes())));
    std::fs::create_dir_all(&dir)?;
    std::fs::write(dir.join("input.u16"), to_bytes(&p))?;
    let mut meta = serde_json::json!({
        "source": path.to_string_lossy(),
        "edge": RETOUCH_EDGE,
        "width": p.width,
        "height": p.height,
        "aspect": p.aspect,
        "features": p.features,
    });
    if auto {
        meta["auto"] = serde_json::to_value(auto_basic(&base)).map_err(|e| MediaError::Decode(e.to_string()))?;
    }
    std::fs::write(dir.join("meta.json"), serde_json::to_vec_pretty(&meta).map_err(|e| MediaError::Decode(e.to_string()))?)?;
    Ok(dir)
}

/// `recipe` rendered on the photo at `edge` px long edge, as the app's export would.
pub fn render_file(path: &Path, recipe: &Recipe, edge: u32) -> Result<RgbImage, MediaError> {
    let (base, full) = load(path, edge)?;
    let ctx = crate::develop::Ctx::new(base.long_edge() as f32 / full.max(1) as f32, 0);
    Ok(crate::develop::render_base(&base, recipe, &ctx))
}

/// The photo's develop base (sensor data for a RAW), display-oriented, at most `edge` px.
fn load(path: &Path, edge: u32) -> Result<(Linear, u32), MediaError> {
    let ext = path.extension().map(|e| e.to_string_lossy().into_owned()).unwrap_or_default();
    let kind = crate::kind::kind_for_extension(&ext);
    let orientation = crate::metadata::read_metadata(path, kind).orientation;
    crate::develop::load_base(path, kind, None, crate::hdr::HdrKind::None, orientation, true, Some(edge))
}

/// FNV-1a: a folder name that is the same for the same path on every run.
fn fnv(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, &b| (h ^ b as u64).wrapping_mul(0x100_0000_01b3))
}

fn features(pixels: &[[f32; 3]], aspect: f32) -> Vec<f32> {
    let mut f = Vec::with_capacity(FEATURES);
    if pixels.is_empty() {
        f.resize(FEATURES, 0.0);
        return f;
    }
    let m = measure(pixels);
    for v in [m.median, m.p_lo, m.p_hi, m.p_25, m.p_95, m.peak_95] {
        f.push(stops(v));
    }
    f.push(m.frac_dark);
    f.push(m.frac_bright);
    f.push(if m.iqr_stops.is_finite() { m.iqr_stops } else { 0.0 });
    match m.grey {
        Some(g) => {
            let g1 = g[1].max(1e-6);
            f.extend([1.0, (g[0].max(1e-6) / g1).log2(), (g[2].max(1e-6) / g1).log2()]);
        }
        None => f.extend([0.0, 0.0, 0.0]),
    }
    f.push(aspect.ln());
    let n = pixels.len() as f32;
    for c in 0..3 {
        f.push(stops(pixels.iter().map(|p| p[c].max(0.0)).sum::<f32>() / n));
    }
    let mut hist = [0.0f32; HIST_BINS];
    let (lo, hi) = HIST_STOPS;
    for p in pixels {
        let s = stops(crate::develop::linear::luma(*p));
        let bin = (((s - lo) / (hi - lo)) * HIST_BINS as f32).floor().clamp(0.0, (HIST_BINS - 1) as f32) as usize;
        hist[bin] += 1.0 / n;
    }
    f.extend(hist);
    f
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ramp(w: u32, h: u32) -> Linear {
        let mut img = Linear::new(w, h, [0.0; 3]);
        for y in 0..h {
            for x in 0..w {
                let v = 0.001 * 2f32.powf(10.0 * x as f32 / w as f32);
                img.data[(y * w + x) as usize] = [v * 1.1, v, v * 0.8];
            }
        }
        img
    }

    #[test]
    fn encode_spans_black_to_beyond_white() {
        assert!(encode(0.0).abs() < 1e-6);
        assert!((encode(4.0 - 1.0 / 64.0) - 1.0).abs() < 1e-5);
        assert!(encode(0.18) > encode(0.05));
        assert_eq!(encode(-1.0), 0.0, "negative light is black");
        assert_eq!(encode(f32::NAN), 0.0);
    }

    #[test]
    fn a_landscape_fills_the_width_and_is_padded_below() {
        let p = prepare(&ramp(3000, 2000));
        let e = RETOUCH_EDGE as usize;
        assert_eq!(p.input.len(), 4 * e * e);
        assert_eq!((p.width, p.height), (256, 171));
        assert!((p.aspect - 1.5).abs() < 1e-6);
        let mask = &p.input[3 * e * e..];
        assert_eq!(mask[170 * e + 255], 1.0);
        assert_eq!(mask[171 * e], 0.0);
        assert_eq!(p.input[171 * e + 10], 0.0, "padding is black");
        assert_eq!(mask.iter().filter(|&&m| m == 1.0).count(), 256 * 171);
    }

    #[test]
    fn a_portrait_fills_the_height_and_is_padded_right() {
        let p = prepare(&ramp(1000, 1500));
        assert_eq!((p.width, p.height), (171, 256));
        let e = RETOUCH_EDGE as usize;
        let mask = &p.input[3 * e * e..];
        assert_eq!(mask[255 * e + 170], 1.0);
        assert_eq!(mask[255 * e + 171], 0.0);
    }

    #[test]
    fn features_have_their_length_and_a_histogram_summing_to_one() {
        let p = prepare(&ramp(800, 600));
        assert_eq!(p.features.len(), FEATURES);
        let hist: f32 = p.features[16..].iter().sum();
        assert!((hist - 1.0).abs() < 1e-4, "{hist}");
        assert!(p.features.iter().all(|f| f.is_finite()));
    }

    #[test]
    fn brighter_photos_read_brighter() {
        let dark = prepare(&Linear::new(64, 64, [0.02; 3]));
        let bright = prepare(&Linear::new(64, 64, [0.5; 3]));
        assert!(bright.features[0] > dark.features[0] + 3.0, "median in stops");
        assert!(bright.input[0] > dark.input[0]);
    }

    fn jpeg(dir: &std::path::Path, name: &str, w: u32, h: u32) -> std::path::PathBuf {
        let img = image::RgbImage::from_fn(w, h, |x, y| image::Rgb([(x % 256) as u8, (y % 256) as u8, 90]));
        let path = dir.join(name);
        img.save(&path).unwrap();
        path
    }

    #[test]
    fn a_prepared_file_holds_the_input_and_its_numbers() {
        let photos = tempfile::tempdir().unwrap();
        let out = tempfile::tempdir().unwrap();
        let path = jpeg(photos.path(), "a.jpg", 300, 200);
        let dir = prep_file(&path, out.path(), true).unwrap();
        assert!(dir.starts_with(out.path()));
        let bytes = std::fs::read(dir.join("input.u16")).unwrap();
        assert_eq!(bytes.len(), 4 * 256 * 256 * 2);
        let meta: serde_json::Value = serde_json::from_slice(&std::fs::read(dir.join("meta.json")).unwrap()).unwrap();
        assert_eq!(meta["width"], 256);
        assert_eq!(meta["height"], 171);
        assert!((meta["aspect"].as_f64().unwrap() - 1.5).abs() < 1e-6);
        assert_eq!(meta["features"].as_array().unwrap().len(), FEATURES);
        assert!(meta["auto"]["exposure"].is_number(), "Auto's Basic, for the evaluation's baseline");
        assert_eq!(meta["source"], path.to_string_lossy().as_ref());
        // The same file prepared again lands in the same folder.
        assert_eq!(prep_file(&path, out.path(), false).unwrap(), dir);
    }

    #[test]
    fn prep_never_writes_inside_the_photos_folder() {
        let photos = tempfile::tempdir().unwrap();
        let path = jpeg(photos.path(), "a.jpg", 64, 48);
        assert!(prep_file(&path, &photos.path().join("prepared"), false).is_err());
        assert!(prep_file(&path, photos.path(), false).is_err());
        assert!(!photos.path().join("prepared").exists());
        // Another spelling of the same folder is the same folder.
        let sub = photos.path().join("sub");
        std::fs::create_dir(&sub).unwrap();
        assert!(prep_file(&path, &sub.join("..").join("prepared"), false).is_err());
        assert!(!photos.path().join("prepared").exists());
    }

    #[test]
    fn a_batch_never_writes_anywhere_under_its_photos() {
        let photos = tempfile::tempdir().unwrap();
        let nested = photos.path().join("2024").join("trip");
        std::fs::create_dir_all(&nested).unwrap();
        // A photo two folders down: its own folder is not the destination, the batch's root is.
        assert!(check_destination(&photos.path().join("prepared"), &nested).is_ok());
        assert!(check_destination(&photos.path().join("prepared"), photos.path()).is_err());
        let elsewhere = tempfile::tempdir().unwrap();
        assert!(check_destination(elsewhere.path(), photos.path()).is_ok());
    }

    #[test]
    fn a_prepared_input_reads_back_as_the_photo() {
        let base = ramp(300, 200);
        let p = prepare(&base);
        let back = read_input(&to_bytes(&p)).unwrap();
        assert_eq!((back.width, back.height), (p.width, p.height));
        let small = base.resize_long_edge(RETOUCH_EDGE);
        for (a, b) in back.data.iter().zip(&small.data) {
            for c in 0..3 {
                assert!((a[c] - b[c]).abs() <= 2e-4 + 2e-3 * b[c], "{a:?} vs {b:?}");
            }
        }
        let (a, b) = (auto_basic(&back), auto_basic(&small));
        assert!((a.exposure - b.exposure).abs() < 0.02 && (a.temperature - b.temperature).abs() < 0.5, "{a:?} vs {b:?}");
        assert!(read_input(&[0; 10]).is_err());
    }

    #[test]
    fn a_recipe_renders_at_the_asked_size() {
        let photos = tempfile::tempdir().unwrap();
        let path = jpeg(photos.path(), "a.jpg", 400, 300);
        let plain = render_file(&path, &photos_core::edits::Recipe::identity(), 128).unwrap();
        assert_eq!((plain.width(), plain.height()), (128, 96));
        let mut brighter = photos_core::edits::Recipe::identity();
        brighter.develop = Some(photos_core::edits::Develop { basic: photos_core::edits::Basic { exposure: 1.0, ..Default::default() }, ..Default::default() });
        let lit = render_file(&path, &brighter, 128).unwrap();
        let sum = |i: &image::RgbImage| i.as_raw().iter().map(|&v| v as u64).sum::<u64>();
        assert!(sum(&lit) > sum(&plain));
    }

    #[test]
    fn an_empty_base_gives_finite_neutral_numbers() {
        let p = prepare(&Linear::new(0, 0, [0.0; 3]));
        assert_eq!(p.features.len(), FEATURES);
        assert!(p.features.iter().all(|f| f.is_finite()));
        assert_eq!((p.width, p.height), (0, 0));
    }
}
