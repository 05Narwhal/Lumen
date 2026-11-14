//! Library backup (Round 5A): runs `photos_core::backup` as one job on the Import lane,
//! by hand or on its schedule, and reports to the activity ring (`backup://progress`,
//! `backup://finished`). Each library's settings and last run live in `backups.json`.
//! A missing destination or a full drive is a state the Backup settings show, never an error.

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use photos_core::backup::{self, BackupConfig, BackupConfigs, BackupProgress, BackupRun, Schedule, Source};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};
use ts_rs::TS;

use crate::error::{AppError, AppResult};
use crate::state::AppState;

pub const BACKUP_PROGRESS: &str = "backup://progress";
pub const BACKUP_FINISHED: &str = "backup://finished";

/// `backups.json` is read and written by commands and the job alike.
static FILE: Mutex<()> = Mutex::new(());

/// What the Backup settings show.
#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct BackupStatus {
    pub config: BackupConfig,
    /// Where the backup is (or will be): the library's folder name inside the destination.
    pub target: Option<String>,
    /// Whether the destination is there now (a drive may be unplugged).
    pub available: bool,
    pub running: bool,
}

fn job_key(library_id: &str) -> String {
    format!("backup:{library_id}")
}

fn update(state: &AppState, library_id: &str, f: impl FnOnce(&mut BackupConfig)) -> AppResult<BackupConfig> {
    let _held = FILE.lock().unwrap_or_else(|p| p.into_inner());
    let path = state.paths.backups_file();
    let mut all = BackupConfigs::load(&path);
    let config = all.libraries.entry(library_id.to_string()).or_default();
    f(config);
    let out = config.clone();
    all.save(&path)?;
    Ok(out)
}

fn config_of(state: &AppState, library_id: &str) -> BackupConfig {
    let _held = FILE.lock().unwrap_or_else(|p| p.into_inner());
    BackupConfigs::load(&state.paths.backups_file()).libraries.get(library_id).cloned().unwrap_or_default()
}

pub fn status(state: &AppState) -> AppResult<BackupStatus> {
    let lib = state.library()?;
    let config = config_of(state, &lib.manifest.id);
    let target = config.destination.as_ref().map(|d| backup::target_for(&lib.root, Path::new(d)));
    Ok(BackupStatus {
        available: config.destination.as_ref().is_some_and(|d| Path::new(d).is_dir()),
        target: target.map(|t| t.to_string_lossy().into_owned()),
        running: state.jobs.is_inflight(&job_key(&lib.manifest.id)),
        config,
    })
}

/// Sets (or clears) the destination, refusing one that would put the library at risk.
pub fn set_destination(state: &AppState, destination: Option<String>) -> AppResult<BackupStatus> {
    let lib = state.library()?;
    if let Some(d) = &destination {
        backup::check_destination(&lib.root, &lib.manifest.id, &backup::target_for(&lib.root, Path::new(d)))?;
    }
    update(state, &lib.manifest.id, |c| c.destination = destination)?;
    status(state)
}

pub fn set_schedule(state: &AppState, schedule: Schedule) -> AppResult<BackupStatus> {
    let lib = state.library()?;
    update(state, &lib.manifest.id, |c| c.schedule = schedule)?;
    status(state)
}

fn no_destination() -> AppError {
    AppError { code: "backup_no_destination".into(), message: "Choose where to back up first.".into(), detail: None }
}

/// Starts a backup of the open library now. One at a time per library.
pub fn start(app: &AppHandle) -> AppResult<()> {
    let state = app.state::<AppState>();
    let lib = state.library()?;
    let config = config_of(&state, &lib.manifest.id);
    let Some(destination) = config.destination.clone() else { return Err(no_destination()) };
    let a = app.clone();
    state.jobs.submit(&job_key(&lib.manifest.id), photos_jobs::Lane::Import, move |cancel| {
        let state = a.state::<AppState>();
        let now = photos_core::now_unix();
        let chosen = PathBuf::from(&destination);
        let outcome = if !chosen.is_dir() {
            Err("The backup drive or folder is not there. Connect it and try again.".to_string())
        } else {
            let target = backup::target_for(&lib.root, &chosen);
            let source = Source { root: &lib.root, id: &lib.manifest.id, name: &lib.manifest.name, db: &lib.db };
            let free = crate::space::free_space(&chosen);
            let mut last_emit = Instant::now() - Duration::from_secs(1);
            let mut report = |p: BackupProgress| {
                // Four a second is plenty for a ring.
                if last_emit.elapsed() >= Duration::from_millis(250) || p.done_files == p.total_files {
                    last_emit = Instant::now();
                    let _ = a.emit(BACKUP_PROGRESS, p);
                }
            };
            tracing::info!(target = %target.display(), "backup started");
            backup::run(&source, &target, free, now, &|| cancel.is_cancelled(), &mut report).map_err(|e| match e {
                photos_core::CoreError::Invalid(m) => m,
                other => other.to_string(),
            })
        };
        let run = match outcome {
            Ok(r) => {
                tracing::info!(copied = r.copied, bytes = r.copied_bytes, unchanged = r.unchanged, retired = r.retired, failed = r.failed.len(), cancelled = r.cancelled, "backup finished");
                BackupRun { at: now, error: None, report: Some(r) }
            }
            Err(message) => {
                tracing::warn!("backup stopped: {message}");
                BackupRun { at: now, error: Some(message), report: None }
            }
        };
        let finished = run.error.is_none() && run.report.as_ref().is_some_and(|r| !r.cancelled);
        let _ = update(&state, &lib.manifest.id, |c| {
            if finished {
                c.last_success_at = Some(now);
            }
            c.last = Some(run.clone());
        });
        let _ = a.emit(BACKUP_FINISHED, &run);
    });
    Ok(())
}

