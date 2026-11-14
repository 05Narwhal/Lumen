use std::path::Path;

use image::DynamicImage;

use crate::ffmpeg::{poster_time, Ffmpeg};
use crate::hdr::HdrKind;
use crate::{kind::MediaKind, MediaError};

/// Decode enough of a file to make derivatives from, without FFmpeg (HEIC and video
/// return `Unsupported`). RAW files use the camera's embedded JPEG preview, never a demosaic.
pub fn decode_preview(path: &Path, kind: MediaKind) -> Result<DynamicImage, MediaError> {
    decode_preview_with(path, kind, None)
}

/// [`decode_preview_with`] for a source whose dynamic range is known, so PQ and HLG
/// pixels are tone mapped on the way down to an SDR derivative.
pub fn decode_preview_hdr(path: &Path, kind: MediaKind, ffmpeg: Option<&Ffmpeg>, hdr: HdrKind) -> Result<DynamicImage, MediaError> {
    match kind {
        MediaKind::Heic | MediaKind::Video => {
            let Some(ff) = ffmpeg else {
                return Err(MediaError::Unsupported(format!("{kind:?} needs FFmpeg")));
            };
            let tmp = TempJpeg::new();
            let tone_map = hdr.needs_tone_mapping();
            if kind == MediaKind::Heic {
                ff.decode_heic_with(path, &tmp.0, tone_map)?;
            } else {
                let at = poster_time(ff.probe(path).ok().and_then(|p| p.duration_ms));
                ff.poster_frame_with(path, at, &tmp.0, tone_map)?;
            }
            Ok(image::ImageReader::open(&tmp.0)?.with_guessed_format()?.decode()?)
        }
        _ => decode_native(path, kind),
    }
}

/// [`decode_preview`] with FFmpeg for HEIC (full-size decode) and video (poster frame at
/// 10 % of the duration). FFmpeg applies the container's rotation itself, so the result
/// is already upright.
pub fn decode_preview_with(path: &Path, kind: MediaKind, ffmpeg: Option<&Ffmpeg>) -> Result<DynamicImage, MediaError> {
    match kind {
        MediaKind::Heic | MediaKind::Video => {
            let Some(ff) = ffmpeg else {
                return Err(MediaError::Unsupported(format!("{kind:?} needs FFmpeg")));
            };
            let tmp = TempJpeg::new();
            if kind == MediaKind::Heic {
                ff.decode_heic(path, &tmp.0)?;
            } else {
                let at = poster_time(ff.probe(path).ok().and_then(|p| p.duration_ms));
                ff.poster_frame(path, at, &tmp.0)?;
            }
            Ok(image::ImageReader::open(&tmp.0)?.with_guessed_format()?.decode()?)
        }
        _ => decode_native(path, kind),
    }
}

/// A uniquely named JPEG in the OS temp dir, removed on drop.
struct TempJpeg(std::path::PathBuf);

impl TempJpeg {
    fn new() -> Self {
        use std::sync::atomic::{AtomicU64, Ordering};
        static N: AtomicU64 = AtomicU64::new(0);
        let n = N.fetch_add(1, Ordering::Relaxed);
        Self(std::env::temp_dir().join(format!("lumen-decode-{}-{n}.jpg", std::process::id())))
    }
}

