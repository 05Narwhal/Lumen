//! Phase 8d: the denoise model on the Apollo 11 fixture with seeded noise, on the CPU and on
//! the GPU (DirectML): it must gain ≥ 3 dB, the two must agree, and no tile seam may show.
//!
//! ```sh
//! LUMEN_MODELS_DIR=<folder holding the model files> cargo test --release -p photos-ml --test denoise_model -- --ignored --nocapture
//! ```

use std::path::Path;
use std::time::Instant;

use photos_ml::denoise::{tiles, Denoiser};
use photos_ml::manifest::manifest;

/// Seeded Gaussian noise (a 64-bit LCG through Box–Muller), so every run sees the same image.
fn noise(n: usize, sigma: f32) -> Vec<f32> {
    let mut s = 0x2545_F491_4F6C_DD1Du64;
    let mut next = || {
        s = s.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
        ((s >> 11) as f64 / (1u64 << 53) as f64).max(1e-12)
    };
    (0..n).map(|_| ((-2.0 * next().ln()).sqrt() * (2.0 * std::f64::consts::PI * next()).cos()) as f32 * sigma).collect()
}

fn psnr(a: &[f32], b: &[f32]) -> f32 {
    let mse = a.iter().zip(b).map(|(x, y)| (x - y).powi(2)).sum::<f32>() / a.len() as f32;
    10.0 * (1.0 / mse).log10()
}

#[test]
#[ignore = "needs the downloaded models"]
fn denoises_the_fixture_on_cpu_and_gpu_without_seams() {
    let dir = std::env::var("LUMEN_MODELS_DIR").expect("LUMEN_MODELS_DIR");
    let mut entry = manifest().by_role("denoise").unwrap().clone();
    // Small tiles, so the fixture has seams to look at.
    let spec = entry.denoise.as_mut().unwrap();
    spec.tile = 128;
    spec.overlap = 16;
    let (tile, overlap) = (spec.tile, spec.overlap);
    let path = Path::new(&dir).join(&entry.file);

    let img = image::load_from_memory(include_bytes!("fixtures/apollo11.jpg")).unwrap().to_rgb8();
    let (w, h) = img.dimensions();
    let clean: Vec<f32> = img.as_raw().iter().map(|&v| v as f32 / 255.0).collect();
    let noisy: Vec<f32> = clean.iter().zip(noise(clean.len(), 10.0 / 255.0)).map(|(c, n)| (c + n).clamp(0.0, 1.0)).collect();

    let mut outs = Vec::new();
    let mut gains = Vec::new();
    for gpu in [false, true] {
        let mut d = Denoiser::open(&path, &entry, gpu).unwrap();
        let t = Instant::now();
        let n = tiles(w, h, tile, overlap).len();
        let out = d.run(&noisy, w, h, &mut |_, _| true).unwrap();
        let took = t.elapsed();
        println!(
            "{} ({}): {w}×{h}, {n} tiles of {tile}, {:.0} ms a tile; PSNR {:.2} → {:.2} dB",
            if gpu { "GPU asked" } else { "CPU" },
            if d.on_gpu() { "DirectML" } else { "CPU" },
            took.as_secs_f64() * 1000.0 / n as f64,
            psnr(&noisy, &clean),
            psnr(&out, &clean)
        );
        assert!(psnr(&out, &clean) - psnr(&noisy, &clean) >= 3.0);
        gains.push(psnr(&out, &clean));
        outs.push(out);
    }

    // The CPU and the GPU agree: DirectML computes in fp16 where the CPU widens to fp32, so
    // they differ by about 3/255 at the 99th percentile (measured) — invisible, and the GPU's
    // result is within 0.5 dB of the CPU's.
    let mut diffs: Vec<f32> = outs[0].iter().zip(&outs[1]).map(|(a, b)| (a - b).abs()).collect();
    diffs.sort_by(f32::total_cmp);
    let p99 = diffs[diffs.len() * 99 / 100];
    println!("CPU vs GPU: 99th percentile difference {:.5}", p99);
    assert!(p99 <= 4.0 / 255.0);
    assert!((gains[0] - gains[1]).abs() <= 0.5, "{gains:?}");

    // No seam: against the same network run on the whole image as one tile, the tiled result
    // differs no more along the tile boundaries than elsewhere. (Comparing neighbour steps
    // mistakes the photo's own edges for seams.)
    let mut whole_entry = entry.clone();
    let ws = whole_entry.denoise.as_mut().unwrap();
    ws.tile = w.max(h).div_ceil(16) * 16;
    ws.overlap = 0;
    let whole = Denoiser::open(&path, &whole_entry, false).unwrap().run(&noisy, w, h, &mut |_, _| true).unwrap();
    let diff = |x: u32, y: u32| (0..3).map(|c| (outs[0][((y * w + x) * 3) as usize + c] - whole[((y * w + x) * 3) as usize + c]).abs()).sum::<f32>() / 3.0;
    let ts = tiles(w, h, tile, overlap);
    let mut xs: Vec<u32> = ts.iter().map(|t| t.x).filter(|&x| x > 0).collect();
    xs.sort_unstable();
    xs.dedup();
    let near = |x: u32| xs.iter().any(|&s| x + 2 >= s && x <= s + overlap + 2);
    let (mut at_seams, mut n_seams, mut away, mut n_away) = (0f32, 0u32, 0f32, 0u32);
    for y in 0..h {
        for x in 0..w {
            if near(x) {
                at_seams += diff(x, y);
                n_seams += 1;
            } else {
                away += diff(x, y);
                n_away += 1;
            }
        }
    }
    let (at_seams, away) = (at_seams / n_seams as f32, away / n_away as f32);
    println!("tiled vs whole: {at_seams:.5} along seams, {away:.5} elsewhere");
    // Small tiles see less context than the whole image, so the two differ a little
    // everywhere (≈ 1.4/255 at 128 px); a seam would show as more of it along the boundaries.
    assert!(at_seams <= 1.5 * away + 1e-4, "seams stand out: {at_seams:.5} against {away:.5}");
}

/// A whole photo at the shipped tiling, timed on the GPU and on the CPU:
/// `LUMEN_DENOISE_PHOTO=<a JPEG copy>` (Phase 8d measured a 24 MP one).
#[test]
#[ignore = "needs the models and a large photo"]
fn times_a_full_photo() {
    let dir = std::env::var("LUMEN_MODELS_DIR").expect("LUMEN_MODELS_DIR");
    let photo = std::env::var("LUMEN_DENOISE_PHOTO").expect("LUMEN_DENOISE_PHOTO");
    let entry = manifest().by_role("denoise").unwrap();
    let spec = entry.denoise.as_ref().unwrap();
    let img = image::open(&photo).unwrap().to_rgb8();
    let (w, h) = img.dimensions();
    let rgb: Vec<f32> = img.as_raw().iter().map(|&v| v as f32 / 255.0).collect();
    drop(img);
    let n = tiles(w, h, spec.tile, spec.overlap).len();
    for gpu in [true, false] {
        let t = Instant::now();
        let mut d = Denoiser::open(&Path::new(&dir).join(&entry.file), entry, gpu).unwrap();
        let out = d.run(&rgb, w, h, &mut |_, _| true).unwrap();
        println!(
            "{w}×{h} ({:.1} MP, {n} tiles) on {}: {:.1} s",
            (w * h) as f64 / 1e6,
            if d.on_gpu() { "DirectML" } else { "the CPU" },
            t.elapsed().as_secs_f64()
        );
        assert_eq!(out.len(), rgb.len());
    }
}
