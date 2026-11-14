//! Reading the text in photos (Round 13): while *Read text in photos* is on and both text
//! models are in the dev folder, every still is read once on the Maintenance lane, newest
//! first, after the pet scan. The detector looks at the photo at 736 px on 4 threads (a
//! screenshot at 1536, its text is small; Julian's choice 2026-11-01, the size that meets
//! the speed budget); lines are recognised from the 1536 px image. The scan key names the
//! detection size, so changing it rescans. What is kept, and made searchable, is decided by
//! `photos_core::ocr::save_scans`.

use std::time::{Duration, Instant};

use photos_core::edits::Recipe;
use photos_core::ocr::{self, NewTextLine, StoredLine};
use photos_core::query::AssetKind;
use photos_media::develop::masks::FrameMap;
use photos_ml::ocr::{OcrEngine, TextLine};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};
use ts_rs::TS;

use crate::error::AppResult;
use crate::state::AppState;

pub const OCR_PROGRESS: &str = "ocr://progress";

/// The text scan's progress, for the activity ring (*Reading text · 1,204 left*).
#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct OcrProgress {
    pub left: u32,
}

/// A line as the viewer draws it (Round 13-3): corners TL, TR, BR, BL in 0..1 of the photo
/// as shown, its edit's flips, turns, straighten and crop applied.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ShownTextLine {
    pub ord: u32,
    pub text: String,
    pub quad: [f32; 8],
}

/// Photos per saved batch (one transaction, one reindex) and per progress event.
const BATCH: usize = 16;
/// The long edge lines are recognised from (spec §2).
const SCAN_SIZE: u32 = 1536;
/// How long after the user's last render the scan stays paused.
const STEP_ASIDE: Duration = Duration::from_millis(600);

/// The detector's long side for any photo but a screenshot.
const DET_SIDE: u32 = 736;
/// Intra-op threads per model; the job's threads run below normal priority.
const THREADS: usize = 4;

/// The detector's long side: 1536 for a screenshot, [`DET_SIDE`] for any other photo.
pub(crate) fn det_side(screenshot: bool) -> u32 {
    if screenshot { SCAN_SIZE } else { DET_SIDE }
}

/// What `ocr_scan.model` records: both models and the detection size, since reading at
/// another size finds other lines and so must rescan, like a new model.
pub(crate) fn scan_key(det_id: &str, rec_id: &str) -> String {
    format!("{det_id}+{rec_id}/{DET_SIDE}")
}

/// Lines in pixels of a `w`×`h` image to what is stored: corners as 0..1 of the photo.
pub(crate) fn to_new(lines: &[TextLine], w: u32, h: u32) -> Vec<NewTextLine> {
    let (fw, fh) = (w.max(1) as f32, h.max(1) as f32);
    lines
        .iter()
        .map(|l| {
            let mut quad = [0f32; 8];
            for (i, p) in l.quad.iter().enumerate() {
                quad[2 * i] = (p[0] / fw).clamp(0.0, 1.0);
                quad[2 * i + 1] = (p[1] / fh).clamp(0.0, 1.0);
            }
            NewTextLine { text: l.text.clone(), conf: l.conf, quad }
        })
        .collect()
}

/// A decoded photo at the scan's size: its long edge at most [`SCAN_SIZE`].
fn scan_pixels(img: &image::DynamicImage) -> (Vec<u8>, u32, u32) {
    if img.width().max(img.height()) > SCAN_SIZE {
        crate::embeddings::rgb_of(&img.resize(SCAN_SIZE, SCAN_SIZE, image::imageops::FilterType::Triangle))
    } else {
        crate::embeddings::rgb_of(img)
    }
}

/// One photo read: its lines, or none when it would not decode or the reader failed on it
/// (it is still marked read, so it never loops; a new model reads it again).
fn read_one(app: &AppHandle, engine: &mut OcrEngine, asset: i64, screenshot: bool) -> Vec<NewTextLine> {
    let img = match crate::derivatives::scan_image_of(app, asset) {
        Ok(img) => img,
        Err(e) => {
            tracing::debug!(asset, cause = %e.message, "text scan: not decoded");
            return Vec::new();
        }
    };
    let (rgb, w, h) = scan_pixels(&img);
    match engine.read(&rgb, w, h, det_side(screenshot)) {
        Ok(lines) => to_new(&lines, w, h),
        Err(e) => {
            tracing::debug!(asset, "text scan: {e}");
            Vec::new()
        }
    }
}

