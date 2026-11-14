//! *Check before deleting* (Round 19 C4): runs `photos_import::check` as one job on the
//! Maintenance lane, reports to the activity ring (`check://progress`, `check://finished`) and
//! keeps the last result of each tool in memory for the settings page to list in pages. Only
//! the date of a finished full library check is written to disk (`library-checks.json`).
//! Nothing here changes a file in the library or in the checked folder.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use photos_import::check::{self, CheckItem, CheckProgress, FolderCheck, LibraryCheck, CheckProblem, CheckProblemItem, CheckVerdict};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager};
use ts_rs::TS;

use crate::error::{AppError, AppResult};
use crate::state::AppState;

pub const CHECK_PROGRESS: &str = "check://progress";
pub const CHECK_FINISHED: &str = "check://finished";

/// The two tools.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum CheckKind {
    Folder,
    Library,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct CheckProgressEvent {
    pub kind: CheckKind,
    pub progress: CheckProgress,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct CheckFinished {
    pub kind: CheckKind,
    pub cancelled: bool,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct FolderCheckSummary {
    pub folder: String,
    #[ts(type = "number")]
    pub checked_at: i64,
    pub safe: u32,
    pub not_in_library: u32,
    pub other: u32,
    pub cancelled: bool,
    pub safe_to_delete: bool,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct LibraryCheckSummary {
    #[ts(type = "number")]
    pub checked_at: i64,
    #[ts(type = "number")]
    pub checked: u64,
    #[ts(type = "number")]
    pub bytes: u64,
    pub problems: u32,
    pub cancelled: bool,
    pub intact: bool,
}

/// What the Check before deleting section shows.
#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct CheckStatus {
    pub running: Option<CheckKind>,
    pub folder: Option<FolderCheckSummary>,
    pub library: Option<LibraryCheckSummary>,
    /// When a full library check last ran to the end (this library).
    #[ts(type = "number | null")]
    pub last_full_check_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct CheckItemsPage {
    pub items: Vec<CheckItem>,
    pub next_cursor: Option<u32>,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct CheckProblemsPage {
    pub items: Vec<CheckProblemItem>,
    pub next_cursor: Option<u32>,
}

#[derive(Default)]
struct Results {
    library_id: String,
    folder: Option<FolderCheck>,
    library: Option<LibraryCheck>,
}

static RESULTS: Mutex<Results> = Mutex::new(Results { library_id: String::new(), folder: None, library: None });
static FILE: Mutex<()> = Mutex::new(());

fn results() -> std::sync::MutexGuard<'static, Results> {
    RESULTS.lock().unwrap_or_else(|p| p.into_inner())
}

fn job_key(library_id: &str) -> String {
    format!("check:{library_id}")
}

fn checks_file(state: &AppState) -> PathBuf {
    state.paths.app_data.join("library-checks.json")
}

#[derive(Default, Serialize, Deserialize)]
struct LastChecks {
    /// library id → unix time of its last full check.
    libraries: HashMap<String, i64>,
}

fn last_full_check(state: &AppState, library_id: &str) -> Option<i64> {
    let _held = FILE.lock().unwrap_or_else(|p| p.into_inner());
    let text = std::fs::read_to_string(checks_file(state)).ok()?;
    serde_json::from_str::<LastChecks>(&text).ok()?.libraries.get(library_id).copied()
}

fn remember_full_check(state: &AppState, library_id: &str, at: i64) {
    let _held = FILE.lock().unwrap_or_else(|p| p.into_inner());
    let path = checks_file(state);
    let mut all = std::fs::read_to_string(&path).ok().and_then(|t| serde_json::from_str::<LastChecks>(&t).ok()).unwrap_or_default();
    all.libraries.insert(library_id.to_string(), at);
    if let Ok(text) = serde_json::to_string_pretty(&all) {
        let tmp = path.with_extension("json.part");
        if std::fs::write(&tmp, text).is_ok() {
            let _ = std::fs::rename(&tmp, &path);
        }
    }
}

fn error(code: &str, message: &str) -> AppError {
    AppError { code: code.into(), message: message.into(), detail: None }
}

pub fn status(state: &AppState) -> AppResult<CheckStatus> {
    let lib = state.library()?;
    let id = lib.manifest.id.clone();
    let held = results();
    let mine = held.library_id == id;
    let running = state.jobs.is_inflight(&job_key(&id)).then(|| running_kind(&id));
    Ok(CheckStatus {
        running: running.flatten(),
        folder: mine.then(|| held.folder.as_ref().map(folder_summary)).flatten(),
        library: mine.then(|| held.library.as_ref().map(library_summary)).flatten(),
        last_full_check_at: last_full_check(state, &id),
    })
}

/// Which tool is running; kept beside the job because the scheduler only knows its key.
static RUNNING: Mutex<Option<(String, CheckKind)>> = Mutex::new(None);

fn running_kind(library_id: &str) -> Option<CheckKind> {
    RUNNING.lock().unwrap_or_else(|p| p.into_inner()).as_ref().filter(|(id, _)| id == library_id).map(|(_, k)| *k)
}

fn folder_summary(c: &FolderCheck) -> FolderCheckSummary {
    FolderCheckSummary { folder: c.folder.clone(), checked_at: c.checked_at, safe: c.safe, not_in_library: c.not_in_library, other: c.other, cancelled: c.cancelled, safe_to_delete: c.is_safe_to_delete() }
}

fn library_summary(c: &LibraryCheck) -> LibraryCheckSummary {
    LibraryCheckSummary { checked_at: c.checked_at, checked: c.checked, bytes: c.bytes, problems: c.problems.len() as u32, cancelled: c.cancelled, intact: c.is_intact() }
}

/// Starts a check of `folder` (`Some`) or of the whole library (`None`). One at a time.
pub fn start(app: &AppHandle, folder: Option<String>) -> AppResult<()> {
    let state = app.state::<AppState>();
    let lib = state.library()?;
    let id = lib.manifest.id.clone();
    let key = job_key(&id);
    if state.jobs.is_inflight(&key) {
        return Err(error("check_running", "A check is already running."));
    }
    let kind = if folder.is_some() { CheckKind::Folder } else { CheckKind::Library };
    if let Some(f) = &folder {
        if !Path::new(f).is_dir() {
            return Err(error("check_no_folder", "That folder is not there."));
        }
    }
    *RUNNING.lock().unwrap_or_else(|p| p.into_inner()) = Some((id.clone(), kind));
    let a = app.clone();
    state.jobs.submit(&key, photos_jobs::Lane::Maintenance, move |cancel| {
        let state = a.state::<AppState>();
        let last = std::sync::Mutex::new(Instant::now() - Duration::from_secs(1));
        let report = |p: CheckProgress| {
            // Four a second is plenty for a ring.
            let mut at = last.lock().unwrap_or_else(|e| e.into_inner());
            if at.elapsed() >= Duration::from_millis(250) || p.done_files == p.total_files {
                *at = Instant::now();
                let _ = a.emit(CHECK_PROGRESS, CheckProgressEvent { kind, progress: p });
            }
        };
        tracing::info!(?kind, "check started");
        let outcome: Result<bool, String> = match &folder {
            Some(f) => check::check_folder(&lib, Path::new(f), cancel, &report).map(|c| {
                let cancelled = c.cancelled;
                let mut held = results();
                if held.library_id != id {
                    *held = Results { library_id: id.clone(), ..Results::default() };
                }
                held.folder = Some(c);
                cancelled
            }),
            None => check::check_library(&lib, cancel, &report).map(|c| {
                let cancelled = c.cancelled;
                if !cancelled {
                    remember_full_check(&state, &id, c.checked_at);
                    // Round 22: the health card opens with this result (missing or changed files).
                    crate::library_health::write_health_file(&lib.root, crate::library_health::HealthFile { checked_at: c.checked_at, missing: c.problems.len() as i64 });
                }
                let mut held = results();
                if held.library_id != id {
                    *held = Results { library_id: id.clone(), ..Results::default() };
                }
                held.library = Some(c);
                cancelled
            }),
        }
        .map_err(|e| e.to_string());
        *RUNNING.lock().unwrap_or_else(|p| p.into_inner()) = None;
        let finished = match outcome {
            Ok(cancelled) => {
                tracing::info!(?kind, cancelled, "check finished");
                CheckFinished { kind, cancelled, error: None }
            }
            Err(message) => {
                tracing::warn!(?kind, "check stopped: {message}");
                CheckFinished { kind, cancelled: false, error: Some(message) }
            }
        };
        let _ = a.emit(CHECK_FINISHED, finished);
    });
    Ok(())
}

pub fn cancel(state: &AppState) -> AppResult<()> {
    let lib = state.library()?;
    state.jobs.cancel(&job_key(&lib.manifest.id));
    Ok(())
}

fn page_bounds(len: usize, after: Option<u32>, limit: u32) -> (usize, usize, Option<u32>) {
    let from = after.map_or(0, |a| a as usize + 1).min(len);
    let to = (from + limit.clamp(1, 500) as usize).min(len);
    (from, to, (to < len).then(|| (to - 1) as u32))
}

/// A page of the folder check's files with one verdict. The cursor is the position of the last
/// file handed out in the (fixed) result, so a page never repeats or skips.
pub fn items(state: &AppState, status: CheckVerdict, after: Option<u32>, limit: u32) -> AppResult<CheckItemsPage> {
    let lib = state.library()?;
    let held = results();
    let Some(c) = held.folder.as_ref().filter(|_| held.library_id == lib.manifest.id) else {
        return Ok(CheckItemsPage { items: vec![], next_cursor: None });
    };
    let all: Vec<&CheckItem> = c.items.iter().filter(|i| i.status == status).collect();
    let (from, to, next_cursor) = page_bounds(all.len(), after, limit);
    Ok(CheckItemsPage { items: all[from..to].iter().map(|i| (*i).clone()).collect(), next_cursor })
}

pub fn problems(state: &AppState, after: Option<u32>, limit: u32) -> AppResult<CheckProblemsPage> {
    let lib = state.library()?;
    let held = results();
    let Some(c) = held.library.as_ref().filter(|_| held.library_id == lib.manifest.id) else {
        return Ok(CheckProblemsPage { items: vec![], next_cursor: None });
    };
    let (from, to, next_cursor) = page_bounds(c.problems.len(), after, limit);
    Ok(CheckProblemsPage { items: c.problems[from..to].to_vec(), next_cursor })
}

fn when(at: i64) -> String {
    chrono::DateTime::from_timestamp(at, 0).map(|d| d.with_timezone(&chrono::Local).format("%Y-%m-%d %H:%M").to_string()).unwrap_or_default()
}

fn reason_text(code: &str) -> &'static str {
    match code {
        "not_imported" => "never imported",
        "in_trash" => "in Lumen's trash",
        "copy_missing" => "the library's copy is missing",
        "copy_differs" => "the library's copy differs from this file",
        "copy_unreadable" => "the library's copy cannot be read",
        "cannot_read" => "this file cannot be read",
        "sidecar_not_kept" => "its sidecar was not kept",
        "failed_verify" => "its copy did not verify when imported",
        "unreadable" => "no reader could open it",
        "burst_frame" => "a burst frame that was left out",
        "not_downloaded" => "not downloaded to the device",
        "unsupported" => "a type Lumen does not import",
        "duplicate" => "a duplicate",
        _ => "unknown reason",
    }
}

/// The report as plain text, for *Save report…*.
pub fn report_text(state: &AppState, kind: CheckKind) -> AppResult<String> {
    use std::fmt::Write;
    let lib = state.library()?;
    let held = results();
    let mine = held.library_id == lib.manifest.id;
    let mut out = String::new();
    match kind {
        CheckKind::Folder => {
            let c = held.folder.as_ref().filter(|_| mine).ok_or_else(|| error("check_no_result", "Check a folder first."))?;
            let _ = writeln!(out, "Lumen — check before deleting\nFolder: {}\nChecked: {}\n", c.folder, when(c.checked_at));
            let verdict = if c.cancelled {
                "Stopped before the end: this is not a verdict."
            } else if c.is_safe_to_delete() {
                "Every photo, video and sidecar in this folder is in the library, and both copies are identical."
            } else {
                "Not everything in this folder is safe in the library. Do not delete it yet."
            };
            let _ = writeln!(out, "{verdict}\nSafe: {}   Not in the library: {}   Other files: {}\n", c.safe, c.not_in_library, c.other);
            for (title, status) in [("Not in the library", CheckVerdict::NotInLibrary), ("Other files (not photos, videos or sidecars)", CheckVerdict::Other), ("Safe", CheckVerdict::Safe)] {
                let list: Vec<&CheckItem> = c.items.iter().filter(|i| i.status == status).collect();
                if list.is_empty() {
                    continue;
                }
                let _ = writeln!(out, "{title} ({})", list.len());
                for i in list {
                    match &i.reason {
                        Some(r) => {
                            let _ = writeln!(out, "  {} — {}", i.path, reason_text(r));
                        }
                        None => {
                            let _ = writeln!(out, "  {}", i.path);
                        }
                    }
                }
                out.push('\n');
            }
        }
        CheckKind::Library => {
            let c = held.library.as_ref().filter(|_| mine).ok_or_else(|| error("check_no_result", "Check the library first."))?;
            let _ = writeln!(out, "Lumen — library check\nLibrary: {}\nChecked: {}\n", lib.manifest.name, when(c.checked_at));
            let verdict = if c.cancelled {
                "Stopped before the end: this is not a verdict."
            } else if c.is_intact() {
                "Every original and sidecar read back exactly as it was imported."
            } else {
                "Some files are missing or changed."
            };
            let _ = writeln!(out, "{verdict}\nFiles re-read: {}   Problems: {}\n", c.checked, c.problems.len());
            for p in &c.problems {
                let what = match p.status {
                    CheckProblem::Missing => "missing",
                    CheckProblem::Changed => "changed",
                };
                let _ = writeln!(out, "  {} ({}) — {what}: originals/{}", p.filename, p.kind, p.relative_path);
            }
        }
    }
    Ok(out)
}

/// Writes the report into `folder` (chosen with the folder dialog) and returns its path.
pub fn save_report(state: &AppState, kind: CheckKind, folder: &str) -> AppResult<String> {
    let dir = Path::new(folder);
    if !dir.is_dir() {
        return Err(error("check_no_folder", "That folder is not there."));
    }
    let lib = state.library()?;
    // Never into the library: a report is for the person, and the library is Lumen's.
    if dir.starts_with(&lib.root) {
        return Err(error("check_report_in_library", "Choose a folder outside the library."));
    }
    let text = report_text(state, kind)?;
    let stamp = chrono::Local::now().format("%Y-%m-%d %H%M");
    let name = match kind {
        CheckKind::Folder => format!("Lumen folder check {stamp}.txt"),
        CheckKind::Library => format!("Lumen library check {stamp}.txt"),
    };
    let path = dir.join(name);
    std::fs::write(&path, text).map_err(|e| AppError { code: "check_report_write".into(), message: "Could not save the report there.".into(), detail: Some(e.to_string()) })?;
    Ok(path.to_string_lossy().into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pages_walk_a_fixed_list_without_repeating_or_skipping() {
        let (from, to, next) = page_bounds(5, None, 2);
        assert_eq!((from, to, next), (0, 2, Some(1)));
        let (from, to, next) = page_bounds(5, next, 2);
        assert_eq!((from, to, next), (2, 4, Some(3)));
        let (from, to, next) = page_bounds(5, next, 2);
        assert_eq!((from, to, next), (4, 5, None));
        assert_eq!(page_bounds(0, None, 50), (0, 0, None));
    }

    #[test]
    fn every_reason_the_check_can_give_has_a_sentence() {
        for code in ["not_imported", "in_trash", "copy_missing", "copy_differs", "copy_unreadable", "cannot_read", "sidecar_not_kept", "failed_verify"] {
            assert_ne!(reason_text(code), "unknown reason", "{code}");
        }
    }
}
