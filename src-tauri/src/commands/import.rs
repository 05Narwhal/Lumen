use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use photos_core::batches::{self, ImportBatchSummary, ImportFailure, BATCH_FAILED};
use photos_core::now_unix;
use photos_import::{fit_newest_first, run_batch, FilesSource, FilteredSource, FolderSource, ImportOptions, ImportSource, SourceItem, IMPORT_FREE_BUFFER};
use serde::Serialize;
use ts_rs::TS;
use photos_jobs::CancelToken;
use tauri::{AppHandle, Emitter, Manager, State};

use crate::derivatives;
use crate::error::{AppError, AppResult};
use crate::events;
use crate::state::AppState;

/// What an import would bring in, and how much of it fits on the library's drive while
/// keeping `IMPORT_FREE_BUFFER` free (round 3). Asked before every import starts.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ImportPlan {
    #[ts(type = "number")]
    pub count: u64,
    #[ts(type = "number")]
    pub bytes: u64,
    /// Free on the library's drive now; `None` when the system would not say.
    #[ts(type = "number | null")]
    pub free_bytes: Option<u64>,
    #[ts(type = "number")]
    pub fits_count: u64,
    #[ts(type = "number")]
    pub fits_bytes: u64,
    /// The room kept free, for the dialog's wording.
    #[ts(type = "number")]
    pub buffer: u64,
}

pub fn plan_for(items: &[SourceItem], free: Option<u64>) -> ImportPlan {
    let bytes = items.iter().map(|i| i.size).sum();
    let (fits, fits_bytes) = match free {
        Some(f) => fit_newest_first(items, photos_import::budget::budget_for(f)),
        None => ((0..items.len()).collect(), bytes),
    };
    ImportPlan { count: items.len() as u64, bytes, free_bytes: free, fits_count: fits.len() as u64, fits_bytes, buffer: IMPORT_FREE_BUFFER }
}

/// The paths of the items to import: all of them, or with `limit` the newest that fit it.
pub fn choose(items: &[SourceItem], limit: Option<u64>) -> HashSet<String> {
    match limit {
        Some(l) => fit_newest_first(items, l).0.into_iter().map(|i| items[i].rel_path.clone()).collect(),
        None => items.iter().map(|i| i.rel_path.clone()).collect(),
    }
}

/// The batch still running from the device with this `import_seen` key.
pub fn running_import_for(state: &AppState, key: &str) -> Option<i64> {
    let imports = state.imports.lock().expect("imports lock");
    let keys = state.import_keys.lock().expect("import keys lock");
    keys.iter().find(|(id, k)| k.as_str() == key && imports.contains_key(id)).map(|(id, _)| *id)
}

/// The import running from a card (`VolumeInfo.key`) or phone (`ios:<udid>`), so its page
/// shows the progress again after you leave and come back.
#[tauri::command(async)]
pub fn running_import(state: State<'_, AppState>, key: String) -> AppResult<Option<i64>> {
    Ok(running_import_for(&state, &key))
}

/// Runs after an import thread finishes, on that thread, with the source still alive.
pub type AfterImport = Box<dyn FnOnce(&photos_import::ImportSummary, &dyn ImportSource) + Send>;

/// Starts an import of `source` on its own thread and returns the batch id at once.
/// Progress arrives as `import://progress`; completion as `import://finished`.
/// `seen_key` names a card or phone in `import_seen`: each file is remembered as it lands in
/// the library, so an import cut short picks up exactly what is left next time.
pub fn spawn_import(
    app: &AppHandle,
    state: &AppState,
    source: Box<dyn ImportSource>,
    path_label: Option<String>,
    seen_key: Option<String>,
    after: Option<AfterImport>,
) -> AppResult<i64> {
    spawn_import_watched(app, state, source, path_label, seen_key, after, None)
}

/// A plugin import's extras for [`spawn_import_watched`].
pub struct Watched {
    /// The plugin importer's cancel flag, used as the import's own token: Cancel sets it (and so
    /// interrupts the plugin's running call), and so does turning the plugin off.
    pub cancel: Arc<AtomicBool>,
    /// Marks the import as running (see [`running_import_for`]) without recording seen items.
    pub key: String,
}

