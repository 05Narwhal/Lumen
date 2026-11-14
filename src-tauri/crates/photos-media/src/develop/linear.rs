//! The develop pipeline's image: linear light, Rec.709/sRGB primaries, f32, unclamped.
//! Values below 0 and above 1 survive every stage, so nothing is clipped until output.

use std::sync::OnceLock;

use image::{DynamicImage, RgbImage};
use rayon::prelude::*;

/// Rec.709 luminance weights.
pub const LUMA: [f32; 3] = [0.2126, 0.7152, 0.0722];

#[inline]
pub fn luma(p: [f32; 3]) -> f32 {
    LUMA[0] * p[0] + LUMA[1] * p[1] + LUMA[2] * p[2]
}

/// The sRGB transfer function, decoded (0–1 in, linear out).
#[inline]
pub fn srgb_to_linear(v: f32) -> f32 {
    if v <= 0.04045 {
        v / 12.92
    } else {
        ((v + 0.055) / 1.055).powf(2.4)
    }
}

/// The sRGB transfer function, encoded. Mirrored for negatives so the curve stays monotone.
#[inline]
pub fn linear_to_srgb(v: f32) -> f32 {
    if v < 0.0 {
        return -linear_to_srgb(-v);
    }
    if v <= 0.003_130_8 {
        v * 12.92
    } else {
        1.055 * v.powf(1.0 / 2.4) - 0.055
    }
}

fn lut8() -> &'static [f32; 256] {
    static LUT: OnceLock<[f32; 256]> = OnceLock::new();
    LUT.get_or_init(|| std::array::from_fn(|i| srgb_to_linear(i as f32 / 255.0)))
}

/// Linear 0–1 on 65 536 steps → sRGB byte. Fine enough that every byte survives a round trip.
fn encode_lut() -> &'static [u8] {
    static LUT: OnceLock<Vec<u8>> = OnceLock::new();
    LUT.get_or_init(|| (0..65_536u32).map(|i| (linear_to_srgb(i as f32 / 65_535.0) * 255.0).round().clamp(0.0, 255.0) as u8).collect())
}

#[inline]
pub fn encode_u8(v: f32) -> u8 {
    let i = (v.clamp(0.0, 1.0) * 65_535.0).round() as usize;
    encode_lut()[i]
}

#[derive(Debug, Clone, PartialEq)]
pub struct Linear {
    pub width: u32,
    pub height: u32,
    pub data: Vec<[f32; 3]>,
}

impl Linear {
    pub fn new(width: u32, height: u32, fill: [f32; 3]) -> Self {
        Self { width, height, data: vec![fill; (width * height) as usize] }
    }

    pub fn long_edge(&self) -> u32 {
        self.width.max(self.height)
    }

    #[inline]
    pub fn get(&self, x: u32, y: u32) -> [f32; 3] {
        self.data[(y * self.width + x) as usize]
    }

    /// Decode an sRGB-encoded image. 16-bit sources keep their precision; float sources are
    /// taken as linear already.
    pub fn from_dynamic(img: &DynamicImage) -> Linear {
        let (width, height) = (img.width(), img.height());
        let data: Vec<[f32; 3]> = match img {
            DynamicImage::ImageRgb16(_) | DynamicImage::ImageRgba16(_) | DynamicImage::ImageLuma16(_) | DynamicImage::ImageLumaA16(_) => {
                let rgb = img.to_rgb16();
                rgb.as_raw().par_chunks_exact(3).map(|p| std::array::from_fn(|c| srgb_to_linear(p[c] as f32 / 65_535.0))).collect()
            }
            DynamicImage::ImageRgb32F(_) | DynamicImage::ImageRgba32F(_) => {
                let rgb = img.to_rgb32f();
                rgb.as_raw().as_chunks::<3>().0.to_vec()
            }
            _ => {
                let lut = lut8();
                let rgb = img.to_rgb8();
                rgb.as_raw().par_chunks_exact(3).map(|p| [lut[p[0] as usize], lut[p[1] as usize], lut[p[2] as usize]]).collect()
            }
        };
        Linear { width, height, data }
    }

