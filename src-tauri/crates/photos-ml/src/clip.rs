//! The image encoder: a photo's pixels to a 512-d embedding and an aesthetic score.

use std::path::Path;

use ort::{inputs, session::Session, value::Tensor};
use photos_core::embedding::{quantize, Vector, DIM};

use crate::manifest::{Aesthetic, ClipInput, ModelEntry};
use crate::MlError;

/// Intra-op threads per session. Measured on the dev machine (24 cores): batches of 16 run
/// at 16.2 img/s on 2 threads and 18.8 on 4, so more threads buy little and would crowd
/// the Interactive lane.
const THREADS: usize = 4;

/// Centre square crop, bilinear resize to `input.size`, CHW, `(x/255 − mean) / std`.
pub fn preprocess(rgb: &[u8], width: u32, height: u32, input: &ClipInput) -> Vec<f32> {
    let (w, h, s) = (width as f32, height as f32, input.size as usize);
    let side = w.min(h);
    let (x0, y0) = ((w - side) / 2.0, (h - side) / 2.0);
    let mut out = vec![0f32; 3 * s * s];
    let px = |x: usize, y: usize, c: usize| rgb[(y * width as usize + x) * 3 + c] as f32 / 255.0;
    for oy in 0..s {
        for ox in 0..s {
            // Sample at the centre of the output pixel, in source coordinates.
            let sx = (x0 + (ox as f32 + 0.5) * side / s as f32 - 0.5).clamp(0.0, w - 1.0);
            let sy = (y0 + (oy as f32 + 0.5) * side / s as f32 - 0.5).clamp(0.0, h - 1.0);
            let (xa, ya) = (sx.floor() as usize, sy.floor() as usize);
            let (xb, yb) = ((xa + 1).min(width as usize - 1), (ya + 1).min(height as usize - 1));
            let (fx, fy) = (sx - xa as f32, sy - ya as f32);
            for c in 0..3 {
                let top = px(xa, ya, c) * (1.0 - fx) + px(xb, ya, c) * fx;
                let bottom = px(xa, yb, c) * (1.0 - fx) + px(xb, yb, c) * fx;
                let v = top * (1.0 - fy) + bottom * fy;
                out[c * s * s + oy * s + ox] = (v - input.mean[c]) / input.std[c];
            }
        }
    }
    out
}

/// The linear aesthetic head on an L2-normalised embedding, mapped onto 0..1.
pub fn aesthetic(embedding: &[f32], head: &Aesthetic) -> f32 {
    let raw: f32 = embedding.iter().zip(&head.weights).map(|(a, b)| a * b).sum::<f32>() + head.bias;
    ((raw - head.offset) / head.scale).clamp(0.0, 1.0)
}

/// One photo's pixels: packed RGB, row-major.
pub struct Rgb<'a> {
    pub data: &'a [u8],
    pub width: u32,
    pub height: u32,
}

pub struct ImageEncoder {
    session: Session,
    input: ClipInput,
    head: Option<Aesthetic>,
}

impl ImageEncoder {
    pub fn open(path: &Path, entry: &ModelEntry) -> Result<Self, MlError> {
        let input = entry.input.clone().ok_or_else(|| MlError::Missing(format!("{} has no input spec", entry.id)))?;
        let session = Session::builder()?.with_intra_threads(THREADS)?.commit_from_file(path)?;
        Ok(Self { session, input, head: entry.aesthetic.clone() })
    }

    /// Embeds several photos in one run — the model is several times faster per photo in a
    /// batch. Returns the quantised vector and the aesthetic (0..1) of each, in order.
    pub fn embed_batch(&mut self, photos: &[Rgb<'_>]) -> Result<Vec<(Vector, Option<f32>)>, MlError> {
        if photos.is_empty() {
            return Ok(Vec::new());
        }
        let s = self.input.size as usize;
        let mut pixels = Vec::with_capacity(photos.len() * 3 * s * s);
        for p in photos {
            pixels.extend(preprocess(p.data, p.width, p.height, &self.input));
        }
        let tensor = Tensor::from_array(([photos.len(), 3, s, s], pixels))?;
        let outputs = self.session.run(inputs!["pixels" => tensor])?;
        let (_, data) = outputs["embedding"].try_extract_tensor::<f32>()?;
        if data.len() != photos.len() * DIM {
            return Err(MlError::Shape(format!("{} values for {} photos", data.len(), photos.len())));
        }
        Ok(data
            .as_chunks::<DIM>()
            .0
            .iter()
            .map(|e| (quantize(e), self.head.as_ref().map(|h| aesthetic(e, h))))
            .collect())
    }

    pub fn embed(&mut self, rgb: &[u8], width: u32, height: u32) -> Result<(Vector, Option<f32>), MlError> {
        let mut out = self.embed_batch(&[Rgb { data: rgb, width, height }])?;
        Ok(out.remove(0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input() -> ClipInput {
        ClipInput { size: 4, mean: [0.5, 0.5, 0.5], std: [0.5, 0.5, 0.5] }
    }

    #[test]
    fn crops_the_centre_square_and_normalises() {
        // 8×4 image: left half black, right half white. The centre 4×4 square straddles the
        // edge, so the first columns are black (−1) and the last ones white (+1).
        let mut rgb = vec![0u8; 8 * 4 * 3];
        for y in 0..4 {
            for x in 4..8 {
                for c in 0..3 {
                    rgb[(y * 8 + x) * 3 + c] = 255;
                }
            }
        }
        let t = preprocess(&rgb, 8, 4, &input());
        assert_eq!(t.len(), 3 * 4 * 4);
        let at = |c: usize, y: usize, x: usize| t[c * 16 + y * 4 + x];
        assert!((at(0, 1, 0) + 1.0).abs() < 1e-3, "{}", at(0, 1, 0));
        assert!((at(2, 2, 3) - 1.0).abs() < 1e-3, "{}", at(2, 2, 3));
    }

    #[test]
    fn the_aesthetic_head_is_rescaled_into_0_to_1() {
        let head = Aesthetic { weights: vec![1.0; 4], bias: 0.0, offset: 0.0, scale: 2.0 };
        assert_eq!(aesthetic(&[0.5, 0.5, 0.5, 0.5], &head), 1.0);
        assert_eq!(aesthetic(&[-1.0, 0.0, 0.0, 0.0], &head), 0.0);
        assert!((aesthetic(&[0.25, 0.25, 0.0, 0.0], &head) - 0.25).abs() < 1e-6);
    }
}
