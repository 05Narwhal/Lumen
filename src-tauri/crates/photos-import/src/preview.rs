//! A look at what is on a device before importing it: the newest few items, rendered small.

use std::io::Read;

use photos_media::Ffmpeg;

use crate::{ImportError, ImportSource, SourceItem};

/// The `n` most recently modified items, newest first; items without a date come last.
pub fn newest(items: &[SourceItem], n: usize) -> Vec<&SourceItem> {
    let mut sorted: Vec<&SourceItem> = items.iter().collect();
    sorted.sort_by(|a, b| b.modified_at.unwrap_or(i64::MIN).cmp(&a.modified_at.unwrap_or(i64::MIN)).then(b.rel_path.cmp(&a.rel_path)));
    sorted.truncate(n);
    sorted
}

/// How much of a file is read looking for its EXIF thumbnail.
const HEAD_BYTES: u64 = 256 * 1024;

/// A small preview from the thumbnail a camera stores in EXIF, reading only the file's head:
/// a RAW on a card is 25 MB and the thumbnail is in its first few KB. `None` when the file
/// has none or it is too small for `long_edge` (it may be up to a third smaller: tiles are
/// small and it is a preview of something not imported yet), and the caller renders the
/// whole file instead.
pub fn quick_preview(source: &dyn ImportSource, item: &SourceItem, long_edge: u32) -> Result<Option<Vec<u8>>, ImportError> {
    let ext = std::path::Path::new(&item.file_name).extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    let kind = photos_media::kind_for_extension(&ext);
    let jpeg_like = matches!(ext.as_str(), "jpg" | "jpeg");
    if !(jpeg_like || kind == photos_media::MediaKind::Raw) {
        return Ok(None);
    }
    let mut head = Vec::new();
    source.open(item)?.take(HEAD_BYTES).read_to_end(&mut head)?;
    let Some((thumb, orientation)) = photos_media::exif_thumbnail(&head) else { return Ok(None) };
    let Ok(img) = image::load_from_memory(&thumb) else { return Ok(None) };
    if img.width().max(img.height()) * 3 < long_edge * 2 {
        return Ok(None);
    }
    let own = img.width().max(img.height());
    let img = photos_media::apply_orientation(img, orientation);
    let img = if own > long_edge { photos_media::resize_long_edge(&img, long_edge) } else { img };
    let bytes = photos_media::encode_jpeg_bytes(&img, 82).map_err(|e| ImportError::Source(format!("{}: {e}", item.file_name)))?;
    Ok(Some(bytes))
}

/// A JPEG of `item` at most `long_edge` pixels on its long side, upright.
///
/// The bytes are copied to a temporary file first, with the original extension, because
/// every decoder here works from a path (FFmpeg for HEIC and video included). Orientation
/// is decided from the decoded pixels as ingest does it, so a preview never shows a photo
/// turned that the library would show upright.
pub fn preview_jpeg(source: &dyn ImportSource, item: &SourceItem, ffmpeg: Option<&Ffmpeg>, long_edge: u32) -> Result<Vec<u8>, ImportError> {
    let ext = std::path::Path::new(&item.file_name).extension().and_then(|e| e.to_str()).unwrap_or("bin").to_ascii_lowercase();
    let dir = tempfile::tempdir()?;
    let input = dir.path().join(format!("in.{ext}"));
    let mut bytes = Vec::new();
    source.open(item)?.read_to_end(&mut bytes)?;
    std::fs::write(&input, &bytes)?;

    let kind = photos_media::kind_for_extension(&ext);
    let media = |e: photos_media::MediaError| ImportError::Source(format!("{}: {e}", item.file_name));
    let meta = photos_media::read_metadata(&input, kind);
    let img = photos_media::decode_preview_with(&input, kind, ffmpeg).map_err(media)?;
    let exif = match (meta.width, meta.height) {
        (Some(w), Some(h)) if w > 0 && h > 0 => Some((w, h)),
        _ => None,
    };
    let orientation =
        photos_media::effective_orientation(meta.orientation, exif, (img.width(), img.height()), photos_media::decoder_orients(kind));
    let small = photos_media::resize_long_edge(&photos_media::apply_orientation(img, orientation), long_edge);
    let out = dir.path().join("out.jpg");
    photos_media::encode_jpeg(&small, 82, &out).map_err(media)?;
    Ok(std::fs::read(out)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::FolderSource;

    fn item(name: &str, modified_at: Option<i64>) -> SourceItem {
        SourceItem { rel_path: name.into(), file_name: name.into(), size: 1, modified_at, created_at: None }
    }

    #[test]
    fn newest_first_and_undated_last() {
        let items = vec![item("a.jpg", Some(10)), item("b.jpg", None), item("c.jpg", Some(30)), item("d.jpg", Some(20))];
        let got: Vec<&str> = newest(&items, 3).iter().map(|i| i.file_name.as_str()).collect();
        assert_eq!(got, ["c.jpg", "d.jpg", "a.jpg"]);
        assert_eq!(newest(&items, 10).last().unwrap().file_name, "b.jpg");
    }

    #[test]
    fn renders_a_small_upright_jpeg_from_any_source() {
        let dir = tempfile::tempdir().unwrap();
        image::RgbImage::from_pixel(1200, 800, image::Rgb([200, 40, 40])).save(dir.path().join("wide.jpg")).unwrap();
        let src = FolderSource::new(dir.path());
        let items = src.enumerate().unwrap();
        let bytes = preview_jpeg(&src, &items[0], None, 320).unwrap();
        let img = image::load_from_memory(&bytes).unwrap();
        assert_eq!((img.width(), img.height()), (320, 213));
    }

    #[test]
    fn a_raw_on_a_card_previews_from_its_exif_thumbnail() {
        use exif::experimental::Writer;
        use exif::{Field, In, Tag, Value};

        let mut thumb = Vec::new();
        image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(320, 240, image::Rgb([90, 90, 200])))
            .write_to(&mut std::io::Cursor::new(&mut thumb), image::ImageFormat::Jpeg)
            .unwrap();
        let mut tiff = std::io::Cursor::new(Vec::new());
        let mut writer = Writer::new();
        let turned = Field { tag: Tag::Orientation, ifd_num: In::PRIMARY, value: Value::Short(vec![6]) };
        writer.push_field(&turned);
        writer.set_jpeg(&thumb, In::THUMBNAIL);
        writer.write(&mut tiff, false).unwrap();
        let dir = tempfile::tempdir().unwrap();
        let mut raw = tiff.into_inner();
        // The sensor data a whole read would have to get through.
        raw.extend(std::iter::repeat_n(0u8, 2_000_000));
        std::fs::write(dir.path().join("DSC_0001.DNG"), &raw).unwrap();
        std::fs::write(dir.path().join("IMG_0002.PNG"), b"x").unwrap();

        let src = FolderSource::new(dir.path());
        let items = src.enumerate().unwrap();
        let bytes = quick_preview(&src, &items[0], 240).unwrap().expect("the EXIF thumbnail");
        let img = image::load_from_memory(&bytes).unwrap();
        assert_eq!((img.width(), img.height()), (180, 240), "upright, and no larger than asked");
        // Too small for a 1024 px look: the whole file is rendered instead.
        assert!(quick_preview(&src, &items[0], 1024).unwrap().is_none());
        assert!(quick_preview(&src, &items[1], 240).unwrap().is_none(), "a PNG has no EXIF thumbnail to try");
    }
}
