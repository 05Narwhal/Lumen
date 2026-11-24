//! Round 15 stall watchdog: what is stuck, and for how long. One normal-priority thread wakes
//! every second, reads the main thread's lag, the longest database lease and the running jobs,
//! logs each new stall once (`stuck`) and its end (`unstuck`), and sends `app://stall` whenever
//! the set changes. The Denoise freeze of 2026-10-08 left no log line at all: `Db::with_conn`
//! only warns once a wait ends, and a permanent wait never ends.

use std::collections::BTreeSet;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use photos_jobs::{Lane, RunningJob};
use tauri::{AppHandle, Emitter, Manager};

use crate::events::{Stall, StallKind, StallReport, STALL};
use crate::state::AppState;

/// The main thread answering a probe later than this is a stall (the window stops repainting).
pub const MAIN_LAG_MS: u64 = 3_000;
/// A database connection held longer than this.
pub const LEASE_MS: u64 = 10_000;
/// A job the user is waiting on (Interactive lane) running longer than this.
pub const INTERACTIVE_JOB_MS: u64 = 15_000;
/// A Denoise run with no progress for this long (its `for_ms` counts from the last tile).
pub const DENOISE_SILENT_MS: u64 = 90_000;

const TICK: Duration = Duration::from_secs(1);
/// After the process finds itself resumed from sleep, page and backend checks wait this long:
/// the page's timers resume later than this thread, and every timer spans the sleep.
pub const WAKE_GRACE: Duration = Duration::from_secs(30);

/// The process itself was suspended (the PC slept): the gap since the previous tick is far
/// beyond one tick. `Instant` counts time asleep on Windows, so every age now includes it.
pub fn woke(prev: Instant, now: Instant) -> bool {
    now.saturating_duration_since(prev) > 5 * TICK
}

/// Whether `now` is still inside the grace that began at `wake`.
pub fn in_grace(wake: Option<Instant>, now: Instant) -> bool {
    matches!(wake, Some(t) if now.saturating_duration_since(t) < WAKE_GRACE)
}

/// The stalls among what is running, in the order main thread, database, jobs. Background
/// lanes have no limit: a long transcode on the maintenance lane is work, not a stall.
pub fn stalls(jobs: &[RunningJob], lease: Option<(u64, String)>, main_lag_ms: Option<u64>) -> Vec<Stall> {
    let mut out = Vec::new();
    if let Some(ms) = main_lag_ms.filter(|ms| *ms > MAIN_LAG_MS) {
        out.push(Stall { what: "main thread".into(), kind: StallKind::MainThread, for_ms: ms });
    }
    if let Some((ms, at)) = lease.filter(|(ms, _)| *ms > LEASE_MS) {
        out.push(Stall { what: format!("database call at {at}"), kind: StallKind::Database, for_ms: ms });
    }
    for j in jobs {
        let limit = if j.key.starts_with("denoise:") {
            Some(DENOISE_SILENT_MS)
        } else if j.lane == Lane::Interactive {
            Some(INTERACTIVE_JOB_MS)
        } else {
            None
        };
        if limit.is_some_and(|l| j.for_ms > l) {
            out.push(Stall { what: format!("job {}", j.key), kind: StallKind::Job, for_ms: j.for_ms });
        }
    }
    out
}

pub fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

/// The last probe the main thread ran.
static ANSWERED: AtomicU64 = AtomicU64::new(0);
/// The last tick: the main thread had answered its probe / nothing froze the window or the
/// database. What *Restart Lumen* reads (`restart::run`).
static MAIN_OK: AtomicBool = AtomicBool::new(true);
static CLEAR: AtomicBool = AtomicBool::new(true);

/// Whether the main thread answered the last tick's probe, so a window getter will not hang.
pub fn main_thread_answering() -> bool {
    MAIN_OK.load(Ordering::SeqCst)
}

/// Whether the last tick saw neither a main-thread nor a database stall.
pub fn nothing_blocking() -> bool {
    CLEAR.load(Ordering::SeqCst)
}

