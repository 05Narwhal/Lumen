//! People (Phase 7b): thin adapters over `photos_core::people`.

use photos_core::people::{self, AssetFace, FacePlace, MergeUndo, PersonSummary, PersonUndo, RejectedFace, Suggestion, SuggestionUndo, Thresholds, UntagUndo};
use photos_core::person_groups::{self, PersonGroup};
use tauri::{AppHandle, State};

use crate::error::AppResult;
use crate::state::AppState;

/// People changed by the user: the grid and names refresh, and people are regrouped (debounced
/// on the Maintenance lane), so faces the changed people now clearly match are placed.
fn changed(app: &AppHandle, state: &AppState) {
    state.changes.changed(app, true);
    crate::faces::schedule_recluster(app);
}

#[tauri::command(async)]
pub fn list_people(state: State<'_, AppState>) -> AppResult<Vec<PersonSummary>> {
    let lib = state.library()?;
    Ok(lib.db.with_conn(people::list_people)?)
}

/// Settings → People: faces set aside as not people (the filter, *Not a person*), newest
/// first, a page at a time.
#[tauri::command(async)]
pub fn list_rejected_faces(state: State<'_, AppState>, before: Option<i64>, limit: Option<usize>) -> AppResult<Vec<RejectedFace>> {
    let lib = state.library()?;
    Ok(lib.db.with_conn(|c| people::rejected_faces(c, before, limit.unwrap_or(200)))?)
}

/// Settings → People: the people removed from People, to restore.
#[tauri::command(async)]
pub fn list_removed_people(state: State<'_, AppState>) -> AppResult<Vec<PersonSummary>> {
    let lib = state.library()?;
    Ok(lib.db.with_conn(people::removed_people)?)
}

#[tauri::command(async)]
pub fn name_person(app: AppHandle, state: State<'_, AppState>, id: i64, name: String) -> AppResult<()> {
    let lib = state.library()?;
    lib.db.with_conn(|c| people::name_person(c, id, &name))?;
    changed(&app, &state);
    Ok(())
}

#[tauri::command(async)]
pub fn merge_people(app: AppHandle, state: State<'_, AppState>, into: i64, from: i64) -> AppResult<MergeUndo> {
    let lib = state.library()?;
    let undo = lib.db.with_conn(|c| people::merge_people(c, into, from))?;
    changed(&app, &state);
    Ok(undo)
}

/// Undo of a merge.
#[tauri::command(async)]
pub fn restore_person(app: AppHandle, state: State<'_, AppState>, undo: MergeUndo) -> AppResult<i64> {
    let lib = state.library()?;
    let id = lib.db.with_conn(|c| people::restore_person(c, &undo, photos_core::now_unix()))?;
    changed(&app, &state);
    Ok(id)
}

#[tauri::command(async)]
pub fn move_face(app: AppHandle, state: State<'_, AppState>, face: i64, person: Option<i64>) -> AppResult<()> {
    let lib = state.library()?;
    let was = lib.db.with_conn(|c| people::face_person(c, face))?;
    lib.db.with_conn(|c| people::move_face(c, face, person))?;
    changed(&app, &state);
    crate::face_radar::seed(&app, &[person, was], &[face]);
    Ok(())
}

/// *Tag…* on a face in the viewer (Round 4I): it is `person`. Returns where it was, for Undo
/// (`restore_faces`).
#[tauri::command(async)]
pub fn tag_face(app: AppHandle, state: State<'_, AppState>, face: i64, person: i64) -> AppResult<FacePlace> {
    let lib = state.library()?;
    let place = lib.db.with_conn(|c| people::tag_face(c, face, person))?;
    changed(&app, &state);
    crate::face_radar::seed(&app, &[Some(person), place.person], &[face]);
    Ok(place)
}

/// *New person…* on a face: returns the new person and where the face was, for
/// `undo_create_person`.
#[tauri::command(async)]
pub fn create_person_with_face(app: AppHandle, state: State<'_, AppState>, face: i64, name: String) -> AppResult<(i64, FacePlace)> {
    let lib = state.library()?;
    let made = lib.db.with_conn(|c| people::create_person_with_face(c, face, &name, photos_core::now_unix()))?;
    changed(&app, &state);
    crate::face_radar::seed(&app, &[Some(made.0), made.1.person], &[face]);
    Ok(made)
}

#[tauri::command(async)]
pub fn undo_create_person(app: AppHandle, state: State<'_, AppState>, person: i64, place: FacePlace) -> AppResult<()> {
    let lib = state.library()?;
    lib.db.with_conn(|c| people::undo_create_person(c, person, &place))?;
    changed(&app, &state);
    Ok(())
}

