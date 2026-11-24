//! The model side of API 1.1 (Round 24 §8.4): downloads of a plugin's `[[models]]` with their
//! blake3 checked, and the vectors files (`kind = "vectors"`, or one a plugin wrote to its own
//! storage) whose top-k runs here so no plugin holds a big matrix in wasm.
//!
//! Vectors file, little-endian: magic `LWVG`, u32 version (1), u32 rows, u32 dim, u32 dtype
//! (1 = f16), then `rows × dim` f16, then payload rows of f32. The payload width is not in the
//! header: it is what is left of the file, divided by `rows × 4`.

use std::cmp::Ordering;
use std::collections::HashMap;
use std::fs::File;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use image::imageops::FilterType;
use image::DynamicImage;
use photos_ml::onnx_run::{NamedTensor, OnnxModel};
use photos_models::store::part_path;
use photos_plugins::host::{ModelHost, Neighbour, Tensor, VectorSource};
use photos_plugins::manifest::{InputKind, InputSpec, Layout, ModelDecl, ModelKind};
use rayon::prelude::*;

const MAGIC: &[u8; 4] = b"LWVG";
const HEADER: usize = 20;
const DTYPE_F16: u32 = 1;

/// Where a plugin's downloaded models live: `<app data>/plugins/<id>/models/`.
pub fn models_dir(plugin_data_dir: &Path) -> PathBuf {
    plugin_data_dir.join("models")
}

/// A vectors file in memory: unit-length rows (f16 as stored) and their payload rows.
pub struct Vectors {
    rows: usize,
    dim: usize,
    payload_dim: usize,
    /// `rows × dim` f16 bit patterns.
    matrix: Vec<u16>,
    payload: Vec<f32>,
}

impl Vectors {
    pub fn open(path: &Path) -> Result<Vectors, String> {
        let mut bytes = Vec::new();
        File::open(path).and_then(|mut f| f.read_to_end(&mut bytes)).map_err(|e| format!("can't read {}: {e}", path.display()))?;
        Vectors::parse(&bytes)
    }

    #[allow(clippy::chunks_exact_to_as_chunks)]
    pub fn parse(b: &[u8]) -> Result<Vectors, String> {
        let bad = |why: &str| Err(format!("not a vectors file: {why}"));
        if b.len() < HEADER || &b[..4] != MAGIC {
            return bad("no LWVG header");
        }
        let word = |i: usize| u32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]]);
        let (version, rows, dim, dtype) = (word(4), word(8) as usize, word(12) as usize, word(16));
        if version != 1 || dtype != DTYPE_F16 {
            return bad("unsupported version or dtype");
        }
        let matrix_bytes = rows.checked_mul(dim).and_then(|n| n.checked_mul(2)).ok_or("not a vectors file: too large")?;
        let rest = b.len().checked_sub(HEADER + matrix_bytes).ok_or("not a vectors file: truncated")?;
        if rows == 0 || dim == 0 {
            return bad("empty");
        }
        if rest % (rows * 4) != 0 {
            return bad("payload doesn't divide into rows");
        }
        let matrix = b[HEADER..HEADER + matrix_bytes].chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
        let payload = b[HEADER + matrix_bytes..].chunks_exact(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect();
        Ok(Vectors { rows, dim, payload_dim: rest / (rows * 4), matrix, payload })
    }

    pub fn rows(&self) -> usize {
        self.rows
    }

    /// The `k` rows with the highest dot product with `query`, best first.
    pub fn nearest(&self, query: &[f32], k: usize) -> Result<Vec<Neighbour>, String> {
        if query.len() != self.dim {
            return Err(format!("query has {} values, the vectors have {}", query.len(), self.dim));
        }
        let mut scores: Vec<(f32, u32)> = self
            .matrix
            .par_chunks(self.dim)
            .enumerate()
            .map(|(row, r)| (r.iter().zip(query).map(|(&h, q)| f16_to_f32(h) * q).sum::<f32>(), row as u32))
            .collect();
        let k = k.min(scores.len());
        let by_score = |a: &(f32, u32), b: &(f32, u32)| b.0.partial_cmp(&a.0).unwrap_or(Ordering::Equal).then(a.1.cmp(&b.1));
        if k > 0 && k < scores.len() {
            scores.select_nth_unstable_by(k - 1, by_score);
        }
        scores.truncate(k);
        scores.sort_by(by_score);
        Ok(scores
            .into_iter()
            .map(|(score, row)| {
                let at = row as usize * self.payload_dim;
                Neighbour { row, score, payload: self.payload[at..at + self.payload_dim].to_vec() }
            })
            .collect())
    }
}

