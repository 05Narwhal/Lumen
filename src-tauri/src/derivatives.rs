//! Derivative generation: thumbhash + 256 px at ingest (Import lane), 1024 / 2560 on
//! first request (Interactive lane). Coalesced per `(hash, size)` by the scheduler.

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use photos_core::assets;
use photos_core::edits::Recipe;
use photos_core::edits::Segment;
use photos_core::export::{ExportDestination, ExportFormat, ExportLoss, ExportMeta, ExportOptions, LostTag, MetadataOutcome, VideoCut, EDITED_ORIGINAL_QUALITY};
use photos_media::encode::{encode_file, EncodeOptions, Format};
use photos_media::exiftool::Session;
use photos_media::metadata_transfer::{self, Expect, Target};
use photos_media::trim_export::trimmed_extension;
use photos_media::develop;
use photos_media::ExifTool;
use photos_import::Ingested;
use photos_jobs::Lane;
use photos_core::Library;
use photos_media::hdr::HdrKind;
use photos_media::{
    apply_orientation, decode_preview_hdr, dominant_color, effective_orientation, encode_jpeg, is_renderable_with, kind_for_extension,
    resize_long_edge, thumbhash_for, DerivativeStore, Ffmpeg, MediaKind, JPEG_QUALITY,
};
use tauri::{AppHandle, Manager};

pub use crate::error::{AppError, AppResult};
use crate::state::AppState;

const WAIT: Duration = Duration::from_secs(30);

/// How to produce a derivative of `size` for a file of `kind`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenderPlan {
    /// Decode the original directly (cheap formats, or the 2560 preview itself).
    DecodeOriginal,
    /// The 2560 preview exists: cut the smaller size from it.
    CutFromPreview,
    /// Expensive to decode (RAW, HEIC, video): make the 2560 preview once, then cut.
    PreviewFirstThenCut,
}

pub fn plan_render(kind: MediaKind, size: u32, preview_exists: bool) -> RenderPlan {
    if size >= 2560 {
        RenderPlan::DecodeOriginal
    } else if preview_exists {
        RenderPlan::CutFromPreview
    } else if matches!(kind, MediaKind::Raw | MediaKind::Heic | MediaKind::Video) {
        RenderPlan::PreviewFirstThenCut
    } else {
        RenderPlan::DecodeOriginal
    }
}

const FULL_QUALITY: u8 = 90;

/// Everything about one asset that decides how it renders. Passed as a unit because the
/// four facts always travel together and are always read from the same row.
#[derive(Debug, Clone, Copy)]
pub struct RenderSpec<'a> {
    pub hash: &'a str,
    pub source: &'a Path,
    pub kind: MediaKind,
    pub orientation: u16,
    pub hdr: HdrKind,
    /// The saved recipe and its version: renders go to the edit's own files.
    pub edit: Option<(&'a Recipe, i64)>,
    /// The AI rasters the recipe's masks name.
    pub ai_rasters: Option<&'a photos_core::ai_masks::AiRasters>,
}

impl<'a> RenderSpec<'a> {
    /// The spec for a catalogued asset, given where its original lives.
    pub fn from_info(info: &'a assets::RenderInfo, source: &'a Path) -> Self {
        Self {
            hash: &info.content_hash,
            source,
            kind: kind_for_extension(&info.extension),
            orientation: info.orientation as u16,
            hdr: HdrKind::from_code(info.hdr),
            edit: info.edit.as_ref().map(|(r, v)| (r, *v)),
            ai_rasters: Some(&info.ai_rasters),
        }
    }

    /// A develop render's context at full size.
    fn ctx(&self) -> develop::Ctx<'a> {
        develop::Ctx { ai_rasters: self.ai_rasters, ..develop::Ctx::new(1.0, develop::seed_for(self.hash)) }
    }
}

/// How much the decode of an image too large for the decoder's default limit may take (Round 22
/// §2): a quarter of the machine's memory, at least 512 MiB and at most 8 GiB. Read once, and
/// handed to `photos_media` so Develop's base uses the same.
pub fn oversize_budget() -> u64 {
    static BUDGET: std::sync::OnceLock<u64> = std::sync::OnceLock::new();
    *BUDGET.get_or_init(|| {
        use sysinfo::{MemoryRefreshKind, RefreshKind, System};
        let sys = System::new_with_specifics(RefreshKind::nothing().with_memory(MemoryRefreshKind::nothing().with_ram()));
        let budget = (sys.total_memory() / 4).clamp(512 << 20, 8 << 30);
        photos_media::decode::set_oversize_budget(budget);
        budget
    })
}

/// An original decoded for a thumbnail or the display size: whole, or, when it is too large for
/// the decoder, at `REDUCED_EDGE` with its real size alongside.
fn decode_original_or_reduced(path: &Path, kind: MediaKind, ffmpeg: Option<&Ffmpeg>, hdr: HdrKind) -> Result<(image::DynamicImage, Option<(u32, u32)>), photos_media::MediaError> {
    match decode_preview_hdr(path, kind, ffmpeg, hdr) {
        Err(e) if photos_media::decode::is_too_large(&e) => {
            let r = photos_media::decode::decode_reduced(path, kind, photos_media::decode::REDUCED_EDGE, oversize_budget())?;
            Ok((r.image, Some((r.real_width, r.real_height))))
        }
        got => got.map(|img| (img, None)),
    }
}

/// The longest edge a JPEG can have is 65 535; stay under it.
const JPEG_MAX_EDGE: u32 = 65_000;

/// The long edge for which an image of `real_w` × `real_h` scaled down (never up) takes at most
/// `budget` bytes at 4 bytes a pixel; the real long edge when it all fits.
fn largest_edge_within(real_w: u32, real_h: u32, budget: u64) -> u32 {
    let long = real_w.max(real_h);
    let bytes = real_w as f64 * real_h as f64 * 4.0;
    if bytes <= budget as f64 {
        return long;
    }
    ((long as f64 * (budget as f64 / bytes).sqrt()).floor() as u32).clamp(1, long)
}

/// For an image file the decoder can't open whole (its pixels at 4 bytes each pass the decoder's
/// default 512 MiB limit): the long edge `render_full` serves it at. `None` for everything else.
fn oversized_full_edge(source: &Path, kind: MediaKind) -> Option<u32> {
    if kind != MediaKind::Image {
        return None;
    }
    let (w, h) = photos_media::decode::header_size(source)?;
    if w as u64 * h as u64 * 4 <= 512 << 20 {
        return None;
    }
    Some(largest_edge_within(w, h, oversize_budget()).min(JPEG_MAX_EDGE))
}

/// The unedited image at full size, display-oriented: the input every edit starts from.
fn decode_unedited_full(store: &DerivativeStore, spec: &RenderSpec<'_>, ffmpeg: Option<&Ffmpeg>) -> AppResult<image::DynamicImage> {
    // An oversized original's served full image is for the viewer; edits work on the reduced one.
    let plain = if oversized_full_edge(spec.source, spec.kind).is_some() { spec.source.to_path_buf() } else { render_full(store, &RenderSpec { edit: None, ..*spec }, ffmpeg)? };
    if plain == spec.source {
        let img = decode_original_or_reduced(spec.source, spec.kind, ffmpeg, spec.hdr).map_err(media_err)?.0;
        Ok(apply_orientation(img, spec.orientation))
    } else {
        // A rendered full-size JPEG: already oriented, already SDR.
        decode_preview_hdr(&plain, MediaKind::Image, ffmpeg, HdrKind::None).map_err(media_err)
    }
}

/// The photo as shown, at full size: its edit applied, upright. What anything that paints
/// the photo itself must decode (slideshow stills) — `render_full`'s path can be a plain
/// JPEG's original, whose orientation tag a decoder does not apply.
pub fn decode_display_full(store: &DerivativeStore, spec: &RenderSpec<'_>, ffmpeg: Option<&Ffmpeg>) -> AppResult<image::DynamicImage> {
    if spec.edit.is_some() {
        // An edit's full size is always rendered by Lumen: upright, SDR, the edit applied.
        let full = render_full(store, spec, ffmpeg)?;
        return decode_preview_hdr(&full, MediaKind::Image, ffmpeg, HdrKind::None).map_err(media_err);
    }
    decode_unedited_full(store, spec, ffmpeg)
}

/// What the face scan looks at (Round 4B): the unedited photo, upright — the cached 2560
/// preview when there is one, else the original decoded in memory. Nothing new is cached
/// for a plain image (a HEIC or RAW still gets its full-size render, as it would anyway).
pub fn decode_for_scan(store: &DerivativeStore, spec: &RenderSpec<'_>, ffmpeg: Option<&Ffmpeg>) -> AppResult<image::DynamicImage> {
    if let Ok(img) = image::open(store.preview_path(spec.hash)) {
        return Ok(img);
    }
    decode_unedited_full(store, &RenderSpec { edit: None, ..*spec }, ffmpeg)
}

/// `decode_for_scan` for one asset.
pub fn scan_image_of(app: &AppHandle, asset_id: i64) -> AppResult<image::DynamicImage> {
    let state = app.state::<AppState>();
    let lib = state.library()?;
    let info = lib.db.with_conn(|c| assets::render_info(c, asset_id))?;
    let owned = OwnedSpec::new(&lib, info, false);
    let ffmpeg = state.ffmpeg();
    if !is_renderable_with(owned.kind, ffmpeg.is_some()) {
        return Err(not_renderable());
    }
    decode_for_scan(&DerivativeStore::new(&lib.root), &owned.spec(), ffmpeg.as_ref())
}

/// The recipe rendered at full size: from sensor data for a developed RAW (spec §3), else
/// from the decoded photo.
fn edited_full(store: &DerivativeStore, spec: &RenderSpec<'_>, ffmpeg: Option<&Ffmpeg>, recipe: &Recipe) -> AppResult<image::DynamicImage> {
    let base = if develop::develops_from_sensor(spec.kind, Some(recipe)) {
        develop::load_base(spec.source, spec.kind, ffmpeg, spec.hdr, spec.orientation, true, None).map_err(media_err)?.0
    } else {
        develop::Linear::from_dynamic(&decode_unedited_full(store, spec, ffmpeg)?)
    };
    // Stage 2: the denoised base, when the recipe asks for it and it has been built.
    let dn = crate::denoise::load_for(store, spec.hash, recipe);
    let base = crate::denoise::apply_denoised(base, recipe, dn.as_ref());
    Ok(image::DynamicImage::ImageRgb8(develop::render_base(&base, recipe, &spec.ctx())))
}

/// An export's denoised base (Phase 8d): built first when the recipe asks for denoise, the
/// file is missing and the model is there (this is already a background job); otherwise the
/// photo is exported without it, which is said in the log.
fn export_denoised(app: &AppHandle, asset_id: i64, owned: &OwnedSpec, cancel: &photos_jobs::CancelToken) -> Option<develop::Linear> {
    let recipe = owned.edit.as_ref().map(|(r, _)| r)?;
    crate::denoise::denoise_of(recipe)?;
    let state = app.state::<AppState>();
    let store = DerivativeStore::new(&state.library().ok()?.root);
    if let Some(dn) = crate::denoise::load_for(&store, &owned.hash, recipe) {
        return Some(dn);
    }
    let result = crate::denoise::build(app, asset_id, cancel);
    // Said like a build from the panel: Develop's progress ends, cached sizes are refreshed.
    if let Some(model) = photos_ml::manifest::manifest().by_role("denoise") {
        crate::denoise::complete(app, asset_id, &model.id, &result, cancel.is_cancelled());
    }
    match result {
        Ok(_) => crate::denoise::load_for(&store, &owned.hash, recipe),
        Err(e) => {
            tracing::warn!(asset_id, "export without its denoise: {}", e.message);
            None
        }
    }
}

/// Pure: decode the source, orient, fit to `size`, write JPEG into the store. Idempotent.
/// Expensive formats decode once for the 2560 preview; every smaller size is cut from it.
pub fn render(store: &DerivativeStore, size: u32, spec: &RenderSpec<'_>, ffmpeg: Option<&Ffmpeg>) -> AppResult<PathBuf> {
    if let Some((recipe, version)) = spec.edit {
        let out = store.edit_path(spec.hash, size, version);
        if out.is_file() {
            return Ok(out);
        }
        if size < 2560 {
            // Every smaller size is cut from the edited 2560, as for the originals.
            let preview = render(store, 2560, spec, ffmpeg)?;
            let img = decode_preview_hdr(&preview, MediaKind::Image, ffmpeg, HdrKind::None).map_err(media_err)?;
            encode_jpeg(&resize_long_edge(&img, size), JPEG_QUALITY, &out).map_err(media_err)?;
            return Ok(out);
        }
        // A crop needs the full frame's pixels, not the 2560 preview's, or a tight crop is soft.
        let edited = edited_full(store, spec, ffmpeg, recipe)?;
        encode_jpeg(&resize_long_edge(&edited, 2560), JPEG_QUALITY, &out).map_err(media_err)?;
        return Ok(out);
    }
    let out = store.path_for(spec.hash, size);
    if out.is_file() {
        return Ok(out);
    }
    let preview = store.preview_path(spec.hash);
    let plan = plan_render(spec.kind, size, preview.is_file());
    if plan == RenderPlan::PreviewFirstThenCut {
        render(store, 2560, spec, ffmpeg)?;
    }
    let (src, kind, orientation) = match plan {
        RenderPlan::DecodeOriginal => (spec.source, spec.kind, spec.orientation),
        // The preview already has orientation baked in.
        RenderPlan::CutFromPreview | RenderPlan::PreviewFirstThenCut => (preview.as_path(), MediaKind::Image, 1),
    };
    let img = decode_original_or_reduced(src, kind, ffmpeg, spec.hdr).map_err(media_err)?.0;
    let img = apply_orientation(img, orientation);
    let small = resize_long_edge(&img, size);
    encode_jpeg(&small, JPEG_QUALITY, &out).map_err(media_err)?;
    Ok(out)
}

