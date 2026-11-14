//! Import through a plugin (Round 23 lane A3, spec §10): the Import page's cards, and a start
//! that runs the normal import flow (batch, progress events, history) over a `PluginSource`.

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use photos_plugins::{PluginError, PluginImporter, PluginPermission};
use tauri::{AppHandle, State};

use crate::commands::import::{running_import_for, spawn_import_watched, Watched};
use crate::error::{AppError, AppResult};
use crate::plugins::importer::{check_chosen, PluginSource};
use crate::plugins::Plugins;
use crate::state::AppState;

/// The enabled, ready plugins that import, as the page's cards.
pub fn importers(p: &Plugins) -> Vec<PluginImporter> {
    p.read()
        .enabled()
        .into_iter()
        // A plugin not granted reading what is chosen could only fail (a hand-edited plugins.json).
        .filter(|i| i.granted.contains(&PluginPermission::ImportReadChosen))
        .filter_map(|i| {
            let d = i.manifest.importer.as_ref()?;
            Some(PluginImporter { plugin: i.info.id.clone(), label: d.label.clone(), pick: d.pick, extensions: d.extensions.clone(), hint: d.hint.clone() })
        })
        .collect()
}

/// The source for one import: the plugin must be on, importing, and granted the right to read
/// what was chosen (a hand-edited `plugins.json` can grant nothing); what was chosen must be
/// what the importer asked for and nothing to do with a library.
pub fn prepare(p: &Plugins, libraries: &[PathBuf], plugin: &str, chosen: &Path, cancel: Arc<AtomicBool>) -> Result<PluginSource, PluginError> {
    let decl = {
        let store = p.read();
        let inst = store.get(plugin).ok_or_else(|| PluginError::invalid(format!("No plugin {plugin}")))?;
        if !inst.granted.contains(&PluginPermission::ImportReadChosen) {
            return Err(PluginError::permission(format!("{} isn't allowed to read what you choose.", inst.info.name)));
        }
        inst.manifest.importer.clone().ok_or_else(|| PluginError::invalid(format!("{} isn't an importer", inst.info.name)))?
    };
    let root = check_chosen(chosen, &decl, libraries)?;
    PluginSource::new(p, plugin, root, cancel)
}

/// Every library folder this install knows, the open one included.
fn library_folders(state: &AppState) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = state.registry.read().expect("registry lock").libraries.iter().map(|l| PathBuf::from(&l.path)).collect();
    if let Ok(lib) = state.library() {
        out.push(lib.root.clone());
    }
    out
}

#[tauri::command(async)]
pub fn list_plugin_importers(plugins: State<'_, Arc<Plugins>>) -> AppResult<Vec<PluginImporter>> {
    Ok(importers(&plugins))
}

/// Starts an import of `chosen` through `plugin` and returns the batch id at once; progress and
/// completion are the other imports' events. Cancel, turning the plugin off and the plugin's
/// first trap all end it, keeping what already came in.
#[tauri::command(async)]
pub fn start_plugin_import(app: AppHandle, state: State<'_, AppState>, plugins: State<'_, Arc<Plugins>>, plugin: String, chosen: String) -> AppResult<i64> {
    let key = running_key(&plugin, Path::new(&chosen));
    if let Some(running) = running_import_for(&state, &key) {
        return Err(AppError {
            code: "already_importing".into(),
            message: "That is already being imported. Wait for it to finish.".into(),
            detail: Some(running.to_string()),
        });
    }
    let cancel = Arc::new(AtomicBool::new(false));
    let source = photos_jobs::interactive(|| prepare(&plugins, &library_folders(&state), &plugin, Path::new(&chosen), cancel.clone()))?;
    spawn_import_watched(&app, &state, Box::new(source), Some(chosen), None, None, Some(Watched { cancel, key }))
}

