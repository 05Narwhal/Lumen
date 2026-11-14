//! The device browser (Round 4E): open a session on a card, an iPhone, a folder on a Mac disk
//! or an Apple Photos library, page through it, and import exactly what was picked.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex, RwLock};

use photos_core::seen::{self, SeenItem};
use photos_import::macdisk::source::MacFolderSource;
use photos_import::{iphone, FilteredSource, ImportSource, IphoneSource, SourceItem, VolumeSource};
use serde::Deserialize;
use tauri::{AppHandle, Emitter, Manager, State};
use ts_rs::TS;

use crate::commands::import::{plan_for, spawn_import, ImportPlan};
use crate::connected::DeviceRef;
use crate::device_browser::{self as browser, BrowserFilter, BrowserPage, BrowserSelection, BrowserSession, Catalogue, DateSource, Session};
use crate::error::{AppError, AppResult};
use crate::events::DEVICE_BROWSER_CHANGED;
use crate::state::AppState;
use crate::{macdisks, volumes};

/// What to browse. Ids are the ones the connected pages already use.
#[derive(Debug, Clone, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "camelCase")]
#[ts(export)]
pub enum BrowseTarget {
    /// `id` is the mount.
    Volume { id: String },
    /// `id` is the UDID.
    Iphone { id: String },
    /// A folder (and its subfolders) on a Mac disk.
    #[serde(rename_all = "camelCase")]
    MacFolder { disk: String, path: String },
    /// An Apple Photos library on a Mac disk (`disk`) or on this computer.
    #[serde(rename_all = "camelCase")]
    PhotosLibrary { disk: Option<String>, path: String },
}

fn gone(what: &str, id: &str) -> AppError {
    AppError { code: "device_gone".into(), message: format!("That {what} is no longer connected."), detail: Some(id.into()) }
}

fn unreadable(e: photos_import::ImportError) -> AppError {
    AppError { code: "device_unreadable".into(), message: "The device's photos could not be read.".into(), detail: Some(e.to_string()) }
}

fn no_session(id: u32) -> AppError {
    AppError { code: "browser_closed".into(), message: "That browser was closed.".into(), detail: Some(id.to_string()) }
}

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> AppResult<T> + Send + 'static) -> AppResult<T> {
    tauri::async_runtime::spawn_blocking(f)
        .await
        .map_err(|e| AppError { code: "internal".into(), message: "The work stopped unexpectedly.".into(), detail: Some(e.to_string()) })?
}

/// A file dated by its modified time, made a wall time like a photo's own date.
fn file_time(i: &SourceItem) -> (Option<i64>, Option<DateSource>) {
    (i.modified_at.map(browser::wall_time), i.modified_at.map(|_| DateSource::Modified))
}

/// Apple Photos' own date, already the wall time it was taken at.
fn photos_time(i: &SourceItem) -> (Option<i64>, Option<DateSource>) {
    (i.modified_at, i.modified_at.map(|_| DateSource::Photos))
}

struct Opened {
    name: String,
    source: Arc<dyn ImportSource>,
    seen_key: Option<String>,
    path_label: Option<String>,
    mount: Option<PathBuf>,
    photos_library: bool,
    /// Whether file times on it are worth checking against the photos' own dates.
    date_pass: bool,
}