pub fn cancel(state: &AppState) -> AppResult<()> {
    let lib = state.library()?;
    state.jobs.cancel(&job_key(&lib.manifest.id));
    Ok(())
}

/// Called by the maintenance tick: starts the open library's scheduled backup when it is due
/// and its destination is there. A drive that is not plugged in waits for a later tick.
pub fn run_if_due(app: &AppHandle) {
    let state = app.state::<AppState>();
    let Ok(lib) = state.library() else { return };
    let config = config_of(&state, &lib.manifest.id);
    let present = config.destination.as_ref().is_some_and(|d| Path::new(d).is_dir());
    if present && config.due(photos_core::now_unix()) && !state.jobs.is_inflight(&job_key(&lib.manifest.id)) {
        if let Err(e) = start(app) {
            tracing::warn!("scheduled backup: {}", e.message);
        }
    }
}

/// How often the schedule is looked at: often enough that a drive plugged in for an hour
/// gets its backup.
pub const CHECK_EVERY: Duration = Duration::from_secs(10 * 60);

/// Looks at the schedule every [`CHECK_EVERY`], from a minute after start-up.
pub fn start_schedule(app: AppHandle) {
    std::thread::Builder::new()
        .name("lumen-backup-schedule".into())
        .spawn(move || {
            std::thread::sleep(Duration::from_secs(60));
            loop {
                run_if_due(&app);
                std::thread::sleep(CHECK_EVERY);
            }
        })
        .expect("spawn backup schedule");
}

/// Before a folder is opened as a library: a backup whose library is still where it was is
/// refused, because opening it would repoint that library at the backup. With the original
/// gone, opening the backup is the restore. Returns whether `root` was a backup.
pub fn guard_open(state: &AppState, root: &Path) -> AppResult<bool> {
    let Some(marker) = backup::read_marker(root) else { return Ok(false) };
    let reg = state.registry.read().expect("registry lock");
    let original = reg.libraries.iter().find(|e| e.id == marker.library_id).map(|e| PathBuf::from(&e.path));
    match refusal(&marker, original.as_deref(), root) {
        Some(message) => Err(AppError { code: "library_is_backup".into(), message, detail: None }),
        None => Ok(true),
    }
}

/// Why opening the backup at `root` is refused, if it is: its library is registered at
/// `original` and still there.
fn refusal(marker: &backup::BackupMarker, original: Option<&Path>, root: &Path) -> Option<String> {
    let there = original?;
    (there != root && there.join("library.db").is_file()).then(|| {
        format!(
            "This is a backup of “{}”, which is still at {}. Open that library instead. A backup is opened only to restore, once the original is gone.",
            marker.library_name,
            there.display()
        )
    })
}

/// After a restore: the folder is the library now, no longer a backup of it.
pub fn forget_marker(root: &Path) {
    let _ = std::fs::remove_file(root.join(backup::MARKER_DIR).join("backup.json"));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_backup_opens_only_once_its_library_is_gone() {
        let original = tempfile::tempdir().unwrap();
        let copy = tempfile::tempdir().unwrap();
        let marker = backup::BackupMarker { library_id: "lib-1".into(), library_name: "Lumen Library".into(), source_path: "x".into(), finished_at: 1 };
        std::fs::write(original.path().join("library.db"), "").unwrap();
        // Its library is still there: opening the backup would repoint it, so no.
        assert!(refusal(&marker, Some(original.path()), copy.path()).is_some_and(|m| m.contains("Lumen Library")));
        // The library is gone (the disk died): this is the restore.
        std::fs::remove_file(original.path().join("library.db")).unwrap();
        assert_eq!(refusal(&marker, Some(original.path()), copy.path()), None);
        // Never registered here, or registered at this very folder: open it.
        assert_eq!(refusal(&marker, None, copy.path()), None);
        assert_eq!(refusal(&marker, Some(copy.path()), copy.path()), None);
    }
}
