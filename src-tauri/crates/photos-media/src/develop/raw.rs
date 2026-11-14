//! A RAW file developed from its sensor data (spec §3, stage 1): rawler scales to the black
//! and white levels, demosaics (PPG for Bayer, bilinear for X-Trans and four-colour sensors),
//! crops to the active area, applies the camera's as-shot white balance and its colour matrix
//! (camera → XYZ D65 → linear sRGB), and crops to the default frame. Everything rawler does
//! except its final sRGB gamma, so the result is this pipeline's working space.
//!
//! The pixels are the sensor's orientation, like the embedded preview: the catalogue's
//! orientation applies to both alike.

use std::path::Path;

use super::Linear;
use crate::MediaError;

/// The start of the `MediaError::Unsupported` text of a RAW that rawler refuses because of what
/// it is: a Nikon High Efficiency NEF (TicoRAW, no open decoder exists), a body or a format it
/// doesn't know (`docs/RAW-FORMATS.md`). Callers turn it into a stand-in or a plain sentence;
/// an I/O error is never one.
pub const REFUSED: &str = "raw-refused";

/// Whether a media error says rawler refuses this kind of file.
pub fn is_refused(e: &MediaError) -> bool {
    matches!(e, MediaError::Unsupported(m) if m.starts_with(REFUSED))
}

pub(crate) fn refused(why: impl std::fmt::Display) -> MediaError {
    MediaError::Unsupported(format!("{REFUSED}: {why}"))
}

/// The refusal's text when the library holding rawler isn't there (`raw_lib`): a state of this
/// install, not of the file, so nothing about the file is remembered from it.
const NO_LIBRARY: &str = "the RAW library is not available";

pub(crate) fn no_library() -> MediaError {
    refused(NO_LIBRARY)
}

/// Whether this refusal only says the library is missing.
pub fn is_no_library(e: &MediaError) -> bool {
    matches!(e, MediaError::Unsupported(m) if m.ends_with(NO_LIBRARY))
}

/// Whether rawler's calibrated output cannot be the photo: almost every pixel at or past
/// white. Samsung Galaxy S21–S23 DNGs decode like this, even through Adobe DNG Converter
/// (`docs/RAW-FORMATS.md`); a sunlit snowfield still has shadows.
pub fn implausible(img: &Linear) -> bool {
    let (mut white, mut n) = (0usize, 0usize);
    for p in img.data.iter().step_by(7) {
        n += 1;
        if super::linear::luma(*p) >= 1.0 {
            white += 1;
        }
    }
    n > 0 && white as f64 >= 0.97 * n as f64
}

/// Lumen's default rendering of sensor data, the way a camera profile is a raw converter's:
/// part of the source, not the recipe. Sensor data is scene-linear and a stop or more darker
/// and flatter than a camera's JPEG; this adds a baseline exposure with a Reinhard shoulder
/// (so what the baseline pushes past white rolls off rather than clipping), then mild contrast
/// and vibrance. Tuned on a Nikon Z 6II sample from raw.pixls.us to the camera JPEG's mean
/// brightness (0.227 vs 0.227), with less of its saturation and none of its sharpening.
pub const BASE_EV: f32 = 1.5;
pub const BASE_CONTRAST: f32 = 0.4;
pub const BASE_VIBRANCE: f32 = 0.45;

pub fn base_look(img: &mut Linear) {
    use rayon::prelude::*;
    let white = 2f32.powf(BASE_EV);
    img.data.par_iter_mut().for_each(|p| {
        let l = super::linear::luma(*p);
        if l > 1e-7 {
            let l1 = l * white;
            let k = l1 * (1.0 + l1 / (white * white)) / (1.0 + l1) / l;
            *p = [p[0] * k, p[1] * k, p[2] * k];
        }
    });
    super::basic::apply_tones(img, &photos_core::edits::Basic { contrast: BASE_CONTRAST, vibrance: BASE_VIBRANCE, ..Default::default() });
}

/// Sensor data with the base look: what Develop starts from for a RAW file.
pub fn decode(path: &Path) -> Result<Linear, MediaError> {
    let mut img = decode_linear(path)?;
    if implausible(&img) {
        return Err(refused("the sensor data decodes almost entirely white"));
    }
    base_look(&mut img);
    Ok(img)
}

