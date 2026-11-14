//! Lightroom presets and catalogs (Round 11). Every file and folder here is a read-only source.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use photos_core::develop_presets::{self, Imported};
use photos_core::lightroom::{convert, read_settings};
use photos_core::seen::{self, SeenItem};
use photos_core::Db;
use photos_import::lightroom::{find_lightroom_catalogs as find_catalogs, CatalogRef, CatalogSummary, LightroomCatalogSource};
use photos_import::{FilteredSource, ImportSource};
use serde::Serialize;
use tauri::{AppHandle, Manager, State};
use ts_rs::TS;

use crate::commands::import::spawn_import;
use crate::error::{AppError, AppResult};
use crate::state::AppState;

#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PresetFailure {
    pub file: String,
    pub reason: String,
}

/// One imported preset's settings that Lumen could not take.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PresetNotes {
    pub name: String,
    pub settings: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PresetImport {
    pub imported: u32,
    pub skipped_same_name: u32,
    pub failed: Vec<PresetFailure>,
    pub not_converted: Vec<PresetNotes>,
}

fn is_preset_file(p: &Path) -> bool {
    p.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).is_some_and(|e| e == "xmp" || e == "lrtemplate")
}

/// The `.xmp` and `.lrtemplate` files among `paths`, folders searched all the way down.
fn preset_files(paths: &[PathBuf]) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack: Vec<PathBuf> = paths.to_vec();
    while let Some(p) = stack.pop() {
        if p.is_dir() {
            if let Ok(entries) = std::fs::read_dir(&p) {
                stack.extend(entries.flatten().map(|e| e.path()));
            }
        } else if is_preset_file(&p) {
            out.push(p);
        }
    }
    out.sort();
    out.dedup();
    out
}

/// Imports each preset file into `conn`'s presets. A file that is no preset (an XMP without
/// Camera Raw settings, or a sidecar written beside a photo) is passed over quietly.
fn import_files(db: &Db, files: &[PathBuf], now: i64) -> PresetImport {
    let mut out = PresetImport::default();
    for file in files {
        let shown = file.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let fail = |reason: String| PresetFailure { file: file.to_string_lossy().into_owned(), reason };
        let text = match std::fs::read(file) {
            Ok(bytes) => String::from_utf8_lossy(&bytes).into_owned(),
            Err(e) => {
                out.failed.push(fail(e.to_string()));
                continue;
            }
        };
        let settings = match read_settings(&shown, &text) {
            Ok(s) => s,
            Err(e) => {
                out.failed.push(fail(e.to_string()));
                continue;
            }
        };
        if settings.is_empty() || settings.contains_key("RawFileName") {
            continue;
        }
        let converted = convert(&settings, None);
        let stem = file.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        let base = converted.name.clone().unwrap_or(stem);
        let name = match &converted.group {
            Some(g) => format!("{g} / {base}"),
            None => base,
        };
        match db.with_conn(|c| develop_presets::import_converted(c, &name, &converted, now)) {
            Ok(Imported::Saved { notes, .. }) => {
                out.imported += 1;
                if !notes.is_empty() {
                    out.not_converted.push(PresetNotes { name, settings: notes });
                }
            }
            Ok(Imported::SameName) => out.skipped_same_name += 1,
            Ok(Imported::Nothing { .. }) => out.failed.push(fail("Nothing in it is something Lumen can set.".into())),
            Err(e) => out.failed.push(fail(e.to_string())),
        }
    }
    out
}

/// Lightroom presets (`.xmp`, `.lrtemplate`) from files or folders, as Lumen presets.
#[tauri::command(async)]
pub async fn import_lightroom_presets(state: State<'_, AppState>, paths: Vec<String>) -> AppResult<PresetImport> {
    let lib = state.library()?;
    let paths: Vec<PathBuf> = paths.into_iter().map(PathBuf::from).collect();
    tauri::async_runtime::spawn_blocking(move || {
        let files = preset_files(&paths);
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
        Ok(import_files(&lib.db, &files, now))
    })
    .await
    .map_err(|e| AppError { code: "internal".into(), message: "The import stopped unexpectedly.".into(), detail: Some(e.to_string()) })?
}

