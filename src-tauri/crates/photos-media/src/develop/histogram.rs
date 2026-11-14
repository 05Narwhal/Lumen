//! The Develop histogram, from the rendered preview (spec §3): 256 bins per channel and for
//! luminance, plus how many pixels clip; and the clipping overlay drawn onto a preview.

use image::RgbImage;
use rayon::prelude::*;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Histogram {
    pub r: Vec<u32>,
    pub g: Vec<u32>,
    pub b: Vec<u32>,
    pub luma: Vec<u32>,
    /// Pixels black in every channel.
    pub clipped_shadows: u32,
    /// Pixels white in any channel.
    pub clipped_highlights: u32,
}

impl Histogram {
    fn empty() -> Self {
        Self { r: vec![0; 256], g: vec![0; 256], b: vec![0; 256], luma: vec![0; 256], clipped_shadows: 0, clipped_highlights: 0 }
    }

    fn merge(mut self, o: Self) -> Self {
        for i in 0..256 {
            self.r[i] += o.r[i];
            self.g[i] += o.g[i];
            self.b[i] += o.b[i];
            self.luma[i] += o.luma[i];
        }
        self.clipped_shadows += o.clipped_shadows;
        self.clipped_highlights += o.clipped_highlights;
        self
    }
}

pub fn compute(img: &RgbImage) -> Histogram {
    img.as_raw()
        .par_chunks(3 * 4096)
        .fold(Histogram::empty, |mut h, chunk| {
            for p in chunk.as_chunks::<3>().0 {
                h.r[p[0] as usize] += 1;
                h.g[p[1] as usize] += 1;
                h.b[p[2] as usize] += 1;
                let y = (0.2126 * p[0] as f32 + 0.7152 * p[1] as f32 + 0.0722 * p[2] as f32).round() as usize;
                h.luma[y.min(255)] += 1;
                if p[0] == 0 && p[1] == 0 && p[2] == 0 {
                    h.clipped_shadows += 1;
                }
                if p[0] == 255 || p[1] == 255 || p[2] == 255 {
                    h.clipped_highlights += 1;
                }
            }
            h
        })
        .reduce(Histogram::empty, Histogram::merge)
}

/// Mark clipped pixels for display: highlights clipped in any channel red, shadows black in
/// every channel blue. Never used on an export.
pub fn clipping_overlay(img: &mut RgbImage) {
    img.par_chunks_exact_mut(3).for_each(|p| {
        if p[0] == 255 || p[1] == 255 || p[2] == 255 {
            p.copy_from_slice(&[255, 40, 40]);
        } else if p[0] == 0 && p[1] == 0 && p[2] == 0 {
            p.copy_from_slice(&[40, 80, 255]);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgb;

    #[test]
    fn counts_every_pixel_once_per_channel() {
        let img = RgbImage::from_fn(300, 200, |x, y| Rgb([(x % 256) as u8, (y % 256) as u8, 7]));
        let h = compute(&img);
        for ch in [&h.r, &h.g, &h.b, &h.luma] {
            assert_eq!(ch.iter().sum::<u32>(), 60_000);
        }
        assert_eq!(h.b[7], 60_000);
    }

    #[test]
    fn clipping_is_counted_and_drawn_only_where_it_happens() {
        let mut img = RgbImage::from_pixel(4, 1, Rgb([120, 120, 120]));
        img.put_pixel(0, 0, Rgb([255, 10, 10]));
        img.put_pixel(1, 0, Rgb([0, 0, 0]));
        let h = compute(&img);
        assert_eq!((h.clipped_highlights, h.clipped_shadows), (1, 1));
        clipping_overlay(&mut img);
        assert_eq!(img.get_pixel(0, 0).0, [255, 40, 40]);
        assert_eq!(img.get_pixel(1, 0).0, [40, 80, 255]);
        assert_eq!(img.get_pixel(2, 0).0, [120, 120, 120]);
    }
}
