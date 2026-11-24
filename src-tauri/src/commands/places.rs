//! The Place review's commands (Round 24 §8.6): thin adapters over `plugins::places`. A plugin
//! suggests, the person confirms, *Write* puts the places in the library (and, with
//! `originals.gps`, the original files) as a background job with one undo.

use std::sync::Arc;

use photos_plugins::PluginPermission;
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};
use ts_rs::TS;

use crate::error::{AppError, AppResult};
use crate::originals_gps::GpsWriter;
use crate::plugins::mcp_tools::{instance, plugin_error};
use crate::plugins::places::{self, FileGps, ReviewAction, ReviewState, UndoReport, WriteReport};
use crate::plugins::Plugins;
use crate::state::AppState;

/// Sent when a write job ends (done or cancelled).
pub const WRITE_FINISHED: &str = "places://write-finished";

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct WriteFinished {
    pub plugin_id: String,
    pub report: WriteReport,
}

fn err(code: &str, message: impl Into<String>) -> AppError {
    AppError { code: code.into(), message: message.into(), detail: None }
}

/// The plugin's grants, or `plugin_missing`.
fn granted(plugins: &Plugins, id: &str) -> AppResult<Vec<PluginPermission>> {
    plugins.read().get(id).map(|i| i.granted.clone()).ok_or_else(|| err("plugin_missing", format!("{id} isn't installed")))
}

#[tauri::command(async)]
pub fn places_review(plugins: State<'_, Arc<Plugins>>, plugin_id: String) -> AppResult<ReviewState> {
    Ok(plugins.reviews().state(&plugin_id))
}

/// One action on the review screen: applied here, forwarded to the plugin's `on-review` (which
/// runs its wasm, so this is an `async` command) and merged with what it changed.
#[tauri::command(async)]
pub fn places_review_event(plugins: State<'_, Arc<Plugins>>, state: State<'_, AppState>, plugin_id: String, event: ReviewAction) -> AppResult<ReviewState> {
    if !granted(&plugins, &plugin_id)?.contains(&PluginPermission::PlacesWrite) {
        return Err(err("permission_denied", "This plugin hasn't been given places.write"));
    }
    let dir = plugins.read().get(&plugin_id).map(|i| i.dir.clone());
    let shared = state.mcp.shared.clone();
    let p: Arc<Plugins> = plugins.inner().clone();
    plugins
        .reviews()
        .event(&plugin_id, &event, |e| {
            let Some(dir) = dir else { return Ok(Vec::new()) };
            let inst = instance(&p, &shared, &plugin_id, &dir).map_err(|e| e.message)?;
            if !inst.has_review() {
                return Ok(Vec::new());
            }
            inst.on_review(e).map_err(|e| plugin_error(e).message)
        })
        .map_err(|m| err("places_review", m))
}

/// The writer for originals: only for a plugin with `originals.gps`, and only where ExifTool is.
fn file_writer(state: &AppState, grants: &[PluginPermission]) -> Option<GpsWriter> {
    if !grants.contains(&PluginPermission::OriginalsGps) {
        return None;
    }
    let exiftool = state.exiftool()?;
    GpsWriter::new(&exiftool, state.ffmpeg()).map_err(|e| tracing::warn!("originals.gps: no writer, places stay in Lumen only: {}", e.message)).ok()
}

/// A plugin's *Write locations into original files* switch (`write_originals`): off means places
/// go into the library only. A plugin without that setting has no switch, so the grant decides.
fn originals_switched_on(setting: Option<serde_json::Value>) -> bool {
    setting.and_then(|v| v.as_bool()).unwrap_or(true)
}