/// Camera Raw's settings folder, where Lightroom Classic and Camera Raw keep presets: under
/// `%APPDATA%` on Windows, `~/Library/Application Support` on macOS. Only when it exists.
pub fn camera_raw_folder(config_dir: Option<PathBuf>) -> Option<PathBuf> {
    if !(cfg!(windows) || cfg!(target_os = "macos")) {
        return None;
    }
    Some(config_dir?.join("Adobe").join("CameraRaw").join("Settings")).filter(|p| p.is_dir())
}

#[tauri::command(async)]
pub fn camera_raw_preset_folder() -> AppResult<Option<String>> {
    Ok(camera_raw_folder(dirs::config_dir()).map(|p| p.to_string_lossy().into_owned()))
}

/// The catalogs Lightroom's preferences list as recent, best effort (a failed read is none).
fn recent_catalogs() -> Vec<PathBuf> {
    let file = if cfg!(windows) {
        dirs::config_dir().map(|d| d.join("Adobe").join("Lightroom").join("Preferences").join("Lightroom Classic CC 7 Preferences.agprefs"))
    } else if cfg!(target_os = "macos") {
        dirs::home_dir().map(|h| h.join("Library").join("Preferences").join("com.adobe.LightroomClassicCC7.plist"))
    } else {
        None
    };
    file.and_then(|f| std::fs::read(f).ok()).map(|b| photos_import::lightroom::find::recent_in_preferences(&String::from_utf8_lossy(&b))).unwrap_or_default()
}

/// Lightroom Classic catalogs Lumen found by itself (Round 11): in Pictures (and its
/// `Lightroom` folder), home, the top of each drive, and Lightroom's recent list. Cached 60 s
/// per set of folders, like Apple Photos libraries.
#[tauri::command(async)]
pub async fn find_lightroom_catalogs(app: AppHandle) -> AppResult<Vec<CatalogRef>> {
    use std::sync::Mutex;
    use std::time::{Duration, Instant};
    type Found = (Vec<PathBuf>, Instant, Vec<CatalogRef>);
    static CACHE: Mutex<Option<Found>> = Mutex::new(None);
    let roots: Vec<PathBuf> = crate::commands::apple::search_roots(&app).into_iter().map(|(p, _)| p).collect();
    if let Some((k, at, found)) = CACHE.lock().expect("catalog cache lock").as_ref() {
        if *k == roots && at.elapsed() < Duration::from_secs(60) {
            return Ok(found.clone());
        }
    }
    let key = roots.clone();
    let found = tauri::async_runtime::spawn_blocking(move || find_catalogs(&roots, &recent_catalogs()))
        .await
        .map_err(|e| AppError { code: "internal".into(), message: "The search stopped unexpectedly.".into(), detail: Some(e.to_string()) })?;
    *CACHE.lock().expect("catalog cache lock") = Some((key, Instant::now(), found.clone()));
    Ok(found)
}

/// What starting a catalog's import began, and what it leaves behind (missing originals,
/// virtual copies, smart collections) for the summary.
#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct LightroomImportStart {
    #[ts(type = "number")]
    pub batch_id: i64,
    pub summary: CatalogSummary,
}

/// Imports a catalog's originals (copied; the catalog and its originals are only read) with
/// their ratings, flags, labels, keywords, collections, captions and develop settings.
#[tauri::command(async)]
pub async fn start_lightroom_import(app: AppHandle, path: String) -> AppResult<LightroomImportStart> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let unreadable = |e: photos_import::ImportError| AppError { code: "catalog_unreadable".into(), message: "The Lightroom catalog could not be read.".into(), detail: Some(e.to_string()) };
        let source = Arc::new(LightroomCatalogSource::open(Path::new(&path)).map_err(unreadable)?);
        let catalogue = state.library()?;
        let key = source.key();
        let items = source.enumerate().map_err(unreadable)?;
        let seen_items: Vec<SeenItem<'_>> = items.iter().map(|i| SeenItem { rel_path: &i.rel_path, size: i.size, mtime: i.modified_at }).collect();
        let unseen = catalogue.db.with_conn(|c| seen::unseen_indexes(c, &key, &seen_items))?;
        if unseen.is_empty() {
            return Err(AppError { code: "nothing_new".into(), message: "Everything in this catalog is already imported.".into(), detail: None });
        }
        let allow: HashSet<String> = unseen.iter().map(|&i| items[i].rel_path.clone()).collect();
        let summary = source.summary().clone();
        let batch_id = spawn_import(&app, &state, Box::new(FilteredSource::new(source, allow)), Some(path.clone()), Some(key), None)?;
        Ok(LightroomImportStart { batch_id, summary })
    })
    .await
    .map_err(|e| AppError { code: "internal".into(), message: "The import stopped unexpectedly.".into(), detail: Some(e.to_string()) })?
}

