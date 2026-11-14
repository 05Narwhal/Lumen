//! Export pixels (Phase 8 spec §5): JPEG, PNG, WebP, AVIF and HEIC, each with its colour
//! declared — an ICC profile where the format carries one, `nclx` for AVIF (and both for
//! HEIC) — so the file looks the same in every colour-managed viewer. Metadata is added
//! afterwards by ExifTool; this module writes pixels only.
//!
//! JPEG, PNG and WebP are encoded in Rust. AVIF and HEIC need FFmpeg (`libaom-av1`,
//! `libx265`); without it they are refused with [`MediaError::Unsupported`].

use std::io::{Read, Write};
use std::path::Path;
use std::process::Stdio;

use image::{DynamicImage, ImageEncoder, RgbImage};

use crate::ffmpeg::cmd;
use crate::heif::{self, NCLX_SRGB, TILE};
use crate::{icc, Ffmpeg, MediaError};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Jpeg,
    Png,
    Webp,
    Avif,
    Heic,
    /// Uncompressed 8-bit RGB: what an external editor is given (Round 22 §8).
    Tiff,
}

impl Format {
    pub fn needs_ffmpeg(self) -> bool {
        matches!(self, Format::Avif | Format::Heic)
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EncodeOptions {
    pub format: Format,
    /// 1–100 for the lossy formats; ignored by PNG and lossless WebP.
    pub quality: u8,
    /// WebP only.
    pub lossless: bool,
}

/// `libx265`'s CRF for a quality: 90 → 15, 80 → 19, 60 → 27.
pub fn hevc_crf(quality: u8) -> u8 {
    (51.0 - 0.4 * quality.clamp(1, 100) as f64).round() as u8
}

/// `libaom-av1`'s CRF (0–63) for a quality: 90 → 13, 80 → 19, 60 → 30.
pub fn av1_crf(quality: u8) -> u8 {
    (63.0 - 0.55 * quality.clamp(1, 100) as f64).round() as u8
}

/// Write `img` to `out` as `opts` says. `out` is the caller's temporary file.
pub fn encode_file(img: &DynamicImage, opts: &EncodeOptions, ffmpeg: Option<&Ffmpeg>, out: &Path) -> Result<(), MediaError> {
    let rgb = img.to_rgb8();
    let need = || ffmpeg.ok_or_else(|| MediaError::Unsupported(format!("{:?} export needs FFmpeg", opts.format)));
    let bytes = match opts.format {
        Format::Jpeg => jpeg(&rgb, opts.quality)?,
        Format::Png => png(&rgb)?,
        Format::Webp => webp(&rgb, opts.quality, opts.lossless)?,
        Format::Avif => return avif(&rgb, opts.quality, need()?, out),
        Format::Heic => heic(&rgb, opts.quality, need()?)?,
        Format::Tiff => tiff(&rgb)?,
    };
    std::fs::write(out, bytes)?;
    Ok(())
}

/// 8 bits per channel: the export render hands the encoder an `RgbImage`
/// (`develop::render_base`), so there are no more bits to write.
fn tiff(rgb: &RgbImage) -> Result<Vec<u8>, MediaError> {
    let mut out = std::io::Cursor::new(Vec::new());
    image::codecs::tiff::TiffEncoder::new(&mut out)
        .write_image(rgb.as_raw(), rgb.width(), rgb.height(), image::ExtendedColorType::Rgb8)
        .map_err(|e| MediaError::Decode(format!("tiff: {e}")))?;
    Ok(out.into_inner())
}

fn jpeg(rgb: &RgbImage, quality: u8) -> Result<Vec<u8>, MediaError> {
    let mut out = Vec::new();
    let mut enc = jpeg_encoder::Encoder::new(&mut out, quality.clamp(1, 100));
    // Above 90 the chroma is kept at full resolution: that is what "high quality" is for.
    enc.set_sampling_factor(if quality > 90 { jpeg_encoder::SamplingFactor::R_4_4_4 } else { jpeg_encoder::SamplingFactor::R_4_2_0 });
    enc.add_icc_profile(icc::srgb()).map_err(|e| MediaError::Decode(format!("jpeg icc: {e}")))?;
    let (w, h) = (u16::try_from(rgb.width()), u16::try_from(rgb.height()));
    let (Ok(w), Ok(h)) = (w, h) else {
        return Err(MediaError::Unsupported("JPEG is limited to 65535 pixels a side".into()));
    };
    enc.encode(rgb.as_raw(), w, h, jpeg_encoder::ColorType::Rgb).map_err(|e| MediaError::Decode(format!("jpeg: {e}")))?;
    Ok(out)
}

fn png(rgb: &RgbImage) -> Result<Vec<u8>, MediaError> {
    let mut out = Vec::new();
    let mut enc = image::codecs::png::PngEncoder::new(&mut out);
    enc.set_icc_profile(icc::srgb().to_vec()).map_err(|e| MediaError::Decode(format!("png icc: {e}")))?;
    enc.write_image(rgb.as_raw(), rgb.width(), rgb.height(), image::ExtendedColorType::Rgb8)?;
    Ok(out)
}

fn webp(rgb: &RgbImage, quality: u8, lossless: bool) -> Result<Vec<u8>, MediaError> {
    let enc = webp::Encoder::from_rgb(rgb.as_raw(), rgb.width(), rgb.height());
    let simple = if lossless { enc.encode_lossless() } else { enc.encode(quality.clamp(1, 100) as f32) };
    with_webp_icc(&simple, rgb.width(), rgb.height(), icc::srgb())
}

fn chunk(out: &mut Vec<u8>, fourcc: &[u8; 4], body: &[u8]) {
    out.extend_from_slice(fourcc);
    out.extend_from_slice(&(body.len() as u32).to_le_bytes());
    out.extend_from_slice(body);
    if !body.len().is_multiple_of(2) {
        out.push(0);
    }
}

/// Re-mux a simple WebP (`RIFF…WEBPVP8 ` or `VP8L`) as an extended one, which is the only
/// kind that can carry an ICC profile: `VP8X` with the ICC flag, `ICCP`, then the image.
pub fn with_webp_icc(simple: &[u8], width: u32, height: u32, icc: &[u8]) -> Result<Vec<u8>, MediaError> {
    if simple.len() < 20 || &simple[0..4] != b"RIFF" || &simple[8..12] != b"WEBP" {
        return Err(MediaError::Decode("webp: not a RIFF WebP".into()));
    }
    let image_chunks = &simple[12..];
    if !matches!(&image_chunks[0..4], b"VP8 " | b"VP8L") {
        return Err(MediaError::Decode("webp: expected a simple WebP".into()));
    }
    let mut vp8x = vec![0x20u8, 0, 0, 0]; // flags: ICC profile
    vp8x.extend_from_slice(&(width - 1).to_le_bytes()[..3]);
    vp8x.extend_from_slice(&(height - 1).to_le_bytes()[..3]);
    let mut body = b"WEBP".to_vec();
    chunk(&mut body, b"VP8X", &vp8x);
    chunk(&mut body, b"ICCP", icc);
    body.extend_from_slice(image_chunks);
    let mut out = b"RIFF".to_vec();
    out.extend_from_slice(&(body.len() as u32).to_le_bytes());
    out.extend_from_slice(&body);
    Ok(out)
}

/// RGB → full-range BT.709 YUV, tagged sRGB, so what the file says matches what was done.
fn yuv_filter(pix_fmt: &str) -> String {
    format!("scale=out_color_matrix=bt709:out_range=pc,format={pix_fmt},setparams=color_primaries=bt709:color_trc=iec61966-2-1:colorspace=bt709:range=pc")
}

/// Run FFmpeg with `input` on stdin, returning stdout. The writer runs on its own thread,
/// so a full stdout pipe can never deadlock a full stdin pipe.
fn pipe_through(mut c: std::process::Command, input: Vec<u8>) -> Result<Vec<u8>, MediaError> {
    c.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = c.spawn()?;
    photos_core::child_jobs::adopt(&child);
    let mut stdin = child.stdin.take().ok_or_else(|| MediaError::Decode("ffmpeg: no stdin".into()))?;
    let writer = std::thread::spawn(move || stdin.write_all(&input));
    let mut stdout = Vec::new();
    child.stdout.take().ok_or_else(|| MediaError::Decode("ffmpeg: no stdout".into()))?.read_to_end(&mut stdout)?;
    let mut stderr = String::new();
    if let Some(mut e) = child.stderr.take() {
        let _ = e.read_to_string(&mut stderr);
    }
    let status = child.wait()?;
    let _ = writer.join();
    if !status.success() {
        return Err(MediaError::Decode(format!("ffmpeg: {}", stderr.trim())));
    }
    Ok(stdout)
}

fn avif(rgb: &RgbImage, quality: u8, ff: &Ffmpeg, out: &Path) -> Result<(), MediaError> {
    // The avif muxer needs a seekable output, so FFmpeg writes the file itself.
    let mut c = cmd(&ff.ffmpeg);
    c.args(["-v", "error", "-y", "-f", "rawvideo", "-pix_fmt", "rgb24", "-s", &format!("{}x{}", rgb.width(), rgb.height()), "-i", "-"])
        .args(["-vf", &yuv_filter("yuv444p")])
        .args(["-frames:v", "1", "-c:v", "libaom-av1", "-still-picture", "1", "-crf", &av1_crf(quality).to_string()])
        .args(["-cpu-used", "6", "-row-mt", "1", "-f", "avif"])
        .arg(out);
    pipe_through(c, rgb.as_raw().clone())?;
    Ok(())
}

/// The image cut into 512 × 512 tiles in raster order, edges padded by repeating the last
/// row and column (which costs far fewer bits than black).
pub fn tiles(rgb: &RgbImage) -> Vec<u8> {
    let (w, h) = rgb.dimensions();
    let (cols, rows) = (w.div_ceil(TILE), h.div_ceil(TILE));
    let raw = rgb.as_raw();
    let mut out = Vec::with_capacity((cols * rows * TILE * TILE * 3) as usize);
    for ty in 0..rows {
        for tx in 0..cols {
            for y in 0..TILE {
                let sy = (ty * TILE + y).min(h - 1) as usize;
                let row = &raw[sy * w as usize * 3..(sy + 1) * w as usize * 3];
                let x0 = tx * TILE;
                let inside = (w - x0).min(TILE) as usize;
                out.extend_from_slice(&row[x0 as usize * 3..(x0 as usize + inside) * 3]);
                let last = &row[(x0 as usize + inside - 1) * 3..(x0 as usize + inside) * 3];
                for _ in inside..TILE as usize {
                    out.extend_from_slice(last);
                }
            }
        }
    }
    out
}

fn heic(rgb: &RgbImage, quality: u8, ff: &Ffmpeg) -> Result<Vec<u8>, MediaError> {
    let mut c = cmd(&ff.ffmpeg);
    c.args(["-v", "error", "-f", "rawvideo", "-pix_fmt", "rgb24", "-s", &format!("{TILE}x{TILE}"), "-r", "1", "-i", "-"])
        .args(["-vf", &yuv_filter("yuv420p")])
        .args(["-c:v", "libx265", "-preset", "medium", "-crf", &hevc_crf(quality).to_string()])
        // Every tile is its own intra picture, each with its parameter sets.
        .args(["-x265-params", "keyint=1:min-keyint=1:bframes=0:repeat-headers=1:log-level=error"])
        .args(["-f", "hevc", "-"]);
    let stream = pipe_through(c, tiles(rgb))?;
    heif::write_grid(&stream, rgb.width(), rgb.height(), NCLX_SRGB, icc::srgb())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A picture with detail and saturated colour, so a wrong matrix or a shifted tile shows.
    /// Its checkerboard is hard on 4:2:0 chroma, which is why the floors below are modest.
    pub(crate) fn sample(w: u32, h: u32) -> RgbImage {
        RgbImage::from_fn(w, h, |x, y| {
            let r = (x * 255 / w.max(1)) as u8;
            let g = (y * 255 / h.max(1)) as u8;
            let b = if (x / 16 + y / 16) % 2 == 0 { 220 } else { 40 };
            image::Rgb([r, g, b])
        })
    }

    pub(crate) fn psnr(a: &RgbImage, b: &RgbImage) -> f64 {
        assert_eq!(a.dimensions(), b.dimensions());
        let mse: f64 = a.as_raw().iter().zip(b.as_raw()).map(|(x, y)| (*x as f64 - *y as f64).powi(2)).sum::<f64>() / a.as_raw().len() as f64;
        if mse == 0.0 {
            f64::INFINITY
        } else {
            10.0 * (255.0f64.powi(2) / mse).log10()
        }
    }

    #[test]
    fn quality_maps_onto_each_encoders_scale() {
        assert_eq!((hevc_crf(90), hevc_crf(80), hevc_crf(60)), (15, 19, 27));
        assert_eq!((av1_crf(90), av1_crf(80), av1_crf(60)), (13, 19, 30));
        assert!(hevc_crf(100) < hevc_crf(1) && av1_crf(100) < av1_crf(1));
    }

    #[test]
    fn tiles_are_cut_in_raster_order_with_edges_repeated() {
        let img = sample(600, 520);
        let t = tiles(&img);
        assert_eq!(t.len(), (2 * 2 * 512 * 512 * 3) as usize);
        let px = |tile: usize, x: u32, y: u32| {
            let i = tile * 512 * 512 * 3 + ((y * 512 + x) * 3) as usize;
            [t[i], t[i + 1], t[i + 2]]
        };
        assert_eq!(px(0, 10, 20), img.get_pixel(10, 20).0);
        assert_eq!(px(1, 5, 5), img.get_pixel(517, 5).0, "second tile starts at x = 512");
        assert_eq!(px(1, 300, 5), img.get_pixel(599, 5).0, "past the right edge repeats the last column");
        assert_eq!(px(3, 0, 100), img.get_pixel(512, 519).0, "past the bottom repeats the last row");
    }

    #[test]
    fn jpeg_png_and_webp_decode_back_and_carry_the_profile() {
        let img = DynamicImage::ImageRgb8(sample(320, 200));
        let dir = tempfile::tempdir().unwrap();
        for (format, quality, lossless, floor) in [(Format::Jpeg, 92, false, 34.0), (Format::Jpeg, 80, false, 26.0), (Format::Png, 0, false, f64::INFINITY), (Format::Webp, 90, false, 26.0), (Format::Webp, 0, true, f64::INFINITY)] {
            let out = dir.path().join("out");
            encode_file(&img, &EncodeOptions { format, quality, lossless }, None, &out).unwrap();
            let bytes = std::fs::read(&out).unwrap();
            let back = image::load_from_memory(&bytes).unwrap().to_rgb8();
            let p = psnr(img.as_rgb8().unwrap(), &back);
            assert!(p >= floor, "{format:?} q{quality}: PSNR {p:.1}");
            let has_icc = bytes.windows(icc::SRGB_DESCRIPTION.len()).any(|w| w == icc::SRGB_DESCRIPTION.as_bytes());
            // PNG's iCCP is deflated, so its text is not visible in the file.
            if format != Format::Png {
                assert!(has_icc, "{format:?} carries the profile");
            }
        }
    }

    #[test]
    fn avif_and_heic_are_refused_without_ffmpeg() {
        let img = DynamicImage::ImageRgb8(sample(16, 16));
        let dir = tempfile::tempdir().unwrap();
        for format in [Format::Avif, Format::Heic] {
            let e = encode_file(&img, &EncodeOptions { format, quality: 80, lossless: false }, None, &dir.path().join("x")).unwrap_err();
            assert!(matches!(e, MediaError::Unsupported(_)), "{e}");
        }
    }

    /// Decode through FFmpeg to RGB at the file's own size.
    fn ffmpeg_decode(ff: &Ffmpeg, path: &Path, w: u32, h: u32) -> RgbImage {
        let out = cmd(&ff.ffmpeg).args(["-v", "error", "-i"]).arg(path).args(["-frames:v", "1", "-f", "rawvideo", "-pix_fmt", "rgb24", "-"]).output().unwrap();
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
        RgbImage::from_raw(w, h, out.stdout).unwrap_or_else(|| panic!("FFmpeg decoded a different size than {w}×{h}"))
    }

    #[test]
    #[ignore = "needs FFmpeg"]
    fn avif_and_heic_decode_back_at_their_exact_size() {
        let ff = Ffmpeg::locate().expect("FFmpeg");
        let dir = tempfile::tempdir().unwrap();
        // 1100 × 700: three tiles across, two down, neither a multiple of 512.
        let src = sample(1100, 700);
        let img = DynamicImage::ImageRgb8(src.clone());
        for format in [Format::Avif, Format::Heic] {
            let out = dir.path().join(format!("out.{format:?}"));
            let t = std::time::Instant::now();
            encode_file(&img, &EncodeOptions { format, quality: 90, lossless: false }, Some(&ff), &out).unwrap();
            let back = ffmpeg_decode(&ff, &out, 1100, 700);
            let p = psnr(&src, &back);
            eprintln!("{format:?}: {} bytes, PSNR {p:.1} dB, {:?}", std::fs::metadata(&out).unwrap().len(), t.elapsed());
            assert!(p > 30.0, "{format:?}: PSNR {p:.1}");
        }
    }

    #[test]
    #[ignore = "needs FFmpeg and ExifTool"]
    fn exiftool_writes_into_our_heic_and_reads_its_profile() {
        let ff = Ffmpeg::locate().expect("FFmpeg");
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("out.heic");
        encode_file(&DynamicImage::ImageRgb8(sample(700, 600)), &EncodeOptions { format: Format::Heic, quality: 85, lossless: false }, Some(&ff), &out).unwrap();
        let mut s = crate::ExifTool::locate().expect("ExifTool").session().unwrap();
        let o = s
            .run(["-overwrite_original", "-EXIF:Make=Nikon", "-EXIF:DateTimeOriginal=2024:05:01 10:00:00", "-XMP-dc:Subject=beach"].map(std::ffi::OsString::from).into_iter().chain([out.clone().into_os_string()]))
            .unwrap();
        assert!(o.errors().is_empty(), "{o:?}");
        let v = s.read_json(&out, &["Make", "DateTimeOriginal", "Subject", "ProfileDescription", "ImageWidth", "ImageHeight"]).unwrap();
        assert_eq!(v["IFD0:Make"], "Nikon", "{v}");
        assert_eq!(v["XMP-dc:Subject"], "beach", "{v}");
        assert_eq!(v["ICC_Profile:ProfileDescription"], icc::SRGB_DESCRIPTION, "{v}");
        // And it still decodes after ExifTool rewrote it.
        let back = ffmpeg_decode(&ff, &out, 700, 600);
        assert!(psnr(&sample(700, 600), &back) > 30.0);
    }

}

#[cfg(test)]
mod tiff_tests {
    use super::*;

    /// TIFF copies are 8-bit: the export pipeline is 8-bit at `develop::render_base`.
    #[test]
    fn a_tiff_export_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("a.tif");
        let img = DynamicImage::ImageRgb8(RgbImage::from_fn(37, 21, |x, y| image::Rgb([x as u8 * 6, y as u8 * 12, 200])));
        encode_file(&img, &EncodeOptions { format: Format::Tiff, quality: 100, lossless: true }, None, &out).unwrap();
        let back = image::open(&out).unwrap();
        assert_eq!((back.width(), back.height()), (37, 21));
        assert_eq!(back.to_rgb8(), img.to_rgb8(), "uncompressed, so every pixel survives");
    }
}
