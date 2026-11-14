//! Face backfill (Phase 7b): every visible photo is looked at once for faces, on the
//! Maintenance lane. Round 4B scans at 1536 px, cut in memory from the cached 2560 preview
//! or else the original (`derivatives::decode_for_scan`, which caches nothing new): on
//! 200 of Julian's photos it found 93 % of the faces a 2048 px scan finds (1024 px: 70 %)
//! at 53 ms a photo against 101 ms. The faces found are embedded and stored, then clustered into
//! people. The face crops the People screen shows are cut once and cached beside the
//! derivatives.

use std::path::PathBuf;

use photos_core::face_filter;
use photos_core::people::{self, NewFace};
use photos_ml::clip::ImageEncoder;
use photos_ml::faces::{align_crop, FaceDetector, FaceEmbedder};
use tauri::{AppHandle, Emitter, Manager};

use crate::events::{PeopleProgress, PEOPLE_PROGRESS};

use crate::error::{AppError, AppResult};
use crate::state::AppState;

const BATCH: usize = 16;
/// The long edge faces are found at.
const SCAN_SIZE: u32 = 1536;
/// The size `face_filter`'s pixel thresholds were measured at, and the face crops' source.
const FILTER_SCALE: u32 = 1024;

/// What `face_scan.model` records: the detector, and Round 4B's rescan at 1536 px, so every
/// photo scanned before it is scanned once more.
pub fn scan_key(detector: &str) -> String {
    format!("{detector}+r4")
}

/// The image the detector sees: the rendering brought down to `SCAN_SIZE`, never enlarged.
pub fn scan_image(img: &image::DynamicImage) -> image::DynamicImage {
    fit_within(img, SCAN_SIZE)
}
/// The People screen's face crops.
const CROP_SIZE: u32 = 160;

/// Round 4I's second look at a photo of people with no face found: the long edge the whole
/// photo is looked at, and the tiles (`photos_ml::faces::tiles`) looked at on top of it, each
/// at its own resolution so small faces in a group photo have pixels enough.
const SECOND_SIZE: u32 = 2560;
const SECOND_TILE: u32 = 1536;
/// People are regrouped during a long scan at most this often (each regroup is a few hundred
/// milliseconds on a real library), and once more at the end.
const REGROUP_EVERY: std::time::Duration = std::time::Duration::from_secs(20);

/// Which look a photo is getting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Look {
    /// Every photo, once, at `SCAN_SIZE`.
    First,
    /// A photo of people the first look found no face in (`people::second_chance`).
    Second,
}

/// What a look found.
pub enum Found {
    /// Faces, judged (possibly none).
    Faces(Vec<NewFace>),
    /// The photo cannot be read (FFmpeg missing for a HEIC with no preview, a broken file): it
    /// is marked scanned and keeps what an earlier scan found (Round 4B review).
    Unreadable,
    /// The models went away (or the library changed): stop, and try again later.
    Stop,
}

/// Finds and judges the faces in one photo. The app's is `AppScanner`; tests use a fake.
pub trait Scanner {
    fn look(&mut self, asset: i64, how: Look) -> Found;
}

/// What the scan loop tells the app about.
pub struct Hooks<'a> {
    /// Regroup people now (`people::recluster`).
    pub regroup: &'a mut dyn FnMut(),
    /// Photos the first pass has left, after each batch.
    pub progress: &'a mut dyn FnMut(usize),
    /// Regroup at most this often while scanning.
    pub every: std::time::Duration,
    pub cancelled: &'a dyn Fn() -> bool,
}

/// Regroups when faces were stored since the last regroup and `every` has passed (or `force`).
struct Regrouper {
    pending: bool,
    last: std::time::Instant,
}

impl Regrouper {
    fn maybe(&mut self, hooks: &mut Hooks<'_>, force: bool) {
        if self.pending && (force || self.last.elapsed() >= hooks.every) {
            (hooks.regroup)();
            self.pending = false;
            self.last = std::time::Instant::now();
        }
    }
}

/// Every unscanned photo gets its first look, newest first, in batches of `BATCH`; then every
/// photo of people with no face found gets its second (Round 4I). People are regrouped after a
/// batch that stored faces (at most every `hooks.every`) and at the end if anything was stored
/// since. Returns how many photos were scanned.
pub fn run(db: &photos_core::db::Db, key: &str, scanner: &mut dyn Scanner, hooks: &mut Hooks<'_>) -> usize {
    let second = people::second_chance_key(key);
    let mut scanned = 0usize;
    let mut regrouper = Regrouper { pending: false, last: std::time::Instant::now() };
    for how in [Look::First, Look::Second] {
        let this_key = if how == Look::First { key } else { second.as_str() };
        loop {
            if (hooks.cancelled)() {
                regrouper.maybe(hooks, true);
                return scanned;
            }
            let batch: Vec<i64> = match how {
                Look::First => db.with_conn(|c| people::unscanned(c, key, BATCH)).map(|r| r.into_iter().map(|(id, _)| id).collect()),
                Look::Second => db.with_conn(|c| people::second_chance(c, key, BATCH)),
            }
            .unwrap_or_default();
            if batch.is_empty() {
                break;
            }
            let (mut stored, mut done) = (false, 0usize);
            for id in batch {
                let saved = match scanner.look(id, how) {
                    Found::Stop => {
                        regrouper.maybe(hooks, true);
                        return scanned;
                    }
                    Found::Unreadable => db.with_conn(|c| people::mark_scanned(c, id, this_key)),
                    // A second look that finds nothing leaves the photo as it was.
                    Found::Faces(faces) if how == Look::Second && faces.is_empty() => db.with_conn(|c| people::mark_scanned(c, id, this_key)),
                    Found::Faces(faces) => {
                        stored |= !faces.is_empty();
                        match how {
                            Look::First => db.with_conn(|c| people::replace_faces_keeping_identity(c, id, this_key, &faces)),
                            Look::Second => db.with_conn(|c| people::store_second_look(c, id, this_key, &faces)),
                        }
                    }
                };
                if saved.is_ok() {
                    done += 1;
                }
            }
            scanned += done;
            if how == Look::First {
                (hooks.progress)(db.with_conn(|c| people::unscanned_count(c, key)).unwrap_or(0));
            }
            regrouper.pending |= stored;
            regrouper.maybe(hooks, false);
            // Nothing could be saved: the database is failing; try again later.
            if done == 0 {
                break;
            }
        }
    }
    regrouper.maybe(hooks, true);
    scanned
}

