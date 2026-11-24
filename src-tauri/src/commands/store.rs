//! The plugin store's commands (Round 24 lane C, spec §7): thin adapters over
//! `crate::plugins::store`. Everything is async, so a slow network never holds the main thread.

use std::sync::Arc;

use tauri::{AppHandle, State};

use crate::error::AppResult;
use crate::plugins::store::{StoreDownload, StoreIndex, StoreService, StoreStatus};
use crate::plugins::Plugins;

/// The store's list: fetched and cached, or the cache when offline (`store_status().offline`).
#[tauri::command]
pub async fn store_index(store: State<'_, Arc<StoreService>>, plugins: State<'_, Arc<Plugins>>) -> AppResult<StoreIndex> {
    let (store, plugins) = (store.inner().clone(), plugins.inner().clone());
    Ok(store.index(&plugins).await?)
}

/// *Get*: downloads and verifies the package; the page opens the install sheet on `path`.
#[tauri::command]
pub async fn store_install(
    store: State<'_, Arc<StoreService>>,
    plugins: State<'_, Arc<Plugins>>,
    id: String,
) -> AppResult<StoreDownload> {
    let (store, plugins) = (store.inner().clone(), plugins.inner().clone());
    Ok(store.install(&plugins, &id).await?)
}

/// *Update*: installs at once when nothing new is asked for, else returns the package for the sheet.
#[tauri::command]
pub async fn store_update(
    app: AppHandle,
    store: State<'_, Arc<StoreService>>,
    plugins: State<'_, Arc<Plugins>>,
    id: String,
) -> AppResult<StoreDownload> {
    let (store, plugins) = (store.inner().clone(), plugins.inner().clone());
    Ok(store.update(&plugins, &app, &id).await?)
}

/// A store plugin's README text (Markdown), or `None` when the index lists none. Offline it is the
/// copy kept the last time; the page never loads the URL.
#[tauri::command]
pub async fn store_readme(
    store: State<'_, Arc<StoreService>>,
    plugins: State<'_, Arc<Plugins>>,
    id: String,
) -> AppResult<Option<String>> {
    let (store, plugins) = (store.inner().clone(), plugins.inner().clone());
    Ok(store.readme(&plugins, &id).await?)
}

#[tauri::command(async)]
pub fn store_status(store: State<'_, Arc<StoreService>>, plugins: State<'_, Arc<Plugins>>) -> StoreStatus {
    store.status(&plugins)
}

/// *Update plugins automatically*.
#[tauri::command(async)]
pub fn store_set_auto_update(store: State<'_, Arc<StoreService>>, on: bool) -> AppResult<()> {
    Ok(store.set_auto_update(on)?)
}

/// Developer mode's local store folder (Round 26 dev store), or `None` for the online store.
#[tauri::command(async)]
pub fn store_local_folder(store: State<'_, Arc<StoreService>>) -> Option<String> {
    store.local_store().map(|p| p.display().to_string())
}

/// *Use a local store folder…* (a folder holding `index.json`) or, with `None`, the online store.
/// Read only while Developer mode is on.
#[tauri::command(async)]
pub fn store_set_local_folder(store: State<'_, Arc<StoreService>>, folder: Option<std::path::PathBuf>) -> AppResult<()> {
    Ok(store.set_local_store(folder)?)
}