/// [`spawn_import`] for a plugin importer (`watch`), whose cancel flag is the import's token.
pub fn spawn_import_watched(
    app: &AppHandle,
    state: &AppState,
    source: Box<dyn ImportSource>,
    path_label: Option<String>,
    seen_key: Option<String>,
    after: Option<AfterImport>,
    watch: Option<Watched>,
) -> AppResult<i64> {
    let lib = state.library()?;
    if let Some(running) = seen_key.as_deref().and_then(|k| running_import_for(state, k)) {
        return Err(AppError {
            code: "already_importing".into(),
            message: "An import from this device is already running. Wait for it to finish.".into(),
            detail: Some(running.to_string()),
        });
    }
    let label = source.label();
    let batch_id = lib
        .db
        .with_conn(|c| batches::start_batch(c, source.kind(), &label, path_label.as_deref(), now_unix()))?;
    let (cancel, busy_key) = match watch {
        Some(w) => (CancelToken::from_flag(w.cancel), Some(w.key)),
        None => (CancelToken::new(), seen_key.clone()),
    };
    state.imports.lock().expect("imports lock").insert(batch_id, cancel.clone());
    if let Some(k) = busy_key {
        state.import_keys.lock().expect("import keys lock").insert(batch_id, k);
    }
    let skip_duplicates = matches!(state.settings.read().expect("settings lock").import.duplicates, photos_core::DuplicateHandling::Skip);
    let ffmpeg = state.ffmpeg();
    // Every import keeps 20 GB free on the library's drive, asking as it goes.
    let root = lib.root.clone();
    let free_space = Some(photos_import::FreeSpace::new(move || crate::space::free_space(&root)));
    let app = app.clone();
    std::thread::Builder::new()
        .name(format!("lumen-import-{batch_id}"))
        .spawn(move || {
            let app_progress = app.clone();
            let app_ingest = app.clone();
            let result = run_batch(
                &lib,
                source.as_ref(),
                batch_id,
                &ImportOptions { skip_duplicates, ffmpeg, seen_key, free_space, ..Default::default() },
                &cancel,
                &mut |p| {
                    let _ = app_progress.emit(events::IMPORT_PROGRESS, p);
                },
                &mut |i| derivatives::schedule_ingest(app_ingest.clone(), i.clone()),
            );
            let state = app.state::<AppState>();
            state.imports.lock().expect("imports lock").remove(&batch_id);
            state.import_keys.lock().expect("import keys lock").remove(&batch_id);
            match result {
                Ok(summary) => {
                    if !summary.cancelled {
                        if let Some(f) = after {
                            f(&summary, source.as_ref());
                        }
                    }
                    let _ = app.emit(events::IMPORT_FINISHED, &summary);
                    if summary.imported > 0 {
                        crate::maintenance::schedule_memories(&app);
                        crate::embeddings::schedule(&app);
                        crate::video_highlights::schedule(&app);
                        crate::video_prep::schedule(&app);
                        // Exact place names, when the switch is on (no-op otherwise).
                        crate::places_online::wake();
                    }
                }
                Err(e) => {
                    tracing::error!(batch_id, "import failed: {e}");
                    let _ = lib.db.with_conn(|c| batches::finish_batch(c, batch_id, BATCH_FAILED, 0, 0, now_unix()));
                    let _ = app.emit(
                        events::IMPORT_FINISHED,
                        photos_import::ImportSummary { batch_id, failed: 1, ..Default::default() },
                    );
                }
            }
            state.changes.changed(&app, true);
        })
        .map_err(|e| AppError { code: "thread".into(), message: "Could not start the import.".into(), detail: Some(e.to_string()) })?;
    Ok(batch_id)
}

/// Starts a folder import on its own thread and returns the batch id at once. Subfolders
/// are included unless `recursive` is `false`. With `as_albums` each folder with photos
/// becomes an album and each folder with subfolders a collection (`photos_import::folder_albums`).
#[tauri::command(async)]
pub fn start_folder_import(
    app: AppHandle,
    state: State<'_, AppState>,
    path: String,
    recursive: Option<bool>,
    limit_bytes: Option<u64>,
    as_albums: Option<bool>,
) -> AppResult<i64> {
    let folder = folder_source(&path, recursive)?;
    let source = if as_albums.unwrap_or(false) { limited(photos_import::FolderAlbumsSource::new(folder), limit_bytes)? } else { limited(folder, limit_bytes)? };
    spawn_import(&app, &state, source, Some(path), None, None)
}

