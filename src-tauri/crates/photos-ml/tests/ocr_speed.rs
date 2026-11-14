//! The text scan's speed budget (Round 13 spec §2, plan Task 10): on two CPU threads, as the
//! scan runs, the detector ≤ 60 ms and a photo without text ≤ 80 ms, so 5,000 photos read in
//! under 10 minutes. Photos are decoded and cut to 1536 px before timing (the scan reads
//! cached previews); only `OcrEngine::read` is timed. *Detection* is a flat grey photo: no
//! quads, so `read` returns right after the detector and its post-processing.
//!
//! ```sh
//! LUMEN_MODELS_DIR=<folder with ocr_detection.json / ocr_recognition.json and the models> \
//!   [LUMEN_OCR_FOLDER=C:/lumen-train/eval-library] \
//!   cargo test --release -p photos-ml --test ocr_speed -- --ignored --nocapture
//! ```

use std::path::{Path, PathBuf};
use std::time::Instant;

use photos_ml::manifest::ModelEntry;
use photos_ml::ocr::OcrEngine;

const SIDE: u32 = 1536;
const WARM_UPS: usize = 3;

/// The scan's settings, `LUMEN_OCR_THREADS` (default 4) and `LUMEN_OCR_DET_SIDE` (default 736, the app's), to compare options.
fn threads() -> usize {
    std::env::var("LUMEN_OCR_THREADS").ok().and_then(|v| v.parse().ok()).unwrap_or(4)
}

fn det_side() -> u32 {
    std::env::var("LUMEN_OCR_DET_SIDE").ok().and_then(|v| v.parse().ok()).unwrap_or(736)
}

fn entry(dir: &Path, role: &str) -> (PathBuf, ModelEntry) {
    let e: ModelEntry = serde_json::from_str(&std::fs::read_to_string(dir.join(format!("{role}.json"))).unwrap()).unwrap();
    (dir.join(&e.file), e)
}

fn median(v: &mut [f64]) -> f64 {
    v.sort_by(f64::total_cmp);
    if v.is_empty() { f64::NAN } else { v[v.len() / 2] }
}

/// Every JPEG / PNG in the folder (not RAW: the scan reads their previews), at most 1536 px.
fn photos(dir: &Path) -> Vec<(String, Vec<u8>, u32, u32)> {
    let mut out = Vec::new();
    let mut files: Vec<PathBuf> = walk(dir);
    files.sort();
    for f in files {
        let ext = f.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
        if !matches!(ext.as_str(), "jpg" | "jpeg" | "png") {
            continue;
        }
        let Ok(img) = image::open(&f) else { continue };
        let img = if img.width().max(img.height()) > SIDE { img.resize(SIDE, SIDE, image::imageops::FilterType::Triangle) } else { img };
        let rgb = img.to_rgb8();
        let (w, h) = rgb.dimensions();
        out.push((f.file_name().unwrap().to_string_lossy().into_owned(), rgb.into_raw(), w, h));
    }
    out
}

fn walk(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for e in std::fs::read_dir(dir).unwrap().flatten() {
        let p = e.path();
        if p.is_dir() { out.extend(walk(&p)) } else { out.push(p) }
    }
    out
}

