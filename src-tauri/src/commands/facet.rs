//! AI masks (Phase 8f): what Lumen Facet found on a photo, as kinds, instances and hashes only.
//! The rasters' bytes go through `photos://ai-mask/<asset>/<hash>`, never `invoke`.

use tauri::{AppHandle, State};

use crate::error::AppResult;
use crate::facet::AiMaskList;
use crate::state::AppState;

/// The masks already found on a photo; runs nothing. `available` false: the items are hidden.
#[tauri::command(async)]
pub fn ai_masks_list(state: State<'_, AppState>, asset_id: i64) -> AppResult<AiMaskList> {
    crate::facet::list(&state, asset_id)
}

/// Finds the masks on a photo on the interactive lane, or answers from the store when the
/// current model has looked already; `again` (*Detect again*) runs it anyway.
#[tauri::command(async)]
pub fn ai_masks_detect(app: AppHandle, state: State<'_, AppState>, asset_id: i64, again: bool, task: Option<String>) -> AppResult<AiMaskList> {
    let task = crate::ai_task::AiTask::start(app, task, "masks");
    let out = photos_jobs::interactive(|| crate::facet::detect(&state, asset_id, again, &task));
    task.finish(out.is_ok());
    out
}
