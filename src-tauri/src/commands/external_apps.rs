//! Thin adapters over `external_apps` (Round 22 lane C, spec §7): which apps are installed, the
//! person's own apps, the default app per kind of media and what is sent to an app.

use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

use photos_jobs::Lane;
use serde::Serialize;
use tauri::{AppHandle, State};
use ts_rs::TS;

use crate::error::{AppError, AppResult};
use crate::external_edits;
use crate::external_apps::{self as apps, custom_app_for, DefaultApps, ExternalApp, Media, SendMode};
use crate::state::AppState;

fn not_an_app(path: &Path) -> AppError {
    AppError {
        code: "not_an_app".into(),
        message: "That isn't an app Lumen can open photos with.".into(),
        detail: Some(path.display().to_string()),
    }
}

/// A file (an executable) or, on a Mac, a `.app` bundle.
fn is_app(path: &Path) -> bool {
    path.is_file() || (path.is_dir() && path.extension().is_some_and(|e| e.eq_ignore_ascii_case("app")))
}

/// The installed apps that take `media` (all of them when `None`).
#[tauri::command(async)]
pub fn list_external_apps(state: State<'_, AppState>, media: Option<Media>) -> AppResult<Vec<ExternalApp>> {
    let settings = state.settings.read().expect("settings lock").external_apps.clone();
    Ok(apps::apps_for(&settings, media))
}

#[tauri::command(async)]
pub fn add_custom_app(state: State<'_, AppState>, path: String) -> AppResult<ExternalApp> {
    let path = Path::new(&path);
    if !is_app(path) {
        return Err(not_an_app(path));
    }
    let entry = custom_app_for(path);
    let external = {
        let mut s = state.settings.write().expect("settings lock");
        if !s.external_apps.custom.iter().any(|c| c.id == entry.id) {
            s.external_apps.custom.push(entry.clone());
        }
        apps::to_external(&apps::detected(&s.external_apps.custom).into_iter().find(|a| a.id == entry.id).ok_or_else(|| not_an_app(path))?, &s.external_apps)
    };
    state.save_settings()?;
    Ok(external)
}

/// Forgets a custom app, and any default or send choice that named it.
#[tauri::command(async)]
pub fn remove_custom_app(state: State<'_, AppState>, id: String) -> AppResult<()> {
    {
        let mut s = state.settings.write().expect("settings lock");
        let e = &mut s.external_apps;
        e.custom.retain(|c| c.id != id);
        e.send.remove(&id);
        for slot in [&mut e.defaults.photo, &mut e.defaults.raw, &mut e.defaults.video] {
            if slot.as_deref() == Some(id.as_str()) {
                *slot = None;
            }
        }
    }
    state.save_settings()
}

#[tauri::command(async)]
pub fn get_default_apps(state: State<'_, AppState>) -> AppResult<DefaultApps> {
    Ok(state.settings.read().expect("settings lock").external_apps.defaults.clone())
}

#[tauri::command(async)]
pub fn set_default_app(state: State<'_, AppState>, media: Media, id: Option<String>) -> AppResult<()> {
    {
        let mut s = state.settings.write().expect("settings lock");
        let d = &mut s.external_apps.defaults;
        match media {
            Media::Photo => d.photo = id,
            Media::Raw => d.raw = id,
            Media::Video => d.video = id,
        }
    }
    state.save_settings()
}

/// What is sent to one app; `Automatic` is the absence of a choice.
#[tauri::command(async)]
pub fn set_app_send_mode(state: State<'_, AppState>, id: String, mode: SendMode) -> AppResult<()> {
    {
        let mut s = state.settings.write().expect("settings lock");
        if mode == SendMode::Automatic {
            s.external_apps.send.remove(&id);
        } else {
            s.external_apps.send.insert(id, mode);
        }
    }
    state.save_settings()
}

/// What *Edit in* / *Open with* did: files handed to the app, and how many were new copies.
#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct OpenedInApp {
    pub opened: u32,
    pub made: u32,
}

fn app_missing(name: &str) -> AppError {
    AppError { code: "app_missing".into(), message: format!("{name} isn't installed any more"), detail: None }
}

/// Opens photos in an app (spec §8): copies are prepared on the Import lane (the activity ring
/// shows them), then the app is started once with every file. A gone app is `app_missing` and
/// the cached list is dropped, so the next menu no longer offers it.
#[tauri::command(async)]
pub fn open_in_app(app: AppHandle, state: State<'_, AppState>, asset_ids: Vec<i64>, app_id: String) -> AppResult<OpenedInApp> {
    static RUNS: AtomicU64 = AtomicU64::new(0);
    let settings = state.settings.read().expect("settings lock").external_apps.clone();
    let target = apps::detected(&settings.custom).into_iter().find(|a| a.id == app_id);
    let Some(target) = target.filter(|a| apps::is_store(&a.target) || a.target.exists()) else {
        apps::forget_detected();
        let name = apps::detected(&settings.custom).into_iter().find(|a| a.id == app_id).map_or(app_id.clone(), |a| a.name);
        return Err(app_missing(&name));
    };
    let mode = settings.send.get(&app_id).copied().unwrap_or_default();
    let lib = state.library()?;
    let (tx, rx) = std::sync::mpsc::channel();
    let (app2, lib2) = (app.clone(), lib.clone());
    let key = format!("open-with:{app_id}:{}", RUNS.fetch_add(1, Ordering::Relaxed));
    state.jobs.submit(&key, Lane::Import, move |_| {
        let render = |id: i64, out: &Path| crate::derivatives::render_tiff(&app2, &lib2, id, out);
        let _ = tx.send(external_edits::open_with(&lib2, &target, mode, &asset_ids, &render, &apps::SystemLauncher));
    });
    let opened = rx.recv().map_err(|_| AppError { code: "cancelled".into(), message: "Opening was stopped.".into(), detail: None })??;
    for &id in &opened.new_assets {
        if let Ok(info) = lib.db.with_conn(|c| photos_core::assets::render_info(c, id)) {
            let kind = photos_media::kind_for_extension(&info.extension);
            crate::derivatives::schedule_ingest(app.clone(), crate::derivatives::ingested_from(&lib, id, &info, kind));
        }
    }
    if !opened.new_assets.is_empty() {
        state.changes.changed(&app, true);
    }
    Ok(OpenedInApp { opened: opened.paths.len() as u32, made: opened.made })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_an_existing_file_or_app_bundle_is_an_app() {
        let dir = tempfile::tempdir().unwrap();
        let exe = dir.path().join("editor.exe");
        std::fs::write(&exe, b"x").unwrap();
        let bundle = dir.path().join("Editor.app");
        std::fs::create_dir(&bundle).unwrap();
        assert!(is_app(&exe) && is_app(&bundle));
        assert!(!is_app(&dir.path().join("missing.exe")));
        assert!(!is_app(dir.path()), "a plain folder is not an app");
        assert_eq!(not_an_app(&exe).code, "not_an_app");
    }
}
