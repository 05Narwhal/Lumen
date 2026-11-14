//! Pets (Round 4C): thin adapters over `photos_core::pets`.

use photos_core::people::Suggestion;
use photos_core::pets::{self, AssetPet, NotAPetUndo, PetMergeUndo, PetPlace, PetSummary};
use tauri::{AppHandle, State};

use crate::error::AppResult;
use crate::state::AppState;

/// Pets changed by the user: the grid and names refresh, and pets are regrouped (debounced).
fn changed(app: &AppHandle, state: &AppState) {
    state.changes.changed(app, true);
    crate::pets::schedule_recluster(app);
}

#[tauri::command(async)]
pub fn list_pets(state: State<'_, AppState>) -> AppResult<Vec<PetSummary>> {
    Ok(state.library()?.db.with_conn(pets::list_pets)?)
}

#[tauri::command(async)]
pub fn list_removed_pets(state: State<'_, AppState>) -> AppResult<Vec<PetSummary>> {
    Ok(state.library()?.db.with_conn(pets::removed_pets)?)
}

#[tauri::command(async)]
pub fn name_pet(app: AppHandle, state: State<'_, AppState>, pet: i64, name: String) -> AppResult<()> {
    state.library()?.db.with_conn(|c| pets::name_pet(c, pet, &name))?;
    changed(&app, &state);
    Ok(())
}

#[tauri::command(async)]
pub fn merge_pets(app: AppHandle, state: State<'_, AppState>, into: i64, from: i64) -> AppResult<PetMergeUndo> {
    let undo = state.library()?.db.with_conn(|c| pets::merge_pets(c, into, from))?;
    changed(&app, &state);
    Ok(undo)
}

#[tauri::command(async)]
pub fn undo_merge_pets(app: AppHandle, state: State<'_, AppState>, undo: PetMergeUndo) -> AppResult<()> {
    state.library()?.db.with_conn(|c| pets::undo_merge(c, &undo))?;
    changed(&app, &state);
    Ok(())
}

/// *These are a different pet*: the photos become a new pet. Returns it and what Undo needs.
#[tauri::command(async)]
pub fn split_pet(app: AppHandle, state: State<'_, AppState>, pet: i64, assets: Vec<i64>) -> AppResult<(i64, Vec<PetPlace>)> {
    let out = state.library()?.db.with_conn(|c| pets::split(c, pet, &assets, photos_core::now_unix()))?;
    changed(&app, &state);
    Ok(out)
}

/// *Not {name}*: the photos are not of this pet (to another, or back into play).
#[tauri::command(async)]
pub fn move_pet_photos(app: AppHandle, state: State<'_, AppState>, pet: i64, assets: Vec<i64>, to: Option<i64>) -> AppResult<Vec<PetPlace>> {
    let moved = state.library()?.db.with_conn(|c| pets::move_photos(c, pet, &assets, to))?;
    changed(&app, &state);
    Ok(moved)
}

/// Undo of a split, a move or a tag.
#[tauri::command(async)]
pub fn restore_pet_places(app: AppHandle, state: State<'_, AppState>, places: Vec<PetPlace>) -> AppResult<()> {
    state.library()?.db.with_conn(|c| pets::restore_places(c, &places))?;
    changed(&app, &state);
    Ok(())
}

#[tauri::command(async)]
pub fn tag_pet(app: AppHandle, state: State<'_, AppState>, detection: i64, pet: i64) -> AppResult<PetPlace> {
    let place = state.library()?.db.with_conn(|c| pets::tag_detection(c, detection, pet))?;
    changed(&app, &state);
    Ok(place)
}

#[tauri::command(async)]
pub fn remove_pet(app: AppHandle, state: State<'_, AppState>, pet: i64) -> AppResult<()> {
    state.library()?.db.with_conn(|c| pets::remove_pet(c, pet))?;
    changed(&app, &state);
    Ok(())
}

#[tauri::command(async)]
pub fn restore_removed_pet(app: AppHandle, state: State<'_, AppState>, pet: i64) -> AppResult<()> {
    state.library()?.db.with_conn(|c| pets::restore_removed(c, pet))?;
    changed(&app, &state);
    Ok(())
}

#[tauri::command(async)]
pub fn not_a_pet(app: AppHandle, state: State<'_, AppState>, pet: i64) -> AppResult<NotAPetUndo> {
    let undo = state.library()?.db.with_conn(|c| pets::not_a_pet(c, pet))?;
    changed(&app, &state);
    Ok(undo)
}

