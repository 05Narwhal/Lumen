//! Lumen Facet (Phase 8f): EoMT-S COCO panoptic through `ort`, turned into the masks the panel
//! offers. The maths is `scripts/ml/eval_facet.py`'s, held to it by the fixtures
//! `scripts/ml/golden_facet.py` writes (below and `tests/facet_model.rs`).

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use image::{imageops, RgbImage};
use ort::{inputs, session::Session, value::Tensor};
use photos_core::ai_masks::AiMaskKind;

use crate::manifest::{ClipInput, ModelEntry};
use crate::MlError;

/// A query counts when its best class scores at least this.
const SCORE_MIN: f32 = 0.5;
/// …and when it keeps at least this share of its own confident pixels.
const KEEP_MIN: f32 = 0.8;
/// False skies score lower than real ones: backdrops 0.93 and 0.92, a dark ceiling 0.78, real
/// skies 0.98 and up (`docs/THIRD-PARTY-MODELS.md`).
pub const SKY_SCORE_MIN: f32 = 0.95;
/// A thing smaller than this share of the photo is never the subject.
const SUBJECT_AREA_MIN: f64 = 0.003;
/// A quarter, not a half: at a half, one of a group portrait's people fell into Background.
const SAME_KIND_MIN: f64 = 0.25;
/// `class_logits`' COCO labels.
const PERSON: u32 = 0;
const LIVING: std::ops::RangeInclusive<u32> = 14..=23; // bird … giraffe; person too
const THINGS_END: u32 = 80;
const SKY: u32 = 119;

/// One mask the model found, at most 160 px on its long side, the photo's aspect only.
#[derive(Debug, Clone, PartialEq)]
pub struct Segment {
    pub kind: AiMaskKind,
    /// Which person, from 0; 0 for the other kinds.
    pub instance: u32,
    pub score: f32,
    pub width: u32,
    pub height: u32,
    pub weights: Vec<u8>,
}

/// The model, open on the GPU (DirectML) or the CPU.
pub struct FacetModel {
    session: Session,
    input: ClipInput,
    ep: &'static str,
}

impl FacetModel {
    /// `gpu` tries DirectML first; if it cannot open the model the CPU takes over. An entry
    /// without its `input` spec is refused.
    pub fn open(path: &Path, entry: &ModelEntry, gpu: bool) -> Result<Self, MlError> {
        let input = entry.input.clone().ok_or_else(|| MlError::Missing(format!("{} has no input spec", entry.id)))?;
        let on_gpu = if gpu {
            crate::retouch::gpu_session(path).inspect_err(|e| tracing::warn!("facet: GPU unavailable, using the CPU: {e}")).ok()
        } else {
            None
        };
        let (session, ep) = match on_gpu {
            Some(s) => (s, "directml"),
            None => (Session::builder()?.with_intra_threads(4)?.with_intra_op_spinning(false)?.commit_from_file(path)?, "cpu"),
        };
        Ok(Self { session, input, ep })
    }

    /// The execution provider the session runs on: `"directml"` or `"cpu"`.
    pub fn ep(&self) -> &'static str {
        self.ep
    }
}

/// The masks on a photo, display-oriented and before any edit. `cancel` is looked at around the
/// model's run; a cancelled run is an error, never a partial result.
pub fn detect(model: &mut FacetModel, img: &RgbImage, cancel: &AtomicBool) -> Result<Vec<Segment>, MlError> {
    let cancelled = || if cancel.load(Ordering::Relaxed) { Err(MlError::Runtime("cancelled".into())) } else { Ok(()) };
    cancelled()?;
    let (pixels, used) = to_input(img, &model.input)?;
    let size = model.input.size as usize;
    let x = Tensor::from_array(([1usize, 3, size, size], pixels))?;
    let out = model.session.run(inputs!["pixels" => x])?;
    cancelled()?;
    let (cls_shape, cls) = out["class_logits"].try_extract_tensor::<f32>()?;
    let (mask_shape, masks) = out["mask_logits"].try_extract_tensor::<f32>()?;
    let (&[1, q, c], &[1, q2, mh, mw]) = (&cls_shape[..], &mask_shape[..]) else {
        return Err(MlError::Shape(format!("class_logits {cls_shape:?}, mask_logits {mask_shape:?}")));
    };
    if q != q2 || c as u32 <= SKY {
        return Err(MlError::Shape(format!("class_logits {cls_shape:?}, mask_logits {mask_shape:?}")));
    }
    let merged = merge(cls, q as usize, c as usize, masks, mh as usize, mw as usize, used, model.input.size);
    Ok(segments(&merged))
}

