//! *Restart Lumen* (Round 15), lightest first. Every job and Denoise run is cancelled, and if
//! the backend answers again within `RECOVER_WAIT` nothing restarts. Otherwise unsaved Develop
//! edits are written, the screen and the window's place are saved, and a new Lumen is started
//! before this one ends itself: a thread stuck in a driver can only be freed by ending the
//! process.
//!
//! Tauri's own `restart()` is not used: off the main thread it only asks the event loop to
//! exit, so a stuck main thread would never restart. The new process is spawned here, then
//! this one ends; its FFmpeg children end with it (`child_jobs`), the new Lumen does not.

use std::path::Path;
use std::sync::atomic::Ordering;
use std::sync::RwLock;
use std::time::{Duration, Instant};

use photos_core::settings::Settings;
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};
use ts_rs::TS;

use crate::state::AppState;

/// How long the in-place recovery waits for the backend to answer again.
pub const RECOVER_WAIT: Duration = Duration::from_secs(3);
/// How long a Lumen started by *Restart* waits for the old one (and its WebView2 browser
/// process, which holds the same profile folder) to be gone.
pub const HANDOVER_WAIT: Duration = Duration::from_secs(5);
/// `<old Lumen pid>[,<its WebView2 browser pid>]`, set on the new process.
const RESTARTED_FROM: &str = "LUMEN_RESTARTED_FROM";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[ts(export)]
pub enum RestartStep {
    /// The backend answers again: nothing restarts.
    Recovered,
    /// Unsaved Develop edits are written first, then Lumen relaunches.
    Flush,
    /// Lumen relaunches. Heard by the page only when starting the new Lumen failed.
    Relaunch,
}

pub fn plan(recovered: bool, unsaved: bool) -> RestartStep {
    match (recovered, unsaved) {
        (true, _) => RestartStep::Recovered,
        (false, true) => RestartStep::Flush,
        (false, false) => RestartStep::Relaunch,
    }
}

/// The location *Restart* saved, once: it is cleared in memory and on disk.
pub fn take_resume(settings: &RwLock<Settings>, file: &Path) -> Option<String> {
    let mut s = settings.write().unwrap_or_else(|e| e.into_inner());
    let resume = s.window.resume.take()?;
    if let Err(e) = s.save(file) {
        tracing::warn!("restart: could not clear the saved location: {e:?}");
    }
    Some(resume)
}

/// The pids in `LUMEN_RESTARTED_FROM`; anything unreadable is skipped.
pub fn parse_handover(v: &str) -> Vec<u32> {
    v.split(',').filter_map(|p| p.trim().parse().ok()).collect()
}

/// At startup, before anything opens the library or the WebView2 profile: a Lumen started
/// by *Restart* waits for the old one to be gone. Its browser process is ended if it lingers,
/// since the new window cannot share the profile with a process that is shutting down.
pub fn wait_for_previous() {
    let Some(v) = std::env::var_os(RESTARTED_FROM) else { return };
    let started = Instant::now();
    let until = started + HANDOVER_WAIT;
    for (i, pid) in parse_handover(&v.to_string_lossy()).into_iter().enumerate() {
        let left = until.saturating_duration_since(Instant::now());
        if photos_core::child_jobs::wait_for_exit(pid, left) {
            continue;
        }
        if i > 0 && photos_core::child_jobs::end_process(pid, Duration::from_secs(1)) {
            tracing::warn!(pid, "restart: ended the old WebView2 browser process");
        } else {
            tracing::warn!(pid, "restart: the old Lumen is still running");
        }
    }
    tracing::info!(ms = started.elapsed().as_millis() as u64, "restart: took over from the old Lumen");
}

/// The work behind `restart_lumen`, on its own thread. Returns only when Lumen stays
/// (`Recovered`, or `Relaunch` when the new Lumen could not be started).
pub fn run(app: &AppHandle, location: String) -> RestartStep {
    let state = app.state::<AppState>();
    for j in state.jobs.running() {
        state.jobs.cancel(&j.key);
    }
    crate::denoise::cancel_all();
    let step = plan(recover(), state.unsaved_edits.load(Ordering::SeqCst));
    tracing::warn!(?step, "restart");
    match step {
        RestartStep::Recovered => return step,
        RestartStep::Flush => flush(app, &state),
        RestartStep::Relaunch => {}
    }
    relaunch(app, &state, location);
    RestartStep::Relaunch
}

