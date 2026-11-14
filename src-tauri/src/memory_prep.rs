//! Music and cuts ready before the click (Round 21 §14). Play on a memory used to wait for the
//! whole build — the track chosen, listened to, the show cut to its beat — when the film was
//! not cached. This step does that work in the background, on the Maintenance lane:
//!
//! 1. every saved track gets its profile (`track_profiles`);
//! 2. the twelve newest memories without a film made from their present photos get one, built
//!    by exactly what Play runs (`commands::slideshows::film_for`) and stored where Play looks
//!    (`memory_films`);
//! 3. every saved slideshow whose music has not been analysed gets its beats (`analyse_track`),
//!    which is the slow part of cutting it to the beat.
//!
//! It starts 20 s after launch, waits while an import or a video preparation runs, and runs
//! again when a memory is created or a slideshow is saved. Nothing here changes which track is
//! chosen; missing FFmpeg or no music at all is a state, and the step does nothing. The pass
//! is idempotent: `photos_core::memory_films::missing` is empty once the films exist.

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::LazyLock;
use std::time::{Duration, Instant};

use photos_jobs::{CancelToken, Lane};
use tauri::{AppHandle, Manager};

use crate::state::AppState;

/// The ring's line while the step has work.
const LABEL: &str = "Preparing memories";
/// How long after launch the step may first run (the maintenance start-up grace's).
const GRACE: Duration = Duration::from_secs(20);
/// How often a waiting step looks again while an import or video preparation runs.
const RECHECK: Duration = Duration::from_secs(30);

static LAUNCHED: LazyLock<Instant> = LazyLock::new(Instant::now);
/// A thread is waiting to start the job.
static WAITING: AtomicBool = AtomicBool::new(false);
/// Something changed (a memory, a slideshow) since the running pass looked.
static DIRTY: AtomicBool = AtomicBool::new(false);

/// Fixes the launch time the grace counts from; called when maintenance starts.
pub fn start() {
    LazyLock::force(&LAUNCHED);
}

/// Whether the step should wait: an import is copying, or videos are being prepared.
fn paused(app: &AppHandle) -> bool {
    let importing = !app.state::<AppState>().imports.lock().expect("imports lock").is_empty();
    importing || photos_jobs::activity::snapshot().1.iter().any(|a| a.id == "video-prep")
}

/// Asks for a pass `delay` from now (never before the launch grace is over). Calls in a burst
/// share one pass: whatever has changed by the time it runs is what it sees.
pub fn schedule(app: &AppHandle, delay: Duration) {
    DIRTY.store(true, Ordering::SeqCst);
    if WAITING.swap(true, Ordering::SeqCst) {
        return;
    }
    let app = app.clone();
    let spawned = std::thread::Builder::new().name("lumen-memory-prep".into()).spawn({
        let app = app.clone();
        move || {
            let earliest = *LAUNCHED + GRACE;
            std::thread::sleep(delay.max(earliest.saturating_duration_since(Instant::now())));
            while paused(&app) {
                std::thread::sleep(RECHECK);
            }
            WAITING.store(false, Ordering::SeqCst);
            let a = app.clone();
            app.state::<AppState>().jobs.submit("maintenance:memory-prep", Lane::Maintenance, move |cancel| {
                while DIRTY.swap(false, Ordering::SeqCst) {
                    if !pass(&a, cancel) {
                        // Cut short by an import, a video walk or a close: what is left comes later.
                        if !cancel.is_cancelled() {
                            schedule(&a, RECHECK);
                        }
                        return;
                    }
                }
            });
        }
    });
    if spawned.is_err() {
        WAITING.store(false, Ordering::SeqCst);
    }
}

/// One pass over tracks, films and slideshows. Logs what it prepared and how long it took.
/// `false` when it stopped early.
fn pass(app: &AppHandle, cancel: &CancelToken) -> bool {
    let state = app.state::<AppState>();
    let Ok(lib) = state.library() else { return true };
    if state.ffmpeg().is_none() {
        return true;
    }
    let has_key = state.settings.read().expect("settings lock").music.jamendo_client_id.as_deref().is_some_and(|k| !k.trim().is_empty());
    let saved = crate::music::list_saved(&lib.root);
    let t = Instant::now();

    let unprofiled: Vec<String> =
        saved.iter().map(|s| s.path.clone()).filter(|p| crate::track_profiles::cached(&lib, Path::new(p)).is_none()).collect();
    // A film built with nothing to choose from would be kept silent; wait until there is music.
    let films = if saved.is_empty() && !has_key { Vec::new() } else { lib.db.with_conn(|c| photos_core::memory_films::missing(c, photos_core::memory_films::PREPARE_NEWEST)).unwrap_or_default() };
    let slideshows = unanalysed_slideshow_tracks(&lib);

    let total = unprofiled.len() + films.len() + slideshows.len();
    if total == 0 {
        return true;
    }
    let ring = photos_jobs::activity::start("memory-prep", LABEL, total as u64);
    let interrupted = || cancel.is_cancelled() || paused(app);
    let (mut profiled, mut made, mut analysed) = (0, 0, 0);

    for path in &unprofiled {
        if interrupted() {
            return false;
        }
        if let Some(ff) = state.ffmpeg() {
            profiled += crate::track_profiles::profile_of(app, &lib, &ff, Path::new(path)).is_some() as u32;
        }
        ring.advance(1);
    }
    for (memory_id, _) in &films {
        if interrupted() {
            return false;
        }
        match tauri::async_runtime::block_on(crate::commands::slideshows::film_for(app, *memory_id, false, false)) {
            Ok(_) => made += 1,
            Err(e) => tracing::info!(memory_id, "memory film not prepared: {}", e.message),
        }
        ring.advance(1);
    }
    for path in &slideshows {
        if interrupted() {
            return false;
        }
        if let Some(ff) = state.ffmpeg() {
            analysed += crate::slideshows::analyse_track(app, &lib, &ff, path).is_ok() as u32;
        }
        ring.advance(1);
    }
    tracing::info!(profiled, films = made, slideshow_tracks = analysed, ms = t.elapsed().as_millis() as u64, "memories prepared");
    true
}

/// The library-music tracks of saved slideshows that have no stored beat analysis.
fn unanalysed_slideshow_tracks(lib: &photos_core::Library) -> Vec<std::path::PathBuf> {
    let Ok(list) = lib.db.with_conn(photos_core::slideshow::list) else { return Vec::new() };
    let mut out: Vec<std::path::PathBuf> = Vec::new();
    for s in list {
        let Ok((_, doc)) = lib.db.with_conn(|c| photos_core::slideshow::get(c, s.id)) else { continue };
        let Some(path) = crate::slideshows::audio_path(&lib.root, &doc) else { continue };
        let Some(name) = path.file_name().and_then(|n| n.to_str()).map(str::to_string) else { continue };
        let cached = path.starts_with(crate::music::music_dir(&lib.root))
            && lib.db.with_conn(|c| photos_core::beatgrid::get_analysis(c, &name)).ok().flatten().is_some();
        if !cached && !out.contains(&path) && path.starts_with(crate::music::music_dir(&lib.root)) {
            out.push(path);
        }
    }
    out
}
