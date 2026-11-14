//! Pet scan (Round 4C): every visible photo is looked at once for cats and dogs, on the
//! Maintenance lane right after the face scan. D-FINE-S finds them in the 1024 px scan image,
//! DINOv2 describes each from its square crop, and the crop's CLIP shares over the `pet:*`
//! prototypes set plushes and cartoons aside (`photos_core::pet_filter`). Nothing is stored
//! unfiltered: without the image model a photo with an animal waits for it. Pets are regrouped
//! as the scan goes (at most every 20 s) and once at the end.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use photos_core::pet_filter;
use photos_core::pets::{self, NewPetDetection};
use photos_ml::clip::Rgb;
use photos_ml::pets::{pet_crop_rect, PetBox, CROP_MARGIN};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};
use ts_rs::TS;

use crate::error::{AppError, AppResult};
use crate::state::AppState;

pub const PETS_PROGRESS: &str = "pets://progress";

/// The pet scan's progress, for the activity ring (*Looking for pets · 1,204 left*).
#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PetsProgress {
    pub left: u32,
}

/// Photos per saved batch (one transaction) and per progress event.
const BATCH: usize = 48;
/// The long edge the detector's photo is cut from (it then sees 640 square).
const SCAN_SIZE: u32 = 1024;
const REGROUP_EVERY: Duration = Duration::from_secs(20);
/// The Pets screen's crops.
const CROP_SIZE: u32 = 200;

/// What `pet_scan.model` records: both models, so a new one of either rescans.
fn scan_key() -> Option<String> {
    let m = photos_ml::manifest::manifest();
    Some(format!("{}+{}", m.by_role("pet_detection")?.id, m.by_role("pet_embedding")?.id))
}

/// Boxes in pixels of a `w`×`h` image to what is stored: 0..1 of the photo.
fn to_new(b: &PetBox, w: u32, h: u32, vec: photos_core::embedding::Vector) -> NewPetDetection {
    let (fw, fh) = (w as f32, h as f32);
    let short = (b.bbox[2] - b.bbox[0]).min(b.bbox[3] - b.bbox[1]);
    NewPetDetection {
        species: b.species.as_str().into(),
        x: b.bbox[0] / fw,
        y: b.bbox[1] / fh,
        w: (b.bbox[2] - b.bbox[0]) / fw,
        h: (b.bbox[3] - b.bbox[1]) / fh,
        score: b.score,
        vec,
        // `short` is in pixels of the scan image, whose long edge is at most SCAN_SIZE.
        quality: pet_filter::quality(b.score, short),
        rejected: None,
    }
}

/// One photo as a worker saw it: its pixels at the scan's size and the animals in them.
pub(crate) struct Seen {
    rgb: Vec<u8>,
    w: u32,
    h: u32,
    boxes: Vec<PetBox>,
}

/// A decoded photo at the scan's size: its long edge at most `SCAN_SIZE`.
fn scan_pixels(img: &image::DynamicImage) -> (Vec<u8>, u32, u32) {
    if img.width().max(img.height()) > SCAN_SIZE {
        crate::embeddings::rgb_of(&img.resize(SCAN_SIZE, SCAN_SIZE, image::imageops::FilterType::Triangle))
    } else {
        crate::embeddings::rgb_of(img)
    }
}

/// How many detectors run side by side and with how many threads each (Round 8A). Measured on
/// 24 threads (`photos-ml --test pet_detector_speed`): 3 × 4 is the fastest layout, 2.3–2.7×
/// one detector × 2; more sessions or more threads each go slower. Half the machine at most.
pub(crate) fn detector_layout() -> (usize, usize) {
    let cores = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4);
    let sessions = (cores / 8).clamp(1, 3);
    (sessions, (cores / 2 / sessions).clamp(1, 4))
}

/// How long after the user's last render the scan stays paused.
const STEP_ASIDE: Duration = Duration::from_millis(600);

type Decode = std::sync::Arc<dyn Fn(i64) -> Option<image::DynamicImage> + Send + Sync>;

