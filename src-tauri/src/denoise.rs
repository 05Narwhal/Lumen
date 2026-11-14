//! Denoise in the app (Phase 8d, develop spec §3 stage 2 and §4): the denoised base — the
//! whole photo run through the denoise model once, on demand — cached as a derivative and
//! blended into every render of a recipe that asks for it.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

use photos_core::assets;
use photos_core::edits::{Denoise, Recipe};
use photos_jobs::{CancelToken, Lane, RunningJob};
use photos_media::develop::linear::{linear_to_srgb, srgb_to_linear};
use photos_media::develop::{self, Linear};
use photos_media::store::DerivativeStore;
use photos_ml::denoise::Denoiser;
use photos_ml::manifest::{manifest, ModelEntry};
use photos_ml::MlError;
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};
use ts_rs::TS;

use crate::error::{AppError, AppResult};
use crate::events::{DenoiseFinished, DenoiseProgress, DENOISE_FINISHED, DENOISE_PROGRESS};
use crate::state::AppState;

/// Where a file is written before it is renamed into place: beside it, never read as a
/// base, and unique to this write, so two writers can never interleave in one file.
pub fn unique_part(p: &Path) -> PathBuf {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let mut name = p.file_name().unwrap_or_default().to_owned();
    name.push(format!(".{}-{n}.part", std::process::id()));
    p.with_file_name(name)
}

/// One denoise build at a time in the whole app: each holds a full-size photo several times
/// over and its own model session, and an export may ask for many at once.
static BUILDS: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// The base at `p`, when it is complete.
pub fn ready_at(p: &Path) -> Option<PathBuf> {
    p.is_file().then(|| p.to_path_buf())
}

/// Writes a base as a 16-bit sRGB PNG (spec amendment: the base is clipped to 0–1 already,
/// and 16-bit sRGB keeps ≥ 12 bits in the shadows), through a `.part` file.
pub fn write_base(p: &Path, img: &Linear) -> std::io::Result<()> {
    if let Some(dir) = p.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let raw: Vec<u16> = img.data.iter().flat_map(|px| px.map(|v| (linear_to_srgb(v).clamp(0.0, 1.0) * 65_535.0).round() as u16)).collect();
    let buf = image::ImageBuffer::<image::Rgb<u16>, _>::from_raw(img.width, img.height, raw).ok_or_else(|| std::io::Error::other("size"))?;
    let part = unique_part(p);
    if let Err(e) = buf.save_with_format(&part, image::ImageFormat::Png) {
        let _ = std::fs::remove_file(&part);
        return Err(std::io::Error::other(e));
    }
    std::fs::rename(&part, p)
}

/// Reads a base written by `write_base`.
pub fn read_base(p: &Path) -> Option<Linear> {
    image::open(p).ok().map(|img| Linear::from_dynamic(&img))
}

/// The denoise a recipe asks for.
pub fn denoise_of(recipe: &Recipe) -> Option<&Denoise> {
    recipe.develop.as_ref().and_then(|d| d.detail.denoise.as_ref())
}

/// Stage 2 on `base`: the denoised base blended in by the recipe's strength, when the recipe
/// asks for denoise and the base exists; otherwise `base` itself (never an error).
pub fn apply_denoised(base: Linear, recipe: &Recipe, denoised: Option<&Linear>) -> Linear {
    match (denoise_of(recipe), denoised) {
        (Some(d), Some(dn)) => develop::blend_denoised(&base, dn, d.strength),
        _ => base,
    }
}

/// The denoised base a recipe asks for, when it has been built.
pub fn load_for(store: &DerivativeStore, hash: &str, recipe: &Recipe) -> Option<Linear> {
    let d = denoise_of(recipe)?;
    read_base(&ready_at(&store.denoise_path(hash, &d.model))?)
}

/// What the Detail panel shows about Denoise for one photo.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DenoiseStatus {
    /// Whether Denoise can run now.
    pub available: bool,
    /// Why it cannot, in one sentence.
    pub reason: Option<String>,
    /// The model it would use (or used).
    pub model: Option<String>,
    /// The denoised base for this photo and model exists.
    pub ready: bool,
    pub running: bool,
    /// Denoise will try the graphics card.
    pub gpu: bool,
}