/// The app's scanner: the photo from the cached preview or the original
/// (`derivatives::scan_image_of`), YuNet and SFace (`models::with_faces`), then the crop filter.
struct AppScanner<'a> {
    app: &'a AppHandle,
    /// The library the run started in: if the user switches, the run stops.
    root: PathBuf,
}

impl Scanner for AppScanner<'_> {
    fn look(&mut self, asset: i64, how: Look) -> Found {
        match self.app.state::<AppState>().library() {
            Ok(lib) if lib.root == self.root => {}
            _ => return Found::Stop,
        }
        let Ok(img) = crate::derivatives::scan_image_of(self.app, asset) else { return Found::Unreadable };
        let size = if how == Look::First { SCAN_SIZE } else { SECOND_SIZE };
        let (rgb, w, h) = crate::embeddings::rgb_of(&fit_within(&img, size));
        let found = crate::models::with_faces(self.app, |det, emb| {
            let boxes = match how {
                Look::First => det.detect(&rgb, w, h),
                Look::Second => det.detect_tiled(&rgb, w, h, SECOND_TILE),
            };
            boxes.map(|b| embed_found(emb, &rgb, w, h, &b)).unwrap_or_default()
        });
        let Some(mut faces) = found else { return Found::Stop };
        // Nothing is stored unfiltered: without the image model the photo waits for it.
        if !judge(self.app, &rgb, w, h, &mut faces) {
            return Found::Stop;
        }
        Found::Faces(faces)
    }
}

/// Regroup people and tell the frontend.
fn regroup_now(app: &AppHandle) {
    let state = app.state::<AppState>();
    let Ok(lib) = state.library() else { return };
    // Read, think with no connection held, then one short write.
    let regrouped = lib
        .db
        .with_conn(people::GroupingRows::read_faces)
        .map(people::ReclusterInput::from_rows)
        .map(|input| input.plan(&people::Thresholds::MEASURED))
        .and_then(|plan| lib.db.with_conn(|c| plan.apply_counting(c, photos_core::now_unix())));
    match regrouped {
        // Nothing moved: no refetch of every screen (Round 5B; each People edit regroups).
        Ok((0, 0)) => {}
        Ok((n, removed)) => {
            tracing::info!(assigned = n, removed, "people regrouped");
            state.changes.changed(app, true);
        }
        Err(e) => {
            tracing::warn!("people: {e}");
            state.changes.changed(app, true);
        }
    }
}

/// Find the faces in everything not yet scanned (and look again at photos of people where
/// none was found), regrouping people as it goes. Coalesced; after every import (through
/// `embeddings::schedule`), on the tick and when models arrive.
pub fn schedule(app: &AppHandle) {
    if !crate::models::enabled(app) {
        return;
    }
    let a = app.clone();
    app.state::<AppState>().jobs.submit("maintenance:faces", photos_jobs::Lane::Maintenance, move |cancel| {
        let state = a.state::<AppState>();
        let Ok(lib) = state.library() else { return };
        let Some(det) = photos_ml::manifest::manifest().by_role("face_detection") else { return };
        let key = scan_key(&det.id);
        // Faces pinned to nobody before Round 4I come back into play, refused by the person
        // they were most likely taken from (idempotent; nothing after 4I makes new ones).
        let repaired = match lib.db.with_conn(|c| people::repair_orphans(c, &people::Thresholds::MEASURED, photos_core::now_unix())) {
            Ok(n) => n,
            Err(e) => {
                tracing::warn!("people: repairing orphaned faces: {e}");
                0
            }
        };
        if repaired > 0 {
            tracing::info!(faces = repaired, "people: faces pinned to nobody are back in play");
        }
        let started = std::time::Instant::now();
        let mut regrouped = false;
        let mut regroup = || {
            regroup_now(&a);
            regrouped = true;
        };
        let mut progress = |left: usize| {
            let _ = a.emit(PEOPLE_PROGRESS, PeopleProgress { left: left as u32 });
        };
        let mut scanner = AppScanner { app: &a, root: lib.root.clone() };
        let scanned = run(&lib.db, &key, &mut scanner, &mut Hooks { regroup: &mut regroup, progress: &mut progress, every: REGROUP_EVERY, cancelled: &|| cancel.is_cancelled() });
        let _ = a.emit(PEOPLE_PROGRESS, PeopleProgress { left: 0 });
        if repaired > 0 && !regrouped {
            regroup_now(&a);
        }
        if scanned > 0 {
            tracing::info!(photos = scanned, per_second = scanned as f32 / started.elapsed().as_secs_f32().max(0.001), "faces scanned");
        }
    });
}

/// Round 4I: regroup people after the user changed them (a move, merge, name, answer or
/// rejection), so faces the changed people now clearly match are placed without waiting for
/// the next scan. Coalesced on the Maintenance lane: a burst of edits regroups once.
pub fn schedule_recluster(app: &AppHandle) {
    if !crate::models::enabled(app) {
        return;
    }
    let a = app.clone();
    app.state::<AppState>().jobs.submit("maintenance:people", photos_jobs::Lane::Maintenance, move |_| regroup_now(&a));
}

