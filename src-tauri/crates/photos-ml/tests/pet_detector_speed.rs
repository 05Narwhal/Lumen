//! Round 8A: how fast D-FINE finds pets with several CPU sessions side by side, on public
//! COCO photos (the `eval_pets.py` subset), and that every layout finds the same animals.
//!
//! Measured on Julian's 24-thread machine (96 photos): one session × 2 threads (the scan before
//! Round 8A) 3.8 photos/s; 3 sessions × 4 threads 8.7 (2.3×), the best; one-thread sessions
//! peak at 8 (6.4) and slow down beyond. DirectML ran 9× faster but missed clear pets on 43 of
//! 200 photos at every optimisation level, so the scan stays on the CPU. The batch axis the
//! export declares fails inside the decoder (`GatherElements`) for more than one photo.
//!
//! ```text
//! PETS_EVAL=<eval dir> LUMEN_MODELS_DIR=<models> [LUMEN_PET_PHOTOS=96] [LUMEN_PET_CONFIGS=3x4,8x1] \
//!   cargo test --release -p photos-ml --test pet_detector_speed -- --ignored --nocapture
//! ```

use std::path::Path;
use std::sync::Mutex;
use std::time::Instant;

use photos_ml::manifest::manifest;
use photos_ml::pets::{PetBox, PetDetector};

fn photos(eval: &Path, n: usize) -> Vec<(Vec<u8>, u32, u32)> {
    let subset: serde_json::Value = serde_json::from_slice(&std::fs::read(eval.join("coco/subset.json")).unwrap()).unwrap();
    let ids: Vec<i64> = ["pets", "none", "others"].iter().flat_map(|g| subset[*g].as_array().unwrap().iter().map(|v| v.as_i64().unwrap())).collect();
    ids.iter()
        .step_by((ids.len() / n).max(1))
        .take(n)
        .map(|id| {
            let img = image::open(eval.join(format!("coco/img/{id:012}.jpg"))).unwrap().to_rgb8();
            let (w, h) = img.dimensions();
            (img.into_raw(), w, h)
        })
        .collect()
}

fn same(a: &[PetBox], b: &[PetBox]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| x.species == y.species && x.bbox.iter().zip(y.bbox).all(|(p, q)| (p - q).abs() <= 1.5))
}

/// Stands in for `photos_jobs::background_priority`: proves the hook runs ONNX Runtime's threads.
fn lowered() {}

#[test]
#[ignore = "needs PETS_EVAL and LUMEN_MODELS_DIR"]
fn detector_layouts() {
    let eval = std::env::var("PETS_EVAL").expect("PETS_EVAL");
    let models = std::env::var("LUMEN_MODELS_DIR").expect("LUMEN_MODELS_DIR");
    let n: usize = std::env::var("LUMEN_PET_PHOTOS").ok().and_then(|v| v.parse().ok()).unwrap_or(96);
    let entry = manifest().by_role("pet_detection").unwrap();
    let path = Path::new(&models).join(&entry.file);
    let photos = photos(Path::new(&eval), n);

    let mut det = PetDetector::open(&path, entry).unwrap();
    let _ = det.detect(&photos[0].0, photos[0].1, photos[0].2);
    let t = Instant::now();
    let base: Vec<Vec<PetBox>> = photos.iter().map(|(p, w, h)| det.detect(p, *w, *h).unwrap()).collect();
    let base_rate = photos.len() as f64 / t.elapsed().as_secs_f64();
    println!("{} photos · one session × 2 threads: {base_rate:.1} photos/s", photos.len());

    let configs: Vec<(usize, usize)> = std::env::var("LUMEN_PET_CONFIGS")
        .unwrap_or_else(|_| "3x4,8x1".into())
        .split(',')
        .filter_map(|c| c.split_once('x'))
        .map(|(a, b)| (a.parse().unwrap(), b.parse().unwrap()))
        .collect();
    for (sessions, threads) in configs {
        let queue = Mutex::new((0..photos.len()).collect::<Vec<_>>());
        let found = Mutex::new(vec![Vec::new(); photos.len()]);
        let t = Instant::now();
        std::thread::scope(|s| {
            for _ in 0..sessions {
                s.spawn(|| {
                    let mut det = PetDetector::open_with(&path, entry, threads, Some(lowered)).unwrap();
                    loop {
                        let Some(i) = queue.lock().unwrap().pop() else { break };
                        let (p, w, h) = &photos[i];
                        let boxes = det.detect(p, *w, *h).unwrap();
                        found.lock().unwrap()[i] = boxes;
                    }
                });
            }
        });
        let rate = photos.len() as f64 / t.elapsed().as_secs_f64();
        let found = found.into_inner().unwrap();
        let agree = found.iter().zip(&base).all(|(a, b)| same(a, b));
        println!("{sessions} sessions × {threads} threads: {rate:.1} photos/s ({:.1}×), same finds: {agree}", rate / base_rate);
        assert!(agree);
    }
}
