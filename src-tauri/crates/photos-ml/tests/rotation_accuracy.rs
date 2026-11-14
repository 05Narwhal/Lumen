//! The gate for rotation suggestions (round 3, Task 8): can CLIP tell which way up a photo
//! is? Each listed thumbnail (already upright) is fed in all four quarter turns; a scorer
//! must name the turn that makes it upright again.
//!
//! ```sh
//! LUMEN_MODELS_DIR=<models> LUMEN_ROTATION_LIST=<file of thumbnail paths> \
//!   cargo test --release -p photos-ml --test rotation_accuracy -- --ignored --nocapture
//! ```
//! Thumbnails are only read.
//!
//! **Measured 2026-09-23 on Julian's library (the gate failed, so rotation suggestions were
//! not shipped; Task 7's manual rotate stands):** 300 upright photos × 4 turns — text prompts
//! pick the upright turn 82.7 % of the time; at a margin of 0.03, precision 0.948 and recall
//! 0.344 on synthetic turns, 0.7 % of upright photos flagged. "Similar to the library's
//! typical photo" is worse than chance (turned photos look *more* generic to CLIP). Over all
//! 5,059 photos as displayed, 20 were flagged: about 16 really turned (all iPhone 12 HEICs
//! tagged 6 while the scene was landscape — the phone pointed steeply up or down), 3 wrong
//! (a cut-out cat, a cat on its back, a sunset), and **no documents at all**, which were the
//! gate (precision ≥ 0.95 at recall ≥ 0.5 on documents).

use std::path::Path;

use image::imageops;
use photos_core::embedding::cosine;
use photos_ml::clip::ImageEncoder;
use photos_ml::manifest::manifest;
use photos_ml::text::TextEncoder;

const UP: &[&str] = &["a photo", "a photo of a person", "a photo of a landscape", "a photo of a page of text", "a photo of a street"];
const TURNED: &[&str] = &[
    "an upside-down photo",
    "a photo rotated sideways",
    "an upside-down photo of a person",
    "a sideways photo of a page of text",
    "an upside-down page of text",
];

#[test]
#[ignore = "needs the models and a list of thumbnails"]
fn rotation_accuracy() {
    let dir = std::env::var("LUMEN_MODELS_DIR").expect("LUMEN_MODELS_DIR");
    let list = std::fs::read_to_string(std::env::var("LUMEN_ROTATION_LIST").expect("LUMEN_ROTATION_LIST")).unwrap();
    let m = manifest();
    let img_entry = m.by_role("image_embedding").unwrap();
    let mut images = ImageEncoder::open(&Path::new(&dir).join(&img_entry.file), img_entry).unwrap();
    let (te, tk) = (m.by_role("text_embedding").unwrap(), m.by_role("tokenizer").unwrap());
    let mut text = TextEncoder::open(&Path::new(&dir).join(&te.file), &Path::new(&dir).join(&tk.file), te).unwrap();
    let up: Vec<_> = UP.iter().map(|p| text.embed(p).unwrap()).collect();
    let turned: Vec<_> = TURNED.iter().map(|p| text.embed(p).unwrap()).collect();

    // Every photo, all four turns: embeddings[i][k] is photo i turned k quarters clockwise.
    let mut embeddings = Vec::new();
    for path in list.lines().filter(|l| !l.trim().is_empty()) {
        let Ok(img) = image::open(path.trim()) else { continue };
        let rgb = img.to_rgb8();
        let turns = [rgb.clone(), imageops::rotate90(&rgb), imageops::rotate180(&rgb), imageops::rotate270(&rgb)];
        let row: Vec<_> = turns.iter().map(|t| images.embed(t.as_raw(), t.width(), t.height()).unwrap().0).collect();
        embeddings.push(row);
    }
    let n = embeddings.len();
    println!("{n} photos × 4 turns");

    // The library's typical photo: the mean of the upright embeddings (leave-one-out would be
    // fairer; with 300 photos one photo moves the mean by a third of a percent).
    let dim = embeddings[0][0].len();
    let mut mean = vec![0f32; dim];
    for row in &embeddings {
        for (m, v) in mean.iter_mut().zip(&row[0]) {
            *m += *v as f32 / n as f32;
        }
    }
    let mean_q: Vec<i8> = mean.iter().map(|v| v.round().clamp(-127.0, 127.0) as i8).collect();

    let text_score = |e: &photos_core::embedding::Vector| {
        let u = up.iter().map(|t| cosine(e, t)).fold(f32::MIN, f32::max);
        let r = turned.iter().map(|t| cosine(e, t)).fold(f32::MIN, f32::max);
        u - r
    };
    let mean_score = |e: &photos_core::embedding::Vector| cosine(e, &mean_q);

    for (name, scorer) in [
        ("text", &text_score as &dyn Fn(&photos_core::embedding::Vector) -> f32),
        ("typical", &mean_score),
        ("both", &|e: &photos_core::embedding::Vector| text_score(e) + mean_score(e)),
    ] {
        // For a photo fed turned by k, the upright version is the one turned back by 4-k;
        // "which of the four feeds is upright" is the same as "which k was applied".
        let mut margins: Vec<(f32, bool, bool)> = Vec::new(); // (margin, needs turning, named the right turn)
        for row in &embeddings {
            let scores: Vec<f32> = row.iter().map(scorer).collect();
            for applied in 0..4usize {
                // Seen from the fed image, candidate j (turning it j more quarters) lands on
                // original turn (applied + j) % 4; the upright one is j = (4 - applied) % 4.
                let cand: Vec<f32> = (0..4).map(|j| scores[(applied + j) % 4]).collect();
                let best = (0..4).max_by(|&a, &b| cand[a].total_cmp(&cand[b])).unwrap();
                let margin = cand[best] - cand[0];
                margins.push((margin, applied != 0, best == (4 - applied) % 4 && best != 0));
            }
        }
        let rotated_total = margins.iter().filter(|m| m.1).count();
        println!("\n{name}: top-1 accuracy {:.3}", {
            let right = embeddings.iter().enumerate().count();
            let _ = right;
            margins.iter().filter(|m| (m.1 && m.2) || (!m.1 && m.0 <= 0.0)).count() as f32 / margins.len() as f32
        });
        for t in [0.0f32, 0.01, 0.02, 0.03, 0.04, 0.05, 0.06, 0.08] {
            let suggested: Vec<_> = margins.iter().filter(|m| m.0 > t).collect();
            let correct = suggested.iter().filter(|m| m.1 && m.2).count();
            let precision = if suggested.is_empty() { 1.0 } else { correct as f32 / suggested.len() as f32 };
            let recall = correct as f32 / rotated_total as f32;
            // What matters in a real library, where few photos are turned: how many upright
            // photos would be flagged.
            let upright_flagged = margins.iter().filter(|m| !m.1 && m.0 > t).count() as f32 / (margins.len() - rotated_total) as f32;
            println!(
                "  threshold {t:.2}: suggestions {:4}, precision {precision:.3}, recall {recall:.3}, upright flagged {upright_flagged:.3}",
                suggested.len()
            );
        }
    }
}