/// Where detection's time goes, per photo at [`det_side`] on [`threads`]: the resize and tensor,
/// the detector itself, and the box post-processing (median ms of each, over every photo).
fn breakdown(det_path: &Path, det: &ModelEntry, photos: &[(String, Vec<u8>, u32, u32)]) {
    use ort::{inputs, session::Session, value::Tensor};
    use photos_ml::ocr::detect::{boxes_from_map, det_tensor};
    let spec = det.ocr_det.clone().unwrap();
    let mut session = Session::builder().unwrap().with_intra_threads(threads()).unwrap().with_intra_op_spinning(false).unwrap().commit_from_file(det_path).unwrap();
    let (mut pre, mut model, mut post) = (Vec::new(), Vec::new(), Vec::new());
    for (i, (_, rgb, w, h)) in photos.iter().enumerate() {
        let t = Instant::now();
        let (x, tw, th) = det_tensor(rgb, *w, *h, det_side());
        let t1 = Instant::now();
        let out = session.run(inputs!["x" => Tensor::from_array(([1usize, 3, th as usize, tw as usize], x)).unwrap()]).unwrap();
        let (_, prob) = out[0].try_extract_tensor::<f32>().unwrap();
        let prob = prob.to_vec();
        let t2 = Instant::now();
        let boxes = boxes_from_map(&prob, tw, th, *w, *h, &spec);
        let t3 = Instant::now();
        std::hint::black_box(boxes);
        if i >= WARM_UPS {
            pre.push((t1 - t).as_secs_f64() * 1000.0);
            model.push((t2 - t1).as_secs_f64() * 1000.0);
            post.push((t3 - t2).as_secs_f64() * 1000.0);
        }
    }
    println!("detection breakdown (median ms): resize + tensor {:.1} · detector {:.1} · boxes {:.1}", median(&mut pre), median(&mut model), median(&mut post));
}

#[test]
#[ignore = "needs LUMEN_MODELS_DIR and a folder of photos"]
fn reads_within_budget() {
    let models = PathBuf::from(std::env::var("LUMEN_MODELS_DIR").expect("LUMEN_MODELS_DIR"));
    let folder = PathBuf::from(std::env::var("LUMEN_OCR_FOLDER").unwrap_or_else(|_| "C:/lumen-train/eval-library".into()));
    let (det_path, det) = entry(&models, "ocr_detection");
    let (rec_path, rec) = entry(&models, "ocr_recognition");
    let mut engine = OcrEngine::open((&det_path, &det), (&rec_path, &rec), threads(), None).unwrap();

    let grey = vec![128u8; (SIDE * 1024 * 3) as usize];
    for _ in 0..WARM_UPS {
        engine.read(&grey, SIDE, 1024, det_side()).unwrap();
    }
    let mut detection: Vec<f64> = (0..7)
        .map(|_| {
            let t = Instant::now();
            assert!(engine.read(&grey, SIDE, 1024, det_side()).unwrap().is_empty());
            t.elapsed().as_secs_f64() * 1000.0
        })
        .collect();

    let photos = photos(&folder);
    assert!(!photos.is_empty(), "no JPEG or PNG in {}", folder.display());
    for (_, rgb, w, h) in photos.iter().take(WARM_UPS) {
        engine.read(rgb, *w, *h, det_side()).unwrap();
    }
    let (mut all, mut no_text, mut lines) = (Vec::new(), Vec::new(), 0usize);
    for (name, rgb, w, h) in &photos {
        let t = Instant::now();
        let found = engine.read(rgb, *w, *h, det_side()).unwrap();
        let ms = t.elapsed().as_secs_f64() * 1000.0;
        lines += found.len();
        all.push(ms);
        if found.is_empty() {
            no_text.push(ms);
        } else if found.len() >= 10 {
            println!("  {name}: {} lines in {ms:.0} ms", found.len());
        }
    }
    breakdown(&det_path, &det, &photos);
    let (det_ms, none_ms, all_ms) = (median(&mut detection), median(&mut no_text), median(&mut all));
    let mean_ms = all.iter().sum::<f64>() / all.len() as f64;
    let five_k_min = mean_ms * 5000.0 / 60_000.0;
    println!(
        "{} photos ({} without text, {lines} lines) · detection {det_ms:.1} ms · photo without text {none_ms:.1} ms · per photo median {all_ms:.1} ms, mean {mean_ms:.1} ms · 5,000 photos ≈ {five_k_min:.1} min",
        photos.len(),
        no_text.len()
    );
    assert!(det_ms <= 60.0, "detection {det_ms:.1} ms over the 60 ms budget");
    if !no_text.is_empty() {
        assert!(none_ms <= 80.0, "a photo without text {none_ms:.1} ms over the 80 ms budget");
    }
    assert!(five_k_min < 10.0, "5,000 photos ≈ {five_k_min:.1} min, over 10");
}
