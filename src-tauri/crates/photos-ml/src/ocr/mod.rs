//! Reading the text in photos (Round 13): PaddleOCR's PP-OCRv5 detector and Latin recogniser
//! through `ort`, with PaddleOCR's pre/post-processing ported to Rust (Apache-2.0,
//! https://github.com/PaddlePaddle/PaddleOCR) and golden-tested against it
//! (`tests/ocr_model.rs`).

pub mod detect;
pub mod recognize;

use std::path::Path;

use ort::{inputs, session::Session, value::Tensor};

use crate::manifest::{ModelEntry, OcrDetSpec, OcrRecSpec};
use crate::pets::HookedThreads;
use crate::MlError;

/// A text box, clockwise from top-left, in pixels of the image it was found in.
pub type Quad = [[f32; 2]; 4];

/// Lines recognised per run (PaddleX sorts them by shape first, so a batch pads little).
const REC_BATCH: usize = 16;

/// One line of text read in an image.
#[derive(Debug, Clone, PartialEq)]
pub struct TextLine {
    /// Where the detector found it, in pixels of the image given to [`OcrEngine::read`].
    pub quad: Quad,
    pub text: String,
    /// The recogniser's mean confidence over the characters it kept (0 when none).
    pub conf: f32,
}

/// PP-OCRv5's text detector and Latin recogniser, run the way PaddleOCR 3's pipeline runs
/// them without its document and line-orientation stages.
pub struct OcrEngine {
    det: Session,
    rec: Session,
    det_out: String,
    rec_out: String,
    det_spec: OcrDetSpec,
    rec_spec: OcrRecSpec,
}

fn session(path: &Path, threads: usize, on_thread: Option<fn()>) -> Result<(Session, String), MlError> {
    let mut builder = Session::builder()?.with_intra_threads(threads.max(1))?.with_intra_op_spinning(false)?;
    if let Some(f) = on_thread {
        builder = builder.with_thread_manager(HookedThreads(f))?;
    }
    let s = builder.commit_from_file(path)?;
    let out = s.outputs.first().map(|o| o.name.clone()).ok_or_else(|| MlError::Shape(format!("{} has no output", path.display())))?;
    Ok((s, out))
}

impl OcrEngine {
    /// Both models on `threads` CPU threads each, never spinning; each ONNX Runtime thread
    /// runs `on_thread` first (the background scan lowers their priority). An entry without
    /// its `ocr_det` / `ocr_rec` spec is refused.
    pub fn open(det: (&Path, &ModelEntry), rec: (&Path, &ModelEntry), threads: usize, on_thread: Option<fn()>) -> Result<Self, MlError> {
        let det_spec = det.1.ocr_det.clone().ok_or_else(|| MlError::Missing(format!("{} has no ocr_det spec", det.1.id)))?;
        let rec_spec = rec.1.ocr_rec.clone().ok_or_else(|| MlError::Missing(format!("{} has no ocr_rec spec", rec.1.id)))?;
        let (det, det_out) = session(det.0, threads, on_thread)?;
        let (rec, rec_out) = session(rec.0, threads, on_thread)?;
        Ok(Self { det, rec, det_out, rec_out, det_spec, rec_spec })
    }

    /// The text in a packed RGB image, in reading order. Detection runs on the image with
    /// its long side brought down to `det_side`; lines are cut from the full image. Nothing
    /// is filtered by confidence here.
    pub fn read(&mut self, rgb: &[u8], w: u32, h: u32, det_side: u32) -> Result<Vec<TextLine>, MlError> {
        if w == 0 || h == 0 || rgb.len() != (w * h * 3) as usize {
            return Err(MlError::Shape(format!("a {w}×{h} RGB image is {} bytes, not {}", w as u64 * h as u64 * 3, rgb.len())));
        }
        let mut boxes = self.detect(rgb, w, h, det_side)?;
        if boxes.is_empty() {
            return Ok(Vec::new());
        }
        detect::reading_order(&mut boxes);
        let crops: Vec<recognize::Crop> = boxes.iter().map(|(q, _)| recognize::crop_quad(rgb, w, h, &detect::crop_box(q))).collect();
        let mut order: Vec<usize> = (0..crops.len()).collect();
        let ratio = |i: usize| crops[i].w as f64 / crops[i].h as f64;
        order.sort_by(|&a, &b| ratio(a).total_cmp(&ratio(b)));
        let mut read: Vec<(String, f32)> = vec![(String::new(), 0.0); crops.len()];
        for batch in order.chunks(REC_BATCH) {
            let refs: Vec<&recognize::Crop> = batch.iter().map(|&i| &crops[i]).collect();
            for (&i, line) in batch.iter().zip(self.recognize(&refs)?) {
                read[i] = line;
            }
        }
        Ok(boxes.into_iter().zip(read).map(|((quad, _), (text, conf))| TextLine { quad, text, conf }).collect())
    }