/// IEEE half to single, without a dependency.
pub fn f16_to_f32(h: u16) -> f32 {
    let (sign, exp, frac) = (u32::from(h >> 15), u32::from((h >> 10) & 0x1f), u32::from(h & 0x3ff));
    let bits = match (exp, frac) {
        (0, 0) => sign << 31,
        (0, _) => {
            // Subnormal: normalise.
            let shift = frac.leading_zeros() - 21;
            (sign << 31) | ((113 - shift) << 23) | (((frac << shift) & 0x3ff) << 13)
        }
        (0x1f, _) => (sign << 31) | 0x7f80_0000 | (frac << 13),
        _ => (sign << 31) | ((exp + 112) << 23) | (frac << 13),
    };
    f32::from_bits(bits)
}

/// Single to half (round to nearest even is not needed here: tests and small files only).
#[cfg(test)]
pub fn f32_to_f16(v: f32) -> u16 {
    let b = v.to_bits();
    let sign = ((b >> 16) & 0x8000) as u16;
    let exp = ((b >> 23) & 0xff) as i32 - 127 + 15;
    let frac = b & 0x7f_ffff;
    if exp <= 0 {
        return sign;
    }
    if exp >= 0x1f {
        return sign | 0x7c00;
    }
    sign | ((exp as u16) << 10) | (frac >> 13) as u16
}

/// blake3 of a file, hex.
pub fn blake3_of(path: &Path) -> std::io::Result<String> {
    let mut f = File::open(path)?;
    let mut h = blake3::Hasher::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(h.finalize().to_hex().to_string())
}

/// Publishes a finished download: its size and blake3 must be the manifest's, then it is renamed
/// over `dest` (one volume, atomic). Anything else removes the `.part`, so a cut or tampered
/// download never leaves a model that looks installed.
pub fn publish_download(part: &Path, dest: &Path, size: u64, blake3_hex: &str) -> Result<(), String> {
    let ok = std::fs::metadata(part).is_ok_and(|m| m.len() == size) && blake3_of(part).is_ok_and(|h| h.eq_ignore_ascii_case(blake3_hex));
    if !ok {
        let _ = std::fs::remove_file(part);
        return Err("The model's download didn't match its manifest and was discarded".into());
    }
    std::fs::rename(part, dest).map_err(|e| format!("can't install the model: {e}"))
}

/// Downloads `url` to `dest` (`progress(done, total)` after each chunk), verified by
/// [`publish_download`]. A model already there at its size is kept: it was verified when it arrived.
pub async fn download_model(url: &str, dest: &Path, size: u64, blake3_hex: &str, mut progress: impl FnMut(u64, u64)) -> Result<(), String> {
    if std::fs::metadata(dest).is_ok_and(|m| m.len() == size) {
        return Ok(());
    }
    if let Some(dir) = dest.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let part = part_path(dest);
    let mut resp = reqwest::get(url).await.and_then(|r| r.error_for_status()).map_err(|e| format!("can't download the model: {e}"))?;
    let mut out = File::create(&part).map_err(|e| e.to_string())?;
    let mut done = 0u64;
    while let Some(chunk) = resp.chunk().await.map_err(|e| format!("the model's download stopped: {e}"))? {
        out.write_all(&chunk).map_err(|e| e.to_string())?;
        done += chunk.len() as u64;
        progress(done, size);
    }
    out.sync_all().map_err(|e| e.to_string())?;
    drop(out);
    publish_download(&part, dest, size, blake3_hex)
}

/// Where a manifest model lives once downloaded: `<data>/models/<id>.onnx` (`.bin` for vectors).
pub fn model_file(plugin_data_dir: &Path, decl: &ModelDecl) -> PathBuf {
    let ext = if decl.kind == ModelKind::Vectors { "bin" } else { "onnx" };
    models_dir(plugin_data_dir).join(format!("{}.{ext}", decl.id))
}

