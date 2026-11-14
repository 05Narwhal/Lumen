//! Recovering the readable part of a damaged file (Round 22 §4). Every file is damaged by the
//! test itself in a temp dir. Tests print a note and pass when FFmpeg (or, for the thumbnail
//! case, ExifTool) is absent, as the crate's other tool tests do.

use std::path::{Path, PathBuf};
use std::process::Command;

use image::{ImageBuffer, Rgb};
use photos_media::recover::{recover, RecoveredKind};
use photos_media::{ExifTool, Ffmpeg};

fn ffmpeg() -> Option<Ffmpeg> {
    let f = Ffmpeg::locate();
    if f.is_none() {
        eprintln!("note: FFmpeg not found (see scripts/fetch-ffmpeg.*); skipping");
    }
    f
}

/// A photo-like 1200 × 800 JPEG: a gradient under noise, so no row is one flat colour.
fn noisy_jpeg(out: &Path, w: u32, h: u32) {
    let mut seed: u32 = 0x1234_5678;
    let mut next = move || {
        seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        (seed >> 24) as u8
    };
    let img: ImageBuffer<Rgb<u8>, Vec<u8>> = ImageBuffer::from_fn(w, h, |x, y| {
        let n = next() / 3;
        Rgb([((x * 160 / w) as u8).saturating_add(n), ((y * 160 / h) as u8).saturating_add(n), 90u8.saturating_add(n)])
    });
    img.save_with_format(out, image::ImageFormat::Jpeg).unwrap();
}

fn cut(path: &Path, keep: usize) -> PathBuf {
    let bytes = std::fs::read(path).unwrap();
    let out = path.with_file_name(format!("cut-{}", path.file_name().unwrap().to_string_lossy()));
    std::fs::write(&out, &bytes[..keep.min(bytes.len())]).unwrap();
    out
}

#[test]
fn a_jpeg_cut_short_recovers_its_top() {
    let Some(f) = ffmpeg() else { return };
    let dir = tempfile::tempdir().unwrap();
    let whole = dir.path().join("IMG_0001.jpg");
    noisy_jpeg(&whole, 1200, 800);
    let len = std::fs::metadata(&whole).unwrap().len() as usize;
    let damaged = cut(&whole, len * 60 / 100);

    let (out, r) = recover(&damaged, "IMG_0001.jpg", &dir.path().join("out/7"), &f, None).expect("a partial photo");
    assert_eq!(r.kind, RecoveredKind::Partial);
    assert!((40..=70).contains(&r.percent), "percent {}", r.percent);
    assert_eq!(out, dir.path().join("out/7.jpg"));
    let img = image::open(&out).unwrap().to_rgb8();
    assert_eq!((img.width(), img.height()), (r.width, r.height));
    assert_eq!(img.width(), 1200);
    assert!(img.height() < 800, "height {}", img.height());
    let last = img.height() - 1;
    let first = *img.get_pixel(0, last);
    assert!((0..img.width()).any(|x| *img.get_pixel(x, last) != first), "the last row is one flat colour");
}

#[test]
fn a_whole_jpeg_recovers_fully() {
    let Some(f) = ffmpeg() else { return };
    let dir = tempfile::tempdir().unwrap();
    let whole = dir.path().join("IMG_0002.jpg");
    noisy_jpeg(&whole, 640, 480);
    let (_, r) = recover(&whole, "IMG_0002.jpg", &dir.path().join("2"), &f, None).expect("the whole photo");
    assert_eq!(r.kind, RecoveredKind::Partial);
    assert_eq!(r.percent, 100);
    assert_eq!((r.width, r.height), (640, 480));
}

