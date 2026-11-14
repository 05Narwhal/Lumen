use std::path::Path;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MediaKind {
    Image,
    Raw,
    Jxl,
    Heic,
    Video,
    Unknown,
}

const IMAGE: &[&str] = &["jpg", "jpeg", "jpe", "png", "webp", "gif", "tif", "tiff", "bmp"];
/// One list for the catalogue and the decoder: the RAW collection must hold exactly what
/// decodes as RAW.
const RAW: &[&str] = photos_core::formats::RAW_EXTENSIONS;
const HEIC: &[&str] = &["heic", "heif", "hif", "avif"];
const VIDEO: &[&str] = &["mp4", "m4v", "mov", "avi", "mkv", "mts", "m2ts", "webm", "3gp", "mpg", "mpeg", "wmv"];

pub fn extension_of(path: &Path) -> String {
    path.extension().and_then(|e| e.to_str()).map(|e| e.to_ascii_lowercase()).unwrap_or_default()
}

pub fn kind_for_extension(ext: &str) -> MediaKind {
    let e = ext.to_ascii_lowercase();
    let e = e.as_str();
    if IMAGE.contains(&e) {
        MediaKind::Image
    } else if RAW.contains(&e) {
        MediaKind::Raw
    } else if e == "jxl" {
        MediaKind::Jxl
    } else if HEIC.contains(&e) {
        MediaKind::Heic
    } else if VIDEO.contains(&e) {
        MediaKind::Video
    } else {
        MediaKind::Unknown
    }
}

