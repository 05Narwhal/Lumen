//! Smart features: what the models are doing, for Settings.

use photos_ml::manifest::manifest;
use serde::Serialize;
use tauri::{AppHandle, State};
use ts_rs::TS;

use crate::error::AppResult;
use crate::state::AppState;

#[derive(Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ModelStatus {
    pub id: String,
    pub role: String,
    #[ts(type = "number")]
    pub bytes: u64,
    pub ready: bool,
}

#[derive(Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct MlStatus {
    pub enabled: bool,
    pub models: Vec<ModelStatus>,
    /// Photos with an embedding, of all the photos that can have one.
    #[ts(type = "number")]
    pub embedded: i64,
    #[ts(type = "number")]
    pub photos: i64,
    /// Faces found, and the people they were grouped into (Phase 7b).
    #[ts(type = "number")]
    pub faces: i64,
    #[ts(type = "number")]
    pub people: i64,
}

#[tauri::command(async)]
pub fn get_ml_status(app: AppHandle, state: State<'_, AppState>) -> AppResult<MlStatus> {
    let store = crate::models::store(&app);
    let lib = state.library()?;
    let (embedded, photos) = lib.db.with_conn(|c| {
        Ok(c.query_row(
            &format!(
                "SELECT (SELECT COUNT(*) FROM asset_embeddings), COUNT(*) FROM assets a
                 WHERE a.state = 0 AND a.media_type != 1 AND NOT {}",
                photos_core::assets::MOVIE_HALF
            ),
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?)
    })?;
    let (faces, people) = lib.db.with_conn(photos_core::people::counts)?;
    let experimental = state.settings.read().map(|s| s.smart.experimental.clone()).unwrap_or_default();
    Ok(MlStatus {
        faces,
        people,
        enabled: crate::models::enabled(&app),
        models: manifest()
            .models
            .iter()
            // A switched-off Experimental model is not missing (Phase 8f).
            .filter(|m| crate::models::wanted(m, &experimental))
            .map(|m| ModelStatus { id: m.id.clone(), role: m.role.clone(), bytes: m.bytes, ready: store.is_ready(m) })
            .collect(),
        embedded,
        photos,
    })
}

/// Settings turned smart features back on: fetch now rather than at the next launch, and
/// embed whatever the models that are already here can.
#[tauri::command(async)]
pub fn fetch_models(app: AppHandle) {
    crate::models::fetch_all(&app);
    crate::embeddings::schedule(&app);
}
