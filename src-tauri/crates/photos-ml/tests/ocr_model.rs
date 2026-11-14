//! `OcrEngine` against PaddleOCR 3 itself (`scripts/ml/export_13.py --golden`) on a
//! public-domain street photo in Prague (`fixtures/ocr/SOURCE.md`): the same lines, in the
//! same order, each corner within 1.5 px, the same text and confidence within 0.02.
//!
//! ```sh
//! LUMEN_MODELS_DIR=<export_13.py --out folder> cargo test --release -p photos-ml --test ocr_model -- --ignored --nocapture
//! ```

use std::path::{Path, PathBuf};

use photos_ml::manifest::ModelEntry;
use photos_ml::ocr::OcrEngine;
use serde::Deserialize;

#[derive(Deserialize)]
struct Golden {
    width: u32,
    height: u32,
    lines: Vec<GoldenLine>,
}

#[derive(Deserialize)]
struct GoldenLine {
    quad: [f32; 8],
    text: String,
    conf: f32,
}

fn entry(dir: &Path, role: &str) -> (PathBuf, ModelEntry) {
    let raw = std::fs::read_to_string(dir.join(format!("{role}.json"))).unwrap_or_else(|e| panic!("{role}.json: {e}"));
    let e: ModelEntry = serde_json::from_str(&raw).unwrap();
    (dir.join(&e.file), e)
}

#[test]
#[ignore = "needs LUMEN_MODELS_DIR"]
fn matches_paddleocr() {
    let dir = PathBuf::from(std::env::var("LUMEN_MODELS_DIR").expect("LUMEN_MODELS_DIR"));
    let (det, rec) = (entry(&dir, "ocr_detection"), entry(&dir, "ocr_recognition"));
    let mut engine = OcrEngine::open((&det.0, &det.1), (&rec.0, &rec.1), 4, None).unwrap();

    let golden: Golden = serde_json::from_str(include_str!("fixtures/ocr/golden.json")).unwrap();
    let img = image::load_from_memory(include_bytes!("fixtures/ocr/signs.jpg")).unwrap().to_rgb8();
    let (w, h) = img.dimensions();
    assert_eq!((w, h), (golden.width, golden.height));
    let rgb = img.into_raw();

    let t = std::time::Instant::now();
    let lines = engine.read(&rgb, w, h, 960).unwrap();
    println!("{} lines in {:?}", lines.len(), t.elapsed());
    for l in &lines {
        println!("  {:.3} {:?} {:?}", l.conf, l.text, l.quad);
    }

    assert_eq!(lines.len(), golden.lines.len(), "number of lines");
    for (i, (got, want)) in lines.iter().zip(&golden.lines).enumerate() {
        for (k, p) in got.quad.iter().enumerate() {
            let (dx, dy) = (p[0] - want.quad[2 * k], p[1] - want.quad[2 * k + 1]);
            assert!(dx.hypot(dy) <= 1.5, "line {i} corner {k}: {p:?} vs {:?}", &want.quad[2 * k..2 * k + 2]);
        }
        assert_eq!(got.text, want.text, "line {i}");
        assert!((got.conf - want.conf).abs() <= 0.02, "line {i}: conf {} vs {}", got.conf, want.conf);
    }
}
