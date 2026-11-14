//! Backup (Round 5A): thin adapters over `crate::backup`.

use photos_core::backup::Schedule;
use tauri::{AppHandle, State};

use crate::backup::{self, BackupStatus};
use crate::error::AppResult;
use crate::state::AppState;

#[tauri::command(async)]
pub fn get_backup(state: State<'_, AppState>) -> AppResult<BackupStatus> {
    backup::status(&state)
}

/// Where to back up the open library; `None` stops backing it up (the backup stays).
#[tauri::command(async)]
pub fn set_backup_destination(state: State<'_, AppState>, destination: Option<String>) -> AppResult<BackupStatus> {
    backup::set_destination(&state, destination)
}

#[tauri::command(async)]
pub fn set_backup_schedule(state: State<'_, AppState>, schedule: Schedule) -> AppResult<BackupStatus> {
    backup::set_schedule(&state, schedule)
}

#[tauri::command(async)]
pub fn start_backup(app: AppHandle) -> AppResult<()> {
    backup::start(&app)
}

#[tauri::command(async)]
pub fn cancel_backup(state: State<'_, AppState>) -> AppResult<()> {
    backup::cancel(&state)
}
