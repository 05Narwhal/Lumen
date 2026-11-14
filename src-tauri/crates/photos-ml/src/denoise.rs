//! Denoise (Phase 8d, develop spec §4): a restoration network run over a photo in overlapping
//! tiles, blended with raised-cosine weights so no seam shows. DirectML on Windows when it
//! works, the CPU otherwise. Buffers in, buffers out: `rgb` is packed row-major, sRGB-encoded,
//! 0–1.

use std::path::{Path, PathBuf};

use ort::session::Session;
use ort::value::Tensor;

pub use crate::manifest::DenoiseSpec;
use crate::manifest::ModelEntry;
use crate::MlError;

/// A rectangle of the image, in pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Tile {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

/// Starts along one axis: every `tile − overlap`, the last one moved back inside the image
/// rather than shrunk. A side shorter than a tile is one span of its own length.
fn spans(len: u32, tile: u32, overlap: u32) -> Vec<(u32, u32)> {
    if len <= tile {
        return vec![(0, len)];
    }
    let stride = (tile - overlap).max(1);
    let mut out = Vec::new();
    let mut start = 0;
    loop {
        if start + tile >= len {
            out.push((len - tile, tile));
            break;
        }
        out.push((start, tile));
        start += stride;
    }
    out
}

/// The tiles covering a `width × height` image, row by row.
pub fn tiles(width: u32, height: u32, tile: u32, overlap: u32) -> Vec<Tile> {
    let xs = spans(width, tile, overlap);
    spans(height, tile, overlap).into_iter().flat_map(|(y, h)| xs.iter().map(move |&(x, w)| Tile { x, y, w, h })).collect()
}

/// A raised-cosine ramp over `overlap` pixels from an edge that meets another tile; never 0,
/// so every pixel keeps some weight.
fn ramp(d: u32, overlap: u32) -> f32 {
    if overlap == 0 || d >= overlap {
        return 1.0;
    }
    0.5 - 0.5 * (std::f32::consts::PI * (d as f32 + 0.5) / overlap as f32).cos()
}

/// A tile's blending weight at image pixel `(x, y)`: ramps on the sides that border another
/// tile, 1 on the image's own edges and inside.
pub fn feather(t: &Tile, width: u32, height: u32, overlap: u32) -> impl Fn(u32, u32) -> f32 {
    let t = *t;
    move |x, y| {
        let mut w = 1f32;
        if t.x > 0 {
            w = w.min(ramp(x - t.x, overlap));
        }
        if t.x + t.w < width {
            w = w.min(ramp(t.x + t.w - 1 - x, overlap));
        }
        let mut v = 1f32;
        if t.y > 0 {
            v = v.min(ramp(y - t.y, overlap));
        }
        if t.y + t.h < height {
            v = v.min(ramp(t.y + t.h - 1 - y, overlap));
        }
        w * v
    }
}

/// Index `i` reflected into `0..n` (a pad past a very small tile clamps).
fn reflect(i: u32, n: u32) -> u32 {
    if i < n {
        i
    } else {
        (2 * n).saturating_sub(2 + i).min(n - 1)
    }
}

fn round_up(v: u32, m: u32) -> u32 {
    v.div_ceil(m.max(1)) * m.max(1)
}

/// The network over one padded CHW tile: `(values, width, height)` in, the same shape out.
pub type Net<'a> = dyn FnMut(&[f32], u32, u32) -> Result<Vec<f32>, MlError> + 'a;