/// `img` brought down so its long edge is at most `size`, never enlarged.
fn fit_within(img: &image::DynamicImage, size: u32) -> image::DynamicImage {
    if img.width().max(img.height()) <= size {
        return img.clone();
    }
    img.resize(size, size, image::imageops::FilterType::Triangle)
}

/// The pixels of a square `(x, y, side)` of a packed RGB image `width` wide.
pub fn crop_pixels(rgb: &[u8], width: u32, (x, y, side): (u32, u32, u32)) -> Vec<u8> {
    let (w, s) = (width as usize, side as usize);
    let mut out = Vec::with_capacity(s * s * 3);
    for row in y as usize..y as usize + s {
        let start = (row * w + x as usize) * 3;
        out.extend_from_slice(&rgb[start..start + s * 3]);
    }
    out
}

/// The faces in a packed RGB image, embedded and placed in 0..1 of it, not yet judged.
pub fn detect_faces(det: &mut FaceDetector, emb: &mut FaceEmbedder, rgb: &[u8], w: u32, h: u32) -> Vec<NewFace> {
    let Ok(faces) = det.detect(rgb, w, h) else { return Vec::new() };
    embed_found(emb, rgb, w, h, &faces)
}

/// Detected faces embedded and placed in 0..1 of the image, not yet judged.
pub fn embed_found(emb: &mut FaceEmbedder, rgb: &[u8], w: u32, h: u32, faces: &[photos_ml::faces::Face]) -> Vec<NewFace> {
    faces
        .iter()
        .filter_map(|f| {
            let vec = emb.embed(&align_crop(rgb, w, h, &f.landmarks)).ok()?;
            Some(NewFace {
                x: (f.bbox[0] / w as f32).clamp(0.0, 1.0),
                y: (f.bbox[1] / h as f32).clamp(0.0, 1.0),
                w: (f.bbox[2] / w as f32).clamp(0.0, 1.0),
                h: (f.bbox[3] / h as f32).clamp(0.0, 1.0),
                score: f.score,
                vec,
                quality: 0.0,
                rejected: None,
            })
        })
        .collect()
}

/// A drawn box is looked in grown by this share of its size, so the detector sees the whole head.
const BOX_GROW: f32 = 0.3;
/// The grown box is brought up to at least this long edge before detecting: YuNet ignores
/// faces under its minimum side, and a hand-drawn box is often around a small face.
const BOX_DETECT_MIN: u32 = 320;

/// Round 4I, Task 6: a box the user drew (0..1 of the packed RGB image) as a face. When the
/// detector finds a face centred inside the box (looking in the box grown by `BOX_GROW`), the
/// face is aligned on its landmarks and keeps the detector's box, score and quality. Otherwise
/// the box's centre is embedded as it is, with quality 0, so it never stands for anyone.
pub fn embed_box_with(det: &mut FaceDetector, emb: &mut FaceEmbedder, rgb: &[u8], w: u32, h: u32, rect: [f32; 4]) -> Option<NewFace> {
    let (wf, hf) = (w as f32, h as f32);
    let [bx, by, bw, bh] = [rect[0] * wf, rect[1] * hf, rect[2] * wf, rect[3] * hf];
    if bw < 1.0 || bh < 1.0 {
        return None;
    }
    let (gx0, gy0) = ((bx - bw * BOX_GROW).max(0.0).floor() as u32, (by - bh * BOX_GROW).max(0.0).floor() as u32);
    let (gx1, gy1) = ((bx + bw * (1.0 + BOX_GROW)).min(wf).ceil() as u32, (by + bh * (1.0 + BOX_GROW)).min(hf).ceil() as u32);
    let (gw, gh) = (gx1.saturating_sub(gx0).max(1), gy1.saturating_sub(gy0).max(1));
    let full = image::RgbImage::from_raw(w, h, rgb.to_vec())?;
    let grown = image::imageops::crop_imm(&full, gx0, gy0, gw, gh).to_image();
    let up = (BOX_DETECT_MIN as f32 / gw.max(gh) as f32).max(1.0);
    let seen = if up > 1.0 {
        image::imageops::resize(&grown, (gw as f32 * up).round() as u32, (gh as f32 * up).round() as u32, image::imageops::FilterType::Triangle)
    } else {
        grown
    };
    let inside = |f: &photos_ml::faces::Face| {
        let (cx, cy) = (gx0 as f32 + (f.bbox[0] + f.bbox[2] / 2.0) / up, gy0 as f32 + (f.bbox[1] + f.bbox[3] / 2.0) / up);
        cx >= bx && cx <= bx + bw && cy >= by && cy <= by + bh
    };
    let found = det.detect(seen.as_raw(), seen.width(), seen.height()).unwrap_or_default().into_iter().filter(inside).max_by(|a, b| a.score.total_cmp(&b.score));
    if let Some(mut f) = found {
        // Back into the whole image's pixels.
        f.bbox = [gx0 as f32 + f.bbox[0] / up, gy0 as f32 + f.bbox[1] / up, f.bbox[2] / up, f.bbox[3] / up];
        for l in f.landmarks.iter_mut() {
            *l = [gx0 as f32 + l[0] / up, gy0 as f32 + l[1] / up];
        }
        let mut face = embed_found(emb, rgb, w, h, std::slice::from_ref(&f)).pop()?;
        let side_px = (face.w * wf).max(face.h * hf) * FILTER_SCALE as f32 / wf.max(hf);
        face.quality = face_filter::quality(face.score, side_px);
        return Some(face);
    }
    // No face the detector can see (the back of a head, a tiny or covered face): the box's
    // centre square, as it is.
    let side = bw.max(bh).min(wf).min(hf);
    let x = (bx + bw / 2.0 - side / 2.0).clamp(0.0, wf - side);
    let y = (by + bh / 2.0 - side / 2.0).clamp(0.0, hf - side);
    let square = image::imageops::crop_imm(&full, x as u32, y as u32, (side as u32).max(1), (side as u32).max(1)).to_image();
    let small = image::imageops::resize(&square, 112, 112, image::imageops::FilterType::Triangle);
    let vec = emb.embed(small.as_raw()).ok()?;
    Some(NewFace { x: rect[0], y: rect[1], w: rect[2], h: rect[3], score: 0.0, vec, quality: 0.0, rejected: None })
}

