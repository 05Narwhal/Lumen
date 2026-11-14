//! Lumen Facet in the app (Phase 8f P3): one run per photo on the base Develop renders,
//! display-oriented and before any edit; the rasters go to `ai_masks` and the panel gets their
//! kinds, instances and hashes. The bytes reach the overlay through `photos://ai-mask/`.

use std::path::PathBuf;

use photos_core::ai_masks::{self, AiMaskInfo, NewAiMask};
use photos_core::db::Db;
use photos_ml::manifest::{manifest, ModelEntry};
use serde::Serialize;
use ts_rs::TS;

use crate::ai_task::{AiTask, Emit};
use crate::develop_auto::cancelled;
use crate::develop_preview::{base_edge, load_preview_base, source, PREVIEW_EDGE};
use crate::error::{AppError, AppResult};
use crate::models::DevModel;
use crate::state::AppState;

/// What the Masks panel offers on a photo.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AiMaskList {
    /// The switch is on and the model is here: the AI items are shown.
    pub available: bool,
    /// The current model has looked at this photo; `masks` is what it found (maybe nothing).
    pub scanned: bool,
    pub masks: Vec<AiMaskInfo>,
}

impl AiMaskList {
    fn hidden() -> Self {
        Self { available: false, scanned: false, masks: Vec::new() }
    }
}

/// Smart features and *AI masks* on the Facet row are both on.
pub fn masks_on(state: &AppState) -> bool {
    state.settings.read().map(|s| s.smart.enabled && s.smart.experimental.ai_masks).unwrap_or(false)
}

/// Where the model is: the dev folder first (`segmentation.json`), then the models store.
pub fn model_source(state: &AppState) -> Option<(PathBuf, ModelEntry)> {
    if let DevModel::Ready { path, entry } = crate::models::dev_model("segmentation") {
        return Some((path, *entry));
    }
    let entry = manifest().by_role("segmentation")?;
    let store = crate::models::store_in(state);
    store.is_ready(entry).then(|| (store.path_of(entry), entry.clone()))
}

/// The stored list when `model` has looked at the photo already (and `again` isn't asked);
/// otherwise `run`'s masks, stored as the photo's current detection. The connection is held
/// only to read and to store, never while the model runs.
pub(crate) fn detect_into(db: &Db, asset: i64, model: &str, again: bool, run: impl FnOnce() -> AppResult<Vec<NewAiMask>>) -> AppResult<AiMaskList> {
    if again || db.with_conn(|c| ai_masks::scanned_by(c, asset))?.as_deref() != Some(model) {
        let masks = run()?;
        db.with_conn(|c| ai_masks::put_detection(c, asset, model, &masks))?;
    }
    Ok(AiMaskList { available: true, scanned: true, masks: db.with_conn(|c| ai_masks::list_for_asset(c, asset))? })
}

/// What the panel offers without running anything: the stored masks of the current model.
pub fn list(state: &AppState, asset: i64) -> AppResult<AiMaskList> {
    let Some((_, entry)) = masks_on(state).then(|| model_source(state)).flatten() else { return Ok(AiMaskList::hidden()) };
    let lib = state.library()?;
    Ok(lib.db.with_conn(|c| {
        let scanned = ai_masks::scanned_by(c, asset)?.as_deref() == Some(entry.id.as_str());
        Ok(AiMaskList { available: true, scanned, masks: if scanned { ai_masks::list_for_asset(c, asset)? } else { Vec::new() } })
    })?)
}

/// Finds the masks on a photo (`again`: even when the current model has already looked).
pub fn detect<E: Emit>(state: &AppState, asset: i64, again: bool, task: &AiTask<E>) -> AppResult<AiMaskList> {
    let Some((path, entry)) = masks_on(state).then(|| model_source(state)).flatten() else { return Ok(AiMaskList::hidden()) };
    let lib = state.library()?;
    let run = || -> AppResult<Vec<NewAiMask>> {
        task.stage("reading", 0, 0);
        let src = source(&lib, asset)?;
        // The preview's own base, uncropped: the model reads 640 px, so any held base will do.
        let edge = base_edge(if src.long > 0 { src.long } else { PREVIEW_EDGE }, None);
        let ffmpeg = state.ffmpeg();
        let base = state.develop.base_at_least(asset, edge, || load_preview_base(&src, ffmpeg.as_ref(), edge))?;
        if task.cancelled() {
            return Err(cancelled());
        }
        let rgb = base.linear.to_rgb8();
        task.stage("detecting", 0, 0);
        let found = crate::models::with_facet(state, &path, &entry, |m| photos_ml::facet::detect(m, &rgb, task.token().flag()))
            .ok_or_else(|| AppError { code: "model_unavailable".into(), message: "The masks model could not be opened.".into(), detail: None })?;
        if task.cancelled() {
            return Err(cancelled());
        }
        let segments = found.map_err(|e| AppError { code: "ml_failed".into(), message: "The masks could not be found.".into(), detail: Some(e.to_string()) })?;
        Ok(segments
            .into_iter()
            .map(|s| NewAiMask { kind: s.kind, instance: s.instance, score: s.score, width: s.width, height: s.height, weights: s.weights })
            .collect())
    };
    detect_into(&lib.db, asset, &entry.id, again, run)
}

