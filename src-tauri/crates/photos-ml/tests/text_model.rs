//! LUMEN_MODELS_DIR=<folder> cargo test --release -p photos-ml --test text_model -- --ignored --nocapture
use std::path::Path;

use photos_core::embedding::{cosine, quantize};
use photos_ml::manifest::manifest;
use photos_ml::text::TextEncoder;

#[test]
#[ignore = "needs the downloaded model"]
fn matches_the_python_embeddings() {
    let dir = std::env::var("LUMEN_MODELS_DIR").expect("LUMEN_MODELS_DIR");
    let (m, t) = (manifest().by_role("text_embedding").unwrap(), manifest().by_role("tokenizer").unwrap());
    let mut enc = TextEncoder::open(&Path::new(&dir).join(&m.file), &Path::new(&dir).join(&t.file), m).unwrap();
    let reference: serde_json::Value = serde_json::from_str(include_str!("fixtures/text_reference.json")).unwrap();
    for (phrase, want) in reference["phrases"].as_array().unwrap().iter().zip(reference["embeddings"].as_array().unwrap()) {
        let want: Vec<f32> = want.as_array().unwrap().iter().map(|v| v.as_f64().unwrap() as f32).collect();
        let started = std::time::Instant::now();
        let got = enc.embed(phrase.as_str().unwrap()).unwrap();
        let c = cosine(&got, &quantize(&want));
        println!("{phrase}: cosine {c:.4} in {:?}", started.elapsed());
        assert!(c >= 0.999);
    }
}
