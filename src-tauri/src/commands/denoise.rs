//! Denoise (Phase 8d): thin adapters over `crate::denoise`.

use tauri::AppHandle;

use crate::denoise::{self, DenoiseStatus};
use crate::error::AppResult;

#[tauri::command(async)]
pub fn denoise_status(app: AppHandle, asset_id: i64) -> AppResult<DenoiseStatus> {
    denoise::status(&app, asset_id)
}

/// Builds the photo's denoised base in the background (`denoise://progress`, `denoise://finished`).
#[tauri::command(async)]
pub fn start_denoise(app: AppHandle, asset_id: i64) -> AppResult<()> {
    denoise::start(&app, asset_id)
}

#[tauri::command(async)]
pub fn cancel_denoise(app: AppHandle, asset_id: i64) {
    denoise::cancel(&app, asset_id);
}
