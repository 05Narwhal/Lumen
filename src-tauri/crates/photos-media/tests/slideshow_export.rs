//! Exporting a slideshow of real photographs. Ignored by default; point it at a folder of
//! originals (it only reads them, and writes into a temp directory):
//!
//! ```sh
//! LUMEN_SLIDESHOW_PHOTOS="C:/Users/you/Pictures/Lumen Library/originals" \
//!   cargo test --release -p photos-media --test slideshow_export -- --ignored --nocapture
//! ```
//!
//! It answers the two questions Phase 6b's acceptance asks: does the MP4 last exactly as
//! long as the preview's timeline says, and how fast does a 1080p export run.

use std::path::PathBuf;
use std::time::Instant;

use photos_core::slideshow::{build, frame_layout, timeline, AudioTrack, FrameKind, LayoutStyle, PhotoShape, SlideshowDoc};
use photos_media::{decode_preview_with, encode_jpeg, frame::compose, kind_for_extension, resize_long_edge, Ffmpeg, MediaKind};

/// As many photos as the export job would use for a memory-sized show (`LUMEN_SLIDESHOW_CLIPS`
/// to try another size).
const CLIPS: usize = 12;

#[test]
#[ignore = "needs LUMEN_SLIDESHOW_PHOTOS and FFmpeg"]
fn exports_a_1080p_show_of_real_photos() {
    let Ok(folder) = std::env::var("LUMEN_SLIDESHOW_PHOTOS") else {
        println!("LUMEN_SLIDESHOW_PHOTOS not set");
        return;
    };
    let Some(ff) = Ffmpeg::locate() else {
        println!("FFmpeg not found");
        return;
    };

    let mut originals: Vec<PathBuf> = walk(PathBuf::from(&folder));
    originals.sort();
    originals.retain(|p| {
        let kind = p.extension().map(|e| kind_for_extension(&e.to_string_lossy())).unwrap_or(MediaKind::Unknown);
        matches!(kind, MediaKind::Image | MediaKind::Heic | MediaKind::Raw)
    });
    let clips = std::env::var("LUMEN_SLIDESHOW_CLIPS").ok().and_then(|v| v.parse().ok()).unwrap_or(CLIPS);
    originals.truncate(clips);
    assert!(originals.len() >= 2, "need at least two photos in {folder}");

    let dir = tempfile::tempdir().unwrap();

    // Decode first, because the layout is decided by the photographs' real shapes.
    let decode_started = Instant::now();
    let decoded: Vec<image::DynamicImage> = originals
        .iter()
        .filter_map(|source| {
            let kind = kind_for_extension(&source.extension().unwrap_or_default().to_string_lossy());
            match decode_preview_with(source, kind, Some(&ff)) {
                Ok(img) => Some(resize_long_edge(&img, 3840)),
                Err(e) => {
                    println!("skipped {}: {e}", source.display());
                    None
                }
            }
        })
        .collect();
    println!("decoded {} originals in {:.1} s", decoded.len(), decode_started.elapsed().as_secs_f64());
    let shapes: Vec<PhotoShape> = decoded
        .iter()
        .enumerate()
        .map(|(i, img)| PhotoShape { id: i as i64 + 1, aspect: img.width() as f32 / img.height().max(1) as f32, video: None })
        .collect();
    let portraits = shapes.iter().filter(|s| s.portrait()).count();
    let doc = SlideshowDoc { resolution: [1920, 1080], ..build(&shapes, LayoutStyle::Smart) };
    let frames: Vec<FrameKind> = doc.clips.iter().map(|c| c.frame).collect();
    println!(
        "{} photos ({portraits} portrait) → {} clips: {} fill, {} blur, {} grid",
        shapes.len(),
        doc.clips.len(),
        frames.iter().filter(|f| **f == FrameKind::Fill).count(),
        frames.iter().filter(|f| **f == FrameKind::Blur).count(),
        frames.iter().filter(|f| **f == FrameKind::Grid).count(),
    );

    // What the export job does first: compose each clip's whole frame, once, at twice the
    // output size so the Ken Burns zoom still has pixels to show.
    let stills_started = Instant::now();
    let frame_aspect = doc.resolution[0] as f32 / doc.resolution[1] as f32;
    // As the app does (Round 4F): several at a time, at most six.
    let jobs = (std::thread::available_parallelism().map(|n| n.get()).unwrap_or(2) / 2).saturating_sub(1).clamp(1, 6);
    let pool = rayon::ThreadPoolBuilder::new().num_threads(jobs).build().unwrap();
    let stills: Vec<PathBuf> = pool.install(|| {
        use rayon::prelude::*;
        doc.clips
        .par_iter()
        .enumerate()
        .map(|(i, clip)| {
            let photos: Vec<image::DynamicImage> =
                clip.assets().iter().map(|id| decoded[(*id - 1) as usize].clone()).collect();
            let aspects: Vec<f32> = photos.iter().map(|p| p.width() as f32 / p.height().max(1) as f32).collect();
            let composed = compose(
                &frame_layout(clip.frame, &aspects, frame_aspect),
                &photos,
                doc.resolution[0] * 2,
                doc.resolution[1] * 2,
            );
            let out = dir.path().join(format!("clip{i}.jpg"));
            encode_jpeg(&composed, 92, &out).unwrap();
            out
        })
        .collect()
    });
    let stills_secs = stills_started.elapsed().as_secs_f64();

    let song = dir.path().join("song.wav");
    let made = std::process::Command::new(&ff.ffmpeg)
        .args(["-v", "error", "-y", "-f", "lavfi", "-i", "sine=frequency=330:duration=120"])
        .arg(&song)
        .status()
        .unwrap();
    assert!(made.success());

    let doc = SlideshowDoc {
        audio: Some(AudioTrack { path: song.display().to_string(), fade_in_ms: 1000, fade_out_ms: 1500, gain_db: -4.0, start_ms: 0 }),
        ..doc
    };
    // Every photograph is in the show exactly once, whatever frame it landed in.
    let shown: usize = doc.clips.iter().map(|c| c.assets().len()).sum();
    assert_eq!(shown, decoded.len(), "every photo is shown once");
    let total = timeline(&doc).total_ms;
    let out = dir.path().join("show.mp4");
    let render_started = Instant::now();
    // As the app does since Round 4F: pieces side by side (`LUMEN_SLIDESHOW_SINGLE` for one run).
    if std::env::var("LUMEN_SLIDESHOW_SINGLE").is_ok() {
        photos_media::slideshow::render_slideshow(&ff, &doc, &stills, Some(&song), &out, &mut |_| {}, &|| false).unwrap();
    } else {
        let cores = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(2);
        let (pieces, threads) = photos_media::slideshow::parallel_plan(cores);
        let bounds = photos_core::slideshow::segment_bounds(&doc, pieces);
        println!("{} pieces of up to {pieces}, {threads} encoder threads each", bounds.len());
        let encoder = photos_media::slideshow::best_encoder(&ff, std::env::var("LUMEN_SLIDESHOW_X264").is_err());
        println!("encoder {}", encoder.ffmpeg_name());
        photos_media::slideshow::render_slideshow_parallel(&ff, &doc, &stills, Some(&song), &out, &dir.path().join("pieces"), pieces, threads, encoder, None, &mut |_| {}, &|| false).unwrap();
    }
    let render_secs = render_started.elapsed().as_secs_f64();

    // Round 4F, Task 4: every encoder that works here, timed in one run each, and its
    // quality: PSNR against a near-lossless x264 render of the same graph.
    if let Ok(list) = std::env::var("LUMEN_SLIDESHOW_ENCODERS") {
        use photos_media::slideshow::{ffmpeg_args_with, probe_encoder, Encoder};
        let reference = dir.path().join("reference.mp4");
        let mut args = ffmpeg_args_with(&doc, &stills, None, &reference, Encoder::X264, 8, None);
        args.retain(|a| a != "-progress" && a != "pipe:1");
        assert!(std::process::Command::new(&ff.ffmpeg).args(&args).status().unwrap().success());
        for item in list.split(',') {
            let (name, q) = item.split_once(':').unwrap_or((item, ""));
            let enc = match name {
                "x264" => Encoder::X264,
                "nvenc" => Encoder::Nvenc,
                "qsv" => Encoder::Qsv,
                "amf" => Encoder::Amf,
                _ => continue,
            };
            if !probe_encoder(&ff, enc) {
                println!("{name}: does not work here");
                continue;
            }
            let q: u32 = q.parse().unwrap_or(enc.default_quality());
            let file = dir.path().join(format!("{name}-{q}.mp4"));
            let mut args = ffmpeg_args_with(&doc, &stills, None, &file, enc, q, None);
            args.retain(|a| a != "-progress" && a != "pipe:1");
            let t = Instant::now();
            assert!(std::process::Command::new(&ff.ffmpeg).args(&args).status().unwrap().success());
            let secs = t.elapsed().as_secs_f64();
            let o = std::process::Command::new(&ff.ffmpeg)
                .args(["-v", "info", "-i"])
                .arg(&file)
                .arg("-i")
                .arg(&reference)
                .args(["-lavfi", "[0:v][1:v]psnr", "-f", "null", "-"])
                .output()
                .unwrap();
            let err = String::from_utf8_lossy(&o.stderr);
            let psnr = err.split_whitespace().find_map(|w| w.strip_prefix("average:")).unwrap_or("?").to_string();
            println!("{name} q{q}: {secs:.1} s, {:.1} MB, PSNR {psnr} dB", std::fs::metadata(&file).unwrap().len() as f64 / 1_048_576.0);
        }
    }

    // Round 4F, Task 2: the filters alone, decoded to nothing (how much of the time is the
    // motion and the transitions rather than the H.264 encoder).
    if std::env::var("LUMEN_SLIDESHOW_NULL").is_ok() {
        let mut args = photos_media::slideshow::ffmpeg_args(&doc, &stills, Some(&song), &dir.path().join("x.mp4"));
        let keep = args.iter().position(|a| a == "-c:v").unwrap();
        args.truncate(keep);
        args.extend(["-f", "null", "-"].map(String::from));
        let t = Instant::now();
        let ok = std::process::Command::new(&ff.ffmpeg).args(&args).status().unwrap();
        assert!(ok.success());
        println!("filters alone: {:.1} s", t.elapsed().as_secs_f64());
    }

    // Round 4F, Task 2: the same graph with FFmpeg's own accounting, to see how many cores it
    // keeps busy (user time over wall time).
    if std::env::var("LUMEN_SLIDESHOW_BENCH").is_ok() {
        let bench = dir.path().join("bench.mp4");
        let mut args = vec!["-benchmark".to_string()];
        args.extend(photos_media::slideshow::ffmpeg_args(&doc, &stills, Some(&song), &bench).into_iter().map(|a| if a == "error" { "info".into() } else { a }));
        let out = std::process::Command::new(&ff.ffmpeg).args(&args).output().unwrap();
        let err = String::from_utf8_lossy(&out.stderr);
        let field = |k: &str| err.split_whitespace().find_map(|w| w.strip_prefix(k)).and_then(|v| v.trim_end_matches('s').parse::<f64>().ok());
        if let (Some(u), Some(r)) = (field("utime="), field("rtime=")) {
            println!("ffmpeg: user {u:.1} s over wall {r:.1} s = {:.2} cores busy", u / r);
        }
    }

    let probe = ff.probe(&out).unwrap();
    let got = probe.duration_ms.unwrap() as i64;
    let bytes = std::fs::metadata(&out).unwrap().len();
    println!(
        "{} clips → {:.1} s of 1080p in {:.1} s ({:.2}× real time; stills {:.1} s), {:.1} MB, duration {got} ms vs timeline {total} ms",
        stills.len(),
        total as f64 / 1000.0,
        render_secs,
        total as f64 / 1000.0 / render_secs,
        stills_secs,
        bytes as f64 / 1_048_576.0,
    );
    assert!((got - total as i64).abs() <= 100, "exported {got} ms, timeline {total} ms");
    assert_eq!(probe.display_size(), (Some(1920), Some(1080)));

    // Keep one composed frame of each kind where a person can look at it.
    if let Ok(look) = std::env::var("LUMEN_FRAME_DUMP") {
        std::fs::create_dir_all(&look).ok();
        for kind in [FrameKind::Blur, FrameKind::Grid, FrameKind::Fill] {
            if let Some(i) = doc.clips.iter().position(|c| c.frame == kind) {
                std::fs::copy(&stills[i], PathBuf::from(&look).join(format!("frame-{kind:?}.jpg"))).ok();
            }
        }
    }
}

fn walk(root: PathBuf) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut stack = vec![root];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else {
                found.push(path);
            }
        }
    }
    found
}