impl Drop for TempJpeg {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn decode_native(path: &Path, kind: MediaKind) -> Result<DynamicImage, MediaError> {
    match kind {
        MediaKind::Image => Ok(image::ImageReader::open(path)?.with_guessed_format()?.decode()?),
        MediaKind::Jxl => {
            let f = std::fs::File::open(path)?;
            let dec = jxl_oxide::integration::JxlDecoder::new(std::io::BufReader::new(f))
                .map_err(|e| MediaError::Decode(e.to_string()))?;
            Ok(DynamicImage::from_decoder(dec)?)
        }
        MediaKind::Raw => decode_raw_preview(path),
        MediaKind::Heic | MediaKind::Video => Err(MediaError::Unsupported(format!("{kind:?} needs FFmpeg"))),
        MediaKind::Unknown => Err(MediaError::Unsupported("unknown format".into())),
    }
}

/// An image decoded smaller than it is (Round 22 §2), with the size it really has.
#[derive(Debug)]
pub struct Reduced {
    pub image: DynamicImage,
    pub real_width: u32,
    pub real_height: u32,
}

/// Whether a decode failed only because the image is bigger than the decoder may allocate
/// (`image`'s limit, 512 MiB by default), as opposed to a damaged file.
pub fn is_too_large(e: &MediaError) -> bool {
    matches!(e, MediaError::Decode(s) if s.to_ascii_lowercase().contains("memory limit exceeded"))
}

/// The long edge Lumen works on for an image too large to decode whole: thumbnails, the
/// display size and Develop's base come from this.
pub const REDUCED_EDGE: u32 = 4096;

static OVERSIZE_BUDGET: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(512 << 20);

/// How much an oversized image's decode may allocate, set once by the app from the machine's
/// memory; 512 MiB until then.
pub fn set_oversize_budget(bytes: u64) {
    OVERSIZE_BUDGET.store(bytes, std::sync::atomic::Ordering::Relaxed);
}

pub fn oversize_budget() -> u64 {
    OVERSIZE_BUDGET.load(std::sync::atomic::Ordering::Relaxed)
}

/// The pixel size in an image file's header, read without decoding it.
pub fn header_size(path: &Path) -> Option<(u32, u32)> {
    image::ImageReader::open(path).ok()?.with_guessed_format().ok()?.into_dimensions().ok()
}

/// Decode `path` with its long edge at most `max_edge`, never allocating more than about
/// `alloc_budget` for it. A non-interlaced PNG is read one row at a time and box-averaged into
/// the target, so it costs one source row plus the output whatever its pixel count; anything
/// else is decoded whole under the budget, then resized. An error for which [`is_too_large`]
/// holds means the budget was not enough.
pub fn decode_reduced(path: &Path, kind: MediaKind, max_edge: u32, alloc_budget: u64) -> Result<Reduced, MediaError> {
    if kind == MediaKind::Image {
        if let Some(r) = png_rows_reduced(path, max_edge)? {
            return Ok(r);
        }
        let mut limits = image::Limits::default();
        limits.max_alloc = Some(alloc_budget);
        let mut reader = image::ImageReader::open(path)?.with_guessed_format()?;
        reader.limits(limits);
        return Ok(fit(reader.decode()?, max_edge));
    }
    Ok(fit(decode_native(path, kind)?, max_edge))
}

fn fit(img: DynamicImage, max_edge: u32) -> Reduced {
    let (real_width, real_height) = (img.width(), img.height());
    let image = if real_width.max(real_height) > max_edge { crate::resize_long_edge(&img, max_edge) } else { img };
    Reduced { image, real_width, real_height }
}

/// [`decode_reduced`]'s PNG path; `None` when the file is not a PNG this can read row by row
/// (interlaced, not a PNG, a header the reader refuses) or is already within `max_edge`.
fn png_rows_reduced(path: &Path, max_edge: u32) -> Result<Option<Reduced>, MediaError> {
    use std::io::{BufReader, Read};
    let mut sig = [0u8; 8];
    let mut f = std::fs::File::open(path)?;
    if f.read_exact(&mut sig).is_err() || sig != [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A] {
        return Ok(None);
    }
    let mut dec = png::Decoder::new(BufReader::new(std::fs::File::open(path)?));
    dec.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let Ok(mut reader) = dec.read_info() else { return Ok(None) };
    let (w, h, interlaced) = {
        let info = reader.info();
        (info.width, info.height, info.interlaced)
    };
    if interlaced || w.max(h) <= max_edge || w == 0 || h == 0 {
        return Ok(None);
    }
    let channels = match reader.output_color_type().0 {
        png::ColorType::Grayscale => 1,
        png::ColorType::GrayscaleAlpha => 2,
        png::ColorType::Rgb => 3,
        png::ColorType::Rgba => 4,
        // EXPAND turns a palette into RGB(A); anything else is not a PNG this path knows.
        png::ColorType::Indexed => return Ok(None),
    };
    let scale = max_edge as f64 / w.max(h) as f64;
    let nw = ((w as f64 * scale).round() as u32).clamp(1, max_edge);
    let nh = ((h as f64 * scale).round() as u32).clamp(1, max_edge);
    // Source column → target column, once.
    let col: Vec<u32> = (0..w).map(|x| (x as u64 * nw as u64 / w as u64) as u32).collect();
    let row_len = nw as usize * channels;
    let mut out = vec![0u8; row_len * nh as usize];
    let mut sum = vec![0u64; row_len];
    let mut count = vec![0u32; nw as usize];
    let mut current = 0u32;
    let flush = |ty: u32, sum: &mut [u64], count: &mut [u32], out: &mut [u8]| {
        let base = ty as usize * row_len;
        for (tx, n) in count.iter_mut().enumerate() {
            for c in 0..channels {
                let i = tx * channels + c;
                out[base + i] = if *n == 0 { 0 } else { ((sum[i] + *n as u64 / 2) / *n as u64) as u8 };
                sum[i] = 0;
            }
            *n = 0;
        }
    };
    for y in 0..h {
        let row = reader.next_row().map_err(|e| MediaError::Decode(e.to_string()))?.ok_or_else(|| MediaError::Decode("unexpected end of file".into()))?;
        let data = row.data();
        let ty = (y as u64 * nh as u64 / h as u64) as u32;
        if ty != current {
            flush(current, &mut sum, &mut count, &mut out);
            current = ty;
        }
        for (x, &tx) in col.iter().enumerate() {
            let (s, t) = (x * channels, tx as usize * channels);
            for c in 0..channels {
                sum[t + c] += data[s + c] as u64;
            }
            count[tx as usize] += 1;
        }
    }
    flush(current, &mut sum, &mut count, &mut out);
    let bad = || MediaError::Decode("reduced buffer".into());
    let image = match channels {
        1 => DynamicImage::ImageLuma8(image::GrayImage::from_raw(nw, nh, out).ok_or_else(bad)?),
        2 => DynamicImage::ImageLumaA8(image::GrayAlphaImage::from_raw(nw, nh, out).ok_or_else(bad)?),
        3 => DynamicImage::ImageRgb8(image::RgbImage::from_raw(nw, nh, out).ok_or_else(bad)?),
        _ => DynamicImage::ImageRgba8(image::RgbaImage::from_raw(nw, nh, out).ok_or_else(bad)?),
    };
    Ok(Some(Reduced { image, real_width: w, real_height: h }))
}

/// Below this long edge a RAW's preview is a thumbnail, not a picture (CHDK DNGs carry 128 px,
/// early Nikons and Leicas 160 px).
const SMALL_PREVIEW: u32 = 640;

fn long_edge(i: &DynamicImage) -> u32 {
    i.width().max(i.height())
}

/// A RAW's picture without developing it: rawler's preview; else, or when that is small, the
/// largest JPEG in the file; else, when still small, the sensor data with the base look; else
/// whatever small preview there was. The same orientation as the sensor's throughout.
fn decode_raw_preview(path: &Path) -> Result<DynamicImage, MediaError> {
    let mut best = rawler_preview(path)?;
    if best.as_ref().is_none_or(|i| long_edge(i) < SMALL_PREVIEW) {
        if let Some(j) = embedded_jpeg(&std::fs::read(path)?) {
            if best.as_ref().is_none_or(|b| long_edge(&j) > long_edge(b)) {
                best = Some(j);
            }
        }
    }
    if best.as_ref().is_none_or(|i| long_edge(i) < SMALL_PREVIEW) {
        if let Ok(lin) = crate::develop::raw::decode(path) {
            return Ok(DynamicImage::ImageRgb8(lin.to_rgb8()));
        }
    }
    best.ok_or_else(|| {
        if crate::raw_lib::current().is_none() {
            return crate::develop::raw::no_library();
        }
        MediaError::Decode("no embedded preview in RAW".into())
    })
}

/// rawler's own preview, full image or thumbnail; `None` when it has none, doesn't know the file
/// or isn't there. Only a file that can't be read is an error.
fn rawler_preview(path: &Path) -> Result<Option<DynamicImage>, MediaError> {
    match crate::raw_lib::current() {
        Some(lib) => lib.preview(path),
        None => Ok(None),
    }
}

/// The largest JPEG inside a file that decodes whole: a RAW's camera preview when rawler doesn't
/// know the body or has no preview for the format (ORF, CRW, SRW…). Lossless-JPEG sensor data
/// does not decode as an image and is passed over. No format is parsed; JPEG streams are found
/// by their start marker.
pub fn embedded_jpeg(bytes: &[u8]) -> Option<DynamicImage> {
    use image::ImageDecoder;
    let mut found: Vec<(u64, usize)> = Vec::new();
    let mut at = 0;
    while let Some(i) = bytes[at..].windows(3).position(|w| w == [0xFF, 0xD8, 0xFF]) {
        let off = at + i;
        if let Ok(dec) = image::codecs::jpeg::JpegDecoder::new(std::io::Cursor::new(&bytes[off..])) {
            let (w, h) = dec.dimensions();
            if w >= 32 && h >= 32 {
                found.push((w as u64 * h as u64, off));
            }
        }
        at = off + 3;
    }
    found.sort_by_key(|a| std::cmp::Reverse(a.0));
    found.into_iter().take(8).find_map(|(_, off)| {
        // A cut-off stream decodes with grey filling the rest: only a whole one counts.
        let end = jpeg_end(&bytes[off..])?;
        image::load_from_memory_with_format(&bytes[off..off + end], image::ImageFormat::Jpeg).ok()
    })
}

/// The length of the JPEG stream at the start of `b`, through its end marker, walking its
/// segments and every scan's entropy-coded data; `None` when it is cut off.
fn jpeg_end(b: &[u8]) -> Option<usize> {
    let mut i = 2;
    loop {
        // Fill bytes may precede a marker.
        while *b.get(i)? == 0xFF && *b.get(i + 1)? == 0xFF {
            i += 1;
        }
        if *b.get(i)? != 0xFF {
            return None;
        }
        let m = *b.get(i + 1)?;
        match m {
            0xD9 => return Some(i + 2),
            0x01 | 0xD0..=0xD7 => i += 2,
            _ => {
                let len = u16::from_be_bytes([*b.get(i + 2)?, *b.get(i + 3)?]) as usize;
                i += 2 + len;
                if m == 0xDA {
                    // Entropy-coded data runs to the next marker that is not a stuffed 0xFF00 or
                    // a restart.
                    loop {
                        if *b.get(i)? == 0xFF {
                            let n = *b.get(i + 1)?;
                            if n != 0x00 && !(0xD0..=0xD7).contains(&n) {
                                break;
                            }
                            i += 2;
                        } else {
                            i += 1;
                        }
                    }
                }
            }
        }
    }
}

/// Whether `orientation` describes a quarter turn, so the display size is the stored
/// size transposed.
pub fn orientation_swaps(orientation: u16) -> bool {
    (5..=8).contains(&orientation)
}

/// The orientation that still needs applying to pixels of size `decoded`, given that EXIF
/// recorded the image as `exif` and claimed `orientation`.
///
/// Two real cases make the stored tag untrustworthy on its own, and both arrived together
/// from one iPhone import:
///
///   * HEIC is decoded through FFmpeg, which honours the HEIF `irot` property, so the
///     pixels come back already upright. Applying the EXIF value on top turned every
///     photo another quarter turn clockwise.
///   * A PNG exported from an iPhone carries the original's EXIF block verbatim while the
///     pixels it stores are already upright. The tag says 6, the image needs nothing, and
///     rotating it broke a picture that was correct to begin with.
///
/// Both are the same shape of problem: the decoded pixels are already in display
/// orientation. That is detectable — they are the transpose of what EXIF recorded — so
/// trust the pixels rather than the tag.
///
/// When the decoder itself applies orientation (`decoder_orients`: FFmpeg's HEIC and video
/// paths), nothing is ever owed. The size check alone missed tags that do not swap axes:
/// an iPhone photo tagged 3 (upside down) came back upright from FFmpeg and was then turned
/// 180° again — 99 photos in one real library.
pub fn effective_orientation(orientation: u16, exif: Option<(u32, u32)>, decoded: (u32, u32), decoder_orients: bool) -> u16 {
    if decoder_orients {
        return 1;
    }
    if !orientation_swaps(orientation) {
        return orientation;
    }
    match exif {
        // Already transposed relative to what EXIF recorded: the decoder turned it for us.
        Some((ew, eh)) if (ew, eh) == (decoded.1, decoded.0) && ew != eh => 1,
        // No recorded size to compare against: fall back to the aspect. A tag that swaps
        // means the stored pixels should be the opposite orientation to the display, so
        // pixels already in portrait have nothing left to do.
        None if decoded.1 > decoded.0 => 1,
        _ => orientation,
    }
}

/// Whether [`decode_preview_hdr`] returns this kind already upright: HEIC and video go through
/// FFmpeg, which applies the container's rotation and mirroring (HEIF `irot`/`imir`, the
/// QuickTime display matrix) before handing back pixels.
pub fn decoder_orients(kind: MediaKind) -> bool {
    matches!(kind, MediaKind::Heic | MediaKind::Video)
}

/// Bake EXIF orientation (1–8) into the pixels so the frontend never rotates.
pub fn apply_orientation(img: DynamicImage, orientation: u16) -> DynamicImage {
    match orientation {
        2 => img.fliph(),
        3 => img.rotate180(),
        4 => img.flipv(),
        5 => img.rotate90().fliph(),
        6 => img.rotate90(),
        7 => img.rotate270().fliph(),
        8 => img.rotate270(),
        _ => img,
    }
}

#[cfg(test)]
mod orientation_tests {
    use super::*;

