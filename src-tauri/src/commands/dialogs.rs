//! File dialogs and *Show in folder* (Round 24 lane D2, spec §8.1): one entry for every dialog
//! and every reveal in the page. A plugin's provider answers when one is active; its failure
//! (never its *cancelled*) falls back to the system, so a broken provider can't trap the person.

use std::path::Path;
use std::sync::Arc;

use photos_plugins::{PickKind, PickOutcome, PickRequest, ProviderHost};
use tauri::{AppHandle, Manager, State};
use tauri_plugin_dialog::DialogExt;

use crate::error::{AppError, AppResult};
use crate::plugins::detect::PluginDetect;
use crate::plugins::providers::PluginProviders;
use crate::plugins::Plugins;

/// The providers as of now. Cheap: it only holds handles; plugins and switches are read per call.
pub fn host(app: &AppHandle, plugins: &Arc<Plugins>) -> PluginProviders {
    let dir = photos_core::paths::AppPaths::resolve()
        .map(|p| p.app_data)
        .unwrap_or_else(|_| photos_core::paths::AppPaths::app_data_under(Path::new(".")))
        .join("plugins");
    let window = app.clone();
    let found = PluginDetect::new(plugins.clone());
    PluginProviders::new(plugins.clone(), dir, move || main_window_handle(&window))
        .with_lookup(move |plugin, d| found.exe(plugin, d))
}

/// The main window's handle for a provider's `{parent}` (Windows only).
fn main_window_handle(app: &AppHandle) -> Option<String> {
    #[cfg(windows)]
    {
        let win = app.get_webview_window("main")?;
        win.hwnd().ok().map(|h| (h.0 as isize).to_string())
    }
    #[cfg(not(windows))]
    {
        let _ = app;
        None
    }
}

fn as_string(p: tauri_plugin_dialog::FilePath) -> Option<String> {
    p.into_path().ok().map(|p| p.to_string_lossy().into_owned())
}

/// The system's own dialog; blocks the calling (non-main) thread until the person answers.
fn system_pick(app: &AppHandle, req: &PickRequest) -> Option<Vec<String>> {
    let mut d = app.dialog().file();
    if let Some(t) = &req.title {
        d = d.set_title(t);
    }
    if let Some(start) = req.start.as_deref().map(Path::new) {
        let dir = if start.is_dir() { Some(start) } else { start.parent().filter(|p| p.is_dir()) };
        if let Some(dir) = dir {
            d = d.set_directory(dir);
        }
    }
    if req.kind == PickKind::Save {
        if let Some(n) = &req.name {
            d = d.set_file_name(n);
        }
    }
    for f in &req.filters {
        let exts: Vec<&str> = f.extensions.iter().map(String::as_str).collect();
        d = d.add_filter(&f.label, &exts);
    }
    if let Some(w) = app.get_webview_window("main") {
        d = d.set_parent(&w);
    }
    match req.kind {
        PickKind::OpenFile => d.blocking_pick_file().and_then(as_string).map(|p| vec![p]),
        PickKind::OpenMultiple => d.blocking_pick_files().map(|v| v.into_iter().filter_map(as_string).collect()),
        PickKind::OpenFolder => d.blocking_pick_folder().and_then(as_string).map(|p| vec![p]),
        PickKind::Save => d.blocking_save_file().and_then(as_string).map(|p| vec![p]),
    }
}

/// `None` is *cancelled*; a single-path kind answers with one path.
#[tauri::command(async)]
pub fn dialog_pick(
    app: AppHandle,
    plugins: State<'_, Arc<Plugins>>,
    req: PickRequest,
) -> AppResult<Option<Vec<String>>> {
    match host(&app, plugins.inner()).pick(&req) {
        Some(Ok(PickOutcome::Picked { paths, .. })) => return Ok(Some(paths)),
        Some(Ok(PickOutcome::Cancelled)) => return Ok(None),
        Some(Err(why)) => tracing::warn!(%why, "a plugin's file picker failed; opening the system dialog"),
        None => {}
    }
    Ok(system_pick(&app, &req))
}

