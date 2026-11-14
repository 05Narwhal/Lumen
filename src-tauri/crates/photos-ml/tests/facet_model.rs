//! `facet::detect` against `scripts/ml/eval_facet.py` on three public photos
//! (`fixtures/facet/SOURCE.md`, written by `scripts/ml/golden_facet.py`): the same masks in the same
//! order, scores within 0.02, each mask's IoU at least 0.99.
//!
//! ```sh
//! LUMEN_MODELS_DIR=<folder with facet-eomt-s-coco-fp16.onnx> cargo test --release -p photos-ml --test facet_model -- --ignored --nocapture
//! ```

use std::path::PathBuf;
use std::sync::atomic::AtomicBool;

use photos_core::ai_masks::AiMaskKind;
use photos_ml::facet::{detect, FacetModel};
use photos_ml::manifest::manifest;
use serde::Deserialize;

#[derive(Deserialize)]
struct Photo {
    file: String,
    rows: Vec<Row>,
}

#[derive(Deserialize)]
struct Row {
    kind: AiMaskKind,
    instance: u32,
    score: f32,
    width: u32,
    height: u32,
    /// Run lengths of weights ≥ 128, starting with a run of 0s.
    rle: Vec<usize>,
}

fn unrun(rle: &[usize]) -> Vec<bool> {
    let mut out = Vec::with_capacity(rle.iter().sum());
    for (i, &n) in rle.iter().enumerate() {
        out.extend(std::iter::repeat_n(i % 2 == 1, n));
    }
    out
}

fn iou(a: &[bool], b: &[bool]) -> f64 {
    let both = a.iter().zip(b).filter(|(x, y)| **x && **y).count();
    let any = a.iter().zip(b).filter(|(x, y)| **x || **y).count();
    if any == 0 {
        1.0
    } else {
        both as f64 / any as f64
    }
}

fn fixture(name: &str) -> &'static [u8] {
    match name {
        "crew.jpg" => include_bytes!("fixtures/facet/crew.jpg"),
        "valley.jpg" => include_bytes!("fixtures/facet/valley.jpg"),
        "car.jpg" => include_bytes!("fixtures/facet/car.jpg"),
        other => panic!("no fixture {other}"),
    }
}

fn open(gpu: bool) -> FacetModel {
    let dir = PathBuf::from(std::env::var("LUMEN_MODELS_DIR").expect("LUMEN_MODELS_DIR"));
    let entry = manifest().by_role("segmentation").expect("Facet is in the manifest");
    FacetModel::open(&dir.join(&entry.file), entry, gpu).unwrap()
}

#[test]
#[ignore = "needs LUMEN_MODELS_DIR"]
fn matches_eval_facet() {
    let golden: Vec<Photo> = serde_json::from_str(include_str!("fixtures/facet/golden.json")).unwrap();
    let mut model = open(false);
    assert_eq!(model.ep(), "cpu");
    let cancel = AtomicBool::new(false);
    for photo in &golden {
        let img = image::load_from_memory(fixture(&photo.file)).unwrap().to_rgb8();
        let t = std::time::Instant::now();
        let got = detect(&mut model, &img, &cancel).unwrap();
        println!("{}: {} masks in {:?}", photo.file, got.len(), t.elapsed());
        let kinds: Vec<_> = got.iter().map(|s| (s.kind, s.instance)).collect();
        assert_eq!(kinds, photo.rows.iter().map(|r| (r.kind, r.instance)).collect::<Vec<_>>(), "{}", photo.file);
        for (s, r) in got.iter().zip(&photo.rows) {
            assert_eq!((s.width, s.height), (r.width, r.height), "{} {:?}", photo.file, s.kind);
            assert!(s.width.max(s.height) <= 160);
            let have: Vec<bool> = s.weights.iter().map(|w| *w >= 128).collect();
            let v = iou(&have, &unrun(&r.rle));
            println!("  {:?} {}: score {:.4} vs {:.4}, IoU {v:.4}", s.kind, s.instance, s.score, r.score);
            assert!((s.score - r.score).abs() <= 0.02, "{} {:?} {}", photo.file, s.kind, s.instance);
            assert!(v >= 0.99, "{} {:?} {}: IoU {v}", photo.file, s.kind, s.instance);
        }
    }
}

#[test]
#[ignore = "needs LUMEN_MODELS_DIR"]
fn a_cancelled_run_is_an_error_not_a_result() {
    let mut model = open(false);
    let img = image::RgbImage::new(64, 48);
    assert!(detect(&mut model, &img, &AtomicBool::new(true)).is_err());
}