/// A photo as the model should see it, from `decode_display_full` (so orientation is right).
pub type AssetImage = Arc<dyn Fn(u64) -> Result<DynamicImage, String> + Send + Sync>;
/// A plugin's `[[models]]` and data folder, `None` when it isn't installed.
pub type ModelLookup = Arc<dyn Fn(&str) -> Option<(Vec<ModelDecl>, PathBuf)> + Send + Sync>;

/// Short side to `size` (Catmull-Rom, which is bicubic), centre crop, `(v/255 − mean) / std` per
/// channel, laid out as the manifest says. Returns the tensor's shape and values.
pub fn preprocess(img: &DynamicImage, spec: &InputSpec) -> Result<(Vec<u32>, Vec<f32>), String> {
    let size = spec.size.filter(|s| *s > 0).ok_or("the input has no size")?;
    let channel = |v: &[f32], default: f32, what: &str| -> Result<[f32; 3], String> {
        match v.len() {
            0 => Ok([default; 3]),
            1 => Ok([v[0]; 3]),
            3 => Ok([v[0], v[1], v[2]]),
            n => Err(format!("{what} has {n} values, not 3")),
        }
    };
    let (mean, std) = (channel(&spec.mean, 0.0, "mean")?, channel(&spec.std, 1.0, "std")?);
    if std.contains(&0.0) {
        return Err("std has a zero".into());
    }
    let rgb = img.to_rgb8();
    let (w, h) = rgb.dimensions();
    if w == 0 || h == 0 {
        return Err("the photo is empty".into());
    }
    let scale = size as f32 / w.min(h) as f32;
    let (nw, nh) = (((w as f32 * scale).round() as u32).max(size), ((h as f32 * scale).round() as u32).max(size));
    let resized = if (nw, nh) == (w, h) { rgb } else { image::imageops::resize(&rgb, nw, nh, FilterType::CatmullRom) };
    let (x0, y0) = ((nw - size) / 2, (nh - size) / 2);
    let s = size as usize;
    let mut out = vec![0f32; 3 * s * s];
    for y in 0..s {
        for x in 0..s {
            let p = resized.get_pixel(x0 + x as u32, y0 + y as u32);
            for c in 0..3 {
                let v = (p[c] as f32 / 255.0 - mean[c]) / std[c];
                match spec.layout.unwrap_or(Layout::Nchw) {
                    Layout::Nchw => out[c * s * s + y * s + x] = v,
                    Layout::Nhwc => out[(y * s + x) * 3 + c] = v,
                }
            }
        }
    }
    let shape = match spec.layout.unwrap_or(Layout::Nchw) {
        Layout::Nchw => vec![1, 3, size, size],
        Layout::Nhwc => vec![1, size, size, 3],
    };
    Ok((shape, out))
}

/// `models` for the app: ONNX sessions cached per (plugin, model), vectors files cached until they
/// change on disk. A model that isn't downloaded is `model_missing:` text, a state the plugin
/// reports as *Model not downloaded*.
pub struct AppModels {
    lookup: ModelLookup,
    image: AssetImage,
    gpu: bool,
    sessions: Mutex<HashMap<(String, String), OnnxModel>>,
    vectors: Mutex<HashMap<PathBuf, VectorsEntry>>,
}

/// A parsed vectors file with the modified time and length it was read at.
type VectorsEntry = (SystemTime, u64, Arc<Vectors>);

pub const MODEL_MISSING: &str = "model_missing";

impl AppModels {
    pub fn new(lookup: ModelLookup, image: AssetImage, gpu: bool) -> Self {
        Self { lookup, image, gpu, sessions: Mutex::new(HashMap::new()), vectors: Mutex::new(HashMap::new()) }
    }

    fn decl(&self, plugin: &str, model: &str) -> Result<(ModelDecl, PathBuf), String> {
        let (decls, data) = (self.lookup)(plugin).ok_or_else(|| format!("{plugin} isn't installed"))?;
        let decl = decls.into_iter().find(|d| d.id == model).ok_or_else(|| format!("the manifest has no model '{model}'"))?;
        let file = model_file(&data, &decl);
        if !file.is_file() {
            return Err(format!("{MODEL_MISSING}: '{model}' isn't downloaded"));
        }
        Ok((decl, file))
    }