    /// 99 of Julian's iPhone photos, tagged 3 (upside down), were turned a second time:
    /// FFmpeg had already turned them, and the size check only runs for tags that swap axes.
    #[test]
    fn a_decoder_that_orients_owes_nothing_whatever_the_tag() {
        for tag in 1..=8 {
            assert_eq!(effective_orientation(tag, Some((4032, 3024)), (4032, 3024), true), 1, "tag {tag}");
            assert_eq!(effective_orientation(tag, None, (3024, 3024), true), 1, "tag {tag}");
        }
        // Decoders that leave the pixels alone still owe the tag.
        assert_eq!(effective_orientation(3, Some((4032, 3024)), (4032, 3024), false), 3);
    }

    #[test]
    fn ffmpeg_decodes_are_the_ones_that_orient() {
        assert!(decoder_orients(MediaKind::Heic));
        assert!(decoder_orients(MediaKind::Video));
        assert!(!decoder_orients(MediaKind::Image));
        assert!(!decoder_orients(MediaKind::Raw));
    }

    #[test]
    fn keeps_the_tag_when_the_decoder_left_the_pixels_alone() {
        // A JPEG or a RAW embedded preview: stored landscape, EXIF says turn it.
        assert_eq!(effective_orientation(6, Some((4032, 3024)), (4032, 3024), false), 6);
        assert_eq!(effective_orientation(8, Some((4032, 3024)), (4032, 3024), false), 8);
    }