/// The best full-resolution image the browser can show: RAW/HEIC get their decoded
/// full-size image rendered once into `derivatives/full/`; everything else is the original.
pub fn render_full(store: &DerivativeStore, spec: &RenderSpec<'_>, ffmpeg: Option<&Ffmpeg>) -> AppResult<PathBuf> {
    if let Some((recipe, version)) = spec.edit {
        let out = store.edit_full_path(spec.hash, version);
        if out.is_file() {
            return Ok(out);
        }
        let edited = edited_full(store, spec, ffmpeg, recipe)?;
        encode_jpeg(&edited, FULL_QUALITY, &out).map_err(media_err)?;
        return Ok(out);
    }
    if !matches!(spec.kind, MediaKind::Raw | MediaKind::Heic) {
        // Round 22 lane A: an original too large for the decoder is served as a JPEG of the
        // largest size that fits the memory budget; a browser can't open it as it is.
        let Some(edge) = oversized_full_edge(spec.source, spec.kind) else {
            return Ok(spec.source.to_path_buf());
        };
        let out = store.full_path(spec.hash);
        if out.is_file() {
            return Ok(out);
        }
        let r = photos_media::decode::decode_reduced(spec.source, spec.kind, edge, oversize_budget()).map_err(media_err)?;
        let img = apply_orientation(r.image, spec.orientation);
        encode_jpeg(&img, FULL_QUALITY, &out).map_err(media_err)?;
        return Ok(out);
    }
    let out = store.full_path(spec.hash);
    if out.is_file() {
        return Ok(out);
    }
    let img = decode_preview_hdr(spec.source, spec.kind, ffmpeg, spec.hdr).map_err(media_err)?;
    let img = apply_orientation(img, spec.orientation);
    encode_jpeg(&img, FULL_QUALITY, &out).map_err(media_err)?;
    Ok(out)
}

/// One ExifTool session for a whole export batch, started by the first file that needs it
/// (a batch of byte-for-byte copies never starts Perl). Jobs take turns: carrying and
/// reading back take milliseconds beside the encode.
pub struct Carrier {
    tool: ExifTool,
    session: Mutex<Option<Session>>,
}

impl Carrier {
    pub fn new(tool: ExifTool) -> Self {
        Carrier { tool, session: Mutex::new(None) }
    }

    fn with<T>(&self, f: impl FnOnce(&mut Session) -> Result<T, photos_media::MediaError>) -> Result<T, photos_media::MediaError> {
        let mut guard = self.session.lock().unwrap_or_else(|p| p.into_inner());
        if guard.is_none() {
            *guard = Some(self.tool.session()?);
        }
        let result = f(guard.as_mut().expect("session just started"));
        if result.is_err() {
            // A hung or dead process is not reused; the next file starts a fresh one.
            *guard = None;
        }
        result
    }
}

/// Where an export is written before it is complete: hidden, beside its final name, with
/// the same extension (ExifTool chooses its writer by the file's type).
pub fn part_path(out: &Path) -> PathBuf {
    let name = out.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    out.with_file_name(format!(".lumen-part-{name}"))
}

/// One exported file at `out`: the photo as shown in Lumen — edited, full size unless
/// `max_long_edge` says otherwise — as JPEG, PNG, WebP, AVIF or HEIC, or the original file
/// untouched. Videos, and photos exported as "Original" without an edit, are copied as they
/// are.
///
/// Everything is written to [`part_path`] and renamed onto `out` only once it is complete,
/// so a failure never leaves a half-written file under a real name. With `carry`, the
/// original's metadata is carried into the export and read back: the answer is what did not
/// survive (`None` when nothing was checked).
pub fn render_export(
    spec: &RenderSpec<'_>,
    ffmpeg: Option<&Ffmpeg>,
    options: &ExportOptions,
    carry: Option<(&Carrier, &ExportMeta)>,
    denoised: Option<&develop::Linear>,
    out: &Path,
) -> AppResult<Option<Vec<LostTag>>> {
    through_part(out, |part| write_export(spec, ffmpeg, options, carry, denoised, part))
}

/// Write with `write` into [`part_path`]`(out)`, then rename onto `out`; on failure remove
/// the part file.
fn through_part<T>(out: &Path, write: impl FnOnce(&Path) -> AppResult<T>) -> AppResult<T> {
    let part = part_path(out);
    match write(&part) {
        Ok(v) => {
            std::fs::rename(&part, out)?;
            Ok(v)
        }
        Err(e) => {
            let _ = std::fs::remove_file(&part);
            Err(e)
        }
    }
}

/// A trimmed video at `out`: the kept segments cut by FFmpeg, then (with `carry`) the
/// original's QuickTime tags carried back and read back, as for a photo.
pub fn render_trimmed_export(
    ffmpeg: Option<&Ffmpeg>,
    source: &Path,
    segments: &[Segment],
    cut: VideoCut,
    carry: Option<(&Carrier, &ExportMeta)>,
    out: &Path,
    cancel: &dyn Fn() -> bool,
) -> AppResult<Option<Vec<LostTag>>> {
    let Some(ff) = ffmpeg else {
        return Err(AppError { code: "ffmpeg_missing".into(), message: "Trimmed videos need FFmpeg, which is not installed.".into(), detail: None });
    };
    through_part(out, |part| {
        ff.export_trimmed(source, segments, cut, part, cancel).map_err(media_err)?;
        let Some((carrier, meta)) = carry else { return Ok(None) };
        let expect = Expect { target: Some(Target::Video), meta: meta.clone(), ..Default::default() };
        Ok(Some(carry_metadata(carrier, source, part, Target::Video, meta, None, &expect)))
    })
}

/// Carry and verify; a failure is reported as a loss, because the file itself is good.
fn carry_metadata(carrier: &Carrier, source: &Path, out: &Path, target: Target, meta: &ExportMeta, size: Option<(u32, u32)>, expect: &Expect) -> Vec<LostTag> {
    carrier
        .with(|s| {
            metadata_transfer::transfer(s, source, out, target, meta, size)?;
            metadata_transfer::verify(s, source, out, expect)
        })
        .unwrap_or_else(|e| vec![LostTag { tag: "Metadata".into(), before: "could not be carried".into(), after: e.to_string() }])
}

fn write_export(
    spec: &RenderSpec<'_>,
    ffmpeg: Option<&Ffmpeg>,
    options: &ExportOptions,
    carry: Option<(&Carrier, &ExportMeta)>,
    denoised: Option<&develop::Linear>,
    out: &Path,
) -> AppResult<Option<Vec<LostTag>>> {
    let format = options.effective_format(spec.edit.is_some());
    if format == ExportFormat::Original || spec.kind == MediaKind::Video {
        std::fs::copy(spec.source, out)?;
        return Ok(None);
    }
    // From the source itself, never from a cached derivative: those are JPEGs at quality 90,
    // and a PNG or lossless WebP must not be a JPEG underneath.
    let img = match spec.edit {
        Some((recipe, _)) => {
            let sensor = develop::develops_from_sensor(spec.kind, Some(recipe));
            let base = develop::load_base(spec.source, spec.kind, ffmpeg, spec.hdr, spec.orientation, sensor, None).map_err(media_err)?.0;
            let base = crate::denoise::apply_denoised(base, recipe, denoised);
            image::DynamicImage::ImageRgb8(develop::render_base(&base, recipe, &spec.ctx()))
        }
        None => apply_orientation(decode_preview_hdr(spec.source, spec.kind, ffmpeg, spec.hdr).map_err(media_err)?, spec.orientation),
    };
    let img = match options.max_long_edge {
        Some(edge) if img.width().max(img.height()) > edge => resize_long_edge(&img, edge),
        _ => img,
    };
    let (encoding, target) = match format {
        ExportFormat::Png => (Format::Png, Target::Png),
        ExportFormat::Webp => (Format::Webp, Target::Webp),
        ExportFormat::Avif => (Format::Avif, Target::Avif),
        ExportFormat::Heic => (Format::Heic, Target::Heic),
        ExportFormat::Tiff => (Format::Tiff, Target::Tiff),
        ExportFormat::Jpeg | ExportFormat::Original => (Format::Jpeg, Target::Jpeg),
    };
    let quality = if options.format == ExportFormat::Original { EDITED_ORIGINAL_QUALITY } else { options.quality.clamp(60, 100) };
    encode_file(&img, &EncodeOptions { format: encoding, quality, lossless: options.lossless }, ffmpeg, out).map_err(media_err)?;
    let Some((carrier, meta)) = carry else { return Ok(None) };
    let size = Some((img.width(), img.height()));
    let expect = Expect { size, target: Some(target), meta: meta.clone() };
    Ok(Some(carry_metadata(carrier, spec.source, out, target, meta, size, &expect)))
}

fn media_err(e: photos_media::MediaError) -> AppError {
    AppError {
        code: "derivative_failed".into(),
        message: "This photo could not be rendered.".into(),
        detail: Some(e.to_string()),
    }
}

fn wait_for(handle: photos_jobs::JobHandle, out: &Path) -> AppResult<PathBuf> {
    if !handle.wait_timeout(WAIT) {
        return Err(AppError { code: "derivative_timeout".into(), message: "Rendering took too long.".into(), detail: None });
    }
    if out.is_file() {
        Ok(out.to_path_buf())
    } else {
        Err(AppError { code: "derivative_failed".into(), message: "This photo could not be rendered.".into(), detail: None })
    }
}

/// A `RenderSpec` that owns its data, for jobs that run on the scheduler's threads.
#[derive(Clone)]
struct OwnedSpec {
    hash: String,
    source: PathBuf,
    kind: MediaKind,
    orientation: u16,
    hdr: HdrKind,
    edit: Option<(Recipe, i64)>,
    ai_rasters: photos_core::ai_masks::AiRasters,
}

impl OwnedSpec {
    /// `with_edit: false` renders the photo as imported, whatever its recipe.
    fn new(lib: &Library, info: assets::RenderInfo, with_edit: bool) -> Self {
        Self {
            source: lib.originals_dir().join(&info.relative_path),
            kind: kind_for_extension(&info.extension),
            orientation: info.orientation as u16,
            hdr: HdrKind::from_code(info.hdr),
            edit: if with_edit { info.edit } else { None },
            ai_rasters: if with_edit { info.ai_rasters } else { Default::default() },
            hash: info.content_hash,
        }
    }

    fn spec(&self) -> RenderSpec<'_> {
        RenderSpec {
            hash: &self.hash,
            source: &self.source,
            kind: self.kind,
            orientation: self.orientation,
            hdr: self.hdr,
            edit: self.edit.as_ref().map(|(r, v)| (r, *v)),
            ai_rasters: Some(&self.ai_rasters),
        }
    }

    fn out(&self, store: &DerivativeStore, size: u32) -> PathBuf {
        match &self.edit {
            Some((_, v)) => store.edit_path(&self.hash, size, *v),
            None => store.path_for(&self.hash, size),
        }
    }

    /// Jobs coalesce by key, so an edit's version is part of it: a job for the old version
    /// must not satisfy a request for the new one.
    fn key(&self, what: &str) -> String {
        match &self.edit {
            Some((_, v)) => format!("{what}:{}:v{v}", self.hash),
            None => format!("{what}:{}", self.hash),
        }
    }
}

/// Full-resolution path for the viewer (see `render_full`), generated on the Interactive lane.
pub fn ensure_full(app: &AppHandle, asset_id: i64) -> AppResult<PathBuf> {
    let state = app.state::<AppState>();
    let lib = state.library()?;
    let info = lib.db.with_conn(|c| assets::render_info(c, asset_id))?;
    let owned = OwnedSpec::new(&lib, info, true);
    if owned.edit.is_none() && !matches!(owned.kind, MediaKind::Raw | MediaKind::Heic) && oversized_full_edge(&owned.source, owned.kind).is_none() {
        return Ok(owned.source);
    }
    let ffmpeg = state.ffmpeg();
    if !is_renderable_with(owned.kind, ffmpeg.is_some()) {
        return Err(not_renderable());
    }
    let store = DerivativeStore::new(&lib.root);
    let out = match &owned.edit {
        Some((_, v)) => store.edit_full_path(&owned.hash, *v),
        None => store.full_path(&owned.hash),
    };
    if out.is_file() {
        return Ok(out);
    }
    let key = owned.key("full");
    let handle = state.jobs.submit(&key, Lane::Interactive, move |cancel| {
        if cancel.is_cancelled() {
            return;
        }
        if let Err(e) = render_full(&store, &owned.spec(), ffmpeg.as_ref()) {
            tracing::warn!(asset_id, "full render failed: {}", e.detail.unwrap_or(e.message));
        }
    });
    wait_for(handle, &out)
}

/// Warm the 1024 and 2560 derivatives for `ids` on the Prefetch lane. Returns at once.
pub fn prefetch(app: &AppHandle, ids: &[i64]) {
    let state = app.state::<AppState>();
    let Ok(lib) = state.library() else { return };
    let ffmpeg = state.ffmpeg();
    let store = DerivativeStore::new(&lib.root);
    for &id in ids {
        let Ok(info) = lib.db.with_conn(|c| assets::render_info(c, id)) else { continue };
        let owned = std::sync::Arc::new(OwnedSpec::new(&lib, info, true));
        if !is_renderable_with(owned.kind, ffmpeg.is_some()) {
            continue;
        }
        for size in [2560u32, 1024] {
            if owned.out(&store, size).is_file() {
                continue;
            }
            let key = owned.key(&format!("deriv:{size}"));
            let (store, owned, ffmpeg) = (store.clone(), owned.clone(), ffmpeg.clone());
            state.jobs.submit(&key, Lane::Prefetch, move |cancel| {
                if cancel.is_cancelled() {
                    return;
                }
                let _ = render(&store, size, &owned.spec(), ffmpeg.as_ref());
            });
        }
    }
}

/// Path of the derivative for `asset_id` at `size` — of the edit, when it has one —
/// generating it through the scheduler if needed.
pub fn ensure(app: &AppHandle, asset_id: i64, size: u32, lane: Lane) -> AppResult<PathBuf> {
    ensure_inner(app, asset_id, size, lane, true)
}

/// The 256 px thumbnail of the photo as shown (its edit included), if it is on disk.
pub fn small_thumb(app: &AppHandle, asset_id: i64) -> Option<PathBuf> {
    let state = app.state::<AppState>();
    let lib = state.library().ok()?;
    let info = lib.db.with_conn(|c| assets::render_info(c, asset_id)).ok()?;
    let owned = OwnedSpec::new(&lib, info, true);
    let out = owned.out(&DerivativeStore::new(&lib.root), 256);
    out.is_file().then_some(out)
}

/// As `ensure`, but the photo as imported, whatever its recipe: the editor starts from it.
pub fn ensure_original(app: &AppHandle, asset_id: i64, size: u32, lane: Lane) -> AppResult<PathBuf> {
    ensure_inner(app, asset_id, size, lane, false)
}

/// `ensure` on the calling thread, for work that is already a Maintenance job and must not
/// queue another and wait on it. The photo as imported: faces are placed on it, not on an edit.
pub fn render_now(app: &AppHandle, asset_id: i64, size: u32) -> AppResult<PathBuf> {
    let state = app.state::<AppState>();
    let lib = state.library()?;
    let info = lib.db.with_conn(|c| assets::render_info(c, asset_id))?;
    let owned = OwnedSpec::new(&lib, info, false);
    let ffmpeg = state.ffmpeg();
    if !is_renderable_with(owned.kind, ffmpeg.is_some()) {
        return Err(not_renderable());
    }
    let store = DerivativeStore::new(&lib.root);
    let out = owned.out(&store, size);
    if out.is_file() {
        return Ok(out);
    }
    render(&store, size, &owned.spec(), ffmpeg.as_ref())
}

