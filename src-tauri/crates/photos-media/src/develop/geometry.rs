//! Geometry first (it only removes pixels, so every later stage runs on fewer, and works on
//! what will be seen): flips, quarter turns, straighten, crop — the order `Recipe::output_size`
//! and the quick editor's CSS assume.

use photos_core::edits::Recipe;
use rayon::prelude::*;

use super::Linear;

/// Whether `r` moves or removes any pixel.
pub fn is_identity(r: &Recipe) -> bool {
    !r.flip_h && !r.flip_v && r.rotate.is_multiple_of(360) && r.straighten == 0.0 && r.crop.is_none()
}

pub fn apply(img: &Linear, r: &Recipe) -> Linear {
    if is_identity(r) {
        return img.clone();
    }
    let mut out = img.clone();
    if r.flip_h {
        out = flip_h(&out);
    }
    if r.flip_v {
        out = flip_v(&out);
    }
    out = match r.rotate % 360 {
        90 => rotate90(&out),
        180 => rotate180(&out),
        270 => rotate270(&out),
        _ => out,
    };
    if r.straighten != 0.0 {
        out = straighten(&out, r.straighten);
    }
    if let Some(k) = r.crop {
        let (w, h) = (out.width, out.height);
        let x = (k.x * w as f32).round() as u32;
        let y = (k.y * h as f32).round() as u32;
        let cw = ((k.w * w as f32).round() as u32).clamp(1, w - x.min(w - 1));
        let ch = ((k.h * h as f32).round() as u32).clamp(1, h - y.min(h - 1));
        out = crop(&out, x, y, cw, ch);
    }
    out
}

/// Build a `w × h` image whose pixel (x, y) is `src(f(x, y))`.
fn remap(src: &Linear, w: u32, h: u32, f: impl Fn(u32, u32) -> (u32, u32) + Sync) -> Linear {
    let mut data = vec![[0f32; 3]; (w * h) as usize];
    data.par_chunks_mut(w as usize).enumerate().for_each(|(y, row)| {
        for (x, o) in row.iter_mut().enumerate() {
            let (sx, sy) = f(x as u32, y as u32);
            *o = src.get(sx, sy);
        }
    });
    Linear { width: w, height: h, data }
}

pub fn flip_h(s: &Linear) -> Linear {
    remap(s, s.width, s.height, |x, y| (s.width - 1 - x, y))
}

pub fn flip_v(s: &Linear) -> Linear {
    remap(s, s.width, s.height, |x, y| (x, s.height - 1 - y))
}

/// Clockwise: the top-left corner goes top-right.
pub fn rotate90(s: &Linear) -> Linear {
    remap(s, s.height, s.width, |x, y| (y, s.height - 1 - x))
}

pub fn rotate180(s: &Linear) -> Linear {
    remap(s, s.width, s.height, |x, y| (s.width - 1 - x, s.height - 1 - y))
}

pub fn rotate270(s: &Linear) -> Linear {
    remap(s, s.height, s.width, |x, y| (s.width - 1 - y, x))
}

pub fn crop(s: &Linear, x0: u32, y0: u32, w: u32, h: u32) -> Linear {
    remap(s, w, h, |x, y| (x0 + x, y0 + y))
}

/// Where output pixel (x, y) of a `w × h` frame straightened by `deg` samples the source:
/// rotated about the centre and zoomed just enough that the frame stays inside the photo,
/// z = cos|θ| + sin|θ| · max(w/h, h/w).
pub fn straighten_source(x: f32, y: f32, w: u32, h: u32, deg: f32) -> (f32, f32) {
    let (s, c) = deg.to_radians().sin_cos();
    let ratio = (w as f32 / h as f32).max(h as f32 / w as f32);
    let z = c.abs() + s.abs() * ratio;
    let (cx, cy) = (w as f32 / 2.0, h as f32 / 2.0);
    let (dx, dy) = ((x + 0.5 - cx) / z, (y + 0.5 - cy) / z);
    (c * dx + s * dy + cx - 0.5, -s * dx + c * dy + cy - 0.5)
}