    fn run_named(&self, plugin: &str, model: &str, inputs: Vec<NamedTensor>) -> Result<Vec<NamedTensor>, String> {
        let (decl, file) = self.decl(plugin, model)?;
        if decl.kind != ModelKind::Onnx {
            return Err(format!("'{model}' is a vectors model; use nearest"));
        }
        let mut sessions = self.sessions.lock().unwrap_or_else(|p| p.into_inner());
        let key = (plugin.to_string(), model.to_string());
        if !sessions.contains_key(&key) {
            let m = OnnxModel::open(&file, self.gpu).map_err(|e| e.to_string())?;
            sessions.insert(key.clone(), m);
        }
        sessions.get_mut(&key).expect("inserted above").run(inputs).map_err(|e| e.to_string())
    }

    fn vectors_at(&self, file: &Path) -> Result<Arc<Vectors>, String> {
        let meta = std::fs::metadata(file).map_err(|e| format!("{}: {e}", file.display()))?;
        let stamp = (meta.modified().unwrap_or(SystemTime::UNIX_EPOCH), meta.len());
        let mut cache = self.vectors.lock().unwrap_or_else(|p| p.into_inner());
        if let Some((m, l, v)) = cache.get(file) {
            if (*m, *l) == stamp {
                return Ok(v.clone());
            }
        }
        let v = Arc::new(Vectors::open(file)?);
        cache.insert(file.to_path_buf(), (stamp.0, stamp.1, v.clone()));
        Ok(v)
    }
}

fn to_named(t: Tensor) -> NamedTensor {
    NamedTensor { name: t.name, shape: t.shape.into_iter().map(|d| d as usize).collect(), data: t.data }
}

fn from_named(t: NamedTensor) -> Tensor {
    Tensor { name: t.name, shape: t.shape.into_iter().map(|d| d as u32).collect(), data: t.data }
}

impl ModelHost for AppModels {
    fn run(&self, plugin: &str, model: &str, inputs: Vec<Tensor>) -> Result<Vec<Tensor>, String> {
        Ok(self.run_named(plugin, model, inputs.into_iter().map(to_named).collect())?.into_iter().map(from_named).collect())
    }

    fn run_on_asset(&self, plugin: &str, model: &str, asset: u64, input: &str) -> Result<Vec<Tensor>, String> {
        let (decl, _) = self.decl(plugin, model)?;
        let spec = decl.inputs.get(input).filter(|s| s.kind == InputKind::Image).ok_or_else(|| format!("'{model}' has no image input '{input}'"))?;
        let (shape, data) = preprocess(&(self.image)(asset)?, spec)?;
        let named = NamedTensor { name: input.to_string(), shape: shape.into_iter().map(|d| d as usize).collect(), data };
        Ok(self.run_named(plugin, model, vec![named])?.into_iter().map(from_named).collect())
    }