fn ensure_inner(app: &AppHandle, asset_id: i64, size: u32, lane: Lane, with_edit: bool) -> AppResult<PathBuf> {
    let state = app.state::<AppState>();
    let lib = state.library()?;
    let info = lib.db.with_conn(|c| assets::render_info(c, asset_id))?;
    let owned = OwnedSpec::new(&lib, info, with_edit);
    let ffmpeg = state.ffmpeg();
    if !is_renderable_with(owned.kind, ffmpeg.is_some()) {
        return Err(not_renderable());
    }
    let store = DerivativeStore::new(&lib.root);
    let out = owned.out(&store, size);
    if out.is_file() {
        return Ok(out);
    }
    // This version of Lumen already failed on it: say so at once, without decoding or warning again.
    if let Some(why) = store.failure(&owned.hash, LUMEN_VERSION) {
        return Err(AppError { code: "derivative_failed".into(), message: "This photo could not be rendered.".into(), detail: Some(why) });
    }
    let key = owned.key(&format!("deriv:{size}"));
    let handle = state.jobs.submit(&key, lane, move |cancel| {
        if cancel.is_cancelled() {
            return;
        }
        let Err(e) = render(&store, size, &owned.spec(), ffmpeg.as_ref()) else { return };
        // A HEIF file named as another kind (`IMG_1855.JPG`): read it as what it is from now on.
        if owned.kind == MediaKind::Image && photos_media::is_heif_file(&owned.source) {
            let fixed = OwnedSpec { kind: MediaKind::Heic, ..owned.clone() };
            if lib.db.with_conn(|c| assets::set_extension(c, asset_id, "heic")).is_ok() && render(&store, size, &fixed.spec(), ffmpeg.as_ref()).is_ok() {
                tracing::info!(asset_id, "a HEIF file named as another kind is read as HEIF now");
                return;
            }
        }
        let why = one_line(&e.detail.unwrap_or(e.message)).to_string();
        store.record_failure(&owned.hash, LUMEN_VERSION, &why);
        tracing::warn!(asset_id, size, "derivative failed: {why}");
    });
    wait_for(handle, &out)
}

/// What a failure marker is written for: a new version of Lumen tries every failed photo once more.
const LUMEN_VERSION: &str = env!("CARGO_PKG_VERSION");

fn not_renderable() -> AppError {
    AppError { code: "not_renderable".into(), message: "This format needs FFmpeg to render.".into(), detail: None }
}

/// Import-lane job for a freshly catalogued file: decode once, store thumbhash, colour
/// and dimensions, write the 256 px thumbnail, then flag the library as changed.
pub fn schedule_ingest(app: AppHandle, ingested: Ingested) {
    let state = app.state::<AppState>();
    let ffmpeg = state.ffmpeg();
    if !is_renderable_with(ingested.kind, ffmpeg.is_some()) {
        return;
    }
    let Ok(lib) = state.library() else { return };
    let key = format!("ingest:{}", ingested.content_hash);
    let app2 = app.clone();
    state.jobs.submit(&key, Lane::Import, move |cancel| {
        if cancel.is_cancelled() {
            return;
        }
        match ingest_now(&lib, ffmpeg.as_ref(), &ingested) {
            Ok(()) => app2.state::<AppState>().changes.changed_background(&app2),
            Err(e) => {
                log_ingest_failure(ingested.asset_id, &e.detail.unwrap_or(e.message));
                // A file just taken off the grids: they redraw without it.
                if ffmpeg.is_some() && DerivativeStore::new(&lib.root).failed_with_ffmpeg(&ingested.content_hash) {
                    app2.state::<AppState>().changes.changed_background(&app2);
                }
            }
        }
    });
}

/// Why an original did not decode, as far as the log is concerned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum IngestFailure {
    /// The bytes are there but are not a whole, readable photo or movie (a copy cut short, a
    /// file that is not what its name says). `ingest_now` has marked it unreadable.
    Damaged,
    /// A real image bigger than the decoder's memory limit (a 100000 × 4000 PNG map).
    TooLarge,
    /// Anything else: a missing original, no FFmpeg, no RAW library, a bug.
    Other,
}

/// Sorts a decode failure by what the decoders and FFmpeg say, through the one list the Repair
/// page also reads (`photos_media::failure::cause`, Round 22 §5); the texts are the ones the
/// first launch on the real library logged.
fn classify_ingest_failure(said: &str) -> IngestFailure {
    use photos_media::failure::{cause, UnreadableCause};
    match cause(said) {
        UnreadableCause::TooLarge => IngestFailure::TooLarge,
        UnreadableCause::Incomplete | UnreadableCause::NotAnImage => IngestFailure::Damaged,
        UnreadableCause::Unknown => IngestFailure::Other,
    }
}

/// A damaged or oversized original is a state the photo itself shows ("could not be rendered"),
/// remembered so start-up stops retrying: one info line. Anything else is a warning.
fn log_ingest_failure(asset: i64, said: &str) {
    match classify_ingest_failure(said) {
        IngestFailure::Damaged => tracing::info!(asset, "no thumbnail: the original is incomplete or damaged ({})", one_line(said)),
        IngestFailure::TooLarge => tracing::info!(asset, "no thumbnail: the image is too large to decode"),
        IngestFailure::Other => tracing::warn!(asset, "ingest failed: {}", one_line(said)),
    }
}

/// FFmpeg's stderr is a page per failure; its last line says what went wrong.
fn one_line(detail: &str) -> &str {
    detail.lines().map(str::trim).rfind(|l| !l.is_empty()).unwrap_or(detail)
}

/// Synchronous body of the ingest job (also driven directly by tests): decode once,
/// store thumbhash, colour and dimensions, write the 256 px thumbnail.
///
/// The orientation still owed to the pixels is decided here, by comparing what the
/// decoder returned with what EXIF recorded, and then written back to the catalogue. Every
/// later stage — the remaining derivative sizes, the justified layout's aspect ratios —
/// reads that corrected value, so the decision is made once, against real pixels, instead
/// of each stage trusting a tag that can be wrong in two different directions.
pub fn ingest_now(lib: &Library, ffmpeg: Option<&Ffmpeg>, ingested: &Ingested) -> AppResult<()> {
    let store = DerivativeStore::new(&lib.root);
    // A HEIF saved as `IMG_1855.JPG`, or a movie saved as `IMG_1730.JPG` / `IMG_1768.HEIC`, is
    // decoded as what it is (`ensure_inner` already does so for HEIF); the first launch on the
    // real library logged these as JPEG/PNG parse errors and "Invalid data".
    let named_otherwise = matches!(ingested.kind, MediaKind::Image | MediaKind::Heic | MediaKind::Video)
        .then(|| photos_media::content_kind(&ingested.original))
        .flatten()
        .filter(|k| *k != ingested.kind);
    let kind = named_otherwise.unwrap_or(ingested.kind);
    // Too large for the decoder is not damaged (Round 22 §2): decoded reduced, catalogued at its
    // real size.
    let (img, real) = match decode_original_or_reduced(&ingested.original, kind, ffmpeg, HdrKind::from_code(ingested.hdr)) {
        Ok(got) => got,
        Err(e) => {
            // The bytes are there and do not decode: remembered, so start-up stops retrying.
            // A missing original (drive offline) or RAW library is not marked.
            if ingested.original.is_file() && !photos_media::develop::raw::is_no_library(&e) {
                store.mark_unreadable(&ingested.content_hash, ffmpeg.is_some());
                // With FFmpeg there to try, nothing can read it: off every grid until Try again.
                if ffmpeg.is_some() {
                    let _ = lib.db.with_conn(|c| photos_core::unreadable::set_unreadable(c, ingested.asset_id, true));
                }
            }
            return Err(media_err(e));
        }
    };
    // The size the decoder found; for a reduced decode the real one, so the orientation check
    // compares like with like and the catalogue keeps the real pixel count.
    let decoded = real.unwrap_or((img.width(), img.height()));
    if named_otherwise.is_some() {
        // Read as what it is from now on (the catalogue's extension drives every later render).
        let video = kind == MediaKind::Video;
        let _ = lib.db.with_conn(|c| assets::set_container(c, ingested.asset_id, if video { "mp4" } else { "heic" }, video));
    }
    let orientation = effective_orientation(ingested.orientation, ingested.exif_size, decoded, photos_media::decoder_orients(kind));
    let oriented = apply_orientation(img, orientation);
    let hash = thumbhash_for(&oriented);
    let colour = dominant_color(&oriented);
    let small = resize_long_edge(&oriented, 256);
    encode_jpeg(&small, JPEG_QUALITY, &store.thumb_path(&ingested.content_hash, 256)).map_err(media_err)?;
    // Curation facts for memories, from the same 256 px image every photo has.
    let (sharp, phash) = (photos_media::sharpness(&small), photos_media::dhash(&small));
    lib.db.with_conn(|c| {
        assets::set_derived(c, ingested.asset_id, &hash, &colour, Some(decoded.0 as i64), Some(decoded.1 as i64))?;
        assets::set_quality(c, ingested.asset_id, sharp, phash)?;
        // It reads now (Try again, a newer reader): back in the grids.
        photos_core::unreadable::set_unreadable(c, ingested.asset_id, false)?;
        assets::set_decoded_geometry(c, ingested.asset_id, orientation as i64, decoded.0 as i64, decoded.1 as i64)
    })?;
    Ok(())
}

/// Curation facts for photos imported before memories existed, from their 256 px thumbnails
/// (cheap: no original is decoded). Returns how many were filled; call until 0.
pub fn backfill_quality(app: &AppHandle) -> usize {
    let state = app.state::<AppState>();
    let Ok(lib) = state.library() else { return 0 };
    let Ok(rows) = lib.db.with_conn(|c| assets::list_missing_quality(c, 500)) else { return 0 };
    let store = DerivativeStore::new(&lib.root);
    let mut n = 0;
    for (id, hash) in rows {
        let Ok(img) = image::open(store.thumb_path(&hash, 256)) else {
            // No thumbnail on disk: mark it so the backfill does not retry it for ever.
            let _ = lib.db.with_conn(|c| assets::set_quality(c, id, 0.0, 0));
            continue;
        };
        // Measured before the connection is taken: it is only for the write.
        let (sharp, phash) = (photos_media::sharpness(&img), photos_media::dhash(&img));
        if lib.db.with_conn(|c| assets::set_quality(c, id, sharp, phash)).is_ok() {
            n += 1;
        }
    }
    n
}

/// Where the playback proxy for `asset_id` lives and whether it exists yet.
pub fn proxy_status(app: &AppHandle, asset_id: i64) -> AppResult<(PathBuf, bool)> {
    let state = app.state::<AppState>();
    let lib = state.library()?;
    let info = lib.db.with_conn(|c| assets::render_info(c, asset_id))?;
    let out = DerivativeStore::new(&lib.root).proxy_path(&info.content_hash);
    let ready = out.is_file();
    Ok((out, ready))
}

/// Start making the playback proxy on the Import lane (coalesced per hash), which a transcode
/// can hold for a while without starving thumbnails. Emits `proxy://progress` while running
/// and `proxy://finished` once. Returns at once.
pub fn start_proxy(app: &AppHandle, asset_id: i64) -> AppResult<()> {
    start_proxy_on(app, asset_id, Lane::Import)
}

/// The video's probe, when import or playback stored one.
fn stored_probe(lib: &photos_core::Library, asset_id: i64) -> Option<photos_media::Probe> {
    let json = lib.db.with_conn(|c| assets::video_metadata(c, asset_id)).ok()??;
    serde_json::from_str::<photos_media::Metadata>(&json).ok()?.probe
}

/// Whether the viewer will ask for `asset_id`'s proxy: a video this WebView cannot play as it is.
fn needs_proxy(app: &AppHandle, lib: &photos_core::Library, asset_id: i64) -> bool {
    let support = *app.state::<AppState>().video_support.read().expect("video support lock");
    stored_probe(lib, asset_id).is_some_and(|p| photos_media::playback_with(&p, std::env::consts::OS, support) == photos_media::Playback::Proxy)
}

/// Makes the proxies of `ids` ahead of time on `lane` (Round 8A): the viewer's neighbours, and
/// the newest videos. Only videos that need one, and only those not made yet.
pub fn prefetch_proxies(app: &AppHandle, ids: &[i64], lane: Lane) {
    let state = app.state::<AppState>();
    let (Ok(lib), Some(_)) = (state.library(), state.ffmpeg()) else { return };
    for &id in ids {
        if needs_proxy(app, &lib, id) {
            let _ = start_proxy_on(app, id, lane);
        }
    }
}

/// How many of the newest videos get their proxy ahead of time.
const NEWEST_PROXIES: i64 = 12;

/// The newest videos' proxies, on the Maintenance lane (Round 8A).
pub fn prefetch_newest_proxies(app: &AppHandle) {
    let Ok(lib) = app.state::<AppState>().library() else { return };
    if let Ok(newest) = lib.db.with_conn(|c| assets::newest_video_ids(c, NEWEST_PROXIES)) {
        prefetch_proxies(app, &newest, Lane::Maintenance);
    }
}

/// `start_proxy` on `lane`. A remux when only the container or the sound keeps the WebView
/// from playing the video (most of a phone's: a Live Photo's PCM sound), else a transcode,
/// on the graphics card when it can.
pub fn start_proxy_on(app: &AppHandle, asset_id: i64, lane: Lane) -> AppResult<()> {
    proxy_job(app, asset_id, lane, false).map(|_| ())
}