/// The panel's Denoise runs, each on its own `lumen-denoise` thread (Round 15: never behind
/// the job queue): how to cancel it and when it last made progress.
static RUNNING: Mutex<Option<HashMap<i64, (CancelToken, Instant)>>> = Mutex::new(None);

fn running<T>(f: impl FnOnce(&mut HashMap<i64, (CancelToken, Instant)>) -> T) -> T {
    f(RUNNING.lock().unwrap_or_else(|p| p.into_inner()).get_or_insert_with(HashMap::new))
}

/// A run's progress just moved (the watchdog ages a run from here).
fn touch(asset_id: i64) {
    running(|m| {
        if let Some((_, at)) = m.get_mut(&asset_id) {
            *at = Instant::now();
        }
    });
}

/// Denoise runs for the watchdog, each aged from its last progress (Round 15).
pub fn denoise_running() -> Vec<RunningJob> {
    running(|m| m.iter().map(|(id, (_, at))| RunningJob { key: key(*id), lane: Lane::Import, for_ms: at.elapsed().as_millis() as u64 }).collect())
}

/// How long the GPU may stay silent before Denoise stops waiting for it: the first tile
/// includes building the graph; later tiles take a fraction of a second.
pub const FIRST_TILE_GPU: Duration = Duration::from_secs(60);
pub const TILE_GPU: Duration = Duration::from_secs(20);

/// Set once a GPU run never came back: the rest of this session denoises on the CPU.
static GPU_DENOISE_BROKEN: AtomicBool = AtomicBool::new(false);

/// Opens the denoise model: on the GPU when asked and it can, else on the CPU.
pub type Open = Arc<dyn Fn(bool) -> Result<Denoiser, MlError> + Send + Sync>;

/// A finished run: the denoised pixels, whether the GPU made them, and whether the GPU
/// stopped answering on the way.
#[derive(Debug)]
pub struct Denoised {
    pub data: Vec<f32>,
    pub on_gpu: bool,
    pub gpu_timed_out: bool,
}

enum GpuMsg {
    Progress(usize, usize),
    Done(Result<(Vec<f32>, bool), MlError>),
}

/// Runs the model over `rgb`, on the GPU first when `gpu`. The GPU run happens on its own
/// thread; when it is silent for longer than `first` (before its first tile) or `tile` (after),
/// or the user cancels while it is silent, that thread is abandoned — a call stuck in the
/// driver cannot be interrupted, so it is leaked on purpose — and the run starts over on the
/// CPU here. `progress(done, total)` returning false or `cancelled()` cancels.
pub fn run_with_fallback(
    rgb: Arc<Vec<f32>>,
    width: u32,
    height: u32,
    open: Open,
    gpu: Option<(Duration, Duration)>,
    progress: &mut dyn FnMut(usize, usize) -> bool,
    cancelled: &dyn Fn() -> bool,
) -> Result<Denoised, MlError> {
    if let Some((first, tile)) = gpu {
        let stop = Arc::new(AtomicBool::new(false));
        let (tx, rx) = mpsc::channel();
        let (o, pixels, s) = (open.clone(), rgb.clone(), stop.clone());
        let spawned = std::thread::Builder::new().name("lumen-denoise-gpu".into()).spawn(move || {
            let run = || -> Result<(Vec<f32>, bool), MlError> {
                let mut d = o(true)?;
                let data = d.run(&pixels, width, height, &mut |done, total| {
                    let _ = tx.send(GpuMsg::Progress(done, total));
                    !s.load(Ordering::SeqCst)
                })?;
                Ok((data, d.on_gpu()))
            };
            let result = run();
            let _ = tx.send(GpuMsg::Done(result));
        });
        if let Err(e) = spawned {
            return Err(MlError::Runtime(format!("denoise thread: {e}")));
        }
        let mut deadline = Instant::now() + first;
        loop {
            if cancelled() {
                stop.store(true, Ordering::SeqCst);
                return Err(MlError::Runtime("cancelled".into()));
            }
            match rx.recv_timeout(Duration::from_millis(100)) {
                Ok(GpuMsg::Progress(done, total)) => {
                    deadline = Instant::now() + tile;
                    if !progress(done, total) {
                        stop.store(true, Ordering::SeqCst);
                    }
                }
                Ok(GpuMsg::Done(r)) => return r.map(|(data, on_gpu)| Denoised { data, on_gpu, gpu_timed_out: false }),
                Err(mpsc::RecvTimeoutError::Timeout) if Instant::now() < deadline => {}
                Err(e) => {
                    stop.store(true, Ordering::SeqCst);
                    let why = if matches!(e, mpsc::RecvTimeoutError::Timeout) { "stopped answering" } else { "thread ended without a result" };
                    tracing::warn!("denoise: the GPU {why}; its thread is left behind and the CPU starts over");
                    break;
                }
            }
        }
    }
    let mut cpu = open(false)?;
    let data = cpu.run(&rgb, width, height, &mut |done, total| progress(done, total) && !cancelled())?;
    Ok(Denoised { data, on_gpu: false, gpu_timed_out: gpu.is_some() })
}