    fn nearest(&self, plugin: &str, source: &VectorSource, query: &[f32], k: u32) -> Result<Vec<Neighbour>, String> {
        let file = match source {
            VectorSource::Model(id) => {
                let (decl, file) = self.decl(plugin, id)?;
                if decl.kind != ModelKind::Vectors {
                    return Err(format!("'{id}' isn't a vectors model"));
                }
                file
            }
            // The plugin's own storage is its data folder; a plain file name only.
            VectorSource::Storage(name) => {
                if name.is_empty() || name.contains(['/', '\\']) || name.starts_with('.') {
                    return Err(format!("'{name}' isn't a storage file name"));
                }
                let (_, data) = (self.lookup)(plugin).ok_or_else(|| format!("{plugin} isn't installed"))?;
                data.join(name)
            }
        };
        self.vectors_at(&file)?.nearest(query, k as usize)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A tiny deterministic generator: no `rand` dependency for a test.
    fn lcg(seed: &mut u64) -> f32 {
        *seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        ((*seed >> 40) as f32 / (1u64 << 24) as f32) * 2.0 - 1.0
    }

    fn vectors_file(rows: usize, dim: usize, seed: u64) -> (Vec<u8>, Vec<Vec<f32>>) {
        let mut s = seed;
        let matrix: Vec<Vec<f32>> = (0..rows).map(|_| (0..dim).map(|_| f16_to_f32(f32_to_f16(lcg(&mut s)))).collect()).collect();
        let mut b = Vec::new();
        b.extend_from_slice(MAGIC);
        for w in [1u32, rows as u32, dim as u32, DTYPE_F16] {
            b.extend_from_slice(&w.to_le_bytes());
        }
        for row in &matrix {
            for &v in row {
                b.extend_from_slice(&f32_to_f16(v).to_le_bytes());
            }
        }
        for r in 0..rows {
            b.extend_from_slice(&(r as f32).to_le_bytes());
            b.extend_from_slice(&(-(r as f32)).to_le_bytes());
        }
        (b, matrix)
    }

    #[test]
    fn nearest_topk_matches_bruteforce() {
        let (bytes, matrix) = vectors_file(1000, 16, 7);
        let v = Vectors::parse(&bytes).unwrap();
        assert_eq!(v.rows(), 1000);
        let mut s = 99;
        let query: Vec<f32> = (0..16).map(|_| lcg(&mut s)).collect();
        let mut brute: Vec<(f32, usize)> =
            matrix.iter().enumerate().map(|(i, r)| (r.iter().zip(&query).map(|(a, b)| a * b).sum(), i)).collect();
        brute.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap().then(a.1.cmp(&b.1)));
        let got = v.nearest(&query, 10).unwrap();
        assert_eq!(got.len(), 10);
        for (g, b) in got.iter().zip(&brute) {
            assert_eq!(g.row as usize, b.1);
            assert!((g.score - b.0).abs() < 1e-4);
            assert_eq!(g.payload, vec![b.1 as f32, -(b.1 as f32)]);
        }
    }

    #[test]
    fn vectors_refuses_what_is_not_a_file_of_its_own() {
        let (mut bytes, _) = vectors_file(4, 4, 1);
        assert!(Vectors::parse(&bytes[..10]).is_err());
        bytes[0] = b'X';
        assert!(Vectors::parse(&bytes).is_err());
        let (bytes, _) = vectors_file(4, 4, 1);
        assert!(Vectors::parse(&bytes[..bytes.len() - 3]).is_err());
        assert!(Vectors::parse(&bytes).unwrap().nearest(&[1.0; 3], 1).is_err());
    }

    #[test]
    fn half_floats_round_trip() {
        for v in [0.0f32, 1.0, -0.5, 0.123, 65504.0, 1e-4] {
            let back = f16_to_f32(f32_to_f16(v));
            assert!((back - v).abs() <= v.abs() * 1e-3 + 1e-7, "{v} -> {back}");
        }
        // The smallest subnormal and the largest one, from the IEEE table.
        assert_eq!(f16_to_f32(0x0001), 2f32.powi(-24));
        assert_eq!(f16_to_f32(0x03ff), 1023.0 * 2f32.powi(-24));
        assert!(f16_to_f32(0x7c00).is_infinite());
    }