/// Stored lines on the photo as shown: through the edit's geometry (`FrameMap`, the
/// renderer's own) when there is one. `w × h` is the display-oriented size. A line whose
/// centre the crop leaves out is left out.
pub(crate) fn shown_lines(lines: &[StoredLine], w: u32, h: u32, recipe: Option<&Recipe>) -> Vec<ShownTextLine> {
    let (w, h) = (w.max(1), h.max(1));
    let map = recipe.map(|r| (FrameMap::new(w, h, r), FrameMap::frame_size(w, h, r)));
    lines
        .iter()
        .filter_map(|l| {
            let mut quad = l.quad;
            if let Some((m, [fw, fh])) = &map {
                for i in 0..4 {
                    let [x, y] = m.to_frame(l.quad[2 * i], l.quad[2 * i + 1]);
                    quad[2 * i] = x / fw;
                    quad[2 * i + 1] = y / fh;
                }
            }
            let cx = (quad[0] + quad[2] + quad[4] + quad[6]) / 4.0;
            let cy = (quad[1] + quad[3] + quad[5] + quad[7]) / 4.0;
            ((0.0..=1.0).contains(&cx) && (0.0..=1.0).contains(&cy)).then(|| ShownTextLine { ord: l.ord as u32, text: l.text.clone(), quad })
        })
        .collect()
}

/// Whether text is read and shown at all: the switch is on and both models are there.
pub(crate) fn reads(on: bool, key: Option<&str>) -> bool {
    on && key.is_some()
}

/// The current scan key while text is read, else `None` (switch off or models missing).
fn current_key(state: &AppState) -> Option<String> {
    let key = crate::models::ocr_scan_key();
    if reads(crate::models::read_text_on(state), key.as_deref()) { key } else { None }
}

/// Reads one photo with the interactive engine (opened once, kept in `MlState`, reopened
/// when the models change; normal priority, unlike the background job's) and saves it.
fn read_and_save(app: &AppHandle, asset: i64, key: &str) {
    let state = app.state::<AppState>();
    let Ok(lib) = state.library() else { return };
    // Read meanwhile by the background job or an earlier open.
    if lib.db.with_conn(|c| ocr::scanned_with(c, asset, key)).unwrap_or(true) {
        return;
    }
    let sql = format!("SELECT {} FROM assets a WHERE a.id = ?1", AssetKind::Screenshot.predicate());
    let screenshot = lib.db.with_conn(|c| Ok(c.query_row(&sql, [asset], |r| r.get::<_, bool>(0))?)).unwrap_or(false);
    let Ok(mut slot) = state.ml.ocr.lock() else { return };
    if slot.as_ref().map(|(k, _)| k.as_str()) != Some(key) {
        let Some(((det_path, det), (rec_path, rec))) = crate::models::ocr_sources() else { return };
        match OcrEngine::open((&det_path, &det), (&rec_path, &rec), THREADS, None) {
            Ok(e) => *slot = Some((key.to_string(), e)),
            Err(e) => {
                tracing::debug!("text models would not open: {e}");
                return;
            }
        }
    }
    let Some((_, engine)) = slot.as_mut() else { return };
    let lines = read_one(app, engine, asset, screenshot);
    drop(slot);
    if let Err(e) = lib.db.with_conn(|c| ocr::save_scans(c, key, &[(asset, lines)])) {
        tracing::warn!(asset, "text scan: {e}");
    }
}

/// A still's display-oriented size; `None` for a video.
fn still_size(state: &AppState, asset: i64) -> AppResult<Option<(u32, u32)>> {
    let info = state.library()?.db.with_conn(|c| photos_core::assets::render_info(c, asset))?;
    if info.media_type == 1 {
        return Ok(None);
    }
    let (w, h) = (info.width.unwrap_or(1).max(1) as u32, info.height.unwrap_or(1).max(1) as u32);
    Ok(Some(if info.orientation >= 5 { (h, w) } else { (w, h) }))
}

