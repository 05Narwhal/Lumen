//! Develop's live preview (Phase 8 spec §3, *Preview speed*). A photo's **base** — decoded,
//! oriented, geometry-free, linear, at most a few thousand pixels — is kept in memory while
//! it is being developed, and every slider move runs the pipeline on it. Requests carry the
//! frontend's counter; a render that a newer request has overtaken stops before it encodes
//! (latest wins). The 100 % view renders only the visible part of the full-size frame, kept
//! in one slot that is released when the view zooms out or the screen closes.

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use photos_core::ai_masks::{self, AiRasters};
use photos_core::assets;
use photos_core::edits::{Crop, Recipe};
use photos_core::Library;
use photos_media::develop::masks::FrameMap;
use photos_media::develop::{self, geometry, histogram, Cancel, Ctx, Linear, Region};
use photos_media::hdr::HdrKind;
use photos_media::{encode_jpeg_bytes, kind_for_extension, Ffmpeg, MediaKind};
use serde::Serialize;
use ts_rs::TS;

use crate::error::{AppError, AppResult};
use crate::state::AppState;

/// Bases kept: the photo being developed and the one before it, so flicking back is free.
const KEEP_BASES: usize = 2;
/// The preview's usual long edge, and the most a tight crop may ask for.
pub const PREVIEW_EDGE: u32 = 2048;
pub const MAX_PREVIEW_EDGE: u32 = 4096;

/// What a preview renders from.
pub struct Base {
    pub linear: Linear,
    /// The full-size image's long edge (the sensor's, for RAW).
    pub full_long: u32,
    /// Rendered from sensor data.
    pub sensor: bool,
    /// A RAW that could only give its camera JPEG (rawler refuses it and no Adobe DNG
    /// Converter): Develop says *Camera rendering, not RAW*.
    pub camera: bool,
    /// The denoised base (Phase 8d) at this base's size, by model: loaded once.
    pub denoised: Mutex<Option<(String, Arc<Linear>)>>,
}

impl Base {
    pub fn new(linear: Linear, full_long: u32, sensor: bool) -> Self {
        Self { linear, full_long, sensor, camera: false, denoised: Mutex::new(None) }
    }

    /// A base as `develop::load_base_as` made it.
    pub fn loaded(l: develop::Loaded) -> Self {
        Self { camera: l.rendering == develop::Rendering::Camera, ..Self::new(l.linear, l.full_long, l.rendering == develop::Rendering::Sensor) }
    }

    /// The base with the recipe's denoise blended in (`apply_denoised`), reading the denoised
    /// file once per model. Without the file, the base as it is.
    pub fn for_recipe(&self, store: &photos_media::store::DerivativeStore, hash: &str, recipe: &Recipe) -> std::borrow::Cow<'_, Linear> {
        let Some(d) = crate::denoise::denoise_of(recipe) else { return std::borrow::Cow::Borrowed(&self.linear) };
        let mut slot = self.denoised.lock().unwrap_or_else(|p| p.into_inner());
        let cached = slot.as_ref().filter(|(m, _)| *m == d.model).map(|(_, l)| l.clone());
        let dn = cached.or_else(|| {
            let full = crate::denoise::load_for(store, hash, recipe)?;
            let fitted = Arc::new(if (full.width, full.height) == (self.linear.width, self.linear.height) { full } else { full.resize(self.linear.width, self.linear.height) });
            *slot = Some((d.model.clone(), fitted.clone()));
            Some(fitted)
        });
        match dn {
            Some(dn) => std::borrow::Cow::Owned(develop::blend_denoised(&self.linear, &dn, d.strength)),
            None => std::borrow::Cow::Borrowed(&self.linear),
        }
    }
}

/// Which stream of requests a render belongs to: each is overtaken only by its own newer
/// requests. The zoomed view asks for the after's and the before's region as a pair.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Channel {
    Preview,
    Region,
    BeforeRegion,
}

/// A photo's before (develop off, geometry kept) as encoded, for split and side by side.
pub struct BeforeRender {
    pub jpeg: Vec<u8>,
    pub width: u32,
    pub height: u32,
    /// Its base was the camera's JPEG (`Base::camera`).
    pub camera: bool,
}

/// Befores kept: the open photo and the one left last.
const KEEP_BEFORES: usize = 2;

/// The full-size frame (geometry applied) for the 100 % view.
struct FullFrame {
    asset_id: i64,
    geometry: String,
    frame: Arc<Linear>,
    /// Where the unedited photo lies in it, for masks.
    map: FrameMap,
}

#[derive(Default)]
pub struct DevelopState {
    bases: Mutex<VecDeque<(i64, u32, Arc<Base>)>>,
    /// The newest request seen per photo and channel; a running render reads its own counter
    /// at every check ([`Turn::overtaken`]).
    latest: Mutex<HashMap<(i64, Channel), Arc<AtomicU64>>>,
    /// One render per channel at a time (Round 24 §6.1). Each slider pause used to start a
    /// zoomed render that ran to the end even once overtaken (seconds each with halation on a
    /// 24 MP frame), each holding an async worker, until the pool starved and the page's
    /// heartbeat could not land. A newer request now waits here for the one it overtook,
    /// which stops at its next check.
    turns: [Mutex<()>; 3],
    befores: Mutex<VecDeque<(i64, String, Arc<BeforeRender>)>>,
    full: Mutex<Option<FullFrame>>,
    /// The 100 % view's denoised frame (geometry applied), by `geometry|model`: kept beside
    /// the plain one so a Strength change blends instead of reloading (Phase 8d).
    full_denoised: Mutex<Option<(i64, String, Arc<Linear>)>>,
}

/// The base's long edge for a crop: the preview's usual size, larger when a tight crop would
/// otherwise leave too few pixels, never beyond the photo.
pub fn base_edge(full_long: u32, crop: Option<Crop>) -> u32 {
    let frac = crop.map(|c| c.w.max(c.h)).unwrap_or(1.0).clamp(0.01, 1.0);
    let want = ((PREVIEW_EDGE as f32 / frac).round() as u32).clamp(PREVIEW_EDGE, MAX_PREVIEW_EDGE);
    want.min(full_long.max(1))
}