/// `embed_box_with` on one photo of the library: the photo as the scan sees it, nothing cached.
pub fn embed_box(app: &AppHandle, asset: i64, rect: [f32; 4]) -> AppResult<people::ManualFace> {
    let unreadable = || AppError { code: "render_failed".into(), message: "The photo could not be read.".into(), detail: None };
    let img = crate::derivatives::scan_image_of(app, asset)?;
    let (rgb, w, h) = crate::embeddings::rgb_of(&scan_image(&img));
    let face = crate::models::with_faces(app, |det, emb| embed_box_with(det, emb, &rgb, w, h, rect))
        .ok_or_else(|| AppError { code: "models_missing".into(), message: "Faces can be tagged once smart features are on and downloaded.".into(), detail: None })?
        .ok_or_else(unreadable)?;
    Ok(people::ManualFace { asset_id: asset, x: face.x, y: face.y, w: face.w, h: face.h, score: face.score, quality: face.quality, vec: face.vec })
}

/// Round 4B: each face's quality, and whether its crop is a screen, a drawing, a print or a
/// statue rather than a person (`face_filter`). The crops go through CLIP in one batch;
/// without the image model every face passes and only its quality is set.
fn judge(app: &AppHandle, rgb: &[u8], w: u32, h: u32, faces: &mut [NewFace]) -> bool {
    crate::models::with_encoder(app, |enc| judge_with(Some(enc), rgb, w, h, faces)).unwrap_or_else(|| judge_with(None, rgb, w, h, faces))
}

/// `judge` over an encoder already open (or none). Returns whether the crops were checked
/// (a photo with no faces needs no check); without them every face gets only its quality.
pub fn judge_with(encoder: Option<&mut ImageEncoder>, rgb: &[u8], w: u32, h: u32, faces: &mut [NewFace]) -> bool {
    if faces.is_empty() {
        return true;
    }
    let rects: Vec<(u32, u32, u32)> = faces.iter().map(|f| crop_rect([f.x, f.y, f.w, f.h], w, h)).collect();
    let crops: Vec<Vec<u8>> = rects.iter().map(|r| crop_pixels(rgb, w, *r)).collect();
    let kinds = encoder.and_then(|enc| {
        let batch: Vec<photos_ml::clip::Rgb<'_>> = crops.iter().zip(&rects).map(|(c, r)| photos_ml::clip::Rgb { data: c, width: r.2, height: r.2 }).collect();
        enc.embed_batch(&batch).ok()
    });
    let protos = photos_ml::prototypes();
    for (i, f) in faces.iter_mut().enumerate() {
        let crop = kinds
            .as_ref()
            .and_then(|k| k.get(i))
            .map(|(v, _)| photos_core::embedding::softmax(v, &protos, "face:", face_filter::CROP_TEMPERATURE))
            .unwrap_or_default();
        let side_px = (f.w * w as f32).max(f.h * h as f32) * FILTER_SCALE as f32 / w.max(h) as f32;
        match face_filter::judge(&face_filter::FaceSignals { score: f.score, side_px, crop }) {
            Ok(q) => f.quality = q,
            Err(reason) => {
                f.quality = face_filter::quality(f.score, side_px);
                f.rejected = Some(reason.to_string());
            }
        }
    }
    kinds.is_some()
}

/// A square crop around the face, a little wider than the detector's box so the whole head
/// shows: centre and side in 0..1 of the photo's width and height.
pub fn crop_rect(face: [f32; 4], photo_w: u32, photo_h: u32) -> (u32, u32, u32) {
    let (w, h) = (photo_w as f32, photo_h as f32);
    let (cx, cy) = ((face[0] + face[2] / 2.0) * w, (face[1] + face[3] / 2.0) * h);
    let side = (face[2] * w).max(face[3] * h) * 1.6;
    let side = side.min(w).min(h).max(1.0);
    let x = (cx - side / 2.0).clamp(0.0, w - side);
    let y = (cy - side / 2.0).clamp(0.0, h - side);
    (x.round() as u32, y.round() as u32, side.round() as u32)
}

/// The flips and quarter turns the user gave a photo. Faces sit on the unedited photo, so a
/// crop is cut there and then turned like the photo the grid shows: a photo turned by hand
/// showed its people sideways or upside down.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Turn {
    rotate: u16,
    flip_h: bool,
    flip_v: bool,
}

impl Turn {
    pub fn of(recipe: Option<&photos_core::edits::Recipe>) -> Self {
        recipe.map_or_else(Self::default, |r| Self { rotate: r.rotate % 360, flip_h: r.flip_h, flip_v: r.flip_v })
    }

    /// The cached crop's name: turning the photo again makes a new one.
    pub fn file_name(&self, face: i64) -> String {
        if *self == Self::default() {
            return format!("{face}.jpg");
        }
        format!("{face}_r{}{}{}.jpg", self.rotate, if self.flip_h { "h" } else { "" }, if self.flip_v { "v" } else { "" })
    }

    /// Flips first, then the turn: the order `photos_media::develop::geometry` renders in.
    pub fn apply(&self, mut img: image::DynamicImage) -> image::DynamicImage {
        if self.flip_h {
            img = img.fliph();
        }
        if self.flip_v {
            img = img.flipv();
        }
        match self.rotate {
            90 => img.rotate90(),
            180 => img.rotate180(),
            270 => img.rotate270(),
            _ => img,
        }
    }
}