/// `run`'s body with the network as a closure (`net(chw, width, height)` on a tile padded to
/// the network's multiple, returning the same shape), so tiling is testable without a model.
/// `progress(done, total)` after each tile; returning false cancels with an error, never a
/// partial image.
pub fn run_with(
    rgb: &[f32],
    width: u32,
    height: u32,
    spec: &DenoiseSpec,
    net: &mut Net<'_>,
    progress: &mut dyn FnMut(usize, usize) -> bool,
) -> Result<Vec<f32>, MlError> {
    let (w, h) = (width as usize, height as usize);
    if rgb.len() != w * h * 3 {
        return Err(MlError::Shape(format!("{} values for {width}×{height}", rgb.len())));
    }
    let all = tiles(width, height, spec.tile, spec.overlap);
    let mut acc = vec![0f32; w * h * 3];
    let mut weight = vec![0f32; w * h];
    for (done, t) in all.iter().enumerate() {
        let (pw, ph) = (round_up(t.w, spec.multiple), round_up(t.h, spec.multiple));
        let plane = (pw * ph) as usize;
        let mut chw = vec![0f32; 3 * plane];
        for py in 0..ph {
            let sy = (t.y + reflect(py, t.h)) as usize;
            for px in 0..pw {
                let sx = (t.x + reflect(px, t.w)) as usize;
                let s = (sy * w + sx) * 3;
                let d = (py * pw + px) as usize;
                for c in 0..3 {
                    chw[c * plane + d] = rgb[s + c];
                }
            }
        }
        let out = net(&chw, pw, ph)?;
        if out.len() != chw.len() {
            return Err(MlError::Shape(format!("{} values back for a {pw}×{ph} tile", out.len())));
        }
        let wt = feather(t, width, height, spec.overlap);
        for ty in 0..t.h {
            for tx in 0..t.w {
                let (x, y) = (t.x + tx, t.y + ty);
                let k = wt(x, y);
                let i = y as usize * w + x as usize;
                let d = (ty * pw + tx) as usize;
                weight[i] += k;
                for c in 0..3 {
                    acc[i * 3 + c] += k * out[c * plane + d];
                }
            }
        }
        if !progress(done + 1, all.len()) {
            return Err(MlError::Runtime("cancelled".into()));
        }
    }
    for (i, k) in weight.iter().enumerate() {
        for c in 0..3 {
            acc[i * 3 + c] = (acc[i * 3 + c] / k.max(1e-12)).clamp(0.0, 1.0);
        }
    }
    Ok(acc)
}

/// One way of running the network on a padded CHW tile.
pub trait Backend {
    fn run(&mut self, chw: &[f32], width: u32, height: u32) -> Result<Vec<f32>, MlError>;
}

struct OrtBackend(Session);

impl Backend for OrtBackend {
    fn run(&mut self, chw: &[f32], width: u32, height: u32) -> Result<Vec<f32>, MlError> {
        let t = Tensor::from_array(([1usize, 3, height as usize, width as usize], chw.to_vec()))?;
        let out = self.0.run(ort::inputs!["image" => t])?;
        let (_, data) = out["denoised"].try_extract_tensor::<f32>()?;
        Ok(data.to_vec())
    }
}

fn cpu_session(path: &Path) -> Result<Box<dyn Backend>, MlError> {
    let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).clamp(1, 8);
    Ok(Box::new(OrtBackend(Session::builder()?.with_intra_threads(threads)?.commit_from_file(path)?)))
}

#[cfg(windows)]
fn gpu_session(path: &Path) -> Result<Box<dyn Backend>, MlError> {
    use ort::execution_providers::DirectMLExecutionProvider;
    // DirectML wants sequential execution and no memory pattern (ONNX Runtime's own advice).
    let s = Session::builder()?
        .with_parallel_execution(false)?
        .with_memory_pattern(false)?
        .with_execution_providers([DirectMLExecutionProvider::default().build().error_on_failure()])?
        .commit_from_file(path)?;
    Ok(Box::new(OrtBackend(s)))
}

#[cfg(not(windows))]
fn gpu_session(_: &Path) -> Result<Box<dyn Backend>, MlError> {
    Err(MlError::Missing("no GPU provider on this platform".into()))
}

type MakeCpu = Box<dyn FnMut() -> Result<Box<dyn Backend>, MlError>>;

/// The denoise model, open on the GPU (DirectML) or the CPU. If the GPU cannot open the
/// model, or fails on any tile, the CPU takes over once — the GPU session is dropped
/// first, so two are never alive at once.
pub struct Denoiser {
    spec: DenoiseSpec,
    backend: Option<Box<dyn Backend>>,
    on_gpu: bool,
    make_cpu: MakeCpu,
}

impl Denoiser {
    pub fn open(path: &Path, entry: &ModelEntry, gpu: bool) -> Result<Self, MlError> {
        let spec = entry.denoise.clone().ok_or_else(|| MlError::Missing(format!("{} has no denoise spec", entry.id)))?;
        let owned: PathBuf = path.to_path_buf();
        let gpu = if gpu {
            match gpu_session(path) {
                Ok(s) => Some(s),
                Err(e) => {
                    tracing::warn!("denoise: GPU unavailable, using the CPU: {e}");
                    None
                }
            }
        } else {
            None
        };
        Self::with_backends(spec, gpu, Box::new(move || cpu_session(&owned)))
    }