/// *Write N locations*: starts the job and returns its activity id. The result arrives as
/// [`WRITE_FINISHED`]; one undo (`places_undo`) takes back the whole write.
#[tauri::command(async)]
pub fn places_write(app: AppHandle, plugins: State<'_, Arc<Plugins>>, state: State<'_, AppState>, plugin_id: String) -> AppResult<String> {
    let grants = granted(&plugins, &plugin_id)?;
    if !grants.contains(&PluginPermission::PlacesWrite) {
        return Err(err("permission_denied", "This plugin hasn't been given places.write"));
    }
    let lib = state.library()?;
    let store = plugins.reviews().clone();
    let cancel = store.begin_write(&plugin_id).ok_or_else(|| err("places_busy", "A write for this plugin is already running"))?;
    let todo = places::pending_writes(&store, &plugin_id);
    let job = format!("places-write-{plugin_id}");
    let switched_on = originals_switched_on(plugins.read().setting(&plugin_id, "write_originals"));
    let mut writer = if switched_on { file_writer(&state, &grants) } else { None };
    let (job_id, id) = (job.clone(), plugin_id.clone());
    let spawned = std::thread::Builder::new().name("lumen-places-write".into()).spawn(move || {
        let ring = photos_jobs::activity::start(&job_id, "Writing places", todo.len() as u64);
        let report = places::write_job(&lib, &id, &todo, writer.as_mut().map(|w| w as &mut dyn FileGps), &cancel, |n| {
            if n > 0 {
                ring.advance(1)
            }
        });
        drop(ring);
        store.end_write(&id);
        match report {
            Ok(report) => {
                let done: Vec<_> = todo.iter().copied().filter(|p| report.written.contains(&p.0)).collect();
                places::forget_written(&store, &id, &done);
                let _ = app.emit(WRITE_FINISHED, WriteFinished { plugin_id: id, report });
                app.state::<AppState>().changes.changed(&app, true);
            }
            Err(e) => tracing::warn!(plugin = %id, "writing places failed: {}", e.message),
        }
    });
    if let Err(e) = spawned {
        plugins.reviews().end_write(&plugin_id);
        return Err(err("places_write", e.to_string()));
    }
    Ok(job)
}

/// Stops a running write between files; what was written stays and can be undone.
#[tauri::command(async)]
pub fn places_write_cancel(plugins: State<'_, Arc<Plugins>>, plugin_id: String) -> AppResult<()> {
    plugins.reviews().cancel_write(&plugin_id);
    Ok(())
}

/// Takes back one whole write: library rows revert; files lose the GPS Lumen wrote unless they
/// changed since (listed in the report).
#[tauri::command(async)]
pub fn places_undo(app: AppHandle, plugins: State<'_, Arc<Plugins>>, state: State<'_, AppState>, write_id: i64) -> AppResult<UndoReport> {
    let lib = state.library()?;
    let plugin: Option<String> = lib.db.with_conn(|c| Ok(c.query_row("SELECT plugin_id FROM place_writes WHERE id = ?1", [write_id], |r| r.get(0)).ok()))?;
    let plugin = plugin.ok_or_else(|| err("not_found", "That write was already undone"))?;
    // The plugin may be gone by now; then there are no grants and the files are left as they are.
    let grants = granted(&plugins, &plugin).unwrap_or_default();
    let mut writer = file_writer(&state, &grants);
    let report = places::undo_write(&lib, write_id, writer.as_mut().map(|w| w as &mut dyn FileGps))?;
    state.changes.changed(&app, true);
    Ok(report)
}

// ------------------------------------------------------------------------------------ tools

/// A plugin tool's answer as JSON: the text parsed when it is JSON (Wayfinder's is), else a string.
fn tool_answer(text: String) -> serde_json::Value {
    serde_json::from_str(&text).unwrap_or(serde_json::Value::String(text))
}

