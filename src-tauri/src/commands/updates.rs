//! Settings → Updates (Round 17): thin adapters over `crate::updater`.

use tauri::AppHandle;

use crate::error::AppResult;
use crate::updater::{self, UpdateReceipt, UpdateStatus};

#[tauri::command(async)]
pub fn update_status(app: AppHandle) -> UpdateStatus {
    updater::status(&app)
}

/// *Check for updates*. With automatic updates on, a build found is downloaded; off, it is offered.
#[tauri::command]
pub async fn check_for_updates(app: AppHandle) -> AppResult<UpdateStatus> {
    updater::check(&app, true).await;
    Ok(updater::status(&app))
}

/// *Download*: the build the last check offered. Runs in the background; the state follows.
#[tauri::command(async)]
pub fn download_update(app: AppHandle) -> UpdateStatus {
    updater::download_offered(&app);
    updater::status(&app)
}

/// *Install now*: close as usual (Develop's flush and the import question still run), install, and
/// open the new version.
#[tauri::command(async)]
pub fn install_update_now(app: AppHandle) -> AppResult<()> {
    updater::install_now(&app)
}

/// The notes of the version just installed, once per launch.
#[tauri::command(async)]
pub fn take_update_receipt(app: AppHandle) -> Option<UpdateReceipt> {
    updater::take_receipt(&app)
}
