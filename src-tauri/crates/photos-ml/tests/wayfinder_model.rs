#![allow(clippy::excessive_precision, clippy::chunks_exact_to_as_chunks)]
//! Lumen Wayfinder's exported image encoder against GeoCLIP's own PyTorch answer.
//!
//! The three files come from `scripts/ml/export_wayfinder.py`; the test is skipped (it prints why
//! and passes) when `LUMEN_WAYFINDER_DIR` is not set or the files are not there.
//!
//! ```sh
//! LUMEN_WAYFINDER_DIR=C:/lumen-train/wayfinder cargo test --release -p photos-ml --test wayfinder_model -- --nocapture
//! ```
//!
//! For each of five CC0 photos the ONNX embedding (CPU, and DirectML on Windows) must have cosine
//! ≥ 0.99 to PyTorch's, and the gallery's top-1 place must be the one PyTorch picks. The Rust
//! preprocessing here is what the plugin host uses: short side to 224, bicubic, centre crop, CLIP's
//! mean and std.

use std::path::{Path, PathBuf};

use image::imageops::FilterType;
use ort::{inputs, session::Session, value::Tensor};

const MEAN: [f32; 3] = [0.481_454_66, 0.457_827_5, 0.408_210_73];
const STD: [f32; 3] = [0.268_629_54, 0.261_302_58, 0.275_777_11];

fn dir() -> Option<PathBuf> {
    let d = PathBuf::from(std::env::var("LUMEN_WAYFINDER_DIR").ok()?);
    d.join("wayfinder-image-fp16.onnx").exists().then_some(d)
}

fn f16_to_f32(h: u16) -> f32 {
    let (sign, exp, frac) = ((h >> 15) as u32, ((h >> 10) & 0x1f) as u32, (h & 0x3ff) as u32);
    let bits = match (exp, frac) {
        (0, 0) => sign << 31,
        (0, f) => {
            // Subnormal: renormalise.
            let shift = f.leading_zeros() - 21;
            (sign << 31) | ((127 - 15 + 1 - shift) << 23) | (((f << shift) & 0x3ff) << 13)
        }
        (0x1f, f) => (sign << 31) | (0xff << 23) | (f << 13),
        (e, f) => (sign << 31) | ((e + 127 - 15) << 23) | (f << 13),
    };
    f32::from_bits(bits)
}

struct Gallery {
    rows: usize,
    dim: usize,
    emb: Vec<u16>,
    gps: Vec<f32>,
}

fn read_gallery(path: &Path) -> Gallery {
    let b = std::fs::read(path).unwrap();
    assert_eq!(&b[..4], b"LWVG");
    let u = |i: usize| u32::from_le_bytes(b[4 + 4 * i..8 + 4 * i].try_into().unwrap());
    let (version, rows, dim, dtype) = (u(0), u(1) as usize, u(2) as usize, u(3));
    assert_eq!((version, dtype), (1, 1));
    let m = 20 + rows * dim * 2;
    assert_eq!(b.len(), m + rows * 8);
    let emb = b[20..m].chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
    let gps = b[m..].chunks_exact(4).map(|c| f32::from_le_bytes(c.try_into().unwrap())).collect();
    Gallery { rows, dim, emb, gps }
}

fn top1(g: &Gallery, e: &[f32]) -> (f32, f32, f32) {
    let mut best = (f32::MIN, 0usize);
    for r in 0..g.rows {
        let row = &g.emb[r * g.dim..(r + 1) * g.dim];
        let s: f32 = row.iter().zip(e).map(|(&h, &x)| f16_to_f32(h) * x).sum();
        if s > best.0 {
            best = (s, r);
        }
    }
    (g.gps[best.1 * 2], g.gps[best.1 * 2 + 1], best.0)
}