/// One photo's lines, on the photo as shown (`asset_text`). Read first when it has not been
/// read with the current models (spec §2 *Scan on open*: Interactive lane, job `ocr:{id}`,
/// so it never runs twice beside the background job). Nothing while text is off.
pub fn shown_text(app: &AppHandle, asset: i64) -> AppResult<Vec<ShownTextLine>> {
    let state = app.state::<AppState>();
    let Some(key) = current_key(&state) else { return Ok(Vec::new()) };
    let Some((w, h)) = still_size(&state, asset)? else { return Ok(Vec::new()) };
    let lib = state.library()?;
    if !lib.db.with_conn(|c| ocr::scanned_with(c, asset, &key))? {
        let (a, k) = (app.clone(), key.clone());
        state.jobs.submit(&format!("ocr:{asset}"), photos_jobs::Lane::Interactive, move |_| read_and_save(&a, asset, &k)).wait();
    }
    let lines = lib.db.with_conn(|c| ocr::lines(c, asset))?;
    if lines.is_empty() {
        return Ok(Vec::new());
    }
    let recipe = lib.db.with_conn(|c| photos_core::edits::get_recipe(c, asset))?.map(|(r, _)| r);
    Ok(shown_lines(&lines, w, h, recipe.as_ref()))
}

/// The photos beside the open one, read ahead on the Prefetch lane (spec §2).
pub fn prefetch(app: &AppHandle, ids: &[i64]) {
    let state = app.state::<AppState>();
    let Some(key) = current_key(&state) else { return };
    let Ok(lib) = state.library() else { return };
    for &asset in ids {
        if !matches!(still_size(&state, asset), Ok(Some(_))) || lib.db.with_conn(|c| ocr::scanned_with(c, asset, &key)).unwrap_or(true) {
            continue;
        }
        let (a, k) = (app.clone(), key.clone());
        state.jobs.submit(&format!("ocr:{asset}"), photos_jobs::Lane::Prefetch, move |cancel| {
            if !cancel.is_cancelled() {
                read_and_save(&a, asset, &k);
            }
        });
    }
}