/// *Add a face*: a box the user drew (0..1 of the unedited photo) on `person`, or on a new
/// person called `new_name`. The photo is decoded and embedded off the main thread; nothing
/// is cached. Returns the face's id and, for a new person, theirs.
#[tauri::command(async)]
pub fn add_face_box(app: AppHandle, state: State<'_, AppState>, asset_id: i64, rect: [f32; 4], person_id: Option<i64>, new_name: Option<String>) -> AppResult<(i64, Option<i64>)> {
    let face = crate::faces::embed_box(&app, asset_id, rect)?;
    let lib = state.library()?;
    let made = lib.db.with_conn(|c| {
        let (person, created) = match (person_id, new_name.as_deref()) {
            (Some(p), _) => (p, None),
            (None, Some(name)) => {
                let p = people::create_person(c, name, photos_core::now_unix())?;
                (p, Some(p))
            }
            (None, None) => return Err(photos_core::CoreError::Invalid("Choose who this is.".into())),
        };
        Ok((people::add_manual_face(c, &face, person, None)?, created))
    })?;
    changed(&app, &state);
    Ok(made)
}

/// *Not {name}* on a face: a scanned face is refused by them, a drawn one deleted.
#[tauri::command(async)]
pub fn untag_face(app: AppHandle, state: State<'_, AppState>, face: i64) -> AppResult<UntagUndo> {
    let lib = state.library()?;
    let undo = lib.db.with_conn(|c| people::untag_face(c, face))?;
    changed(&app, &state);
    // *Not Ruta*: her other photos of this look-alike come off her too (Round 10).
    crate::face_radar::seed(&app, &[undo.place.person], &[face]);
    Ok(undo)
}

/// *Not a face* on a face box (Round 4G): rejected by the user, as Settings → People lists it.
#[tauri::command(async)]
pub fn reject_face(app: AppHandle, state: State<'_, AppState>, face: i64) -> AppResult<FacePlace> {
    let lib = state.library()?;
    let place = lib.db.with_conn(|c| people::reject_face(c, face))?;
    changed(&app, &state);
    Ok(place)
}

#[tauri::command(async)]
pub fn undo_reject_face(app: AppHandle, state: State<'_, AppState>, place: FacePlace) -> AppResult<()> {
    let lib = state.library()?;
    lib.db.with_conn(|c| people::undo_reject_face(c, &place))?;
    changed(&app, &state);
    Ok(())
}

#[tauri::command(async)]
pub fn undo_untag_face(app: AppHandle, state: State<'_, AppState>, undo: UntagUndo) -> AppResult<()> {
    let lib = state.library()?;
    lib.db.with_conn(|c| people::undo_untag_face(c, &undo))?;
    changed(&app, &state);
    Ok(())
}

/// Faces that may be this person, for "Is this Anna?" on their page (Round 4B). The likeness
/// scan runs on the render pool: on the global one it queued behind the face scan's work
/// (Round 5B, real library under load: 0.7–1.2 s there, 0.24–0.3 s here).
#[tauri::command(async)]
pub fn person_suggestions(state: State<'_, AppState>, person: i64, limit: Option<usize>) -> AppResult<Vec<Suggestion>> {
    let lib = state.library()?;
    Ok(photos_jobs::interactive(|| people::suggestions_on(&lib.db, person, &Thresholds::MEASURED, limit.unwrap_or(24)))?)
}

#[tauri::command(async)]
pub fn answer_suggestion(app: AppHandle, state: State<'_, AppState>, face: i64, person: i64, yes: bool) -> AppResult<SuggestionUndo> {
    let lib = state.library()?;
    let undo = lib.db.with_conn(|c| people::answer_suggestion(c, face, person, yes, photos_core::now_unix()))?;
    changed(&app, &state);
    crate::face_radar::seed(&app, &[Some(person)], &[face]);
    Ok(undo)
}

#[tauri::command(async)]
pub fn undo_suggestion(app: AppHandle, state: State<'_, AppState>, undo: SuggestionUndo) -> AppResult<()> {
    let lib = state.library()?;
    lib.db.with_conn(|c| people::undo_suggestion(c, &undo))?;
    changed(&app, &state);
    Ok(())
}

/// "This is a real face": a detection the filter took for a screen, a drawing or a statue
/// comes back (Round 4B).
#[tauri::command(async)]
pub fn restore_face(app: AppHandle, state: State<'_, AppState>, face: i64) -> AppResult<()> {
    let lib = state.library()?;
    lib.db.with_conn(|c| people::restore_face(c, face))?;
    changed(&app, &state);
    Ok(())
}

