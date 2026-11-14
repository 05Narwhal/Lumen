use photos_core::album_collections::{self, AlbumCollection, CollectionUndo};
use photos_core::albums::{self, AlbumSummary, CoverStyle};
use photos_core::query::AssetQuery;
use photos_core::now_unix;
use tauri::{AppHandle, State};

use crate::error::AppResult;
use crate::state::AppState;

#[tauri::command(async)]
pub fn list_albums(state: State<'_, AppState>) -> AppResult<Vec<AlbumSummary>> {
    let lib = state.library()?;
    Ok(lib.db.with_conn(albums::list_albums)?)
}

#[tauri::command(async)]
pub fn get_album(state: State<'_, AppState>, id: i64) -> AppResult<AlbumSummary> {
    let lib = state.library()?;
    Ok(lib.db.with_conn(|c| albums::get_album(c, id))?)
}

#[tauri::command(async)]
pub fn create_album(app: AppHandle, state: State<'_, AppState>, name: String, parent_id: Option<i64>) -> AppResult<AlbumSummary> {
    let lib = state.library()?;
    let id = lib.db.with_conn(|c| albums::create_album(c, &name, parent_id, now_unix()))?;
    state.changes.changed(&app, true);
    Ok(lib.db.with_conn(|c| albums::get_album(c, id))?)
}

#[tauri::command(async)]
pub fn rename_album(app: AppHandle, state: State<'_, AppState>, id: i64, name: String) -> AppResult<()> {
    let lib = state.library()?;
    lib.db.with_conn(|c| albums::rename_album(c, id, &name, now_unix()))?;
    state.changes.changed(&app, true);
    Ok(())
}

/// Whether an album hides its photos from the rest of the library (round 3).
#[tauri::command(async)]
pub fn set_album_hides(app: AppHandle, state: State<'_, AppState>, id: i64, hides: bool) -> AppResult<()> {
    let lib = state.library()?;
    lib.db.with_conn(|c| photos_core::concealed::set_album_hides(c, id, hides, now_unix()))?;
    // Photos leave or rejoin every grid and count (the new library version drops the
    // layout cache) and every memory.
    state.changes.changed(&app, true);
    crate::maintenance::schedule_memories(&app);
    Ok(())
}

/// *Duplicate album* (Round 4G): the same photos in the same order, or the same rules.
#[tauri::command(async)]
pub fn duplicate_album(app: AppHandle, state: State<'_, AppState>, id: i64) -> AppResult<AlbumSummary> {
    let lib = state.library()?;
    let copy = lib.db.with_conn(|c| albums::duplicate_album(c, id, now_unix()))?;
    state.changes.changed(&app, true);
    Ok(lib.db.with_conn(|c| albums::get_album(c, copy))?)
}

#[tauri::command(async)]
pub fn delete_album(app: AppHandle, state: State<'_, AppState>, id: i64) -> AppResult<()> {
    let lib = state.library()?;
    lib.db.with_conn(|c| albums::delete_album(c, id))?;
    state.changes.changed(&app, true);
    Ok(())
}

#[tauri::command(async)]
pub fn add_to_album(app: AppHandle, state: State<'_, AppState>, album_id: i64, asset_ids: Vec<i64>) -> AppResult<usize> {
    let lib = state.library()?;
    let n = lib.db.with_conn(|c| albums::add_to_album(c, album_id, &asset_ids, now_unix()))?;
    state.changes.changed(&app, true);
    Ok(n)
}

#[tauri::command(async)]
pub fn remove_from_album(app: AppHandle, state: State<'_, AppState>, album_id: i64, asset_ids: Vec<i64>) -> AppResult<usize> {
    let lib = state.library()?;
    let n = lib.db.with_conn(|c| albums::remove_from_album(c, album_id, &asset_ids))?;
    state.changes.changed(&app, true);
    Ok(n)
}

#[tauri::command(async)]
pub fn move_in_album(app: AppHandle, state: State<'_, AppState>, album_id: i64, asset_id: i64, before: Option<i64>) -> AppResult<()> {
    let lib = state.library()?;
    lib.db.with_conn(|c| albums::move_in_album(c, album_id, asset_id, before))?;
    state.changes.changed(&app, true);
    Ok(())
}

#[tauri::command(async)]
pub fn set_album_cover(app: AppHandle, state: State<'_, AppState>, album_id: i64, asset_id: Option<i64>) -> AppResult<()> {
    let lib = state.library()?;
    lib.db.with_conn(|c| albums::set_cover(c, album_id, asset_id))?;
    state.changes.changed(&app, true);
    Ok(())
}