/// Read the text in every still not yet read with the current models. Coalesced; after the
/// pet scan (`embeddings::schedule`) and when *Read text in photos* turns on. Stops when the
/// switch goes off, the library changes or the job is cancelled; what is saved stays.
pub fn schedule(app: &AppHandle) {
    if !crate::models::read_text_on(&app.state::<AppState>()) {
        return;
    }
    let Some(((det_path, det), (rec_path, rec))) = crate::models::ocr_sources() else { return };
    let key = scan_key(&det.id, &rec.id);
    let a = app.clone();
    app.state::<AppState>().jobs.submit("maintenance:ocr", photos_jobs::Lane::Maintenance, move |cancel| {
        let state = a.state::<AppState>();
        let Ok(lib) = state.library() else { return };
        let mut engine = match OcrEngine::open((&det_path, &det), (&rec_path, &rec), THREADS, Some(photos_jobs::background_priority)) {
            Ok(e) => e,
            Err(e) => {
                tracing::debug!("text models would not open: {e}");
                return;
            }
        };
        let (started, mut read) = (Instant::now(), 0usize);
        let stop = || cancel.is_cancelled() || !crate::models::read_text_on(&state) || state.library().map_or(true, |l| l.root != lib.root);
        'scan: loop {
            if stop() {
                break;
            }
            let Ok(batch) = lib.db.with_conn(|c| ocr::unscanned(c, &key, BATCH)) else { break };
            if batch.is_empty() {
                break;
            }
            let mut scans = Vec::with_capacity(batch.len());
            for (asset, screenshot) in batch {
                while photos_jobs::user_busy(STEP_ASIDE) {
                    if stop() {
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(100));
                }
                if stop() {
                    // Save what was read; the rest waits for the next run.
                    let _ = lib.db.with_conn(|c| ocr::save_scans(c, &key, &scans));
                    read += scans.len();
                    break 'scan;
                }
                scans.push((asset, read_one(&a, &mut engine, asset, screenshot)));
            }
            if let Err(e) = lib.db.with_conn(|c| ocr::save_scans(c, &key, &scans)) {
                // The database is failing; try again later.
                tracing::warn!("text scan: {e}");
                break;
            }
            read += scans.len();
            let left = lib.db.with_conn(|c| ocr::unscanned_count(c, &key)).unwrap_or(0);
            let _ = a.emit(OCR_PROGRESS, OcrProgress { left: left as u32 });
        }
        let _ = a.emit(OCR_PROGRESS, OcrProgress { left: 0 });
        if read > 0 {
            let secs = started.elapsed().as_secs_f32().max(0.001);
            tracing::info!(photos = read, per_second = read as f32 / secs, "text read");
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn screenshots_detect_at_1536() {
        assert_eq!(det_side(true), 1536);
        assert_eq!(det_side(false), 736);
    }

    #[test]
    fn the_scan_key_names_the_detection_size() {
        // Reading at another size finds other lines, so it rescans like a new model.
        assert_eq!(scan_key("det@1", "rec@1"), "det@1+rec@1/736");
    }

    fn stored(x0: f32, y0: f32, x1: f32, y1: f32) -> ocr::StoredLine {
        ocr::StoredLine { ord: 0, text: "Gostilna".into(), conf: 0.9, quad: [x0, y0, x1, y0, x1, y1, x0, y1] }
    }

    fn close(a: &[f32], b: &[f32]) -> bool {
        a.iter().zip(b).all(|(a, b)| (a - b).abs() < 1e-4)
    }

    #[test]
    fn lines_follow_the_edit() {
        use photos_core::edits::{Crop, Recipe};
        let line = [stored(0.1, 0.1, 0.3, 0.2)];
        let plain = shown_lines(&line, 200, 100, None);
        assert!(close(&plain[0].quad, &line[0].quad), "no edit: as stored");
        assert_eq!(plain[0].text, "Gostilna");
        let turned = shown_lines(&line, 200, 100, Some(&Recipe { rotate: 90, ..Recipe::default() }));
        assert!(close(&turned[0].quad[..2], &[0.9, 0.1]), "a quarter turn clockwise: {:?}", turned[0].quad);
        let flipped = shown_lines(&line, 200, 100, Some(&Recipe { flip_h: true, ..Recipe::default() }));
        assert!(close(&flipped[0].quad[..4], &[0.9, 0.1, 0.7, 0.1]), "{:?}", flipped[0].quad);
        let right_half = Recipe { crop: Some(Crop { x: 0.5, y: 0.0, w: 0.5, h: 1.0 }), ..Recipe::default() };
        assert!(shown_lines(&line, 200, 100, Some(&right_half)).is_empty(), "cropped away");
        let left_half = Recipe { crop: Some(Crop { x: 0.0, y: 0.0, w: 0.5, h: 1.0 }), ..Recipe::default() };
        assert!(close(&shown_lines(&line, 200, 100, Some(&left_half))[0].quad[..2], &[0.2, 0.1]));
    }

    #[test]
    fn hidden_while_off() {
        assert!(!reads(false, Some("k")), "switch off: nothing is read or shown");
        assert!(!reads(true, None), "no models: nothing");
        assert!(reads(true, Some("k")));
    }

    #[test]
    fn an_unreadable_photo_is_read_once() {
        let db = photos_core::db::Db::open_in_memory().unwrap();
        db.with_conn(|c| {
            c.execute(
                "INSERT INTO assets (id, content_hash, relative_path, filename, extension, media_type, byte_size, orientation, imported_at)
                 VALUES (1, 'h', 'a.jpg', 'a.jpg', 'jpg', 0, 1, 1, 1)",
                [],
            )?;
            // What a scan on open saves for a photo that would not decode: no lines.
            ocr::save_scans(c, "k", &[(1, Vec::new())])?;
            assert!(ocr::scanned_with(c, 1, "k")?, "the next open reads storage, not the photo");
            assert!(ocr::lines(c, 1)?.is_empty());
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn quads_are_stored_relative() {
        let line = TextLine { quad: [[150.0, 75.0], [1500.0, 75.0], [1500.0, 150.0], [150.0, 150.0]], text: "Gostilna".into(), conf: 0.9 };
        let got = to_new(&[line], 1536, 768);
        assert_eq!(got.len(), 1);
        let want = [150.0 / 1536.0, 75.0 / 768.0, 1500.0 / 1536.0, 75.0 / 768.0, 1500.0 / 1536.0, 150.0 / 768.0, 150.0 / 1536.0, 150.0 / 768.0];
        for (g, w) in got[0].quad.iter().zip(want) {
            assert!((g - w).abs() < 1e-6, "{:?}", got[0].quad);
        }
        assert_eq!((got[0].text.as_str(), got[0].conf), ("Gostilna", 0.9));
    }

    /// Plan Task 10: the scan's loop body on a **copy** of a real `library.db`, reading only the
    /// previews already in the library's cache (read-only; a photo without one is skipped, so
    /// nothing is written into the library), saving into the copy. Prints speed, the share of
    /// photos with text, sample lines and a few searches; checks 0031 re-runs as a no-op.
    /// `LUMEN_DB=<copy> LUMEN_LIBRARY=<library> LUMEN_DEV_MODELS=<dev folder> [LUMEN_OCR_LIMIT=500]
    /// [LUMEN_OCR_SEARCHES=illy,union,menu,exit,2575] cargo test --release -p lumen --lib backfill_on_a_real_library -- --ignored --nocapture`
    #[test]
    #[ignore = "needs a copy of a real library.db, the library and the text models"]
    fn backfill_on_a_real_library() {
        use photos_media::store::DerivativeStore;
        let env = |k: &str| std::env::var(k).unwrap_or_else(|_| panic!("{k}"));
        let copy = std::path::PathBuf::from(env("LUMEN_DB"));
        let library = std::path::PathBuf::from(env("LUMEN_LIBRARY"));
        assert!(!copy.starts_with(&library), "LUMEN_DB must be a copy outside the library");
        let limit: usize = std::env::var("LUMEN_OCR_LIMIT").ok().and_then(|v| v.parse().ok()).unwrap_or(500);
        let db = photos_core::db::Db::open(&copy).unwrap();
        let ((det_path, det), (rec_path, rec)) = crate::models::ocr_sources().expect("both text models in LUMEN_DEV_MODELS");
        let key = scan_key(&det.id, &rec.id);
        let mut engine = OcrEngine::open((&det_path, &det), (&rec_path, &rec), THREADS, None).unwrap();
        let store = DerivativeStore::new(&library);

        let batch = db.with_conn(|c| ocr::unscanned(c, &key, limit)).unwrap();
        let (started, mut read, mut with_text, mut skipped) = (Instant::now(), 0usize, 0usize, 0usize);
        let mut samples: Vec<String> = Vec::new();
        for chunk in batch.chunks(BATCH) {
            let mut scans = Vec::new();
            for &(asset, screenshot) in chunk {
                let info = db.with_conn(|c| photos_core::assets::render_info(c, asset)).unwrap();
                let Ok(img) = image::open(store.preview_path(&info.content_hash)) else {
                    skipped += 1;
                    continue;
                };
                let (rgb, w, h) = scan_pixels(&img);
                let lines = to_new(&engine.read(&rgb, w, h, det_side(screenshot)).unwrap(), w, h);
                let kept: Vec<&NewTextLine> = lines.iter().filter(|l| l.conf >= ocr::MIN_CONF && l.text.trim().chars().count() >= 2).collect();
                if !kept.is_empty() {
                    with_text += 1;
                    if samples.len() < 20 {
                        samples.push(format!("#{asset}: {}", kept.iter().map(|l| format!("{:?} {:.2}", l.text, l.conf)).collect::<Vec<_>>().join(" · ")));
                    }
                }
                scans.push((asset, lines));
            }
            read += scans.len();
            db.with_conn(|c| ocr::save_scans(c, &key, &scans)).unwrap();
        }
        let secs = started.elapsed().as_secs_f64().max(0.001);
        println!("{read} photos read in {secs:.1} s ({:.2} photos/s), {skipped} skipped (no cached preview)", read as f64 / secs);
        println!("with text: {with_text} ({:.1} %)", 100.0 * with_text as f64 / read.max(1) as f64);
        for s in &samples {
            println!("  {s}");
        }
        let searches = std::env::var("LUMEN_OCR_SEARCHES").unwrap_or_else(|_| "illy,union,menu,exit,2575".into());
        for word in searches.split(',') {
            let hits: i64 = db
                .with_conn(|c| Ok(c.query_row("SELECT COUNT(*) FROM assets_fts WHERE assets_fts MATCH ?1", [format!("photo_text : \"{}\"*", word.trim())], |r| r.get(0))?))
                .unwrap();
            println!("  search {word:?}: {hits} photos");
        }

        let counts = |db: &photos_core::db::Db| {
            db.with_conn(|c| {
                let n = |t: &str| c.query_row(&format!("SELECT COUNT(*) FROM {t}"), [], |r| r.get::<_, i64>(0));
                Ok((n("ocr_scan")?, n("asset_text")?, n("assets_fts")?))
            })
            .unwrap()
        };
        let before = counts(&db);
        db.with_conn(|c| Ok(c.execute_batch(include_str!("../crates/photos-core/migrations/0031_ocr.sql"))?)).unwrap();
        assert_eq!(counts(&db), before, "0031 a second time keeps every row");
        // 0031 refills the search rows without photo text (a fresh library has none); put it back
        // so the copy stays searchable for another run.
        db.with_conn(photos_core::fts::rebuild).unwrap();
    }

    #[test]
    fn quads_stay_inside_the_photo() {
        let line = TextLine { quad: [[-3.0, -1.0], [1540.0, 0.0], [1540.0, 770.0], [0.0, 770.0]], text: "EXIT".into(), conf: 0.8 };
        assert!(to_new(&[line], 1536, 768)[0].quad.iter().all(|v| (0.0..=1.0).contains(v)));
    }
}
