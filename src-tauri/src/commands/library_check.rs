//! Check before deleting (Round 19 C4): thin adapters over `crate::library_check`.

use photos_import::check::CheckVerdict;
use tauri::{AppHandle, State};

use crate::error::AppResult;
use crate::library_check::{self, CheckItemsPage, CheckKind, CheckStatus, CheckProblemsPage};
use crate::state::AppState;

#[tauri::command(async)]
pub fn get_check_status(state: State<'_, AppState>) -> AppResult<CheckStatus> {
    library_check::status(&state)
}

/// Looks at every file under `folder` (a folder or a card). Reads only.
#[tauri::command(async)]
pub fn start_check_folder(app: AppHandle, folder: String) -> AppResult<()> {
    library_check::start(&app, Some(folder))
}

/// Re-hashes every original and sidecar of the open library. Reads only.
#[tauri::command(async)]
pub fn start_check_library(app: AppHandle) -> AppResult<()> {
    library_check::start(&app, None)
}

#[tauri::command(async)]
pub fn cancel_check(state: State<'_, AppState>) -> AppResult<()> {
    library_check::cancel(&state)
}

#[tauri::command(async)]
pub fn get_check_items(state: State<'_, AppState>, status: CheckVerdict, after: Option<u32>, limit: u32) -> AppResult<CheckItemsPage> {
    library_check::items(&state, status, after, limit)
}

#[tauri::command(async)]
pub fn get_check_problems(state: State<'_, AppState>, after: Option<u32>, limit: u32) -> AppResult<CheckProblemsPage> {
    library_check::problems(&state, after, limit)
}

/// Writes the last result of `kind` as a text file into `folder`; returns the file's path.
#[tauri::command(async)]
pub fn save_check_report(state: State<'_, AppState>, kind: CheckKind, folder: String) -> AppResult<String> {
    library_check::save_report(&state, kind, &folder)
}
