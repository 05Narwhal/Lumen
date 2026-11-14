//! YuNet and SFace against OpenCV's own answers on a public-domain photograph (the Apollo
//! 11 crew, NASA): the same faces, in the same places, described the same way.
//!
//! ```sh
//! LUMEN_MODELS_DIR=<folder holding the model files> cargo test --release -p photos-ml --test faces_model -- --ignored --nocapture
//! ```

use std::path::Path;

use photos_core::embedding::{cosine, quantize};
use photos_ml::faces::{align_crop, FaceDetector, FaceEmbedder};
use photos_ml::manifest::manifest;

fn models() -> (FaceDetector, FaceEmbedder) {
    let dir = std::env::var("LUMEN_MODELS_DIR").expect("LUMEN_MODELS_DIR");
    let det = manifest().by_role("face_detection").unwrap();
    let emb = manifest().by_role("face_embedding").unwrap();
    (
        FaceDetector::open(&Path::new(&dir).join(&det.file), det).unwrap(),
        FaceEmbedder::open(&Path::new(&dir).join(&emb.file), emb).unwrap(),
    )
}

#[test]
#[ignore = "needs the downloaded models"]
fn finds_and_describes_the_faces_opencv_does() {
    let img = image::load_from_memory(include_bytes!("fixtures/apollo11.jpg")).unwrap().to_rgb8();
    let (w, h) = img.dimensions();
    let rgb = img.into_raw();
    let reference: serde_json::Value = serde_json::from_str(include_str!("fixtures/faces_reference.json")).unwrap();
    let want = reference["faces"].as_array().unwrap();

    let (mut det, mut emb) = models();
    let t = std::time::Instant::now();
    let faces = det.detect(&rgb, w, h).unwrap();
    println!("{} faces in {:?}", faces.len(), t.elapsed());
    assert_eq!(faces.len(), want.len());

    let mut vectors = Vec::new();
    for r in want {
        let rb: Vec<f32> = r["box"].as_array().unwrap().iter().map(|v| v.as_f64().unwrap() as f32).collect();
        // The face we found nearest this reference face.
        let f = faces
            .iter()
            .min_by(|a, b| ((a.bbox[0] - rb[0]).abs() + (a.bbox[1] - rb[1]).abs()).total_cmp(&((b.bbox[0] - rb[0]).abs() + (b.bbox[1] - rb[1]).abs())))
            .unwrap();
        let off = f.bbox.iter().zip(&rb).map(|(a, b)| (a - b).abs()).fold(0.0, f32::max);
        let score = r["score"].as_f64().unwrap() as f32;
        let v = emb.embed(&align_crop(&rgb, w, h, &f.landmarks)).unwrap();
        let rf: Vec<f32> = r["feature"].as_array().unwrap().iter().map(|v| v.as_f64().unwrap() as f32).collect();
        let c = cosine(&v, &quantize(&rf));
        println!("box off by {off:.2} px, score {:.3} vs {score:.3}, embedding cosine {c:.4}", f.score);
        assert!(off <= 3.0, "box {:?} vs {rb:?}", f.bbox);
        assert!((f.score - score).abs() <= 0.02);
        assert!(c >= 0.98, "alignment differs from OpenCV's: cosine {c}");
        vectors.push(v);
    }
    for i in 0..vectors.len() {
        for j in i + 1..vectors.len() {
            assert!(cosine(&vectors[i], &vectors[j]) < 0.363, "three different people");
        }
    }
}
