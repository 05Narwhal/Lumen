use photos_core::Settings;
use tauri::{AppHandle, State};

use crate::error::AppResult;
use crate::state::AppState;

#[tauri::command(async)]
pub fn get_settings(state: State<'_, AppState>) -> AppResult<Settings> {
    Ok(state.settings.read().expect("settings lock").clone())
}

#[tauri::command(async)]
pub fn update_settings(app: AppHandle, state: State<'_, AppState>, mut settings: Settings) -> AppResult<Settings> {
    let was_preparing = crate::video_prep::enabled(&state.settings.read().expect("settings lock").playback);
    let was_reading = crate::models::read_text_on(&state);
    let was_masking = crate::facet::masks_on(&state);
    let was_exact = state.settings.read().expect("settings lock").exact_places;
    let old_updates =state.settings.read().expect("settings lock").updates.clone();
    // The window's place and *Restart*'s location belong to Rust; the page's copy may be stale.
    settings.window = state.settings.read().expect("settings lock").window.clone();
    // The AI assistants' key is made by Rust on first enable (Round 16), never by the page.
    settings.ai_assistants.token = state.settings.read().expect("settings lock").ai_assistants.token.clone();
    if settings.ai_assistants.enabled && settings.ai_assistants.token.is_empty() {
        settings.ai_assistants.token = crate::mcp::new_token();
    }
    *state.settings.write().expect("settings lock") = settings.clone();
    state.save_settings()?;
    crate::mcp::apply(&app);
    // Round 17: a staged build the new channel doesn't take goes; a new channel is asked.
    crate::updater::settings_changed(&app, &old_updates, &settings.updates);
    // Turned on: prepare what is waiting now (Round 9A). Turned off, the walk stops by itself.
    if !was_preparing && crate::video_prep::enabled(&settings.playback) {
        crate::video_prep::schedule(&app);
    }
    // *Read text in photos* turned on (Round 13): read what is waiting. Off, the scan stops by itself.
    if !was_reading && crate::models::read_text_on(&state) {
        crate::ocr::schedule(&app);
    }
    // AI masks turned on (Phase 8f): fetch Facet now, not at the next launch.
    if !was_masking && crate::facet::masks_on(&state) {
        crate::models::fetch_all(&app);
    }
    // Exact place names turned on: name what the atlas named. Off, the run stops by itself.
    if !was_exact && settings.exact_places {
        crate::places_online::wake();
    }
    let hide = settings.appearance.hide_raw_with_jpeg;
    if hide != photos_core::scope::hide_raw_with_jpeg() {
        photos_core::scope::set_hide_raw_with_jpeg(hide);
        state.layout.lock().expect("layout lock").invalidate();
        state.changes.changed(&app, true);
    }
    Ok(settings)
}

/// The date order in force (Round 11): the setting, or the system's when it is *Automatic*.
#[tauri::command(async)]
pub fn get_date_order(state: State<'_, AppState>) -> AppResult<photos_core::DateOrder> {
    Ok(crate::locale::current(&state))
}

/// What *Automatic* means on this system, for Settings to name it.
#[tauri::command(async)]
pub fn system_date_order() -> AppResult<photos_core::DateOrder> {
    Ok(crate::locale::date_order(photos_core::DateOrderSetting::Auto))
}