/// `source`, cut down to what fits in `limit_bytes` when there is a limit.
fn limited<S: ImportSource + 'static>(source: S, limit_bytes: Option<u64>) -> AppResult<Box<dyn ImportSource>> {
    Ok(match limit_bytes {
        Some(_) => {
            let items = source.enumerate().map_err(unreadable)?;
            Box::new(FilteredSource::new(source, choose(&items, limit_bytes)))
        }
        None => Box::new(source),
    })
}

fn folder_source(path: &str, recursive: Option<bool>) -> AppResult<FolderSource> {
    let root = PathBuf::from(path);
    if !root.is_dir() {
        return Err(AppError { code: "not_a_folder".into(), message: "That path is not a folder.".into(), detail: Some(path.into()) });
    }
    Ok(FolderSource::new(&root).recursive(recursive.unwrap_or(true)))
}

fn unreadable(e: photos_import::ImportError) -> AppError {
    AppError { code: "source_unreadable".into(), message: "The files could not be read.".into(), detail: Some(e.to_string()) }
}

/// What an import would bring in and how much fits (round 3): `kind` is `folder` (`id` the
/// path), `files` (`paths`), `volume` (`id` the mount) or `iphone` (`id` the UDID). Listing a
/// phone takes a while, so this runs off the main thread.
#[tauri::command]
pub async fn plan_import(app: AppHandle, kind: String, id: Option<String>, paths: Option<Vec<String>>, recursive: Option<bool>) -> AppResult<ImportPlan> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let lib = state.library()?;
        let items = match kind.as_str() {
            "folder" => folder_source(id.as_deref().unwrap_or(""), recursive)?.enumerate().map_err(unreadable)?,
            "files" => FilesSource::new(paths.unwrap_or_default().into_iter().map(PathBuf::from).collect()).enumerate().map_err(unreadable)?,
            "volume" => crate::commands::devices::volume_new_items(&state, id.as_deref().unwrap_or(""))?.1,
            "iphone" => crate::commands::devices::iphone_new_items(&state, id.as_deref().unwrap_or(""))?.1,
            other => return Err(AppError { code: "bad_kind".into(), message: "Unknown import source.".into(), detail: Some(other.into()) }),
        };
        Ok(plan_for(&items, crate::space::free_space(&lib.root)))
    })
    .await
    .map_err(|e| AppError { code: "internal".into(), message: "Could not plan the import.".into(), detail: Some(e.to_string()) })?
}

/// Imports exactly the files given (from the file picker, or dropped on the window).
#[tauri::command(async)]
pub fn start_files_import(app: AppHandle, state: State<'_, AppState>, paths: Vec<String>, limit_bytes: Option<u64>) -> AppResult<i64> {
    let files: Vec<PathBuf> = paths.iter().map(PathBuf::from).collect();
    if !files.iter().any(|f| f.is_file()) {
        return Err(AppError { code: "no_files".into(), message: "None of those files could be found.".into(), detail: None });
    }
    // The common parent folder, when there is one, is what the history names.
    let parent = files.first().and_then(|f| f.parent()).filter(|p| files.iter().all(|f| f.parent() == Some(*p)));
    let source = FilesSource::new(files.clone());
    let source: Box<dyn ImportSource> = match limit_bytes {
        Some(_) => {
            let items = source.enumerate().map_err(unreadable)?;
            Box::new(FilteredSource::new(source, choose(&items, limit_bytes)))
        }
        None => Box::new(source),
    };
    spawn_import(&app, &state, source, parent.map(|p| p.display().to_string()), None, None)
}

/// The files of a folder import that failed, as paths to try again: a folder's failures are
/// relative to the folder, picked files' are already whole paths, and the line that says the
/// source went away or the drive filled (its label, not a file) is not a file to retry.
pub fn retry_paths(source_path: Option<&str>, failures: &[ImportFailure]) -> Vec<PathBuf> {
    let base = source_path.map(PathBuf::from);
    failures
        .iter()
        .filter(|f| f.reason != photos_import::ingest::DISCONNECTED && f.reason != photos_import::budget::OUT_OF_ROOM)
        .map(|f| {
            let p = PathBuf::from(&f.path);
            match (&base, p.is_absolute()) {
                (Some(b), false) => b.join(p),
                _ => p,
            }
        })
        .collect()
}

