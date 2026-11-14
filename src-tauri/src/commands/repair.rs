//! Import → Repair (Round 21 §6): thin adapters over `crate::repair`.

use photos_import::repair::{RepairClass, RepairSelection, RepairSource, RepairSourceRef, RepairStatus};
use tauri::{AppHandle, State};

use crate::error::AppResult;
use crate::repair::{self, RepairItemsPage};
use crate::state::AppState;

/// Volumes, iPhones and Apple-formatted disks to offer as a source.
#[tauri::command(async)]
pub fn list_repair_sources() -> AppResult<Vec<RepairSource>> {
    Ok(repair::list_sources())
}

/// Judges `source` against the library. Reads only. With `only_assets` it looks only at those photos.
#[tauri::command(async)]
pub fn start_repair_check(app: AppHandle, source: RepairSourceRef, only_assets: Option<Vec<i64>>) -> AppResult<()> {
    repair::start(&app, source, only_assets)
}

#[tauri::command(async)]
pub fn get_repair_status() -> AppResult<RepairStatus> {
    Ok(repair::status())
}

#[tauri::command(async)]
pub fn get_repair_items(state: State<'_, AppState>, status: RepairClass, after: Option<i64>, limit: u32) -> AppResult<RepairItemsPage> {
    repair::items(&state, status, after, limit)
}

/// Brings in the ticked items of the last check: new files, sidecars, damaged originals swapped.
/// The outcome arrives as `repair://imported`.
#[tauri::command(async)]
pub fn start_repair_import(app: AppHandle, selection: RepairSelection) -> AppResult<()> {
    repair::start_import(&app, selection)
}

/// How each item of the last re-import ended (Round 22 §3 rule 6).
#[tauri::command(async)]
pub fn get_repair_outcomes() -> AppResult<Vec<photos_import::repair::RepairOutcomeRow>> {
    Ok(repair::outcomes())
}

/// The health card: what the library knows about its own files without a source.
#[tauri::command(async)]
pub fn get_library_health(state: State<'_, AppState>) -> AppResult<crate::library_health::LibraryHealth> {
    let lib = state.library()?;
    Ok(crate::library_health::health(&lib)?)
}

/// *Recover what's readable* (Round 22 §4) for `ids`, or every flagged photo not in Trash.
/// Ends with `repair://recovered`.
#[tauri::command(async)]
pub fn start_recover(app: AppHandle, ids: Option<Vec<i64>>) -> AppResult<()> {
    repair::start_recover(&app, ids)
}

/// The staged recoveries, waiting for *Keep* or *Discard*.
#[tauri::command(async)]
pub fn get_recovered(state: State<'_, AppState>) -> AppResult<Vec<repair::RecoveredRow>> {
    repair::recovered_rows(&*state.library()?)
}

#[tauri::command(async)]
pub fn discard_recovered(state: State<'_, AppState>, ids: Vec<i64>) -> AppResult<()> {
    repair::discard(&*state.library()?, &ids);
    Ok(())
}

/// Each staged recovery replaces its damaged original (kept aside). Undone by `undo_keep_recovered`.
#[tauri::command(async)]
pub fn keep_recovered(app: AppHandle, ids: Vec<i64>) -> AppResult<repair::RecoveredChanges> {
    repair::keep_and_refresh(&app, &ids)
}

#[tauri::command(async)]
pub fn undo_keep_recovered(app: AppHandle, ids: Vec<i64>) -> AppResult<repair::RecoveredChanges> {
    repair::undo_keep_and_refresh(&app, &ids)
}

#[tauri::command(async)]
pub fn cancel_repair(state: State<'_, AppState>) -> AppResult<()> {
    repair::cancel(&state)
}
