//! Pets (Round 4C): D-FINE-S (COCO-only weights, Apache-2.0) finds cats and dogs. The export
//! carries D-FINE's own post-processor, so there is no anchor decoding and no NMS here: the
//! model takes the photo resized to 640×640 (a plain resize, RGB 0..1, no mean/std) with its
//! original size, and returns 300 (label, box in the photo's pixels, score) triples. Shipped in
//! fp32: converted to fp16 its distribution head overflows and it finds nothing.
//!
//! DINOv2 ViT-S/14 (Apache-2.0) then describes each animal from its square crop: which one
//! it is, as 384 numbers (stored quantised like every embedding).

use std::path::Path;

use ort::{inputs, session::Session, value::Tensor};
use photos_core::embedding::{quantize, Vector};

use crate::clip::Rgb;
use crate::manifest::{ClipInput, ModelEntry};
use crate::MlError;

/// D-FINE's contiguous COCO classes.
const LABEL_CAT: i64 = 15;
const LABEL_DOG: i64 = 16;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Species {
    Cat,
    Dog,
}

impl Species {
    pub fn as_str(self) -> &'static str {
        match self {
            Species::Cat => "cat",
            Species::Dog => "dog",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "cat" => Some(Species::Cat),
            "dog" => Some(Species::Dog),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PetBox {
    /// x0, y0, x1, y1 in the photo's pixels.
    pub bbox: [f32; 4],
    pub score: f32,
    pub species: Species,
}

/// The detector's settings from the manifest (`pets`: `size`, `score`, `min_side_px`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DetectSpec {
    pub size: u32,
    pub score: f32,
    pub min_side: f32,
}

impl DetectSpec {
    /// Measured on COCO val2017 (plan Task 1), one reading per animal: at 0.5, 90 % of cats and
    /// 87 % of dogs at 0.90 precision, and a pet reported on 18 of 752 photos of other animals.
    pub const MEASURED: DetectSpec = DetectSpec { size: 640, score: 0.5, min_side: 48.0 };

    pub fn from_entry(entry: &ModelEntry) -> Self {
        let get = |k: &str, d: f32| entry.pets.as_ref().and_then(|p| p.get(k)).and_then(|v| v.as_f64()).map(|v| v as f32).unwrap_or(d);
        let m = Self::MEASURED;
        DetectSpec { size: get("size", m.size as f32) as u32, score: get("score", m.score), min_side: get("min_side_px", m.min_side) }
    }
}

/// Boxes overlapping more than this are one animal: D-FINE may read it both as a cat and as
/// a dog (a black cat on a lawn, 0.76 and 0.64), and only the likelier reading is kept.
const SAME_ANIMAL_IOU: f32 = 0.7;

/// Overlap of two `x0, y0, x1, y1` boxes over their union.
fn iou(a: &[f32; 4], b: &[f32; 4]) -> f32 {
    let inter = (a[2].min(b[2]) - a[0].max(b[0])).max(0.0) * (a[3].min(b[3]) - a[1].max(b[1])).max(0.0);
    let union = (a[2] - a[0]) * (a[3] - a[1]) + (b[2] - b[0]) * (b[3] - b[1]) - inter;
    if union > 0.0 { inter / union } else { 0.0 }
}

/// D-FINE's outputs for one photo (`labels` [300], `boxes` [300×4], `scores` [300]) as the
/// cats and dogs worth keeping: at least `score`, at least `min_side` pixels on their short
/// side, one reading per animal, best first.
pub fn decode(labels: &[i64], boxes: &[f32], scores: &[f32], spec: &DetectSpec) -> Vec<PetBox> {
    let mut out: Vec<PetBox> = labels
        .iter()
        .zip(scores)
        .enumerate()
        .filter_map(|(i, (&label, &score))| {
            let species = match label {
                LABEL_CAT => Species::Cat,
                LABEL_DOG => Species::Dog,
                _ => return None,
            };
            let b = [boxes[i * 4], boxes[i * 4 + 1], boxes[i * 4 + 2], boxes[i * 4 + 3]];
            let short = (b[2] - b[0]).min(b[3] - b[1]);
            (score >= spec.score && short >= spec.min_side).then_some(PetBox { bbox: b, score, species })
        })
        .collect();
    out.sort_by(|a, b| b.score.total_cmp(&a.score));
    let mut kept: Vec<PetBox> = Vec::with_capacity(out.len());
    for b in out {
        if kept.iter().all(|k| iou(&k.bbox, &b.bbox) <= SAME_ANIMAL_IOU) {
            kept.push(b);
        }
    }
    kept
}

/// The square an animal is described from: its box grown by `margin` on each side, as square
/// as the photo allows, inside it. `(x, y, side)` in pixels.
pub fn pet_crop_rect(bbox: [f32; 4], width: u32, height: u32, margin: f32) -> (u32, u32, u32) {
    let (w, h) = (width as f32, height as f32);
    let side = ((bbox[2] - bbox[0]).max(bbox[3] - bbox[1]) * (1.0 + 2.0 * margin)).min(w).min(h).max(1.0);
    let (cx, cy) = ((bbox[0] + bbox[2]) / 2.0, (bbox[1] + bbox[3]) / 2.0);
    let x = (cx - side / 2.0).clamp(0.0, w - side);
    let y = (cy - side / 2.0).clamp(0.0, h - side);
    (x.round() as u32, y.round() as u32, (side.round() as u32).min(width).min(height))
}

/// The margin pets are cropped with (the evaluation's crops).
pub const CROP_MARGIN: f32 = 0.15;

/// D-FINE's input: the whole photo stretched to `size` square (bilinear), CHW, 0..1.
pub fn to_input(rgb: &[u8], width: u32, height: u32, size: u32) -> Vec<f32> {
    let (w, h, s) = (width as f32, height as f32, size as usize);
    let mut out = vec![0f32; 3 * s * s];
    let px = |x: usize, y: usize, c: usize| rgb[(y * width as usize + x) * 3 + c] as f32 / 255.0;
    for oy in 0..s {
        for ox in 0..s {
            let sx = ((ox as f32 + 0.5) * w / s as f32 - 0.5).clamp(0.0, w - 1.0);
            let sy = ((oy as f32 + 0.5) * h / s as f32 - 0.5).clamp(0.0, h - 1.0);
            let (xa, ya) = (sx.floor() as usize, sy.floor() as usize);
            let (xb, yb) = ((xa + 1).min(width as usize - 1), (ya + 1).min(height as usize - 1));
            let (fx, fy) = (sx - xa as f32, sy - ya as f32);
            for c in 0..3 {
                let top = px(xa, ya, c) * (1.0 - fx) + px(xb, ya, c) * fx;
                let bottom = px(xa, yb, c) * (1.0 - fx) + px(xb, yb, c) * fx;
                out[c * s * s + oy * s + ox] = top * (1.0 - fy) + bottom * fy;
            }
        }
    }
    out
}

/// DINOv2 ViT-S/14: an animal's square crop to who it is.
pub struct PetEmbedder {
    session: Session,
    input: ClipInput,
}

impl PetEmbedder {
    pub fn open(path: &Path, entry: &ModelEntry) -> Result<Self, MlError> {
        let input = entry.input.clone().ok_or_else(|| MlError::Missing(format!("{} has no input spec", entry.id)))?;
        Ok(Self { session: Session::builder()?.with_intra_threads(2)?.commit_from_file(path)?, input })
    }