/// The face crop for `photos://face/{id}`, cut from the 1024 px rendering on first request.
pub fn ensure_crop(app: &AppHandle, face: i64) -> AppResult<PathBuf> {
    let state = app.state::<AppState>();
    let lib = state.library()?;
    let dir = lib.root.join("derivatives").join("faces");
    let not_found = || AppError { code: "not_found".into(), message: "No such face.".into(), detail: None };
    let (asset, rect) = lib.db.with_conn(|c| people::face_rect(c, face))?.ok_or_else(not_found)?;
    let turn = Turn::of(lib.db.with_conn(|c| photos_core::edits::get_recipe(c, asset))?.map(|(r, _)| r).as_ref());
    let out = dir.join(turn.file_name(face));
    if out.is_file() {
        return Ok(out);
    }
    let img = image::open(crate::derivatives::render_now(app, asset, FILTER_SCALE)?).map_err(|e| AppError {
        code: "render_failed".into(),
        message: "The photo could not be read.".into(),
        detail: Some(e.to_string()),
    })?;
    let (x, y, side) = crop_rect(rect, img.width(), img.height());
    let crop = turn.apply(img.crop_imm(x, y, side, side).resize_exact(CROP_SIZE, CROP_SIZE, image::imageops::FilterType::Triangle));
    std::fs::create_dir_all(&dir).map_err(|e| AppError { code: "io".into(), message: "Could not save the face.".into(), detail: Some(e.to_string()) })?;
    photos_media::encode_jpeg(&crop, 85, &out).map_err(|e| AppError { code: "io".into(), message: "Could not save the face.".into(), detail: Some(e.to_string()) })?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Round 4B, Task 4: the whole rescan replayed on a COPY of a library's database
    /// (`LUMEN_DB`). From `LUMEN_LIBRARY` it only reads: the cached previews, and the
    /// originals of plain images without one (anything a decode would cache goes to a temp
    /// folder). Photos needing FFmpeg with no preview are left as they were. Every pinned or
    /// rejected face must keep its place.
    ///
    /// ```sh
    /// LUMEN_DB=<copy> LUMEN_LIBRARY=<library> LUMEN_MODELS_DIR=<models> cargo test --release -p lumen --lib rescan_on_a_real_library -- --ignored --nocapture
    /// ```
    #[test]
    #[ignore = "needs a copy of a real library.db and the models"]
    fn rescan_on_a_real_library() {
        use photos_ml::manifest::manifest;
        let env = |k: &str| std::env::var(k).unwrap_or_else(|_| panic!("{k}"));
        let db = photos_core::db::Db::open(std::path::Path::new(&env("LUMEN_DB"))).unwrap();
        let library = std::path::PathBuf::from(env("LUMEN_LIBRARY"));
        let store = photos_media::store::DerivativeStore::new(&library);
        let scratch_dir = tempfile::tempdir().unwrap();
        let scratch = photos_media::store::DerivativeStore::new(scratch_dir.path());
        let models = std::path::PathBuf::from(env("LUMEN_MODELS_DIR"));
        let entry = |role: &str| manifest().by_role(role).unwrap();
        let mut det = FaceDetector::open(&models.join(&entry("face_detection").file), entry("face_detection")).unwrap();
        let mut emb = FaceEmbedder::open(&models.join(&entry("face_embedding").file), entry("face_embedding")).unwrap();
        let mut enc = ImageEncoder::open(&models.join(&entry("image_embedding").file), entry("image_embedding")).unwrap();
        let key = scan_key(&entry("face_detection").id);
        let pinned = || -> Vec<(i64, Option<i64>, Option<String>)> {
            db.with_conn(|c| {
                let mut s = c.prepare("SELECT id, person_id, rejected FROM faces WHERE pinned = 1 OR rejected = 'user' ORDER BY id")?;
                let rows = s.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
                Ok(rows.collect::<Result<Vec<_>, _>>()?)
            })
            .unwrap()
        };
        let count = |sql: &str| -> i64 { db.with_conn(|c| Ok(c.query_row(sql, [], |r| r.get(0))?)).unwrap() };
        let before = pinned();
        let faces_before = count("SELECT COUNT(*) FROM faces WHERE rejected IS NULL");
        let started = std::time::Instant::now();
        let (mut scanned, mut skipped) = (0usize, 0usize);
        for (id, hash) in db.with_conn(|c| people::unscanned(c, &key, usize::MAX >> 1)).unwrap() {
            let img = image::open(store.preview_path(&hash)).ok().or_else(|| {
                let info = db.with_conn(|c| photos_core::assets::render_info(c, id)).ok()?;
                let kind = photos_media::kind_for_extension(&info.extension);
                let source = library.join("originals").join(&info.relative_path);
                let spec = crate::derivatives::RenderSpec {
                    hash: &hash,
                    source: &source,
                    kind,
                    orientation: info.orientation as u16,
                    hdr: photos_media::hdr::HdrKind::from_code(info.hdr),
                    edit: None,
                    ai_rasters: None,
                };
                photos_media::is_renderable_with(kind, false).then(|| crate::derivatives::decode_for_scan(&scratch, &spec, None).ok()).flatten()
            });
            let Some(img) = img else {
                skipped += 1;
                continue;
            };
            let (rgb, w, h) = crate::embeddings::rgb_of(&scan_image(&img));
            let mut faces = detect_faces(&mut det, &mut emb, &rgb, w, h);
            judge_with(Some(&mut enc), &rgb, w, h, &mut faces);
            scanned += 1;
            db.with_conn(|c| people::replace_faces_keeping_identity(c, id, &key, &faces)).unwrap();
        }
        let took = started.elapsed();
        let placed = db.with_conn(|c| people::recluster(c, &people::Thresholds::MEASURED, 0)).unwrap();
        let after = pinned();
        let faces_after = count("SELECT COUNT(*) FROM faces WHERE rejected IS NULL");
        let rejected = count("SELECT COUNT(*) FROM faces WHERE rejected IS NOT NULL AND rejected != 'user'");
        let suggestable = count("SELECT COUNT(*) FROM faces WHERE person_id IS NULL AND rejected IS NULL AND pinned = 0");
        println!(
            "scanned {scanned} (skipped {skipped}: no preview and needing FFmpeg) in {took:.1?}, {:.1} photos/s; faces {faces_before} → {faces_after}, filtered {rejected}, placed {placed}, unplaced {suggestable}; user decisions {} → {}",
            scanned as f64 / took.as_secs_f64(),
            before.len(),
            after.len()
        );
        assert_eq!(before, after, "every pinned or rejected face keeps its person");
    }

    /// Round 4I, Task 4: the second look replayed on a COPY of a library's database
    /// (`LUMEN_DB`), reading only from `LUMEN_LIBRARY` as `rescan_on_a_real_library` does.
    /// Prints how many photos of people with no face found now have one, the time per photo,
    /// and (with `LUMEN_ME=<person id>`) that person's photos before and after a recluster.
    ///
    /// ```sh
    /// LUMEN_DB=<copy> LUMEN_LIBRARY=<library> LUMEN_MODELS_DIR=<models> [LUMEN_ME=60] cargo test --release -p lumen --lib second_chance_on_a_real_library -- --ignored --nocapture
    /// ```
    #[test]
    #[ignore = "needs a copy of a real library.db and the models"]
    fn second_chance_on_a_real_library() {
        use photos_ml::manifest::manifest;
        let env = |k: &str| std::env::var(k).unwrap_or_else(|_| panic!("{k}"));
        let db = photos_core::db::Db::open(std::path::Path::new(&env("LUMEN_DB"))).unwrap();
        let library = std::path::PathBuf::from(env("LUMEN_LIBRARY"));
        let store = photos_media::store::DerivativeStore::new(&library);
        let scratch_dir = tempfile::tempdir().unwrap();
        let scratch = photos_media::store::DerivativeStore::new(scratch_dir.path());
        let models = std::path::PathBuf::from(env("LUMEN_MODELS_DIR"));
        let entry = |role: &str| manifest().by_role(role).unwrap();
        let mut det = FaceDetector::open(&models.join(&entry("face_detection").file), entry("face_detection")).unwrap();
        let mut emb = FaceEmbedder::open(&models.join(&entry("face_embedding").file), entry("face_embedding")).unwrap();
        let mut enc = ImageEncoder::open(&models.join(&entry("image_embedding").file), entry("image_embedding")).unwrap();
        let key = scan_key(&entry("face_detection").id);
        let second = people::second_chance_key(&key);
        let me: Option<i64> = std::env::var("LUMEN_ME").ok().and_then(|v| v.parse().ok());
        let count = |sql: &str| -> i64 { db.with_conn(|c| Ok(c.query_row(sql, [], |r| r.get(0))?)).unwrap() };
        let decisions = || count("SELECT COUNT(*) FROM faces WHERE pinned = 1 OR rejected = 'user'");
        let mine = || me.map(|p| count(&format!("SELECT COUNT(DISTINCT asset_id) FROM faces WHERE person_id = {p} AND rejected IS NULL")));
        let (decisions_before, mine_before) = (decisions(), mine());
        let todo = db.with_conn(|c| people::second_chance(c, &key, usize::MAX >> 1)).unwrap();
        println!("{} photos of people with no face found (scan key {key})", todo.len());
        let started = std::time::Instant::now();
        let (mut looked, mut skipped, mut with_faces, mut found, mut filtered) = (0usize, 0usize, 0usize, 0usize, 0usize);
        for id in todo {
            let hash: String = db.with_conn(|c| Ok(c.query_row("SELECT content_hash FROM assets WHERE id = ?1", [id], |r| r.get(0))?)).unwrap();
            let img = image::open(store.preview_path(&hash)).ok().or_else(|| {
                let info = db.with_conn(|c| photos_core::assets::render_info(c, id)).ok()?;
                let kind = photos_media::kind_for_extension(&info.extension);
                let source = library.join("originals").join(&info.relative_path);
                let spec = crate::derivatives::RenderSpec {
                    hash: &hash,
                    source: &source,
                    kind,
                    orientation: info.orientation as u16,
                    hdr: photos_media::hdr::HdrKind::from_code(info.hdr),
                    edit: None,
                    ai_rasters: None,
                };
                photos_media::is_renderable_with(kind, false).then(|| crate::derivatives::decode_for_scan(&scratch, &spec, None).ok()).flatten()
            });
            let Some(img) = img else {
                skipped += 1;
                continue;
            };
            let (rgb, w, h) = crate::embeddings::rgb_of(&fit_within(&img, SECOND_SIZE));
            let boxes = det.detect_tiled(&rgb, w, h, SECOND_TILE).unwrap_or_default();
            let mut faces = embed_found(&mut emb, &rgb, w, h, &boxes);
            judge_with(Some(&mut enc), &rgb, w, h, &mut faces);
            looked += 1;
            let real = faces.iter().filter(|f| f.rejected.is_none()).count();
            filtered += faces.len() - real;
            found += real;
            with_faces += usize::from(real > 0);
            db.with_conn(|c| people::store_second_look(c, id, &second, &faces)).unwrap();
        }
        let took = started.elapsed();
        let placed = db.with_conn(|c| people::recluster(c, &people::Thresholds::MEASURED, 0)).unwrap();
        println!(
            "looked again at {looked} (skipped {skipped}: no preview and needing FFmpeg) in {took:.1?}, {:.0} ms a photo; {with_faces} now have a face ({found} faces, {filtered} filtered); recluster placed {placed}; person {me:?}: {mine_before:?} → {:?} photos; user decisions {decisions_before} → {}",
            took.as_secs_f64() * 1000.0 / looked.max(1) as f64,
            mine(),
            decisions()
        );
        assert_eq!(decisions_before, decisions(), "user decisions are never touched");
    }

    /// Round 4I, Task 6: a box drawn loosely around Aldrin (the Apollo 11 crew photo) is the
    /// face the scan finds there, aligned on its landmarks; a box over the backdrop is embedded
    /// as it is, with quality 0.
    ///
    /// ```sh
    /// LUMEN_MODELS_DIR=<models> cargo test --release -p lumen --lib embed_box_uses_landmarks -- --ignored --nocapture
    /// ```
    #[test]
    #[ignore = "needs the models"]
    fn embed_box_uses_landmarks_when_the_detector_finds_a_face_inside() {
        use photos_ml::manifest::manifest;
        let models = std::path::PathBuf::from(std::env::var("LUMEN_MODELS_DIR").expect("LUMEN_MODELS_DIR"));
        let entry = |role: &str| manifest().by_role(role).unwrap();
        let mut det = FaceDetector::open(&models.join(&entry("face_detection").file), entry("face_detection")).unwrap();
        let mut emb = FaceEmbedder::open(&models.join(&entry("face_embedding").file), entry("face_embedding")).unwrap();
        let img = image::load_from_memory(include_bytes!("../crates/photos-ml/tests/fixtures/apollo11.jpg")).unwrap();
        let (rgb, w, h) = crate::embeddings::rgb_of(&img);
        let scanned = detect_faces(&mut det, &mut emb, &rgb, w, h);
        let aldrin = scanned.iter().find(|f| (f.x * w as f32 - 214.8).abs() < 4.0).expect("the scan finds the face at x = 215");
        // Drawn by hand: too big, and off-centre.
        let (fw, fh) = (w as f32, h as f32);
        let rect = [195.0 / fw, 255.0 / fh, 125.0 / fw, 160.0 / fh];
        let drawn = embed_box_with(&mut det, &mut emb, &rgb, w, h, rect).unwrap();
        let cos = photos_core::embedding::cosine(&drawn.vec, &aldrin.vec);
        println!("drawn vs scanned: cosine {cos:.4}, quality {:.3}, box {:?}", drawn.quality, [drawn.x * fw, drawn.y * fh, drawn.w * fw, drawn.h * fh]);
        // The detector runs on the box grown and brought up to 320 px, so its landmarks move by
        // a fraction of a pixel: 0.98 measured. Two photos of one person sit near 0.5–0.7.
        assert!(cos > 0.95, "{cos}");
        assert!(drawn.quality > 0.0);
        assert!((drawn.x - aldrin.x).abs() < 0.01 && (drawn.w - aldrin.w).abs() < 0.01, "the detector's box, not the drawn one");

        let sky = embed_box_with(&mut det, &mut emb, &rgb, w, h, [0.02, 0.02, 0.08, 0.08]).unwrap();
        assert_eq!(sky.quality, 0.0);
        assert_eq!([sky.x, sky.y, sky.w, sky.h], [0.02, 0.02, 0.08, 0.08], "the drawn box, as drawn");
    }

    /// A library of `n` photos (ids 1..=n) with these scene tags, for the scan loop.
    fn photos_db(n: i64, tags: &[(i64, &str)]) -> photos_core::db::Db {
        let db = photos_core::db::Db::open_in_memory().unwrap();
        db.with_conn(|c| {
            let rows: Vec<String> = (1..=n).map(|i| format!("({i}, 'h{i}', 'p', '{i}.jpg', 'jpg', 0, 1, 1, 10)")).collect();
            c.execute_batch(&format!(
                "INSERT INTO assets (id, content_hash, relative_path, filename, extension, media_type, byte_size, orientation, imported_at) VALUES {};",
                rows.join(", ")
            ))?;
            for (id, t) in tags {
                photos_core::embedding::set_scene_tags(c, *id, &[t.to_string()])?;
            }
            Ok(())
        })
        .unwrap();
        db
    }

    /// Finds `first[id]` faces on a photo's first look and `second[id]` on its second.
    #[derive(Default)]
    struct Fake {
        first: std::collections::HashMap<i64, usize>,
        second: std::collections::HashMap<i64, usize>,
        looks: Vec<(i64, Look)>,
    }

    impl Scanner for Fake {
        fn look(&mut self, asset: i64, how: Look) -> Found {
            self.looks.push((asset, how));
            let n = match how {
                Look::First => self.first.get(&asset),
                Look::Second => self.second.get(&asset),
            };
            Found::Faces((0..*n.unwrap_or(&0)).map(|i| NewFace { x: 0.1 + 0.3 * i as f32, y: 0.2, w: 0.2, h: 0.2, score: 0.9, vec: vec![1; 128], quality: 0.9, rejected: None }).collect())
        }
    }

    fn quiet(db: &photos_core::db::Db, scanner: &mut Fake) -> (usize, usize, Vec<usize>) {
        let (mut regrouped, mut left) = (0, Vec::new());
        let scanned = run(db, "d+r4", scanner, &mut Hooks { regroup: &mut || regrouped += 1, progress: &mut |n| left.push(n), every: std::time::Duration::ZERO, cancelled: &|| false });
        (scanned, regrouped, left)
    }

    /// Round 4I: a photo of people where the first look found no face is looked at once more
    /// (larger, in tiles); what it finds is stored, the key moves to `+r4i`, and a later run
    /// looks at nothing again.
    #[test]
    fn a_photo_with_no_face_and_a_people_scene_is_rescanned_once() {
        let db = photos_db(2, &[(1, "party"), (2, "beach")]);
        let mut fake = Fake { second: [(1, 1)].into(), ..Default::default() };
        quiet(&db, &mut fake);
        assert_eq!(fake.looks, vec![(2, Look::First), (1, Look::First), (1, Look::Second)], "the beach gets no second look");
        let (faces, key): (i64, String) = db.with_conn(|c| Ok(c.query_row("SELECT (SELECT COUNT(*) FROM faces WHERE asset_id = 1), model FROM face_scan WHERE asset_id = 1", [], |r| Ok((r.get(0)?, r.get(1)?)))?)).unwrap();
        assert_eq!((faces, key.as_str()), (1, "d+r4i"));
        let mut again = Fake::default();
        quiet(&db, &mut again);
        assert!(again.looks.is_empty(), "{:?}", again.looks);
    }

    /// Round 4I (report 5): people are regrouped after every batch that stored faces, so new
    /// people appear while a big import is still being scanned; the ring hears what is left.
    #[test]
    fn recluster_runs_after_each_batch_that_found_faces() {
        // Newest first in batches of 16: 40–25, 24–9, 8–1. Faces in the first and the last.
        let db = photos_db(40, &[]);
        let mut fake = Fake { first: [(40, 2), (3, 1)].into(), ..Default::default() };
        let (scanned, regrouped, left) = quiet(&db, &mut fake);
        assert_eq!(scanned, 40);
        assert_eq!(regrouped, 2);
        assert_eq!(left, vec![24, 8, 0]);
    }

    /// Review fix: without the image model no crop is checked, and the scan says so, so the
    /// photo waits for the model instead of being stamped scanned unfiltered.
    #[test]
    fn without_the_image_model_nothing_is_judged() {
        let rgb = vec![128u8; 64 * 64 * 3];
        let mut faces = vec![NewFace { x: 0.25, y: 0.25, w: 0.5, h: 0.5, score: 0.9, vec: vec![0; 128], quality: 0.0, rejected: None }];
        assert!(!judge_with(None, &rgb, 64, 64, &mut faces));
        assert_eq!(faces[0].rejected, None);
    }

    /// Round 4B: every photo is scanned once more, at 1536 px, under a new key.
    #[test]
    fn the_rescan_has_its_own_key_and_size() {
        assert_eq!(scan_key("yunet@2023mar"), "yunet@2023mar+r4");
        let img = image::DynamicImage::new_rgb8(2560, 1920);
        let s = scan_image(&img);
        assert_eq!((s.width(), s.height()), (1536, 1152));
        let small = image::DynamicImage::new_rgb8(800, 600);
        assert_eq!(scan_image(&small).width(), 800, "never enlarged");
    }

    #[test]
    fn a_crop_is_square_around_the_face_and_inside_the_photo() {
        let (x, y, side) = crop_rect([0.4, 0.4, 0.1, 0.1], 1000, 800);
        assert_eq!(side, 160, "1.6 times the face");
        assert_eq!((x, y), (370, 280));
        // A face at the corner is kept inside the picture.
        let (x, y, side) = crop_rect([0.0, 0.0, 0.1, 0.1], 1000, 800);
        assert_eq!((x, y, side), (0, 0, 160));
        // A face bigger than the photo's short side is cropped to it.
        let (_, _, side) = crop_rect([0.0, 0.0, 1.0, 1.0], 1000, 800);
        assert_eq!(side, 800);
    }

    #[test]
    fn a_crop_is_turned_like_the_photo_it_shows() {
        use photos_core::edits::Recipe;
        // 2×1: red on the left, blue on the right.
        let mut img = image::RgbImage::new(2, 1);
        img.put_pixel(0, 0, image::Rgb([255, 0, 0]));
        img.put_pixel(1, 0, image::Rgb([0, 0, 255]));
        let img = image::DynamicImage::ImageRgb8(img);
        let px = |i: &image::DynamicImage, x, y| i.to_rgb8().get_pixel(x, y).0;

        assert_eq!(Turn::of(None), Turn::default());
        assert_eq!(Turn::of(Some(&Recipe::identity())), Turn::default());
        assert_eq!(Turn::default().file_name(7), "7.jpg", "an unturned photo keeps its old crop");

        // A quarter turn clockwise puts the left edge on top.
        let cw = Turn::of(Some(&Recipe { rotate: 90, ..Recipe::identity() }));
        assert_ne!(cw.file_name(7), "7.jpg", "a turn changes the cached file");
        let out = cw.apply(img.clone());
        assert_eq!((out.width(), out.height()), (1, 2));
        assert_eq!(px(&out, 0, 0), [255, 0, 0]);

        // Flips come before the turn, as in the render (`develop::geometry`).
        let both = Turn::of(Some(&Recipe { rotate: 90, flip_h: true, ..Recipe::identity() }));
        assert_eq!(px(&both.apply(img.clone()), 0, 0), [0, 0, 255]);
        assert_eq!(px(&Turn::of(Some(&Recipe { rotate: 180, ..Recipe::identity() })).apply(img), 0, 0), [0, 0, 255]);
    }

    #[test]
    fn the_crop_pixels_are_the_square_around_the_face() {
        // 4×2 RGB image whose red channel is the pixel's index.
        let rgb: Vec<u8> = (0..8u8).flat_map(|i| [i, 0, 0]).collect();
        let px = crop_pixels(&rgb, 4, (1, 0, 2));
        assert_eq!(px.len(), 2 * 2 * 3);
        assert_eq!(px.chunks(3).map(|p| p[0]).collect::<Vec<_>>(), vec![1, 2, 5, 6]);
    }
}
