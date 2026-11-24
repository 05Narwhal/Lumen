//! Themes for the page (Round 26 goal 4: `.lumentheme`, apart from plugins). Thin adapters over
//! `plugins::themes`. `plugin_theme_css` fills the page's theme stylesheet; `themes_list`, `theme_install`, `theme_remove`, `theme_try` / `theme_try_end` /
//! `theme_keep` and `theme_apply` are the interface Lane C and B2 use.

use std::path::PathBuf;
use std::sync::Arc;

use photos_core::ThemeChoice;
use photos_plugins::theme_pkg::{InstalledTheme, THEME_SLOT};
use tauri::{AppHandle, Emitter, State};

use crate::error::{AppError, AppResult};
use crate::plugins::store::StoreService;
use crate::plugins::themes::Themes;
use crate::plugins::Plugins;
use crate::state::AppState;

/// Sent with the new `Settings` whenever Rust changes a setting the page shows (Round 26:
/// `theme_apply`). The page's own `update_settings` doesn't send it.
pub const SETTINGS_CHANGED_EVENT: &str = "app://settings-changed";

/// The CSS for every installed theme and the one being tried (empty when there are none).
#[tauri::command(async)]
pub fn plugin_theme_css(themes: State<'_, Arc<Themes>>) -> AppResult<String> {
    Ok(themes.css())
}

/// The installed themes, for Settings → Appearance and the store.
#[tauri::command(async)]
pub fn themes_list(themes: State<'_, Arc<Themes>>) -> AppResult<Vec<InstalledTheme>> {
    Ok(themes.list())
}

/// The theme being tried, if any: it is not in `themes_list`, but the page draws it.
#[tauri::command(async)]
pub fn theme_tried(themes: State<'_, Arc<Themes>>) -> AppResult<Option<InstalledTheme>> {
    Ok(themes.tried())
}

/// What the install sheet shows for a `.lumentheme` the person opened: the theme and, when it
/// can't be installed, why. Nothing is installed.
#[derive(serde::Serialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ThemeInspect {
    pub theme: InstalledTheme,
    pub problem: Option<String>,
}

#[tauri::command(async)]
pub async fn theme_inspect(
    themes: State<'_, Arc<Themes>>,
    plugins: State<'_, Arc<Plugins>>,
    path: PathBuf,
) -> AppResult<ThemeInspect> {
    let dev = plugins.read().developer_mode();
    let t = themes.inner().clone();
    let (theme, problem) = tauri::async_runtime::spawn_blocking(move || t.inspect(&path, dev))
        .await
        .map_err(|e| err("internal", e.to_string()))??;
    Ok(ThemeInspect { theme, problem })
}

/// Installs (or updates) a `.lumentheme`: a file the person chose (`path`), or the store's
/// package (`store_id`, downloaded and checked against the index's blake3 first). Developer mode
/// lets a contrast failure through.
#[tauri::command(async)]
pub async fn theme_install(
    app: AppHandle,
    themes: State<'_, Arc<Themes>>,
    plugins: State<'_, Arc<Plugins>>,
    store: State<'_, Arc<StoreService>>,
    path: Option<PathBuf>,
    store_id: Option<String>,
) -> AppResult<InstalledTheme> {
    let dev = plugins.read().developer_mode();
    let (file, downloaded) = match (path, store_id) {
        (Some(p), None) => (p, false),
        (None, Some(id)) => (store.fetch_theme(&plugins, &id).await?.0, true),
        _ => return Err(err("invalid_input", "Pass a file or a store id")),
    };
    let t = themes.inner().clone();
    let f = file.clone();
    let r = tauri::async_runtime::spawn_blocking(move || t.install(&f, dev)).await;
    if downloaded {
        let _ = std::fs::remove_file(&file);
    }
    let meta = r.map_err(|e| err("internal", e.to_string()))??;
    plugins.changed(&app);
    Ok(meta)
}

/// Removes an installed theme (its folder under app data).
#[tauri::command(async)]
pub fn theme_remove(app: AppHandle, themes: State<'_, Arc<Themes>>, plugins: State<'_, Arc<Plugins>>, id: String) -> AppResult<()> {
    themes.remove(&id)?;
    plugins.changed(&app);
    Ok(())
}

/// *Try*: the store's theme downloaded, checked against the index's blake3, validated like an
/// install and shown from `cache/theme-try/`. Nothing is installed; a relaunch ends it.
#[tauri::command(async)]
pub async fn theme_try(
    app: AppHandle,
    themes: State<'_, Arc<Themes>>,
    plugins: State<'_, Arc<Plugins>>,
    store: State<'_, Arc<StoreService>>,
    store_id: String,
) -> AppResult<InstalledTheme> {
    let (file, hash) = store.fetch_theme(&plugins, &store_id).await?;
    let t = themes.inner().clone();
    let f = file.clone();
    let r = tauri::async_runtime::spawn_blocking(move || t.try_file(&f, &hash)).await;
    let _ = std::fs::remove_file(&file);
    let meta = r.map_err(|e| err("internal", e.to_string()))??;
    plugins.changed(&app);
    Ok(meta)
}

/// Ends *Try* (the page puts the previous theme back).
#[tauri::command(async)]
pub fn theme_try_end(app: AppHandle, themes: State<'_, Arc<Themes>>, plugins: State<'_, Arc<Plugins>>) -> AppResult<()> {
    themes.try_end();
    plugins.changed(&app);
    Ok(())
}

/// *Keep* after *Try*: installs the theme being tried.
#[tauri::command(async)]
pub fn theme_keep(app: AppHandle, themes: State<'_, Arc<Themes>>, plugins: State<'_, Arc<Plugins>>) -> AppResult<InstalledTheme> {
    let meta = themes.keep()?;
    plugins.changed(&app);
    Ok(meta)
}

/// Makes an installed (or the tried) theme the app's theme: `appearance.theme = {plugin: id,
/// theme: "theme"}`, saved, then the settings and plugins change events.
#[tauri::command(async)]
pub fn theme_apply(
    app: AppHandle,
    state: State<'_, AppState>,
    themes: State<'_, Arc<Themes>>,
    plugins: State<'_, Arc<Plugins>>,
    id: String,
) -> AppResult<()> {
    if !themes.is_available(&id) {
        return Err(err("not_found", "That theme isn't installed"));
    }
    let settings = {
        let mut s = state.settings.write().expect("settings lock");
        s.appearance.theme = ThemeChoice::Plugin { plugin: id, theme: THEME_SLOT.into() };
        s.clone()
    };
    state.save_settings()?;
    let _ = app.emit(SETTINGS_CHANGED_EVENT, &settings);
    plugins.changed(&app);
    Ok(())
}

fn err(code: &str, message: impl Into<String>) -> AppError {
    AppError { code: code.into(), message: message.into(), detail: None }
}
