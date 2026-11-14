//! One RAW file measured, one JSON line out, through `photos-media`'s public API only, so the
//! same program runs before and after RAW decoding moved to `photos-raw` (Round 17) and its
//! hashes prove the results did not change. `docs/RAW-FORMATS.md` says how it was used.
//!
//! `cargo run --release -p photos-media --example raw_sweep -- <file>`
//!
//! One file per process: a driver runs it with a timeout, so an abort or a hang is a result.
//! It only reads the file.

use std::path::Path;
use std::time::Instant;

use image::DynamicImage;
use photos_media::develop::{raw, Linear};
use photos_media::{decode_preview, read_metadata, MediaKind};

/// The edge both images are compared at: enough to tell the same photo from a garbled one,
/// small enough that sharpening and noise reduction do not count.
const EDGE: u32 = 64;

fn esc(s: &str) -> String {
    s.chars()
        .flat_map(|c| match c {
            '"' => vec!['\\', '"'],
            '\\' => vec!['\\', '\\'],
            c if (c as u32) < 0x20 => vec![' '],
            c => vec![c],
        })
        .collect()
}

fn opt(s: &Option<String>) -> String {
    s.as_deref().map(|v| format!("\"{}\"", esc(v))).unwrap_or_else(|| "null".into())
}

fn hash_f32(l: &Linear) -> String {
    let mut h = blake3::Hasher::new();
    for p in &l.data {
        for v in p {
            h.update(&v.to_bits().to_le_bytes());
        }
    }
    h.finalize().to_hex()[..16].to_string()
}

fn hash_image(i: &DynamicImage) -> String {
    let mut h = blake3::Hasher::new();
    h.update(format!("{:?}", i.color()).as_bytes());
    h.update(i.as_bytes());
    h.finalize().to_hex()[..16].to_string()
}

fn fit(w: u32, h: u32) -> (u32, u32) {
    if w >= h {
        (EDGE, (EDGE * h / w.max(1)).max(1))
    } else {
        ((EDGE * w / h.max(1)).max(1), EDGE)
    }
}

/// (stops, luma correlation, mean chroma distance) of two images of one frame. Sensor data
/// after the base look against the camera's JPEG: stops says how far apart their brightness
/// is, correlation whether they show the same picture (a garbled decode falls far below 0.8)
/// and chroma whether their colours agree. Tried in all four quarter-turn poses of the
/// preview, because some cameras store it turned.
fn compare(sensor: &Linear, preview: &DynamicImage) -> (f64, f64, f64, u32) {
    let (w, h) = fit(sensor.width, sensor.height);
    let a = sensor.resize(w, h);
    let mut best = (0.0, -2.0, 0.0, 0);
    for turn in 0..4u32 {
        let p = match turn {
            0 => preview.clone(),
            1 => preview.rotate90(),
            2 => preview.rotate180(),
            _ => preview.rotate270(),
        };
        let aspect = |x: f64, y: f64| (x / y.max(1.0)).ln();
        if (aspect(p.width() as f64, p.height() as f64) - aspect(sensor.width as f64, sensor.height as f64)).abs() > 0.35 {
            continue;
        }
        let b = Linear::from_dynamic(&p).resize(w, h);
        let (mut sa, mut sb, mut saa, mut sbb, mut sab, mut chroma) = (0.0, 0.0, 0.0, 0.0, 0.0, 0.0);
        let n = a.data.len().max(1) as f64;
        // Compared in a perceptual-ish space, so the shadows count as much as the highlights.
        let tone = |v: f32| (v.max(0.0) as f64).powf(1.0 / 2.2);
        for (x, y) in a.data.iter().zip(&b.data) {
            let la = 0.2126 * x[0] as f64 + 0.7152 * x[1] as f64 + 0.0722 * x[2] as f64;
            let lb = 0.2126 * y[0] as f64 + 0.7152 * y[1] as f64 + 0.0722 * y[2] as f64;
            let (ta, tb) = (la.max(0.0).powf(1.0 / 2.2), lb.max(0.0).powf(1.0 / 2.2));
            sa += ta;
            sb += tb;
            saa += ta * ta;
            sbb += tb * tb;
            sab += ta * tb;
            chroma += ((tone(x[0]) - tone(x[1])) - (tone(y[0]) - tone(y[1]))).abs() + ((tone(x[2]) - tone(x[1])) - (tone(y[2]) - tone(y[1]))).abs();
        }
        let cov = sab / n - sa / n * sb / n;
        let (va, vb) = ((saa / n - (sa / n).powi(2)).max(1e-12), (sbb / n - (sb / n).powi(2)).max(1e-12));
        let corr = cov / (va * vb).sqrt();
        if corr > best.1 {
            let mean = |l: &Linear| l.data.iter().map(|p| 0.2126 * p[0] as f64 + 0.7152 * p[1] as f64 + 0.0722 * p[2] as f64).sum::<f64>() / n;
            let stops = (mean(&a).max(1e-9) / mean(&b).max(1e-9)).log2();
            best = (stops, corr, chroma / (2.0 * n), turn);
        }
    }
    best
}