#[tauri::command(async)]
pub fn undo_not_a_pet(app: AppHandle, state: State<'_, AppState>, undo: NotAPetUndo) -> AppResult<()> {
    state.library()?.db.with_conn(|c| pets::undo_not_a_pet(c, &undo))?;
    changed(&app, &state);
    Ok(())
}

/// *Use as main photo*. Returns the cover it had.
#[tauri::command(async)]
pub fn set_pet_cover(app: AppHandle, state: State<'_, AppState>, pet: i64, detection: i64) -> AppResult<Option<i64>> {
    let before = state.library()?.db.with_conn(|c| pets::set_cover(c, pet, detection))?;
    state.changes.changed(&app, true);
    Ok(before)
}

/// "Is this Milo?": photos that may show the pet, best first.
#[tauri::command(async)]
pub fn pet_suggestions(state: State<'_, AppState>, pet: i64, limit: Option<usize>) -> AppResult<Vec<Suggestion>> {
    Ok(state.library()?.db.with_conn(|c| pets::suggestions(c, pet, &pets::MEASURED, limit.unwrap_or(24)))?)
}

/// Yes places the detection on the pet for good (Undo: `restore_pet_places` with what this
/// returns); No records that it is not this pet (Undo: `lift_pet_refusal`).
#[tauri::command(async)]
pub fn answer_pet_suggestion(app: AppHandle, state: State<'_, AppState>, detection: i64, pet: i64, yes: bool) -> AppResult<Option<PetPlace>> {
    let lib = state.library()?;
    let place = if yes {
        Some(lib.db.with_conn(|c| pets::tag_detection(c, detection, pet))?)
    } else {
        lib.db.with_conn(|c| pets::refuse(c, detection, pet, photos_core::now_unix()))?;
        None
    };
    changed(&app, &state);
    Ok(place)
}

#[tauri::command(async)]
pub fn lift_pet_refusal(app: AppHandle, state: State<'_, AppState>, detection: i64, pet: i64) -> AppResult<()> {
    state.library()?.db.with_conn(|c| pets::lift_refusal(c, detection, pet))?;
    changed(&app, &state);
    Ok(())
}

/// The pets in a photo, for the viewer's info panel.
#[tauri::command(async)]
pub fn asset_pets(state: State<'_, AppState>, asset: i64) -> AppResult<Vec<AssetPet>> {
    Ok(state.library()?.db.with_conn(|c| pets::asset_pets(c, asset))?)
}

/// *Not {name}* on one box in the viewer (Round 8B). Undo: `restore_pet_places`.
#[tauri::command(async)]
pub fn untag_pet_detection(app: AppHandle, state: State<'_, AppState>, detection: i64) -> AppResult<PetPlace> {
    let place = state.library()?.db.with_conn(|c| pets::untag_detection(c, detection))?;
    changed(&app, &state);
    Ok(place)
}

/// *Not a pet* on one box (Round 8B).
#[tauri::command(async)]
pub fn reject_pet_detection(app: AppHandle, state: State<'_, AppState>, detection: i64) -> AppResult<PetPlace> {
    let place = state.library()?.db.with_conn(|c| pets::reject_detection(c, detection))?;
    changed(&app, &state);
    Ok(place)
}

#[tauri::command(async)]
pub fn undo_reject_pet_detection(app: AppHandle, state: State<'_, AppState>, place: PetPlace) -> AppResult<()> {
    state.library()?.db.with_conn(|c| pets::undo_reject_detection(c, &place))?;
    changed(&app, &state);
    Ok(())
}

/// *New pet…* on one box (Round 8B). Returns the pet and where the box was.
#[tauri::command(async)]
pub fn create_pet_with_detection(app: AppHandle, state: State<'_, AppState>, detection: i64, name: String) -> AppResult<(i64, PetPlace)> {
    let out = state.library()?.db.with_conn(|c| pets::create_pet_with_detection(c, detection, &name, photos_core::now_unix()))?;
    changed(&app, &state);
    Ok(out)
}

#[tauri::command(async)]
pub fn undo_create_pet(app: AppHandle, state: State<'_, AppState>, pet: i64, place: PetPlace) -> AppResult<()> {
    state.library()?.db.with_conn(|c| pets::undo_create_pet(c, pet, &place))?;
    changed(&app, &state);
    Ok(())
}
