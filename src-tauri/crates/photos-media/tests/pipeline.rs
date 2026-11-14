use std::path::Path;

use image::{ImageBuffer, Rgb};
use photos_media::*;

fn fixture(dir: &Path) -> std::path::PathBuf {
    let img = ImageBuffer::from_fn(640, 480, |x, y| Rgb([(x / 3) as u8, (y / 2) as u8, 128u8]));
    let p = dir.join("gradient.jpg");
    img.save(&p).unwrap();
    p
}

#[test]
fn detects_kind_and_rejects_unknown() {
    assert_eq!(kind_for_path(Path::new("a.JPG")), MediaKind::Image);
    assert_eq!(kind_for_path(Path::new("a.cr3")), MediaKind::Raw);
    assert_eq!(kind_for_path(Path::new("a.jxl")), MediaKind::Jxl);
    assert_eq!(kind_for_path(Path::new("a.heic")), MediaKind::Heic);
    assert_eq!(kind_for_path(Path::new("a.mov")), MediaKind::Video);
    assert!(!is_supported(Path::new("notes.txt")));
    assert!(is_renderable(MediaKind::Raw));
    assert!(!is_renderable(MediaKind::Heic));
    assert_eq!(media_type_code(MediaKind::Video), 1);
}

#[test]
fn decodes_resizes_and_derives() {
    let tmp = tempfile::tempdir().unwrap();
    let src = fixture(tmp.path());
    let img = decode_preview(&src, MediaKind::Image).unwrap();
    assert_eq!((img.width(), img.height()), (640, 480));
    let small = resize_long_edge(&img, 256);
    assert_eq!((small.width(), small.height()), (256, 192));
    assert_eq!(resize_long_edge(&img, 4000).width(), 640, "never upscales");
    let out = tmp.path().join("nested/t.jpg");
    let bytes = encode_jpeg(&small, 82, &out).unwrap();
    assert!(bytes > 1000 && out.is_file());
    let th = thumbhash_for(&img);
    assert!((20..=30).contains(&th.len()), "thumbhash is ~25 bytes, got {}", th.len());
    let c = dominant_color(&img);
    assert!(c.starts_with('#') && c.len() == 7, "{c}");
    let meta = read_metadata(&src, MediaKind::Image);
    assert_eq!(meta.orientation, 1);
    assert_eq!(meta.width, Some(640));
    assert!(meta.captured_at.is_none());
}

#[test]
fn orientation_six_rotates() {
    let img = image::DynamicImage::new_rgb8(40, 20);
    let r = apply_orientation(img, 6);
    assert_eq!((r.width(), r.height()), (20, 40));
}

#[test]
fn store_paths_fan_out_by_hash_prefix() {
    let s = DerivativeStore::new(Path::new("/lib"));
    assert_eq!(s.thumb_path("3f8a2b1c", 256), Path::new("/lib/derivatives/thumbs/3f/3f8a2b1c_256.jpg"));
    assert_eq!(s.preview_path("3f8a2b1c"), Path::new("/lib/derivatives/previews/3f/3f8a2b1c_2560.jpg"));
    assert_eq!(s.path_for("3f8a2b1c", 2560), s.preview_path("3f8a2b1c"));
}

#[test]
fn unsupported_kinds_error_cleanly() {
    let tmp = tempfile::tempdir().unwrap();
    let p = tmp.path().join("clip.mov");
    std::fs::write(&p, b"not a video").unwrap();
    assert!(matches!(decode_preview(&p, MediaKind::Video), Err(MediaError::Unsupported(_))));
}