    /// Encode to 8-bit sRGB, clamping.
    pub fn to_rgb8(&self) -> RgbImage {
        let mut out = vec![0u8; self.data.len() * 3];
        out.par_chunks_exact_mut(3).zip(self.data.par_iter()).for_each(|(o, p)| {
            o[0] = encode_u8(p[0]);
            o[1] = encode_u8(p[1]);
            o[2] = encode_u8(p[2]);
        });
        RgbImage::from_raw(self.width, self.height, out).expect("sized to fit")
    }

    /// Area-averaged downscale (in linear light, so highlights keep their energy) to a long
    /// edge of `edge`. Never upscales.
    pub fn resize_long_edge(&self, edge: u32) -> Linear {
        let long = self.long_edge();
        if long <= edge || edge == 0 {
            return self.clone();
        }
        let k = edge as f64 / long as f64;
        let nw = ((self.width as f64 * k).round() as u32).max(1);
        let nh = ((self.height as f64 * k).round() as u32).max(1);
        self.resize(nw, nh)
    }

    /// Decode an 8-bit sRGB image straight into a downscale with a long edge of `edge`:
    /// the horizontal pass reads bytes through the decode table, so the full-size float
    /// image is never made.
    pub fn from_rgb8_downscaled(img: &RgbImage, edge: u32) -> Linear {
        let (w, h) = img.dimensions();
        let long = w.max(h);
        if long <= edge || edge == 0 {
            return Linear::from_dynamic(&DynamicImage::ImageRgb8(img.clone()));
        }
        let k = edge as f64 / long as f64;
        let nw = ((w as f64 * k).round() as u32).max(1);
        let nh = ((h as f64 * k).round() as u32).max(1);
        let lut = lut8();
        let raw = img.as_raw();
        let taps = axis_taps(w, nw);
        let mut horiz = vec![[0f32; 3]; (nw * h) as usize];
        horiz.par_chunks_mut(nw as usize).enumerate().for_each(|(y, row)| {
            let base = y * w as usize * 3;
            for (i, o) in row.iter_mut().enumerate() {
                let mut acc = [0f32; 3];
                for &(j, wgt) in &taps[i] {
                    let p = base + j as usize * 3;
                    acc = [acc[0] + lut[raw[p] as usize] * wgt, acc[1] + lut[raw[p + 1] as usize] * wgt, acc[2] + lut[raw[p + 2] as usize] * wgt];
                }
                *o = acc;
            }
        });
        let data = resample_axis(&horiz, nw, h, nh, false);
        Linear { width: nw, height: nh, data }
    }

    /// Area-averaged resample to exactly `nw × nh` (a downscale; upscaling repeats pixels).
    pub fn resize(&self, nw: u32, nh: u32) -> Linear {
        let horiz = resample_axis(&self.data, self.width, self.height, nw, true);
        let data = resample_axis(&horiz, nw, self.height, nh, false);
        Linear { width: nw, height: nh, data }
    }
}

/// For output index `i` of `n` over a source of `len`: the source span `[a, b)` in source
/// pixels.
fn span(i: u32, n: u32, len: u32) -> (f64, f64) {
    let s = len as f64 / n as f64;
    (i as f64 * s, (i + 1) as f64 * s)
}

/// Weights per output index of an axis of `len` resampled to `n`, shared by every row (or
/// column).
fn axis_taps(len: u32, n: u32) -> Vec<Vec<(u32, f32)>> {
    (0..n)
        .map(|i| {
            let (a, b) = span(i, n, len);
            let (first, last) = (a.floor() as u32, (b.ceil() as u32).min(len).max(a.floor() as u32 + 1));
            let total = b - a;
            (first..last)
                .map(|j| {
                    let cover = (b.min(j as f64 + 1.0) - a.max(j as f64)).max(0.0);
                    (j.min(len - 1), (cover / total) as f32)
                })
                .filter(|(_, wgt)| *wgt > 0.0)
                .collect()
        })
        .collect()
}