    #[test]
    fn drops_the_tag_when_the_pixels_are_the_transpose_of_what_exif_recorded() {
        // EXIF recorded 4032x3024; the decoder handed back 3024x4032, already upright.
        assert_eq!(effective_orientation(6, Some((4032, 3024)), (3024, 4032), false), 1);
    }

    #[test]
    fn drops_a_stale_tag_on_an_iphone_png_export() {
        // The eXIf block is copied from the original, but the pixels are already upright.
        assert_eq!(effective_orientation(6, Some((4032, 3024)), (3024, 4032), false), 1);
        // Without a recorded size, the aspect is the only evidence there is.
        assert_eq!(effective_orientation(6, None, (3024, 4032), false), 1);
        assert_eq!(effective_orientation(6, None, (4032, 3024), false), 6);
    }

    #[test]
    fn leaves_orientations_that_do_not_turn_the_image_alone() {
        for o in [1, 2, 3, 4] {
            assert_eq!(effective_orientation(o, Some((4032, 3024)), (3024, 4032), false), o);
        }
    }

    #[test]
    fn a_square_image_cannot_be_judged_by_its_shape_so_the_tag_stands() {
        assert_eq!(effective_orientation(6, Some((3024, 3024)), (3024, 3024), false), 6);
    }
}

#[cfg(test)]
mod raw_preview_tests {
    use super::*;