/// *Retry failed* on a folder import's row (Round 4G): a new import of exactly the files that
/// failed and are still where they were. Cards and phones pick up what is left on their own.
#[tauri::command(async)]
pub fn retry_import_failures(app: AppHandle, state: State<'_, AppState>, batch_id: i64) -> AppResult<i64> {
    let lib = state.library()?;
    let batch = lib
        .db
        .with_conn(|c| batches::list_batches(c, i64::MAX))?
        .into_iter()
        .find(|b| b.id == batch_id)
        .ok_or_else(|| AppError { code: "batch_gone".into(), message: "That import is no longer in the history.".into(), detail: None })?;
    if batch.source_kind != batches::SOURCE_FOLDER {
        return Err(AppError { code: "not_a_folder".into(), message: "Import from the device again to pick up what is left.".into(), detail: None });
    }
    let failures = lib.db.with_conn(|c| batches::list_failures(c, batch_id))?;
    let files: Vec<PathBuf> = retry_paths(batch.source_path.as_deref(), &failures).into_iter().filter(|p| p.is_file()).collect();
    if files.is_empty() {
        return Err(AppError { code: "nothing_to_retry".into(), message: "None of the files that failed can be found any more.".into(), detail: None });
    }
    spawn_import(&app, &state, Box::new(FilesSource::new(files)), batch.source_path.clone(), None, None)
}

#[tauri::command(async)]
pub fn cancel_import(state: State<'_, AppState>, batch_id: i64) -> AppResult<()> {
    if let Some(t) = state.imports.lock().expect("imports lock").get(&batch_id) {
        t.cancel();
    }
    Ok(())
}

#[tauri::command(async)]
pub fn list_import_batches(state: State<'_, AppState>, limit: Option<i64>) -> AppResult<Vec<ImportBatchSummary>> {
    let lib = state.library()?;
    Ok(lib.db.with_conn(|c| batches::list_batches(c, limit.unwrap_or(50)))?)
}

#[tauri::command(async)]
pub fn get_import_failures(state: State<'_, AppState>, batch_id: i64) -> AppResult<Vec<ImportFailure>> {
    let lib = state.library()?;
    Ok(lib.db.with_conn(|c| batches::list_failures(c, batch_id))?)
}

/// Every file an import did not bring in, with the reason (Round 19 C3).
#[tauri::command(async)]
pub fn get_import_skips(state: State<'_, AppState>, batch_id: i64) -> AppResult<Vec<photos_core::sidecars::ImportSkip>> {
    let lib = state.library()?;
    Ok(lib.db.with_conn(|c| photos_core::sidecars::skips_of(c, batch_id))?)
}

#[cfg(test)]
mod plan_tests {
    use super::*;
    use photos_import::SourceItem;

    fn item(name: &str, gb: u64, at: i64) -> SourceItem {
        SourceItem { rel_path: name.into(), file_name: name.into(), size: gb * 1_000_000_000, modified_at: Some(at), created_at: None }
    }

    #[test]
    fn a_plan_says_what_fits_above_the_buffer() {
        let items = vec![item("a", 10, 1), item("b", 10, 2), item("c", 10, 3)];
        let p = plan_for(&items, Some(45_000_000_000));
        assert_eq!((p.count, p.bytes, p.fits_count, p.fits_bytes), (3, 30_000_000_000, 2, 20_000_000_000));
        // Not knowing the drive, nothing is held back (the guard still watches).
        let p = plan_for(&items, None);
        assert_eq!(p.fits_count, 3);
        assert_eq!(p.free_bytes, None);
    }

    #[test]
    fn a_retry_finds_the_failed_files_where_they_were() {
        let f = |path: &str, reason: &str| ImportFailure { path: path.into(), reason: reason.into() };
        let whole = if cfg!(windows) { "C:/elsewhere/b.jpg" } else { "/elsewhere/b.jpg" };
        let got = retry_paths(
            Some("D:/Trip"),
            &[f("day1/a.jpg", "decode failed"), f(whole, "copy failed"), f("Card", photos_import::ingest::DISCONNECTED)],
        );
        assert_eq!(got, [PathBuf::from("D:/Trip").join("day1/a.jpg"), PathBuf::from(whole)]);
        assert_eq!(retry_paths(None, &[f("x.jpg", "bad")]), [PathBuf::from("x.jpg")]);
    }

    #[test]
    fn a_limit_keeps_the_newest_that_fit() {
        let items = vec![item("a", 10, 1), item("b", 10, 2), item("c", 10, 3)];
        let allow = choose(&items, Some(20_000_000_000));
        assert!(allow.contains("b") && allow.contains("c") && !allow.contains("a"));
        assert_eq!(choose(&items, None).len(), 3);
    }
}
