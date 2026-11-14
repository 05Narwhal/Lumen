//! Edit commands: thin adapters over `photos_core::edits` and the render path.

use photos_core::assets;
use photos_core::edits::{self, EditVersion, Recipe};
use photos_jobs::Lane;
use photos_media::{develop, encode_jpeg_bytes, resize_long_edge, DerivativeStore};
use tauri::{AppHandle, State};

use crate::error::{AppError, AppResult};
use crate::state::AppState;

fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

fn render_failed(e: impl std::fmt::Display) -> AppError {
    AppError { code: "derivative_failed".into(), message: "The preview could not be rendered.".into(), detail: Some(e.to_string()) }
}

#[tauri::command(async)]
pub fn get_edit(state: State<'_, AppState>, asset_id: i64) -> AppResult<Option<Recipe>> {
    let lib = state.library()?;
    Ok(lib.db.with_conn(|c| edits::get_recipe(c, asset_id))?.map(|(r, _)| r))
}

/// Saves `recipe` and returns its version (0 when it was no edit at all, which reverts).
#[tauri::command(async)]
pub fn save_edit(app: AppHandle, state: State<'_, AppState>, asset_id: i64, recipe: Recipe) -> AppResult<i64> {
    let lib = state.library()?;
    let before = lib.db.with_conn(|c| edits::get_trim(c, asset_id))?;
    let version = lib.db.with_conn(|c| edits::save_recipe(c, asset_id, &recipe, now_ms()))?;
    // A video trimmed differently may have lost the part a slideshow uses (Round 6).
    if lib.db.with_conn(|c| edits::get_trim(c, asset_id))? != before {
        lib.db.with_conn(|c| photos_core::video_highlight::forget(c, asset_id))?;
        crate::video_highlights::schedule(&app);
    }
    state.changes.changed(&app, true);
    // The grid asks for the new 256 as soon as it sees the new version; start it now.
    let app2 = app.clone();
    std::thread::spawn(move || {
        let _ = crate::derivatives::ensure(&app2, asset_id, 256, Lane::Interactive);
    });
    Ok(version)
}

/// Turns photos a quarter from the grid (`dir` 1 clockwise, -1 anticlockwise); returns what
/// their recipes were, for *Undo* through `restore_recipes`.
#[tauri::command(async)]
pub fn rotate_assets(app: AppHandle, state: State<'_, AppState>, ids: Vec<i64>, dir: i32) -> AppResult<Vec<photos_core::develop::PreviousRecipe>> {
    let lib = state.library()?;
    let previous = lib.db.with_conn(|c| photos_core::rotation::rotate_assets(c, &ids, dir, now_ms()))?;
    state.changes.changed(&app, true);
    let turned: Vec<i64> = previous.iter().map(|p| p.asset_id).collect();
    let app2 = app.clone();
    std::thread::spawn(move || {
        for id in turned {
            let _ = crate::derivatives::ensure(&app2, id, 256, Lane::Interactive);
        }
    });
    Ok(previous)
}

/// Removes the recipe and returns it, so the frontend can offer *Undo*.
#[tauri::command(async)]
pub fn revert_edit(app: AppHandle, state: State<'_, AppState>, asset_id: i64) -> AppResult<Option<Recipe>> {
    let lib = state.library()?;
    let info = lib.db.with_conn(|c| assets::render_info(c, asset_id))?;
    let old = lib.db.with_conn(|c| edits::delete_recipe(c, asset_id))?;
    // The original's derivatives were never touched, so nothing is re-rendered.
    DerivativeStore::new(&lib.root).delete_edits(&info.content_hash);
    state.changes.changed(&app, true);
    Ok(old)
}

#[tauri::command(async)]
pub fn list_edit_versions(state: State<'_, AppState>) -> AppResult<Vec<EditVersion>> {
    let lib = state.library()?;
    Ok(lib.db.with_conn(edits::list_versions)?)
}

/// Renders `recipe` at 1024 px for the editor and returns a token for
/// `photos://edit-preview/{token}`. Runs on Tauri's command pool, not a scheduler lane: it
/// is the one thing the user is waiting on, and it must never queue behind an import.
#[tauri::command(async)]
pub fn render_edit_preview(app: AppHandle, state: State<'_, AppState>, asset_id: i64, recipe: Recipe) -> AppResult<String> {
    let base = state.edit_previews.base(asset_id, || -> AppResult<_> {
        // Always the unedited photo: previews start from the original, so edits never compound.
        let preview = crate::derivatives::ensure_original(&app, asset_id, 2560, Lane::Interactive)?;
        let img = image::open(&preview).map_err(render_failed)?;
        Ok(resize_long_edge(&img, 1024))
    })?;
    let lib = state.library()?;
    let hash = lib.db.with_conn(|c| assets::render_info(c, asset_id))?.content_hash;
    let rasters = crate::develop_preview::rasters_of(&lib, asset_id, &recipe)?;
    let ctx = develop::Ctx { ai_rasters: Some(&rasters), ..develop::Ctx::new(1.0, develop::seed_for(&hash)) };
    let out = photos_jobs::interactive(|| image::DynamicImage::ImageRgb8(develop::render_base(&develop::Linear::from_dynamic(&base), &recipe, &ctx)));
    let bytes = encode_jpeg_bytes(&out, 82).map_err(render_failed)?;
    Ok(state.edit_previews.put(bytes).to_string())
}

/// What a selection holds, for the export sheet.
#[tauri::command(async)]
pub fn export_summary(state: tauri::State<'_, crate::state::AppState>, ids: Vec<i64>) -> AppResult<photos_core::export::ExportSummary> {
    let lib = state.library()?;
    Ok(lib.db.with_conn(|c| photos_core::export::export_summary(c, &ids))?)
}

/// Exports `ids` as the user asked; returns the batch id at once (see `start_export`).
#[tauri::command(async)]
pub fn export_assets(app: AppHandle, ids: Vec<i64>, options: photos_core::export::ExportOptions) -> AppResult<u64> {
    crate::derivatives::start_export(&app, ids, options)
}