    #[test]
    fn model_download_checks_blake3() {
        let dir = tempfile::tempdir().unwrap();
        let data = b"model bytes".to_vec();
        let good = blake3::hash(&data).to_hex().to_string();
        let (part, dest) = (dir.path().join("m.onnx.part"), dir.path().join("m.onnx"));
        std::fs::write(&part, &data).unwrap();
        publish_download(&part, &dest, data.len() as u64, &good).unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), data);
        assert!(!part.exists());

        // Wrong hash: discarded, nothing installed.
        let dest2 = dir.path().join("n.onnx");
        let part2 = dir.path().join("n.onnx.part");
        std::fs::write(&part2, &data).unwrap();
        assert!(publish_download(&part2, &dest2, data.len() as u64, &"0".repeat(64)).is_err());
        assert!(!part2.exists() && !dest2.exists());

        // Cut short (the size differs): discarded too.
        std::fs::write(&part2, &data[..4]).unwrap();
        assert!(publish_download(&part2, &dest2, data.len() as u64, &good).is_err());
        assert!(!part2.exists() && !dest2.exists());
    }

    fn image_spec(layout: Layout) -> InputSpec {
        InputSpec { kind: InputKind::Image, size: Some(2), mean: vec![0.5, 0.0, 0.25], std: vec![0.5, 1.0, 0.25], layout: Some(layout) }
    }

    fn two_by_two() -> DynamicImage {
        // Row-major: red, green / blue, white.
        let mut img = image::RgbImage::new(2, 2);
        for (i, px) in [[255, 0, 0], [0, 255, 0], [0, 0, 255], [255, 255, 255]].iter().enumerate() {
            img.put_pixel((i % 2) as u32, (i / 2) as u32, image::Rgb(*px));
        }
        DynamicImage::ImageRgb8(img)
    }

    #[test]
    fn run_on_asset_uses_decode_display_full_and_normalises() {
        // The host hands the model what `decode_display_full` returned (the asset loader of the
        // hosts); a 2x2 photo at size 2 is not resized, just normalised: (v/255 - mean) / std.
        let (shape, nchw) = preprocess(&two_by_two(), &image_spec(Layout::Nchw)).unwrap();
        assert_eq!(shape, vec![1, 3, 2, 2]);
        // R plane: (1-.5)/.5=1, (0-.5)/.5=-1, -1, 1. G plane: 0,1,0,1. B plane: (0-.25)/.25=-1,-1,3,3.
        let want = [1., -1., -1., 1., 0., 1., 0., 1., -1., -1., 3., 3.];
        assert_eq!(nchw, want);
        let (shape, nhwc) = preprocess(&two_by_two(), &image_spec(Layout::Nhwc)).unwrap();
        assert_eq!(shape, vec![1, 2, 2, 3]);
        assert_eq!(&nhwc[..6], &[1., 0., -1., -1., 1., -1.]);

        // A long photo is cut at its centre after the short side is brought to `size`.
        let wide = DynamicImage::ImageRgb8(image::RgbImage::from_fn(6, 2, |x, _| image::Rgb([if x < 2 { 255 } else { 0 }, 0, 0])));
        let spec = InputSpec { kind: InputKind::Image, size: Some(2), mean: vec![], std: vec![], layout: Some(Layout::Nchw) };
        let (_, v) = preprocess(&wide, &spec).unwrap();
        assert!(v[..4].iter().all(|r| *r < 0.01), "the red third is cropped away: {v:?}");

        // A model that isn't downloaded is a state.
        let dir = tempfile::tempdir().unwrap();
        let decl = ModelDecl {
            id: "image".into(),
            url: String::new(),
            blake3: String::new(),
            size: 0,
            licence: "MIT".into(),
            kind: ModelKind::Onnx,
            inputs: [("pixels".to_string(), image_spec(Layout::Nchw))].into(),
        };
        let data = dir.path().to_path_buf();
        let lookup: ModelLookup = Arc::new(move |p| (p == "jl.t").then(|| (vec![decl.clone()], data.clone())));
        let called = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let flag = called.clone();
        let host = AppModels::new(lookup, Arc::new(move |_| { flag.store(true, std::sync::atomic::Ordering::SeqCst); Ok(two_by_two()) }), false);
        let err = host.run_on_asset("jl.t", "image", 1, "pixels").unwrap_err();
        assert!(err.starts_with(MODEL_MISSING), "{err}");
        assert!(!called.load(std::sync::atomic::Ordering::SeqCst), "no photo is decoded for a missing model");
        assert!(host.run_on_asset("jl.other", "image", 1, "pixels").is_err());
    }

    #[test]
    fn nearest_reads_a_storage_file_and_refuses_paths() {
        let dir = tempfile::tempdir().unwrap();
        let (bytes, rows) = vectors_file(50, 8, 3);
        std::fs::write(dir.path().join("mine.bin"), bytes).unwrap();
        let data = dir.path().to_path_buf();
        let lookup: ModelLookup = Arc::new(move |_| Some((Vec::new(), data.clone())));
        let host = AppModels::new(lookup, Arc::new(|_| Err("no".into())), false);
        let hit = host.nearest("jl.t", &VectorSource::Storage("mine.bin".into()), &rows[7], 1).unwrap();
        assert_eq!(hit[0].row, 7);
        assert!(host.nearest("jl.t", &VectorSource::Storage("../mine.bin".into()), &rows[7], 1).is_err());
        assert!(host.nearest("jl.t", &VectorSource::Model("nope".into()), &rows[7], 1).is_err());
    }
}