/// One call of a tool the person asked for from the app (*Find locations…*), through the plugin's
/// shared instance (30 s per call). Unlike the MCP path this does not need AI assistants on. A
/// resumable tool answers `{"status":"working",…}`; the page calls again until it says done.
#[tauri::command(async)]
pub fn plugin_tool_run(plugins: State<'_, Arc<Plugins>>, state: State<'_, AppState>, plugin_id: String, tool: String, args: serde_json::Value) -> AppResult<serde_json::Value> {
    let dir = {
        let store = plugins.read();
        let inst = store.enabled().into_iter().find(|i| i.manifest.id == plugin_id).ok_or_else(|| err("plugin_missing", format!("{plugin_id} isn't installed or isn't turned on")))?;
        if !inst.manifest.tools.iter().any(|t| t.name == tool) {
            return Err(err("tool_missing", format!("{plugin_id} has no tool {tool}")));
        }
        inst.dir.clone()
    };
    let shared = state.mcp.shared.clone();
    let p: Arc<Plugins> = plugins.inner().clone();
    // A grid's or the viewer's selection is the selection of this call only (`library.query`).
    let run = || instance(&p, &shared, &plugin_id, &dir)?.call(&tool, &args.to_string()).map_err(plugin_error);
    let out = match crate::plugins::tool_selection::assets_of(&args) {
        Some(ids) => shared.tool_selection.scoped(ids, run),
        None => run(),
    }?;
    Ok(tool_answer(out))
}

// ------------------------------------------------------------------------------------ models

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PluginModelStatus {
    pub id: String,
    pub size: f64,
    pub licence: String,
    pub downloaded: bool,
}

fn declared(plugins: &Plugins, id: &str) -> AppResult<(Vec<photos_plugins::manifest::ModelDecl>, std::path::PathBuf)> {
    let store = plugins.read();
    let inst = store.get(id).ok_or_else(|| err("plugin_missing", format!("{id} isn't installed")))?;
    Ok((inst.manifest.models.clone(), store.data_dir(id)))
}

/// Which of a plugin's `[[models]]` are on disk. A missing model is a state, never an error.
#[tauri::command(async)]
pub fn plugin_models_status(plugins: State<'_, Arc<Plugins>>, plugin_id: String) -> AppResult<Vec<PluginModelStatus>> {
    let (decls, data) = declared(&plugins, &plugin_id)?;
    Ok(decls
        .iter()
        .map(|d| PluginModelStatus { id: d.id.clone(), size: d.size as f64, licence: d.licence.clone(), downloaded: crate::plugins::models::model_file(&data, d).is_file() })
        .collect())
}

/// Downloads the models the plugin declares and the person agreed to (the page asks first; the
/// install sheet names their size and licence). Each file is checked against its blake3 before
/// it is kept; progress is in the activity ring. An `async fn` that awaits the download: a sync
/// `command(async)` runs on a runtime worker, where `block_on` panics (and a release build aborts).
#[tauri::command]
pub async fn plugin_models_download(plugins: State<'_, Arc<Plugins>>, plugin_id: String) -> AppResult<Vec<PluginModelStatus>> {
    if !granted(&plugins, &plugin_id)?.contains(&PluginPermission::Models) {
        return Err(err("permission_denied", "This plugin hasn't been given models"));
    }
    let (decls, data) = declared(&plugins, &plugin_id)?;
    let total: u64 = decls.iter().map(|d| d.size).sum();
    let ring = photos_jobs::activity::start(&format!("plugin-models-{plugin_id}"), "Downloading a plugin's models", total.max(1));
    for d in &decls {
        let dest = crate::plugins::models::model_file(&data, d);
        let mut last = 0u64;
        let ring = &ring;
        crate::plugins::models::download_model(&d.url, &dest, d.size, &d.blake3, |done, _| {
            ring.advance(done - last);
            last = done;
        })
        .await
        .map_err(|m| err("model_download", m))?;
    }
    drop(ring);
    plugin_models_status(plugins, plugin_id)
}

#[cfg(test)]
mod tests {
    use super::{originals_switched_on, tool_answer};
    use serde_json::json;

    #[test]
    fn a_tool_answer_is_json_when_it_can_be() {
        assert_eq!(tool_answer(r#"{"status":"working","done":3}"#.into()), json!({"status": "working", "done": 3}));
        assert_eq!(tool_answer("plain words".into()), json!("plain words"));
    }

    #[test]
    fn write_originals_setting_decides() {
        assert!(!originals_switched_on(Some(json!(false))));
        assert!(originals_switched_on(Some(json!(true))));
        // No such setting (or a value of the wrong type): the grant alone decides, as before.
        assert!(originals_switched_on(None));
        assert!(originals_switched_on(Some(json!("no"))));
    }
}
