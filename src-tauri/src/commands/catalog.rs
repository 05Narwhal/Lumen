use photos_core::assets::{self, AssetDetail, LibraryStats, Neighbors};
use photos_core::batches;
use photos_core::develop_source::{self, DevelopSourceFile, RememberedStrip};
use photos_core::home::{self, HomeSummary};
use photos_core::labels;
use photos_core::layout::{count_scoped, ids_scoped, GroupBy, LayoutRow, LayoutSummary};
use photos_core::scope::LayoutScope;
use tauri::{AppHandle, State};

use crate::error::{AppError, AppResult};
use crate::state::{AppState, LayoutKey};

const MAX_ROWS_PER_CALL: u32 = 200;

#[tauri::command(async)]
pub fn get_layout(
    state: State<'_, AppState>,
    width: u32,
    row_height: u32,
    gap: u32,
    scope: Option<LayoutScope>,
    group_by: Option<GroupBy>,
) -> AppResult<LayoutSummary> {
    let key = LayoutKey { width, row_height, gap, scope: scope.unwrap_or_default(), group_by: group_by.unwrap_or_default() };
    Ok(state.layout_for(key)?.summary.clone())
}

/// The row holding `asset_id` in that layout, so a grid can open scrolled to a photo —
/// how a tile in the year view opens the month view where that photo is.
#[tauri::command(async)]
pub fn find_layout_row(
    state: State<'_, AppState>,
    width: u32,
    row_height: u32,
    gap: u32,
    scope: Option<LayoutScope>,
    group_by: Option<GroupBy>,
    asset_id: i64,
) -> AppResult<Option<u32>> {
    let key = LayoutKey { width, row_height, gap, scope: scope.unwrap_or_default(), group_by: group_by.unwrap_or_default() };
    Ok(state.layout_for(key)?.row_of(asset_id))
}

/// Previous and next of `id` in that layout, so the viewer pages through exactly what the
/// grid that opened it shows, in its order. `None` when the photo is not in it (it left
/// the album while open); the viewer then falls back to the timeline.
#[tauri::command(async)]
pub fn get_scope_neighbors(
    state: State<'_, AppState>,
    width: u32,
    row_height: u32,
    gap: u32,
    scope: Option<LayoutScope>,
    group_by: Option<GroupBy>,
    id: i64,
) -> AppResult<Option<Neighbors>> {
    let key = LayoutKey { width, row_height, gap, scope: scope.unwrap_or_default(), group_by: group_by.unwrap_or_default() };
    Ok(state.layout_for(key)?.neighbors_of(id))
}

/// [`get_scope_neighbors`] for a collection's photos by album (Round 19 §10): the photo may be
/// in two sections, so the answer names the section of the copy opened and of each neighbour.
#[tauri::command(async)]
#[allow(clippy::too_many_arguments)]
pub fn get_section_neighbors(
    state: State<'_, AppState>,
    width: u32,
    row_height: u32,
    gap: u32,
    scope: Option<LayoutScope>,
    group_by: Option<GroupBy>,
    id: i64,
    section: Option<u32>,
) -> AppResult<Option<photos_core::layout::SlotNeighbors>> {
    let key = LayoutKey { width, row_height, gap, scope: scope.unwrap_or_default(), group_by: group_by.unwrap_or_default() };
    Ok(state.layout_for(key)?.slots_around(id, section))
}

/// Shift-click: every tile between `from` and `to` as the grid draws them, however far
/// apart (Round 5). Read from the cached layout, so an album's own order and search results
/// hold. `None` when either has left the grid.
#[tauri::command(async)]
#[allow(clippy::too_many_arguments)]
pub fn layout_range_ids(
    state: State<'_, AppState>,
    width: u32,
    row_height: u32,
    gap: u32,
    scope: Option<LayoutScope>,
    group_by: Option<GroupBy>,
    from: i64,
    to: i64,
) -> AppResult<Option<Vec<i64>>> {
    let key = LayoutKey { width, row_height, gap, scope: scope.unwrap_or_default(), group_by: group_by.unwrap_or_default() };
    Ok(state.layout_for(key)?.ids_between(from, to))
}

/// Every tile of that layout, in its order: *Select all* (Mod+A) takes the whole grid, not
/// just the rows loaded so far (Round 10).
#[tauri::command(async)]
pub fn layout_ids(state: State<'_, AppState>, width: u32, row_height: u32, gap: u32, scope: Option<LayoutScope>, group_by: Option<GroupBy>) -> AppResult<Vec<i64>> {
    let key = LayoutKey { width, row_height, gap, scope: scope.unwrap_or_default(), group_by: group_by.unwrap_or_default() };
    Ok(state.layout_for(key)?.ids())
}