/// [`start_proxy_on`], returning the job to wait on (`None` when the proxy is already there).
/// `background` (Round 9A's video prep: nobody is waiting) runs FFmpeg below normal priority.
pub fn proxy_job(app: &AppHandle, asset_id: i64, lane: Lane, background: bool) -> AppResult<Option<photos_jobs::JobHandle>> {
    let state = app.state::<AppState>();
    let lib = state.library()?;
    let Some(ffmpeg) = state.ffmpeg() else { return Err(not_renderable()) };
    let info = lib.db.with_conn(|c| assets::render_info(c, asset_id))?;
    let store = DerivativeStore::new(&lib.root);
    let out = store.proxy_path(&info.content_hash);
    if out.is_file() {
        return Ok(None);
    }
    let original = lib.originals_dir().join(&info.relative_path);
    let support = *state.video_support.read().expect("video support lock");
    let probe = stored_probe(&lib, asset_id).or_else(|| ffmpeg.probe(&original).ok());
    let size = std::fs::metadata(&original).ok().map(|m| m.len());
    let plan = probe.as_ref().map_or(photos_media::ProxyPlan::Transcode, |p| photos_media::proxy_plan(p, size, std::env::consts::OS, support));
    let hevc = probe.as_ref().and_then(|p| p.video_codec.as_deref()) == Some("hevc");
    let hardware = state.settings.read().expect("settings lock").playback.hardware_encoding.unwrap_or(true);
    let key = format!("proxy:{}", info.content_hash);
    let app2 = app.clone();
    let handle = state.jobs.submit(&key, lane, move |cancel| {
        use tauri::Emitter;
        let mut last = -1.0f32;
        let mut on_progress = |p: f32| {
            if p - last >= 0.02 || p >= 1.0 {
                last = p;
                let _ = app2.emit(crate::events::PROXY_PROGRESS, crate::events::ProxyProgress { asset_id, progress: p });
            }
        };
        let started = std::time::Instant::now();
        let encoder = match plan {
            photos_media::ProxyPlan::Remux { .. } => None,
            photos_media::ProxyPlan::Transcode => Some(proxy_encoder(background, || photos_media::slideshow::best_encoder(&ffmpeg, hardware))),
        };
        let make = |encoder: Option<photos_media::slideshow::Encoder>, on_progress: &mut dyn FnMut(f32)| {
            let video = encoder.map(|e| e.proxy_args()).unwrap_or_default();
            ffmpeg.make_proxy(&original, &out, plan, &video, hevc, background, on_progress, &|| cancel.is_cancelled())
        };
        let mut result = make(encoder, &mut on_progress);
        // A graphics card that refuses (too many sessions, a driver reset) leaves it to x264.
        if result.is_err() && !cancel.is_cancelled() && encoder.is_some_and(|e| e != photos_media::slideshow::Encoder::X264) {
            result = make(Some(photos_media::slideshow::Encoder::X264), &mut on_progress);
        }
        // A remux the stream would not take: encode it after all.
        if result.is_err() && !cancel.is_cancelled() && encoder.is_none() {
            let video = photos_media::slideshow::Encoder::X264.proxy_args();
            result = ffmpeg.make_proxy(&original, &out, photos_media::ProxyPlan::Transcode, &video, false, background, &mut on_progress, &|| cancel.is_cancelled());
        }
        match &result {
            Ok(()) => tracing::info!(asset_id, ?plan, encoder = encoder.map(|e| e.ffmpeg_name()), ms = started.elapsed().as_millis() as u64, "proxy made"),
            Err(e) => tracing::warn!(asset_id, "proxy failed: {}", one_line(&e.to_string())),
        }
        let _ = app2.emit(crate::events::PROXY_FINISHED, crate::events::ProxyFinished { asset_id, ok: result.is_ok(), background });
    });
    Ok(Some(handle))
}

/// The encoder for a transcoded proxy: the fastest (`best`: the graphics card's when there is
/// one and *Hardware encoding* is on), in the background too. Round 9A took x264 there to save
/// space (NVENC writes up to the 8 Mbit/s ceiling where x264 wrote 1–8), but x264 below normal
/// priority took 54 minutes for one video on 2026-10-08, on a CPU the rest of the app needs,
/// while a GPU encoder uses next to none (Round 15). `background` is kept for the call sites.
fn proxy_encoder(_background: bool, best: impl FnOnce() -> photos_media::slideshow::Encoder) -> photos_media::slideshow::Encoder {
    best()
}

/// One file of an export batch, its name already reserved.
struct ExportJob {
    id: i64,
    owned: OwnedSpec,
    meta: ExportMeta,
    /// A video's kept segments, when it is trimmed.
    trim: Option<Vec<Segment>>,
    out: PathBuf,
}

/// What an export batch's jobs share.
struct ExportBatch {
    done: std::sync::atomic::AtomicU32,
    failed: std::sync::atomic::AtomicU32,
    /// Some file's metadata was carried and read back.
    checked: std::sync::atomic::AtomicBool,
    losses: Mutex<Vec<ExportLoss>>,
    /// Each file written, and the asset it was made from.
    made: Mutex<Vec<(PathBuf, i64)>>,
    carrier: Option<Carrier>,
}

/// The batch is written: announce it, or first import it when it goes into the library
/// (and into `album`, when one was chosen).
fn deliver(app: &AppHandle, b: &ExportBatch, library_dir: Option<PathBuf>, album: Option<i64>, event: crate::events::ExportFinished) {
    use tauri::Emitter;
    let made = std::mem::take(&mut *b.made.lock().unwrap_or_else(|p| p.into_inner()));
    match library_dir {
        None => {
            let _ = app.emit(crate::events::EXPORT_FINISHED, event);
        }
        Some(dir) => add_exports_to_library(app, dir, made, album, event),
    }
}

/// Removes an into-the-library export's temporary folder and announces the batch when it is
/// dropped: after the import's `after` has run, or unrun when the import failed or was
/// cancelled, so neither the folder nor the announcement is ever lost.
struct LibraryAdd {
    app: AppHandle,
    dir: PathBuf,
    event: Option<crate::events::ExportFinished>,
}

impl Drop for LibraryAdd {
    fn drop(&mut self) {
        use tauri::Emitter;
        if let Err(e) = std::fs::remove_dir_all(&self.dir) {
            tracing::warn!(dir = %self.dir.display(), "could not remove an export's temporary folder: {e}");
        }
        if let Some(event) = self.event.take() {
            let _ = self.app.emit(crate::events::EXPORT_FINISHED, event);
        }
    }
}

/// Import `made` like any chosen files (copied into `originals/`, derivatives, memories),
/// then point each new photo at the one it was exported from. Files are matched to their
/// new assets by content hash, which is the catalogue's key. A file already in the library
/// is a duplicate: reported, not added — but still put in `album` when one was chosen, since
/// the user asked for those pictures there.
fn add_exports_to_library(app: &AppHandle, dir: PathBuf, made: Vec<(PathBuf, i64)>, album: Option<i64>, event: crate::events::ExportFinished) {
    let links: Vec<(String, i64)> = made.iter().filter_map(|(p, id)| photos_import::content_hash(p).ok().map(|h| (h, *id))).collect();
    let files: Vec<PathBuf> = made.into_iter().map(|(p, _)| p).collect();
    let guard = LibraryAdd { app: app.clone(), dir, event: Some(event) };
    if files.is_empty() {
        return;
    }
    let state = app.state::<AppState>();
    let after: crate::commands::import::AfterImport = Box::new(move |summary, _source| {
        let mut guard = guard;
        if let Ok(lib) = guard.app.state::<AppState>().library() {
            let linked = lib.db.with_conn(|c| {
                for (hash, source) in &links {
                    assets::link_derived(c, hash, *source)?;
                }
                Ok(())
            });
            if let Err(e) = linked {
                tracing::warn!("could not link exports to their sources: {e}");
            }
        }
        let album_name = album.and_then(|album| {
            let state = guard.app.state::<AppState>();
            let lib = state.library().ok()?;
            let hashes: Vec<String> = links.iter().map(|(h, _)| h.clone()).collect();
            let named = lib.db.with_conn(|c| {
                let ids = assets::ids_by_hash(c, &hashes)?;
                photos_core::albums::add_to_album(c, album, &ids, photos_core::now_unix())?;
                Ok(photos_core::albums::get_album(c, album)?.name)
            });
            match named {
                Ok(name) => {
                    state.changes.changed(&guard.app, true);
                    Some(name)
                }
                Err(e) => {
                    tracing::warn!("could not add exports to album {album}: {e}");
                    None
                }
            }
        });
        if let Some(event) = guard.event.as_mut() {
            event.library = Some(crate::events::AddedToLibrary { added: summary.imported as u32, duplicates: summary.skipped as u32, album: album_name });
        }
    });
    if let Err(e) = crate::commands::import::spawn_import(app, &state, Box::new(photos_import::FilesSource::new(files)), None, None, Some(after)) {
        // `after` was dropped unrun, and the guard with it: the folder is gone and the batch
        // is announced as written but not added.
        tracing::warn!("could not import exports: {}", e.detail.unwrap_or(e.message));
    }
}

/// Export `ids` into the chosen folder (default: the library's `exports`). One Import-lane
/// job per photo, so the activity ring counts them and a failure costs one file; names are
/// reserved up front, so two photos called `IMG_1` never race for one file. Returns the
/// batch id at once; `export://progress` and `export://finished` report the rest.
pub fn start_export(app: &AppHandle, ids: Vec<i64>, options: ExportOptions) -> AppResult<u64> {
    use std::sync::atomic::Ordering;
    use std::sync::Arc;
    use tauri::Emitter;
    let state = app.state::<AppState>();
    let lib = state.library()?;
    let ffmpeg = state.ffmpeg();
    if options.format.needs_ffmpeg() && ffmpeg.is_none() {
        return Err(AppError {
            code: "ffmpeg_missing".into(),
            message: "AVIF and HEIC need FFmpeg, which is not installed.".into(),
            detail: None,
        });
    }
    let batch = state.export_batches.fetch_add(1, Ordering::Relaxed) + 1;
    // Into the library: written beside it first, then imported like any other files.
    let (into_library, album) = match options.destination {
        ExportDestination::Library { album_id } => (true, album_id),
        ExportDestination::Folder { .. } => (false, None),
    };
    let dir = match &options.destination {
        ExportDestination::Folder { dir } => dir.as_ref().map(PathBuf::from).unwrap_or_else(|| lib.root.join("exports")),
        ExportDestination::Library { .. } => lib.root.join("tmp").join(format!("export-{batch}")),
    };
    std::fs::create_dir_all(&dir)?;
    if !dir.is_dir() {
        return Err(AppError { code: "not_a_folder".into(), message: "That path is not a folder.".into(), detail: Some(dir.display().to_string()) });
    }
    let jobs: Vec<ExportJob> = ids
        .iter()
        .filter_map(|&id| {
            lib.db.with_conn(|c| Ok((assets::export_render_info(c, id)?, photos_core::export::export_meta(c, id)?, photos_core::edits::get_trim(c, id)?))).ok().map(|(info, meta, trim)| (id, info, meta, trim))
        })
        .filter_map(|(id, info, meta, trim)| {
            // `render_info` leaves a trim out (it changes no pixel of a thumbnail), so it is
            // read on its own.
            let edited = info.edit.is_some();
            let video = kind_for_extension(&info.extension) == MediaKind::Video;
            let trim = trim.filter(|_| video);
            let name = match &trim {
                Some(_) => photos_core::export::trimmed_name(&info.filename, trimmed_extension(&info.extension, options.video_cut)),
                None => photos_core::export::export_name(&info.filename, if video { ExportFormat::Original } else { options.format }, edited),
            };
            let out = assets::unique_export_name(&dir, &name);
            // Reserve the name now; the finished file is renamed over the empty one.
            std::fs::File::create(&out).ok()?;
            Some(ExportJob { id, owned: OwnedSpec::new(&lib, info, true), meta, trim, out })
        })
        .collect();
    let total = jobs.len() as u32;
    let dir_label = if into_library { String::new() } else { dir.display().to_string() };
    let exiftool = if options.metadata { state.exiftool() } else { None };
    let unavailable = options.metadata && exiftool.is_none();
    let batch_state = Arc::new(ExportBatch {
        done: std::sync::atomic::AtomicU32::new(0),
        failed: std::sync::atomic::AtomicU32::new(0),
        checked: std::sync::atomic::AtomicBool::new(false),
        losses: Mutex::new(Vec::new()),
        made: Mutex::new(Vec::new()),
        carrier: exiftool.map(Carrier::new),
    });
    let finished = move |b: &ExportBatch, done: u32, failed: u32, dir: String| {
        let metadata = if unavailable {
            MetadataOutcome::Unavailable
        } else if b.checked.load(Ordering::SeqCst) {
            MetadataOutcome::Verified
        } else {
            MetadataOutcome::Skipped
        };
        let losses = std::mem::take(&mut *b.losses.lock().unwrap_or_else(|p| p.into_inner()));
        let library = into_library.then_some(crate::events::AddedToLibrary { added: 0, duplicates: 0, album: None });
        crate::events::ExportFinished { batch, done, failed, dir, metadata, losses, library }
    };
    let library_dir = into_library.then(|| dir.clone());
    if total == 0 {
        deliver(app, &batch_state, library_dir, album, finished(&batch_state, 0, 0, dir_label));
        return Ok(batch);
    }
    let finished = Arc::new(finished);
    for ExportJob { id, owned, meta, trim, out } in jobs {
        let (app2, b, options, ffmpeg, dir_label, finished, library_dir) =
            (app.clone(), batch_state.clone(), options.clone(), ffmpeg.clone(), dir_label.clone(), finished.clone(), library_dir.clone());
        state.jobs.submit(&format!("export:{batch}:{id}"), Lane::Import, move |cancel| {
            let carry = b.carrier.as_ref().map(|c| (c, &meta));
            let result = match &trim {
                Some(segments) => render_trimmed_export(ffmpeg.as_ref(), &owned.source, segments, options.video_cut, carry, &out, &|| cancel.is_cancelled()),
                None => {
                    let denoised = export_denoised(&app2, id, &owned, cancel);
                    render_export(&owned.spec(), ffmpeg.as_ref(), &options, carry, denoised.as_ref(), &out)
                }
            };
            let ok = match result {
                Ok(lost) => {
                    if let Some(lost) = lost {
                        b.checked.store(true, Ordering::SeqCst);
                        if !lost.is_empty() {
                            let file = out.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                            tracing::warn!(asset = id, file, ?lost, "export lost metadata");
                            b.losses.lock().unwrap_or_else(|p| p.into_inner()).push(ExportLoss { asset_id: id, file, lost });
                        }
                    }
                    b.made.lock().unwrap_or_else(|p| p.into_inner()).push((out.clone(), id));
                    true
                }
                Err(e) => {
                    tracing::warn!(asset = id, "export failed: {}", e.detail.unwrap_or(e.message));
                    let _ = std::fs::remove_file(&out);
                    false
                }
            };
            let (done, failed) = if ok {
                (b.done.fetch_add(1, Ordering::SeqCst) + 1, b.failed.load(Ordering::SeqCst))
            } else {
                (b.done.load(Ordering::SeqCst), b.failed.fetch_add(1, Ordering::SeqCst) + 1)
            };
            let _ = app2.emit(crate::events::EXPORT_PROGRESS, crate::events::ExportProgress { batch, done, failed, total });
            if done + failed == total {
                deliver(&app2, &b, library_dir, album, finished(&b, done, failed, dir_label));
            }
        });
    }
    Ok(batch)
}