fn open_target(state: &AppState, target: &BrowseTarget) -> AppResult<Opened> {
    Ok(match target {
        BrowseTarget::Volume { id } => {
            let info = volumes::snapshot().into_iter().find(|v| &v.mount == id).ok_or_else(|| gone("card or drive", id))?;
            let total_space: u64 = info.key.rsplit(':').next().and_then(|s| s.parse().ok()).unwrap_or(0);
            let source = VolumeSource::detect(&PathBuf::from(id), &info.label, total_space).ok_or_else(|| gone("card or drive", id))?;
            Opened {
                name: info.label.clone(),
                seen_key: Some(source.key().to_string()),
                path_label: Some(id.clone()),
                mount: Some(PathBuf::from(id)),
                source: Arc::new(source),
                photos_library: false,
                date_pass: true,
            }
        }
        BrowseTarget::Iphone { id } => {
            let phone = iphone::list_iphones()?.into_iter().find(|p| &p.udid == id).ok_or_else(|| gone("iPhone", id))?;
            if !phone.paired {
                return Err(photos_import::IphoneError::TrustRequired.into());
            }
            let source = IphoneSource::new(&phone.udid, &phone.name);
            // A phone's file times are when it took the photo; reading every file's head
            // over USB to confirm that would cost minutes.
            Opened {
                name: phone.name.clone(),
                seen_key: Some(source.key()),
                path_label: None,
                mount: None,
                source: Arc::new(source),
                photos_library: false,
                date_pass: false,
            }
        }
        BrowseTarget::MacFolder { disk, path } => {
            let (info, fs) = macdisks::get(disk).ok_or_else(|| gone("Mac disk", disk))?;
            let name = path.trim_end_matches('/').rsplit('/').next().filter(|s| !s.is_empty()).unwrap_or(&info.name).to_string();
            Opened {
                name,
                path_label: Some(format!("{}: {path}", info.name)),
                source: Arc::new(MacFolderSource::new(fs, &info.name, path, true)),
                seen_key: None,
                mount: None,
                photos_library: false,
                // Every read off a Mac disk copies the whole file first.
                date_pass: false,
            }
        }
        BrowseTarget::PhotosLibrary { disk, path } => {
            let lib = crate::commands::apple::library(state, disk.as_deref(), path)?;
            Opened {
                name: lib.summary().name.clone(),
                seen_key: Some(lib.key()),
                path_label: Some(path.clone()),
                mount: None,
                source: lib,
                photos_library: true,
                date_pass: false,
            }
        }
    })
}

/// Which of `files` this source has brought into the library before.
fn seen_paths(state: &AppState, key: Option<&str>, files: &[&SourceItem]) -> HashSet<String> {
    let (Some(key), Ok(lib)) = (key, state.library()) else { return HashSet::new() };
    let items: Vec<SeenItem<'_>> = files.iter().map(|i| SeenItem { rel_path: &i.rel_path, size: i.size, mtime: i.modified_at }).collect();
    let unseen: HashSet<usize> = match lib.db.with_conn(|c| seen::unseen_indexes(c, key, &items)) {
        Ok(u) => u.into_iter().collect(),
        Err(_) => return HashSet::new(),
    };
    files.iter().enumerate().filter(|(i, _)| !unseen.contains(i)).map(|(_, f)| f.rel_path.clone()).collect()
}

fn mark_imported(state: &AppState, session: &Session) {
    let files: Vec<SourceItem> = session.catalogue.read().expect("catalogue").entries.iter().flat_map(|e| e.files.iter().cloned()).collect();
    let refs: Vec<&SourceItem> = files.iter().collect();
    let seen = seen_paths(state, session.seen_key.as_deref(), &refs);
    session.catalogue.write().expect("catalogue").mark_imported(|f| seen.contains(&f.rel_path));
}

fn publish(app: &AppHandle, session: &Session) {
    let _ = app.emit(DEVICE_BROWSER_CHANGED, session.changed());
}

fn close(state: &AppState, session: Arc<Session>) {
    let tokens: Vec<u64> = session.tokens.lock().expect("tokens").values().copied().collect();
    state.device_previews.forget(&tokens);
}

/// Lists everything on the device (slow on a phone: a USB listing of its camera roll), groups
/// Live Photos and RAW pairs, marks what was imported before, and on a card starts reading
/// the photos' own dates in the background (`device-browser://changed` as they arrive).
#[tauri::command(async)]
pub async fn open_device_browser(app: AppHandle, target: BrowseTarget) -> AppResult<BrowserSession> {
    blocking(move || {
        let state = app.state::<AppState>();
        let o = open_target(&state, &target)?;
        let items = o.source.enumerate().map_err(unreadable)?;
        let entries = if o.photos_library {
            browser::group(items, browser::pair_by_uuid, photos_time)
        } else {
            browser::group(items, browser::pair_by_name, file_time)
        };
        let session = Arc::new(Session {
            id: state.device_browsers.next_id(),
            name: o.name,
            source: o.source,
            seen_key: o.seen_key,
            path_label: o.path_label,
            mount: o.mount,
            catalogue: RwLock::new(Catalogue::new(entries)),
            tokens: Mutex::new(HashMap::new()),
            closed: AtomicBool::new(false),
            dating: AtomicBool::new(o.date_pass),
        });
        mark_imported(&state, &session);
        for old in state.device_browsers.insert(session.clone()) {
            close(&state, old);
        }
        if o.date_pass {
            let (app, s) = (app.clone(), session.clone());
            let spawned = std::thread::Builder::new().name("lumen-device-dates".into()).spawn(move || {
                photos_jobs::background_priority();
                browser::date_pass(&s, |s| publish(&app, s));
            });
            if spawned.is_err() {
                session.dating.store(false, std::sync::atomic::Ordering::Relaxed);
            }
        }
        Ok(session.summary())
    })
    .await
}