/// Bilinear sampling through [`straighten_source`].
pub fn straighten(src: &Linear, deg: f32) -> Linear {
    let (w, h) = (src.width, src.height);
    let mut data = vec![[0f32; 3]; (w * h) as usize];
    data.par_chunks_mut(w as usize).enumerate().for_each(|(y, row)| {
        for (x, o) in row.iter_mut().enumerate() {
            let (sx, sy) = straighten_source(x as f32, y as f32, w, h, deg);
            *o = bilinear(src, sx, sy);
        }
    });
    Linear { width: w, height: h, data }
}

pub fn bilinear(img: &Linear, x: f32, y: f32) -> [f32; 3] {
    let x = x.clamp(0.0, (img.width - 1) as f32);
    let y = y.clamp(0.0, (img.height - 1) as f32);
    let (x0, y0) = (x.floor() as u32, y.floor() as u32);
    let (x1, y1) = ((x0 + 1).min(img.width - 1), (y0 + 1).min(img.height - 1));
    let (fx, fy) = (x - x0 as f32, y - y0 as f32);
    let (a, b, c, d) = (img.get(x0, y0), img.get(x1, y0), img.get(x0, y1), img.get(x1, y1));
    std::array::from_fn(|i| (a[i] * (1.0 - fx) + b[i] * fx) * (1.0 - fy) + (c[i] * (1.0 - fx) + d[i] * fx) * fy)
}

#[cfg(test)]
mod tests {
    use super::*;
    use photos_core::edits::Crop;

    const RED: [f32; 3] = [1.0, 0.0, 0.0];

    fn marked(w: u32, h: u32) -> Linear {
        let mut img = Linear::new(w, h, [0.0; 3]);
        img.data[0] = RED; // top-left
        img
    }

    fn at(img: &Linear, x: u32, y: u32) -> [f32; 3] {
        img.get(x, y)
    }

    #[test]
    fn quarter_turn_and_flips_move_pixels_where_expected() {
        let img = marked(4, 2);
        let turned = apply(&img, &Recipe { rotate: 90, ..Recipe::identity() });
        assert_eq!((turned.width, turned.height), (2, 4));
        assert_eq!(at(&turned, 1, 0), RED, "clockwise: top-left goes top-right");
        let back = apply(&img, &Recipe { rotate: 270, ..Recipe::identity() });
        assert_eq!(at(&back, 0, 3), RED, "anticlockwise: top-left goes bottom-left");
        assert_eq!(at(&apply(&img, &Recipe { rotate: 180, ..Recipe::identity() }), 3, 1), RED);
        assert_eq!(at(&apply(&img, &Recipe { flip_h: true, ..Recipe::identity() }), 3, 0), RED);
        assert_eq!(at(&apply(&img, &Recipe { flip_v: true, ..Recipe::identity() }), 0, 1), RED);
    }

    #[test]
    fn crop_and_straighten_produce_the_frame_the_core_predicts() {
        let img = Linear::new(400, 300, [0.2; 3]);
        for r in [
            Recipe { crop: Some(Crop { x: 0.25, y: 0.1, w: 0.5, h: 0.5 }), ..Recipe::identity() },
            Recipe { rotate: 270, straighten: 7.5, crop: Some(Crop { x: 0.0, y: 0.0, w: 1.0, h: 0.5 }), ..Recipe::identity() },
            Recipe { rotate: 90, flip_v: true, crop: Some(Crop { x: 0.1, y: 0.2, w: 0.33, h: 0.7 }), ..Recipe::identity() },
        ] {
            let out = apply(&img, &r);
            assert_eq!((out.width, out.height), r.output_size(400, 300), "{r:?}");
        }
    }

    #[test]
    fn straighten_zooms_so_no_corner_leaves_the_photo() {
        for (w, h) in [(300u32, 200u32), (200, 300), (256, 256)] {
            for deg in [-45.0f32, -20.0, -3.5, 0.7, 12.0, 45.0] {
                for (x, y) in [(0.0, 0.0), (w as f32 - 1.0, 0.0), (0.0, h as f32 - 1.0), (w as f32 - 1.0, h as f32 - 1.0)] {
                    let (sx, sy) = straighten_source(x, y, w, h, deg);
                    // Pixel centres span [0, w-1]; a sample may sit up to half a pixel beyond.
                    assert!(sx >= -0.51 && sx <= w as f32 - 0.49 && sy >= -0.51 && sy <= h as f32 - 0.49, "{w}x{h} {deg}°: ({x},{y}) → ({sx},{sy})");
                }
            }
        }
    }
}