/// New automatic prints for an album's cover (*Shuffle cover*); a hand-picked cover is forgotten.
#[tauri::command(async)]
pub fn shuffle_album_cover(app: AppHandle, state: State<'_, AppState>, id: i64) -> AppResult<()> {
    state.library()?.db.with_conn(|c| albums::shuffle_cover(c, id))?;
    state.changes.changed(&app, true);
    Ok(())
}

/// An icon cover for an album (Julian's 2026-11-08 list, item 3), or `None` for its photos.
#[tauri::command(async)]
pub fn set_album_cover_style(app: AppHandle, state: State<'_, AppState>, album_id: i64, style: Option<CoverStyle>) -> AppResult<()> {
    state.library()?.db.with_conn(|c| albums::set_cover_style(c, album_id, style.as_ref()))?;
    state.changes.changed(&app, true);
    Ok(())
}

/// An icon cover for a collection, or `None` for its albums' covers.
#[tauri::command(async)]
pub fn set_collection_cover_style(app: AppHandle, state: State<'_, AppState>, id: i64, style: Option<CoverStyle>) -> AppResult<()> {
    state.library()?.db.with_conn(|c| album_collections::set_cover_style(c, id, style.as_ref()))?;
    state.changes.changed(&app, true);
    Ok(())
}

/// Create a smart album from a query. Its contents follow the library from then on.
#[tauri::command(async)]
pub fn create_smart_album(app: AppHandle, state: State<'_, AppState>, name: String, query: AssetQuery) -> AppResult<AlbumSummary> {
    let lib = state.library()?;
    let id = lib.db.with_conn(|c| albums::create_smart_album(c, &name, &query, now_unix()))?;
    state.changes.changed(&app, true);
    Ok(lib.db.with_conn(|c| albums::get_album(c, id))?)
}

#[tauri::command(async)]
pub fn update_smart_album(app: AppHandle, state: State<'_, AppState>, id: i64, name: String, query: AssetQuery) -> AppResult<()> {
    let lib = state.library()?;
    lib.db.with_conn(|c| albums::update_smart_album(c, id, &name, &query, now_unix()))?;
    state.changes.changed(&app, true);
    Ok(())
}

// Collections (Round 5A): folders of albums, `photos_core::album_collections`.

#[tauri::command(async)]
pub fn list_album_collections(state: State<'_, AppState>) -> AppResult<Vec<AlbumCollection>> {
    Ok(state.library()?.db.with_conn(album_collections::list)?)
}

#[tauri::command(async)]
pub fn create_album_collection(app: AppHandle, state: State<'_, AppState>, name: String, parent_id: Option<i64>) -> AppResult<i64> {
    let id = state.library()?.db.with_conn(|c| album_collections::create(c, &name, parent_id, now_unix()))?;
    state.changes.changed(&app, true);
    Ok(id)
}

#[tauri::command(async)]
pub fn rename_album_collection(app: AppHandle, state: State<'_, AppState>, id: i64, name: String) -> AppResult<()> {
    state.library()?.db.with_conn(|c| album_collections::rename(c, id, &name, now_unix()))?;
    state.changes.changed(&app, true);
    Ok(())
}

/// Puts an album in a collection (or none). Returns where it was, for Undo.
#[tauri::command(async)]
pub fn move_album_to_collection(app: AppHandle, state: State<'_, AppState>, album_id: i64, collection_id: Option<i64>) -> AppResult<Option<i64>> {
    let was = state.library()?.db.with_conn(|c| album_collections::move_album(c, album_id, collection_id, now_unix()))?;
    state.changes.changed(&app, true);
    Ok(was)
}

/// Puts a collection inside another (or at the top). Returns where it was, for Undo.
#[tauri::command(async)]
pub fn move_album_collection(app: AppHandle, state: State<'_, AppState>, id: i64, into: Option<i64>) -> AppResult<Option<i64>> {
    let was = state.library()?.db.with_conn(|c| album_collections::move_collection(c, id, into, now_unix()))?;
    state.changes.changed(&app, true);
    Ok(was)
}

/// Deletes a collection; its albums and collections move up. Returns what Undo needs.
#[tauri::command(async)]
pub fn delete_album_collection(app: AppHandle, state: State<'_, AppState>, id: i64) -> AppResult<CollectionUndo> {
    let undo = state.library()?.db.with_conn(|c| album_collections::delete(c, id))?;
    state.changes.changed(&app, true);
    Ok(undo)
}

#[tauri::command(async)]
pub fn restore_album_collection(app: AppHandle, state: State<'_, AppState>, undo: CollectionUndo) -> AppResult<()> {
    state.library()?.db.with_conn(|c| album_collections::restore(c, &undo))?;
    state.changes.changed(&app, true);
    Ok(())
}