    /// Square crops (from [`pet_crop_rect`]) to quantised embeddings, in order.
    pub fn embed_batch(&mut self, crops: &[Rgb<'_>]) -> Result<Vec<Vector>, MlError> {
        if crops.is_empty() {
            return Ok(Vec::new());
        }
        let s = self.input.size as usize;
        let mut pixels = Vec::with_capacity(crops.len() * 3 * s * s);
        for c in crops {
            pixels.extend(crate::clip::preprocess(c.data, c.width, c.height, &self.input));
        }
        let tensor = Tensor::from_array(([crops.len(), 3, s, s], pixels))?;
        let out = self.session.run(inputs!["crops" => tensor])?;
        let (shape, data) = out["embedding"].try_extract_tensor::<f32>()?;
        let dim = *shape.last().unwrap_or(&0) as usize;
        if dim == 0 || data.len() != crops.len() * dim {
            return Err(MlError::Shape(format!("{} values for {} crops", data.len(), crops.len())));
        }
        Ok(data.chunks(dim).map(quantize).collect())
    }
}

pub struct PetDetector {
    session: Session,
    spec: DetectSpec,
}

/// Runs ONNX Runtime's worker threads for one session through `on_start` first (Round 8A):
/// the pet scan lowers them to background priority. Without this they would run at normal
/// priority beside whatever the user is doing, whatever the calling thread's priority.
pub(crate) struct HookedThreads(pub(crate) fn());

impl ort::environment::ThreadManager for HookedThreads {
    type Thread = std::thread::JoinHandle<()>;

    fn create(&mut self, worker: ort::environment::ThreadWorker) -> ort::Result<Self::Thread> {
        let on_start = self.0;
        std::thread::Builder::new()
            .name("lumen-ort".into())
            .spawn(move || {
                on_start();
                worker.work();
            })
            .map_err(|e| ort::Error::new(e.to_string()))
    }

    fn join(thread: Self::Thread) -> ort::Result<()> {
        thread.join().map_err(|_| ort::Error::new("an ONNX Runtime thread panicked"))
    }
}

impl PetDetector {
    pub fn open(path: &Path, entry: &ModelEntry) -> Result<Self, MlError> {
        Self::open_with(path, entry, 2, None)
    }