/// Sensor data as rawler calibrates it, before the base look (`photos-raw`, `raw_lib`).
pub fn decode_linear(path: &Path) -> Result<Linear, MediaError> {
    let lib = crate::raw_lib::current().ok_or_else(no_library)?;
    let (width, height, data) = lib.decode_linear(path)?;
    Ok(Linear { width, height, data })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_all_but_white_decode_is_implausible() {
        // Samsung Galaxy S21–S23 DNGs decode to this (docs/RAW-FORMATS.md).
        assert!(implausible(&Linear::new(20, 10, [1.0; 3])));
        assert!(implausible(&Linear::new(20, 10, [40.0, 38.0, 41.0])));
        // A snowfield in sunshine is bright, not a failure.
        let mut snow = Linear::new(20, 10, [1.0; 3]);
        for p in snow.data.iter_mut().step_by(4) {
            *p = [0.3, 0.32, 0.35];
        }
        assert!(!implausible(&snow));
        assert!(!implausible(&Linear::new(20, 10, [0.18; 3])));
    }

    #[test]
    fn a_missing_library_is_told_apart_from_a_refused_file() {
        assert!(is_refused(&no_library()) && is_no_library(&no_library()));
        assert!(!is_no_library(&refused("Unknown camera")));
    }

    #[test]
    fn a_missing_file_is_an_io_error_not_a_refusal() {
        let dir = tempfile::tempdir().unwrap();
        let got = decode(&dir.path().join("unplugged.NEF"));
        assert!(matches!(got, Err(MediaError::Io(_))), "{got:?}");
    }

    #[test]
    fn a_file_with_no_decoder_is_a_refusal() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("new-format.NEF");
        std::fs::write(&p, vec![7u8; 4096]).unwrap();
        let got = decode(&p);
        assert!(got.as_ref().err().is_some_and(is_refused), "{got:?}");
    }

    /// `LUMEN_HE_NEF=<a Nikon HE / HE* NEF>`: the real file is recognised, not just the text.
    #[test]
    #[ignore = "needs LUMEN_HE_NEF"]
    fn a_real_he_nef_is_recognised() {
        let path = std::path::PathBuf::from(std::env::var("LUMEN_HE_NEF").expect("LUMEN_HE_NEF"));
        let e = decode(&path).unwrap_err();
        assert!(is_refused(&e), "{e:?}");
    }

    /// `LUMEN_RAW_SAMPLE=<file>`; a CC0 sample comes from raw.pixls.us.
    #[test]
    #[ignore = "needs LUMEN_RAW_SAMPLE"]
    fn the_sensor_data_is_the_photo_the_camera_previewed() {
        let path = std::path::PathBuf::from(std::env::var("LUMEN_RAW_SAMPLE").expect("LUMEN_RAW_SAMPLE"));
        let t = std::time::Instant::now();
        let lin = decode(&path).unwrap();
        eprintln!("sensor decode: {}x{} in {:?}", lin.width, lin.height, t.elapsed());
        let preview = crate::decode_preview(&path, crate::MediaKind::Raw).unwrap();
        let (pw, ph) = (preview.width() as f32, preview.height() as f32);
        let aspect = |w: f32, h: f32| w.max(h) / w.min(h);
        assert!((aspect(lin.width as f32, lin.height as f32) / aspect(pw, ph) - 1.0).abs() < 0.01, "{}x{} vs {pw}x{ph}", lin.width, lin.height);
        let mean = |l: &Linear| l.data.iter().map(|p| super::super::linear::luma(*p) as f64).sum::<f64>() / l.data.len() as f64;
        let prev = Linear::from_dynamic(&preview);
        let stops = (mean(&lin) / mean(&prev)).log2();
        eprintln!("mean luminance: sensor {:.4}, preview {:.4} ({stops:+.2} stops)", mean(&lin), mean(&prev));
        assert!(stops.abs() < 0.5, "the base look should land near the camera's brightness: {stops:+.2} stops");
        let chroma = lin.data.iter().step_by(97).map(|p| (p[0] - p[1]).abs() + (p[2] - p[1]).abs()).sum::<f32>();
        assert!(chroma > 0.0, "a colour camera gives colour");
    }
}
