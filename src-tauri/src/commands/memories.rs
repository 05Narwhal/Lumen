//! Memories: thin adapters over `photos_core::memories`.

use photos_core::memories::{self, MemorySummary};
use tauri::{AppHandle, State};

use crate::error::AppResult;
use crate::state::AppState;

#[tauri::command(async)]
pub fn list_memories(state: State<'_, AppState>, include_hidden: Option<bool>) -> AppResult<Vec<MemorySummary>> {
    let lib = state.library()?;
    Ok(lib.db.with_conn(|c| memories::list_memories(c, include_hidden.unwrap_or(false)))?)
}

#[tauri::command(async)]
pub fn get_memory(state: State<'_, AppState>, id: i64) -> AppResult<MemorySummary> {
    let lib = state.library()?;
    Ok(lib.db.with_conn(|c| memories::get_memory(c, id))?)
}

/// The curated set, best first, for *Play*.
#[tauri::command(async)]
pub fn memory_curated_ids(state: State<'_, AppState>, id: i64) -> AppResult<Vec<i64>> {
    let lib = state.library()?;
    Ok(lib.db.with_conn(|c| memories::curated_ids(c, id))?)
}

#[tauri::command(async)]
pub fn set_memory_hidden(app: AppHandle, state: State<'_, AppState>, id: i64, hidden: bool) -> AppResult<()> {
    let lib = state.library()?;
    lib.db.with_conn(|c| memories::set_hidden(c, id, hidden))?;
    state.changes.changed(&app, false);
    Ok(())
}

/// Queues a regeneration (curation facts first); returns at once.
#[tauri::command(async)]
pub fn regenerate_memories(app: AppHandle) -> AppResult<()> {
    crate::maintenance::schedule_memories(&app);
    Ok(())
}
