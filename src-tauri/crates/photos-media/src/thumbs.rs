use std::io::Write;
use std::path::Path;

use fast_image_resize::{images::Image, IntoImageView, ResizeOptions, Resizer};
use image::{DynamicImage, ImageEncoder};

use crate::MediaError;

/// Fit the long edge to `long_edge`. Never upscales. Output is RGB8.
pub fn resize_long_edge(img: &DynamicImage, long_edge: u32) -> DynamicImage {
    let (w, h) = (img.width(), img.height());
    let scale = long_edge as f64 / w.max(h) as f64;
    let src = DynamicImage::ImageRgb8(img.to_rgb8());
    if scale >= 1.0 {
        return src;
    }
    let nw = ((w as f64 * scale).round() as u32).max(1);
    let nh = ((h as f64 * scale).round() as u32).max(1);
    let mut dst = Image::new(nw, nh, src.pixel_type().expect("rgb8 pixel type"));
    Resizer::new().resize(&src, &mut dst, &ResizeOptions::new()).expect("resize rgb8");
    DynamicImage::ImageRgb8(image::RgbImage::from_raw(nw, nh, dst.into_vec()).expect("resized buffer"))
}

/// JPEG at `quality`, written atomically (tmp + rename). Returns bytes written.
pub fn encode_jpeg(img: &DynamicImage, quality: u8, out: &Path) -> Result<u64, MediaError> {
    if let Some(p) = out.parent() {
        std::fs::create_dir_all(p)?;
    }
    let bytes = encode_jpeg_bytes(img, quality)?;
    let tmp = out.with_extension("jpg.tmp");
    {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(&bytes)?;
        f.flush()?;
    }
    std::fs::rename(&tmp, out)?;
    Ok(bytes.len() as u64)
}

/// A lossless PNG at `out` (export), written through a temporary file like the JPEGs.
pub fn encode_png(img: &DynamicImage, out: &Path) -> Result<u64, MediaError> {
    if let Some(p) = out.parent() {
        std::fs::create_dir_all(p)?;
    }
    let tmp = out.with_extension("png.tmp");
    DynamicImage::ImageRgb8(img.to_rgb8()).save_with_format(&tmp, image::ImageFormat::Png)?;
    std::fs::rename(&tmp, out)?;
    Ok(std::fs::metadata(out)?.len())
}

/// The JPEG in memory: for bytes served straight from a cache (the editor's preview).
pub fn encode_jpeg_bytes(img: &DynamicImage, quality: u8) -> Result<Vec<u8>, MediaError> {
    let rgb = img.to_rgb8();
    let mut out = Vec::with_capacity((rgb.width() * rgb.height() / 4) as usize);
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, quality).write_image(
        rgb.as_raw(),
        rgb.width(),
        rgb.height(),
        image::ExtendedColorType::Rgb8,
    )?;
    Ok(out)
}

/// ~25 bytes; decodes client-side into a blurred placeholder.
pub fn thumbhash_for(img: &DynamicImage) -> Vec<u8> {
    let small = resize_long_edge(img, 100).to_rgba8();
    thumbhash::rgba_to_thumb_hash(small.width() as usize, small.height() as usize, small.as_raw())
}

/// Mean colour as `#rrggbb`, for tile backgrounds before the placeholder decodes.
pub fn dominant_color(img: &DynamicImage) -> String {
    let small = resize_long_edge(img, 16).to_rgb8();
    let n = (small.width() as u64 * small.height() as u64).max(1);
    let (mut r, mut g, mut b) = (0u64, 0u64, 0u64);
    for p in small.pixels() {
        r += p[0] as u64;
        g += p[1] as u64;
        b += p[2] as u64;
    }
    format!("#{:02x}{:02x}{:02x}", r / n, g / n, b / n)
}

/// Variance of the 4-neighbour Laplacian of the luma: high for crisp detail, near zero for
/// blur or a flat field. Callers pass the 256 px image, so values compare across photos.
pub fn sharpness(img: &DynamicImage) -> f32 {
    let luma = img.to_luma8();
    let (w, h) = luma.dimensions();
    if w < 3 || h < 3 {
        return 0.0;
    }
    let p = |x: u32, y: u32| luma.get_pixel(x, y)[0] as f64;
    let (mut sum, mut sum_sq, mut n) = (0f64, 0f64, 0f64);
    for y in 1..h - 1 {
        for x in 1..w - 1 {
            let l = 4.0 * p(x, y) - p(x - 1, y) - p(x + 1, y) - p(x, y - 1) - p(x, y + 1);
            sum += l;
            sum_sq += l * l;
            n += 1.0;
        }
    }
    let mean = sum / n;
    (sum_sq / n - mean * mean).max(0.0) as f32
}

/// dHash: the image as 9×8 grayscale, one bit per horizontal neighbour pair (left brighter).
/// Near-duplicates differ in a few bits: `(a ^ b).count_ones()`.
pub fn dhash(img: &DynamicImage) -> u64 {
    let small = img.resize_exact(9, 8, image::imageops::FilterType::Triangle).to_luma8();
    let mut bits = 0u64;
    for y in 0..8 {
        for x in 0..8 {
            if small.get_pixel(x, y)[0] > small.get_pixel(x + 1, y)[0] {
                bits |= 1 << (y * 8 + x);
            }
        }
    }
    bits
}

#[cfg(test)]
mod quality_tests {
    use super::*;

    #[test]
    fn sharp_edges_score_higher_than_blur() {
        let sharp = DynamicImage::ImageRgb8(image::RgbImage::from_fn(64, 64, |x, _| if (x / 4) % 2 == 0 { image::Rgb([0, 0, 0]) } else { image::Rgb([255, 255, 255]) }));
        let flat = DynamicImage::ImageRgb8(image::RgbImage::from_pixel(64, 64, image::Rgb([128, 128, 128])));
        let blurred = sharp.blur(3.0);
        assert!(sharpness(&sharp) > sharpness(&blurred) * 4.0, "{} vs {}", sharpness(&sharp), sharpness(&blurred));
        assert_eq!(sharpness(&flat), 0.0);
    }

    #[test]
    fn near_duplicates_hash_close_and_different_photos_far() {
        let a = DynamicImage::ImageRgb8(image::RgbImage::from_fn(200, 150, |x, y| image::Rgb([(x % 256) as u8, (y * 2 % 256) as u8, 90])));
        let brighter = DynamicImage::ImageRgb8(image::RgbImage::from_fn(200, 150, |x, y| {
            image::Rgb([((x % 256) as u8).saturating_add(12), ((y * 2 % 256) as u8).saturating_add(12), 100])
        }));
        let other = DynamicImage::ImageRgb8(image::RgbImage::from_fn(200, 150, |x, y| image::Rgb([(255 - x % 256) as u8, (y % 256) as u8, 30])));
        assert!((dhash(&a) ^ dhash(&brighter)).count_ones() <= 6);
        assert!((dhash(&a) ^ dhash(&other)).count_ones() > 20, "{}", (dhash(&a) ^ dhash(&other)).count_ones());
    }
}