/// Resample along x (`horizontal`) or y, averaging each output pixel's span with fractional
/// coverage at its ends.
fn resample_axis(src: &[[f32; 3]], w: u32, h: u32, n: u32, horizontal: bool) -> Vec<[f32; 3]> {
    let len = if horizontal { w } else { h };
    let taps = axis_taps(len, n);
    let (ow, oh) = if horizontal { (n, h) } else { (w, n) };
    let mut out = vec![[0f32; 3]; (ow * oh) as usize];
    if horizontal {
        out.par_chunks_mut(n as usize).enumerate().for_each(|(y, row)| {
            let base = y * w as usize;
            for (i, o) in row.iter_mut().enumerate() {
                let mut acc = [0f32; 3];
                for &(j, wgt) in &taps[i] {
                    let p = src[base + j as usize];
                    acc = [acc[0] + p[0] * wgt, acc[1] + p[1] * wgt, acc[2] + p[2] * wgt];
                }
                *o = acc;
            }
        });
    } else {
        out.par_chunks_mut(w as usize).enumerate().for_each(|(i, row)| {
            for (x, o) in row.iter_mut().enumerate() {
                let mut acc = [0f32; 3];
                for &(j, wgt) in &taps[i] {
                    let p = src[j as usize * w as usize + x];
                    acc = [acc[0] + p[0] * wgt, acc[1] + p[1] * wgt, acc[2] + p[2] * wgt];
                }
                *o = acc;
            }
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{Rgb, Rgb32FImage, RgbImage, ImageBuffer};

    #[test]
    fn every_byte_survives_a_round_trip() {
        let img = RgbImage::from_fn(256, 1, |x, _| Rgb([x as u8, 255 - x as u8, (x * 7 % 256) as u8]));
        let back = Linear::from_dynamic(&DynamicImage::ImageRgb8(img.clone())).to_rgb8();
        assert_eq!(back.as_raw(), img.as_raw());
    }

    #[test]
    fn sixteen_bit_keeps_its_precision() {
        let img: ImageBuffer<Rgb<u16>, Vec<u16>> = ImageBuffer::from_fn(3, 1, |x, _| Rgb([1000 + x as u16, 30_000, 65_535]));
        let lin = Linear::from_dynamic(&DynamicImage::ImageRgb16(img));
        let back = linear_to_srgb(lin.data[0][0]) * 65_535.0;
        assert!((back - 1000.0).abs() < 0.01, "{back}");
        assert!(lin.data[1][0] > lin.data[0][0], "one 16-bit step is still a step");
    }

    #[test]
    fn floats_are_taken_as_linear() {
        let img = Rgb32FImage::from_pixel(1, 1, Rgb([0.5, 2.0, -0.1]));
        assert_eq!(Linear::from_dynamic(&DynamicImage::ImageRgb32F(img)).data[0], [0.5, 2.0, -0.1]);
    }

    #[test]
    fn the_transfer_function_is_monotone_through_zero() {
        let xs: Vec<f32> = (-100..=200).map(|i| i as f32 / 100.0).collect();
        for w in xs.windows(2) {
            assert!(linear_to_srgb(w[1]) > linear_to_srgb(w[0]));
        }
    }

    #[test]
    fn decoding_straight_into_a_downscale_matches_decoding_then_downscaling() {
        let img = RgbImage::from_fn(301, 200, |x, y| Rgb([(x % 256) as u8, (y * 3 % 256) as u8, ((x + y) % 256) as u8]));
        let direct = Linear::from_rgb8_downscaled(&img, 100);
        let two_step = Linear::from_dynamic(&DynamicImage::ImageRgb8(img)).resize_long_edge(100);
        assert_eq!((direct.width, direct.height), (two_step.width, two_step.height));
        for (a, b) in direct.data.iter().zip(&two_step.data) {
            for c in 0..3 {
                assert!((a[c] - b[c]).abs() < 1e-5);
            }
        }
    }

    #[test]
    fn downscaling_keeps_the_mean_and_the_shape() {
        let mut img = Linear::new(301, 200, [0.0; 3]);
        for (i, p) in img.data.iter_mut().enumerate() {
            let v = (i % 7) as f32 / 6.0;
            *p = [v, v * 0.5, 1.0 - v];
        }
        let small = img.resize_long_edge(100);
        assert_eq!((small.width, small.height), (100, 66));
        let mean = |l: &Linear| l.data.iter().map(|p| p[0] as f64).sum::<f64>() / l.data.len() as f64;
        assert!((mean(&img) - mean(&small)).abs() < 0.01, "{} vs {}", mean(&img), mean(&small));
        assert_eq!(img.resize_long_edge(500), img, "never upscales");
    }
}
