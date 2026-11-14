//! Prepares photos for Retouch's training scripts, exactly as the app prepares them.
//!
//! `cargo run --release -p photos-media --example retouch_prep -- <photo-or-folder> <out-dir> [--auto] [--jobs N]`
//!
//! Each photo becomes `<out-dir>/<stem>-<hash>/{input.u16, meta.json}`. Sources are only read;
//! nothing is written inside their folder.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use photos_media::kind::{kind_for_extension, MediaKind};
use photos_media::retouch_input::{check_destination, prep_file};
use rayon::prelude::*;

fn photos_in(root: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(root) else { return };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            photos_in(&p, out);
        } else {
            let ext = p.extension().map(|e| e.to_string_lossy().into_owned()).unwrap_or_default();
            if matches!(kind_for_extension(&ext), MediaKind::Image | MediaKind::Raw | MediaKind::Heic | MediaKind::Jxl) {
                out.push(p);
            }
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let flag = |name: &str| args.iter().any(|a| a == name);
    let value = |name: &str| args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned();
    let positional: Vec<&String> = args.iter().enumerate().filter(|(i, a)| !a.starts_with("--") && (*i == 0 || args[i - 1] != "--jobs")).map(|(_, a)| a).collect();
    let [input, out] = positional[..] else {
        eprintln!("usage: retouch_prep <photo-or-folder> <out-dir> [--auto] [--jobs N]");
        std::process::exit(2);
    };
    let (input, out) = (PathBuf::from(input), PathBuf::from(out));
    let mut photos = Vec::new();
    if input.is_dir() {
        // Each photo checks its own folder; a batch also keeps out of every folder it reads.
        if let Err(e) = check_destination(&out, &input) {
            eprintln!("{e}");
            std::process::exit(2);
        }
        photos_in(&input, &mut photos);
    } else {
        photos.push(input);
    }
    photos.sort();
    let jobs = value("--jobs").and_then(|j| j.parse().ok()).unwrap_or(4);
    let auto = flag("--auto");
    let done = AtomicUsize::new(0);
    let failed = AtomicUsize::new(0);
    let total = photos.len();
    rayon::ThreadPoolBuilder::new().num_threads(jobs).build().expect("thread pool").install(|| {
        photos.par_iter().for_each(|p| {
            if let Err(e) = prep_file(p, &out, auto) {
                failed.fetch_add(1, Ordering::Relaxed);
                eprintln!("skipped {}: {e}", p.display());
            }
            let n = done.fetch_add(1, Ordering::Relaxed) + 1;
            if n.is_multiple_of(100) || n == total {
                eprintln!("{n} / {total}");
            }
        });
    });
    println!("prepared {} of {total} into {}", total - failed.load(Ordering::Relaxed), out.display());
}
