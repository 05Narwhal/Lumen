//! The Hidden folder (round 3): thin adapters over `photos_core::vault` and the lock in
//! `crate::vault`.

use photos_core::{now_unix, vault};
use serde::Serialize;
use tauri::{AppHandle, State};
use ts_rs::TS;

use crate::error::{AppError, AppResult};
use crate::state::AppState;

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct VaultInfo {
    /// A password has been chosen.
    pub has_password: bool,
    #[ts(type = "number")]
    pub count: i64,
    pub unlocked: bool,
}

fn locked() -> AppError {
    AppError { code: "vault_locked".into(), message: "The Hidden folder is locked.".into(), detail: None }
}

#[tauri::command(async)]
pub fn vault_status(state: State<'_, AppState>) -> AppResult<VaultInfo> {
    let s = state.library()?.db.with_conn(vault::status)?;
    Ok(VaultInfo { has_password: s.has_password, count: s.count, unlocked: state.vault_lock.is_unlocked() })
}

/// The first password, or a new one while the folder is open. Opens it.
#[tauri::command(async)]
pub fn vault_set_password(state: State<'_, AppState>, password: String) -> AppResult<()> {
    let lib = state.library()?;
    if lib.db.with_conn(vault::status)?.has_password && !state.vault_lock.is_unlocked() {
        return Err(locked());
    }
    lib.db.with_conn(|c| vault::set_password(c, &password, now_unix()))?;
    state.vault_lock.unlock();
    Ok(())
}

/// Opens the folder when the password is right. Hashing takes a moment, so off the main thread.
#[tauri::command(async)]
pub fn vault_unlock(state: State<'_, AppState>, password: String) -> AppResult<bool> {
    let ok = state.library()?.db.with_conn(|c| vault::verify(c, &password))?;
    if ok {
        state.vault_lock.unlock();
    }
    Ok(ok)
}

#[tauri::command(async)]
pub fn vault_lock(state: State<'_, AppState>) {
    state.vault_lock.lock();
}

/// The forgotten-password path. The frontend warns and makes the user wait first; the
/// photos stay where they are.
#[tauri::command(async)]
pub fn vault_reset_password(state: State<'_, AppState>, password: String) -> AppResult<()> {
    state.library()?.db.with_conn(|c| vault::reset_password(c, &password, now_unix()))?;
    state.vault_lock.unlock();
    Ok(())
}

/// Moves photos into the folder; returns every id moved (Live Photo halves included), for Undo.
#[tauri::command(async)]
pub fn move_to_vault(app: AppHandle, state: State<'_, AppState>, ids: Vec<i64>) -> AppResult<Vec<i64>> {
    let moved = state.library()?.db.with_conn(|c| vault::move_in(c, &ids))?;
    state.vault_lock.remember_moved(&moved);
    state.changes.changed(&app, true);
    crate::maintenance::schedule_memories(&app);
    Ok(moved)
}

/// Moves photos out of the folder, which must be open — unless this is the Undo of the move
/// that just put them there.
#[tauri::command(async)]
pub fn move_from_vault(app: AppHandle, state: State<'_, AppState>, ids: Vec<i64>) -> AppResult<Vec<i64>> {
    if !state.vault_lock.is_unlocked() && !state.vault_lock.may_undo(&ids) {
        return Err(locked());
    }
    let moved = state.library()?.db.with_conn(|c| vault::move_out(c, &ids))?;
    state.changes.changed(&app, true);
    crate::maintenance::schedule_memories(&app);
    Ok(moved)
}