/// Assets with a cached size (256, 1024, the 2560 preview or the full size) that is landscape
/// where the catalogue says portrait, or the reverse: rendered under an orientation that was
/// later corrected (HEICs imported before FFmpeg's own rotation was trusted), and never
/// redrawn because `render` keeps what is on disk. Only headers are read. Square-ish photos
/// (within 5 %) cannot disagree.
pub fn misshapen(store: &DerivativeStore, rows: &[assets::DerivedShape]) -> Vec<i64> {
    rows.iter()
        .filter_map(|(id, hash, w, h, orientation)| {
            let (w, h) = if photos_media::orientation_swaps(*orientation as u16) { (*h, *w) } else { (*w, *h) };
            if (w - h).abs() * 20 < w.max(h) {
                return None;
            }
            // Every size on disk, not only the thumbnail: a preview rendered under an old
            // orientation rule outlives a thumbnail redrawn under the new one (Round 4A).
            let sizes = [store.thumb_path(hash, 256), store.thumb_path(hash, 1024), store.preview_path(hash), store.full_path(hash)];
            sizes
                .iter()
                .filter_map(|p| image::image_dimensions(p).ok())
                .any(|(tw, th)| (tw > th) != (w > h))
                .then_some(*id)
        })
        .collect()
}

/// Sends every misshapen asset (see `misshapen`) back through ingest. Idempotent: once
/// redrawn, a thumbnail agrees with the catalogue and is left alone.
pub fn refix_shapes(app: &AppHandle) -> usize {
    let state = app.state::<AppState>();
    let Ok(lib) = state.library() else { return 0 };
    let Ok(rows) = lib.db.with_conn(assets::derived_shapes) else { return 0 };
    let ids = misshapen(&DerivativeStore::new(&lib.root), &rows);
    let n = lib.db.with_conn(|c| assets::mark_underived(c, &ids)).unwrap_or(0);
    if n > 0 {
        tracing::info!(n, "thumbnails with the wrong shape will be redrawn");
    }
    n
}

/// Round 22 lane C: the photo as Lumen shows it, written as a full-size TIFF at `out` (the copy
/// *Edit in* hands an editor).
pub fn render_tiff(app: &AppHandle, lib: &photos_core::Library, id: i64, out: &Path) -> AppResult<()> {
    let owned = OwnedSpec::new(lib, lib.db.with_conn(|c| assets::export_render_info(c, id))?, true);
    let options = ExportOptions { format: ExportFormat::Tiff, quality: 100, lossless: true, max_long_edge: None, metadata: false, destination: Default::default(), video_cut: Default::default() };
    let denoised = export_denoised(app, id, &owned, &photos_jobs::CancelToken::default());
    render_export(&owned.spec(), app.state::<AppState>().ffmpeg().as_ref(), &options, None, denoised.as_ref(), out).map(|_| ())
}

/// What ingest needs to decode `id` again, from its catalogue row.
pub(crate) fn ingested_from(lib: &photos_core::Library, id: i64, info: &assets::RenderInfo, kind: MediaKind) -> Ingested {
    Ingested {
        asset_id: id,
        content_hash: info.content_hash.clone(),
        original: lib.originals_dir().join(&info.relative_path),
        kind,
        orientation: info.orientation as u16,
        // Already-normalised rows compare equal to what the decoder returns, so
        // re-running this is a no-op rather than a second correction.
        exif_size: match (info.width, info.height) {
            (Some(w), Some(h)) if w > 0 && h > 0 => Some((w as u32, h as u32)),
            _ => None,
        },
        hdr: info.hdr,
    }
}

/// Flags the assets whose unreadable mark says FFmpeg was there and the original is on disk
/// (Round 18 C1): the marks are files, so SQL can't do this. Idempotent; only the derived
/// `unreadable` flag changes. A file that is offline is never flagged, and a mark made without
/// FFmpeg never counts. Returns how many were newly flagged.
pub fn flag_unreadable_from_marks(lib: &photos_core::Library) -> usize {
    let n = flag_marked(&lib.db, &DerivativeStore::new(&lib.root), &lib.originals_dir());
    if n > 0 {
        tracing::info!(n, "files no reader can open taken off the grids");
    }
    n
}

/// [`flag_unreadable_from_marks`] with the three places named, so a replay can read the marks
/// and originals of the real library (read only) while writing a copy of its database.
fn flag_marked(db: &photos_core::Db, store: &DerivativeStore, originals: &Path) -> usize {
    let Ok(ids) = db.with_conn(|c| assets::list_underived(c, 1_000_000)) else { return 0 };
    let mut n = 0;
    for id in ids {
        let Ok(info) = db.with_conn(|c| assets::render_info(c, id)) else { continue };
        if !store.failed_with_ffmpeg(&info.content_hash) || !originals.join(&info.relative_path).is_file() {
            continue;
        }
        if db.with_conn(|c| photos_core::unreadable::set_unreadable(c, id, true)).unwrap_or(false) {
            n += 1;
        }
    }
    n
}

/// *Try again* (Round 18 C1): forgets the marks of these flagged files (nothing else) and sends
/// them through ingest. A file that now reads comes back on its own (ingest clears its flag);
/// one that still fails stays flagged. Returns how many were queued.
pub fn retry_unreadable(app: &AppHandle, ids: &[i64]) -> usize {
    let state = app.state::<AppState>();
    let Ok(lib) = state.library() else { return 0 };
    let ffmpeg = state.ffmpeg_available();
    let store = DerivativeStore::new(&lib.root);
    let mut n = 0;
    for &id in ids {
        let Ok(info) = lib.db.with_conn(|c| assets::render_info(c, id)) else { continue };
        let kind = kind_for_extension(&info.extension);
        if !is_renderable_with(kind, ffmpeg) {
            continue;
        }
        // The flag stays until ingest decides: a success clears it, a failure keeps it, so a
        // file that still can't be read never flashes back onto a grid as a broken tile.
        store.forget_unreadable(&info.content_hash);
        schedule_ingest(app.clone(), ingested_from(&lib, id, &info, kind));
        n += 1;
    }
    n
}

