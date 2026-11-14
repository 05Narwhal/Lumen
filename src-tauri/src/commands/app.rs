use serde::Serialize;
use tauri::State;
use ts_rs::TS;

use crate::error::AppResult;
use crate::state::AppState;

#[derive(Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AppInfo {
    pub version: String,
    pub platform: String,
    pub app_data_dir: String,
    pub default_library_dir: String,
    pub is_debug: bool,
}

#[tauri::command(async)]
pub fn get_app_info(state: State<'_, AppState>) -> AppResult<AppInfo> {
    Ok(AppInfo {
        version: env!("CARGO_PKG_VERSION").into(),
        platform: std::env::consts::OS.into(),
        app_data_dir: state.paths.app_data.display().to_string(),
        default_library_dir: state.paths.default_library.display().to_string(),
        is_debug: cfg!(debug_assertions),
    })
}

/// Free bytes on the volume that holds `path` (or its nearest existing ancestor).
#[tauri::command(async)]
pub fn get_free_space(path: String) -> AppResult<u64> {
    let mut p = std::path::PathBuf::from(path);
    while !p.exists() {
        if !p.pop() {
            return Ok(0);
        }
    }
    Ok(fs4::available_space(&p)?)
}

/// The cheapest round trip (Round 15): ms since the epoch. The page times it to tell a busy
/// backend from a stuck one.
#[tauri::command(async)]
pub fn ping() -> u64 {
    crate::watchdog::now_ms()
}

/// The theme's base colour for the strip below a maximized window (`window_edge`).
#[tauri::command(async)]
pub fn paint_window_edge(app: tauri::AppHandle, r: u8, g: u8, b: u8) {
    crate::window_edge::set_colour(&app, r, g, b);
}

/// The page is alive (Round 15): every 2 s, and at once when it is hidden or shown. The
/// watchdog reloads a shown page that stops beating (`watchdog::ui_frozen`).
#[tauri::command(async)]
pub fn ui_heartbeat(hidden: bool) {
    crate::watchdog::ui_beat(hidden);
}

/// How long each of the page's start gates (`settings`, `library`, `resume`) took, from the
/// page's own start, so a slow start after a reload names its cause in the log (Round 21).
#[tauri::command(async)]
pub fn log_boot_gate(gate: String, ms: u64) {
    tracing::info!(gate = %gate, ms, "boot gate open");
}

/// Develop has a save waiting (`true`) or has written it (`false`), Round 5. While held, a
/// close first asks the page to write (`window::unsaved_close`).
#[tauri::command(async)]
pub fn hold_close(state: State<'_, AppState>, hold: bool) -> AppResult<()> {
    use std::sync::atomic::Ordering;
    state.unsaved_edits.store(hold, Ordering::SeqCst);
    if !hold {
        *state.flush_asked_at.lock().expect("flush lock") = None;
    }
    Ok(())
}

/// *Restart Lumen* (Round 15). The one command that is not `async`: it runs on the main
/// thread, which a starved async pool (what it may be rescuing) cannot hold up, and does
/// nothing there but start the `lumen-restart` thread. The page hears back
/// (`app://restart`) only when Lumen stays; otherwise this process ends and a new one opens
/// on `location`.
#[tauri::command]
pub fn restart_lumen(app: tauri::AppHandle, location: String) -> AppResult<()> {
    use tauri::Emitter;
    std::thread::Builder::new().name("lumen-restart".into()).spawn(move || {
        let step = crate::restart::run(&app, location);
        let _ = app.emit(crate::events::RESTART, step);
    })?;
    Ok(())
}

/// Where *Restart Lumen* left off, once (Round 15): the page opens there at boot.
#[tauri::command(async)]
pub fn take_resume(state: State<'_, AppState>) -> Option<String> {
    crate::restart::take_resume(&state.settings, &state.paths.settings_file())
}

/// *Stop import and close*: cancel every running import, give each up to five seconds to
/// finish the file it is on (the journal makes an interrupted one safe either way), then close.
#[tauri::command]
pub async fn confirm_close(app: tauri::AppHandle) -> AppResult<()> {
    use std::sync::atomic::Ordering;
    use tauri::Manager;
    let state = app.state::<AppState>();
    for t in state.imports.lock().expect("imports lock").values() {
        t.cancel();
    }
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while !state.imports.lock().expect("imports lock").is_empty() && std::time::Instant::now() < deadline {
        // An async command runs off the main thread, so this wait never blocks the window.
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    state.close_confirmed.store(true, Ordering::SeqCst);
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.close();
    }
    Ok(())
}
