//! What Lumen makes of a real folder of photographs. Ignored by default; it only reads
//! the files:
//!
//! ```sh
//! LUMEN_FFMPEG_DIR=<repo>/src-tauri/binaries LUMEN_DATES_FOLDER="C:/Users/you/Pictures/Lumen Library/originals" \
//!   cargo test -p photos-media --test capture_dates -- --ignored --nocapture
//! ```

use std::path::PathBuf;

use photos_media::{kind_for_path, read_metadata_with, CaptureSource, Ffmpeg, MediaKind};

#[test]
#[ignore = "needs LUMEN_DATES_FOLDER"]
fn every_photo_that_knows_its_date_reports_one() {
    let Ok(folder) = std::env::var("LUMEN_DATES_FOLDER") else {
        println!("LUMEN_DATES_FOLDER not set");
        return;
    };
    let ffmpeg = Ffmpeg::locate();
    let mut files: Vec<PathBuf> = Vec::new();
    let mut stack = vec![PathBuf::from(folder)];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if !matches!(kind_for_path(&path), MediaKind::Unknown) {
                files.push(path);
            }
        }
    }
    files.sort();

    let mut counts = std::collections::BTreeMap::<String, usize>::new();
    for path in &files {
        let m = read_metadata_with(path, kind_for_path(path), ffmpeg.as_ref());
        let source = m.capture_source.map_or("none".to_string(), |s| s.as_str().to_string());
        let when = m
            .captured_at
            .and_then(|t| chrono::DateTime::from_timestamp(t, 0))
            .map_or("-".to_string(), |d| d.format("%Y-%m-%d %H:%M").to_string());
        println!("{:34} {when}  {source}", path.file_name().unwrap().to_string_lossy());
        *counts.entry(source).or_default() += 1;
    }
    println!("\n{} files: {counts:?}", files.len());
    // The bug this fixes: a whole library dated by the moment of export.
    assert!(
        counts.get(CaptureSource::Modified.as_str()).copied().unwrap_or(0) < files.len() / 2,
        "most files still fall back to the edit time: {counts:?}"
    );
}