/// Re-queue ingest for assets that have no thumbhash yet and are renderable now (FFmpeg
/// appeared after they were imported, or a crash lost the job). Returns how many were queued.
///
/// Anything derived earlier is discarded first. A re-ingest re-decides orientation from
/// the decoded pixels, so a derivative rendered under the old answer would otherwise
/// survive: `render` returns early when the file is already on disk.
pub fn requeue_underived(app: &AppHandle) -> usize {
    let state = app.state::<AppState>();
    let Ok(lib) = state.library() else { return 0 };
    let ffmpeg = state.ffmpeg_available();
    let Ok(ids) = lib.db.with_conn(|c| assets::list_underived(c, 5000)) else { return 0 };
    let store = DerivativeStore::new(&lib.root);
    let mut n = 0;
    for id in ids {
        let Ok(info) = lib.db.with_conn(|c| assets::render_info(c, id)) else { continue };
        let kind = kind_for_extension(&info.extension);
        if !is_renderable_with(kind, ffmpeg) || store.is_unreadable(&info.content_hash, ffmpeg) {
            continue;
        }
        store.delete_all(&info.content_hash);
        schedule_ingest(app.clone(), ingested_from(&lib, id, &info, kind));
        n += 1;
    }
    if n > 0 {
        tracing::info!(n, "re-queued ingest for underived assets");
    }
    n
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Round 15's measurement: x264 against the best encoder on this PC, at below-normal
    /// priority as the video prep runs them, on clips COPIED out of a library. Writes only to a
    /// temp dir.
    ///
    /// LUMEN_PROXY_CLIPS="<clip>;<clip>" cargo test -p lumen --lib proxy_encoders_on_clips -- --ignored --nocapture
    #[test]
    #[ignore = "needs FFmpeg and video clips"]
    fn proxy_encoders_on_clips() {
        use photos_media::slideshow::Encoder;
        let ffmpeg = photos_media::ffmpeg::Ffmpeg::locate().expect("ffmpeg");
        let best = photos_media::slideshow::best_encoder(&ffmpeg, true);
        let out = tempfile::tempdir().unwrap();
        for clip in std::env::var("LUMEN_PROXY_CLIPS").unwrap().split(';') {
            let path = std::path::Path::new(clip);
            let probe = ffmpeg.probe(path).unwrap();
            let hevc = probe.video_codec.as_deref() == Some("hevc");
            for encoder in [Encoder::X264, best] {
                let mp4 = out.path().join(format!("{}.mp4", encoder.ffmpeg_name()));
                let t = std::time::Instant::now();
                let r = ffmpeg.make_proxy(path, &mp4, photos_media::ProxyPlan::Transcode, &encoder.proxy_args(), hevc, true, &mut |_| {}, &|| false);
                let mb = std::fs::metadata(&mp4).map(|m| m.len() as f64 / 1e6).unwrap_or(0.0);
                println!("{clip} {:?} {} ms {mb:.1} MB {:?}", encoder, t.elapsed().as_millis(), r.err());
                let _ = std::fs::remove_file(&mp4);
            }
        }
    }

    #[test]
    fn background_transcodes_use_the_hardware_encoder() {
        use photos_media::slideshow::Encoder;
        assert_eq!(proxy_encoder(true, || Encoder::Nvenc), Encoder::Nvenc);
        assert_eq!(proxy_encoder(true, || Encoder::X264), Encoder::X264, "no GPU encoder: x264");
        assert_eq!(proxy_encoder(false, || Encoder::Nvenc), Encoder::Nvenc, "someone waiting: the fastest");
    }

    #[test]
    fn ffmpeg_failures_log_their_last_line() {
        let detail = "[in#0 @ 0001] moov atom not found\n[in#0 @ 0002] Error opening input\nError opening input files: Invalid data found when processing input\n";
        assert_eq!(one_line(detail), "Error opening input files: Invalid data found when processing input");
        assert_eq!(one_line("unexpected end of file"), "unexpected end of file");
    }

    #[test]
    fn expensive_kinds_render_the_preview_first() {
        assert_eq!(plan_render(MediaKind::Raw, 256, false), RenderPlan::PreviewFirstThenCut);
        assert_eq!(plan_render(MediaKind::Video, 1024, false), RenderPlan::PreviewFirstThenCut);
        assert_eq!(plan_render(MediaKind::Raw, 256, true), RenderPlan::CutFromPreview);
        assert_eq!(plan_render(MediaKind::Raw, 2560, false), RenderPlan::DecodeOriginal);
        assert_eq!(plan_render(MediaKind::Image, 256, false), RenderPlan::DecodeOriginal);
        assert_eq!(plan_render(MediaKind::Image, 256, true), RenderPlan::CutFromPreview);
    }

    #[test]
    fn render_cuts_smaller_sizes_from_the_preview() {
        let tmp = tempfile::tempdir().unwrap();
        let store = DerivativeStore::new(tmp.path());
        let src = tmp.path().join("big.jpg");
        image::ImageBuffer::from_fn(3000, 2000, |x, _| image::Rgb([(x % 256) as u8, 40u8, 90u8]))
            .save(&src)
            .unwrap();
        let spec = RenderSpec { hash: "ab12cd34", source: &src, kind: MediaKind::Image, orientation: 1, hdr: HdrKind::None, edit: None, ai_rasters: None };
        let p2560 = render(&store, 2560, &spec, None).unwrap();
        assert!(p2560.is_file());
        let p256 = render(&store, 256, &spec, None).unwrap();
        let img = image::open(&p256).unwrap();
        assert_eq!((img.width(), img.height()), (256, 171));
        assert_eq!(render(&store, 256, &spec, None).unwrap(), p256, "idempotent");
        assert_eq!(render_full(&store, &spec, None).unwrap(), src, "plain images serve the original");
    }

    #[test]
    fn a_heif_named_jpg_is_ingested_as_heif_not_parsed_as_jpeg() {
        let tmp = tempfile::tempdir().unwrap();
        let lib = Library::create(&tmp.path().join("lib")).unwrap();
        let file = tmp.path().join("IMG_1730.JPG");
        std::fs::write(&file, b"\0\0\0\x18ftypheic\0\0\0\0mif1heic").unwrap();
        let ingested = Ingested { asset_id: 0, content_hash: "cafe0001".into(), original: file, kind: MediaKind::Image, orientation: 1, exif_size: None, hdr: 0 };
        let err = ingest_now(&lib, None, &ingested).unwrap_err();
        let said = err.detail.unwrap_or(err.message);
        assert!(!said.contains("Illegal start bytes"), "decoded as a JPEG: {said}");
        assert!(said.contains("needs FFmpeg"), "not routed to the HEIF path: {said}");
    }

    /// The first launch on the real library: movies saved as `IMG_1730.JPG` / `IMG_1768.HEIC` and a
    /// HEIF saved as `IMG_2028.MOV` were handed to the wrong decoder ("Illegal start bytes",
    /// "Invalid data", an HEVC abort). The container's brand decides.
    #[test]
    fn a_file_is_ingested_as_what_its_container_says_not_what_its_name_says() {
        let tmp = tempfile::tempdir().unwrap();
        let lib = Library::create(&tmp.path().join("lib")).unwrap();
        let said = |name: &str, brand: &[u8; 4], kind: MediaKind| {
            let file = tmp.path().join(name);
            std::fs::write(&file, [b"\0\0\0\x14ftyp".as_slice(), brand, b"\0\0\0\0"].concat()).unwrap();
            let ingested = Ingested { asset_id: 0, content_hash: format!("cafe{name}"), original: file, kind, orientation: 1, exif_size: None, hdr: 0 };
            let err = ingest_now(&lib, None, &ingested).unwrap_err();
            err.detail.unwrap_or(err.message)
        };
        let said_of_movie = said("IMG_1730.JPG", b"qt  ", MediaKind::Image);
        assert!(said_of_movie.contains("Video needs FFmpeg"), "a movie saved as .JPG was not routed to the video path: {said_of_movie}");
        let said_of_movie = said("IMG_1768.HEIC", b"mp42", MediaKind::Heic);
        assert!(said_of_movie.contains("Video needs FFmpeg"), "a movie saved as .HEIC was not routed to the video path: {said_of_movie}");
        let said_of_heif = said("IMG_2028.MOV", b"heic", MediaKind::Video);
        assert!(said_of_heif.contains("Heic needs FFmpeg"), "a HEIF saved as .MOV was not routed to the HEIF path: {said_of_heif}");
    }

    /// The rest of the first launch's warnings (52 photos, all with the bytes there): every one is
    /// an original that is cut short, or an image too big for the decoder. Both are states
    /// the person is told about on the photo itself, not warnings in the log.
    #[test]
    fn ingest_failures_from_the_real_log_are_told_apart() {
        use IngestFailure::{Damaged, Other, TooLarge};
        for said in [
            "decode failed: unexpected end of file",
            "decode failed: Format error decoding Jpeg: Error parsing image. Illegal start bytes:0",
            "decode failed: Format error decoding Png: Invalid PNG signature.",
            "Error opening input files: Invalid data found when processing input",
            "[in#0 @ 0001] moov atom not found\n[in#0 @ 0002] Error opening input: Invalid data found when processing input\nError opening input files: Invalid data found when processing input",
            "[hevc @ 0001] Invalid NAL unit size (74149 > 5092).\n[vist#0:7/hevc @ 0002] [dec:hevc @ 0003] Terminating thread with return code -1145393733 (Error number -1145393733 occurred)",
        ] {
            assert_eq!(classify_ingest_failure(said), Damaged, "{said}");
        }
        assert_eq!(classify_ingest_failure("decode failed: Memory limit exceeded"), TooLarge);
        for said in ["HEIC needs FFmpeg", "decode failed: Permission denied (os error 5)", "RAW library missing", ""] {
            assert_eq!(classify_ingest_failure(said), Other, "{said}");
        }
    }

    #[test]
    fn a_cut_short_png_and_a_cut_short_movie_fail_as_damaged_originals() {
        let tmp = tempfile::tempdir().unwrap();
        let lib = Library::create(&tmp.path().join("lib")).unwrap();
        let fail = |name: &str, kind: MediaKind, ff: Option<&Ffmpeg>| {
            let ingested = Ingested { asset_id: 0, content_hash: format!("dead{name}"), original: tmp.path().join(name), kind, orientation: 1, exif_size: None, hdr: 0 };
            let err = ingest_now(&lib, ff, &ingested).unwrap_err();
            classify_ingest_failure(&err.detail.unwrap_or(err.message))
        };
        // A PNG whose second half never arrived (hairstreak32_Wu.png in the real library).
        let png = tmp.path().join("hairstreak.png");
        image::RgbImage::from_fn(200, 200, |x, y| image::Rgb([(x * y % 256) as u8, (x % 256) as u8, (y % 256) as u8])).save(&png).unwrap();
        let bytes = std::fs::read(&png).unwrap();
        std::fs::write(&png, &bytes[..bytes.len() / 2]).unwrap();
        assert_eq!(fail("hairstreak.png", MediaKind::Image, None), IngestFailure::Damaged);
        // Not an image at all, saved as IMG_1755.PNG.
        std::fs::write(tmp.path().join("IMG_1755.PNG"), [0u8; 64]).unwrap();
        assert_eq!(fail("IMG_1755.PNG", MediaKind::Image, None), IngestFailure::Damaged);

        let Some(ff) = Ffmpeg::locate() else {
            eprintln!("note: FFmpeg not found; skipping the movie half");
            return;
        };
        // A movie copied only half way (no `moov` atom yet), saved as IMG_1768.HEIC.
        let mov = tmp.path().join("IMG_1768.HEIC");
        let st = std::process::Command::new(&ff.ffmpeg)
            .args(["-v", "error", "-y", "-f", "lavfi", "-i", "testsrc=size=320x240:rate=10", "-t", "2", "-pix_fmt", "yuv420p", "-f", "mp4"])
            .arg(&mov)
            .status()
            .unwrap();
        assert!(st.success());
        let bytes = std::fs::read(&mov).unwrap();
        std::fs::write(&mov, &bytes[..bytes.len() / 2]).unwrap();
        assert_eq!(fail("IMG_1768.HEIC", MediaKind::Heic, Some(&ff)), IngestFailure::Damaged);
    }

    /// Replays start-up's ingest of every underived photo, as the first launch did, and prints
    /// how each ends. `LUMEN_COPY` is a **copy** of the library folder's `library.db` (and
    /// nothing else: thumbnails land beside it); `LUMEN_LIBRARY` is the real library, whose
    /// originals are only read. No outcome may be `Other` (that is a warning in the log).
    /// `LUMEN_COPY=<scratch folder with library.db> LUMEN_LIBRARY=<library> cargo test -p lumen --lib ingest_replay_on_a_copy -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn ingest_replay_on_a_copy() {
        let copy = PathBuf::from(std::env::var("LUMEN_COPY").unwrap());
        let real = PathBuf::from(std::env::var("LUMEN_LIBRARY").unwrap());
        assert_ne!(copy.canonicalize().unwrap(), real.canonicalize().unwrap(), "the replay writes: never point it at the real library");
        let lib = Library::open(&copy).unwrap();
        let ff = Ffmpeg::locate();
        let ids = lib.db.with_conn(|c| assets::list_underived(c, 5000)).unwrap();
        let (mut done, mut damaged, mut too_large, mut other) = (0, 0, 0, Vec::new());
        for id in &ids {
            let info = lib.db.with_conn(|c| assets::render_info(c, *id)).unwrap();
            let ingested = Ingested {
                asset_id: *id,
                content_hash: info.content_hash.clone(),
                original: real.join("originals").join(&info.relative_path),
                kind: kind_for_extension(&info.extension),
                orientation: info.orientation as u16,
                exif_size: None,
                hdr: info.hdr,
            };
            match ingest_now(&lib, ff.as_ref(), &ingested) {
                Ok(()) => done += 1,
                Err(e) => match classify_ingest_failure(&e.detail.clone().unwrap_or(e.message.clone())) {
                    IngestFailure::Damaged => damaged += 1,
                    IngestFailure::TooLarge => too_large += 1,
                    IngestFailure::Other => other.push((*id, one_line(&e.detail.unwrap_or(e.message)).to_string())),
                },
            }
        }
        println!("{} underived: {done} read, {damaged} damaged, {too_large} too large, {} other: {other:?}", ids.len(), other.len());
        assert!(other.is_empty(), "these would still be warnings: {other:?}");
    }

    /// Round 18 C1 on a copy: `LUMEN_COPY` is a scratch folder with a **copy** of `library.db`;
    /// `LUMEN_LIBRARY` is the real library, whose marks and originals are only read. Prints how
    /// many files the backfill flags (Julian's library: 52 of 21,865).
    /// `LUMEN_COPY=<scratch> LUMEN_LIBRARY=<library> cargo test -p lumen --lib unreadable_backfill_on_a_copy -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn unreadable_backfill_on_a_copy() {
        let copy = PathBuf::from(std::env::var("LUMEN_COPY").unwrap());
        let real = PathBuf::from(std::env::var("LUMEN_LIBRARY").unwrap());
        assert_ne!(copy.canonicalize().unwrap(), real.canonicalize().unwrap(), "the replay writes: never point it at the real library");
        let db = photos_core::Db::open(&copy.join("library.db")).unwrap();
        let store = DerivativeStore::new(&real);
        let first = flag_marked(&db, &store, &real.join("originals"));
        let again = flag_marked(&db, &store, &real.join("originals"));
        let (listed, hidden) = db
            .with_conn(|c| {
                let listed = photos_core::unreadable::count(c)?;
                let hidden: i64 = c.query_row("SELECT COUNT(*) FROM assets WHERE unreadable = 1 AND concealed = 1", [], |r| r.get(0))?;
                Ok((listed, hidden))
            })
            .unwrap();
        println!("flagged {first}, again {again}, listed {listed}, concealed {hidden}");
        assert_eq!(again, 0, "idempotent");
        assert_eq!(listed, hidden);
    }

    #[test]
    fn the_backfill_flags_only_marks_that_say_ffmpeg_tried_and_whose_original_is_there() {
        use photos_core::assets::{insert_assets, NewAsset};
        let tmp = tempfile::tempdir().unwrap();
        let lib = Library::create(&tmp.path().join("lib")).unwrap();
        let store = DerivativeStore::new(&lib.root);
        let new = |h: &str| NewAsset { content_hash: h.into(), relative_path: format!("{h}.jpg"), filename: format!("{h}.jpg"), extension: "jpg".into(), media_type: 0, byte_size: 1, imported_at: 1, ..Default::default() };
        let ids = lib.db.with_conn(|c| insert_assets(c, &[new("ab000001"), new("ab000002"), new("ab000003"), new("ab000004")])).unwrap();
        std::fs::create_dir_all(lib.originals_dir()).unwrap();
        for h in ["ab000001", "ab000002", "ab000004"] {
            std::fs::write(lib.originals_dir().join(format!("{h}.jpg")), b"bytes").unwrap();
        }
        store.mark_unreadable("ab000001", true); // tried with FFmpeg, original there: flagged
        store.mark_unreadable("ab000002", false); // no FFmpeg was there: not evidence
        store.mark_unreadable("ab000003", true); // original offline: never flagged
        // ab000004 has no mark at all (still queued).
        assert_eq!(flag_unreadable_from_marks(&lib), 1);
        assert_eq!(flag_unreadable_from_marks(&lib), 0, "idempotent");
        let list = lib.db.with_conn(|c| photos_core::unreadable::list(c, None, 10)).unwrap();
        assert_eq!(list.items.iter().map(|i| i.id).collect::<Vec<_>>(), vec![ids[0]]);
    }

    #[test]
    fn ingest_flags_a_file_ffmpeg_could_not_read_and_a_later_success_clears_it() {
        use photos_core::assets::{insert_assets, NewAsset};
        let Some(ff) = Ffmpeg::locate() else {
            eprintln!("note: FFmpeg not found; skipping");
            return;
        };
        let tmp = tempfile::tempdir().unwrap();
        let lib = Library::create(&tmp.path().join("lib")).unwrap();
        let file = tmp.path().join("cut.png");
        image::RgbImage::from_fn(200, 200, |x, y| image::Rgb([(x * y % 256) as u8, (x % 256) as u8, (y % 256) as u8])).save(&file).unwrap();
        let whole = std::fs::read(&file).unwrap();
        let id = lib
            .db
            .with_conn(|c| insert_assets(c, &[NewAsset { content_hash: "ab000005".into(), relative_path: "cut.png".into(), filename: "cut.png".into(), extension: "png".into(), media_type: 0, byte_size: 1, imported_at: 1, ..Default::default() }]))
            .unwrap()[0];
        let flagged = || lib.db.with_conn(photos_core::unreadable::count).unwrap();
        let ingested = Ingested { asset_id: id, content_hash: "ab000005".into(), original: file.clone(), kind: MediaKind::Image, orientation: 1, exif_size: None, hdr: 0 };

        std::fs::write(&file, &whole[..whole.len() / 2]).unwrap();
        ingest_now(&lib, None, &ingested).unwrap_err();
        assert_eq!(flagged(), 0, "without FFmpeg it is only marked, never hidden");
        ingest_now(&lib, Some(&ff), &ingested).unwrap_err();
        assert_eq!(flagged(), 1, "with FFmpeg's say-so it is hidden");

        // Try again with the whole file: the marks go, ingest succeeds, the flag clears.
        std::fs::write(&file, &whole).unwrap();
        DerivativeStore::new(&lib.root).forget_unreadable("ab000005");
        ingest_now(&lib, Some(&ff), &ingested).unwrap();
        assert_eq!(flagged(), 0);
    }

    /// One asset in a fresh temp library for an original at `file`.
    fn one_asset(lib: &Library, hash: &str, file: &Path) -> Ingested {
        use photos_core::assets::{insert_assets, NewAsset};
        let name = file.file_name().unwrap().to_string_lossy().into_owned();
        let ext = file.extension().unwrap().to_string_lossy().into_owned();
        let id = lib
            .db
            .with_conn(|c| insert_assets(c, &[NewAsset { content_hash: hash.into(), relative_path: name.clone(), filename: name, extension: ext, media_type: 0, byte_size: 1, imported_at: 1, ..Default::default() }]))
            .unwrap()[0];
        Ingested { asset_id: id, content_hash: hash.into(), original: file.to_path_buf(), kind: MediaKind::Image, orientation: 1, exif_size: None, hdr: 0 }
    }

    /// Round 22 §2: Julian's 846 MB map PNG sat among the damaged files. 16000 × 12000 RGB is
    /// 576 MB, just over the decoder's 512 MiB default.
    /// A 16000 × 12000 RGB PNG (576 MB decoded: over the decoder's 512 MiB default) at `file`.
    fn write_oversized_png(file: &Path) {
        use std::io::Write;
        let (w, h) = (16000u32, 12000u32);
        let mut enc = png::Encoder::new(std::io::BufWriter::new(std::fs::File::create(file).unwrap()), w, h);
        enc.set_color(png::ColorType::Rgb);
        enc.set_compression(png::Compression::Fast);
        let mut wr = enc.write_header().unwrap().into_stream_writer().unwrap();
        let row: Vec<u8> = (0..w).flat_map(|x| [(x / 63) as u8, 120, 40]).collect();
        for _ in 0..h {
            wr.write_all(&row).unwrap();
        }
        wr.finish().unwrap();
    }

    #[test]
    fn an_oversized_original_ingests_and_is_not_flagged() {
        let tmp = tempfile::tempdir().unwrap();
        let lib = Library::create(&tmp.path().join("lib")).unwrap();
        let file = tmp.path().join("map.png");
        write_oversized_png(&file);
        let ingested = one_asset(&lib, "ab220001", &file);
        let ff = Ffmpeg::locate();

        ingest_now(&lib, ff.as_ref(), &ingested).expect("ingests reduced");
        assert_eq!(lib.db.with_conn(photos_core::unreadable::count).unwrap(), 0);
        assert!(DerivativeStore::new(&lib.root).thumb_path("ab220001", 256).is_file());
        let size: (i64, i64) = lib
            .db
            .with_conn(|c| Ok(c.query_row("SELECT width, height FROM assets WHERE id = ?1", [ingested.asset_id], |r| Ok((r.get(0)?, r.get(1)?)))?))
            .unwrap();
        assert_eq!(size, (16000, 12000), "catalogued at its real size");
    }

    /// Round 22 final seam: a file flagged "too large" before the reduced decode existed leaves
    /// lane B's page once ingest has read it (the counts and the list both come up empty).
    #[test]
    fn a_flagged_oversized_file_is_healthy_on_the_repair_page_once_ingested() {
        let tmp = tempfile::tempdir().unwrap();
        let lib = Library::create(&tmp.path().join("lib")).unwrap();
        let file = tmp.path().join("map.png");
        write_oversized_png(&file);
        let ingested = one_asset(&lib, "ab220002", &file);
        lib.db.with_conn(|c| photos_core::unreadable::set_unreadable(c, ingested.asset_id, true)).unwrap();
        DerivativeStore::new(&lib.root).record_failure("ab220002", "26.1.0-alpha", "decode failed: Memory limit exceeded");
        let before = crate::library_health::health(&lib).unwrap();
        assert_eq!((before.unreadable, before.causes.len()), (1, 1), "flagged as too large to start with");

        ingest_now(&lib, Ffmpeg::locate().as_ref(), &ingested).expect("ingests reduced");

        let after = crate::library_health::health(&lib).unwrap();
        assert_eq!((after.total, after.unreadable, after.causes.len()), (1, 0, 0));
        assert!(crate::library_health::unreadable_page(&lib, false, None, 10).unwrap().items.is_empty());
    }

    #[test]
    fn the_largest_edge_that_fits_a_budget() {
        let long = largest_edge_within(100_000, 4_000, 512 << 20);
        assert!((57_800..58_000).contains(&long), "{long}");
        assert_eq!(largest_edge_within(4_000, 3_000, 512 << 20), 4_000, "fits: the real long edge");
        assert_eq!(largest_edge_within(3_000, 4_000, 512 << 20), 4_000, "portrait too");
        // The scaled size really is within the budget.
        let (w, h) = (16_000u64, 12_000u64);
        let e = largest_edge_within(16_000, 12_000, 512 << 20) as u64;
        assert!(e * (e * h / w) * 4 <= 512 << 20);
    }

    /// Round 22 §2: the viewer's full-size source for an image the decoder can't open whole is a
    /// JPEG at the largest size that fits; a normal image is served as it is.
    #[test]
    fn render_full_serves_an_oversized_original_reduced() {
        use std::io::Write;
        let tmp = tempfile::tempdir().unwrap();
        let big = tmp.path().join("map.png");
        let (w, h) = (16000u32, 12000u32);
        let mut enc = png::Encoder::new(std::io::BufWriter::new(std::fs::File::create(&big).unwrap()), w, h);
        enc.set_color(png::ColorType::Rgb);
        enc.set_compression(png::Compression::Fast);
        let mut wr = enc.write_header().unwrap().into_stream_writer().unwrap();
        let row: Vec<u8> = (0..w).flat_map(|x| [(x / 63) as u8, 120, 40]).collect();
        for _ in 0..h {
            wr.write_all(&row).unwrap();
        }
        wr.finish().unwrap();
        let small = tmp.path().join("small.png");
        image::RgbImage::new(64, 48).save(&small).unwrap();
        let store = DerivativeStore::new(&tmp.path().join("lib"));
        let spec = |hash, source| RenderSpec { hash, source, kind: MediaKind::Image, orientation: 1, hdr: HdrKind::None, edit: None, ai_rasters: None };

        assert_eq!(render_full(&store, &spec("ab220003", &small), None).unwrap(), small, "a normal image is served as it is");
        let out = render_full(&store, &spec("ab220004", &big), None).unwrap();
        assert_ne!(out, big);
        let (ow, oh) = photos_media::decode::header_size(&out).unwrap();
        assert_eq!(ow.max(oh), largest_edge_within(w, h, oversize_budget()), "the largest size that fits");
        assert!(ow <= w, "never larger than the original");
        assert_eq!(render_full(&store, &spec("ab220004", &big), None).unwrap(), out, "made once");
    }

    #[test]
    fn ingest_still_flags_a_truncated_jpeg() {
        let Some(ff) = Ffmpeg::locate() else {
            eprintln!("note: FFmpeg not found; skipping");
            return;
        };
        let tmp = tempfile::tempdir().unwrap();
        let lib = Library::create(&tmp.path().join("lib")).unwrap();
        let file = tmp.path().join("cut.jpg");
        image::RgbImage::from_fn(300, 200, |x, y| image::Rgb([(x % 256) as u8, (y % 256) as u8, 90])).save(&file).unwrap();
        // Cut inside its headers: a JPEG cut later decodes with grey filling the rest.
        let whole = std::fs::read(&file).unwrap();
        std::fs::write(&file, &whole[..100]).unwrap();
        let ingested = one_asset(&lib, "ab220002", &file);

        ingest_now(&lib, Some(&ff), &ingested).unwrap_err();
        assert_eq!(lib.db.with_conn(photos_core::unreadable::count).unwrap(), 1);
    }

    #[test]
    fn a_movie_saved_as_jpg_gets_its_poster_and_is_catalogued_as_a_video() {
        use photos_core::assets::{insert_assets, NewAsset};
        let Some(ff) = Ffmpeg::locate() else {
            eprintln!("note: FFmpeg not found; skipping");
            return;
        };
        let tmp = tempfile::tempdir().unwrap();
        let lib = Library::create(&tmp.path().join("lib")).unwrap();
        let clip = tmp.path().join("IMG_1730.JPG");
        let st = std::process::Command::new(&ff.ffmpeg)
            .args(["-v", "error", "-y", "-f", "lavfi", "-i", "testsrc=size=320x240:rate=10", "-t", "2", "-pix_fmt", "yuv420p", "-f", "mp4"])
            .arg(&clip)
            .status()
            .unwrap();
        assert!(st.success());
        let id = lib
            .db
            .with_conn(|c| {
                insert_assets(
                    c,
                    &[NewAsset { content_hash: "cafe0002".into(), relative_path: "IMG_1730.JPG".into(), filename: "IMG_1730.JPG".into(), extension: "jpg".into(), media_type: 0, byte_size: 1, orientation: 1, imported_at: 1, ..Default::default() }],
                )
            })
            .unwrap()[0];
        let ingested = Ingested { asset_id: id, content_hash: "cafe0002".into(), original: clip, kind: MediaKind::Image, orientation: 1, exif_size: None, hdr: 0 };
        ingest_now(&lib, Some(&ff), &ingested).unwrap();
        let thumb = image::open(DerivativeStore::new(&lib.root).thumb_path("cafe0002", 256)).unwrap();
        assert_eq!((thumb.width(), thumb.height()), (256, 192));
        let (ext, media_type): (String, i64) = lib.db.with_conn(|c| Ok(c.query_row("SELECT extension, media_type FROM assets WHERE id = ?1", [id], |r| Ok((r.get(0)?, r.get(1)?)))?)).unwrap();
        assert_eq!((ext.as_str(), media_type), ("mp4", 1), "the catalogue reads it as a video from now on");
    }

    #[test]
    fn video_needs_ffmpeg_and_gets_a_poster_thumbnail() {
        use photos_core::assets::{insert_assets, NewAsset};
        let tmp = tempfile::tempdir().unwrap();
        let lib = Library::create(&tmp.path().join("lib")).unwrap();
        let clip = tmp.path().join("clip.mp4");
        let ingested = Ingested { asset_id: 0, content_hash: "feedface".into(), original: clip.clone(), kind: MediaKind::Video, orientation: 1, exif_size: None, hdr: 0 };

        // Without FFmpeg the job cannot run and says so.
        std::fs::write(&clip, b"not a video").unwrap();
        let err = ingest_now(&lib, None, &ingested).unwrap_err();
        assert_eq!(err.code, "derivative_failed");
        let store = DerivativeStore::new(&lib.root);
        assert!(store.is_unreadable("feedface", false), "start-up does not retry it");
        assert!(!store.is_unreadable("feedface", true), "until FFmpeg appears");
        store.delete_all("feedface");

        let Some(ff) = Ffmpeg::locate() else {
            eprintln!("note: FFmpeg not found; skipping the poster half");
            return;
        };
        let st = std::process::Command::new(&ff.ffmpeg)
            .args(["-v", "error", "-y", "-f", "lavfi", "-i", "testsrc=size=320x240:rate=10", "-t", "2", "-pix_fmt", "yuv420p"])
            .arg(&clip)
            .status()
            .unwrap();
        assert!(st.success());
        let id = lib
            .db
            .with_conn(|c| {
                insert_assets(
                    c,
                    &[NewAsset {
                        content_hash: "feedface".into(),
                        relative_path: "clip.mp4".into(),
                        filename: "clip.mp4".into(),
                        extension: "mp4".into(),
                        media_type: 1,
                        byte_size: 1,
                        orientation: 1,
                        imported_at: 1,
                        ..Default::default()
                    }],
                )
            })
            .unwrap()[0];
        let ingested = Ingested { asset_id: id, ..ingested };
        ingest_now(&lib, Some(&ff), &ingested).unwrap();
        let store = DerivativeStore::new(&lib.root);
        let thumb = image::open(store.thumb_path("feedface", 256)).unwrap();
        assert_eq!((thumb.width(), thumb.height()), (256, 192));
        let (w, h, hash): (i64, i64, Option<Vec<u8>>) = lib
            .db
            .with_conn(|c| Ok(c.query_row("SELECT width, height, thumbhash FROM assets WHERE id = ?1", [id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?))
            .unwrap();
        assert_eq!((w, h), (320, 240));
        assert!(hash.is_some());
        assert!(lib.db.with_conn(|c| assets::list_underived(c, 10)).unwrap().is_empty());
        // Smaller sizes are cut from the poster-derived preview through the normal path.
        let clip_spec = RenderSpec { hash: "feedface", source: &clip, kind: MediaKind::Video, orientation: 1, hdr: HdrKind::None, edit: None, ai_rasters: None };
        let p = render(&store, 1024, &clip_spec, Some(&ff)).unwrap();
        assert!(p.is_file());
    }

    #[test]
    fn export_renders_the_edit_at_full_size_and_honours_the_long_edge() {
        use photos_core::edits::Recipe;
        use photos_core::export::ExportFormat;
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("a.jpg");
        image::RgbImage::from_pixel(1200, 800, image::Rgb([90, 120, 150])).save(&src).unwrap();
        let recipe = Recipe { rotate: 90, ..Recipe::identity() };
        let spec = RenderSpec { hash: "0badc0ffee", source: &src, kind: MediaKind::Image, orientation: 1, hdr: HdrKind::None, edit: Some((&recipe, 3)), ai_rasters: None };
        let out = dir.path().join("out.jpg");
        render_export(&spec, None, &export_opts(ExportFormat::Jpeg, None), None, None, &out).unwrap();
        assert_eq!(image::image_dimensions(&out).unwrap(), (800, 1200));
        render_export(&spec, None, &export_opts(ExportFormat::Jpeg, Some(600)), None, None, &out).unwrap();
        assert_eq!(image::image_dimensions(&out).unwrap(), (400, 600));
        let png = dir.path().join("out.png");
        render_export(&spec, None, &export_opts(ExportFormat::Png, Some(300)), None, None, &png).unwrap();
        assert_eq!(&std::fs::read(&png).unwrap()[1..4], b"PNG");
        let webp = dir.path().join("out.webp");
        render_export(&spec, None, &export_opts(ExportFormat::Webp, None), None, None, &webp).unwrap();
        assert_eq!(image::image_dimensions(&webp).unwrap(), (800, 1200));
        // Unedited and "Original": the file, byte for byte.
        let copy = dir.path().join("copy.jpg");
        render_export(&RenderSpec { edit: None, ..spec }, None, &export_opts(ExportFormat::Original, None), None, None, &copy).unwrap();
        assert_eq!(std::fs::read(&copy).unwrap(), std::fs::read(&src).unwrap());
    }

    #[test]
    fn an_ai_mask_reaches_the_export_and_the_edited_full() {
        use photos_core::ai_masks::{AiMaskKind, AiMaskRaster, AiRasters};
        use photos_core::edits::{Develop, LocalAdjust, Mask, MaskShape};
        let dir = tempfile::tempdir().unwrap();
        let store = DerivativeStore::new(dir.path());
        let src = dir.path().join("a.png");
        image::RgbImage::from_pixel(120, 80, image::Rgb([60, 60, 60])).save(&src).unwrap();
        let shape = MaskShape::Ai { add: true, target: AiMaskKind::Sky, instance: 0, raster: "aa".into() };
        let mask = Mask { id: 1, name: "Sky".into(), shapes: vec![shape], invert: false, opacity: 1.0, adjust: LocalAdjust { exposure: 2.0, ..Default::default() }, visible: true };
        let recipe = Recipe { develop: Some(Develop { masks: vec![mask], ..Default::default() }), ..Recipe::identity() };
        // The left half of the photo.
        let raster = AiMaskRaster { kind: AiMaskKind::Sky, instance: 0, width: 16, height: 10, weights: (0..160).map(|i| if i % 16 < 8 { 255 } else { 0 }).collect() };
        let rasters = AiRasters::from([("aa".to_string(), raster)]);
        let lit = |p: &Path| {
            let img = image::open(p).unwrap().to_rgb8();
            (img.get_pixel(20, 40)[1], img.get_pixel(100, 40)[1])
        };
        let spec = RenderSpec { hash: "0badc0ffee", source: &src, kind: MediaKind::Image, orientation: 1, hdr: HdrKind::None, edit: Some((&recipe, 1)), ai_rasters: Some(&rasters) };
        let out = dir.path().join("out.png");
        render_export(&spec, None, &export_opts(ExportFormat::Png, None), None, None, &out).unwrap();
        let (left, right) = lit(&out);
        assert!(left > right + 40, "the export carries the mask: {left} vs {right}");
        let full = render_full(&store, &spec, None).unwrap();
        let (left, right) = lit(&full);
        assert!(left > right + 40, "and so do the library's edited files: {left} vs {right}");
        // Pasted onto a photo without the raster: nothing, and no error.
        render_export(&RenderSpec { ai_rasters: None, ..spec }, None, &export_opts(ExportFormat::Png, None), None, None, &out).unwrap();
        let (left, right) = lit(&out);
        assert_eq!(left, right);
    }

    fn export_opts(format: photos_core::export::ExportFormat, max_long_edge: Option<u32>) -> photos_core::export::ExportOptions {
        photos_core::export::ExportOptions { format, quality: 90, lossless: false, max_long_edge, metadata: false, destination: Default::default(), video_cut: Default::default() }
    }

    /// Every file in `dir` but the ones named.
    fn others(dir: &Path, except: &[&str]) -> Vec<String> {
        std::fs::read_dir(dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).filter(|n| !except.contains(&n.as_str())).collect()
    }

    #[test]
    fn a_lossless_export_of_an_edit_is_the_edit_exactly() {
        // Not re-encoded from a cached JPEG: every pixel of a turned PNG matches.
        use photos_core::edits::Recipe;
        use photos_core::export::ExportFormat;
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("a.png");
        let img = image::RgbImage::from_fn(64, 32, |x, y| image::Rgb([(x * 4) as u8, (y * 8) as u8, ((x + y) % 2 * 255) as u8]));
        img.save(&src).unwrap();
        let recipe = Recipe { rotate: 90, ..Recipe::identity() };
        let spec = RenderSpec { hash: "c0ffee", source: &src, kind: MediaKind::Image, orientation: 1, hdr: HdrKind::None, edit: Some((&recipe, 1)), ai_rasters: None };
        let out = dir.path().join("out.png");
        render_export(&spec, None, &export_opts(ExportFormat::Png, None), None, None, &out).unwrap();
        let want = image::imageops::rotate90(&img);
        assert_eq!(image::open(&out).unwrap().to_rgb8(), want);
    }

    #[test]
    fn a_failed_export_leaves_nothing_behind_and_the_original_alone() {
        use photos_core::export::ExportFormat;
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("a.jpg");
        image::RgbImage::from_pixel(64, 48, image::Rgb([90, 120, 150])).save(&src).unwrap();
        let before = (std::fs::read(&src).unwrap(), std::fs::metadata(&src).unwrap().modified().unwrap());
        let spec = RenderSpec { hash: "abad1dea", source: &src, kind: MediaKind::Image, orientation: 1, hdr: HdrKind::None, edit: None, ai_rasters: None };
        // AVIF without FFmpeg cannot be encoded.
        let out = dir.path().join("a.avif");
        assert!(render_export(&spec, None, &export_opts(ExportFormat::Avif, None), None, None, &out).is_err());
        assert!(!out.exists() && !part_path(&out).exists());
        // A good export leaves only itself: the part file became it.
        let ok = dir.path().join("b.jpg");
        render_export(&spec, None, &export_opts(ExportFormat::Jpeg, None), None, None, &ok).unwrap();
        assert_eq!(others(dir.path(), &["a.jpg", "b.jpg"]), Vec::<String>::new());
        assert_eq!((std::fs::read(&src).unwrap(), std::fs::metadata(&src).unwrap().modified().unwrap()), before);
    }

    #[test]
    fn a_trimmed_video_without_ffmpeg_fails_cleanly() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("clip.mov");
        std::fs::write(&src, b"not really a movie").unwrap();
        let out = dir.path().join("clip-trimmed.mov");
        let e = render_trimmed_export(None, &src, &[Segment { start_ms: 0, end_ms: 1000 }], VideoCut::Precise, None, &out, &|| false).unwrap_err();
        assert_eq!(e.code, "ffmpeg_missing");
        assert_eq!(others(dir.path(), &["clip.mov"]), Vec::<String>::new());
    }

    #[test]
    #[ignore = "needs FFmpeg and ExifTool"]
    fn a_trimmed_video_exports_its_kept_segments_with_its_tags() {
        use photos_core::export::ExportMeta;
        let ff = Ffmpeg::locate().expect("FFmpeg");
        let carrier = Carrier::new(ExifTool::locate().expect("ExifTool"));
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("clip.mov");
        let made = std::process::Command::new(&ff.ffmpeg)
            .args(["-v", "error", "-y", "-f", "lavfi", "-i", "testsrc=size=320x240:rate=30:duration=5", "-c:v", "libx264", "-pix_fmt", "yuv420p"])
            .args(["-metadata", "creation_time=2022-07-09T08:15:00Z", "-metadata", "location=+45.5283+013.5683/", "-movflags", "use_metadata_tags"])
            .arg(&src)
            .status()
            .unwrap();
        assert!(made.success());
        let out = dir.path().join("clip-trimmed.mov");
        let segments = [Segment { start_ms: 500, end_ms: 1500 }, Segment { start_ms: 3000, end_ms: 4000 }];
        let lost = render_trimmed_export(Some(&ff), &src, &segments, VideoCut::Precise, Some((&carrier, &ExportMeta::default())), &out, &|| false).unwrap();
        assert_eq!(lost, Some(vec![]));
        let ms = ff.probe(&out).unwrap().duration_ms.unwrap();
        assert!((ms - 2000).abs() <= 34, "{ms} ms");
        assert_eq!(others(dir.path(), &["clip.mov", "clip-trimmed.mov"]), Vec::<String>::new());
    }

    #[test]
    fn a_part_file_keeps_the_extension_and_hides() {
        assert_eq!(part_path(Path::new("D:/out/IMG_1 (2).heic")), Path::new("D:/out/.lumen-part-IMG_1 (2).heic"));
    }

    #[test]
    #[ignore = "needs ExifTool (LUMEN_EXIFTOOL_DIR or src-tauri/binaries)"]
    fn an_export_carries_the_originals_metadata_and_lumens_marks() {
        use photos_core::export::{ExportFormat, ExportMeta};
        let tool = ExifTool::locate().expect("ExifTool: run scripts/fetch-exiftool");
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("a.jpg");
        image::RgbImage::from_pixel(300, 200, image::Rgb([90, 120, 150])).save(&src).unwrap();
        let mut s = tool.session().unwrap();
        let out = s
            .run(["-overwrite_original", "-IFD0:Make=FUJIFILM", "-IFD0:Model=X-T5", "-IFD0:Orientation#=6", "-ExifIFD:DateTimeOriginal=2023:08:14 18:30:00", "-GPSLatitude=45.52", "-GPSLatitudeRef=N", "-IPTC:Keywords=dusk", src.to_str().unwrap()])
            .unwrap();
        assert!(out.errors().is_empty(), "{out:?}");
        drop(s);
        let before = std::fs::read(&src).unwrap();
        let spec = RenderSpec { hash: "f00d", source: &src, kind: MediaKind::Image, orientation: 6, hdr: HdrKind::None, edit: None, ai_rasters: None };
        let carrier = Carrier::new(tool);
        let meta = ExportMeta { rating: 5, color_label: 1, keywords: vec!["Piran harbour".into()], ..Default::default() };
        for format in [ExportFormat::Jpeg, ExportFormat::Png, ExportFormat::Webp] {
            let out = dir.path().join(photos_core::export::export_name("a.jpg", format, false).replace("a.", "out."));
            let lost = render_export(&spec, None, &ExportOptions { metadata: true, ..export_opts(format, None) }, Some((&carrier, &meta)), None, &out).unwrap();
            assert_eq!(lost, Some(vec![]), "{format:?}");
            // Turned upright by orientation 6, and says so.
            assert_eq!(image::image_dimensions(&out).unwrap(), (200, 300));
        }
        assert_eq!(std::fs::read(&src).unwrap(), before);
    }

    /// A thumbnail drawn sideways under an orientation that was later corrected is found,
    /// and one that agrees with the catalogue (or is square) is not.
    #[test]
    fn misshapen_thumbnails_are_found_by_their_shape() {
        let dir = tempfile::tempdir().unwrap();
        let store = DerivativeStore::new(dir.path());
        let thumb = |hash: &str, w: u32, h: u32| {
            let p = store.thumb_path(hash, 256);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            image::RgbImage::new(w, h).save(&p).unwrap();
        };
        thumb("aa0001", 256, 192); // sideways: the catalogue says portrait
        thumb("aa0002", 192, 256); // agrees
        thumb("aa0003", 256, 192); // a landscape stored with orientation 6 is portrait on screen
        thumb("aa0004", 192, 256); // agrees with orientation 6
        thumb("aa0005", 256, 250); // square-ish: never judged
        let rows = vec![
            (1, "aa0001".to_string(), 3024, 4032, 1),
            (2, "aa0002".to_string(), 3024, 4032, 1),
            (3, "aa0003".to_string(), 4032, 3024, 6),
            (4, "aa0004".to_string(), 4032, 3024, 6),
            (5, "aa0005".to_string(), 3000, 3024, 1),
            (6, "aa0006".to_string(), 3024, 4032, 1), // no thumbnail yet
        ];
        assert_eq!(misshapen(&store, &rows), vec![1, 3]);
    }

    /// Round 4A: 57 HEICs on Julian's library had a right thumbnail but a sideways 1024 and
    /// 2560 preview (rendered before the orientation fix), shown sideways in the viewer and in
    /// memory films. Every cached size is compared, not just the thumbnail.
    #[test]
    fn a_stale_preview_is_found_even_when_the_thumbnail_is_right() {
        let dir = tempfile::tempdir().unwrap();
        let store = DerivativeStore::new(dir.path());
        let jpeg = |p: std::path::PathBuf, w: u32, h: u32| {
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            image::RgbImage::new(w, h).save(&p).unwrap();
        };
        jpeg(store.thumb_path("bb0001", 256), 192, 256);
        jpeg(store.preview_path("bb0001"), 2560, 1920); // sideways
        jpeg(store.thumb_path("bb0002", 256), 192, 256);
        jpeg(store.thumb_path("bb0002", 1024), 1024, 768); // sideways
        jpeg(store.thumb_path("bb0003", 256), 192, 256);
        jpeg(store.full_path("bb0003"), 400, 300); // sideways
        jpeg(store.thumb_path("bb0004", 256), 192, 256);
        jpeg(store.thumb_path("bb0004", 1024), 768, 1024);
        jpeg(store.preview_path("bb0004"), 1920, 2560);
        let rows = (1..=4).map(|i| (i, format!("bb000{i}"), 3024, 4032, 1)).collect::<Vec<_>>();
        assert_eq!(misshapen(&store, &rows), vec![1, 2, 3]);
    }

    /// Replay on real data, read-only: `LUMEN_DB=<copy of library.db>` and
    /// `LUMEN_LIBRARY=<library folder>` (only its thumbnails' headers are read). Prints how
    /// many photos `refix_shapes` would send back through ingest; after a run of the app, 0.
    #[test]
    #[ignore]
    fn misshapen_on_a_real_library() {
        let db = photos_core::Db::open(std::path::Path::new(&std::env::var("LUMEN_DB").unwrap())).unwrap();
        let store = DerivativeStore::new(std::path::Path::new(&std::env::var("LUMEN_LIBRARY").unwrap()));
        let rows = db.with_conn(assets::derived_shapes).unwrap();
        let ids = misshapen(&store, &rows);
        println!("{} of {} photos have a thumbnail of the wrong shape: {:?}", ids.len(), rows.len(), &ids[..ids.len().min(20)]);
    }

    /// A phone JPEG stores its pixels sideways with an orientation tag. The full-size image a
    /// slideshow paints must be the photo as shown (Round 4A: memories' films had them sideways,
    /// because `render_full` serves a plain JPEG's original file, tag unapplied).
    #[test]
    fn an_edited_render_blends_in_the_denoised_base_when_it_exists() {
        use photos_core::edits::{Denoise, Detail, Develop, Recipe};
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("noisy.jpg");
        image::RgbImage::from_pixel(64, 48, image::Rgb([200, 60, 60])).save(&src).unwrap();
        let store = DerivativeStore::new(&dir.path().join("lib"));
        let recipe = Recipe {
            develop: Some(Develop { detail: Detail { denoise: Some(Denoise { strength: 1.0, model: "m@1".into() }), ..Default::default() }, ..Default::default() }),
            ..Recipe::identity()
        };
        let spec = RenderSpec { hash: "0123abcd", source: &src, kind: MediaKind::Image, orientation: 1, hdr: HdrKind::None, edit: Some((&recipe, 1)), ai_rasters: None };
        // No denoised base yet: the photo as it is.
        let plain = decode_display_full(&store, &spec, None).unwrap().to_rgb8();
        assert!(plain.get_pixel(5, 5)[0] > 150);
        // Built: a full strength denoise shows the base.
        let grey = develop::Linear::new(64, 48, [0.2, 0.2, 0.2]);
        crate::denoise::write_base(&store.denoise_path("0123abcd", "m@1"), &grey).unwrap();
        let spec = RenderSpec { edit: Some((&recipe, 2)), ..spec };
        let px = *decode_display_full(&store, &spec, None).unwrap().to_rgb8().get_pixel(5, 5);
        assert!((px[0] as i32 - px[2] as i32).abs() <= 2, "{px:?}");
        // And an export with it.
        let out = dir.path().join("out.png");
        render_export(&spec, None, &export_opts(ExportFormat::Png, None), None, Some(&grey), &out).unwrap();
        let e = *image::open(&out).unwrap().to_rgb8().get_pixel(5, 5);
        assert!((e[0] as i32 - e[2] as i32).abs() <= 2, "{e:?}");
    }

    #[test]
    fn the_face_scan_reads_the_cached_preview_or_else_the_original_writing_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("phone.jpg");
        image::RgbImage::from_pixel(400, 300, image::Rgb([90, 120, 150])).save(&src).unwrap();
        let store = DerivativeStore::new(&dir.path().join("lib"));
        let spec = RenderSpec { hash: "0123abcd", source: &src, kind: MediaKind::Image, orientation: 6, hdr: HdrKind::None, edit: None, ai_rasters: None };
        let img = decode_for_scan(&store, &spec, None).unwrap();
        assert_eq!((img.width(), img.height()), (300, 400), "the original, upright");
        assert!(!dir.path().join("lib").exists(), "nothing cached for it");
        let preview = store.preview_path("0123abcd");
        std::fs::create_dir_all(preview.parent().unwrap()).unwrap();
        image::RgbImage::new(1920, 2560).save(&preview).unwrap();
        assert_eq!(decode_for_scan(&store, &spec, None).unwrap().height(), 2560, "the cached preview when there is one");
    }

    #[test]
    fn the_display_full_size_is_upright_and_edited() {
        use photos_core::edits::Recipe;
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("phone.jpg");
        image::RgbImage::from_pixel(400, 300, image::Rgb([90, 120, 150])).save(&src).unwrap();
        let store = DerivativeStore::new(dir.path());
        let mut spec = RenderSpec { hash: "0123abcd", source: &src, kind: MediaKind::Image, orientation: 6, hdr: HdrKind::None, edit: None, ai_rasters: None };
        let upright = decode_display_full(&store, &spec, None).unwrap();
        assert_eq!((upright.width(), upright.height()), (300, 400), "orientation 6 is a portrait on screen");
        let recipe = Recipe { rotate: 90, ..Recipe::identity() };
        spec.edit = Some((&recipe, 3));
        let turned = decode_display_full(&store, &spec, None).unwrap();
        assert_eq!((turned.width(), turned.height()), (400, 300), "the edit's quarter turn on top");
    }

    #[test]
    fn an_edited_render_has_the_edited_frame_and_leaves_the_original_alone() {
        use photos_core::edits::Recipe;
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("a.jpg");
        image::RgbImage::from_pixel(400, 300, image::Rgb([90, 120, 150])).save(&src).unwrap();
        let store = DerivativeStore::new(dir.path());
        let recipe = Recipe { rotate: 90, ..Recipe::identity() };
        let mut spec = RenderSpec { hash: "feedfacecafe", source: &src, kind: MediaKind::Image, orientation: 1, hdr: HdrKind::None, edit: None, ai_rasters: None };
        let plain = render(&store, 256, &spec, None).unwrap();
        spec.edit = Some((&recipe, 7));
        let edited = render(&store, 256, &spec, None).unwrap();
        assert_ne!(plain, edited);
        assert_eq!(image::image_dimensions(&plain).unwrap(), (256, 192));
        assert_eq!(image::image_dimensions(&edited).unwrap(), (192, 256));
        let full = render_full(&store, &spec, None).unwrap();
        assert_ne!(full, src, "an edited photo's full size is rendered, not the original");
        assert_eq!(image::image_dimensions(&full).unwrap(), (300, 400));
    }
}
