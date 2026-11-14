//! A clip's frame, composed once: the export renders this into a single still and hands
//! FFmpeg one input per clip, so a blurred backdrop is never recomputed thirty times a
//! second. The geometry comes from `photos_core::slideshow::frame_layout`, which the
//! preview canvas reads too — one set of rectangles for the video and the editor.

use image::{DynamicImage, GenericImage, GenericImageView, Rgba};
use photos_core::slideshow::Placement;

/// A backdrop is blurred at a fraction of its size and scaled back up: a wide blur for a
/// fraction of the cost, and indistinguishable once it is behind the photograph.
const BLUR_DOWNSCALE: u32 = 12;
const BLUR_SIGMA: f32 = 3.0;
/// The backdrop is darkened so the sharp copy is what the eye lands on.
const BACKDROP_DIM: f32 = 0.82;

/// Paints `placements` in order into a `width` × `height` frame. A rectangle may reach
/// outside the frame — that is what covering means — and is cropped by its edge.
pub fn compose(placements: &[Placement], photos: &[DynamicImage], width: u32, height: u32) -> DynamicImage {
    let mut canvas = DynamicImage::new_rgb8(width.max(1), height.max(1));
    for p in placements {
        let Some(photo) = photos.get(p.photo) else { continue };
        if photo.width() == 0 || photo.height() == 0 {
            continue;
        }
        let w = (p.rect.w * width as f32).round().max(1.0) as u32;
        let h = (p.rect.h * height as f32).round().max(1.0) as u32;
        let x = (p.rect.x * width as f32).round() as i64;
        let y = (p.rect.y * height as f32).round() as i64;
        let piece = if p.blurred {
            let small = photo.resize_exact(
                (w / BLUR_DOWNSCALE).max(1),
                (h / BLUR_DOWNSCALE).max(1),
                image::imageops::FilterType::Triangle,
            );
            let blurred = small.blur(BLUR_SIGMA);
            dim(&blurred.resize_exact(w, h, image::imageops::FilterType::Triangle), BACKDROP_DIM)
        } else {
            photo.resize_exact(w, h, image::imageops::FilterType::Lanczos3)
        };
        overlay_clipped(&mut canvas, &piece, x, y);
    }
    canvas
}

fn dim(img: &DynamicImage, factor: f32) -> DynamicImage {
    let mut out = img.to_rgba8();
    for pixel in out.pixels_mut() {
        let Rgba([r, g, b, a]) = *pixel;
        *pixel = Rgba([(r as f32 * factor) as u8, (g as f32 * factor) as u8, (b as f32 * factor) as u8, a]);
    }
    DynamicImage::ImageRgba8(out)
}

/// `copy_from` with the parts outside the canvas dropped, because a covering rectangle is
/// deliberately larger than the frame.
fn overlay_clipped(canvas: &mut DynamicImage, piece: &DynamicImage, x: i64, y: i64) {
    let (cw, ch) = (canvas.width() as i64, canvas.height() as i64);
    let (pw, ph) = (piece.width() as i64, piece.height() as i64);
    let (sx, sy) = ((-x).max(0), (-y).max(0));
    let w = (pw - sx).min(cw - x.max(0));
    let h = (ph - sy).min(ch - y.max(0));
    if w <= 0 || h <= 0 {
        return;
    }
    let cropped = piece.view(sx as u32, sy as u32, w as u32, h as u32).to_image();
    let _ = canvas.copy_from(&DynamicImage::ImageRgba8(cropped), x.max(0) as u32, y.max(0) as u32);
}

#[cfg(test)]
mod tests {
    use super::*;
    use photos_core::slideshow::{frame_layout, FrameKind};

    fn solid(w: u32, h: u32, colour: [u8; 3]) -> DynamicImage {
        DynamicImage::ImageRgb8(image::RgbImage::from_pixel(w, h, image::Rgb(colour)))
    }

    #[test]
    fn a_filled_frame_is_the_photo_edge_to_edge() {
        let out = compose(&frame_layout(FrameKind::Fill, &[1.5], 16.0 / 9.0), &[solid(300, 200, [200, 30, 30])], 320, 180);
        assert_eq!((out.width(), out.height()), (320, 180));
        for (x, y) in [(2, 2), (160, 90), (317, 177)] {
            assert_eq!(out.to_rgb8().get_pixel(x, y)[0], 200, "the photo reaches {x},{y}");
        }
    }

    #[test]
    fn a_blurred_frame_shows_the_whole_photo_with_colour_behind_it() {
        let photo = DynamicImage::ImageRgb8(image::RgbImage::from_fn(200, 400, |_, y| {
            if y < 200 {
                image::Rgb([0, 0, 255])
            } else {
                image::Rgb([255, 255, 0])
            }
        }));
        let out = compose(&frame_layout(FrameKind::Blur, &[0.5], 16.0 / 9.0), &[photo], 320, 180).to_rgb8();
        // The sharp copy is centred and whole: its top half blue, its bottom yellow.
        let upper = out.get_pixel(160, 40);
        assert!(upper[2] > 200 && upper[0] < 60, "sharp top is blue: {upper:?}");
        let lower = out.get_pixel(160, 140);
        assert!(lower[0] > 200 && lower[1] > 200, "sharp bottom is yellow: {lower:?}");
        // The edges are the blurred backdrop, so they carry colour rather than black.
        let edge = out.get_pixel(3, 90);
        assert!(edge[0] as u32 + edge[1] as u32 + edge[2] as u32 > 90, "backdrop is not black: {edge:?}");
    }

    #[test]
    fn a_grid_puts_each_photo_in_its_own_column() {
        let red = solid(100, 200, [255, 0, 0]);
        let green = solid(100, 200, [0, 255, 0]);
        let out = compose(&frame_layout(FrameKind::Grid, &[0.5, 0.5], 16.0 / 9.0), &[red, green], 320, 180).to_rgb8();
        assert!(out.get_pixel(80, 90)[0] > 200, "left column is the first photo");
        assert!(out.get_pixel(240, 90)[1] > 200, "right column is the second");
    }

    #[test]
    fn a_missing_photo_does_not_panic() {
        let out = compose(&frame_layout(FrameKind::Grid, &[0.5, 0.5], 16.0 / 9.0), &[solid(100, 200, [255, 0, 0])], 320, 180);
        assert_eq!((out.width(), out.height()), (320, 180));
    }
}
