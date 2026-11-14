//! Text in photos (Round 13-3): one photo's lines for the viewer, and its neighbours read ahead.

use tauri::AppHandle;

use crate::error::AppResult;
use crate::ocr::ShownTextLine;

/// The lines read in a photo, on the photo as shown; read first when it has not been yet.
/// Empty while *Text in photos* is off, for a video, and for a photo without text.
#[tauri::command(async)]
pub fn asset_text(app: AppHandle, id: i64) -> AppResult<Vec<ShownTextLine>> {
    crate::ocr::shown_text(&app, id)
}

/// Reads the open photo's neighbours ahead, on the Prefetch lane.
#[tauri::command(async)]
pub fn prefetch_text(app: AppHandle, ids: Vec<i64>) -> AppResult<()> {
    crate::ocr::prefetch(&app, &ids);
    Ok(())
}