fn norm(v: u8, input: &ClipInput, ch: usize) -> f32 {
    (v as f32 / 255.0 - input.mean[ch]) / input.std[ch]
}

/// `EomtImageProcessor`: the long side brought to `size` (bilinear, antialiased), normalised,
/// padded with black at the bottom and right. Channel-major; also the part of the input the photo
/// takes up.
fn to_input(img: &RgbImage, input: &ClipInput) -> Result<(Vec<f32>, (u32, u32)), MlError> {
    let (w, h) = img.dimensions();
    if w == 0 || h == 0 {
        return Err(MlError::Shape(format!("a {w}×{h} image")));
    }
    let size = input.size;
    let scale = size as f64 / w.max(h) as f64;
    let side = |v: u32| (((v as f64 * scale).round_ties_even()) as u32).clamp(1, size);
    let (nw, nh) = (side(w), side(h));
    let small = imageops::resize(img, nw, nh, imageops::FilterType::Triangle);
    let s = size as usize;
    let mut out = Vec::with_capacity(3 * s * s);
    for ch in 0..3 {
        let black = norm(0, input, ch);
        let start = out.len();
        out.resize(start + s * s, black);
        for (i, px) in small.pixels().enumerate() {
            let (x, y) = (i % nw as usize, i / nw as usize);
            out[start + y * s + x] = norm(px[ch], input, ch);
        }
    }
    Ok((out, (nw, nh)))
}

/// A panoptic reading of the photo: each pixel's owner, and what each kept query is.
#[derive(Debug, PartialEq)]
struct Merged {
    width: usize,
    height: usize,
    /// Each pixel's index into `found`, or −1.
    seg: Vec<i32>,
    /// Label and score of each kept query.
    found: Vec<(u32, f32)>,
    /// Each kept query's sigmoid over the whole photo.
    soft: Vec<Vec<f32>>,
}

/// Per query, class score × sigmoid(mask); each pixel goes to the query with the highest product;
/// a query stays if its score and the share of its own confident pixels it keeps are high enough.
/// The padding is cropped. `cls` is queries × classes (the last class is "no object"), `masks`
/// queries × `mh` × `mw`, `used` the part of the `input_size` square the photo takes up.
#[allow(clippy::too_many_arguments)]
fn merge(cls: &[f32], queries: usize, classes: usize, masks: &[f32], mh: usize, mw: usize, used: (u32, u32), input_size: u32) -> Merged {
    let side = |u: u32, m: usize| ((u as f64 * m as f64 / input_size as f64).round_ties_even() as usize).clamp(1, m);
    let (width, height) = (side(used.0, mw), side(used.1, mh));
    let area = width * height;

    let mut candidates: Vec<(u32, f32, Vec<f32>)> = Vec::new();
    for q in 0..queries {
        let logits = &cls[q * classes..(q + 1) * classes];
        let top = logits.iter().copied().fold(f32::NEG_INFINITY, f32::max);
        let total: f32 = logits.iter().map(|l| (l - top).exp()).sum();
        let (label, score) = logits[..classes - 1]
            .iter()
            .enumerate()
            .fold((0, f32::NEG_INFINITY), |best, (i, l)| if *l > best.1 { (i as u32, *l) } else { best });
        let score = (score - top).exp() / total;
        if score < SCORE_MIN {
            continue;
        }
        let mut sig = Vec::with_capacity(area);
        let logits = &masks[q * mh * mw..(q + 1) * mh * mw];
        for row in logits.chunks_exact(mw).take(height) {
            // Clamped: exp(−x) overflows on the large logits a confident model gives.
            sig.extend(row[..width].iter().map(|m| 1.0 / (1.0 + (-m.clamp(-40.0, 40.0)).exp())));
        }
        candidates.push((label, score, sig));
    }

    let mut owner = vec![0usize; area];
    for (p, o) in owner.iter_mut().enumerate() {
        let mut best = f32::NEG_INFINITY;
        for (i, (_, score, sig)) in candidates.iter().enumerate() {
            let v = sig[p] * score;
            if v > best {
                best = v;
                *o = i;
            }
        }
    }

    let mut merged = Merged { width, height, seg: vec![-1; area], found: Vec::new(), soft: Vec::new() };
    for (i, (label, score, sig)) in candidates.into_iter().enumerate() {
        let kept: Vec<usize> = (0..area).filter(|&p| owner[p] == i && sig[p] >= 0.5).collect();
        let confident = sig.iter().filter(|v| **v >= 0.5).count();
        if kept.is_empty() || (kept.len() as f32 / confident as f32) < KEEP_MIN {
            continue;
        }
        let id = merged.found.len() as i32;
        for p in kept {
            merged.seg[p] = id;
        }
        merged.found.push((label, score));
        merged.soft.push(sig);
    }
    merged
}