/// Whether request `n` has been overtaken, given the newest seen.
pub fn is_stale(newest: u64, n: u64) -> bool {
    n < newest
}

/// A render's turn on its channel ([`DevelopState::begin`]); the next request waits until
/// it is dropped.
pub struct Turn<'a> {
    _held: MutexGuard<'a, ()>,
    newest: Arc<AtomicU64>,
    n: u64,
}

impl Turn<'_> {
    /// A newer request on the same photo and channel has been seen: stop.
    pub fn overtaken(&self) -> bool {
        is_stale(self.newest.load(Ordering::Relaxed), self.n)
    }
}

/// A string that changes exactly when the geometry does.
pub fn geometry_key(r: &Recipe) -> String {
    let r = r.clone().normalized();
    serde_json::to_string(&(r.crop, r.rotate, r.straighten, r.flip_h, r.flip_v)).unwrap_or_default()
}

/// `(x, y, width, height)` in pixels.
pub type PixelBox = (u32, u32, u32, u32);

/// The pixel box of `rect` (0–1 of a `fw × fh` frame) grown by `margin` on every side and
/// clamped to the frame: `(x, y, w, h)` of the part to render and `(ix, iy, iw, ih)` of the
/// visible rect inside it.
pub fn region_box(rect: Crop, fw: u32, fh: u32, margin: u32) -> (PixelBox, PixelBox) {
    let x0 = ((rect.x.clamp(0.0, 1.0) * fw as f32).floor() as u32).min(fw - 1);
    let y0 = ((rect.y.clamp(0.0, 1.0) * fh as f32).floor() as u32).min(fh - 1);
    let x1 = ((((rect.x + rect.w).clamp(0.0, 1.0)) * fw as f32).ceil() as u32).clamp(x0 + 1, fw);
    let y1 = ((((rect.y + rect.h).clamp(0.0, 1.0)) * fh as f32).ceil() as u32).clamp(y0 + 1, fh);
    let (px, py) = (x0.saturating_sub(margin), y0.saturating_sub(margin));
    let (px1, py1) = ((x1 + margin).min(fw), (y1 + margin).min(fh));
    ((px, py, px1 - px, py1 - py), (x0 - px, y0 - py, x1 - x0, y1 - y0))
}

impl DevelopState {
    /// Record request `n` for `asset_id` on `channel` (each counts separately) and say
    /// whether it is the newest.
    pub fn note(&self, asset_id: i64, channel: Channel, n: u64) -> bool {
        let newest = self.counter(asset_id, channel);
        !is_stale(newest.fetch_max(n, Ordering::SeqCst).max(n), n)
    }

    pub fn stale(&self, asset_id: i64, channel: Channel, n: u64) -> bool {
        let l = self.latest.lock().unwrap_or_else(|p| p.into_inner());
        l.get(&(asset_id, channel)).is_some_and(|newest| is_stale(newest.load(Ordering::SeqCst), n))
    }

    fn counter(&self, asset_id: i64, channel: Channel) -> Arc<AtomicU64> {
        self.latest.lock().unwrap_or_else(|p| p.into_inner()).entry((asset_id, channel)).or_default().clone()
    }

