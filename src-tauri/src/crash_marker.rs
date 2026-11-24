//! A `running.json` marker (Round 24 §6.4): written when Lumen starts, removed on a clean exit.
//! One found at the next start means the last run did not end by itself (a crash, a kill, a
//! stuck start ended by hand), and the page opens Home with an alert instead of resuming.
//!
//! The marker holds the last route the page reported, so the alert can offer *Back to Develop*.
//! A run that starts from a crash carries that route forward until the page reports its own, so
//! two crashes in a row still say "crashed" and still know where the person was; the route is
//! never followed automatically (`resume_info` only reports it).

use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use serde::{Deserialize, Serialize};
use ts_rs::TS;

const FILE_NAME: &str = "running.json";

/// What the previous run left behind.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CrashInfo {
    pub route: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct Marker {
    route: Option<String>,
    pid: u32,
}

/// Why this run opens where it does: `normal` launches open as always, `crash` and `restart`
/// open Home with the alert (`route` is only for its *Back to …* button).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ResumeInfo {
    pub reason: ResumeReason,
    pub route: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum ResumeReason {
    Normal,
    Crash,
    Restart,
}

/// A restart that Lumen asked for itself (*Restart Lumen*, the watchdog) wins over a marker the
/// old process left behind when it ended; the marker alone means a crash.
pub fn resume_info(restarted: Option<String>, crashed: Option<CrashInfo>) -> ResumeInfo {
    match (restarted, crashed) {
        (Some(route), _) => ResumeInfo { reason: ResumeReason::Restart, route: Some(route) },
        (None, Some(c)) => ResumeInfo { reason: ResumeReason::Crash, route: c.route },
        (None, None) => ResumeInfo { reason: ResumeReason::Normal, route: None },
    }
}

fn read(file: &Path) -> Option<Marker> {
    let text = match std::fs::read_to_string(file) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return None,
        // A marker that exists but cannot be read still means the last run did not end cleanly.
        Err(_) => return Some(Marker { route: None, pid: 0 }),
    };
    Some(serde_json::from_str(&text).unwrap_or(Marker { route: None, pid: 0 }))
}

/// Temp file, then rename: a crash in the middle never leaves half a marker.
fn write(file: &Path, marker: &Marker) {
    let Ok(json) = serde_json::to_vec(marker) else { return };
    let tmp = file.with_extension("json.tmp");
    let result = std::fs::write(&tmp, json).and_then(|_| std::fs::rename(&tmp, file));
    if let Err(e) = result {
        tracing::warn!("crash marker: could not write {}: {e}", file.display());
        let _ = std::fs::remove_file(&tmp);
    }
}

/// Reads the previous run's marker and writes this run's, keeping the previous route as a hint.
fn begin(file: &Path) -> Option<CrashInfo> {
    let previous = read(file).map(|m| CrashInfo { route: m.route });
    write(file, &Marker { route: previous.as_ref().and_then(|c| c.route.clone()), pid: std::process::id() });
    previous
}

fn end(file: &Path) {
    if let Err(e) = std::fs::remove_file(file) {
        if e.kind() != std::io::ErrorKind::NotFound {
            tracing::warn!("crash marker: could not remove {}: {e}", file.display());
        }
    }
}

/// Only while a marker exists: a route reported during shutdown must not bring it back.
fn note_route(file: &Path, route: &str) {
    if let Some(mut m) = read(file) {
        m.route = Some(route.to_string());
        write(file, &m);
    }
}

struct Global {
    file: PathBuf,
    previous: Option<CrashInfo>,
}

static GLOBAL: OnceLock<Mutex<Global>> = OnceLock::new();

/// At launch, after the single-instance check: notes whether the last run crashed and marks this
/// one as running. `app_data` is `config_dir/JL-Studios/Lumen`.
pub fn start(app_data: &Path) {
    let file = app_data.join(FILE_NAME);
    let previous = begin(&file);
    if previous.is_some() {
        tracing::warn!(route = ?previous.as_ref().and_then(|c| c.route.as_deref()), "the last run did not end cleanly");
    }
    let _ = GLOBAL.set(Mutex::new(Global { file, previous }));
}

/// The page's current screen, so the next start can offer to go back to it.
pub fn set_route(route: &str) {
    if let Some(g) = GLOBAL.get() {
        let g = g.lock().unwrap_or_else(|e| e.into_inner());
        note_route(&g.file, route);
    }
}

/// The normal shutdown: the marker goes. Safe to call more than once.
pub fn clean_exit() {
    if let Some(g) = GLOBAL.get() {
        let g = g.lock().unwrap_or_else(|e| e.into_inner());
        end(&g.file);
    }
}

/// What the last run left, once: the page asks at boot, and a reloaded page asks again and
/// finds nothing, so a reload never shows the alert twice.
pub fn take_previous() -> Option<CrashInfo> {
    let g = GLOBAL.get()?;
    g.lock().unwrap_or_else(|e| e.into_inner()).previous.take()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn marker_file() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join(FILE_NAME);
        (dir, file)
    }

    #[test]
    fn clean_exit_removes_marker() {
        let (_dir, file) = marker_file();
        assert_eq!(begin(&file), None, "a first start has nothing before it");
        assert!(file.exists(), "a running Lumen leaves its marker");
        end(&file);
        assert!(!file.exists());
        assert_eq!(begin(&file), None, "after a clean exit the next start is normal");
        end(&file); // twice is fine
        end(&file);
    }

    #[test]
    fn marker_found_means_crashed() {
        let (_dir, file) = marker_file();
        begin(&file);
        note_route(&file, "/develop/12?panel=detail");
        let crashed = begin(&file).expect("the marker was still there");
        assert_eq!(crashed.route.as_deref(), Some("/develop/12?panel=detail"));
    }

    #[test]
    fn marker_twice_goes_home() {
        let (_dir, file) = marker_file();
        begin(&file);
        note_route(&file, "/develop/12");
        // The run after the first crash ends before its page reports a route, then crashes too.
        assert!(begin(&file).is_some(), "first crash");
        let second = begin(&file).expect("second crash still reports crashed");
        assert_eq!(second.route.as_deref(), Some("/develop/12"), "the route is carried, not lost");
        // Whatever the markers say, a crash is only ever `crash`: it never resumes by itself.
        assert_eq!(resume_info(None, Some(second)).reason, ResumeReason::Crash);
    }

    #[test]
    fn a_route_after_shutdown_does_not_bring_the_marker_back() {
        let (_dir, file) = marker_file();
        begin(&file);
        end(&file);
        note_route(&file, "/library");
        assert!(!file.exists());
    }

    #[test]
    fn an_unreadable_marker_still_means_crashed() {
        let (_dir, file) = marker_file();
        std::fs::write(&file, b"{ not json").unwrap();
        assert_eq!(begin(&file), Some(CrashInfo { route: None }));
    }

    #[test]
    fn a_restart_lumen_asked_for_is_a_restart_not_a_crash() {
        let crashed = Some(CrashInfo { route: Some("/library".into()) });
        let r = resume_info(Some("/develop/3".into()), crashed.clone());
        assert_eq!((r.reason, r.route.as_deref()), (ResumeReason::Restart, Some("/develop/3")));
        let r = resume_info(None, crashed);
        assert_eq!((r.reason, r.route.as_deref()), (ResumeReason::Crash, Some("/library")));
        let r = resume_info(None, None);
        assert_eq!((r.reason, r.route), (ResumeReason::Normal, None));
    }
}