pub fn kind_for_path(path: &Path) -> MediaKind {
    // `index.d.mts` is a TypeScript declaration file, not an MPEG transport stream: a
    // folder holding `node_modules` would otherwise import them as videos.
    let name = path.file_name().map(|n| n.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
    if name.ends_with(".d.mts") {
        return MediaKind::Unknown;
    }
    kind_for_extension(&extension_of(path))
}

pub fn is_supported(path: &Path) -> bool {
    kind_for_path(path) != MediaKind::Unknown
}

/// `.mts` and `.m2ts` name both camera video (AVCHD) and TypeScript modules.
const TRANSPORT_STREAM: &[&str] = &["mts", "m2ts"];

/// [`is_supported`], and for a name that can be something else, a look at the file itself
/// (Round 8A): a folder holding `node_modules` had TypeScript `.mts` files imported as videos.
/// A file that cannot be read is not taken.
pub fn is_media_file(path: &Path) -> bool {
    if !is_supported(path) {
        return false;
    }
    if !TRANSPORT_STREAM.contains(&extension_of(path).as_str()) {
        return true;
    }
    let mut head = [0u8; 600];
    let n = std::fs::File::open(path).and_then(|mut f| std::io::Read::read(&mut f, &mut head)).unwrap_or(0);
    is_transport_stream(&head[..n])
}

/// Whether the file is HEIF or AVIF whatever its name says (Round 8A): an ISO base media
/// file whose `ftyp` brand is an image one. Phones sometimes save a HEIC as `IMG_1855.JPG`.
pub fn is_heif_file(path: &Path) -> bool {
    content_kind(path) == Some(MediaKind::Heic)
}

/// `ftyp` brands of HEIF and AVIF stills.
const IMAGE_BRANDS: &[&[u8; 4]] = &[b"heic", b"heix", b"hevc", b"heim", b"heis", b"mif1", b"msf1", b"avif"];
/// `ftyp` brands of QuickTime and MP4 movies.
const VIDEO_BRANDS: &[&[u8; 4]] = &[b"qt  ", b"mp41", b"mp42", b"isom", b"iso2", b"avc1", b"M4V ", b"M4VH", b"M4VP", b"3gp4", b"3gp5", b"3gp6", b"3g2a", b"dash"];

/// What an ISO base media file (HEIF, QuickTime, MP4) is, from its `ftyp` brand, whatever its
/// name says: `Heic` for an image brand, `Video` for a movie one. `None` for anything else
/// (the name decides), an unknown brand, or a file that cannot be read. The first launch on
/// the real library logged HEIFs saved as `.JPG`, movies saved as `.JPG`/`.HEIC`, and a HEIF
/// saved as `.MOV`.
pub fn content_kind(path: &Path) -> Option<MediaKind> {
    let mut head = [0u8; 12];
    let n = std::fs::File::open(path).and_then(|mut f| std::io::Read::read(&mut f, &mut head)).unwrap_or(0);
    if n < 12 || &head[4..8] != b"ftyp" {
        return None;
    }
    let brand = [head[8], head[9], head[10], head[11]];
    if IMAGE_BRANDS.contains(&&brand) {
        Some(MediaKind::Heic)
    } else if VIDEO_BRANDS.contains(&&brand) {
        Some(MediaKind::Video)
    } else {
        None
    }
}

/// MPEG-TS packets: a sync byte 0x47 every 188 bytes, or every 192 for M2TS (AVCHD) whose
/// packets carry a 4-byte time code first. Three packets in a row, or as many as a short file has.
pub fn is_transport_stream(head: &[u8]) -> bool {
    let synced = |first: usize, step: usize| {
        let at: Vec<usize> = (0..3).map(|i| first + i * step).filter(|&i| i < head.len()).collect();
        !at.is_empty() && at.iter().all(|&i| head[i] == 0x47)
    };
    head.len() >= 188 && (synced(0, 188) || synced(4, 192))
}

/// `assets.media_type`: 0 image (incl. RAW, JXL, HEIC), 1 video.
pub fn media_type_code(kind: MediaKind) -> i64 {
    if kind == MediaKind::Video {
        1
    } else {
        0
    }
}

/// Whether a derivative can be made without FFmpeg (images, RAW, JXL).
pub fn is_renderable(kind: MediaKind) -> bool {
    is_renderable_with(kind, false)
}

/// Whether a derivative can be made *now*: HEIC and video need FFmpeg.
pub fn is_renderable_with(kind: MediaKind, ffmpeg_available: bool) -> bool {
    match kind {
        MediaKind::Image | MediaKind::Raw | MediaKind::Jxl => true,
        MediaKind::Heic | MediaKind::Video => ffmpeg_available,
        MediaKind::Unknown => false,
    }
}

/// `is_renderable_with` by file extension, for code that only has the row.
pub fn is_renderable_extension(ext: &str, ffmpeg_available: bool) -> bool {
    is_renderable_with(kind_for_extension(ext), ffmpeg_available)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_transport_stream_is_told_from_typescript_by_its_bytes() {
        let packets = |offset: usize, step: usize| {
            let mut v = vec![0u8; 600];
            for i in (offset..600).step_by(step) {
                v[i] = 0x47;
            }
            v
        };
        assert!(is_transport_stream(&packets(0, 188)), "MPEG-TS");
        assert!(is_transport_stream(&packets(4, 192)), "M2TS from a camcorder");
        assert!(!is_transport_stream(b"export const useLatest = <T>(value: T) => { const ref = useRef(value); return ref; };\n".repeat(8).as_slice()), "TypeScript");
        assert!(!is_transport_stream(&[0x47; 10]), "too short to tell");

        let dir = tempfile::tempdir().unwrap();
        let ts = dir.path().join("index-browser.mts");
        std::fs::write(&ts, "export default function x() {}\n".repeat(40)).unwrap();
        assert!(!is_media_file(&ts));
        let avchd = dir.path().join("00001.MTS");
        std::fs::write(&avchd, packets(4, 192)).unwrap();
        assert!(is_media_file(&avchd));
        assert!(!is_media_file(&dir.path().join("missing.mts")));
        let jpg = dir.path().join("a.jpg");
        assert!(is_media_file(&jpg), "other kinds go by name, as before");

        let heic_named_jpg = dir.path().join("IMG_1855.JPG");
        std::fs::write(&heic_named_jpg, b"\0\0\0\x18ftypheic\0\0\0\0mif1heic").unwrap();
        assert!(is_heif_file(&heic_named_jpg));
        let mov = dir.path().join("IMG_1852.MOV");
        std::fs::write(&mov, b"\0\0\0\x14ftypqt  \0\0\0\0").unwrap();
        assert!(!is_heif_file(&mov), "a QuickTime movie is not an image");
        std::fs::write(&jpg, [0xFF, 0xD8, 0xFF, 0xE0, 0, 0x10, b'J', b'F', b'I', b'F', 0, 1]).unwrap();
        assert!(!is_heif_file(&jpg));
    }

    #[test]
    fn the_container_brand_says_what_a_file_is_whatever_its_name() {
        let dir = tempfile::tempdir().unwrap();
        let write = |name: &str, bytes: &[u8]| {
            let p = dir.path().join(name);
            std::fs::write(&p, bytes).unwrap();
            p
        };
        let ftyp = |brand: &[u8; 4]| [b"\0\0\0\x14ftyp".as_slice(), brand.as_slice(), b"\0\0\0\0".as_slice()].concat();
        // The real library: QuickTime movies saved as IMG_1730.JPG and IMG_1768.HEIC, a HEIC as IMG_2028.MOV.
        for brand in [b"qt  ", b"mp42", b"mp41", b"isom", b"M4V "] {
            assert_eq!(content_kind(&write("IMG_1730.JPG", &ftyp(brand))), Some(MediaKind::Video), "{}", String::from_utf8_lossy(brand));
        }
        for brand in [b"heic", b"mif1", b"avif"] {
            assert_eq!(content_kind(&write("IMG_2028.MOV", &ftyp(brand))), Some(MediaKind::Heic), "{}", String::from_utf8_lossy(brand));
        }
        assert_eq!(content_kind(&write("a.jpg", &[0xFF, 0xD8, 0xFF, 0xE0, 0, 0x10, b'J', b'F', b'I', b'F', 0, 1])), None, "not a container: go by name");
        assert_eq!(content_kind(&write("b.heic", &ftyp(b"zzzz"))), None, "a brand nobody here knows");
        assert_eq!(content_kind(&write("c.mov", b"\0\0\0")), None, "too short");
        assert_eq!(content_kind(&dir.path().join("missing.jpg")), None);
    }

    #[test]
    fn typescript_declarations_are_not_videos() {
        assert_eq!(kind_for_path(Path::new("node_modules/@emotion/hash/dist/emotion-hash.cjs.d.mts")), MediaKind::Unknown);
        assert_eq!(kind_for_path(Path::new("PRIVATE/AVCHD/BDMV/STREAM/00001.MTS")), MediaKind::Video);
        assert_eq!(kind_for_path(Path::new("clip.m2ts")), MediaKind::Video);
    }
}