    /// Request `n`'s turn on `channel`: noted, then waits for the render running there (which
    /// a newer request stops at its next check). `None` when `n` was overtaken, before or while
    /// it waited: it renders nothing.
    pub fn begin(&self, asset_id: i64, channel: Channel, n: u64) -> Option<Turn<'_>> {
        if !self.note(asset_id, channel, n) {
            return None;
        }
        let held = self.turns[channel as usize].lock().unwrap_or_else(|p| p.into_inner());
        let newest = self.counter(asset_id, channel);
        let turn = Turn { _held: held, newest, n };
        (!turn.overtaken()).then_some(turn)
    }

    /// Request `n`'s turn on `channel`, waited for on the calling thread, then `render` in the
    /// interactive pool. `None` when `n` was overtaken: it renders nothing.
    ///
    /// Never wait for a turn inside the pool (Round 26 B2): a render holding the turn runs
    /// rayon stages, and while its thread waits for them rayon may hand it a queued render,
    /// which then waits for the turn that same thread holds. Nothing returns again and
    /// Develop's spinner turns forever.
    pub fn in_turn<R: Send>(&self, asset_id: i64, channel: Channel, n: u64, render: impl FnOnce(&Turn<'_>) -> R + Send) -> Option<R> {
        let turn = self.begin(asset_id, channel, n)?;
        Some(photos_jobs::interactive(|| render(&turn)))
    }

    /// The photo's before for `geometry`, rendered (outside the lock) when absent.
    pub fn before<E>(&self, asset_id: i64, geometry: &str, render: impl FnOnce() -> Result<BeforeRender, E>) -> Result<Arc<BeforeRender>, E> {
        if let Some((_, _, b)) = self.befores.lock().expect("develop befores lock").iter().find(|(id, g, _)| *id == asset_id && g == geometry) {
            return Ok(b.clone());
        }
        let b = Arc::new(render()?);
        let mut befores = self.befores.lock().expect("develop befores lock");
        befores.retain(|(id, _, _)| *id != asset_id);
        befores.push_back((asset_id, geometry.to_string(), b.clone()));
        while befores.len() > KEEP_BEFORES {
            befores.pop_front();
        }
        Ok(b)
    }

    /// The base for `asset_id` at `edge`, loading it (outside the lock) when absent.
    pub fn base<E>(&self, asset_id: i64, edge: u32, load: impl FnOnce() -> Result<Base, E>) -> Result<Arc<Base>, E> {
        if let Some((_, _, b)) = self.bases.lock().expect("develop bases lock").iter().find(|(id, e, _)| *id == asset_id && *e == edge) {
            return Ok(b.clone());
        }
        let base = Arc::new(load()?);
        let mut bases = self.bases.lock().expect("develop bases lock");
        // One base per photo: a new size replaces the old one.
        bases.retain(|(id, _, _)| *id != asset_id);
        bases.push_back((asset_id, edge, base.clone()));
        while bases.len() > KEEP_BASES {
            bases.pop_front();
        }
        Ok(base)
    }

    /// The base held for `asset_id`, at whatever size.
    pub fn held_base(&self, asset_id: i64) -> Option<Arc<Base>> {
        self.bases.lock().expect("develop bases lock").iter().find(|(id, _, _)| *id == asset_id).map(|(_, _, b)| b.clone())
    }

    /// The base for `asset_id` at `edge` **or larger**, loading one at `edge` (outside the lock)
    /// only when none is held: for readers that bring the image down to a few hundred pixels
    /// anyway (Auto's model, Retouch: 256 px), so they never evict the preview's larger base
    /// of a cropped photo (`base_edge` grows with the crop).
    pub fn base_at_least<E>(&self, asset_id: i64, edge: u32, load: impl FnOnce() -> Result<Base, E>) -> Result<Arc<Base>, E> {
        if let Some((_, _, b)) = self.bases.lock().expect("develop bases lock").iter().find(|(id, e, _)| *id == asset_id && *e >= edge) {
            return Ok(b.clone());
        }
        self.base(asset_id, edge, load)
    }

    /// The full-size frame for the 100 % view, rebuilt when the photo or its geometry
    /// changes. Only one is ever held.
    pub fn full_frame<E>(&self, asset_id: i64, geometry: &str, load: impl FnOnce() -> Result<(Linear, FrameMap), E>) -> Result<(Arc<Linear>, FrameMap), E> {
        let mut slot = self.full.lock().expect("develop full lock");
        if let Some(f) = slot.as_ref().filter(|f| f.asset_id == asset_id && f.geometry == geometry) {
            return Ok((f.frame.clone(), f.map));
        }
        // Free the old frame before building the new one: two would not fit the budget.
        *slot = None;
        let (frame, map) = load()?;
        let frame = Arc::new(frame);
        *slot = Some(FullFrame { asset_id, geometry: geometry.to_string(), frame: frame.clone(), map });
        Ok((frame, map))
    }

    /// Forget a photo's loaded denoised base (a new one was just built).
    pub fn clear_denoised(&self, asset_id: i64) {
        for (id, _, b) in self.bases.lock().unwrap().iter() {
            if *id == asset_id {
                *b.denoised.lock().unwrap_or_else(|p| p.into_inner()) = None;
            }
        }
        let mut dn = self.full_denoised.lock().unwrap_or_else(|p| p.into_inner());
        if dn.as_ref().is_some_and(|(a, _, _)| *a == asset_id) {
            *dn = None;
        }
    }

    pub fn release_full(&self) {
        *self.full.lock().expect("develop full lock") = None;
        *self.full_denoised.lock().expect("develop denoised lock") = None;
    }

    /// The denoised frame under `key`, loaded once; `None` (and nothing kept) without a base.
    pub fn full_denoised(&self, asset_id: i64, key: &str, load: impl FnOnce() -> Option<Linear>) -> Option<Arc<Linear>> {
        let mut slot = self.full_denoised.lock().expect("develop denoised lock");
        if let Some((_, _, f)) = slot.as_ref().filter(|(a, k, _)| *a == asset_id && k == key) {
            return Some(f.clone());
        }
        *slot = None;
        let frame = Arc::new(load()?);
        *slot = Some((asset_id, key.to_string(), frame.clone()));
        Some(frame)
    }

    /// Forget every base and frame (the library changed).
    pub fn clear(&self) {
        self.bases.lock().expect("develop bases lock").clear();
        self.befores.lock().expect("develop befores lock").clear();
        self.release_full();
    }
}

/// How far past the visible rect a 100 % render reaches, so blurs near its edges see real
/// neighbours: three clarity radii of the frame.
pub fn region_margin(frame_long: u32) -> u32 {
    (3.0 * develop::local::CLARITY_SIGMA * frame_long as f32).ceil() as u32
}

/// [`region_margin`] for this recipe: an AI mask's refined edge needs its own reach, which goes
/// by the uncropped photo's size, so a tight crop can need more than the frame's blurs do.
pub fn recipe_margin(recipe: &Recipe, map: &FrameMap, frame_long: u32) -> u32 {
    let margin = region_margin(frame_long);
    if recipe.ai_rasters().is_empty() {
        margin
    } else {
        margin.max(develop::masks::refine_pad(map))
    }
}

/// The rasters `recipe`'s AI masks name. No database read for a recipe without one.
pub fn rasters_of(lib: &Library, asset_id: i64, recipe: &Recipe) -> AppResult<AiRasters> {
    if recipe.ai_rasters().is_empty() {
        return Ok(AiRasters::new());
    }
    Ok(lib.db.with_conn(|c| ai_masks::rasters_for(c, asset_id, recipe))?)
}

/// The histogram sent with each preview (spec §3): counts per bin, 256 bins.
#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DevelopHistogram {
    pub r: Vec<u32>,
    pub g: Vec<u32>,
    pub b: Vec<u32>,
    pub luma: Vec<u32>,
    pub clipped_shadows: u32,
    pub clipped_highlights: u32,
}

impl From<histogram::Histogram> for DevelopHistogram {
    fn from(h: histogram::Histogram) -> Self {
        Self { r: h.r, g: h.g, b: h.b, luma: h.luma, clipped_shadows: h.clipped_shadows, clipped_highlights: h.clipped_highlights }
    }
}

/// A rendered preview: its bytes wait at `photos://edit-preview/{token}`.
#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DevelopPreview {
    /// Empty when `superseded`.
    pub token: String,
    pub width: u32,
    pub height: u32,
    pub histogram: Option<DevelopHistogram>,
    /// A newer request overtook this one; nothing was rendered.
    pub superseded: bool,
    /// Rendered from the camera's JPEG because rawler can't read the RAW's sensor data and Adobe
    /// DNG Converter is not there or failed (`docs/RAW-FORMATS.md`).
    pub camera_rendering: bool,
}