    fn jpeg(w: u32, h: u32) -> Vec<u8> {
        let img = image::RgbImage::from_fn(w, h, |x, y| image::Rgb([(x * 7) as u8, (y * 5) as u8, 90]));
        let mut out = Vec::new();
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 80).encode_image(&img).unwrap();
        out
    }

    /// A file shaped like a RAW rawler doesn't know: noise, a small JPEG, noise, a larger one.
    fn unknown_raw(dir: &Path, sizes: &[(u32, u32)], truncate_last: bool) -> std::path::PathBuf {
        let mut bytes = vec![0x4d, 0x4d, 0, 42];
        let mut seed = 7u32;
        for (i, &(w, h)) in sizes.iter().enumerate() {
            bytes.extend((0..3000).map(|_| {
                seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12345);
                (seed >> 16) as u8
            }));
            let mut j = jpeg(w, h);
            if truncate_last && i == sizes.len() - 1 {
                j.truncate(j.len() / 3);
            }
            bytes.extend(j);
        }
        let p = dir.join("NEW_BODY.ARW");
        std::fs::write(&p, bytes).unwrap();
        p
    }

    #[test]
    fn the_largest_embedded_jpeg_is_found() {
        let dir = tempfile::tempdir().unwrap();
        let p = unknown_raw(dir.path(), &[(160, 120), (1600, 1200), (640, 480)], false);
        let img = embedded_jpeg(&std::fs::read(&p).unwrap()).expect("a preview");
        assert_eq!((img.width(), img.height()), (1600, 1200));
    }

    #[test]
    fn a_broken_jpeg_is_passed_over() {
        let dir = tempfile::tempdir().unwrap();
        let p = unknown_raw(dir.path(), &[(320, 240), (2000, 1500)], true);
        let img = embedded_jpeg(&std::fs::read(&p).unwrap()).expect("the whole one");
        assert_eq!((img.width(), img.height()), (320, 240));
        assert!(embedded_jpeg(&[0u8; 5000]).is_none());
    }

    /// The Sony A7 V, the FX2 and new Panasonics: rawler says "Unknown camera", and before
    /// Round 17 they got no thumbnail at all.
    #[test]
    fn a_raw_rawler_does_not_know_still_has_a_preview() {
        let dir = tempfile::tempdir().unwrap();
        let p = unknown_raw(dir.path(), &[(160, 120), (1920, 1280)], false);
        let img = decode_preview(&p, MediaKind::Raw).expect("its own JPEG");
        assert_eq!((img.width(), img.height()), (1920, 1280));
    }

    #[test]
    fn a_raw_with_nothing_readable_is_still_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("x.ORF");
        std::fs::write(&p, vec![3u8; 4096]).unwrap();
        assert!(decode_preview(&p, MediaKind::Raw).is_err());
        assert!(matches!(decode_preview(&dir.path().join("gone.ORF"), MediaKind::Raw), Err(MediaError::Io(_))));
    }
}