fn main() {
    // Named as Lumen's interactive pool is, so the library decodes at normal priority, the one
    // this program ran at before RAW decoding moved: the timings compare like with like.
    std::thread::Builder::new().name("lumen-render-sweep".into()).spawn(sweep).unwrap().join().unwrap();
}

fn sweep() {
    let path = std::env::args().nth(1).expect("usage: raw_sweep <file> [--no-sensor]");
    let path = Path::new(&path);
    let mut out = vec![format!("\"file\":\"{}\"", esc(&path.file_name().unwrap_or_default().to_string_lossy()))];

    let t = Instant::now();
    let m = read_metadata(path, MediaKind::Raw);
    out.push(format!(
        "\"meta\":{{\"ms\":{},\"make\":{},\"model\":{},\"width\":{},\"height\":{},\"orientation\":{},\"captured_at\":{}}}",
        t.elapsed().as_millis(),
        opt(&m.make),
        opt(&m.model),
        m.width.map(|v| v.to_string()).unwrap_or("null".into()),
        m.height.map(|v| v.to_string()).unwrap_or("null".into()),
        m.orientation,
        m.captured_at.map(|v| v.to_string()).unwrap_or("null".into()),
    ));

    let t = Instant::now();
    let preview = decode_preview(path, MediaKind::Raw);
    out.push(match &preview {
        Ok(p) => format!("\"preview\":{{\"ms\":{},\"width\":{},\"height\":{},\"hash\":\"{}\"}}", t.elapsed().as_millis(), p.width(), p.height(), hash_image(p)),
        Err(e) => format!("\"preview\":{{\"error\":\"{}\"}}", esc(&e.to_string())),
    });

    if !std::env::args().any(|a| a == "--no-sensor") {
        let t = Instant::now();
        let sensor = raw::decode(path);
        let ms = t.elapsed().as_millis();
        out.push(match &sensor {
            Ok(l) => {
                let mut s = format!("\"sensor\":{{\"ms\":{ms},\"width\":{},\"height\":{},\"hash\":\"{}\"", l.width, l.height, hash_f32(l));
                if let Ok(p) = &preview {
                    let (stops, corr, chroma, turn) = compare(l, p);
                    s += &format!(",\"stops\":{stops:.3},\"corr\":{corr:.4},\"chroma\":{chroma:.4},\"turn\":{turn}");
                }
                s + "}"
            }
            Err(e) => format!("\"sensor\":{{\"ms\":{ms},\"error\":\"{}\"}}", esc(&e.to_string())),
        });
        // Small JPEGs for a contact sheet, when a folder is given.
        if let (Ok(l), Some(dir)) = (&sensor, std::env::var_os("LUMEN_SWEEP_SHEET")) {
            let stem = path.file_stem().unwrap_or_default().to_string_lossy().to_string();
            let _ = image::DynamicImage::ImageRgb8(l.resize_long_edge(400).to_rgb8()).save(Path::new(&dir).join(format!("{stem}_s.jpg")));
            if let Ok(p) = &preview {
                let _ = p.thumbnail(400, 400).to_rgb8().save(Path::new(&dir).join(format!("{stem}_p.jpg")));
            }
        }
    }
    println!("{{{}}}", out.join(","));
}
