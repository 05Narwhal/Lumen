//! Round 24 W5 Step 4: Wayfinder's real model files through the app's `AppModels` host (the same
//! code a plugin's `models.run-on-asset` / `models.nearest` reach), against GeoCLIP's own
//! reference values. Everything runs in a temp folder; the model files are hard-linked (else
//! copied) from `LUMEN_WAYFINDER_DIR`, never changed.
//!
//! ```sh
//! LUMEN_WAYFINDER_DIR=C:/lumen-train/wayfinder cargo test --release -p lumen --test wayfinder_real -- --ignored --nocapture
//! ```

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use lumen_lib::plugins::models::{model_file, AppModels};
use photos_plugins::host::{ModelHost, VectorSource};
use photos_plugins::manifest::{parse, Manifest};

const ID: &str = "jl.wayfinder";

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("crates/photos-ml/tests/fixtures/wayfinder")
}

fn manifest() -> Manifest {
    let toml = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("../plugins/first-party/wayfinder/plugin.toml")).unwrap();
    parse(&toml).unwrap()
}

fn cosine(a: &[f32], b: &[f32]) -> f32 {
    let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
    let n = |v: &[f32]| v.iter().map(|x| x * x).sum::<f32>().sqrt();
    dot / (n(a) * n(b))
}

#[test]
#[ignore = "needs LUMEN_WAYFINDER_DIR (the three model files); run with --release"]
fn real_models_match_geoclip_reference() {
    let Some(src) = std::env::var("LUMEN_WAYFINDER_DIR").ok().map(PathBuf::from) else {
        eprintln!("LUMEN_WAYFINDER_DIR not set; skipped");
        return;
    };
    let m = manifest();
    let data = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(data.path().join("models")).unwrap();
    for d in &m.models {
        let from = src.join(d.url.rsplit('/').next().unwrap());
        let to = model_file(data.path(), d);
        if std::fs::hard_link(&from, &to).is_err() {
            std::fs::copy(&from, &to).unwrap();
        }
    }

    let reference: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(fixtures().join("wayfinder_reference.json")).unwrap()).unwrap();
    let images = reference["images"].as_array().unwrap().clone();
    let files: Vec<PathBuf> = images.iter().map(|i| fixtures().join(i["file"].as_str().unwrap())).collect();

    let decls = m.models.clone();
    let dir = data.path().to_path_buf();
    let lookup: lumen_lib::plugins::models::ModelLookup = Arc::new(move |id| (id == ID).then(|| (decls.clone(), dir.clone())));
    let list = files.clone();
    let image: lumen_lib::plugins::models::AssetImage = Arc::new(move |asset| image::open(&list[asset as usize]).map_err(|e| e.to_string()));
    let host = AppModels::new(lookup, image, false);

    let mut worst = 1.0f32;
    let mut total = 0.0;
    for (i, want) in images.iter().enumerate() {
        let t = Instant::now();
        let out = host.run_on_asset(ID, "image", i as u64, "pixel_values").unwrap();
        total += t.elapsed().as_secs_f64();
        let e = &out[0].data;
        assert_eq!(e.len(), 512);
        let w: Vec<f32> = want["embedding"].as_array().unwrap().iter().map(|v| v.as_f64().unwrap() as f32).collect();
        let c = cosine(e, &w);
        worst = worst.min(c);
        // GeoCLIP's own best match is the host's best match, to a few metres.
        let hits = host.nearest(ID, &VectorSource::Model("gallery".into()), e, 5).unwrap();
        let top = &want["top"][0];
        let (lat, lon) = (top["lat"].as_f64().unwrap() as f32, top["lon"].as_f64().unwrap() as f32);
        let got = &hits[0].payload;
        println!("{}: cosine {c:.5}, top-1 ({:.3}, {:.3}) want ({lat:.3}, {lon:.3}), score {:.4}", want["file"], got[0], got[1], hits[0].score);
        assert!((got[0] - lat).abs() < 0.01 && (got[1] - lon).abs() < 0.01, "top-1 differs from the reference");
    }
    println!("worst cosine {worst:.5}, {:.2} s per photo (CPU, first includes session load)", total / images.len() as f64);
    assert!(worst >= 0.99, "worst cosine {worst}");

    // The location encoder takes any (lat, lon) and gives an embedding of the same size.
    let ll = photos_plugins::host::Tensor { name: "latlon".into(), shape: vec![1, 2], data: vec![48.858, 2.294] };
    let out = host.run(ID, "location", vec![ll]).unwrap();
    assert_eq!(out[0].data.len(), 512);
}