#[test]
fn random_bytes_recover_nothing() {
    let Some(f) = ffmpeg() else { return };
    let dir = tempfile::tempdir().unwrap();
    let junk = dir.path().join("IMG_0003.jpg");
    let mut seed: u32 = 7;
    let bytes: Vec<u8> = (0..50_000)
        .map(|_| {
            seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12_345);
            (seed >> 16) as u8
        })
        .collect();
    std::fs::write(&junk, bytes).unwrap();
    assert!(recover(&junk, "IMG_0003.jpg", &dir.path().join("3"), &f, ExifTool::locate().as_ref()).is_none());
    assert!(!dir.path().join("3.jpg").exists());
}

/// The end of the JPEG's EXIF (APP1) segment.
fn end_of_app1(bytes: &[u8]) -> usize {
    let mut at = 2;
    while at + 4 <= bytes.len() && bytes[at] == 0xFF {
        let len = u16::from_be_bytes([bytes[at + 2], bytes[at + 3]]) as usize;
        if bytes[at + 1] == 0xE1 {
            return at + 2 + len;
        }
        at += 2 + len;
    }
    panic!("no APP1 segment");
}

#[test]
fn a_file_with_only_a_thumbnail_is_preview_only() {
    let Some(f) = ffmpeg() else { return };
    let Some(et) = ExifTool::locate() else {
        eprintln!("note: ExifTool not found; skipping");
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let whole = dir.path().join("IMG_0004.jpg");
    let thumb = dir.path().join("thumb.jpg");
    noisy_jpeg(&whole, 1200, 800);
    noisy_jpeg(&thumb, 160, 120);
    let st = Command::new(&et.program)
        .args(["-q", "-overwrite_original", "-ExifImageWidth=1200", "-ExifImageHeight=800"])
        .arg(format!("-ThumbnailImage<={}", thumb.display()))
        .arg(&whole)
        .status()
        .unwrap();
    assert!(st.success());
    let bytes = std::fs::read(&whole).unwrap();
    let damaged = cut(&whole, end_of_app1(&bytes));

    let (_, r) = recover(&damaged, "IMG_0004.jpg", &dir.path().join("4"), &f, Some(&et)).expect("the thumbnail");
    assert_eq!(r.kind, RecoveredKind::Preview);
    assert_eq!((r.width, r.height), (160, 120));
    assert!((1..=5).contains(&r.percent), "percent {}", r.percent);
}

/// A 4 s 320 × 240 H.264 clip, its index at the front or at the end.
fn clip(f: &Ffmpeg, out: &Path, faststart: bool) {
    let mut c = Command::new(&f.ffmpeg);
    c.args(["-v", "error", "-y", "-f", "lavfi", "-i", "testsrc=size=320x240:rate=25"])
        // A steady bitrate, so half the bytes is about half the length.
        .args(["-t", "4", "-c:v", "libx264", "-pix_fmt", "yuv420p", "-g", "25", "-b:v", "300k", "-maxrate", "300k", "-bufsize", "150k"]);
    if faststart {
        c.args(["-movflags", "+faststart"]);
    }
    assert!(c.arg(out).status().unwrap().success());
}

#[test]
fn a_video_cut_short_keeps_its_start() {
    let Some(f) = ffmpeg() else { return };
    let dir = tempfile::tempdir().unwrap();

    let front = dir.path().join("front.mp4");
    clip(&f, &front, true);
    let half = std::fs::metadata(&front).unwrap().len() as usize / 2;
    let damaged = cut(&front, half);
    let (out, r) = recover(&damaged, "front.mp4", &dir.path().join("5"), &f, None).expect("the start of the clip");
    assert_eq!(r.kind, RecoveredKind::Video);
    assert_eq!(out, dir.path().join("5.mp4"));
    assert!((30..=60).contains(&r.percent), "percent {}", r.percent);

    let end = dir.path().join("end.mp4");
    clip(&f, &end, false);
    let half = std::fs::metadata(&end).unwrap().len() as usize / 2;
    let damaged = cut(&end, half);
    assert!(recover(&damaged, "end.mp4", &dir.path().join("6"), &f, None).is_none());
    assert!(!dir.path().join("6.mp4").exists());
}