impl DevelopPreview {
    pub fn superseded() -> Self {
        Self { token: String::new(), width: 0, height: 0, histogram: None, superseded: true, camera_rendering: false }
    }
}

/// The facts the canvas needs about a photo in Develop.
#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DevelopBaseInfo {
    /// Display-oriented size of the full image.
    pub width: u32,
    pub height: u32,
    /// A camera RAW file, developed from its sensor data.
    pub raw: bool,
}

/// The sentence for a RAW Lumen cannot develop at all: rawler refuses it and it has no camera
/// JPEG either (`docs/RAW-FORMATS.md`). Names no maker: it can be anyone's file.
pub(crate) const RAW_REFUSED_MESSAGE: &str = "Lumen can't develop this RAW file. The photo is safe. Adobe DNG Converter may be able to read it: see Settings, RAW formats.";

pub(crate) fn render_failed(e: impl std::fmt::Display) -> AppError {
    let detail = e.to_string();
    // A known limit, not a fault: its own code and a sentence that says what to do.
    if detail.contains(photos_media::develop::raw::REFUSED) {
        return AppError { code: "raw_unsupported".into(), message: RAW_REFUSED_MESSAGE.into(), detail: Some(detail) };
    }
    AppError { code: "develop_failed".into(), message: "This photo could not be developed.".into(), detail: Some(detail) }
}

pub(crate) struct Source {
    pub path: std::path::PathBuf,
    pub kind: MediaKind,
    pub hdr: HdrKind,
    pub orientation: u16,
    pub hash: String,
    /// Display-oriented long edge, from the catalogue (0 when unknown).
    pub long: u32,
}

pub(crate) fn source(lib: &Library, asset_id: i64) -> AppResult<Source> {
    let info = lib.db.with_conn(|c| assets::render_info(c, asset_id))?;
    let kind = kind_for_extension(&info.extension);
    if kind == MediaKind::Video {
        return Err(AppError { code: "not_a_photo".into(), message: "Videos cannot be developed.".into(), detail: None });
    }
    Ok(Source {
        path: lib.originals_dir().join(&info.relative_path),
        kind,
        hdr: HdrKind::from_code(info.hdr),
        orientation: info.orientation as u16,
        long: info.width.unwrap_or(0).max(info.height.unwrap_or(0)) as u32,
        hash: info.content_hash,
    })
}

/// Develop always reads a RAW file's sensor data.
pub(crate) fn load(src: &Source, ffmpeg: Option<&Ffmpeg>, edge: Option<u32>) -> AppResult<(Linear, u32)> {
    load_as(src, ffmpeg, edge).map(|l| (l.linear, l.full_long))
}

/// [`load`], saying what the base was made from.
pub(crate) fn load_as(src: &Source, ffmpeg: Option<&Ffmpeg>, edge: Option<u32>) -> AppResult<develop::Loaded> {
    develop::load_base_as(&src.path, src.kind, ffmpeg, src.hdr, src.orientation, true, edge).map_err(render_failed)
}

/// The preview base at `edge`, as Develop, Auto and Retouch keep it.
pub(crate) fn load_preview_base(src: &Source, ffmpeg: Option<&Ffmpeg>, edge: u32) -> AppResult<Base> {
    Ok(Base::loaded(load_as(src, ffmpeg, Some(edge))?))
}

fn encode(img: image::RgbImage, quality: u8) -> AppResult<Vec<u8>> {
    encode_jpeg_bytes(&image::DynamicImage::ImageRgb8(img), quality).map_err(render_failed)
}

/// Render `recipe` on the photo's preview base. `before` leaves out every develop setting
/// (geometry stays); `clipping` paints clipped pixels (the histogram is measured first).
pub fn render_preview(state: &AppState, turn: &Turn<'_>, asset_id: i64, recipe: &Recipe, before: bool, clipping: bool) -> AppResult<DevelopPreview> {
    let d = &state.develop;
    let overtaken = || turn.overtaken();
    let lib = state.library()?;
    let src = source(&lib, asset_id)?;
    let recipe = recipe.clone().normalized();
    let edge = base_edge(if src.long > 0 { src.long } else { PREVIEW_EDGE }, recipe.crop);
    let ffmpeg = state.ffmpeg();
    let base = d.base(asset_id, edge, || -> AppResult<Base> {
        load_preview_base(&src, ffmpeg.as_ref(), edge)
    })?;
    if turn.overtaken() {
        return Ok(DevelopPreview::superseded());
    }
    let shown = if before { Recipe { develop: None, ..recipe } } else { recipe };
    let rasters = rasters_of(&lib, asset_id, &shown)?;
    let ctx = Ctx { ai_rasters: Some(&rasters), cancel: Cancel(Some(&overtaken)), ..Ctx::new(base.linear.long_edge() as f32 / base.full_long.max(1) as f32, develop::seed_for(&src.hash)) };
    let linear = base.for_recipe(&photos_media::store::DerivativeStore::new(&lib.root), &src.hash, &shown);
    let mut img = develop::render_base(&linear, &shown, &ctx);
    if turn.overtaken() {
        return Ok(DevelopPreview::superseded());
    }
    let hist = histogram::compute(&img);
    if clipping {
        histogram::clipping_overlay(&mut img);
    }
    let (width, height) = img.dimensions();
    let token = state.edit_previews.put(encode(img, 90)?);
    Ok(DevelopPreview { token: token.to_string(), width, height, histogram: Some(hist.into()), superseded: false, camera_rendering: base.camera })
}

