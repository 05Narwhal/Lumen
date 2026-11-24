//! Develop commands (Phase 8c): thin adapters over `develop_preview` and `photos_core`.

use photos_core::develop::{self, Panel, PreviousRecipe};
use photos_core::develop_history::{self, HistoryStep};
use photos_core::develop_presets::{self, Preset};
use photos_core::develop_strip::{self, StripCursor, StripPage, StripSort};
use photos_core::edits::{Crop, Develop, Recipe};
use photos_core::scope::LayoutScope;
use tauri::{AppHandle, State};

use crate::develop_preview::{self, DevelopBaseInfo, DevelopPreview};
use crate::error::{AppError, AppResult};
use crate::state::AppState;

/// The live preview: off the main thread, latest wins (see `develop_preview`), on the render
/// pool so background work never slows it (`photos_jobs::interactive`).
#[tauri::command(async)]
pub fn develop_preview(state: State<'_, AppState>, asset_id: i64, recipe: Recipe, request: u64, before: bool, clipping: bool) -> AppResult<DevelopPreview> {
    // The turn is waited for outside the render pool (Round 26 B2, `DevelopState::in_turn`).
    state
        .develop
        .in_turn(asset_id, develop_preview::Channel::Preview, request, |turn| develop_preview::render_preview(&state, turn, asset_id, &recipe, before, clipping))
        .unwrap_or_else(|| Ok(DevelopPreview::superseded()))
}

/// *Auto* on the draft recipe (Phase 8d): the Basic values to apply, off the main thread.
#[tauri::command(async)]
pub fn develop_auto(app: AppHandle, state: State<'_, AppState>, asset_id: i64, recipe: Recipe, task: Option<String>) -> AppResult<crate::develop_auto::AutoResult> {
    let task = crate::ai_task::AiTask::start(app.clone(), task, "auto");
    let out = photos_jobs::interactive(|| crate::develop_auto::run(&app, &state, asset_id, &recipe, &task));
    task.finish(out.is_ok());
    out
}

/// *Retouch* on the draft recipe (Round 12): the trained model's whole recipe, or classic
/// Auto while it is switched off or not ready. Writes nothing.
#[tauri::command(async)]
pub fn develop_retouch(app: AppHandle, state: State<'_, AppState>, asset_id: i64, recipe: Recipe, task: Option<String>) -> AppResult<crate::develop_retouch::RetouchResult> {
    let task = crate::ai_task::AiTask::start(app.clone(), task, "retouch");
    let out = photos_jobs::interactive(|| crate::develop_retouch::run(&app, &state, asset_id, &recipe, &task));
    task.finish(out.is_ok());
    out
}

/// Settings → Experimental: one row per AI task with its models, tiers and timings here.
#[tauri::command(async)]
pub fn experimental_catalog(app: AppHandle, state: State<'_, AppState>) -> AppResult<Vec<crate::model_bench::ExperimentalTask>> {
    Ok(crate::model_bench::experimental_catalog(&app, &state))
}

/// Times every model × tier on this PC again (*Re-test*).
#[tauri::command(async)]
pub fn retest_models(app: AppHandle) -> AppResult<()> {
    crate::model_bench::schedule(&app, true, std::time::Duration::ZERO);
    Ok(())
}

/// The photo's before (develop off, geometry kept) for split and side by side.
#[tauri::command(async)]
pub fn develop_before(state: State<'_, AppState>, asset_id: i64, recipe: Recipe) -> AppResult<DevelopPreview> {
    photos_jobs::interactive(|| develop_preview::render_before(&state, asset_id, &recipe))
}

/// The zoomed view of the visible `rect`, at `scale` of full resolution (1 when omitted).
#[tauri::command(async)]
pub fn develop_region(state: State<'_, AppState>, asset_id: i64, recipe: Recipe, rect: Crop, scale: Option<f32>, request: u64, before: bool) -> AppResult<DevelopPreview> {
    let channel = if before { develop_preview::Channel::BeforeRegion } else { develop_preview::Channel::Region };
    state
        .develop
        .in_turn(asset_id, channel, request, |turn| develop_preview::render_region(&state, turn, asset_id, &recipe, rect, scale.unwrap_or(1.0), before))
        .unwrap_or_else(|| Ok(DevelopPreview::superseded()))
}

/// Frees the 100 % view's full-size frame.
#[tauri::command(async)]
pub fn develop_release_full(state: State<'_, AppState>) {
    state.develop.release_full();
}

#[tauri::command(async)]
pub fn develop_base_info(state: State<'_, AppState>, asset_id: i64) -> AppResult<DevelopBaseInfo> {
    develop_preview::base_info(&state, asset_id)
}

fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

// Presets

/// Built-ins, the person's own, then each enabled plugin's presets (read-only, id 0, told apart by `key`).
#[tauri::command(async)]
pub fn list_presets(state: State<'_, AppState>, plugins: State<'_, std::sync::Arc<crate::plugins::Plugins>>) -> AppResult<Vec<Preset>> {
    let mut all = state.library()?.db.with_conn(develop_presets::list_presets)?;
    all.extend(crate::plugins::data::plugin_presets(&plugins.read()));
    Ok(all)
}

/// *Duplicate to my presets*: writes a plugin's preset as the person's own row.
#[tauri::command(async)]
pub fn duplicate_plugin_preset(state: State<'_, AppState>, plugins: State<'_, std::sync::Arc<crate::plugins::Plugins>>, key: String) -> AppResult<Preset> {
    let source = crate::plugins::data::plugin_preset(&plugins.read(), &key)
        .ok_or_else(|| AppError { code: "not_found".into(), message: "That plugin preset isn't there any more.".into(), detail: Some(key.clone()) })?;
    Ok(state.library()?.db.with_conn(|c| develop_presets::duplicate_preset(c, &source, now_ms() / 1000))?)
}