/// Shows `path` in the file manager: a plugin's reveal provider, else the system's.
pub fn reveal(app: &AppHandle, plugins: &Arc<Plugins>, path: &Path) -> AppResult<()> {
    match host(app, plugins).reveal(path) {
        Some(Ok(())) => return Ok(()),
        Some(Err(why)) => tracing::warn!(%why, "a plugin's reveal failed; using the system's"),
        None => {}
    }
    tauri_plugin_opener::reveal_item_in_dir(path).map_err(|e| AppError {
        code: "reveal_failed".into(),
        message: "Could not open the file manager.".into(),
        detail: Some(e.to_string()),
    })
}

#[tauri::command(async)]
pub fn reveal_path(app: AppHandle, plugins: State<'_, Arc<Plugins>>, path: String) -> AppResult<()> {
    reveal(&app, plugins.inner(), Path::new(&path))
}

/// One kind of provider and the plugins that could answer it (Settings → Plugins shows a choice
/// only when there are two or more).
#[derive(Debug, Clone, serde::Serialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ProviderChoice {
    /// `file-picker` or `reveal`.
    pub kind: String,
    pub options: Vec<ProviderOption>,
    /// The plugin id answering now.
    pub chosen: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ProviderOption {
    pub plugin_id: String,
    pub name: String,
}

#[tauri::command(async)]
pub fn plugin_provider_choices(app: AppHandle, plugins: State<'_, Arc<Plugins>>) -> Vec<ProviderChoice> {
    use crate::plugins::providers::{active, kind_key, Choice};
    use photos_plugins::manifest::ProviderKind;
    let host = host(&app, plugins.inner());
    let cands = host.candidates();
    let choice = Choice::load(&host.dir());
    let store = plugins.read();
    [ProviderKind::FilePicker, ProviderKind::Reveal]
        .into_iter()
        .map(|kind| {
            let options = cands
                .iter()
                .filter(|c| c.provider.kind == kind)
                .map(|c| ProviderOption {
                    plugin_id: c.plugin.clone(),
                    name: store.get(&c.plugin).map_or_else(|| c.plugin.clone(), |i| i.info.name.clone()),
                })
                .collect();
            ProviderChoice {
                kind: kind_key(kind).into(),
                options,
                chosen: active(&cands, kind, choice.get(kind)).map(|c| c.plugin.clone()),
            }
        })
        .collect()
}

#[tauri::command(async)]
pub fn set_provider_choice(app: AppHandle, plugins: State<'_, Arc<Plugins>>, kind: String, plugin_id: String) -> AppResult<()> {
    use crate::plugins::providers::Choice;
    use photos_plugins::manifest::ProviderKind;
    let kind = match kind.as_str() {
        "file-picker" => ProviderKind::FilePicker,
        "reveal" => ProviderKind::Reveal,
        _ => return Err(AppError { code: "bad_kind".into(), message: "Unknown provider kind.".into(), detail: Some(kind) }),
    };
    let dir = host(&app, plugins.inner()).dir();
    let mut choice = Choice::load(&dir);
    choice.set(kind, &plugin_id);
    choice.save(&dir).map_err(|e| AppError {
        code: "provider_choice_failed".into(),
        message: "Could not save the choice.".into(),
        detail: Some(e.to_string()),
    })
}

/// Where each app a plugin declares is on this machine (§8.2). Runs the version command, so it
/// is async and never on the UI thread.
#[tauri::command(async)]
pub fn plugin_detect(plugins: State<'_, Arc<Plugins>>, plugin_id: String) -> Vec<photos_plugins::DetectStatus> {
    use photos_plugins::DetectHost;
    PluginDetect::new(plugins.inner().clone()).detect(&plugin_id)
}

/// *Locate…*: keeps a person-chosen program for a detect entry when it exists and is named as declared.
#[tauri::command(async)]
pub fn plugin_locate(
    app: AppHandle,
    plugins: State<'_, Arc<Plugins>>,
    plugin_id: String,
    detect_id: String,
    exe: String,
) -> AppResult<photos_plugins::DetectStatus> {
    use photos_plugins::DetectHost;
    let status = PluginDetect::new(plugins.inner().clone())
        .locate(&plugin_id, &detect_id, Path::new(&exe))
        .map_err(|m| AppError { code: "locate_refused".into(), message: m, detail: None })?;
    plugins.changed(&app);
    Ok(status)
}
