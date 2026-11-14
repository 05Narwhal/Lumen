//! Whether two images are the same picture, pixel for pixel (Round 4D's *Exact copy*): a
//! small signature of each, compared by mean absolute difference.

use std::path::Path;

/// A signature's side: enough to tell an edit from a re-encode, small enough to keep many.
pub const SIGNATURE_SIDE: u32 = 32;

/// The image at `path` brought down to `SIGNATURE_SIDE`² RGB, whatever its shape (a resized
/// copy has the same signature). `None` when it cannot be read.
pub fn signature(path: &Path) -> Option<Vec<u8>> {
    let img = image::open(path).ok()?;
    Some(img.resize_exact(SIGNATURE_SIDE, SIGNATURE_SIDE, image::imageops::FilterType::Triangle).to_rgb8().into_raw())
}

/// Mean absolute difference of two signatures, 0..255.
pub fn difference(a: &[u8], b: &[u8]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 255.0;
    }
    a.iter().zip(b).map(|(x, y)| (*x as i32 - *y as i32).unsigned_abs()).sum::<u32>() as f32 / a.len() as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    fn save(dir: &Path, name: &str, img: &image::RgbImage) -> std::path::PathBuf {
        let p = dir.join(name);
        img.save(&p).unwrap();
        p
    }

    #[test]
    fn a_resized_reencode_is_the_same_picture_and_an_edit_is_not() {
        let dir = tempfile::tempdir().unwrap();
        let photo = image::RgbImage::from_fn(400, 300, |x, y| image::Rgb([(x % 256) as u8, (y % 256) as u8, ((x + y) % 200) as u8]));
        let original = save(dir.path(), "a.png", &photo);
        let small = image::imageops::resize(&photo, 200, 150, image::imageops::FilterType::Triangle);
        let copy = dir.path().join("b.jpg");
        small.save(&copy).unwrap();
        let brighter = image::RgbImage::from_fn(400, 300, |x, y| {
            let p = photo.get_pixel(x, y).0;
            image::Rgb(p.map(|c| c.saturating_add(25)))
        });
        let edit = save(dir.path(), "c.png", &brighter);
        let (a, b, c) = (signature(&original).unwrap(), signature(&copy).unwrap(), signature(&edit).unwrap());
        let same = difference(&a, &b);
        let edited = difference(&a, &c);
        println!("re-encode {same:.2}, edit {edited:.2}");
        assert!(same < 1.5, "{same}");
        assert!(edited > 15.0, "{edited}");
        assert!(signature(&dir.path().join("missing.jpg")).is_none());
    }
}