/// The subject: the most prominent thing, weighted for size and centre, living things first; with
/// the others of its kind that score at least a quarter as much. Its weights and best score.
fn subject(m: &Merged) -> Option<(Vec<f32>, f32)> {
    let mut ranked: Vec<(f64, usize)> = Vec::new();
    for (i, &(label, score)) in m.found.iter().enumerate() {
        if label >= THINGS_END {
            continue;
        }
        let (mut n, mut sx, mut sy) = (0u64, 0u64, 0u64);
        for (p, _) in m.seg.iter().enumerate().filter(|(_, s)| **s == i as i32) {
            n += 1;
            sx += (p % m.width) as u64;
            sy += (p / m.width) as u64;
        }
        let area = n as f64 / m.seg.len() as f64;
        if area < SUBJECT_AREA_MIN {
            continue;
        }
        let (cx, cy) = (sx as f64 / n as f64 / m.width as f64, sy as f64 / n as f64 / m.height as f64);
        let off = ((cx - 0.5).hypot(cy - 0.5) / std::f64::consts::FRAC_1_SQRT_2).min(1.0);
        let weight = if label == PERSON || LIVING.contains(&label) { 2.0 } else { 1.0 };
        ranked.push((area * (1.0 - 0.6 * off) * weight * score as f64, i));
    }
    let &(top, top_i) = ranked.iter().max_by(|a, b| a.0.total_cmp(&b.0))?;
    let top_label = m.found[top_i].0;
    let chosen: Vec<usize> = ranked.iter().filter(|(s, i)| *s >= top * SAME_KIND_MIN && m.found[*i].0 == top_label).map(|(_, i)| *i).collect();
    let mut out = vec![0f32; m.seg.len()];
    for &i in &chosen {
        for (p, v) in out.iter_mut().enumerate() {
            // Its own pixels, and those nobody kept: they soften its edge.
            if m.seg[p] == i as i32 || m.seg[p] == -1 {
                *v = v.max(m.soft[i][p]);
            }
        }
    }
    let score = chosen.iter().map(|&i| m.found[i].1).fold(0.0, f32::max);
    Some((out, score))
}

fn to_u8(weights: &[f32]) -> Vec<u8> {
    weights.iter().map(|w| (w.clamp(0.0, 1.0) * 255.0).round() as u8).collect()
}

