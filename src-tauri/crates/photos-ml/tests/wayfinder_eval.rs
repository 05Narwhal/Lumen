#![allow(clippy::excessive_precision, clippy::chunks_exact_to_as_chunks)]
//! Speed run and embedding dump for Wayfinder's accuracy check (Round 24 W2 Step 3).
//!
//! Skipped (prints why, passes) unless both variables are set:
//!
//! ```sh
//! LUMEN_WAYFINDER_DIR=C:/lumen-train/wayfinder LUMEN_WAYFINDER_EVAL=<scratch dir> \
//!   cargo test --release -p photos-ml --test wayfinder_eval -- --nocapture
//! ```
//!
//! `<scratch dir>/thumbs.txt` lists one image path per line (made by `scripts/ml/eval_wayfinder.py
//! prep`, from a copy of a library). For each image this embeds three views through the exported
//! image model on DirectML (CPU elsewhere): the CLIP view (short side to 224, centre crop) and two
//! tighter centre crops (75 % and 50 % of the short side), and writes `embeddings.f32`
//! (`n x 3 x 512` little-endian f32) and `speed.json`. `eval_wayfinder.py score` reads both.
//! Nothing is written outside `<scratch dir>`, and the listed images are only read.

use std::{
    io::Write,
    path::{Path, PathBuf},
    time::Instant,
};

use image::{imageops::FilterType, RgbImage};
use ort::{inputs, session::Session, value::Tensor};

const MEAN: [f32; 3] = [0.481_454_66, 0.457_827_5, 0.408_210_73];
const STD: [f32; 3] = [0.268_629_54, 0.261_302_6, 0.275_777_1];
const VIEWS: [f32; 3] = [1.0, 0.75, 0.5];

fn session(path: &Path) -> Session {
    #[cfg(windows)]
    {
        use ort::execution_providers::DirectMLExecutionProvider;
        if let Ok(b) = Session::builder() {
            let built = b
                .with_parallel_execution(false)
                .and_then(|b| b.with_memory_pattern(false))
                .and_then(|b| {
                    b.with_execution_providers([DirectMLExecutionProvider::default()
                        .build()
                        .error_on_failure()])
                })
                .and_then(|b| b.commit_from_file(path));
            if let Ok(s) = built {
                println!("provider: DirectML");
                return s;
            }
        }
    }
    println!("provider: CPU");
    Session::builder()
        .unwrap()
        .with_intra_threads(4)
        .unwrap()
        .commit_from_file(path)
        .unwrap()
}

/// A centre square of `frac` of the short side, resized to 224 (Catmull-Rom is bicubic) and
/// normalised to CLIP's mean and std; `frac == 1.0` is CLIP's own short-side resize and centre crop.
fn view(img: &RgbImage, frac: f32) -> Vec<f32> {
    let (w, h) = img.dimensions();
    let side = (w.min(h) as f32 * frac).round().max(1.0) as u32;
    let (x0, y0) = ((w - side) / 2, (h - side) / 2);
    let square = image::imageops::crop_imm(img, x0, y0, side, side).to_image();
    let r = image::imageops::resize(&square, 224, 224, FilterType::CatmullRom);
    let mut out = vec![0f32; 3 * 224 * 224];
    for (i, p) in r.pixels().enumerate() {
        for c in 0..3 {
            out[c * 224 * 224 + i] = (p[c] as f32 / 255.0 - MEAN[c]) / STD[c];
        }
    }
    out
}

fn run(session: &mut Session, px: Vec<f32>, n: usize) -> Vec<f32> {
    let x = Tensor::from_array(([n, 3, 224, 224], px)).unwrap();
    let out = session.run(inputs!["pixel_values" => x]).unwrap();
    let (shape, v) = out["embedding"].try_extract_tensor::<f32>().unwrap();
    assert_eq!(&shape[..], &[n as i64, 512]);
    v.to_vec()
}

fn percentile(v: &mut [f64], p: f64) -> f64 {
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    v[((v.len() - 1) as f64 * p).round() as usize]
}

#[test]
fn embed_library_copy() {
    let (Ok(models), Ok(scratch)) = (
        std::env::var("LUMEN_WAYFINDER_DIR"),
        std::env::var("LUMEN_WAYFINDER_EVAL"),
    ) else {
        println!("skipped: LUMEN_WAYFINDER_DIR and LUMEN_WAYFINDER_EVAL are not both set");
        return;
    };
    let (models, scratch) = (PathBuf::from(models), PathBuf::from(scratch));
    let list = std::fs::read_to_string(scratch.join("thumbs.txt")).unwrap();
    let paths: Vec<&str> = list.lines().filter(|l| !l.is_empty()).collect();
    let mut s = session(&models.join("wayfinder-image-fp16.onnx"));

    // Warm-up: the first runs pay for shader compilation.
    let warm = image::open(paths[0]).unwrap().to_rgb8();
    for _ in 0..3 {
        run(&mut s, view(&warm, 1.0), 1);
    }

    // Speed 1: one view per photo, decode included, one photo at a time (what Wayfinder's default does).
    let n_speed = paths.len().min(100);
    let t = Instant::now();
    for p in &paths[..n_speed] {
        let img = image::open(p).unwrap().to_rgb8();
        run(&mut s, view(&img, 1.0), 1);
    }
    let one_view = n_speed as f64 / t.elapsed().as_secs_f64();

    // Speed 2 and the dump: three views in one batch.
    let mut file =
        std::io::BufWriter::new(std::fs::File::create(scratch.join("embeddings.f32")).unwrap());
    let (mut decode, mut infer) = (Vec::new(), Vec::new());
    let t = Instant::now();
    for p in &paths {
        let a = Instant::now();
        let img = image::open(p).unwrap().to_rgb8();
        let mut px = Vec::with_capacity(3 * 3 * 224 * 224);
        for f in VIEWS {
            px.extend(view(&img, f));
        }
        decode.push(a.elapsed().as_secs_f64() * 1000.0);
        let a = Instant::now();
        let e = run(&mut s, px, 3);
        infer.push(a.elapsed().as_secs_f64() * 1000.0);
        for v in e {
            file.write_all(&v.to_le_bytes()).unwrap();
        }
    }
    file.flush().unwrap();
    let total = t.elapsed().as_secs_f64();
    let three_views = paths.len() as f64 / total;

    let speed = serde_json::json!({
        "photos": paths.len(),
        "one_view_photos_per_s": one_view,
        "three_views_photos_per_s": three_views,
        "decode_and_preprocess_ms_median": percentile(&mut decode.clone(), 0.5),
        "inference_3_views_ms_median": percentile(&mut infer.clone(), 0.5),
        "inference_3_views_ms_p95": percentile(&mut infer, 0.95),
        "note": "thumbnails of 1024 px (long side), decode + preprocess + inference, one thread, no pipelining",
    });
    std::fs::write(
        scratch.join("speed.json"),
        serde_json::to_string_pretty(&speed).unwrap(),
    )
    .unwrap();
    println!("{speed}");
}