/// The photo's before for split and side by side (Round 4J): develop off, geometry kept, on
/// the same base as the preview so the two line up, and rendered once per photo and geometry.
/// It never competes with the slider's renders.
pub fn render_before(state: &AppState, asset_id: i64, recipe: &Recipe) -> AppResult<DevelopPreview> {
    let d = &state.develop;
    let lib = state.library()?;
    let src = source(&lib, asset_id)?;
    let shown = Recipe { develop: None, ..recipe.clone().normalized() };
    let before = d.before(asset_id, &geometry_key(&shown), || -> AppResult<BeforeRender> {
        let edge = base_edge(if src.long > 0 { src.long } else { PREVIEW_EDGE }, shown.crop);
        let ffmpeg = state.ffmpeg();
        let base = d.base(asset_id, edge, || -> AppResult<Base> {
            load_preview_base(&src, ffmpeg.as_ref(), edge)
        })?;
        let ctx = Ctx::new(base.linear.long_edge() as f32 / base.full_long.max(1) as f32, develop::seed_for(&src.hash));
        let img = develop::render_base(&base.linear, &shown, &ctx);
        let (width, height) = img.dimensions();
        Ok(BeforeRender { jpeg: encode(img, 90)?, width, height, camera: base.camera })
    })?;
    // Pinned: the key (photo + geometry) outlives any number of slider renders, so the
    // before's token must too (Round 24 §4: Reset all left a broken image after 8 renders).
    let token = state.edit_previews.put_pinned(before.jpeg.clone());
    Ok(DevelopPreview { token: token.to_string(), width: before.width, height: before.height, histogram: None, superseded: false, camera_rendering: before.camera })
}

/// The visible box of a rendered region, at `scale` of full resolution (Round 4J): below
/// 100 % the webview needs fewer pixels, above it the full-resolution ones are scaled up in
/// the webview, so the scale never exceeds 1.
pub fn finish_region(part: &Linear, (ix, iy, iw, ih): PixelBox, scale: f32) -> image::RgbImage {
    let visible = geometry::crop(part, ix, iy, iw, ih);
    let scale = if scale.is_finite() { scale.min(1.0) } else { 1.0 };
    if scale >= 0.999 {
        return visible.to_rgb8();
    }
    let (w, h) = (((iw as f32 * scale).round() as u32).max(1), ((ih as f32 * scale).round() as u32).max(1));
    visible.resize(w, h).to_rgb8()
}

/// The zoomed view: the visible `rect` (0-1 of the edited frame) from the full-resolution
/// frame, rendered with the frame's radii, vignette centre, grain and airlight so it matches
/// the whole, and returned at `scale` of full resolution (1 at 100 % and above).
pub fn render_region(state: &AppState, turn: &Turn<'_>, asset_id: i64, recipe: &Recipe, rect: Crop, scale: f32, before: bool) -> AppResult<DevelopPreview> {
    let d = &state.develop;
    let overtaken = || turn.overtaken();
    let lib = state.library()?;
    let src = source(&lib, asset_id)?;
    let recipe = recipe.clone().normalized();
    let ffmpeg = state.ffmpeg();
    // The frame is cached by geometry and denoise: both change its pixels before develop.
    let geometry = geometry_key(&recipe);
    let (frame, map) = d.full_frame(asset_id, &geometry, || -> AppResult<(Linear, FrameMap)> {
        let (full, _) = load(&src, ffmpeg.as_ref(), None)?;
        Ok((geometry::apply(&full, &recipe), FrameMap::new(full.width, full.height, &recipe)))
    })?;
    // Stage 2 on the visible part only: the denoised frame is loaded once per geometry and
    // model, and a Strength change just blends again.
    let denoised = match (crate::denoise::denoise_of(&recipe), before) {
        (Some(dn), false) => d.full_denoised(asset_id, &format!("{geometry}|{}", dn.model), || {
            let full = crate::denoise::load_for(&photos_media::store::DerivativeStore::new(&lib.root), &src.hash, &recipe)?;
            Some(geometry::apply(&full, &recipe))
        }),
        _ => None,
    };
    if turn.overtaken() {
        return Ok(DevelopPreview::superseded());
    }
    let (fw, fh) = (frame.width, frame.height);
    let ((px, py, pw, ph), (ix, iy, iw, ih)) = region_box(rect, fw, fh, recipe_margin(&recipe, &map, fw.max(fh)));
    let mut part = geometry::crop(&frame, px, py, pw, ph);
    if let (Some(dn), Some(d)) = (denoised.as_ref(), crate::denoise::denoise_of(&recipe)) {
        if (dn.width, dn.height) == (fw, fh) {
            part = develop::blend_denoised(&part, &geometry::crop(dn, px, py, pw, ph), d.strength);
        }
    }
    if let (Some(dv), false) = (recipe.develop.as_ref(), before) {
        let rasters = rasters_of(&lib, asset_id, &recipe)?;
        let ctx = Ctx {
            region: Some(Region { x: px, y: py, frame_width: fw, frame_height: fh }),
            airlight: Some(develop::frame_airlight(&frame, dv)),
            frame_map: Some(map),
            ai_rasters: Some(&rasters),
            cancel: Cancel(Some(&overtaken)),
            ..Ctx::new(1.0, develop::seed_for(&src.hash))
        };
        develop::develop_stages(&mut part, dv, &ctx);
    }
    if turn.overtaken() {
        return Ok(DevelopPreview::superseded());
    }
    let img = finish_region(&part, (ix, iy, iw, ih), scale);
    let (width, height) = img.dimensions();
    let token = state.edit_previews.put(encode(img, 92)?);
    Ok(DevelopPreview { token: token.to_string(), width, height, histogram: None, superseded: false, camera_rendering: d.held_base(asset_id).is_some_and(|b| b.camera) })
}