/// How many assets a scope selects. Cheap enough for a live count while typing.
#[tauri::command(async)]
pub fn count_scope(state: State<'_, AppState>, scope: Option<LayoutScope>) -> AppResult<i64> {
    let lib = state.library()?;
    let scope = scope.unwrap_or_default();
    state.check_vault(&scope)?;
    let scope = crate::search::understood(&state, &scope);
    Ok(lib.db.with_conn(|c| count_scoped(c, &scope))?)
}

/// The ids a scope selects, in its order, at most `limit` (10,000 when not given): *Export…*
/// and *Start slideshow* from an album's, a collection's or Favorites' menu (Round 4G).
#[tauri::command(async)]
pub fn scope_ids(state: State<'_, AppState>, scope: Option<LayoutScope>, limit: Option<i64>) -> AppResult<Vec<i64>> {
    let lib = state.library()?;
    let scope = scope.unwrap_or_default();
    state.check_vault(&scope)?;
    let scope = crate::search::understood(&state, &scope);
    Ok(lib.db.with_conn(|c| ids_scoped(c, &scope, limit.unwrap_or(10_000)))?)
}

/// The built-in collections (Favorites, Videos, Live Photos, RAW, HDR, …) with counts and
/// covers, for the Albums index.
#[tauri::command(async)]
pub fn list_collections(state: State<'_, AppState>) -> AppResult<Vec<photos_core::collections::Collection>> {
    let lib = state.library()?;
    Ok(lib.db.with_conn(photos_core::collections::list_collections)?)
}

/// Rebuild the search index from the catalogue (Settings → Library). Returns the row count.
#[tauri::command(async)]
pub fn rebuild_search_index(app: AppHandle, state: State<'_, AppState>) -> AppResult<usize> {
    let lib = state.library()?;
    let n = lib.db.with_conn(photos_core::fts::rebuild)?;
    state.changes.changed(&app, true);
    Ok(n)
}

// A command's arguments are its wire format: `get_layout`'s plus the window of rows.
#[allow(clippy::too_many_arguments)]
#[tauri::command(async)]
pub fn get_layout_rows(
    state: State<'_, AppState>,
    width: u32,
    row_height: u32,
    gap: u32,
    scope: Option<LayoutScope>,
    group_by: Option<GroupBy>,
    start: u32,
    count: u32,
) -> AppResult<Vec<LayoutRow>> {
    let key = LayoutKey { width, row_height, gap, scope: scope.unwrap_or_default(), group_by: group_by.unwrap_or_default() };
    let layout = state.layout_for(key)?;
    Ok(layout.rows(start as usize, count.min(MAX_ROWS_PER_CALL) as usize).to_vec())
}

#[tauri::command(async)]
pub fn get_asset(state: State<'_, AppState>, id: i64) -> AppResult<AssetDetail> {
    let lib = state.library()?;
    let ffmpeg = state.ffmpeg_available();
    Ok(lib.db.with_conn(|c| assets::get_asset(c, id, ffmpeg))?)
}

#[tauri::command(async)]
pub fn get_asset_neighbors(state: State<'_, AppState>, id: i64) -> AppResult<Neighbors> {
    let lib = state.library()?;
    Ok(lib.db.with_conn(|c| assets::neighbors(c, id))?)
}

#[tauri::command(async)]
pub fn get_library_stats(state: State<'_, AppState>) -> AppResult<LibraryStats> {
    let lib = state.library()?;
    Ok(lib.db.with_conn(assets::stats)?)
}

/// Home's extras (Round 20 C): the latest Develop drafts, files Lumen can't open, and the four
/// places with the most photos (from the cached tag facets, so no catalogue read).
#[tauri::command(async)]
pub fn home_summary(app: AppHandle, state: State<'_, AppState>) -> AppResult<HomeSummary> {
    let facets = crate::search::current_facets(&app, &state)?;
    let places = facets.top_places(4);
    let lib = state.library()?;
    let remembered = read_develop_source(&state, &lib.manifest.id);
    Ok(lib.db.with_conn(|c| home::summary(c, &places, remembered.as_ref()))?)
}

