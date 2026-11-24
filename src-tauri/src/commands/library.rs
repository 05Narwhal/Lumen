use std::path::Path;

use photos_core::{Library, LibraryEntry, LibraryInfo, LibraryListing, LibraryManifest};
use tauri::{AppHandle, State};

use crate::error::{AppError, AppResult};
use crate::state::AppState;

#[tauri::command(async)]
pub fn list_libraries(state: State<'_, AppState>) -> AppResult<LibraryListing> {
    // Round 24 §6.3, debug builds only: `LUMEN_DEBUG_SLOW_BOOT=<seconds>` holds the library load so
    // the splash's ten-second ceiling and Close during a stuck start can be checked by hand.
    #[cfg(debug_assertions)]
    if let Some(secs) = std::env::var("LUMEN_DEBUG_SLOW_BOOT").ok().and_then(|v| v.parse::<u64>().ok()) {
        std::thread::sleep(std::time::Duration::from_secs(secs));
    }
    Ok(state.registry.read().expect("registry lock").listing())
}

#[tauri::command(async)]
pub fn get_current_library(state: State<'_, AppState>) -> AppResult<Option<LibraryInfo>> {
    state.current_library_info()
}

/// The registry's current entry when its disk is not connected and nothing else is open.
#[tauri::command(async)]
pub fn get_missing_library(state: State<'_, AppState>) -> AppResult<Option<LibraryEntry>> {
    if state.library.read().expect("library lock").is_some() {
        return Ok(None);
    }
    let reg = state.registry.read().expect("registry lock");
    Ok(reg.current_entry().filter(|e| !Library::exists_at(Path::new(&e.path))).cloned())
}

/// Why the current library did not open at launch, while nothing else is open (Round 17).
#[tauri::command(async)]
pub fn get_library_problem(state: State<'_, AppState>) -> AppResult<Option<crate::state::LibraryProblem>> {
    if state.library.read().expect("library lock").is_some() {
        return Ok(None);
    }
    Ok(state.library_problem.read().expect("library problem lock").clone())
}

#[tauri::command(async)]
pub fn library_exists_at(path: String) -> bool {
    Library::exists_at(Path::new(&path))
}

#[tauri::command(async)]
pub fn create_library(app: AppHandle, state: State<'_, AppState>, path: String) -> AppResult<LibraryInfo> {
    let info = state.adopt_library(Library::create(Path::new(&path))?)?;
    state.changes.changed(&app, true);
    Ok(info)
}

#[tauri::command(async)]
pub fn open_library(app: AppHandle, state: State<'_, AppState>, path: String) -> AppResult<LibraryInfo> {
    // A backup is opened only to restore (Round 5A): never while its library is still there.
    let restoring = crate::backup::guard_open(&state, Path::new(&path))?;
    let lib = Library::open_protected(Path::new(&path), &state.paths.pre_migration_dir())?;
    lib.reconcile()?;
    if let Err(e) = photos_import::recover(&lib) {
        tracing::warn!("import recovery failed: {e}");
    }
    let info = state.adopt_library(lib)?;
    if restoring {
        crate::backup::forget_marker(Path::new(&path));
    }
    state.changes.changed(&app, true);
    Ok(info)
}

/// Drops a library from the registry. The folder on disk is untouched.
#[tauri::command(async)]
pub fn forget_library(state: State<'_, AppState>, id: String) -> AppResult<LibraryListing> {
    let current_id = state.library.read().expect("library lock").as_ref().map(|l| l.manifest.id.clone());
    if current_id.as_deref() == Some(id.as_str()) {
        return Err(AppError {
            code: "library_in_use".into(),
            message: "Switch to another library before forgetting this one.".into(),
            detail: None,
        });
    }
    let mut reg = state.registry.write().expect("registry lock");
    reg.remove(&id);
    reg.save(&state.paths.registry_file())?;
    Ok(reg.listing())
}

/// Points a known library at a new path (the disk moved or mounted elsewhere).
#[tauri::command(async)]
pub fn locate_library(app: AppHandle, state: State<'_, AppState>, id: String, path: String) -> AppResult<LibraryInfo> {
    let root = Path::new(&path);
    let manifest = LibraryManifest::load(root)?;
    match manifest {
        Some(m) if m.id == id => {}
        Some(m) => {
            return Err(AppError {
                code: "library_mismatch".into(),
                message: format!("That folder holds a different library ({}).", m.name),
                detail: Some(m.id),
            })
        }
        None if Library::exists_at(root) => {
            return Err(AppError {
                code: "library_mismatch".into(),
                message: "That folder holds a library without an identity file.".into(),
                detail: None,
            })
        }
        None => return Err(photos_core::CoreError::NotFound(path.clone()).into()),
    }
    open_library(app, state, path)
}

/// *Files Lumen can't open* (Round 18 C1): the originals no reader could decode, a page at a
/// time by asset id. They are off every grid; nothing is deleted.
#[tauri::command(async)]
pub fn list_unreadable(state: State<'_, AppState>, after: Option<i64>, limit: Option<u32>, trashed: Option<bool>) -> AppResult<crate::library_health::UnreadableRowsPage> {
    let lib = state.library()?;
    let limit = limit.unwrap_or(200).clamp(1, 500) as usize;
    // Round 22: each row carries its cause (the reader's own message, sorted).
    Ok(crate::library_health::unreadable_page(&lib, trashed.unwrap_or(false), after, limit)?)
}

/// *Restore…*: the folders these files (`ids`, or every flagged one) came from that still exist,
/// the one holding most first. Offered as a one-click *Look in <path>*.
#[tauri::command(async)]
pub fn unreadable_source_folders(state: State<'_, AppState>, ids: Option<Vec<i64>>) -> AppResult<Vec<photos_core::unreadable::SourceFolder>> {
    let lib = state.library()?;
    let ids = ids.filter(|i| !i.is_empty());
    let mut folders = lib.db.with_conn(|c| photos_core::unreadable::source_folders(c, ids.as_deref()))?;
    folders.retain(|f| std::path::Path::new(&f.path).is_dir());
    Ok(folders)
}

/// The flagged files the person removed to the Trash (*n in Trash · Put back*).
#[tauri::command(async)]
pub fn trashed_unreadable_ids(state: State<'_, AppState>) -> AppResult<Vec<i64>> {
    let lib = state.library()?;
    Ok(lib.db.with_conn(photos_core::unreadable::trashed_ids)?)
}

#[tauri::command(async)]
pub fn count_unreadable(state: State<'_, AppState>) -> AppResult<i64> {
    let lib = state.library()?;
    Ok(lib.db.with_conn(photos_core::unreadable::count)?)
}

/// *Try again*: decodes these files afresh (`ids` empty or missing: all of them). A file that
/// reads now comes back onto the grids; the rest stay listed. Returns how many were queued.
#[tauri::command(async)]
pub fn retry_unreadable(app: AppHandle, state: State<'_, AppState>, ids: Option<Vec<i64>>) -> AppResult<usize> {
    let lib = state.library()?;
    let ids = match ids.filter(|i| !i.is_empty()) {
        Some(ids) => ids,
        None => {
            let mut all = Vec::new();
            let mut after = None;
            loop {
                let page = lib.db.with_conn(|c| photos_core::unreadable::list(c, after, 500))?;
                all.extend(page.items.iter().map(|i| i.id));
                match page.next_cursor {
                    Some(next) => after = Some(next),
                    None => break,
                }
            }
            all
        }
    };
    Ok(crate::derivatives::retry_unreadable(&app, &ids))
}
