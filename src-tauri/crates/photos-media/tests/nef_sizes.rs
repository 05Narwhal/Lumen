//! Round 19 §16: are NEFs developed at the camera's full resolution?
//!
//! `LUMEN_NEF=<a copy of a standard NEF>` (and optionally `LUMEN_HE_NEF=<a copy of an HE / HE*
//! NEF>`) — copies, in a temp folder, never a library. Prints one table row per file: the
//! camera's pixel count (ExifTool), `decode_linear`'s size, Develop's base without a size cap
//! (what an *Original size* export renders), and the two stand-ins (the converter's DNG when
//! Adobe DNG Converter is installed, the camera's JPEG). The DNG cache is a temp folder; nothing
//! is written beside the NEF.
//!
//! `LUMEN_PHOTOS_RAW=<path of photos_raw.dll, from cargo build -p photos-raw --release>` must be
//! set when the library is not beside the test binary.
//!
//! `cargo test -p photos-media --test nef_sizes -- --ignored --nocapture`
//!
//! A few pixels of sensor border (the active-area crop) are expected; anything smaller than the
//! sensor minus its border fails.

use std::path::{Path, PathBuf};

use photos_media::develop::{self, raw, stand_in};
use photos_media::exiftool::ExifTool;
use photos_media::hdr::HdrKind;
use photos_media::{decode_preview, MediaKind};

/// Width x height of the full sensor image as ExifTool lists it (`Composite:ImageSize`, which
/// includes the border the active-area crop later removes).
fn camera_size(path: &Path) -> Option<(u32, u32)> {
    let tool = ExifTool::locate()?;
    let mut session = tool.session().map_err(|e| eprintln!("exiftool session: {e:?}")).ok()?;
    let v = session.read_json(path, &[]).map_err(|e| eprintln!("exiftool read: {e:?}")).ok()?;
    let v = v.get(0).unwrap_or(&v);
    let text = |k: &str| v.get(k).and_then(|x| x.as_str()).map(str::to_owned);
    let size = text("Composite:ImageSize").or_else(|| text("ImageSize"))?;
    let mut it = size.split(['x', ' ']).filter_map(|p| p.trim().parse::<u32>().ok());
    it.next().zip(it.next())
}

fn dims(w: u32, h: u32) -> String {
    format!("{w}x{h} ({:.1} MP)", (w as f64 * h as f64) / 1e6)
}

/// Landscape-first, so a rotated file compares with its sensor.
fn norm(w: u32, h: u32) -> (u32, u32) {
    (w.max(h), w.min(h))
}

fn row(label: &str, path: &Path) -> Vec<String> {
    let mut out = vec![format!("== {label}: {}", path.file_name().unwrap().to_string_lossy())];
    let cam = camera_size(path);
    out.push(format!("camera (ExifTool): {}", cam.map(|(w, h)| dims(w, h)).unwrap_or_else(|| "unavailable".into())));
    // What the border may be: the sensor minus a margin of a few dozen pixels.
    let big_enough = |what: &str, w: u32, h: u32, out: &mut Vec<String>| {
        out.push(format!("{what}: {}", dims(w, h)));
        if let Some((cw, ch)) = cam {
            let ((w, h), (cw, ch)) = (norm(w, h), norm(cw, ch));
            assert!(w + 64 >= cw && h + 64 >= ch, "{what} {w}x{h} is smaller than the camera's {cw}x{ch} by more than a border");
        }
    };

    match raw::decode_linear(path) {
        Ok(l) => big_enough("decode_linear (rawler)", l.width, l.height, &mut out),
        Err(e) => out.push(format!("decode_linear (rawler): refused — {e:?}")),
    }
    match develop::load_base_as(path, MediaKind::Raw, None, HdrKind::None, 1, true, None) {
        Ok(l) => {
            big_enough(&format!("Develop base, no cap ({:?}) = Original size export", l.rendering), l.linear.width, l.linear.height, &mut out);
            out.push(format!("  full_long: {}", l.full_long));
        }
        Err(e) => out.push(format!("Develop base: failed — {e:?}")),
    }
    match decode_preview(path, MediaKind::Raw) {
        Ok(p) => out.push(format!("camera JPEG stand-in: {}", dims(p.width(), p.height()))),
        Err(e) => out.push(format!("camera JPEG stand-in: {e:?}")),
    }
    match stand_in::converter() {
        None => out.push("converter DNG stand-in: Adobe DNG Converter not found".into()),
        Some(_) => {
            let cache = tempfile::tempdir().unwrap();
            stand_in::configure(cache.path());
            match stand_in::develop_base(path) {
                Some(Ok(stand_in::Base::Sensor(l))) => big_enough("converter DNG stand-in", l.width, l.height, &mut out),
                Some(Ok(stand_in::Base::Camera)) => out.push("converter DNG stand-in: fell back to the camera JPEG".into()),
                Some(Err(e)) => out.push(format!("converter DNG stand-in: failed — {e:?}")),
                None => out.push("converter DNG stand-in: not configured".into()),
            }
        }
    }
    out
}

#[test]
#[ignore = "needs LUMEN_NEF=<a copy of a NEF>"]
fn nefs_develop_at_the_cameras_full_resolution() {
    let nef = PathBuf::from(std::env::var("LUMEN_NEF").expect("LUMEN_NEF"));
    let mut lines = row("standard NEF", &nef);
    if let Ok(he) = std::env::var("LUMEN_HE_NEF") {
        lines.extend(row("HE NEF", Path::new(&he)));
    }
    eprintln!("\n{}\n", lines.join("\n"));
}