/// `person` is not in these photos: their faces there go to `to`, or to nobody.
#[tauri::command(async)]
pub fn move_person_photos(app: AppHandle, state: State<'_, AppState>, person: i64, asset_ids: Vec<i64>, to: Option<i64>) -> AppResult<Vec<FacePlace>> {
    let lib = state.library()?;
    let moved = lib.db.with_conn(|c| people::move_photos(c, person, &asset_ids, to))?;
    changed(&app, &state);
    crate::face_radar::seed(&app, &[Some(person), to], &moved.iter().map(|m| m.face).collect::<Vec<_>>());
    Ok(moved)
}

#[tauri::command(async)]
pub fn restore_faces(app: AppHandle, state: State<'_, AppState>, places: Vec<FacePlace>) -> AppResult<()> {
    let lib = state.library()?;
    lib.db.with_conn(|c| people::restore_faces(c, &places))?;
    changed(&app, &state);
    Ok(())
}

#[tauri::command(async)]
pub fn remove_person(app: AppHandle, state: State<'_, AppState>, id: i64) -> AppResult<()> {
    let lib = state.library()?;
    lib.db.with_conn(|c| people::remove_person(c, id))?;
    changed(&app, &state);
    Ok(())
}

#[tauri::command(async)]
pub fn restore_removed_person(app: AppHandle, state: State<'_, AppState>, id: i64) -> AppResult<()> {
    let lib = state.library()?;
    lib.db.with_conn(|c| people::restore_removed(c, id))?;
    changed(&app, &state);
    Ok(())
}

/// *Not a person*: every face of this "person" is a false detection.
#[tauri::command(async)]
pub fn reject_person(app: AppHandle, state: State<'_, AppState>, id: i64) -> AppResult<PersonUndo> {
    let lib = state.library()?;
    let undo = lib.db.with_conn(|c| people::reject_person(c, id))?;
    changed(&app, &state);
    Ok(undo)
}

#[tauri::command(async)]
pub fn undo_reject_person(app: AppHandle, state: State<'_, AppState>, undo: PersonUndo) -> AppResult<()> {
    let lib = state.library()?;
    lib.db.with_conn(|c| people::undo_reject_person(c, &undo))?;
    changed(&app, &state);
    Ok(())
}

/// *Use as main photo*. Returns the cover it replaced, for Undo.
#[tauri::command(async)]
pub fn set_person_cover(app: AppHandle, state: State<'_, AppState>, person: i64, face: i64) -> AppResult<Option<i64>> {
    let lib = state.library()?;
    let before = lib.db.with_conn(|c| people::set_cover(c, person, face))?;
    changed(&app, &state);
    Ok(before)
}

#[tauri::command(async)]
pub fn faces_for_asset(state: State<'_, AppState>, id: i64) -> AppResult<Vec<AssetFace>> {
    let lib = state.library()?;
    Ok(lib.db.with_conn(|c| people::faces_for_asset(c, id))?)
}

// Manual groups (Family, Friends, …). They change nothing a photo shows, so no
// `library://changed` is sent; the People screen refetches the list itself.

#[tauri::command(async)]
pub fn list_person_groups(state: State<'_, AppState>) -> AppResult<Vec<PersonGroup>> {
    Ok(state.library()?.db.with_conn(person_groups::list)?)
}

#[tauri::command(async)]
pub fn create_person_group(state: State<'_, AppState>, name: String) -> AppResult<i64> {
    Ok(state.library()?.db.with_conn(|c| person_groups::create(c, &name))?)
}

#[tauri::command(async)]
pub fn rename_person_group(state: State<'_, AppState>, id: i64, name: String) -> AppResult<()> {
    Ok(state.library()?.db.with_conn(|c| person_groups::rename(c, id, &name))?)
}

/// Returns the members it had, so Undo can recreate it.
#[tauri::command(async)]
pub fn delete_person_group(state: State<'_, AppState>, id: i64) -> AppResult<Vec<i64>> {
    Ok(state.library()?.db.with_conn(|c| person_groups::delete(c, id))?)
}

#[tauri::command(async)]
pub fn set_person_group_members(state: State<'_, AppState>, group: i64, people: Vec<i64>, member: bool) -> AppResult<()> {
    Ok(state.library()?.db.with_conn(|c| person_groups::set_members(c, group, &people, member))?)
}
