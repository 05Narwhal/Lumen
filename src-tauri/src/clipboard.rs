//! *Copy image* (Round 4G, the viewer's menu): the photo as the viewer shows it — its edits
//! applied, the 2560 preview — put on the system clipboard as pixels. The original file is
//! never read for this, and nothing is written anywhere but the clipboard.

use std::path::Path;
use std::sync::Mutex;

use crate::error::{AppError, AppResult};

/// Pixels ready for the clipboard: 8-bit RGBA, row by row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClipboardImage {
    pub width: usize,
    pub height: usize,
    pub rgba: Vec<u8>,
}

/// Where the image goes: the system clipboard, or a test's stand-in.
pub trait Clipboard {
    fn put_image(&mut self, image: ClipboardImage) -> Result<(), String>;
}

/// Decodes a rendered derivative (a JPEG on disk) into clipboard pixels.
pub fn prepare(path: &Path) -> AppResult<ClipboardImage> {
    let img = image::open(path).map_err(|e| AppError {
        code: "copy_failed".into(),
        message: "The photo could not be copied.".into(),
        detail: Some(e.to_string()),
    })?;
    let rgba = img.to_rgba8();
    Ok(ClipboardImage { width: rgba.width() as usize, height: rgba.height() as usize, rgba: rgba.into_raw() })
}

pub fn copy(to: &mut dyn Clipboard, image: ClipboardImage) -> AppResult<()> {
    to.put_image(image).map_err(|e| AppError { code: "clipboard".into(), message: "The clipboard would not take the image.".into(), detail: Some(e) })
}

/// The system clipboard, kept open for the life of the app: on X11 a clipboard's contents
/// live in the process that set them, so a dropped handle would take the image with it.
pub struct System;

static CLIPBOARD: Mutex<Option<arboard::Clipboard>> = Mutex::new(None);

impl Clipboard for System {
    fn put_image(&mut self, image: ClipboardImage) -> Result<(), String> {
        let mut slot = CLIPBOARD.lock().map_err(|e| e.to_string())?;
        if slot.is_none() {
            *slot = Some(arboard::Clipboard::new().map_err(|e| e.to_string())?);
        }
        let clip = slot.as_mut().expect("clipboard opened above");
        clip.set_image(arboard::ImageData { width: image.width, height: image.height, bytes: image.rgba.into() }).map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct Fake(Vec<ClipboardImage>);

    impl Clipboard for Fake {
        fn put_image(&mut self, image: ClipboardImage) -> Result<(), String> {
            self.0.push(image);
            Ok(())
        }
    }

    #[test]
    fn a_rendered_photo_becomes_rgba_of_its_size() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("preview.jpg");
        image::RgbImage::from_pixel(64, 40, image::Rgb([200, 120, 40])).save(&path).unwrap();
        let img = prepare(&path).unwrap();
        assert_eq!((img.width, img.height), (64, 40));
        assert_eq!(img.rgba.len(), 64 * 40 * 4);
        assert_eq!(img.rgba[3], 255, "opaque");
        let mut fake = Fake::default();
        copy(&mut fake, img.clone()).unwrap();
        assert_eq!(fake.0, [img]);
        assert!(prepare(&dir.path().join("missing.jpg")).is_err());
    }
}