fn key(asset_id: i64) -> String {
    format!("denoise:{asset_id}")
}

/// The denoise model's entry, when it is on disk and smart features are on; else why not.
fn model(app: &AppHandle) -> Result<&'static ModelEntry, String> {
    if !crate::models::enabled(app) {
        return Err("Turn on smart features to denoise.".into());
    }
    let entry = manifest().by_role("denoise").ok_or("This version of Lumen has no denoise model.")?;
    if !crate::models::store(app).is_ready(entry) {
        return Err("The denoise model is still downloading.".into());
    }
    Ok(entry)
}

fn use_gpu(state: &AppState) -> bool {
    crate::models::use_gpu(state) && !GPU_DENOISE_BROKEN.load(Ordering::SeqCst)
}

pub fn status(app: &AppHandle, asset_id: i64) -> AppResult<DenoiseStatus> {
    let state = app.state::<AppState>();
    let lib = state.library()?;
    let hash = lib.db.with_conn(|c| assets::render_info(c, asset_id))?.content_hash;
    let entry = model(app);
    let store = DerivativeStore::new(&lib.root);
    let ready = entry.as_ref().ok().is_some_and(|e| ready_at(&store.denoise_path(&hash, &e.id)).is_some());
    Ok(DenoiseStatus {
        available: entry.is_ok(),
        reason: entry.as_ref().err().cloned(),
        model: entry.as_ref().ok().map(|e| e.id.clone()),
        ready,
        running: running(|m| m.contains_key(&asset_id)),
        gpu: use_gpu(&state),
    })
}

fn failed(e: impl std::fmt::Display) -> AppError {
    AppError { code: "denoise_failed".into(), message: "This photo could not be denoised.".into(), detail: Some(e.to_string()) }
}

