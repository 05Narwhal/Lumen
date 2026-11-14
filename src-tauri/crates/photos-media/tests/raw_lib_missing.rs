//! With the RAW library hidden (`LUMEN_PHOTOS_RAW` pointing at nothing), set once for this whole
//! test process before anything loads it: RAW files are a state, never a crash.

use std::io::Cursor;
use std::path::Path;

use image::{DynamicImage, ImageFormat, RgbImage};
use photos_media::develop::raw;
use photos_media::{decode_preview, read_metadata, raw_lib, MediaKind};

fn hide_the_library() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| unsafe { std::env::set_var("LUMEN_PHOTOS_RAW", if cfg!(windows) { r"C:\nothing\photos_raw.dll" } else { "/nothing/libphotos_raw.so" }) });
}

/// A RAW-shaped file: noise around one whole camera JPEG.
fn raw_with_a_jpeg(path: &Path) {
    let mut jpeg = Cursor::new(Vec::new());
    DynamicImage::ImageRgb8(RgbImage::from_pixel(800, 600, image::Rgb([90, 120, 150]))).write_to(&mut jpeg, ImageFormat::Jpeg).unwrap();
    let mut bytes = vec![7u8; 2048];
    bytes.extend(jpeg.into_inner());
    bytes.extend(vec![9u8; 2048]);
    std::fs::write(path, bytes).unwrap();
}

#[test]
fn a_missing_library_is_a_state() {
    hide_the_library();
    assert!(raw_lib::current().is_none());
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("DSC_0001.NEF");
    raw_with_a_jpeg(&p);

    // Develop is refused, in a way that says it's the install and not the file.
    let e = raw::decode(&p).unwrap_err();
    assert!(raw::is_refused(&e) && raw::is_no_library(&e), "{e:?}");

    // The camera's JPEG is still the preview; metadata still reads.
    let preview = decode_preview(&p, MediaKind::Raw).unwrap();
    assert_eq!((preview.width(), preview.height()), (800, 600));
    let _ = read_metadata(&p, MediaKind::Raw);

    // Nothing to preview either: not "unreadable", which would be remembered on disk.
    let bare = dir.path().join("DSC_0002.NEF");
    std::fs::write(&bare, vec![7u8; 4096]).unwrap();
    let e = decode_preview(&bare, MediaKind::Raw).unwrap_err();
    assert!(raw::is_no_library(&e), "{e:?}");

    // A file that can't be read stays an I/O error.
    let e = decode_preview(&dir.path().join("unplugged.NEF"), MediaKind::Raw).unwrap_err();
    assert!(matches!(e, photos_media::MediaError::Io(_)), "{e:?}");
}