pub fn base_info(state: &AppState, asset_id: i64) -> AppResult<DevelopBaseInfo> {
    let lib = state.library()?;
    let info = lib.db.with_conn(|c| assets::render_info(c, asset_id))?;
    let (w, h) = (info.width.unwrap_or(0) as u32, info.height.unwrap_or(0) as u32);
    let (width, height) = if photos_media::orientation_swaps(info.orientation as u16) { (h, w) } else { (w, h) };
    Ok(DevelopBaseInfo { width, height, raw: kind_for_extension(&info.extension) == MediaKind::Raw })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_region_reaches_far_enough_for_a_refined_ai_edge() {
        use photos_core::edits::{Develop, LocalAdjust, Mask, MaskShape};
        // A tight crop of a large photo: the frame's blurs need little, the raster's edge doesn't.
        let crop = Some(Crop { x: 0.4, y: 0.4, w: 0.1, h: 0.1 });
        let plain = Recipe { crop, ..Recipe::identity() };
        let shape = MaskShape::Ai { add: true, target: ai_masks::AiMaskKind::Sky, instance: 0, raster: "aa".into() };
        let mask = Mask { id: 1, name: "Sky".into(), shapes: vec![shape], invert: false, opacity: 1.0, adjust: LocalAdjust { exposure: 1.0, ..Default::default() }, visible: true };
        let masked = Recipe { develop: Some(Develop { masks: vec![mask], ..Default::default() }), ..plain.clone() };
        let map = FrameMap::new(6048, 4024, &masked);
        assert_eq!(recipe_margin(&plain, &map, 605), region_margin(605));
        assert_eq!(recipe_margin(&masked, &map, 605), develop::masks::refine_pad(&map));
        assert!(develop::masks::refine_pad(&map) > region_margin(605), "the case this is for");
    }

    #[test]
    fn a_tight_crop_asks_for_a_larger_base_within_limits() {
        assert_eq!(base_edge(6048, None), 2048);
        assert_eq!(base_edge(6048, Some(Crop { x: 0.0, y: 0.0, w: 0.5, h: 0.4 })), 4096);
        assert_eq!(base_edge(6048, Some(Crop { x: 0.0, y: 0.0, w: 0.8, h: 0.8 })), 2560);
        assert_eq!(base_edge(1600, None), 1600, "never beyond the photo");
    }

    /// Round 24 §6.1: a zoomed render with halation on an ellipse, overtaken while it runs,
    /// stops at its next check; the newer request waits for it (one render per channel) and an
    /// older one arriving later renders nothing.
    #[test]
    fn develop_preview_cancels_stale() {
        use photos_core::edits::{Basic, Develop, Halation, LocalAdjust, Mask, MaskShape};
        use std::sync::atomic::AtomicBool;
        use std::time::{Duration, Instant};
        let (fw, fh) = (1800, 1200);
        let mut part = Linear::new(fw, fh, [0.0; 3]);
        for (i, p) in part.data.iter_mut().enumerate() {
            let l = 0.1 + 0.9 * ((i as u32 % fw) as f32 / fw as f32);
            *p = [l, l * 0.9, l * 0.8];
        }
        let shape = MaskShape::Radial { add: true, center: [0.5, 0.5], radius: [0.3, 0.35], angle: 0.0, feather: 0.5, inside: true };
        let adjust = LocalAdjust { exposure: 0.5, halation: Halation { amount: 1.0, size: 1.0 }, glow: photos_core::edits::Glow { amount: 1.0, size: 1.0 }, ..Default::default() };
        let mask = Mask { id: 1, name: "Ellipse".into(), shapes: vec![shape], invert: false, opacity: 1.0, adjust, visible: true };
        let dv = Develop { basic: Basic { exposure: 1.0, clarity: 0.5, ..Default::default() }, masks: vec![mask], ..Default::default() };
        let ctx = Ctx { frame_map: Some(FrameMap::new(fw, fh, &Recipe::identity())), ..Ctx::new(1.0, 1) };
        let t = Instant::now();
        develop::develop_stages(&mut part.clone(), &dv, &ctx);
        let whole = t.elapsed();

        let s = DevelopState::default();
        let started = AtomicBool::new(false);
        let finished = AtomicBool::new(false);
        let first_took = std::thread::scope(|scope| {
            let first = scope.spawn(|| {
                let turn = s.begin(1, Channel::Region, 1).expect("the first request has its turn");
                let overtaken = || {
                    started.store(true, Ordering::SeqCst);
                    turn.overtaken()
                };
                let t = Instant::now();
                develop::develop_stages(&mut part.clone(), &dv, &Ctx { cancel: Cancel(Some(&overtaken)), ..ctx });
                let took = t.elapsed();
                assert!(turn.overtaken(), "request 2 arrived while it ran");
                finished.store(true, Ordering::SeqCst);
                took
            });
            while !started.load(Ordering::SeqCst) {
                std::thread::yield_now();
            }
            let second = s.begin(1, Channel::Region, 2).expect("the newest request has its turn");
            assert!(finished.load(Ordering::SeqCst), "it waited for the render it overtook");
            assert!(s.begin(1, Channel::Region, 1).is_none(), "an older request renders nothing");
            drop(second);
            first.join().unwrap()
        });
        assert!(first_took < whole / 2 + Duration::from_millis(50), "the overtaken render stopped early: {first_took:?} of {whole:?}");
    }

    /// Round 26 B2, the Develop spinner that never stopped: renders of other photos queued
    /// behind one long parallel render must all come back. Waiting for the turn inside the
    /// pool deadlocked it: while the holder's thread waited for its own rayon stages, rayon
    /// handed it a queued render, which then waited for the turn that thread already held.
    #[test]
    fn renders_queued_behind_a_long_one_all_come_back() {
        use rayon::prelude::*;
        use std::sync::{atomic::AtomicBool, mpsc};
        use std::time::Duration;
        let s = Arc::new(DevelopState::default());
        let (done, finished) = mpsc::channel();
        let holding = Arc::new(AtomicBool::new(false));
        {
            let (s, done, holding) = (s.clone(), done.clone(), holding.clone());
            std::thread::spawn(move || {
                let r = s.in_turn(1, Channel::Preview, 1, |_| {
                    holding.store(true, Ordering::SeqCst);
                    (0..1500u32).into_par_iter().map(|i| {
                        std::thread::sleep(Duration::from_millis(1));
                        i
                    }).sum::<u32>()
                });
                done.send(r.is_some()).unwrap();
            });
        }
        while !holding.load(Ordering::SeqCst) {
            std::thread::yield_now();
        }
        // More first renders than the pool has threads, as opening Develop on photos with edits
        // sends (preview, region, before; the filmstrip's neighbours).
        let queued = 4 * std::thread::available_parallelism().map_or(4, |n| n.get()) as i64 + 8;
        for id in 2..2 + queued {
            let (s, done) = (s.clone(), done.clone());
            std::thread::spawn(move || {
                let r = s.in_turn(id, Channel::Preview, 1, |_| 0u32);
                done.send(r.is_some()).unwrap();
            });
        }
        for _ in 0..=queued {
            let rendered = finished.recv_timeout(Duration::from_secs(30)).expect("a render never came back: the pool deadlocked");
            assert!(rendered, "each was the newest for its photo");
        }
    }

    #[test]
    fn the_newest_request_wins() {
        let s = DevelopState::default();
        assert!(s.note(1, Channel::Preview, 1));
        assert!(s.note(1, Channel::Preview, 3));
        assert!(!s.note(1, Channel::Preview, 2), "an older request arriving late is already stale");
        assert!(s.stale(1, Channel::Preview, 1) && !s.stale(1, Channel::Preview, 3));
        assert!(s.note(1, Channel::Region, 1), "region requests count on their own");
        assert!(s.note(2, Channel::Preview, 1), "and so does another photo");
        // Round 4J: the before's region and the after's go out as a pair with one number;
        // neither overtakes the other.
        assert!(s.note(1, Channel::Region, 5));
        assert!(s.note(1, Channel::BeforeRegion, 5));
        assert!(!s.stale(1, Channel::Region, 5));
    }

    #[test]
    fn bases_are_kept_per_photo_and_size() {
        let s = DevelopState::default();
        let calls = std::cell::Cell::new(0);
        let load = |n: u32| {
            calls.set(calls.get() + 1);
            Ok::<_, ()>(Base::new(Linear::new(n, 1, [0.0; 3]), 6000, false))
        };
        s.base(1, 2048, || load(1)).unwrap();
        s.base(1, 2048, || load(1)).unwrap();
        assert_eq!(calls.get(), 1);
        s.base(1, 4096, || load(2)).unwrap();
        assert_eq!(calls.get(), 2, "a larger size is a new base");
        s.base(2, 2048, || load(3)).unwrap();
        s.base(3, 2048, || load(4)).unwrap();
        s.base(1, 4096, || load(5)).unwrap();
        assert_eq!(calls.get(), 5, "only two photos are kept");
    }

    #[test]
    fn a_reader_of_the_whole_photo_takes_the_larger_base_a_crop_holds() {
        let s = DevelopState::default();
        let calls = std::cell::Cell::new(0);
        let load = |n: u32| {
            calls.set(calls.get() + 1);
            Ok::<_, ()>(Base::new(Linear::new(n, 1, [0.0; 3]), 6000, true))
        };
        // The preview of a cropped photo holds 4096 px; Retouch asks for the uncropped 2048.
        s.base(1, 4096, || load(4096)).unwrap();
        assert_eq!(s.base_at_least(1, 2048, || load(2048)).unwrap().linear.width, 4096);
        assert_eq!(calls.get(), 1, "nothing loaded");
        s.base(1, 4096, || load(4096)).unwrap();
        assert_eq!(calls.get(), 1, "and the preview's base was not evicted");
        // Smaller than asked: loaded at the size asked for.
        s.base(2, 2048, || load(2048)).unwrap();
        assert_eq!(s.base_at_least(2, 4096, || load(4096)).unwrap().linear.width, 4096);
        assert_eq!(calls.get(), 3);
        // None held: loaded.
        assert_eq!(s.base_at_least(3, 2048, || load(2048)).unwrap().linear.width, 2048);
    }

    /// `LUMEN_RAW_SAMPLE=<a RAW>` (copied into a temp dir; only the copy is read): what an Auto
    /// (with the model) or Retouch click costs on a photo cropped to half its width. The model
    /// reads the uncropped base; the preview holds the crop's larger one. Before 12-7's fix the
    /// model's `base` evicted the preview's and the next preview decoded again.
    #[test]
    #[ignore = "needs LUMEN_RAW_SAMPLE"]
    fn a_click_on_a_cropped_raw_decodes_nothing_more() {
        let given = std::path::PathBuf::from(std::env::var("LUMEN_RAW_SAMPLE").expect("LUMEN_RAW_SAMPLE"));
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(given.file_name().unwrap());
        std::fs::copy(&given, &path).unwrap();
        let mut src = Source { path, kind: MediaKind::Raw, hdr: HdrKind::None, orientation: 1, hash: String::new(), long: 0 };
        src.long = load(&src, None, Some(64)).unwrap().1;
        let crop = Some(Crop { x: 0.25, y: 0.25, w: 0.5, h: 0.5 });
        let (cropped, whole) = (base_edge(src.long, crop), base_edge(src.long, None));
        eprintln!("full {} px; preview base {cropped} px, model base {whole} px", src.long);
        let loads = std::cell::Cell::new(0);
        let load_at = |edge: u32| {
            loads.set(loads.get() + 1);
            load_preview_base(&src, None, edge)
        };
        for at_least in [false, true] {
            let s = DevelopState::default();
            s.base(1, cropped, || load_at(cropped)).unwrap();
            loads.set(0);
            let t = std::time::Instant::now();
            for _ in 0..3 {
                // A click: the model reads the whole photo, then the next preview renders.
                if at_least {
                    s.base_at_least(1, whole, || load_at(whole)).unwrap();
                } else {
                    s.base(1, whole, || load_at(whole)).unwrap();
                }
                s.base(1, cropped, || load_at(cropped)).unwrap();
            }
            eprintln!("{}: 3 clicks, {} RAW decodes, {:?} decoding", if at_least { "after" } else { "before" }, loads.get(), t.elapsed());
            assert_eq!(loads.get(), if at_least { 0 } else { 6 });
        }
    }

    #[test]
    fn a_refusal_says_what_to_do_and_names_no_maker() {
        let e = render_failed(photos_media::MediaError::Unsupported(format!("{}: Unknown camera", photos_media::develop::raw::REFUSED)));
        assert_eq!(e.code, "raw_unsupported");
        assert_eq!(e.message, RAW_REFUSED_MESSAGE);
        for maker in ["Nikon", "Canon", "Sony", "Fujifilm", "HE"] {
            assert!(!e.message.contains(maker), "{}", e.message);
        }
        assert!(e.message.contains("Adobe DNG Converter") && e.message.contains("Settings"), "{}", e.message);
        assert_eq!(render_failed("some other failure").code, "develop_failed");
    }

    #[test]
    fn a_camera_rendering_is_marked_on_its_base() {
        use photos_media::develop::{Loaded, Rendering};
        let l = |rendering| Base::loaded(Loaded { linear: Linear::new(2, 1, [0.0; 3]), full_long: 8256, rendering });
        let camera = l(Rendering::Camera);
        assert!(camera.camera && !camera.sensor);
        let sensor = l(Rendering::Sensor);
        assert!(!sensor.camera && sensor.sensor);
        assert!(!l(Rendering::File).camera);
        let s = DevelopState::default();
        s.base(7, 2048, || Ok::<_, ()>(l(Rendering::Camera))).unwrap();
        assert!(s.held_base(7).is_some_and(|b| b.camera) && s.held_base(8).is_none());
    }

    #[test]
    fn one_full_frame_is_kept_and_released() {
        let s = DevelopState::default();
        let calls = std::cell::Cell::new(0);
        let load = || {
            calls.set(calls.get() + 1);
            Ok::<_, ()>((Linear::new(2, 2, [0.0; 3]), FrameMap::identity(2, 2)))
        };
        s.full_frame(1, "a", load).unwrap();
        s.full_frame(1, "a", load).unwrap();
        s.full_frame(1, "b", load).unwrap();
        assert_eq!(calls.get(), 2, "new geometry, new frame");
        s.release_full();
        s.full_frame(1, "b", load).unwrap();
        assert_eq!(calls.get(), 3);
    }

    /// Review fix: a Strength change in the 100 % view reloads nothing. The denoised frame is
    /// kept per geometry and model, beside the plain one, and released with it.
    #[test]
    fn the_denoised_frame_is_kept_per_geometry_and_model() {
        let s = DevelopState::default();
        let calls = std::cell::Cell::new(0);
        let load = || {
            calls.set(calls.get() + 1);
            Some(Linear::new(2, 2, [0.5; 3]))
        };
        assert!(s.full_denoised(1, "a|m@1", load).is_some());
        assert!(s.full_denoised(1, "a|m@1", load).is_some());
        assert_eq!(calls.get(), 1, "the same geometry and model: loaded once");
        s.full_denoised(1, "b|m@1", load);
        assert_eq!(calls.get(), 2);
        assert!(s.full_denoised(1, "c|m@1", || None).is_none(), "no base built: nothing kept");
        s.full_denoised(1, "b|m@1", load);
        s.release_full();
        s.full_denoised(1, "b|m@1", load);
        assert_eq!(calls.get(), 4, "released with the plain frame");
        s.clear_denoised(1);
        s.full_denoised(1, "b|m@1", load);
        assert_eq!(calls.get(), 5, "a rebuilt base is read again");
    }

    #[test]
    fn a_before_is_rendered_once_per_photo_and_geometry() {
        let s = DevelopState::default();
        let calls = std::cell::Cell::new(0);
        let render = || {
            calls.set(calls.get() + 1);
            Ok::<_, ()>(BeforeRender { jpeg: vec![1, 2, 3], width: 30, height: 20, camera: false })
        };
        s.before(1, "a", render).unwrap();
        s.before(1, "a", render).unwrap();
        assert_eq!(calls.get(), 1, "the same geometry is served from the cache");
        s.before(1, "b", render).unwrap();
        assert_eq!(calls.get(), 2, "a new crop renders again");
        s.before(2, "a", render).unwrap();
        s.before(3, "a", render).unwrap();
        s.before(1, "b", render).unwrap();
        assert_eq!(calls.get(), 5, "two photos are kept");
        s.clear();
        s.before(3, "a", render).unwrap();
        assert_eq!(calls.get(), 6, "a library change forgets them");
    }

    #[test]
    fn a_region_at_half_scale_has_half_the_pixels() {
        let part = Linear::new(400, 300, [0.5; 3]);
        let inner = (20, 10, 200, 160);
        let full = finish_region(&part, inner, 1.0);
        let half = finish_region(&part, inner, 0.5);
        assert_eq!(full.dimensions(), (200, 160));
        assert_eq!(half.dimensions(), (100, 80));
        assert_eq!(finish_region(&part, inner, 2.0).dimensions(), (200, 160), "never above full resolution");
        assert_eq!(finish_region(&part, inner, 0.001).dimensions(), (1, 1), "never empty");
    }

    #[test]
    fn a_region_box_grows_by_its_margin_and_stays_in_the_frame() {
        let (outer, inner) = region_box(Crop { x: 0.5, y: 0.25, w: 0.25, h: 0.5 }, 400, 200, 20);
        assert_eq!(outer, (180, 30, 140, 140));
        assert_eq!(inner, (20, 20, 100, 100));
        let (outer, inner) = region_box(Crop { x: 0.0, y: 0.0, w: 0.1, h: 0.1 }, 400, 200, 20);
        assert_eq!(outer, (0, 0, 60, 40));
        assert_eq!(inner, (0, 0, 40, 20));
    }

    #[test]
    fn the_geometry_key_ignores_develop() {
        let a = Recipe { rotate: 90, ..Recipe::identity() };
        let mut b = a.clone();
        b.develop = Some(photos_core::edits::Develop { basic: photos_core::edits::Basic { exposure: 1.0, ..Default::default() }, ..Default::default() });
        assert_eq!(geometry_key(&a), geometry_key(&b));
        assert_ne!(geometry_key(&a), geometry_key(&Recipe::identity()));
    }
}