    /// A detector on `threads` CPU threads (the caller's and `threads − 1` of ONNX Runtime's,
    /// never spinning); each of those runs `on_thread` first. DirectML is not offered: on
    /// D-FINE it misses clear pets outright (Round 8A measured 43 of 200 COCO photos).
    pub fn open_with(path: &Path, entry: &ModelEntry, threads: usize, on_thread: Option<fn()>) -> Result<Self, MlError> {
        let mut builder = Session::builder()?.with_intra_threads(threads.max(1))?.with_intra_op_spinning(false)?;
        if let Some(f) = on_thread {
            builder = builder.with_thread_manager(HookedThreads(f))?;
        }
        Ok(Self { session: builder.commit_from_file(path)?, spec: DetectSpec::from_entry(entry) })
    }

    /// The cats and dogs in a photo (packed RGB at any size); boxes in its pixels.
    pub fn detect(&mut self, rgb: &[u8], width: u32, height: u32) -> Result<Vec<PetBox>, MlError> {
        let s = self.spec.size as usize;
        if rgb.len() != (width * height * 3) as usize {
            return Err(MlError::Shape(format!("a {width}×{height} RGB image is {} bytes, not {}", width * height * 3, rgb.len())));
        }
        let input = to_input(rgb, width, height, self.spec.size);
        let images = Tensor::from_array(([1usize, 3, s, s], input))?;
        let sizes = Tensor::from_array(([1usize, 2], vec![width as i64, height as i64]))?;
        let out = self.session.run(inputs!["images" => images, "orig_target_sizes" => sizes])?;
        let (_, labels) = out["labels"].try_extract_tensor::<i64>()?;
        let (_, boxes) = out["boxes"].try_extract_tensor::<f32>()?;
        let (_, scores) = out["scores"].try_extract_tensor::<f32>()?;
        Ok(decode(labels, boxes, scores, &self.spec))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_cats_and_dogs_over_the_score_and_size_best_first() {
        let spec = DetectSpec { size: 640, score: 0.5, min_side: 48.0 };
        // A dog, a cat, a horse (17), a weak cat, and a cat too small to describe.
        let labels = [16, 15, 17, 15, 15];
        let boxes = [10.0, 10.0, 210.0, 160.0, 300.0, 50.0, 400.0, 200.0, 0.0, 0.0, 500.0, 500.0, 5.0, 5.0, 300.0, 300.0, 20.0, 20.0, 60.0, 60.0];
        let scores = [0.7, 0.9, 0.99, 0.3, 0.95];
        let got = decode(&labels, &boxes, &scores, &spec);
        assert_eq!(got.len(), 2);
        assert_eq!((got[0].species, got[0].score), (Species::Cat, 0.9));
        assert_eq!(got[0].bbox, [300.0, 50.0, 400.0, 200.0]);
        assert_eq!(got[1].species, Species::Dog);
    }

    #[test]
    fn one_animal_read_as_both_a_cat_and_a_dog_is_kept_once_as_the_likelier() {
        let spec = DetectSpec { size: 640, score: 0.5, min_side: 48.0 };
        // D-FINE on a public-domain photo of Socks the Cat: the same box as a cat at 0.76 and
        // a dog at 0.64. A dog lying beside it, overlapping a little, stays.
        let labels = [15, 16, 16];
        let boxes = [550.0, 342.0, 816.0, 523.0, 550.0, 342.0, 816.0, 523.0, 358.0, 173.0, 741.0, 393.0];
        let scores = [0.76, 0.64, 0.95];
        let got = decode(&labels, &boxes, &scores, &spec);
        assert_eq!(got.iter().map(|b| b.species).collect::<Vec<_>>(), vec![Species::Dog, Species::Cat]);
    }

    #[test]
    fn a_crop_is_a_square_around_the_animal_inside_the_photo() {
        // 100 × 50 box in the middle of a 1000 × 800 photo: 130 square around its centre.
        assert_eq!(pet_crop_rect([450.0, 375.0, 550.0, 425.0], 1000, 800, 0.15), (435, 335, 130));
        // At the corner it is pushed back inside.
        assert_eq!(pet_crop_rect([0.0, 0.0, 100.0, 100.0], 1000, 800, 0.15), (0, 0, 130));
        // Never bigger than the photo's short side.
        let (x, y, side) = pet_crop_rect([0.0, 0.0, 1000.0, 800.0], 1000, 800, 0.15);
        assert_eq!((side, y), (800, 0));
        assert!(x + side <= 1000);
    }

    #[test]
    fn the_input_is_the_whole_photo_stretched_square() {
        // 4×2: left half black, right half white; at 2×2 the left column is black, the right white.
        let rgb: Vec<u8> = (0..8).flat_map(|i| if i % 4 < 2 { [0, 0, 0] } else { [255, 255, 255] }).collect();
        let x = to_input(&rgb, 4, 2, 2);
        assert_eq!((x[0], x[1]), (0.0, 1.0));
        assert_eq!(x.len(), 12);
    }

    #[test]
    fn species_round_trip_as_text() {
        for s in [Species::Cat, Species::Dog] {
            assert_eq!(Species::parse(s.as_str()), Some(s));
        }
        assert_eq!(Species::parse("horse"), None);
    }
}