/// The running-import key of `plugin` over `chosen`: the same folder through the same plugin
/// is imported once at a time (two would race on the duplicate check).
fn running_key(plugin: &str, chosen: &Path) -> String {
    let canon = std::fs::canonicalize(chosen).unwrap_or_else(|_| chosen.to_path_buf());
    let path = canon.to_string_lossy();
    #[cfg(windows)]
    let path = path.to_lowercase();
    format!("plugin:{plugin}:{path}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    const ID: &str = "com.test.fixture";
    const WASM: &[u8] = include_bytes!("../../crates/photos-plugins/tests/wasm/importer_ok.wasm");
    const MANIFEST: &str = r#"id = "com.test.fixture"
name = "Fixture importer plugin"
version = "1.0.0"
author = "Tester"
description = "A test importer."
license = "MIT"
lumen = ">=26.0.0"
api = 1
permissions = ["import.read-chosen"]

[importer]
label = "Fixture importer"
pick = "folder"
"#;

    /// The fixture as a Developer-mode plugin in a temp store.
    fn rig() -> (tempfile::TempDir, Arc<Plugins>) {
        let dir = tempfile::tempdir().unwrap();
        let app = dir.path().join("app");
        let plugins = Arc::new(Plugins::load(&app));
        let folder = dir.path().join("plugin");
        fs::create_dir_all(&folder).unwrap();
        fs::write(folder.join("plugin.toml"), MANIFEST).unwrap();
        fs::write(folder.join("plugin.wasm"), WASM).unwrap();
        {
            let mut store = plugins.write();
            store.set_developer_mode(true).unwrap();
            store.load_dev(&folder, &plugins.ctx()).unwrap();
        }
        (dir, plugins)
    }

    #[test]
    fn lists_enabled_importers_only() {
        let (_d, p) = rig();
        let got = importers(&p);
        assert_eq!(got.len(), 1);
        assert_eq!((got[0].plugin.as_str(), got[0].label.as_str()), (ID, "Fixture importer"));
        p.write().set_enabled(ID, false).unwrap();
        assert!(importers(&p).is_empty(), "a plugin that is off has no card");
    }

    #[test]
    fn start_refuses_library_folder() {
        let (d, p) = rig();
        let lib = d.path().join("lib");
        fs::create_dir_all(lib.join("sub")).unwrap();
        for chosen in [lib.join("sub"), lib.clone(), d.path().to_path_buf()] {
            let e = prepare(&p, std::slice::from_ref(&lib), ID, &chosen, Arc::new(AtomicBool::new(false))).err().expect("refused");
            assert_eq!(e.code, "plugin_chosen_root", "{chosen:?}");
        }
        let other = d.path().join("export");
        fs::create_dir_all(&other).unwrap();
        assert!(prepare(&p, &[lib], ID, &other, Arc::new(AtomicBool::new(false))).is_ok());
    }

    #[test]
    fn start_without_permission_refused() {
        let (d, p) = rig();
        // A hand-edited plugins.json that grants nothing, read on the next start.
        let file = d.path().join("app").join("plugins").join("plugins.json");
        let mut json: serde_json::Value = serde_json::from_slice(&fs::read(&file).unwrap()).unwrap();
        json["plugins"][ID]["granted"] = serde_json::json!([]);
        fs::write(&file, serde_json::to_vec(&json).unwrap()).unwrap();
        let p = {
            drop(p);
            Arc::new(Plugins::load(&d.path().join("app")))
        };
        let export = d.path().join("export");
        fs::create_dir_all(&export).unwrap();
        let e = prepare(&p, &[], ID, &export, Arc::new(AtomicBool::new(false))).err().expect("refused");
        assert_eq!(e.code, "plugin_permission");
        assert!(importers(&p).is_empty(), "no card for an importer that may not read what is chosen");
    }

    #[test]
    fn running_key_names_the_folder_once() {
        let d = tempfile::tempdir().unwrap();
        let export = d.path().join("export");
        fs::create_dir_all(export.join("sub")).unwrap();
        assert_eq!(running_key(ID, &export), running_key(ID, &export.join("sub").join("..")));
        assert_ne!(running_key(ID, &export), running_key("com.other", &export));
    }
}