    fn detect(&mut self, rgb: &[u8], w: u32, h: u32, side: u32) -> Result<Vec<(Quad, f32)>, MlError> {
        let (input, dw, dh) = detect::det_tensor(rgb, w, h, side);
        let x = Tensor::from_array(([1usize, 3, dh as usize, dw as usize], input))?;
        let out = self.det.run(inputs!["x" => x])?;
        let (_, prob) = out[self.det_out.as_str()].try_extract_tensor::<f32>()?;
        if prob.len() != (dw * dh) as usize {
            return Err(MlError::Shape(format!("{} probabilities for a {dw}×{dh} map", prob.len())));
        }
        Ok(detect::boxes_from_map(prob, dw, dh, w, h, &self.det_spec))
    }

    fn recognize(&mut self, crops: &[&recognize::Crop]) -> Result<Vec<(String, f32)>, MlError> {
        let (input, width) = recognize::rec_batch(crops, &self.rec_spec);
        let x = Tensor::from_array(([crops.len(), 3, self.rec_spec.height as usize, width as usize], input))?;
        let out = self.rec.run(inputs!["x" => x])?;
        let (shape, probs) = out[self.rec_out.as_str()].try_extract_tensor::<f32>()?;
        let classes = self.rec_spec.chars.len() + 2;
        if shape.len() != 3 || shape[0] as usize != crops.len() || shape[2] as usize != classes {
            return Err(MlError::Shape(format!("recogniser returned {shape:?} for {} crops and {classes} classes", crops.len())));
        }
        let steps = shape[1] as usize;
        Ok(probs.chunks(steps * classes).map(|p| recognize::ctc_greedy(p, steps, classes, &self.rec_spec.chars)).collect())
    }
}

/// OpenCV's `resize(…, INTER_LINEAR)` on packed RGB: pixel centres aligned, edges clamped,
/// no anti-aliasing (PaddleOCR resizes with it, so the models were fed it).
pub(crate) fn resize_bilinear(rgb: &[u8], w: u32, h: u32, dw: u32, dh: u32) -> Vec<u8> {
    if (w, h) == (dw, dh) {
        return rgb.to_vec();
    }
    let axis = |d: u32, from: u32, to: u32| {
        let s = (d as f32 + 0.5) * from as f32 / to as f32 - 0.5;
        let s = s.max(0.0);
        let i0 = (s.floor() as u32).min(from - 1);
        let i1 = (i0 + 1).min(from - 1);
        (i0 as usize, i1 as usize, s - i0 as f32)
    };
    let xs: Vec<_> = (0..dw).map(|x| axis(x, w, dw)).collect();
    let mut out = Vec::with_capacity((dw * dh * 3) as usize);
    let row = w as usize * 3;
    for y in 0..dh {
        let (y0, y1, fy) = axis(y, h, dh);
        for &(x0, x1, fx) in &xs {
            for c in 0..3 {
                let at = |yy: usize, xx: usize| rgb[yy * row + xx * 3 + c] as f32;
                let top = at(y0, x0) * (1.0 - fx) + at(y0, x1) * fx;
                let bottom = at(y1, x0) * (1.0 - fx) + at(y1, x1) * fx;
                out.push((top * (1.0 - fy) + bottom * fy).round().clamp(0.0, 255.0) as u8);
            }
        }
    }
    out
}
