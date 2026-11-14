//! Recovering what a damaged photo or video still holds (Round 22 §4): the rows that decode
//! before the damage, else the largest embedded preview, else for a video what FFmpeg can remux.
//! Writes one file beside `out_stem` and nothing else; the original is only read.

use std::path::{Path, PathBuf};

use image::{DynamicImage, RgbImage};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::exiftool::ExifTool;
use crate::ffmpeg::{cmd, Ffmpeg};
use crate::kind::{content_kind, kind_for_extension, MediaKind};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum RecoveredKind {
    /// The top of the photo, cropped where the damage starts.
    Partial,
    /// Only an embedded preview could be read.
    Preview,
    /// The start of a video, remuxed.
    Video,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Recovered {
    pub kind: RecoveredKind,
    /// Recovered pixels (photo) or length (video) over what the file's header announces, 1..=100.
    pub percent: u8,
    pub width: u32,
    pub height: u32,
}

/// A partial photo shorter than this, or than 5 % of the header's height, is not worth keeping.
const MIN_ROWS: u32 = 64;
/// A recovered video shorter than this is nothing.
const MIN_VIDEO_MS: i64 = 1000;
/// Two pixels within this per channel count as the same colour (the decoder's fill is exact,
/// colour conversion is not).
const FLAT_TOLERANCE: u8 = 2;

/// Recovers what `original` still holds into `<out_stem>.jpg` or `<out_stem>.mp4`. `file_name`
/// is the file's real name (its extension decides when the bytes do not). `None` when nothing is
/// readable; then no output file exists.
pub fn recover(
    original: &Path,
    file_name: &str,
    out_stem: &Path,
    ffmpeg: &Ffmpeg,
    exiftool: Option<&ExifTool>,
) -> Option<(PathBuf, Recovered)> {
    let ext = Path::new(file_name).extension().and_then(|e| e.to_str()).unwrap_or("");
    let kind = content_kind(original).unwrap_or_else(|| kind_for_extension(ext));
    if let Some(dir) = out_stem.parent() {
        std::fs::create_dir_all(dir).ok()?;
    }
    if kind == MediaKind::Video {
        video(original, out_stem, ffmpeg)
    } else {
        photo(original, out_stem, ffmpeg, exiftool)
    }
}

fn with_suffix(stem: &Path, suffix: &str) -> PathBuf {
    let mut s = stem.as_os_str().to_owned();
    s.push(suffix);
    PathBuf::from(s)
}

fn photo(original: &Path, out_stem: &Path, ffmpeg: &Ffmpeg, exiftool: Option<&ExifTool>) -> Option<(PathBuf, Recovered)> {
    let decoded = decode_tolerant(original, &with_suffix(out_stem, ".part.png"), ffmpeg);
    let probed = ffmpeg.probe(original).ok().and_then(|p| Some((p.width?, p.height?)));
    // A tiled HEIC's first stream is one tile: the header is at least what decoded.
    let header = match (probed.or_else(|| exif_dims(original)), decoded.as_ref()) {
        (Some((w, h)), Some(img)) => Some((w.max(img.width()), h.max(img.height()))),
        (Some(d), None) => Some(d),
        (None, Some(img)) => Some((img.width(), img.height())),
        (None, None) => None,
    };

    let partial = decoded.and_then(|img| {
        let (_, header_h) = header?;
        let rows = kept_rows(&img);
        if rows < MIN_ROWS || (rows as u64) * 100 < header_h as u64 * 5 {
            return None;
        }
        let w = img.width();
        Some(image::imageops::crop_imm(&img, 0, 0, w, rows).to_image())
    });
    let preview = largest_preview(original, exiftool);

    let out = with_suffix(out_stem, ".jpg");
    let pixels = |w: u32, h: u32| w as u64 * h as u64;
    let header_pixels = header.map(|(w, h)| pixels(w, h)).unwrap_or(0);
    let percent_of = |px: u64| -> u8 { (px * 100).checked_div(header_pixels).map_or(1, |p| p.clamp(1, 100) as u8) };
    let preview_px = preview.as_ref().map(|(_, w, h)| pixels(*w, *h)).unwrap_or(0);
    let result = match (partial, preview) {
        (Some(img), _) if pixels(img.width(), img.height()) > preview_px => {
            let (w, h) = (img.width(), img.height());
            crate::thumbs::encode_jpeg(&DynamicImage::ImageRgb8(img), 95, &out).ok()?;
            Recovered { kind: RecoveredKind::Partial, percent: percent_of(pixels(w, h)), width: w, height: h }
        }
        (_, Some((bytes, w, h))) => {
            // Already a JPEG: written as it is rather than encoded a second time.
            std::fs::write(&out, bytes).ok()?;
            Recovered { kind: RecoveredKind::Preview, percent: percent_of(pixels(w, h)), width: w, height: h }
        }
        (None, None) => return None,
        (Some(_), None) => unreachable!("a partial with no preview always has more pixels"),
    };
    if let Some(et) = exiftool {
        copy_tags(et, original, &out, result.width, result.height);
    }
    Some((out, result))
}

/// One frame decoded with errors ignored, pixels as stored (no rotation: the tags copied from
/// the original carry the orientation). The temp PNG is always removed.
fn decode_tolerant(original: &Path, tmp_png: &Path, ffmpeg: &Ffmpeg) -> Option<RgbImage> {
    let ran = cmd(&ffmpeg.ffmpeg)
        .args(["-v", "error", "-y", "-noautorotate", "-err_detect", "ignore_err", "-flags", "+output_corrupt", "-i"])
        .arg(original)
        .args(["-frames:v", "1", "-f", "image2", "-c:v", "png"])
        .arg(tmp_png)
        .output();
    let img = ran.ok().filter(|_| tmp_png.is_file()).and_then(|_| image::open(tmp_png).ok()).map(|i| i.to_rgb8());
    let _ = std::fs::remove_file(tmp_png);
    img
}

/// Rows kept after dropping, from the bottom, every row that is one flat colour and the same
/// colour as the row under it: the decoder's fill where the data ran out.
pub fn kept_rows(img: &RgbImage) -> u32 {
    let mut keep = img.height();
    let mut below: Option<[u8; 3]> = None;
    while keep > 0 {
        let Some(c) = flat_row(img, keep - 1) else { break };
        if below.is_some_and(|b| !close(b, c)) {
            break;
        }
        below = Some(c);
        keep -= 1;
    }
    keep
}

fn close(a: [u8; 3], b: [u8; 3]) -> bool {
    a.iter().zip(b).all(|(x, y)| x.abs_diff(y) <= FLAT_TOLERANCE)
}

fn flat_row(img: &RgbImage, y: u32) -> Option<[u8; 3]> {
    let first = img.get_pixel(0, y).0;
    (0..img.width()).all(|x| close(img.get_pixel(x, y).0, first)).then_some(first)
}

/// The largest embedded JPEG (ExifTool's `PreviewImage`, `JpgFromRaw`, `ThumbnailImage`, else
/// the EXIF thumbnail read in Rust) as its bytes and size.
fn largest_preview(original: &Path, exiftool: Option<&ExifTool>) -> Option<(Vec<u8>, u32, u32)> {
    let mut found: Vec<Vec<u8>> = Vec::new();
    if let Some(et) = exiftool {
        for tag in ["-PreviewImage", "-JpgFromRaw", "-ThumbnailImage"] {
            if let Ok(o) = cmd(&et.program).args(["-b", tag]).arg(original).output() {
                found.push(o.stdout);
            }
        }
    }
    if let Some((jpeg, _)) = read_head(original).and_then(|head| crate::metadata::exif_thumbnail(&head)) {
        found.push(jpeg);
    }
    found
        .into_iter()
        .filter(|b| b.starts_with(&[0xFF, 0xD8]))
        .filter_map(|b| {
            let img = image::load_from_memory(&b).ok()?;
            Some((b, img.width(), img.height()))
        })
        .max_by_key(|(_, w, h)| *w as u64 * *h as u64)
}

fn read_head(path: &Path) -> Option<Vec<u8>> {
    use std::io::Read;
    let mut head = Vec::new();
    std::fs::File::open(path).ok()?.take(512 * 1024).read_to_end(&mut head).ok()?;
    Some(head)
}

/// The pixel size EXIF announces, for a file cut before its image data.
fn exif_dims(path: &Path) -> Option<(u32, u32)> {
    let head = read_head(path)?;
    let exif = exif::Reader::new().read_from_container(&mut std::io::Cursor::new(head)).ok()?;
    let get = |tag| exif.get_field(tag, exif::In::PRIMARY).and_then(|f| f.value.get_uint(0));
    Some((get(exif::Tag::PixelXDimension)?, get(exif::Tag::PixelYDimension)?))
}

/// Dates, camera and orientation come along so the recovered photo reads like the original.
/// Best effort: a damaged file may have nothing ExifTool can read.
fn copy_tags(et: &ExifTool, original: &Path, out: &Path, w: u32, h: u32) {
    let _ = cmd(&et.program)
        .args(["-q", "-q", "-m", "-overwrite_original", "-TagsFromFile"])
        .arg(original)
        .args(["-EXIF:all", "--IFD1:all", "-XMP:all"])
        .arg(format!("-ExifImageWidth={w}"))
        .arg(format!("-ExifImageHeight={h}"))
        .arg(out)
        .output();
}

fn video(original: &Path, out_stem: &Path, ffmpeg: &Ffmpeg) -> Option<(PathBuf, Recovered)> {
    let out = with_suffix(out_stem, ".mp4");
    let ran = cmd(&ffmpeg.ffmpeg)
        .args(["-v", "error", "-y", "-err_detect", "ignore_err", "-i"])
        .arg(original)
        .args(["-c", "copy", "-ignore_unknown", "-f", "mp4"])
        .arg(&out)
        .output();
    let got = ran.ok().filter(|o| o.status.success() && out.is_file()).and_then(|_| ffmpeg.probe(&out).ok());
    let Some(got) = got.filter(|p| p.duration_ms.unwrap_or(0) >= MIN_VIDEO_MS) else {
        let _ = std::fs::remove_file(&out);
        return None;
    };
    let length = got.duration_ms.unwrap_or(0);
    let percent = match ffmpeg.probe(original).ok().and_then(|p| p.duration_ms).filter(|d| *d > 0) {
        Some(header) => (length * 100 / header).clamp(1, 100) as u8,
        None => {
            let size = |p: &Path| std::fs::metadata(p).map(|m| m.len()).unwrap_or(0);
            (size(&out) * 100 / size(original).max(1)).clamp(1, 99) as u8
        }
    };
    let (w, h) = got.display_size();
    Some((out, Recovered { kind: RecoveredKind::Video, percent, width: w.unwrap_or(0), height: h.unwrap_or(0) }))
}