#[cfg(test)]
mod tests {
    use super::*;

    const PRESET: &str = r#"<x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
<rdf:Description rdf:about="" xmlns:crs="http://ns.adobe.com/camera-raw-settings/1.0/" crs:Vibrance="+20" crs:LensProfileEnable="1">
<crs:Name><rdf:Alt><rdf:li xml:lang="x-default">Pop</rdf:li></rdf:Alt></crs:Name>
<crs:Group><rdf:Alt><rdf:li xml:lang="x-default">Colour</rdf:li></rdf:Alt></crs:Group>
</rdf:Description></rdf:RDF></x:xmpmeta>"#;

    #[test]
    fn imports_every_preset_in_a_folder_and_passes_over_what_is_none() {
        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("Portrait").join("Deep");
        std::fs::create_dir_all(&sub).unwrap();
        std::fs::write(dir.path().join("pop.XMP"), PRESET).unwrap();
        std::fs::write(sub.join("matte.lrtemplate"), "s = { title = \"Matte\", value = { settings = { Contrast2012 = -20 } } }").unwrap();
        // A sidecar is no preset, nor an XMP without Camera Raw settings, nor a text file.
        std::fs::write(sub.join("IMG_1.xmp"), PRESET.replace("crs:Vibrance", "crs:RawFileName=\"IMG_1.CR3\" crs:Vibrance")).unwrap();
        std::fs::write(sub.join("other.xmp"), "<x:xmpmeta xmlns:x=\"adobe:ns:meta/\"/>").unwrap();
        std::fs::write(sub.join("notes.txt"), "hello").unwrap();
        std::fs::write(sub.join("broken.xmp"), "<x:xmpmeta>").unwrap();
        let files = preset_files(&[dir.path().to_path_buf()]);
        assert_eq!(files.len(), 5, "{files:?}");

        let db = Db::open_in_memory().unwrap();
        let report = import_files(&db, &files, 0);
        assert_eq!(report.imported, 2, "{report:?}");
        assert_eq!(report.failed.len(), 1);
        assert!(report.failed[0].file.ends_with("broken.xmp"));
        assert_eq!(report.not_converted, vec![PresetNotes { name: "Colour / Pop".into(), settings: vec!["LensProfileEnable".into()] }]);
        let names: Vec<String> = db.with_conn(develop_presets::list_presets).unwrap().into_iter().filter(|p| !p.builtin).map(|p| p.name).collect();
        assert_eq!(names, vec!["Colour / Pop", "Matte"]);

        // Again: nothing is duplicated.
        let again = import_files(&db, &files, 0);
        assert_eq!((again.imported, again.skipped_same_name), (0, 2));
    }

    #[test]
    fn the_camera_raw_folder_is_offered_only_when_it_exists() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(camera_raw_folder(Some(dir.path().to_path_buf())), None);
        let settings = dir.path().join("Adobe").join("CameraRaw").join("Settings");
        std::fs::create_dir_all(&settings).unwrap();
        let found = camera_raw_folder(Some(dir.path().to_path_buf()));
        if cfg!(windows) || cfg!(target_os = "macos") {
            assert_eq!(found, Some(settings));
        } else {
            assert_eq!(found, None, "Linux has no Camera Raw");
        }
        assert_eq!(camera_raw_folder(None), None);
    }
}