pub fn start(app: AppHandle) {
    std::thread::Builder::new()
        .name("lumen-watchdog".into())
        .spawn(move || {
            // (probe number, posted at) of the probe the main thread has not run yet
            let mut posted: Option<(u64, u64)> = None;
            let mut seq = 0u64;
            let mut last: BTreeSet<(String, &'static str)> = BTreeSet::new();
            let mut last_reload: Option<Instant> = None;
            let mut prev_tick = Instant::now();
            let mut wake_at: Option<Instant> = None;
            loop {
                std::thread::sleep(TICK);
                let tick_at = Instant::now();
                if woke(prev_tick, tick_at) {
                    // Every age (a beat, a probe, a lease, a job) now includes the sleep.
                    tracing::info!(gap_s = tick_at.saturating_duration_since(prev_tick).as_secs(), "woke: checks paused");
                    wake_at = Some(tick_at);
                    posted = None;
                    reset_beat(tick_at);
                }
                prev_tick = tick_at;
                let quiet = in_grace(wake_at, tick_at);
                let now = now_ms();
                let raw_lag = match posted {
                    Some((n, at)) if ANSWERED.load(Ordering::SeqCst) < n => Some(now.saturating_sub(at)),
                    _ => {
                        seq += 1;
                        let n = seq;
                        posted = Some((n, now));
                        let _ = app.run_on_main_thread(move || ANSWERED.store(n, Ordering::SeqCst));
                        None
                    }
                };
                let state = app.state::<AppState>();
                let mut jobs = state.jobs.running();
                jobs.extend(crate::denoise::denoise_running());
                // Never wait here: a writer holding the library lock is itself what may be stuck.
                let lease = state.library.try_read().ok().and_then(|l| l.as_ref().and_then(|l| l.db.longest_lease()));
                let lag = raw_lag.filter(|_| !quiet);
                let now_stalls = if quiet { Vec::new() } else { stalls(&jobs, lease, lag) };
                MAIN_OK.store(raw_lag.is_none(), Ordering::SeqCst);
                CLEAR.store(
                    !now_stalls.iter().any(|s| matches!(s.kind, StallKind::MainThread | StallKind::Database)),
                    Ordering::SeqCst,
                );
                let set: BTreeSet<(String, &'static str)> =
                    now_stalls.iter().map(|s| (s.what.clone(), kind_name(s.kind))).collect();
                for s in &now_stalls {
                    if !last.contains(&(s.what.clone(), kind_name(s.kind))) {
                        tracing::warn!(what = %s.what, for_ms = s.for_ms, "stuck");
                    }
                }
                for (what, _) in last.difference(&set) {
                    tracing::info!(what = %what, "unstuck");
                }
                if set != last {
                    let _ = app.emit(STALL, &StallReport { stalls: now_stalls });
                    last = set;
                }
                // The window getters wait on the main thread, so only ask once it has answered.
                if raw_lag.is_none() && !quiet {
                    check_page(&app, &mut last_reload, wake_at);
                }
            }
        })
        .expect("spawn watchdog");
}

/// The page not beating for this long while shown is a frozen page (its script never yields).
pub const UI_FROZEN: Duration = Duration::from_secs(10);
/// At most one reload this often, so a page that freezes as it starts is not reloaded forever.
const RELOAD_EVERY: Duration = Duration::from_secs(60);

/// The page's last heartbeat and whether it was hidden then; `None` until the first one.
static UI_BEAT: Mutex<Option<(Instant, bool)>> = Mutex::new(None);

/// `ui_heartbeat`: the page is alive (every 2 s, and at once when it is hidden or shown).
pub fn ui_beat(hidden: bool) {
    ui_beat_at(Instant::now(), hidden);
}

/// A hidden page's timers are throttled to about one a minute, so its silence means nothing.
pub fn ui_frozen(last_beat: Instant, last_hidden: bool, now: Instant) -> bool {
    !last_hidden && now.saturating_duration_since(last_beat) > UI_FROZEN
}

/// How long the page may be silent before a Close stops asking it anything (Round 24 §6.3).
const PAGE_DEAD_AFTER: Duration = Duration::from_secs(10);

/// Whether a page that beat at `last` (`None`: never) can still answer a question. A hidden page's
/// timers are throttled, so its silence says nothing; a page that never beat cannot answer.
pub fn page_answers(last: Option<(Instant, bool)>, now: Instant) -> bool {
    match last {
        None => false,
        Some((_, true)) => true,
        Some((at, false)) => now.saturating_duration_since(at) <= PAGE_DEAD_AFTER,
    }
}

/// Close asks the page (unsaved Develop edits, a running import) only while this holds: a stuck
/// page must never keep the window open (Round 24 §6.3).
pub fn page_is_answering() -> bool {
    page_answers(*UI_BEAT.lock().unwrap_or_else(|e| e.into_inner()), Instant::now())
}

fn may_reload(last_reload: Option<Instant>, now: Instant) -> bool {
    !matches!(last_reload, Some(t) if now.saturating_duration_since(t) < RELOAD_EVERY)
}

/// Whether to end the renderer and reload now. `wake` is when the process last found itself
/// resumed from sleep (`woke`).
fn should_reload(beat: Instant, hidden: bool, now: Instant, last_reload: Option<Instant>, wake: Option<Instant>) -> bool {
    !in_grace(wake, now) && ui_frozen(beat, hidden, now) && may_reload(last_reload, now)
}

/// A wake: the page's last beat is "now", whatever the clock says, and stays as hidden as it was.
fn reset_beat(now: Instant) {
    if let Some((_, hidden)) = *UI_BEAT.lock().unwrap_or_else(|e| e.into_inner()) {
        ui_beat_at(now, hidden);
    }
}

fn ui_beat_at(at: Instant, hidden: bool) {
    *UI_BEAT.lock().unwrap_or_else(|e| e.into_inner()) = Some((at, hidden));
}

/// The WebView2 processes of kind `want` behind `w` (Windows): the renderers for a frozen
/// page, the browser process for *Restart*. Asked on the main thread, which must have just
/// answered a probe; waits for the answer at most a second.
#[cfg(windows)]
pub fn webview_pids(
    w: &tauri::WebviewWindow,
    want: webview2_com::Microsoft::Web::WebView2::Win32::COREWEBVIEW2_PROCESS_KIND,
) -> Vec<u32> {
    use webview2_com::Microsoft::Web::WebView2::Win32::{ICoreWebView2Environment8, ICoreWebView2_2, COREWEBVIEW2_PROCESS_KIND};
    use windows_core::Interface;
    let (tx, rx) = std::sync::mpsc::channel();
    let asked = w.with_webview(move |wv| {
        // SAFETY: COM calls on the main thread, on interfaces the controller hands out.
        let pids = unsafe {
            (|| -> windows_core::Result<Vec<u32>> {
                let env = wv.controller().CoreWebView2()?.cast::<ICoreWebView2_2>()?.Environment()?;
                let infos = env.cast::<ICoreWebView2Environment8>()?.GetProcessInfos()?;
                let mut n = 0u32;
                infos.Count(&mut n)?;
                let mut pids = Vec::new();
                for i in 0..n {
                    let info = infos.GetValueAtIndex(i)?;
                    let (mut kind, mut pid) = (COREWEBVIEW2_PROCESS_KIND::default(), 0i32);
                    info.Kind(&mut kind)?;
                    info.ProcessId(&mut pid)?;
                    if kind == want {
                        pids.push(pid as u32);
                    }
                }
                Ok(pids)
            })()
        };
        let _ = tx.send(pids.unwrap_or_else(|e| {
            tracing::warn!("could not list the WebView2 processes: {e}");
            Vec::new()
        }));
    });
    if asked.is_err() {
        return Vec::new();
    }
    rx.recv_timeout(Duration::from_secs(1)).unwrap_or_default()
}

/// Reloads a frozen page. `reload` keeps the URL, so the route and `?asset=` survive. A reload
/// alone never reaches a renderer stuck in a script (checked 2026-10-09: the page stayed
/// frozen through four), so on Windows the renderer is ended first and the reload starts a
/// fresh one.
fn check_page(app: &AppHandle, last_reload: &mut Option<Instant>, wake: Option<Instant>) {
    let Some((beat, hidden)) = *UI_BEAT.lock().unwrap_or_else(|e| e.into_inner()) else { return };
    let now = Instant::now();
    if !should_reload(beat, hidden, now, *last_reload, wake) {
        return;
    }
    let Some(w) = app.get_webview_window("main") else { return };
    // Minimized after the page froze: it could not say so, and nobody is looking anyway.
    if !w.is_visible().unwrap_or(false) || w.is_minimized().unwrap_or(true) {
        return;
    }
    #[cfg(windows)]
    let ended = webview_pids(&w, webview2_com::Microsoft::Web::WebView2::Win32::COREWEBVIEW2_PROCESS_KIND_RENDERER)
        .into_iter()
        .filter(|&pid| photos_core::child_jobs::end_process(pid, Duration::from_secs(1)))
        .count();
    #[cfg(not(windows))]
    let ended = 0;
    tracing::warn!(silent_ms = now.saturating_duration_since(beat).as_millis() as u64, ended, "page frozen: reloaded");
    *last_reload = Some(now);
    // The new page gets a full UI_FROZEN to start beating.
    ui_beat(false);
    let _ = w.reload();
}

fn kind_name(k: StallKind) -> &'static str {
    match k {
        StallKind::MainThread => "main",
        StallKind::Database => "db",
        StallKind::Job => "job",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_what_is_past_its_limit_is_a_stall() {
        let job = |key: &str, lane, for_ms| RunningJob { key: key.into(), lane, for_ms };
        let jobs = [
            job("thumb:1", Lane::Interactive, 16_000),
            job("proxy:ab", Lane::Maintenance, 3_600_000),
            job("thumb:2", Lane::Interactive, 200),
        ];
        let s = stalls(&jobs, Some((12_000, "tag_search.rs:621".into())), Some(4_000));
        let kinds: Vec<_> = s.iter().map(|s| s.kind).collect();
        assert_eq!(kinds, vec![StallKind::MainThread, StallKind::Database, StallKind::Job]);
        assert!(s[2].what.contains("thumb:1"));
        // A long transcode on the maintenance lane is work, not a stall.
        assert!(!s.iter().any(|s| s.what.contains("proxy:")));
    }

    #[test]
    fn a_silent_denoise_is_a_stall_whatever_its_lane() {
        let s = stalls(&[RunningJob { key: "denoise:7".into(), lane: Lane::Import, for_ms: 91_000 }], None, None);
        assert_eq!(s.len(), 1);
        assert!(stalls(&[RunningJob { key: "denoise:7".into(), lane: Lane::Import, for_ms: 60_000 }], None, None).is_empty());
    }

    #[test]
    fn a_hidden_page_is_never_judged_frozen() {
        let t = Instant::now();
        assert!(!ui_frozen(t, true, t + Duration::from_secs(3600)), "hidden: timers are throttled");
        assert!(!ui_frozen(t, false, t + Duration::from_secs(9)));
        assert!(ui_frozen(t, false, t + Duration::from_secs(11)));
    }

    #[test]
    fn a_tick_gap_far_past_the_tick_is_a_wake() {
        let t = Instant::now();
        assert!(!woke(t, t + Duration::from_secs(1)), "an ordinary tick");
        assert!(!woke(t, t + Duration::from_secs(4)), "a busy machine, not a sleep");
        assert!(woke(t, t + Duration::from_secs(6)));
        assert!(woke(t, t + Duration::from_secs(2 * 3600)), "two hours asleep");
    }

    #[test]
    fn the_page_is_not_reloaded_for_the_time_the_pc_slept() {
        // Verify (spec §12): the beat is two hours old when the first tick after waking runs.
        // Without the grace this reloads, which is the bug.
        let beat = Instant::now();
        let now = beat + Duration::from_secs(2 * 3600);
        assert!(ui_frozen(beat, false, now), "the plain rule calls the page frozen");
        assert!(!should_reload(beat, false, now, None, Some(now)), "but not just after a wake");
        // Still inside the grace 29 s later, with the page's beat as stale as ever.
        let later = now + Duration::from_secs(29);
        assert!(!should_reload(beat, false, later, None, Some(now)));
    }

    #[test]
    fn a_real_freeze_after_the_grace_still_reloads() {
        let woke_at = Instant::now();
        // The wake reset the beat to `woke_at`; the page never beat again.
        let beat = woke_at;
        let now = woke_at + WAKE_GRACE + Duration::from_secs(11);
        assert!(should_reload(beat, false, now, None, Some(woke_at)));
        // Never woken: the old rule holds.
        assert!(should_reload(beat, false, beat + Duration::from_secs(11), None, None));
        // Reload budget still applies.
        assert!(!should_reload(beat, false, now, Some(now - Duration::from_secs(5)), Some(woke_at)));
    }

    #[test]
    fn the_grace_lasts_thirty_seconds() {
        let t = Instant::now();
        assert!(in_grace(Some(t), t + Duration::from_secs(29)));
        assert!(!in_grace(Some(t), t + Duration::from_secs(31)));
        assert!(!in_grace(None, t));
    }

    #[test]
    fn a_frozen_page_is_reloaded_at_most_once_a_minute() {
        let t = Instant::now();
        assert!(may_reload(None, t));
        assert!(!may_reload(Some(t), t + Duration::from_secs(59)));
        assert!(may_reload(Some(t), t + Duration::from_secs(61)));
    }

    #[test]
    fn close_stops_asking_a_silent_page() {
        let t = Instant::now();
        let later = |s| t + Duration::from_secs(s);
        assert!(!page_answers(None, t), "a page that never beat cannot answer");
        assert!(page_answers(Some((t, false)), later(5)));
        assert!(!page_answers(Some((t, false)), later(11)), "silent for over 10 s: closing does not wait for it");
        assert!(page_answers(Some((t, true)), later(600)), "a hidden page's timers are throttled");
    }
}