/// Builds the denoised base of `asset_id` (the same base Develop uses: sensor data for RAW),
/// reporting each tile; returns its path and whether the GPU ran it. The model is opened for
/// this run only, so its memory is not held while idle.
pub fn build(app: &AppHandle, asset_id: i64, cancel: &CancelToken) -> AppResult<(PathBuf, bool)> {
    let state = app.state::<AppState>();
    let lib = state.library()?;
    let entry = model(app).map_err(|m| AppError { code: "denoise_unavailable".into(), message: m, detail: None })?;
    let src = crate::develop_preview::source(&lib, asset_id)?;
    let store = DerivativeStore::new(&lib.root);
    let out = store.denoise_path(&src.hash, &entry.id);
    if let Some(p) = ready_at(&out) {
        return Ok((p, false));
    }
    let _one = BUILDS.lock().unwrap_or_else(|p| p.into_inner());
    // Built while this one waited (the panel and an export asked for the same photo).
    if let Some(p) = ready_at(&out) {
        return Ok((p, false));
    }
    // The bar shows at once, not after the base has loaded and the model has opened.
    let _ = app.emit(DENOISE_PROGRESS, DenoiseProgress { asset_id, done: 0, total: 0 });
    let ffmpeg = state.ffmpeg();
    let (base, _) = develop::load_base(&src.path, src.kind, ffmpeg.as_ref(), src.hdr, src.orientation, true, None).map_err(failed)?;
    let rgb: Arc<Vec<f32>> = Arc::new(base.data.iter().flat_map(|p| p.map(|v| linear_to_srgb(v).clamp(0.0, 1.0))).collect());
    let (w, h) = (base.width, base.height);
    drop(base);
    let path = crate::models::store(app).path_of(entry);
    let open: Open = Arc::new(move |gpu| Denoiser::open(&path, entry, gpu));
    let mut progress = |done: usize, total: usize| {
        touch(asset_id);
        let _ = app.emit(DENOISE_PROGRESS, DenoiseProgress { asset_id, done: done as u32, total: total as u32 });
        !cancel.is_cancelled()
    };
    let run = run_with_fallback(rgb, w, h, open, use_gpu(&state).then_some((FIRST_TILE_GPU, TILE_GPU)), &mut progress, &|| cancel.is_cancelled()).map_err(failed)?;
    if run.gpu_timed_out {
        GPU_DENOISE_BROKEN.store(true, Ordering::SeqCst);
    }
    let on_gpu = run.on_gpu;
    let data = run.data.as_chunks::<3>().0.iter().map(|p| p.map(srgb_to_linear)).collect();
    write_base(&out, &Linear { width: w, height: h, data }).map_err(failed)?;
    Ok((out, on_gpu))
}

/// After a base is built: the edited sizes rendered without it go, and a recipe that asks for
/// denoise is saved again under a new version, so every cached image of it (`immutable`,
/// versioned URLs) is fetched afresh.
pub fn finish_build(db: &photos_core::Db, store: &DerivativeStore, asset_id: i64) -> photos_core::Result<()> {
    db.with_conn(|conn| {
        let hash = assets::render_info(conn, asset_id)?.content_hash;
        store.delete_edits(&hash);
        let now_ms = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0);
        // Denoise is set in Develop, so it lives in the draft (Round 6); an older library
        // recipe may still ask for it.
        if let Some((draft, _)) = photos_core::edits::get_draft(conn, asset_id)? {
            if denoise_of(&draft).is_some() {
                photos_core::edits::save_draft(conn, asset_id, &draft, now_ms)?;
            }
        }
        if let Some((recipe, _)) = photos_core::edits::get_recipe(conn, asset_id)? {
            if denoise_of(&recipe).is_some() {
                photos_core::edits::save_recipe(conn, asset_id, &recipe, now_ms)?;
            }
        }
        Ok(())
    })
}

/// What follows a build, from the panel or an export: `finish_build`, the preview's loaded base
/// forgotten, the library changed and `denoise://finished` said.
pub fn complete(app: &AppHandle, asset_id: i64, model_id: &str, result: &AppResult<(PathBuf, bool)>, cancelled: bool) {
    let state = app.state::<AppState>();
    let (ok, on_gpu, error) = match result {
        Ok((_, gpu)) => (true, *gpu, None),
        Err(e) if cancelled => (false, false, Some(format!("cancelled: {}", e.message))),
        Err(e) => (false, false, Some(e.message.clone())),
    };
    if ok {
        if let Ok(lib) = state.library() {
            if let Err(e) = finish_build(&lib.db, &DerivativeStore::new(&lib.root), asset_id) {
                tracing::warn!(asset_id, "denoise: {e}");
            }
        }
        state.develop.clear_denoised(asset_id);
        state.changes.changed(app, true);
    }
    let _ = app.emit(DENOISE_FINISHED, DenoiseFinished { asset_id, ok, model: model_id.to_string(), on_gpu, error });
}

