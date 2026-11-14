//! Round 8A: which photos the pet scan may skip, chosen on **public data only** (COCO val2017's
//! pets / other animals / no animals subset, from `scripts/ml/eval_pets.py`), never on Julian's
//! photos. Every photo already has a CLIP embedding; a photo whose embedding is far from both
//! `tag:dog` and `tag:cat` is skipped without running the detector. The positives are the photos
//! where D-FINE itself finds a pet that the scan keeps (score ≥ 0.5, short side ≥ 48 px), so the
//! recall printed is exactly the share of the scan's own finds the prefilter keeps.
//!
//! ```text
//! PETS_EVAL=<eval dir> LUMEN_MODELS_DIR=<models> \
//!   cargo test --release -p photos-ml --test pet_prefilter_public -- --ignored --nocapture
//! ```

use std::path::Path;

use photos_core::embedding::{cosine, softmax};
use photos_ml::clip::ImageEncoder;
use photos_ml::manifest::manifest;
use photos_ml::pets::DetectSpec;
use photos_ml::prototypes;

/// (group, the scan keeps a pet, cosine, softmax share)
type Row = (String, bool, f32, f32);

#[test]
#[ignore = "needs PETS_EVAL and LUMEN_MODELS_DIR"]
fn a_threshold_that_keeps_the_scans_pets() {
    let eval = std::env::var("PETS_EVAL").expect("PETS_EVAL");
    let models = std::env::var("LUMEN_MODELS_DIR").expect("LUMEN_MODELS_DIR");
    let eval = Path::new(&eval);
    let entry = manifest().by_role("image_embedding").expect("an image model");
    let mut enc = ImageEncoder::open(&Path::new(&models).join(&entry.file), entry).unwrap();
    let protos = prototypes();
    let (dog, cat) = (protos.get("tag:dog").unwrap().to_vec(), protos.get("tag:cat").unwrap().to_vec());

    let subset: serde_json::Value = serde_json::from_slice(&std::fs::read(eval.join("coco/subset.json")).unwrap()).unwrap();
    let dets: serde_json::Value = serde_json::from_slice(&std::fs::read(eval.join("detections.json")).unwrap()).unwrap();
    let spec = DetectSpec::MEASURED;
    // (group, has a pet the scan keeps, max cosine, dog+cat share)
    let mut rows: Vec<(String, bool, f32, f32)> = Vec::new();
    for group in ["pets", "others", "none"] {
        for id in subset[group].as_array().unwrap() {
            let id = id.as_i64().unwrap();
            let found = dets["results"][id.to_string()].as_array().is_some_and(|v| {
                v.iter().any(|d| {
                    let b: Vec<f32> = d["box"].as_array().unwrap().iter().map(|x| x.as_f64().unwrap() as f32).collect();
                    d["score"].as_f64().unwrap() as f32 >= spec.score && (b[2] - b[0]).min(b[3] - b[1]) >= spec.min_side
                })
            });
            let img = image::open(eval.join(format!("coco/img/{id:012}.jpg"))).unwrap().to_rgb8();
            let (v, _) = enc.embed(img.as_raw(), img.width(), img.height()).unwrap();
            let cos = cosine(&v, &dog).max(cosine(&v, &cat));
            let share: f32 = softmax(&v, &protos, "tag:", 0.01).iter().filter(|(n, _)| n == "dog" || n == "cat").map(|(_, s)| s).sum();
            rows.push((group.to_string(), found, cos, share));
        }
    }
    let positives = rows.iter().filter(|r| r.1).count();
    println!("{} photos, {positives} where the scan keeps a pet", rows.len());
    let report = |name: &str, score: &dyn Fn(&Row) -> f32, cuts: &[f32]| {
        for &t in cuts {
            let kept_pos = rows.iter().filter(|r| r.1 && score(r) >= t).count();
            let skipped = |g: &str| {
                let all: Vec<_> = rows.iter().filter(|r| r.0 == g && !r.1).collect();
                all.iter().filter(|r| score(r) < t).count() as f32 / all.len().max(1) as f32
            };
            println!(
                "{name} ≥ {t:.3}: recall {:.1} % ({} missed) · skips no-animal {:.0} %, other-animal {:.0} %",
                100.0 * kept_pos as f32 / positives as f32,
                positives - kept_pos,
                100.0 * skipped("none"),
                100.0 * skipped("others"),
            );
        }
    };
    report("cosine", &|r| r.2, &[0.14, 0.16, 0.17, 0.18, 0.19, 0.20, 0.21, 0.22]);
    report("share", &|r| r.3, &[0.0005, 0.001, 0.002, 0.005, 0.01, 0.02, 0.05]);
    let mut missed: Vec<_> = rows.iter().filter(|r| r.1).map(|r| r.2).collect();
    missed.sort_by(|a, b| a.partial_cmp(b).unwrap());
    println!("lowest cosines among the scan's pets: {:?}", &missed[..missed.len().min(10)]);
}