/// What the detector would suggest on a real library, photo by photo as it is displayed now:
/// writes `<path>\t<quarter turns clockwise>\t<margin>` for every photo flagged above
/// `LUMEN_ROTATION_THRESHOLD` (default 0.03) to `LUMEN_ROTATION_OUT`, to be judged by eye.
#[test]
#[ignore = "needs the models and a list of thumbnails"]
fn rotation_suggestions_on_a_library() {
    let dir = std::env::var("LUMEN_MODELS_DIR").expect("LUMEN_MODELS_DIR");
    let list = std::fs::read_to_string(std::env::var("LUMEN_ROTATION_LIST").expect("LUMEN_ROTATION_LIST")).unwrap();
    let out = std::env::var("LUMEN_ROTATION_OUT").expect("LUMEN_ROTATION_OUT");
    let threshold: f32 = std::env::var("LUMEN_ROTATION_THRESHOLD").ok().and_then(|v| v.parse().ok()).unwrap_or(0.03);
    let m = manifest();
    let img_entry = m.by_role("image_embedding").unwrap();
    let mut images = ImageEncoder::open(&Path::new(&dir).join(&img_entry.file), img_entry).unwrap();
    let (te, tk) = (m.by_role("text_embedding").unwrap(), m.by_role("tokenizer").unwrap());
    let mut text = TextEncoder::open(&Path::new(&dir).join(&te.file), &Path::new(&dir).join(&tk.file), te).unwrap();
    let up: Vec<_> = UP.iter().map(|p| text.embed(p).unwrap()).collect();
    let turned: Vec<_> = TURNED.iter().map(|p| text.embed(p).unwrap()).collect();
    let score = |e: &photos_core::embedding::Vector| {
        up.iter().map(|t| cosine(e, t)).fold(f32::MIN, f32::max) - turned.iter().map(|t| cosine(e, t)).fold(f32::MIN, f32::max)
    };
    let mut flagged = String::new();
    let (mut seen, started) = (0, std::time::Instant::now());
    for path in list.lines().map(str::trim).filter(|l| !l.is_empty()) {
        let Ok(img) = image::open(path) else { continue };
        let rgb = img.to_rgb8();
        let turns = [rgb.clone(), imageops::rotate90(&rgb), imageops::rotate180(&rgb), imageops::rotate270(&rgb)];
        let s: Vec<f32> = turns.iter().map(|t| score(&images.embed(t.as_raw(), t.width(), t.height()).unwrap().0)).collect();
        let best = (0..4).max_by(|&a, &b| s[a].total_cmp(&s[b])).unwrap();
        if best != 0 && s[best] - s[0] > threshold {
            flagged.push_str(&format!("{path}\t{best}\t{:.4}\n", s[best] - s[0]));
        }
        seen += 1;
        if seen % 500 == 0 {
            println!("{seen} in {:?}", started.elapsed());
        }
    }
    std::fs::write(&out, &flagged).unwrap();
    println!("{seen} photos, {} flagged at {threshold}", flagged.lines().count());
}