/// A raster for the overlay: width and height as u32 little-endian, then `width × height`
/// weight bytes; `None` when this photo has no such raster. The size travels with the bytes
/// because a shape can name an older detection's raster, which `list` no longer shows.
pub fn raster_bytes(db: &Db, asset: i64, hash: &str) -> Option<Vec<u8>> {
    let r = db.with_conn(|c| ai_masks::get(c, asset, hash)).ok().flatten()?;
    let mut out = Vec::with_capacity(8 + r.weights.len());
    out.extend_from_slice(&r.width.to_le_bytes());
    out.extend_from_slice(&r.height.to_le_bytes());
    out.extend_from_slice(&r.weights);
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use photos_core::ai_masks::AiMaskKind;

    fn conn() -> Db {
        let db = Db::open_in_memory().unwrap();
        db.with_conn(|c| {
            c.execute(
                "INSERT INTO assets (id, content_hash, relative_path, filename, extension, media_type, byte_size, orientation, imported_at)
                 VALUES (1, 'h1', 'h1', 'h1', 'jpg', 0, 1, 1, 1)",
                [],
            )?;
            Ok(())
        })
        .unwrap();
        db
    }

    fn sky() -> NewAiMask {
        NewAiMask { kind: AiMaskKind::Sky, instance: 0, score: 0.98, width: 3, height: 2, weights: vec![0, 50, 100, 150, 200, 255] }
    }

    #[test]
    fn a_second_call_answers_from_the_store() {
        let c = conn();
        let mut runs = 0;
        let first = detect_into(&c, 1, "facet@1", false, || {
            runs += 1;
            Ok(vec![sky()])
        })
        .unwrap();
        assert_eq!(first.masks.len(), 1);
        assert!(first.available && first.scanned);
        let second = detect_into(&c, 1, "facet@1", false, || panic!("the store should answer")).unwrap();
        assert_eq!(second, first);
        assert_eq!(runs, 1);
    }

    #[test]
    fn again_or_a_new_model_runs_once_more() {
        let c = conn();
        detect_into(&c, 1, "facet@1", false, || Ok(vec![sky()])).unwrap();
        let mut runs = 0;
        detect_into(&c, 1, "facet@1", true, || {
            runs += 1;
            Ok(vec![sky()])
        })
        .unwrap();
        let none = detect_into(&c, 1, "facet@2", false, || {
            runs += 1;
            Ok(vec![])
        })
        .unwrap();
        assert_eq!(runs, 2);
        // The new model found nothing: nothing is offered, and it isn't run a third time.
        assert!(none.scanned && none.masks.is_empty());
        detect_into(&c, 1, "facet@2", false, || panic!("nothing to mask is remembered too")).unwrap();
    }

    #[test]
    fn a_failed_run_stores_nothing() {
        let c = conn();
        assert!(detect_into(&c, 1, "facet@1", false, || Err(cancelled())).is_err());
        assert_eq!(c.with_conn(|c| ai_masks::scanned_by(c, 1)).unwrap(), None);
    }

    #[test]
    fn the_overlay_gets_the_raw_weights_by_hash() {
        let c = conn();
        let list = detect_into(&c, 1, "facet@1", false, || Ok(vec![sky()])).unwrap();
        let bytes = |asset: i64, hash: &str| raster_bytes(&c, asset, hash);
        // Width and height lead, u32 little-endian: a shape can name an older raster the list
        // no longer shows, and the overlay still has to know its size.
        let mut want = vec![3, 0, 0, 0, 2, 0, 0, 0];
        want.extend(sky().weights);
        assert_eq!(bytes(1, &list.masks[0].hash), Some(want));
        assert_eq!(bytes(1, "0123456789abcdef0123456789abcdef"), None);
        assert_eq!(bytes(2, &list.masks[0].hash), None);
    }
}