/// Runs `build` on its own `lumen-denoise` thread (one per photo; `BUILDS` still lets one
/// build run at a time), then `complete`. Round 15: never behind the job queue.
pub fn start(app: &AppHandle, asset_id: i64) -> AppResult<()> {
    let model_id = model(app).map_err(|m| AppError { code: "denoise_unavailable".into(), message: m, detail: None })?.id.clone();
    let cancel = CancelToken::new();
    let fresh = running(|m| {
        if m.contains_key(&asset_id) {
            return false;
        }
        m.insert(asset_id, (cancel.clone(), Instant::now()));
        true
    });
    if !fresh {
        return Ok(());
    }
    let a = app.clone();
    let spawned = std::thread::Builder::new().name("lumen-denoise".into()).spawn(move || {
        photos_jobs::background_priority();
        let result = build(&a, asset_id, &cancel);
        running(|m| m.remove(&asset_id));
        complete(&a, asset_id, &model_id, &result, cancel.is_cancelled());
    });
    if let Err(e) = spawned {
        running(|m| m.remove(&asset_id));
        return Err(failed(e));
    }
    Ok(())
}

pub fn cancel(_app: &AppHandle, asset_id: i64) {
    if let Some((token, _)) = running(|m| m.get(&asset_id).cloned()) {
        token.cancel();
    }
}

/// Cancels every Denoise run (Close, Task 1.5).
pub fn cancel_all() {
    running(|m| m.values().for_each(|(token, _)| token.cancel()));
}

#[cfg(test)]
mod tests {
    use super::*;
    use photos_core::edits::{Denoise, Detail, Develop, Recipe};
    use photos_media::develop::Linear;

    fn with_denoise(strength: f32) -> Recipe {
        Recipe {
            develop: Some(Develop { detail: Detail { denoise: Some(Denoise { strength, model: "m@1".into() }), ..Default::default() }, ..Default::default() }),
            ..Recipe::identity()
        }
    }

    use photos_ml::denoise::{Backend, DenoiseSpec};

    /// Gives back what it is given.
    struct Echo;
    impl Backend for Echo {
        fn run(&mut self, chw: &[f32], _: u32, _: u32) -> Result<Vec<f32>, MlError> {
            Ok(chw.to_vec())
        }
    }

    /// A driver call that never returns.
    struct Hang;
    impl Backend for Hang {
        fn run(&mut self, _: &[f32], _: u32, _: u32) -> Result<Vec<f32>, MlError> {
            loop {
                std::thread::park();
            }
        }
    }

    fn fake_open(gpu_hangs: bool) -> Open {
        Arc::new(move |gpu| {
            let spec = DenoiseSpec { tile: 32, overlap: 8, multiple: 8, input: "image".into() };
            let g: Option<Box<dyn Backend>> = gpu.then(|| if gpu_hangs { Box::new(Hang) as Box<dyn Backend> } else { Box::new(Echo) });
            Denoiser::with_backends(spec, g, Box::new(|| Ok(Box::new(Echo) as Box<dyn Backend>)))
        })
    }

    fn tiny_rgb() -> Arc<Vec<f32>> {
        Arc::new(vec![0.5; 64 * 48 * 3])
    }

    const FAST: (Duration, Duration) = (Duration::from_millis(200), Duration::from_millis(200));

    #[test]
    fn a_tile_that_never_returns_falls_back_to_the_cpu() {
        let started = Instant::now();
        let out = run_with_fallback(tiny_rgb(), 64, 48, fake_open(true), Some(FAST), &mut |_, _| true, &|| false).unwrap();
        assert!(!out.on_gpu, "finished on the CPU");
        assert!(out.gpu_timed_out);
        assert!(out.data.iter().all(|v| (v - 0.5).abs() < 1e-5));
        assert!(started.elapsed() < Duration::from_secs(3));
    }

    #[test]
    fn a_working_gpu_finishes_on_the_gpu() {
        let mut ticks = 0;
        let out = run_with_fallback(tiny_rgb(), 64, 48, fake_open(false), Some(FAST), &mut |_, _| { ticks += 1; true }, &|| false).unwrap();
        assert!(out.on_gpu && !out.gpu_timed_out);
        assert!(ticks > 0, "progress reaches the caller");
    }

