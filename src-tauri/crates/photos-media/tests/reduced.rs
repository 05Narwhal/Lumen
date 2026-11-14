//! Round 22 §2: an image too large for the decoder's memory limit decodes at a reduced size
//! with bounded memory instead of failing as if it were damaged.

use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};

use photos_media::decode::{decode_reduced, is_too_large};
use photos_media::{MediaError, MediaKind};

const LEFT: [u8; 3] = [200, 30, 40];
const RIGHT: [u8; 3] = [20, 60, 220];

/// A `w × h` RGB PNG split down the middle into two colours, written one row at a time so the
/// test never holds the image either.
fn split_png(dir: &Path, w: u32, h: u32, interlaced: bool) -> PathBuf {
    let p = dir.join(format!("split-{w}x{h}{}.png", if interlaced { "-i" } else { "" }));
    let file = BufWriter::new(std::fs::File::create(&p).unwrap());
    let mut enc = png::Encoder::new(file, w, h);
    enc.set_color(png::ColorType::Rgb);
    enc.set_depth(png::BitDepth::Eight);
    enc.set_compression(png::Compression::Fast);
    let row: Vec<u8> = (0..w).flat_map(|x| if x < w / 2 { LEFT } else { RIGHT }).collect();
    if interlaced {
        drop(enc);
        std::fs::write(&p, adam7_png(w, h, &row)).unwrap();
        return p;
    }
    let mut wr = enc.write_header().unwrap().into_stream_writer().unwrap();
    for _ in 0..h {
        wr.write_all(&row).unwrap();
    }
    wr.finish().unwrap();
    p
}

/// An interlaced RGB PNG of `h` copies of `row`, made by hand: the `png` encoder writes no
/// Adam7. Filter 0 on every line, the zlib stream in stored (uncompressed) blocks.
fn adam7_png(w: u32, h: u32, row: &[u8]) -> Vec<u8> {
    const PASSES: [(u32, u32, u32, u32); 7] = [(0, 0, 8, 8), (4, 0, 8, 8), (0, 4, 4, 8), (2, 0, 4, 4), (0, 2, 2, 4), (1, 0, 2, 2), (0, 1, 1, 2)];
    let mut raw = Vec::new();
    for (x0, y0, dx, dy) in PASSES {
        if x0 >= w || y0 >= h {
            continue;
        }
        // Each line: filter type 0, then the pass's pixels.
        let line: Vec<u8> = std::iter::once(0).chain((x0..w).step_by(dx as usize).flat_map(|x| row[x as usize * 3..x as usize * 3 + 3].iter().copied())).collect();
        for _ in (y0..h).step_by(dy as usize) {
            raw.extend_from_slice(&line);
        }
    }
    let mut z = vec![0x78, 0x01];
    let chunks: Vec<&[u8]> = raw.chunks(65535).collect();
    for (i, c) in chunks.iter().enumerate() {
        z.push(u8::from(i == chunks.len() - 1));
        z.extend_from_slice(&(c.len() as u16).to_le_bytes());
        z.extend_from_slice(&(!(c.len() as u16)).to_le_bytes());
        z.extend_from_slice(c);
    }
    let (mut a, mut b) = (1u32, 0u32);
    for &x in &raw {
        a = (a + x as u32) % 65521;
        b = (b + a) % 65521;
    }
    z.extend_from_slice(&((b << 16) | a).to_be_bytes());
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&w.to_be_bytes());
    ihdr.extend_from_slice(&h.to_be_bytes());
    ihdr.extend_from_slice(&[8, 2, 0, 0, 1]);
    let mut out = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    for (kind, data) in [(b"IHDR", ihdr), (b"IDAT", z), (b"IEND", Vec::new())] {
        out.extend_from_slice(&(data.len() as u32).to_be_bytes());
        let start = out.len();
        out.extend_from_slice(kind);
        out.extend_from_slice(&data);
        let crc = crc32(&out[start..]);
        out.extend_from_slice(&crc.to_be_bytes());
    }
    out
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut c = !0u32;
    for &b in bytes {
        c ^= b as u32;
        for _ in 0..8 {
            c = if c & 1 != 0 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 };
        }
    }
    !c
}

fn near(a: [u8; 3], b: [u8; 3]) -> bool {
    a.iter().zip(b).all(|(x, y)| x.abs_diff(y) <= 1)
}