/// What is stored for a photo. A sky under the bar is no sky; no subject means no Background.
fn segments(m: &Merged) -> Vec<Segment> {
    let seg = |kind, instance, score, weights| Segment { kind, instance, score, width: m.width as u32, height: m.height as u32, weights };
    let mut out = Vec::new();
    let skies: Vec<usize> = (0..m.found.len()).filter(|&i| m.found[i].0 == SKY && m.found[i].1 >= SKY_SCORE_MIN).collect();
    if !skies.is_empty() {
        let mut sky = vec![0f32; m.seg.len()];
        for &i in &skies {
            sky.iter_mut().zip(&m.soft[i]).for_each(|(a, b)| *a = a.max(*b));
        }
        let score = skies.iter().map(|&i| m.found[i].1).fold(0.0, f32::max);
        out.push(seg(AiMaskKind::Sky, 0, score, to_u8(&sky)));
    }
    if let Some((weights, score)) = subject(m) {
        let weights = to_u8(&weights);
        let inverse = weights.iter().map(|w| 255 - w).collect();
        out.push(seg(AiMaskKind::Subject, 0, score, weights));
        out.push(seg(AiMaskKind::Background, 0, score, inverse));
    }
    let people = (0..m.found.len()).filter(|&i| m.found[i].0 == PERSON);
    for (n, i) in people.enumerate() {
        let own: Vec<f32> = m.soft[i].iter().zip(&m.seg).map(|(v, s)| if *s == i as i32 { *v } else { 0.0 }).collect();
        out.push(seg(AiMaskKind::Person, n as u32, m.found[i].1, to_u8(&own)));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;

    fn input() -> ClipInput {
        crate::manifest::manifest().by_role("segmentation").expect("Facet is in the manifest").input.clone().expect("its input spec")
    }

    #[derive(Deserialize)]
    struct Preprocess {
        width: u32,
        height: u32,
        used: (u32, u32),
        samples: Vec<(usize, usize, f32, f32, f32)>,
    }

    /// `to_input` against `EomtImageProcessor` on a public photo: within a pixel level inside the
    /// photo, and black padding (the model was trained on it; mean grey shifts its scores).
    #[test]
    fn preprocess_matches_the_processor() {
        let want: Preprocess = serde_json::from_str(include_str!("../tests/fixtures/facet/preprocess.json")).unwrap();
        let img = image::load_from_memory(include_bytes!("../tests/fixtures/facet/preprocess.png")).unwrap().to_rgb8();
        assert_eq!(img.dimensions(), (want.width, want.height));
        let spec = input();
        let (got, used) = to_input(&img, &spec).unwrap();
        assert_eq!(used, want.used);
        let s = spec.size as usize;
        assert_eq!(got.len(), 3 * s * s);
        let level = |ch: usize, x: usize, y: usize| (got[ch * s * s + y * s + x] * spec.std[ch] + spec.mean[ch]) * 255.0;
        let mut worst = 0f32;
        for &(x, y, r, g, b) in &want.samples {
            for (ch, v) in [r, g, b].into_iter().enumerate() {
                worst = worst.max((level(ch, x, y) - v).abs());
            }
        }
        assert!(worst <= 1.0 + 1e-3, "{worst} levels from the processor");
        for ch in 0..3 {
            assert!(level(ch, s - 1, s - 1).abs() < 1e-3, "the padding is black");
            assert!(level(ch, 0, used.1 as usize).abs() < 1e-3, "the padding starts right under the photo");
            assert!(level(ch, 0, used.1 as usize - 1).abs() > 1.0 || want.used.1 == 0, "the photo ends where it should");
        }
    }

    #[test]
    fn an_empty_image_is_refused() {
        assert!(to_input(&RgbImage::new(0, 4), &input()).is_err());
    }

    #[test]
    fn a_tall_photo_is_padded_on_the_right() {
        let (got, used) = to_input(&RgbImage::from_pixel(100, 200, image::Rgb([255, 255, 255])), &input()).unwrap();
        assert_eq!(used, (320, 640));
        let spec = input();
        let s = spec.size as usize;
        let level = |x: usize, y: usize| (got[y * s + x] * spec.std[0] + spec.mean[0]) * 255.0;
        assert!((level(319, 639) - 255.0).abs() < 1e-2);
        assert!(level(320, 0).abs() < 1e-3);
    }

    #[derive(Deserialize)]
    struct Case {
        name: String,
        queries: usize,
        classes: usize,
        side: usize,
        used: (u32, u32),
        cls: Vec<f32>,
        masks: Vec<f32>,
        seg: Vec<i32>,
        found: Vec<(u32, f32)>,
        rows: Vec<Row>,
    }

    #[derive(Deserialize)]
    struct Row {
        kind: AiMaskKind,
        instance: u32,
        score: f32,
        width: u32,
        height: u32,
        weights: Vec<u8>,
    }

    fn cases() -> Vec<Case> {
        serde_json::from_str(include_str!("../tests/fixtures/facet/synthetic.json")).unwrap()
    }

    fn merged(c: &Case) -> Merged {
        merge(&c.cls, c.queries, c.classes, &c.masks, c.side, c.side, c.used, 640)
    }

    #[test]
    fn merge_matches_eval_facet_pixel_for_pixel() {
        for c in cases() {
            let m = merged(&c);
            assert_eq!(m.seg, c.seg, "{}: the segment map", c.name);
            assert_eq!(m.found.len(), c.found.len(), "{}", c.name);
            for (got, want) in m.found.iter().zip(&c.found) {
                assert_eq!(got.0, want.0, "{}: label", c.name);
                assert!((got.1 - want.1).abs() < 1e-4, "{}: score {} vs {}", c.name, got.1, want.1);
            }
        }
    }

    #[test]
    fn segments_match_eval_facet_within_a_weight_level() {
        for c in cases() {
            let got = segments(&merged(&c));
            let kinds: Vec<_> = got.iter().map(|s| (s.kind, s.instance)).collect();
            assert_eq!(kinds, c.rows.iter().map(|r| (r.kind, r.instance)).collect::<Vec<_>>(), "{}", c.name);
            for (s, r) in got.iter().zip(&c.rows) {
                assert_eq!((s.width, s.height), (r.width, r.height), "{}", c.name);
                assert!((s.score - r.score).abs() < 1e-4, "{}: {:?} score", c.name, s.kind);
                let off = s.weights.iter().zip(&r.weights).map(|(a, b)| a.abs_diff(*b)).max().unwrap();
                assert!(off <= 1, "{}: {:?} {} is {off} levels off", c.name, s.kind, s.instance);
            }
        }
    }

    fn by_name(name: &str) -> Vec<Segment> {
        let c = cases().into_iter().find(|c| c.name == name).unwrap();
        segments(&merged(&c))
    }

    #[test]
    fn a_sky_under_the_bar_is_no_sky() {
        // The fixture must really have a sky between SCORE_MIN and the bar, or this proves nothing.
        let all = cases();
        let c = all.iter().find(|c| c.name == "backdrop").unwrap();
        let score = merged(c).found.iter().find(|f| f.0 == SKY).map(|f| f.1).expect("the model saw a sky");
        assert!((SCORE_MIN..SKY_SCORE_MIN).contains(&score), "sky scored {score}");
        assert!(by_name("backdrop").iter().all(|s| s.kind != AiMaskKind::Sky));
    }

    #[test]
    fn a_real_sky_with_no_subject_has_no_background() {
        let got = by_name("landscape");
        assert_eq!(got.iter().map(|s| s.kind).collect::<Vec<_>>(), [AiMaskKind::Sky]);
    }

    #[test]
    fn background_is_exactly_the_inverse_of_subject() {
        let got = by_name("group");
        let subject = got.iter().find(|s| s.kind == AiMaskKind::Subject).unwrap();
        let background = got.iter().find(|s| s.kind == AiMaskKind::Background).unwrap();
        assert!(subject.weights.iter().zip(&background.weights).all(|(a, b)| 255 - a == *b));
        assert_eq!(subject.score, background.score);
    }

    #[test]
    fn nothing_found_is_no_segments() {
        assert!(by_name("empty").is_empty());
    }

    #[test]
    fn huge_logits_are_not_a_nan() {
        // One query, one pixel-sized class choice: exp would overflow without the clamp's help.
        let (cls, masks) = (vec![9.0f32, -9.0], vec![1e6f32, -1e6, 1e6, -1e6]);
        let m = merge(&cls, 1, 2, &masks, 2, 2, (640, 640), 640);
        assert_eq!(m.soft.len(), 1);
        assert!(m.soft[0].iter().all(|v| v.is_finite() && (0.0..=1.0).contains(v)));
    }
}