    /// `open` with the backends given: the GPU's (if any) and how to build the CPU's.
    pub fn with_backends(spec: DenoiseSpec, gpu: Option<Box<dyn Backend>>, mut make_cpu: MakeCpu) -> Result<Self, MlError> {
        let on_gpu = gpu.is_some();
        let backend = match gpu {
            Some(g) => g,
            None => make_cpu()?,
        };
        Ok(Self { spec, backend: Some(backend), on_gpu, make_cpu })
    }

    pub fn on_gpu(&self) -> bool {
        self.on_gpu
    }

    /// Denoises `rgb` (packed, sRGB-encoded 0–1). See `run_with`.
    pub fn run(&mut self, rgb: &[f32], width: u32, height: u32, progress: &mut dyn FnMut(usize, usize) -> bool) -> Result<Vec<f32>, MlError> {
        let spec = self.spec.clone();
        let (backend, on_gpu, make_cpu) = (&mut self.backend, &mut self.on_gpu, &mut self.make_cpu);
        run_with(
            rgb,
            width,
            height,
            &spec,
            &mut |chw, w, h| {
                let b = backend.as_mut().ok_or_else(|| MlError::Missing("denoise session".into()))?;
                let result = b.run(chw, w, h);
                // The GPU failing on any tile (a driver reset, its memory full) hands this tile
                // and the rest to the CPU, once: the tiles done so far are already blended.
                if result.is_ok() || !*on_gpu {
                    return result;
                }
                if let Err(e) = &result {
                    tracing::warn!("denoise: the GPU failed, the CPU takes over: {e}");
                }
                *backend = None;
                *on_gpu = false;
                let mut cpu = make_cpu()?;
                let out = cpu.run(chw, w, h);
                *backend = Some(cpu);
                out
            },
            progress,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use std::rc::Rc;

    fn spec() -> DenoiseSpec {
        DenoiseSpec { tile: 512, overlap: 32, multiple: 16, input: "srgb".into() }
    }

    #[test]
    fn tiles_cover_every_pixel_and_stay_inside() {
        for (w, h) in [(1, 1), (100, 37), (512, 512), (513, 512), (4032, 3024)] {
            let ts = tiles(w, h, 512, 32);
            let mut hit = vec![false; (w * h) as usize];
            for t in &ts {
                assert!(t.x + t.w <= w && t.y + t.h <= h);
                for y in t.y..t.y + t.h {
                    for x in t.x..t.x + t.w {
                        hit[(y * w + x) as usize] = true;
                    }
                }
            }
            assert!(hit.iter().all(|&b| b), "{w}×{h}");
        }
    }

    #[test]
    fn feathered_weights_never_vanish_across_a_seam() {
        let (w, h) = (1000, 600);
        let ts = tiles(w, h, 512, 32);
        for (x, y) in [(470, 300), (488, 300), (500, 480), (999, 599), (0, 0)] {
            let sum: f32 = ts.iter().filter(|t| x >= t.x && x < t.x + t.w && y >= t.y && y < t.y + t.h).map(|t| feather(t, w, h, 32)(x, y)).sum();
            assert!(sum > 0.0, "({x}, {y})");
        }
    }

    #[test]
    fn blending_an_identity_network_reproduces_the_image() {
        let (w, h) = (700, 530);
        let img: Vec<f32> = (0..w * h * 3).map(|i| ((u64::from(i) * 7919) % 1000) as f32 / 1000.0).collect();
        let out = run_with(&img, w, h, &spec(), &mut |t: &[f32], _, _| Ok(t.to_vec()), &mut |_, _| true).unwrap();
        let max = img.iter().zip(&out).map(|(a, b)| (a - b).abs()).fold(0.0, f32::max);
        assert!(max < 1e-5, "{max}");
    }

    #[test]
    fn tiles_reach_the_network_padded_to_its_multiple() {
        let img = vec![0.5; 100 * 37 * 3];
        let mut sizes = vec![];
        run_with(
            &img,
            100,
            37,
            &spec(),
            &mut |t: &[f32], w, h| {
                sizes.push((w, h));
                assert_eq!(t.len(), (3 * w * h) as usize);
                Ok(t.to_vec())
            },
            &mut |_, _| true,
        )
        .unwrap();
        assert_eq!(sizes, vec![(112, 48)]);
    }

    #[test]
    fn every_tile_reaches_the_net_at_one_shape() {
        // A new shape makes DirectML build a new graph, so edge tiles must not shrink.
        let spec = DenoiseSpec { tile: 256, overlap: 32, multiple: 16, input: "srgb".into() };
        let (w, h) = (1000u32, 700u32);
        let rgb = vec![0.5f32; (w * h * 3) as usize];
        let mut shapes = std::collections::BTreeSet::new();
        let out = run_with(
            &rgb,
            w,
            h,
            &spec,
            &mut |chw: &[f32], pw, ph| {
                shapes.insert((pw, ph));
                Ok(chw.to_vec())
            },
            &mut |_, _| true,
        )
        .unwrap();
        assert_eq!(shapes.len(), 1, "{shapes:?}");
        assert!(out.iter().all(|v| (v - 0.5).abs() < 1e-5), "padding never leaks into the result");
    }

    #[test]
    fn a_cancelled_run_returns_an_error_not_a_partial_image() {
        let img = vec![0.5; 1200 * 900 * 3];
        let r = run_with(&img, 1200, 900, &spec(), &mut |t: &[f32], _, _| Ok(t.to_vec()), &mut |done, _| done < 2);
        assert!(r.is_err());
    }

    /// A backend that counts its runs and fails every one when told to.
    struct Fake {
        fail: bool,
        runs: Rc<Cell<usize>>,
    }

    impl Backend for Fake {
        fn run(&mut self, chw: &[f32], _: u32, _: u32) -> Result<Vec<f32>, MlError> {
            self.runs.set(self.runs.get() + 1);
            if self.fail {
                Err(MlError::Runtime("device lost".into()))
            } else {
                Ok(chw.to_vec())
            }
        }
    }

    #[test]
    fn a_failing_first_gpu_tile_falls_back_to_the_cpu_once() {
        let (gpu_runs, cpu_runs, built) = (Rc::new(Cell::new(0)), Rc::new(Cell::new(0)), Rc::new(Cell::new(0)));
        let gpu = Box::new(Fake { fail: true, runs: gpu_runs.clone() });
        let (cr, b) = (cpu_runs.clone(), built.clone());
        let mut d = Denoiser::with_backends(spec(), Some(gpu), Box::new(move || {
            b.set(b.get() + 1);
            Ok(Box::new(Fake { fail: false, runs: cr.clone() }) as Box<dyn Backend>)
        }))
        .unwrap();
        assert!(d.on_gpu());
        let img = vec![0.25; 1000 * 600 * 3];
        let out = d.run(&img, 1000, 600, &mut |_, _| true).unwrap();
        assert!(!d.on_gpu());
        assert_eq!((gpu_runs.get(), built.get()), (1, 1), "one GPU try, one CPU session");
        assert_eq!(cpu_runs.get(), tiles(1000, 600, 512, 32).len());
        assert!(out.iter().all(|v| (v - 0.25).abs() < 1e-5));
    }

    #[test]
    fn a_gpu_that_fails_mid_run_hands_the_rest_to_the_cpu_once() {
        struct Late(usize);
        impl Backend for Late {
            fn run(&mut self, chw: &[f32], _: u32, _: u32) -> Result<Vec<f32>, MlError> {
                self.0 += 1;
                if self.0 > 2 { Err(MlError::Runtime("device removed".into())) } else { Ok(chw.to_vec()) }
            }
        }
        let built = Rc::new(Cell::new(0));
        let b = built.clone();
        let mut d = Denoiser::with_backends(spec(), Some(Box::new(Late(0))), Box::new(move || {
            b.set(b.get() + 1);
            Ok(Box::new(Fake { fail: false, runs: Rc::new(Cell::new(0)) }) as Box<dyn Backend>)
        }))
        .unwrap();
        let out = d.run(&vec![0.5; 1000 * 600 * 3], 1000, 600, &mut |_, _| true).unwrap();
        assert!(!d.on_gpu());
        assert_eq!(built.get(), 1);
        assert!(out.iter().all(|v| (v - 0.5).abs() < 1e-5));
    }

    #[test]
    fn a_cpu_failure_is_an_error() {
        let mut d = Denoiser::with_backends(spec(), None, Box::new(|| Ok(Box::new(Fake { fail: true, runs: Rc::new(Cell::new(0)) }) as Box<dyn Backend>))).unwrap();
        assert!(d.run(&vec![0.5; 100 * 60 * 3], 100, 60, &mut |_, _| true).is_err());
    }
}