/// CLIP's preprocessing: short side to 224 (Catmull-Rom, which is bicubic), centre crop, CHW.
fn pixel_values(path: &Path) -> Vec<f32> {
    let img = image::open(path).unwrap().to_rgb8();
    let (w, h) = img.dimensions();
    let scale = 224.0 / w.min(h) as f32;
    let (nw, nh) = (((w as f32 * scale).round() as u32).max(224), ((h as f32 * scale).round() as u32).max(224));
    let r = image::imageops::resize(&img, nw, nh, FilterType::CatmullRom);
    let (x0, y0) = ((nw - 224) / 2, (nh - 224) / 2);
    let mut out = vec![0f32; 3 * 224 * 224];
    for y in 0..224u32 {
        for x in 0..224u32 {
            let p = r.get_pixel(x0 + x, y0 + y);
            for c in 0..3 {
                out[c * 224 * 224 + (y * 224 + x) as usize] = (p[c] as f32 / 255.0 - MEAN[c]) / STD[c];
            }
        }
    }
    out
}

fn embed(session: &mut Session, px: Vec<f32>) -> Vec<f32> {
    let x = Tensor::from_array(([1usize, 3, 224, 224], px)).unwrap();
    let out = session.run(inputs!["pixel_values" => x]).unwrap();
    let (shape, v) = out["embedding"].try_extract_tensor::<f32>().unwrap();
    assert_eq!(&shape[..], &[1, 512]);
    v.to_vec()
}

fn cpu(path: &Path) -> Session {
    Session::builder().unwrap().with_intra_threads(4).unwrap().commit_from_file(path).unwrap()
}

#[cfg(windows)]
fn gpu(path: &Path) -> Option<Session> {
    use ort::execution_providers::DirectMLExecutionProvider;
    Session::builder()
        .ok()?
        .with_parallel_execution(false)
        .ok()?
        .with_memory_pattern(false)
        .ok()?
        .with_execution_providers([DirectMLExecutionProvider::default().build().error_on_failure()])
        .ok()?
        .commit_from_file(path)
        .ok()
}

#[cfg(not(windows))]
fn gpu(_: &Path) -> Option<Session> {
    None
}

fn check(label: &str, session: &mut Session, gallery: &Gallery, fixtures: &Path) {
    let reference: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(fixtures.join("wayfinder_reference.json")).unwrap()).unwrap();
    let images = reference["images"].as_array().unwrap();
    assert_eq!(images.len(), 5);
    for item in images {
        let file = item["file"].as_str().unwrap();
        let want: Vec<f32> = item["embedding"].as_array().unwrap().iter().map(|v| v.as_f64().unwrap() as f32).collect();
        let got = embed(session, pixel_values(&fixtures.join(file)));
        let cos: f32 = want.iter().zip(&got).map(|(a, b)| a * b).sum();
        let (lat, lon, score) = top1(gallery, &got);
        let t = &item["top"][0];
        let (wlat, wlon) = (t["lat"].as_f64().unwrap() as f32, t["lon"].as_f64().unwrap() as f32);
        println!("{label} {file}: cosine {cos:.4}, top-1 ({lat:.3}, {lon:.3}) score {score:.3} vs PyTorch ({wlat:.3}, {wlon:.3})");
        assert!(cos >= 0.99, "{label} {file}: cosine {cos}");
        assert!((lat - wlat).abs() < 1e-3 && (lon - wlon).abs() < 1e-3, "{label} {file}: top-1 differs");
    }
}

#[test]
fn onnx_matches_pytorch_on_cpu_and_directml() {
    let Some(dir) = dir() else {
        println!("skipped: LUMEN_WAYFINDER_DIR does not hold wayfinder-image-fp16.onnx");
        return;
    };
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/wayfinder");
    let gallery = read_gallery(&dir.join("wayfinder-gallery.bin"));
    assert_eq!((gallery.rows, gallery.dim), (100_000, 512));
    let model = dir.join("wayfinder-image-fp16.onnx");
    check("cpu", &mut cpu(&model), &gallery, &fixtures);
    match gpu(&model) {
        Some(mut s) => check("directml", &mut s, &gallery, &fixtures),
        None => println!("directml: not available here, CPU only"),
    }
}