/// Serialises `develop-source.json`: the page writes it from a debounce and a flush at once.
static DEVELOP_SOURCE_FILE: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn develop_source_path(state: &AppState) -> std::path::PathBuf {
    state.paths.app_data.join("develop-source.json")
}

fn read_develop_source(state: &AppState, library_id: &str) -> Option<DevelopSourceFile> {
    let _held = DEVELOP_SOURCE_FILE.lock().unwrap_or_else(|p| p.into_inner());
    develop_source::read(&develop_source_path(state), library_id)
}

/// The Develop filmstrip this library last had, without the photos that have left it (Round 21 §13).
#[tauri::command(async)]
pub fn get_develop_source(state: State<'_, AppState>) -> AppResult<DevelopSourceFile> {
    let lib = state.library()?;
    let Some(mut file) = read_develop_source(&state, &lib.manifest.id) else { return Ok(DevelopSourceFile::default()) };
    lib.db.with_conn(|c| develop_source::prune(c, &mut file))?;
    Ok(file)
}

/// Remember the filmstrip and the photo last open.
#[tauri::command(async)]
pub fn set_develop_source(state: State<'_, AppState>, source: Option<RememberedStrip>, last: Option<i64>) -> AppResult<()> {
    let lib = state.library()?;
    let _held = DEVELOP_SOURCE_FILE.lock().unwrap_or_else(|p| p.into_inner());
    develop_source::write(&develop_source_path(&state), &lib.manifest.id, &DevelopSourceFile { source, last })
        .map_err(|e| AppError { code: "develop_source_write".into(), message: "Couldn't remember the Develop filmstrip.".into(), detail: Some(e.to_string()) })
}

/// Every photo with Develop work, newest change first (the card's *Open Develop* with nothing remembered).
#[tauri::command(async)]
pub fn list_draft_ids(state: State<'_, AppState>) -> AppResult<Vec<i64>> {
    let lib = state.library()?;
    Ok(lib.db.with_conn(|c| home::draft_ids(c, home::DRAFT_IDS_CAP))?)
}

#[tauri::command(async)]
pub fn get_recent_asset_ids(state: State<'_, AppState>, batch_id: i64, limit: i64) -> AppResult<Vec<i64>> {
    let lib = state.library()?;
    Ok(lib.db.with_conn(|c| batches::recent_asset_ids(c, batch_id, limit.clamp(1, 100)))?)
}

#[tauri::command(async)]
pub fn set_favorite(app: AppHandle, state: State<'_, AppState>, ids: Vec<i64>, value: bool) -> AppResult<usize> {
    let lib = state.library()?;
    let n = lib.db.with_conn(|c| assets::set_favorite(c, &ids, value))?;
    state.changes.changed(&app, true);
    Ok(n)
}

/// Stars for a selection (viewer keys 1–5, 0 clears).
#[tauri::command(async)]
pub fn set_rating(app: AppHandle, state: State<'_, AppState>, ids: Vec<i64>, rating: i64) -> AppResult<usize> {
    let lib = state.library()?;
    let n = lib.db.with_conn(|c| assets::set_rating(c, &ids, rating))?;
    state.changes.changed(&app, true);
    Ok(n)
}

/// A colour label for a selection (0 clears); returns what each changed photo had, for Undo.
#[tauri::command(async)]
pub fn set_color_label(app: AppHandle, state: State<'_, AppState>, ids: Vec<i64>, label: i64) -> AppResult<Vec<labels::PreviousMark>> {
    let lib = state.library()?;
    let changed = lib.db.with_conn(|c| labels::set_color_label(c, &ids, label))?;
    state.changes.changed(&app, true);
    Ok(changed)
}

/// Pick (1), reject (−1) or clear (0) a selection; returns what each changed photo had.
#[tauri::command(async)]
pub fn set_flag(app: AppHandle, state: State<'_, AppState>, ids: Vec<i64>, flag: i64) -> AppResult<Vec<labels::PreviousMark>> {
    let lib = state.library()?;
    let changed = lib.db.with_conn(|c| labels::set_flag(c, &ids, flag))?;
    state.changes.changed(&app, true);
    Ok(changed)
}

#[tauri::command(async)]
pub fn get_favorite_ids(state: State<'_, AppState>, limit: i64) -> AppResult<Vec<i64>> {
    let lib = state.library()?;
    Ok(lib.db.with_conn(|c| assets::favorite_ids(c, limit.clamp(1, 200)))?)
}

