//! Round 4B, Task 4: how many faces each scan size finds, and what it costs. Reads a folder of
//! JPEGs (copies of the 2560 px previews — never the library's own folder) and runs YuNet on
//! each at 1024, 1536 and 2048 px on the long edge.
//!
//! ```sh
//! LUMEN_MODELS_DIR=<models> LUMEN_SCAN_FOLDER=<folder of copies> cargo test --release -p photos-ml --test scan_size -- --ignored --nocapture
//! ```

use std::path::Path;
use std::time::{Duration, Instant};

use photos_ml::faces::FaceDetector;
use photos_ml::manifest::manifest;

#[test]
#[ignore = "needs the models and a folder of photos"]
fn faces_found_per_scan_size() {
    let dir = std::env::var("LUMEN_MODELS_DIR").expect("LUMEN_MODELS_DIR");
    let folder = std::env::var("LUMEN_SCAN_FOLDER").expect("LUMEN_SCAN_FOLDER");
    let entry = manifest().by_role("face_detection").unwrap();
    let mut det = FaceDetector::open(&Path::new(&dir).join(&entry.file), entry).unwrap();
    let sizes = [1024u32, 1536, 2048];
    let (mut found, mut took) = ([0usize; 3], [Duration::ZERO; 3]);
    let mut photos = 0;
    for f in std::fs::read_dir(&folder).unwrap() {
        let Ok(img) = image::open(f.unwrap().path()) else { continue };
        photos += 1;
        for (i, s) in sizes.iter().enumerate() {
            let small = img.resize(*s, *s, image::imageops::FilterType::Triangle).to_rgb8();
            let (w, h) = small.dimensions();
            let t = Instant::now();
            found[i] += det.detect(small.as_raw(), w, h).unwrap().len();
            took[i] += t.elapsed();
        }
    }
    for (i, s) in sizes.iter().enumerate() {
        println!(
            "{s} px: {} faces ({:.0} % of 2048's), {:.0} ms a photo",
            found[i],
            100.0 * found[i] as f64 / found[2].max(1) as f64,
            took[i].as_secs_f64() * 1000.0 / photos as f64
        );
    }
}
