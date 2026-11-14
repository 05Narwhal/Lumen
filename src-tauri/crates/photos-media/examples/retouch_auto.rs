//! Auto's Basic (the measured prior) for prepared inputs, so Retouch's evaluation compares
//! against Auto on exactly the pixels Retouch saw, synthetic degradations included.
//!
//! `cargo run --release -p photos-media --example retouch_auto -- <file.u16 or folder>...`
//!
//! Prints one JSON line per file: `{"file": "<path>", "auto": <Basic>}` (or `"error"`). Files
//! are only read.

use std::path::{Path, PathBuf};

use photos_media::retouch_input::{auto_basic, read_input};

fn inputs(p: &Path, out: &mut Vec<PathBuf>) {
    if p.is_dir() {
        let Ok(entries) = std::fs::read_dir(p) else { return };
        let mut paths: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
        paths.sort();
        for q in paths {
            inputs(&q, out);
        }
    } else if p.extension().is_some_and(|e| e == "u16") {
        out.push(p.to_path_buf());
    }
}

fn main() {
    let args: Vec<PathBuf> = std::env::args().skip(1).map(PathBuf::from).collect();
    if args.is_empty() {
        eprintln!("usage: retouch_auto <file.u16 or folder>...");
        std::process::exit(2);
    }
    let mut files = Vec::new();
    for a in &args {
        inputs(a, &mut files);
    }
    for f in files {
        let line = match std::fs::read(&f).map_err(|e| e.to_string()).and_then(|b| read_input(&b).map_err(|e| e.to_string())) {
            Ok(img) => serde_json::json!({ "file": f.to_string_lossy(), "auto": auto_basic(&img) }),
            Err(e) => serde_json::json!({ "file": f.to_string_lossy(), "error": e }),
        };
        println!("{line}");
    }
}