#[tauri::command(async)]
pub fn save_preset(state: State<'_, AppState>, name: String, develop: Develop, panels: Vec<Panel>) -> AppResult<Preset> {
    Ok(state.library()?.db.with_conn(|c| develop_presets::save_preset(c, &name, &develop, &panels, now_ms() / 1000))?)
}

#[tauri::command(async)]
pub fn rename_preset(state: State<'_, AppState>, id: i64, name: String) -> AppResult<()> {
    Ok(state.library()?.db.with_conn(|c| develop_presets::rename_preset(c, id, &name))?)
}

#[tauri::command(async)]
pub fn delete_preset(state: State<'_, AppState>, id: i64) -> AppResult<()> {
    Ok(state.library()?.db.with_conn(|c| develop_presets::delete_preset(c, id))?)
}

// History and snapshots

#[tauri::command(async)]
pub fn list_history(state: State<'_, AppState>, asset_id: i64) -> AppResult<Vec<HistoryStep>> {
    Ok(state.library()?.db.with_conn(|c| develop_history::list_history(c, asset_id))?)
}

#[tauri::command(async)]
pub fn push_history(state: State<'_, AppState>, asset_id: i64, recipe: Recipe, label: String) -> AppResult<i64> {
    Ok(state.library()?.db.with_conn(|c| develop_history::push_step(c, asset_id, &recipe, &label, now_ms()))?)
}

#[tauri::command(async)]
pub fn truncate_history(state: State<'_, AppState>, asset_id: i64, step_id: i64) -> AppResult<usize> {
    Ok(state.library()?.db.with_conn(|c| develop_history::truncate_after(c, asset_id, step_id))?)
}

#[tauri::command(async)]
pub fn clear_history(state: State<'_, AppState>, asset_id: i64) -> AppResult<usize> {
    Ok(state.library()?.db.with_conn(|c| develop_history::clear_history(c, asset_id))?)
}

#[tauri::command(async)]
pub fn save_snapshot(state: State<'_, AppState>, asset_id: i64, name: String, recipe: Recipe) -> AppResult<i64> {
    Ok(state.library()?.db.with_conn(|c| develop_history::save_snapshot(c, asset_id, &name, &recipe, now_ms()))?)
}

#[tauri::command(async)]
pub fn rename_snapshot(state: State<'_, AppState>, id: i64, name: String) -> AppResult<()> {
    Ok(state.library()?.db.with_conn(|c| develop_history::rename_snapshot(c, id, &name))?)
}

#[tauri::command(async)]
pub fn delete_snapshot(state: State<'_, AppState>, id: i64) -> AppResult<()> {
    Ok(state.library()?.db.with_conn(|c| develop_history::delete_snapshot(c, id))?)
}

// Drafts (Round 6): Develop's settings stay in Develop until a photo is exported.

/// What Develop opens a photo with: its draft, or else the library's edit.
#[tauri::command(async)]
pub fn get_develop_draft(state: State<'_, AppState>, asset_id: i64) -> AppResult<Option<Recipe>> {
    Ok(state.library()?.db.with_conn(|c| photos_core::edits::develop_recipe(c, asset_id))?)
}

/// Keeps Develop's settings; the library, viewer and thumbnails go on showing `edits`.
#[tauri::command(async)]
pub fn save_develop_draft(state: State<'_, AppState>, asset_id: i64, recipe: Recipe) -> AppResult<i64> {
    Ok(state.library()?.db.with_conn(|c| photos_core::edits::save_draft(c, asset_id, &recipe, now_ms()))?)
}

/// *Undo* of a paste or a reset in Develop: the drafts as they were.
#[tauri::command(async)]
pub fn restore_develop_drafts(app: AppHandle, state: State<'_, AppState>, previous: Vec<PreviousRecipe>) -> AppResult<()> {
    state.library()?.db.with_conn(|c| develop::restore_drafts(c, &previous, now_ms()))?;
    state.changes.changed(&app, true);
    Ok(())
}

// Paste and sync

/// Take `panels` of `develop` onto `ids`; returns what they had, for *Undo*.
#[tauri::command(async)]
pub fn paste_develop(app: AppHandle, state: State<'_, AppState>, ids: Vec<i64>, develop: Develop, panels: Vec<Panel>, label: String) -> AppResult<Vec<PreviousRecipe>> {
    let previous = state.library()?.db.with_conn(|c| develop::paste_develop(c, &ids, &develop, &panels, &label, now_ms()))?;
    state.changes.changed(&app, true);
    Ok(previous)
}

#[tauri::command(async)]
pub fn restore_recipes(app: AppHandle, state: State<'_, AppState>, previous: Vec<PreviousRecipe>) -> AppResult<()> {
    state.library()?.db.with_conn(|c| develop::restore_recipes(c, &previous, now_ms()))?;
    state.changes.changed(&app, true);
    Ok(())
}

// The filmstrip

#[tauri::command(async)]
pub fn strip_page(state: State<'_, AppState>, scope: LayoutScope, only: Option<Vec<i64>>, sort: StripSort, cursor: Option<StripCursor>, limit: u32) -> AppResult<StripPage> {
    Ok(state.library()?.db.with_conn(|c| develop_strip::strip_page(c, &scope, only.as_deref(), sort, cursor.as_ref(), limit))?)
}

#[tauri::command(async)]
pub fn strip_ids(state: State<'_, AppState>, scope: LayoutScope, only: Option<Vec<i64>>, sort: StripSort) -> AppResult<Vec<i64>> {
    Ok(state.library()?.db.with_conn(|c| develop_strip::strip_ids(c, &scope, only.as_deref(), sort))?)
}