/// A page of the session's items under `filter`, from `cursor` (a position in the filtered
/// order, from the previous page).
#[tauri::command(async)]
pub fn browse_device(state: State<'_, AppState>, session: u32, filter: BrowserFilter, cursor: Option<u32>, limit: Option<u32>) -> AppResult<BrowserPage> {
    let s = state.device_browsers.get(session).ok_or_else(|| no_session(session))?;
    let device = DeviceRef::Shared(s.source.clone());
    Ok(s.page(&filter, cursor, limit.unwrap_or(200), |item| state.device_previews.register(&device, item)))
}

/// Every index under `filter`, for *Select all*.
#[tauri::command(async)]
pub fn device_browser_ids(state: State<'_, AppState>, session: u32, filter: BrowserFilter) -> AppResult<Vec<u32>> {
    let s = state.device_browsers.get(session).ok_or_else(|| no_session(session))?;
    let ids = s.catalogue.read().expect("catalogue").ids(&filter);
    Ok(ids)
}

/// How many files and bytes a selection is, and how much of it is already in the library.
#[tauri::command(async)]
pub fn device_browser_selection(state: State<'_, AppState>, session: u32, indexes: Vec<u32>) -> AppResult<BrowserSelection> {
    let s = state.device_browsers.get(session).ok_or_else(|| no_session(session))?;
    let sel = s.catalogue.read().expect("catalogue").selection(&indexes);
    Ok(sel)
}

/// The session's current summary (after an import, or once the dates are read).
#[tauri::command(async)]
pub fn get_device_browser(state: State<'_, AppState>, session: u32) -> AppResult<BrowserSession> {
    Ok(state.device_browsers.get(session).ok_or_else(|| no_session(session))?.summary())
}

#[tauri::command(async)]
pub fn close_device_browser(state: State<'_, AppState>, session: u32) -> AppResult<()> {
    if let Some(s) = state.device_browsers.remove(session) {
        close(&state, s);
    }
    Ok(())
}

/// The space check for a selection: items are whole tiles (a Live Photo with its video).
#[tauri::command(async)]
pub fn plan_device_browser_import(state: State<'_, AppState>, session: u32, indexes: Vec<u32>) -> AppResult<ImportPlan> {
    let s = state.device_browsers.get(session).ok_or_else(|| no_session(session))?;
    let lib = state.library()?;
    let tiles = s.catalogue.read().expect("catalogue").as_items(&indexes);
    Ok(plan_for(&tiles, crate::space::free_space(&lib.root)))
}

/// Imports the picked tiles — with `limit_bytes`, the newest that fit — through the same
/// import as everything else: each file is remembered as seen as it lands, and an import
/// cut short resumes from `import_seen`, with or without this browser.
#[tauri::command(async)]
pub fn start_device_browser_import(app: AppHandle, state: State<'_, AppState>, session: u32, indexes: Vec<u32>, limit_bytes: Option<u64>) -> AppResult<i64> {
    let s = state.device_browsers.get(session).ok_or_else(|| no_session(session))?;
    let files = s.catalogue.read().expect("catalogue").files_for(&indexes, limit_bytes);
    if files.is_empty() {
        return Err(AppError { code: "nothing_selected".into(), message: "Nothing is selected.".into(), detail: None });
    }
    let allow: HashSet<String> = files.into_iter().map(|f| f.rel_path).collect();
    let source = FilteredSource::new(s.source.clone(), allow);
    let after_app = app.clone();
    let after_session = s.clone();
    spawn_import(
        &app,
        &state,
        Box::new(source),
        s.path_label.clone(),
        s.seen_key.clone(),
        Some(Box::new(move |_summary, _src| {
            if let Some(mount) = &after_session.mount {
                volumes::mark_dirty(mount);
            }
            if !after_session.closed.load(std::sync::atomic::Ordering::Relaxed) {
                mark_imported(&after_app.state::<AppState>(), &after_session);
                publish(&after_app, &after_session);
            }
        })),
    )
}