/// Whether the backend answers again within `RECOVER_WAIT`: the async pool runs a task, and
/// the watchdog sees neither the main thread nor the database stuck.
fn recover() -> bool {
    let until = Instant::now() + RECOVER_WAIT;
    let (tx, rx) = std::sync::mpsc::channel();
    tauri::async_runtime::spawn(async move {
        let _ = tx.send(());
    });
    let pool = rx.recv_timeout(RECOVER_WAIT).is_ok();
    while pool && !crate::watchdog::nothing_blocking() && Instant::now() < until {
        std::thread::sleep(Duration::from_millis(100));
    }
    pool && crate::watchdog::nothing_blocking()
}

/// Asks the page to write its Develop edits without closing (`false`), and waits for it up
/// to `FLUSH_WAIT`: a page that does not answer cannot hold the restart.
fn flush(app: &AppHandle, state: &AppState) {
    let _ = app.emit(crate::events::FLUSH_REQUESTED, false);
    let until = Instant::now() + crate::window::FLUSH_WAIT;
    while state.unsaved_edits.load(Ordering::SeqCst) && Instant::now() < until {
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// Saves the screen and the window's place, stops the jobs, starts the new Lumen and ends
/// this one. Returns only if the new Lumen could not be started.
fn relaunch(app: &AppHandle, state: &AppState, location: String) {
    // The window getters (and the browser pid) wait on the main thread: only ask when it answers.
    let main_ok = crate::watchdog::main_thread_answering();
    let window = app.get_webview_window("main").filter(|_| main_ok);
    let geometry = window
        .as_ref()
        .map(|w| {
            (w.is_maximized().unwrap_or(false), w.is_minimized().unwrap_or(false), w.inner_size().ok(), w.outer_position().ok())
        });
    let mut handover = vec![std::process::id()];
    #[cfg(windows)]
    if let Some(w) = &window {
        handover.extend(crate::watchdog::webview_pids(
            w,
            webview2_com::Microsoft::Web::WebView2::Win32::COREWEBVIEW2_PROCESS_KIND_BROWSER,
        ));
    }
    let file = state.paths.settings_file();
    let until = Instant::now() + Duration::from_secs(1);
    let saved = loop {
        let saved = crate::window::save_window_state_with(&state.settings, |s| {
            s.window.resume = Some(location.clone());
            if let Some((maximized, minimized, size, pos)) = geometry {
                crate::window::remember_geometry(&mut s.window, maximized, minimized, size, pos);
            }
            if let Err(e) = s.save(&file) {
                tracing::warn!("restart: could not save the location: {e:?}");
            }
        });
        if saved || Instant::now() >= until {
            break saved;
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    if !saved {
        tracing::warn!("restart: settings were busy; the new Lumen opens on its usual screen");
    }
    state.jobs.shutdown();
    let env = app.env();
    let spawned = tauri::process::current_binary(&env).and_then(|exe| {
        std::process::Command::new(exe)
            // A restart comes back in front, never minimised or closing (Round 18 A2's launch flags).
            .args(env.args_os.iter().skip(1).filter(|a| !matches!(a.to_str(), Some("--minimized" | "--quit"))))
            .env(RESTARTED_FROM, handover.iter().map(u32::to_string).collect::<Vec<_>>().join(","))
            .spawn()
    });
    match spawned {
        Ok(child) => {
            tracing::warn!(pid = child.id(), "restart: started the new Lumen; ending this one");
            photos_core::child_jobs::end_this_process();
        }
        Err(e) => tracing::error!("restart: could not start a new Lumen: {e}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn restart_flushes_then_saves_the_location() {
        assert_eq!(plan(true, true), RestartStep::Recovered, "recovered in place: nothing restarts");
        assert_eq!(plan(true, false), RestartStep::Recovered);
        assert_eq!(plan(false, true), RestartStep::Flush);
        assert_eq!(plan(false, false), RestartStep::Relaunch);
    }

    #[test]
    fn resume_is_read_once() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("settings.json");
        let mut s = Settings::default();
        s.window.resume = Some("/develop/12?panel=detail".into());
        s.save(&file).unwrap();
        let lock = RwLock::new(Settings::load(&file).unwrap());
        assert_eq!(take_resume(&lock, &file).as_deref(), Some("/develop/12?panel=detail"));
        assert_eq!(take_resume(&lock, &file), None);
        assert_eq!(Settings::load(&file).unwrap().window.resume, None, "still on disk");
        assert!(!std::fs::read_to_string(&file).unwrap().contains("resume"));
    }

    #[test]
    fn the_handover_lists_the_old_pids() {
        assert_eq!(parse_handover("1234"), vec![1234]);
        assert_eq!(parse_handover("1234,5678"), vec![1234, 5678]);
        assert_eq!(parse_handover("x,5678,"), vec![5678]);
    }
}