/// The detector workers (Round 8A): `sessions` threads at background priority, each with its
/// own detector of `threads` threads (ONNX Runtime's lowered too). They take photos from `ids`,
/// decode them with `decode`, and send back what each shows, or `None` when it cannot be
/// read or its detector would not open. Between photos they wait while the user is busy.
/// They stop when `ids` closes.
pub(crate) fn spawn_detectors(
    (sessions, threads): (usize, usize),
    model: (PathBuf, &'static photos_ml::manifest::ModelEntry),
    decode: Decode,
    ids: std::sync::mpsc::Receiver<i64>,
    out: std::sync::mpsc::Sender<(i64, Option<Seen>)>,
) -> Vec<std::thread::JoinHandle<()>> {
    let ids = std::sync::Arc::new(std::sync::Mutex::new(ids));
    (0..sessions)
        .map(|i| {
            let (ids, out, decode, (path, entry)) = (ids.clone(), out.clone(), decode.clone(), (model.0.clone(), model.1));
            std::thread::Builder::new()
                .name(format!("lumen-pets-{i}"))
                .spawn(move || {
                    photos_jobs::background_priority();
                    let mut det = photos_ml::pets::PetDetector::open_with(&path, entry, threads, Some(photos_jobs::background_priority));
                    if let Err(e) = &det {
                        tracing::warn!("pet detector: {e}");
                    }
                    loop {
                        let next = ids.lock().ok().and_then(|r| r.recv().ok());
                        let Some(asset) = next else { break };
                        while photos_jobs::user_busy(STEP_ASIDE) {
                            std::thread::sleep(Duration::from_millis(100));
                        }
                        let seen = det.as_mut().ok().and_then(|det| {
                            let (rgb, w, h) = scan_pixels(&decode(asset)?);
                            match det.detect(&rgb, w, h) {
                                Ok(boxes) => Some(Seen { rgb, w, h, boxes }),
                                Err(e) => {
                                    tracing::debug!(asset, "pet scan: {e}");
                                    None
                                }
                            }
                        });
                        if out.send((asset, seen)).is_err() {
                            break;
                        }
                    }
                })
                .expect("spawn a pet detector")
        })
        .collect()
}

/// Who each animal a worker found is, and whether it is a pet at all (Round 4C's crop check):
/// `embed` describes the square crops, `clip` gives their CLIP vectors. `None` when either
/// model is missing: the photo then waits for them.
pub(crate) fn finish(
    seen: &Seen,
    embed: impl FnOnce(&[Rgb<'_>]) -> Option<Vec<photos_core::embedding::Vector>>,
    clip: impl FnOnce(&[Rgb<'_>]) -> Option<Vec<photos_core::embedding::Vector>>,
) -> Option<Vec<NewPetDetection>> {
    let Seen { rgb, w, h, boxes } = seen;
    if boxes.is_empty() {
        return Some(Vec::new());
    }
    let rects: Vec<(u32, u32, u32)> = boxes.iter().map(|b| pet_crop_rect(b.bbox, *w, *h, CROP_MARGIN)).collect();
    let crops: Vec<Vec<u8>> = rects.iter().map(|r| crate::faces::crop_pixels(rgb, *w, *r)).collect();
    let batch: Vec<Rgb<'_>> = crops.iter().zip(&rects).map(|(c, r)| Rgb { data: c, width: r.2, height: r.2 }).collect();
    let vecs = embed(&batch)?;
    let shares = clip(&batch)?;
    let protos = photos_ml::prototypes();
    let mut out: Vec<NewPetDetection> = boxes.iter().zip(vecs).map(|(b, v)| to_new(b, *w, *h, v)).collect();
    // The crop check: a plush or a cartoon is not a pet. And which animal it is, when CLIP is
    // sure the detector got it wrong (some black cats read as dogs).
    for (d, v) in out.iter_mut().zip(shares) {
        let crop = photos_core::embedding::softmax(&v, &protos, "pet:", pet_filter::CROP_TEMPERATURE);
        d.rejected = pet_filter::crop_reason(&crop).map(str::to_string);
        let kind = photos_core::embedding::softmax(&v, &protos, "species:", pet_filter::CROP_TEMPERATURE);
        d.species = pet_filter::species(&d.species, &kind).to_string();
    }
    Some(out)
}

/// Regroup pets and tell the frontend when anything moved.
fn regroup_now(app: &AppHandle) {
    let state = app.state::<AppState>();
    let Ok(lib) = state.library() else { return };
    match lib.db.with_conn(|c| pets::recluster(c, &pets::MEASURED, photos_core::now_unix())) {
        Ok(0) => {}
        Ok(n) => {
            tracing::info!(placed = n, "pets regrouped");
            state.changes.changed(app, true);
        }
        Err(e) => tracing::warn!("pets: {e}"),
    }
}

/// Find the pets in everything not yet scanned, regrouping as it goes. Coalesced; after the
/// face scan (`embeddings::schedule`), on the tick and when models arrive.
///
/// Round 8A: photos whose CLIP embedding shows no dog or cat skip the detector; the rest go to
/// [`detector_layout`] detectors at background priority that pause while the user is busy.
/// Results are saved a batch at a time in one transaction.
pub fn schedule(app: &AppHandle) {
    if !crate::models::enabled(app) {
        return;
    }
    let a = app.clone();
    app.state::<AppState>().jobs.submit("maintenance:pets", photos_jobs::Lane::Maintenance, move |cancel| {
        let state = a.state::<AppState>();
        let Ok(lib) = state.library() else { return };
        let (Some(key), Some(model)) = (scan_key(), crate::models::pet_detector_source(&a)) else { return };
        let protos = photos_ml::prototypes();
        let layout = detector_layout();
        let (id_tx, id_rx) = std::sync::mpsc::channel::<i64>();
        let (seen_tx, seen_rx) = std::sync::mpsc::channel();
        let decoder = a.clone();
        let decode: Decode = std::sync::Arc::new(move |asset| crate::derivatives::scan_image_of(&decoder, asset).ok());
        let workers = spawn_detectors(layout, model, decode, id_rx, seen_tx);
        let (started, mut scanned, mut skipped, mut pending) = (Instant::now(), 0usize, 0usize, false);
        let mut last_regroup = Instant::now();
        let embed = |b: &[Rgb<'_>]| crate::models::with_pet_embedder(&a, |e| e.embed_batch(b).ok()).flatten();
        let clip = |b: &[Rgb<'_>]| crate::models::with_encoder(&a, |e| e.embed_batch(b).ok().map(|v| v.into_iter().map(|(v, _)| v).collect())).flatten();
        'scan: loop {
            if cancel.is_cancelled() || state.library().map_or(true, |l| l.root != lib.root) {
                break;
            }
            let Ok(batch) = lib.db.with_conn(|c| pets::unscanned(c, &key, BATCH)) else { break };
            if batch.is_empty() {
                break;
            }
            let ids: Vec<i64> = batch.iter().map(|(id, _)| *id).collect();
            let Ok((look, skip)) = lib.db.with_conn(|c| pets::split_for_detector(c, &ids, &protos)) else { break };
            let mut results: Vec<(i64, Option<Vec<NewPetDetection>>)> = skip.iter().map(|id| (*id, None)).collect();
            skipped += skip.len();
            for id in &look {
                let _ = id_tx.send(*id);
            }
            let mut models_gone = false;
            for _ in 0..look.len() {
                let Ok((asset, seen)) = seen_rx.recv() else { break 'scan };
                if models_gone {
                    continue;
                }
                let found = match &seen {
                    None => None,
                    Some(seen) => match finish(seen, embed, clip) {
                        Some(found) => Some(found),
                        // The models went away: what is saved stays, the rest waits for them.
                        None => {
                            models_gone = true;
                            continue;
                        }
                    },
                };
                pending |= found.as_ref().is_some_and(|f| f.iter().any(|d| d.rejected.is_none()));
                results.push((asset, found));
            }
            if let Err(e) = lib.db.with_conn(|c| pets::save_scans(c, &key, &results)) {
                // The database is failing; try again later.
                tracing::warn!("pet scan: {e}");
                break;
            }
            scanned += results.len();
            if models_gone {
                break;
            }
            let left = lib.db.with_conn(|c| pets::unscanned_count(c, &key)).unwrap_or(0);
            let _ = a.emit(PETS_PROGRESS, PetsProgress { left: left as u32 });
            if pending && last_regroup.elapsed() >= REGROUP_EVERY {
                regroup_now(&a);
                pending = false;
                last_regroup = Instant::now();
            }
        }
        drop(id_tx);
        for w in workers {
            let _ = w.join();
        }
        let _ = a.emit(PETS_PROGRESS, PetsProgress { left: 0 });
        if pending {
            regroup_now(&a);
        }
        if scanned > 0 {
            let secs = started.elapsed().as_secs_f32().max(0.001);
            tracing::info!(photos = scanned, skipped, per_second = scanned as f32 / secs, sessions = layout.0, threads = layout.1, "pets scanned");
        }
    });
}

/// Regroup after the user changed pets (a split, a merge, a name, an answer), coalesced.
pub fn schedule_recluster(app: &AppHandle) {
    if !crate::models::enabled(app) {
        return;
    }
    let a = app.clone();
    app.state::<AppState>().jobs.submit("maintenance:pets-regroup", photos_jobs::Lane::Maintenance, move |_| regroup_now(&a));
}

/// The crop for `photos://pet/{id}`, cut from the 1024 px rendering and turned like the photo.
pub fn ensure_crop(app: &AppHandle, detection: i64) -> AppResult<PathBuf> {
    let state = app.state::<AppState>();
    let lib = state.library()?;
    let not_found = || AppError { code: "not_found".into(), message: "No such pet photo.".into(), detail: None };
    let (asset, rect) = lib.db.with_conn(|c| pets::detection_rect(c, detection))?.ok_or_else(not_found)?;
    let turn = crate::faces::Turn::of(lib.db.with_conn(|c| photos_core::edits::get_recipe(c, asset))?.map(|(r, _)| r).as_ref());
    let dir = lib.root.join("derivatives").join("pets");
    let out = dir.join(turn.file_name(detection));
    if out.is_file() {
        return Ok(out);
    }
    let img = image::open(crate::derivatives::render_now(app, asset, 1024)?).map_err(|e| AppError { code: "render_failed".into(), message: "The photo could not be read.".into(), detail: Some(e.to_string()) })?;
    let (w, h) = (img.width() as f32, img.height() as f32);
    let bbox = [rect[0] * w, rect[1] * h, (rect[0] + rect[2]) * w, (rect[1] + rect[3]) * h];
    let (x, y, side) = pet_crop_rect(bbox, img.width(), img.height(), CROP_MARGIN);
    let crop = turn.apply(img.crop_imm(x, y, side, side).resize_exact(CROP_SIZE, CROP_SIZE, image::imageops::FilterType::Triangle));
    std::fs::create_dir_all(&dir).map_err(|e| AppError { code: "io".into(), message: "Could not save the crop.".into(), detail: Some(e.to_string()) })?;
    photos_media::encode_jpeg(&crop, 85, &out).map_err(|e| AppError { code: "io".into(), message: "Could not save the crop.".into(), detail: Some(e.to_string()) })?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use photos_ml::pets::Species;

    /// Round 8A's pet scan against the one before it, on a copy of a real catalogue: the same
    /// photos (the newest `LUMEN_PET_PHOTOS`, default 400), the old way (one detector × 2 threads,
    /// every photo) and the new (`detector_layout` detectors, the CLIP prefilter). Reads the
    /// library's previews and originals only; saves nothing.
    /// `LUMEN_DB=<copy> LUMEN_LIBRARY=<library> LUMEN_MODELS_DIR=<models> cargo test --release -p lumen --lib pets_scan_on_a_copy -- --ignored --nocapture`
    #[test]
    #[ignore = "needs a copy of a real library.db, the library and the models"]
    fn pets_scan_on_a_copy() {
        use photos_ml::manifest::manifest;
        let env = |k: &str| std::env::var(k).unwrap_or_else(|_| panic!("{k}"));
        let db = std::sync::Arc::new(photos_core::db::Db::open(std::path::Path::new(&env("LUMEN_DB"))).unwrap());
        let library = PathBuf::from(env("LUMEN_LIBRARY"));
        let models = PathBuf::from(env("LUMEN_MODELS_DIR"));
        let n: usize = std::env::var("LUMEN_PET_PHOTOS").ok().and_then(|v| v.parse().ok()).unwrap_or(400);
        let entry = manifest().by_role("pet_detection").unwrap();
        let ids: Vec<i64> = db
            .with_conn(|c| {
                let mut stmt = c.prepare(&format!("SELECT a.id FROM assets a WHERE a.state = 0 AND a.media_type != 1 AND NOT {} ORDER BY a.id DESC LIMIT ?1", photos_core::assets::MOVIE_HALF))?;
                let rows = stmt.query_map([n as i64], |r| r.get(0))?;
                Ok(rows.collect::<std::result::Result<Vec<i64>, _>>()?)
            })
            .unwrap();
        let scratch = tempfile::tempdir().unwrap();
        let decode: Decode = {
            let (db, library, scratch) = (db.clone(), library.clone(), scratch.path().to_path_buf());
            std::sync::Arc::new(move |id| {
                let info = db.with_conn(|c| photos_core::assets::render_info(c, id)).ok()?;
                if let Ok(img) = image::open(photos_media::store::DerivativeStore::new(&library).preview_path(&info.content_hash)) {
                    return Some(img);
                }
                let kind = photos_media::kind_for_extension(&info.extension);
                let source = library.join("originals").join(&info.relative_path);
                let spec = crate::derivatives::RenderSpec { hash: &info.content_hash, source: &source, kind, orientation: info.orientation as u16, hdr: photos_media::hdr::HdrKind::from_code(info.hdr), edit: None, ai_rasters: None };
                photos_media::is_renderable_with(kind, false).then(|| crate::derivatives::decode_for_scan(&photos_media::store::DerivativeStore::new(&scratch), &spec, None).ok()).flatten()
            })
        };
        let run = |layout: (usize, usize), look: &[i64]| {
            let (id_tx, id_rx) = std::sync::mpsc::channel();
            let (tx, rx) = std::sync::mpsc::channel();
            let started = Instant::now();
            let workers = spawn_detectors(layout, (models.join(&entry.file), entry), decode.clone(), id_rx, tx);
            for id in look {
                id_tx.send(*id).unwrap();
            }
            drop(id_tx);
            let mut found: std::collections::HashMap<i64, Vec<(String, [f32; 4])>> = Default::default();
            for (id, seen) in rx.iter().take(look.len()) {
                found.insert(id, seen.map(|s| s.boxes.iter().map(|b| (b.species.as_str().to_string(), b.bbox)).collect()).unwrap_or_default());
            }
            for w in workers {
                w.join().unwrap();
            }
            (started.elapsed().as_secs_f64(), found)
        };
        // The old scan: one detector, two threads, every photo.
        let (old_secs, old) = run((1, 2), &ids);
        println!("{} photos · before: {:.1} photos/s ({old_secs:.1} s)", ids.len(), ids.len() as f64 / old_secs);
        // The new: the CLIP prefilter, then the parallel detectors.
        let protos = photos_ml::prototypes();
        let started = Instant::now();
        let (look, skip) = db.with_conn(|c| pets::split_for_detector(c, &ids, &protos)).unwrap();
        let (_, new) = run(detector_layout(), &look);
        let new_secs = started.elapsed().as_secs_f64();
        println!(
            "after: {:.1} photos/s ({new_secs:.1} s, {:.1}×) · {:?} detectors · {} of {} skipped by the prefilter",
            ids.len() as f64 / new_secs,
            old_secs / new_secs,
            detector_layout(),
            skip.len(),
            ids.len()
        );
        let with_pets: Vec<i64> = old.iter().filter(|(_, b)| !b.is_empty()).map(|(id, _)| *id).collect();
        let lost: Vec<i64> = with_pets.iter().filter(|id| skip.contains(id)).copied().collect();
        let differ = look.iter().filter(|id| old.get(id) != new.get(id)).count();
        println!("photos where the old scan found an animal: {} · skipped by the prefilter: {lost:?} · detected differently: {differ}", with_pets.len());
        assert_eq!(differ, 0, "the parallel detectors find exactly what one did");
    }

    #[test]
    fn a_box_is_stored_as_parts_of_the_photo() {
        let b = PetBox { bbox: [100.0, 50.0, 300.0, 250.0], score: 0.8, species: Species::Dog };
        let d = to_new(&b, 1000, 500, vec![1, 2]);
        assert_eq!((d.species.as_str(), d.x, d.y, d.w, d.h), ("dog", 0.1, 0.1, 0.2, 0.4));
        assert!((d.quality - 0.8).abs() < 1e-6, "200 px is big enough to describe");
    }
}