    #[test]
    fn cancel_while_the_gpu_hangs_returns_at_once() {
        let started = Instant::now();
        let long = (Duration::from_secs(60), Duration::from_secs(20));
        let r = run_with_fallback(tiny_rgb(), 64, 48, fake_open(true), Some(long), &mut |_, _| true, &|| started.elapsed() > Duration::from_millis(150));
        assert!(r.is_err(), "cancelled, not finished on the CPU");
        assert!(started.elapsed() < Duration::from_secs(2));
    }

    #[test]
    fn a_part_file_is_never_taken_for_the_base() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("abc_m-1.png");
        std::fs::write(unique_part(&p), b"half").unwrap();
        assert!(ready_at(&p).is_none());
    }

    #[test]
    fn two_writers_never_share_a_part_file() {
        let p = std::path::Path::new("x").join("abc_m-1.png");
        let (a, b) = (unique_part(&p), unique_part(&p));
        assert_ne!(a, b);
        assert_eq!(a.parent(), p.parent());
        assert!(ready_at(&a).is_none());
    }

    #[test]
    fn a_finished_build_drops_edited_sizes_and_moves_the_version_on() {
        use photos_core::edits::{get_recipe, save_recipe};
        let dir = tempfile::tempdir().unwrap();
        let store = DerivativeStore::new(dir.path());
        let db = photos_core::Db::open_in_memory().unwrap();
        let (v1, v2) = db
            .with_conn(|c| {
                for (id, hash) in [(1i64, "aa11"), (2, "bb22")] {
                    c.execute(
                        "INSERT INTO assets (id, content_hash, relative_path, filename, extension, media_type, byte_size, width, height, orientation, imported_at)
                         VALUES (?1, ?2, ?2, 'a.jpg', 'jpg', 0, 1, 40, 30, 1, 1)",
                        (id, hash),
                    )?;
                }
                Ok((save_recipe(c, 1, &with_denoise(0.5), 1_000)?, save_recipe(c, 2, &Recipe { rotate: 90, ..Recipe::identity() }, 2_000)?))
            })
            .unwrap();
        let edited = store.edit_path("aa11", 256, v1);
        std::fs::create_dir_all(edited.parent().unwrap()).unwrap();
        std::fs::write(&edited, b"jpeg").unwrap();
        finish_build(&db, &store, 1).unwrap();
        finish_build(&db, &store, 2).unwrap();
        assert!(!edited.exists(), "the sizes rendered without the base are gone");
        db.with_conn(|c| {
            assert!(get_recipe(c, 1)?.unwrap().1 > v1, "a new version, so cached images are fetched again");
            assert_eq!(get_recipe(c, 2)?.unwrap().1, v2, "no denoise asked for: the version stays");
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn a_recipe_with_denoise_and_no_base_renders_like_one_without() {
        let base = Linear::new(8, 6, [0.2, 0.3, 0.4]);
        assert_eq!(apply_denoised(base.clone(), &with_denoise(0.8), None), base);
        let dn = Linear::new(8, 6, [0.4, 0.3, 0.2]);
        let mixed = apply_denoised(base.clone(), &with_denoise(0.5), Some(&dn));
        assert!((mixed.get(0, 0)[0] - 0.3).abs() < 1e-5);
        assert_eq!(apply_denoised(base.clone(), &Recipe::identity(), Some(&dn)), base, "no denoise asked for");
    }

    #[test]
    fn the_16_bit_cache_round_trips_within_a_65535th() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("x").join("abc_m-1.png");
        let img = Linear { width: 5, height: 3, data: (0..15).map(|i| { let v = i as f32 / 14.0; [v, v * v, 1.0 - v] }).collect() };
        write_base(&p, &img).unwrap();
        let left: Vec<_> = std::fs::read_dir(p.parent().unwrap()).unwrap().flatten().map(|e| e.file_name()).collect();
        assert_eq!(left.len(), 1, "renamed into place, nothing else left: {left:?}");
        let back = read_base(&p).unwrap();
        let enc = photos_media::develop::linear::linear_to_srgb;
        let max = img.data.iter().zip(&back.data).flat_map(|(a, b)| (0..3).map(move |c| (enc(a[c]) - enc(b[c])).abs())).fold(0.0, f32::max);
        assert!(max <= 1.0 / 65_535.0 + 1e-6, "{max}");
    }
}