fn assert_split(img: &image::DynamicImage) {
    let rgb = img.to_rgb8();
    let (w, h) = rgb.dimensions();
    for y in [0, h / 2, h - 1] {
        // Clear of the edge, where a resampling filter may ring.
        for x in [0, w / 4, w / 2 - 8] {
            assert!(near(rgb.get_pixel(x, y).0, LEFT), "left at {x},{y}: {:?}", rgb.get_pixel(x, y).0);
        }
        for x in [w / 2 + 8, 3 * w / 4, w - 1] {
            assert!(near(rgb.get_pixel(x, y).0, RIGHT), "right at {x},{y}: {:?}", rgb.get_pixel(x, y).0);
        }
    }
}

/// 16000 × 12000 × 3 = 576 MB, just over `image`'s 512 MiB default allocation limit.
#[test]
fn a_png_over_the_limit_decodes_reduced() {
    let dir = tempfile::tempdir().unwrap();
    let p = split_png(dir.path(), 16000, 12000, false);
    let r = decode_reduced(&p, MediaKind::Image, 4096, 512 << 20).expect("reduced");
    assert_eq!((r.real_width, r.real_height), (16000, 12000));
    assert_eq!(r.image.width().max(r.image.height()), 4096);
    assert_eq!((r.image.width(), r.image.height()), (4096, 3072));
    assert_split(&r.image);
}

#[test]
fn is_too_large_knows_the_limit_error() {
    let dir = tempfile::tempdir().unwrap();
    let p = split_png(dir.path(), 16000, 12000, false);
    let plain = image::ImageReader::open(&p).unwrap().with_guessed_format().unwrap().decode();
    let e = MediaError::from(plain.expect_err("over the default limit"));
    assert!(is_too_large(&e), "{e}");
    assert!(!is_too_large(&MediaError::Decode("unexpected end of file".into())));
}

#[test]
fn a_small_image_is_returned_whole_when_under_max_edge() {
    let dir = tempfile::tempdir().unwrap();
    let p = split_png(dir.path(), 800, 600, false);
    let r = decode_reduced(&p, MediaKind::Image, 4096, 512 << 20).unwrap();
    assert_eq!((r.image.width(), r.image.height()), (800, 600));
    assert_eq!((r.real_width, r.real_height), (800, 600));
    assert_split(&r.image);
}

#[test]
fn an_interlaced_png_falls_back() {
    let dir = tempfile::tempdir().unwrap();
    let p = split_png(dir.path(), 1200, 900, true);
    let r = decode_reduced(&p, MediaKind::Image, 600, 512 << 20).unwrap();
    assert_eq!((r.image.width(), r.image.height()), (600, 450));
    assert_eq!((r.real_width, r.real_height), (1200, 900));
    assert_split(&r.image);
}

/// A real oversized file, read only: `LUMEN_OVERSIZED=<path> cargo test -p photos-media --test
/// reduced -- --ignored`.
#[test]
#[ignore]
fn a_real_oversized_file_decodes_reduced() {
    let p = PathBuf::from(std::env::var("LUMEN_OVERSIZED").expect("LUMEN_OVERSIZED"));
    let t = std::time::Instant::now();
    let r = decode_reduced(&p, MediaKind::Image, 4096, 512 << 20).expect("reduced");
    eprintln!("{} × {} → {} × {} in {:?}", r.real_width, r.real_height, r.image.width(), r.image.height(), t.elapsed());
}

/// A JPEG whose frame header claims 60000 × 60000 (10.8 GB of RGB): refused before anything is
/// allocated, as an error the caller can tell from a damaged file.
#[test]
fn over_budget_is_an_error_not_an_abort() {
    let dir = tempfile::tempdir().unwrap();
    let small = image::RgbImage::from_pixel(16, 16, image::Rgb(LEFT));
    let mut bytes = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut bytes, 80).encode_image(&small).unwrap();
    // SOF0: FF C0, length (2), precision (1), height (2), width (2).
    let sof = bytes.windows(2).position(|w| w == [0xFF, 0xC0]).expect("baseline frame header");
    bytes[sof + 5..sof + 7].copy_from_slice(&60000u16.to_be_bytes());
    bytes[sof + 7..sof + 9].copy_from_slice(&60000u16.to_be_bytes());
    let p = dir.path().join("huge.jpg");
    std::fs::write(&p, bytes).unwrap();
    let e = decode_reduced(&p, MediaKind::Image, 4096, 64 << 20).expect_err("over budget");
    assert!(is_too_large(&e), "{e}");
}
