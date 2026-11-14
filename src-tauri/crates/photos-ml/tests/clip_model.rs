//! The image encoder against the real model and the Python export's own answer.
//!
//! ```sh
//! LUMEN_MODELS_DIR=<folder holding the model files> cargo test --release -p photos-ml --test clip_model -- --ignored --nocapture
//! ```

use std::path::Path;

use photos_core::embedding::{cosine, quantize};
use photos_ml::clip::{ImageEncoder, Rgb};
use photos_ml::manifest::manifest;

fn gradient() -> Vec<u8> {
    let mut rgb = vec![0u8; 224 * 224 * 3];
    for y in 0..224 {
        for x in 0..224 {
            rgb[(y * 224 + x) * 3] = x as u8;
            rgb[(y * 224 + x) * 3 + 1] = y as u8;
            rgb[(y * 224 + x) * 3 + 2] = 128;
        }
    }
    rgb
}

fn encoder() -> ImageEncoder {
    let dir = std::env::var("LUMEN_MODELS_DIR").expect("LUMEN_MODELS_DIR");
    let entry = manifest().by_role("image_embedding").unwrap();
    ImageEncoder::open(&Path::new(&dir).join(&entry.file), entry).unwrap()
}

#[test]
#[ignore = "needs the downloaded model"]
fn matches_the_python_reference() {
    let mut enc = encoder();
    let (v, aesthetic) = enc.embed(&gradient(), 224, 224).unwrap();
    let reference: serde_json::Value = serde_json::from_str(include_str!("fixtures/clip_reference.json")).unwrap();
    let r: Vec<f32> = reference["embedding"].as_array().unwrap().iter().map(|x| x.as_f64().unwrap() as f32).collect();
    let c = cosine(&v, &quantize(&r));
    println!("cosine to the Python reference {c:.4}, aesthetic {aesthetic:?}");
    assert!(c > 0.995, "Rust and Python disagree: cosine {c}");
    assert!(aesthetic.is_some());
}

#[test]
#[ignore = "needs the downloaded model"]
fn a_batch_gives_the_same_answers_as_one_at_a_time() {
    let mut enc = encoder();
    let a = gradient();
    let b: Vec<u8> = a.iter().map(|v| 255 - v).collect();
    let one = enc.embed(&b, 224, 224).unwrap().0;
    let t = std::time::Instant::now();
    let photos: Vec<Rgb<'_>> = (0..16).map(|i| Rgb { data: if i % 2 == 0 { &a } else { &b }, width: 224, height: 224 }).collect();
    let batch = enc.embed_batch(&photos).unwrap();
    println!("16 photos in {:?} ({:.1}/s)", t.elapsed(), 16.0 / t.elapsed().as_secs_f32());
    assert!(cosine(&batch[1].0, &one) > 0.999);
}