#[tauri::command(async)]
pub fn trash_assets(app: AppHandle, state: State<'_, AppState>, ids: Vec<i64>) -> AppResult<usize> {
    let lib = state.library()?;
    let n = lib.db.with_conn(|c| assets::trash_assets(c, &ids, photos_core::now_unix()))?;
    state.changes.changed(&app, true);
    Ok(n)
}

#[tauri::command(async)]
pub fn restore_assets(app: AppHandle, state: State<'_, AppState>, ids: Vec<i64>) -> AppResult<usize> {
    let lib = state.library()?;
    let n = lib.db.with_conn(|c| assets::restore_assets(c, &ids))?;
    state.changes.changed(&app, true);
    Ok(n)
}

/// Permanently deletes trashed items (files and rows). Irreversible; the UI confirms first.
#[tauri::command(async)]
pub fn delete_now(app: AppHandle, state: State<'_, AppState>, ids: Vec<i64>) -> AppResult<usize> {
    let lib = state.library()?;
    let store = photos_media::DerivativeStore::new(&lib.root);
    let n = assets::delete_permanently(&lib, Some(&ids), &|h| store.delete_all(h))?;
    state.changes.changed(&app, true);
    Ok(n)
}

#[tauri::command(async)]
pub fn empty_trash(app: AppHandle, state: State<'_, AppState>) -> AppResult<usize> {
    let lib = state.library()?;
    let store = photos_media::DerivativeStore::new(&lib.root);
    let n = assets::delete_permanently(&lib, None, &|h| store.delete_all(h))?;
    state.changes.changed(&app, true);
    Ok(n)
}

/// Warm the viewer-size derivatives for neighbours on the Prefetch lane; returns at once.
#[tauri::command(async)]
pub fn prefetch_previews(app: AppHandle, ids: Vec<i64>) -> AppResult<()> {
    crate::derivatives::prefetch(&app, &ids);
    Ok(())
}

/// Shows the original in Explorer / Finder / the file manager.
/// Copy the originals of `ids` into `dir`. Copies, never moves; never overwrites. Runs on
/// the Prefetch lane so a big export cannot starve thumbnails.
#[tauri::command(async)]
pub fn copy_assets_to(state: State<'_, AppState>, ids: Vec<i64>, dir: String) -> AppResult<u64> {
    let lib = state.library()?;
    let target = std::path::PathBuf::from(&dir);
    if !target.is_dir() {
        return Err(AppError { code: "not_a_folder".into(), message: "That path is not a folder.".into(), detail: Some(dir) });
    }
    let rows = lib.db.with_conn(|c| assets::export_paths(c, &ids))?;
    let originals = lib.originals_dir();
    let total = rows.len() as u64;
    let copied = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
    let counter = copied.clone();
    let handle = state.jobs.submit("export:copy", photos_jobs::Lane::Prefetch, move |cancel| {
        for (rel, filename) in &rows {
            if cancel.is_cancelled() {
                return;
            }
            let out = assets::unique_export_name(&target, filename);
            match std::fs::copy(originals.join(rel), &out) {
                Ok(_) => {
                    counter.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                }
                Err(e) => tracing::warn!(file = %rel, "export copy failed: {e}"),
            }
        }
    });
    // An export is a foreground action and the caller wants the count, so wait for it.
    handle.wait_timeout(std::time::Duration::from_secs(600));
    let n = copied.load(std::sync::atomic::Ordering::Relaxed);
    if n < total {
        tracing::warn!(copied = n, total, "some files could not be copied");
    }
    Ok(n)
}

/// *Copy image* (Round 4G): the photo as the viewer shows it, edits applied, at the 2560
/// preview's size, onto the system clipboard.
#[tauri::command(async)]
pub fn copy_image_to_clipboard(app: AppHandle, id: i64) -> AppResult<()> {
    let path = crate::derivatives::ensure(&app, id, 2560, photos_jobs::Lane::Interactive)?;
    let image = crate::clipboard::prepare(&path)?;
    crate::clipboard::copy(&mut crate::clipboard::System, image)
}

#[tauri::command(async)]
pub fn reveal_asset(app: AppHandle, state: State<'_, AppState>, id: i64) -> AppResult<()> {
    let lib = state.library()?;
    let path = lib.original_path(id)?;
    tauri_plugin_opener::reveal_item_in_dir(&path).map_err(|e| AppError {
        code: "reveal_failed".into(),
        message: "Could not open the file manager.".into(),
        detail: Some(e.to_string()),
    })?;
    let _ = app;
    Ok(())
}
