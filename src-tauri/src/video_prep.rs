//! Every video ready to play before it is opened (Round 9A).
//!
//! The videos this WebView cannot play as they are get their proxy in the background: at
//! import, and once for the ones already in the library. Smart size (`proxy_plan`): the
//! picture is copied for files under 1 GB, a 1080p version is made above. Copies and 1080p
//! versions drain side by side (Round 15: a quick copy never waits behind an hour of encoding),
//! each newest first, from its own thread; each proxy is an ordinary `proxy:<hash>` job on the
//! Maintenance lane, so the viewer asking for the same video joins it instead of making a
//! second. Between videos it steps aside while the user is busy, and it stops before the disk
//! holds less than 10 GB. The proxies on disk are its progress: a restart walks again and
//! finds only what is left.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use photos_core::assets::{self, VideoPrepRow};
use photos_media::{DerivativeStore, Playback, Probe, ProxyPlan, VideoSupport};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager};
use ts_rs::TS;

use crate::state::AppState;

/// The walk never leaves the library's disk with less than this free.
pub const MIN_FREE: u64 = 10_000_000_000;

/// A 1080p proxy's most bytes per second, for the space check and the estimate: the video's
/// ceiling (`PROXY_MAX_BITRATE`, 8 Mbit/s) and 160 kbit/s of AAC. x264 often writes far less.
pub const TRANSCODE_BYTES_PER_SECOND: u64 = (photos_media::PROXY_MAX_BITRATE as u64 + 160_000) / 8;

/// How long after the user's last render the walk stays paused (the pet scan's).
const STEP_ASIDE: Duration = Duration::from_millis(600);

/// Rows read per query: short holds on the connection (the DB speed rule).
const PAGE: i64 = 200;

/// `Settings → Playback & codecs → Prepare videos in the background`; on unless turned off.
pub fn enabled(p: &photos_core::settings::Playback) -> bool {
    p.prepare_videos.unwrap_or(true)
}

/// A video that needs a proxy and has none.
#[derive(Debug, Clone, PartialEq)]
pub struct Candidate {
    pub id: i64,
    pub content_hash: String,
    /// Relative to the library's `originals/`.
    pub original: PathBuf,
    pub plan: ProxyPlan,
    /// What its proxy will take on disk, roughly.
    pub estimate: u64,
}

/// Why a walk ended before its last video.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, rename = "VideoPrepStopped")]
pub enum Stopped {
    /// Less than 10 GB would be left: *Not enough space to prepare videos*.
    NoSpace,
    /// Turned off in Settings, or the library closed.
    Off,
}

/// `video-prep://progress`: *Preparing videos · 41 of 322*.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, rename = "VideoPrepProgress")]
pub struct Progress {
    pub done: u32,
    pub total: u32,
    pub stopped: Option<Stopped>,
}

/// How a proxy came out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Made {
    Done,
    /// FFmpeg could not make it: remembered, so the next launch does not try again.
    Failed,
    /// Nobody finished it (FFmpeg gone, the app closing): tried again next time.
    Cancelled,
}

/// What the walk needs from the app; a fake in tests.
pub trait Host {
    fn enabled(&self) -> bool;
    fn user_busy(&self) -> bool;
    fn pause(&self);
    fn free_space(&self) -> Option<u64>;
    fn proxy_exists(&self, c: &Candidate) -> bool;
    /// Makes `c`'s proxy and returns once it is made or has failed.
    fn make(&self, c: &Candidate) -> Made;
    fn mark_failed(&self, c: &Candidate);
    fn report(&self, p: &Progress);
}

#[derive(Deserialize)]
struct Stored {
    #[serde(default)]
    probe: Option<Probe>,
}

/// The videos of `rows` that `support` cannot play as they are, whose proxy is neither made
/// (`made`) nor known to fail (`failed`), newest first (undated last). A video without a probe
/// is left to the viewer: there is nothing to decide by.
pub fn candidates(
    rows: Vec<VideoPrepRow>,
    os: &str,
    support: Option<VideoSupport>,
    made: impl Fn(&str) -> bool,
    failed: impl Fn(&str) -> bool,
) -> Vec<Candidate> {
    let mut picked: Vec<(Option<i64>, Candidate)> = rows
        .into_iter()
        .filter_map(|r| {
            let probe = serde_json::from_str::<Stored>(r.metadata_json.as_deref()?).ok()?.probe?;
            if photos_media::playback_with(&probe, os, support) != Playback::Proxy || made(&r.content_hash) || failed(&r.content_hash) {
                return None;
            }
            let size = r.byte_size.max(0) as u64;
            let plan = photos_media::proxy_plan(&probe, Some(size), os, support);
            let estimate = estimate_bytes(plan, size, r.duration_ms.or(probe.duration_ms));
            Some((r.captured_at, Candidate { id: r.id, content_hash: r.content_hash, original: PathBuf::from(r.relative_path), plan, estimate }))
        })
        .collect();
    // `Option`'s order puts `None` first; reversed, the undated come last.
    picked.sort_by(|(a, x), (b, y)| b.cmp(a).then(y.id.cmp(&x.id)));
    picked.into_iter().map(|(_, c)| c).collect()
}

/// A proxy's rough size: a copy is the original's; a 1080p version runs at
/// [`TRANSCODE_BYTES_PER_SECOND`], never more than the original.
pub fn estimate_bytes(plan: ProxyPlan, size: u64, duration_ms: Option<i64>) -> u64 {
    match (plan, duration_ms) {
        (ProxyPlan::Transcode, Some(ms)) if ms > 0 => (ms as u64 / 1000 * TRANSCODE_BYTES_PER_SECOND).min(size),
        _ => size,
    }
}

/// Whether making a proxy of `estimate` bytes leaves [`MIN_FREE`]. Unknown free space carries
/// on, as the import does.
pub fn room_for(free: Option<u64>, estimate: u64) -> bool {
    free.is_none_or(|f| f.saturating_sub(estimate) >= MIN_FREE)
}

/// Makes the proxies of `list`, reporting as it goes: the copies (disk-bound, seconds each) and
/// the 1080p versions (the encoder) in two drains at once, each in list order, so a copy never
/// waits behind a transcode. Stopping (turned off, disk full) stops both.
pub fn walk(host: &(impl Host + Sync), list: Vec<Candidate>) -> Progress {
    let p = std::sync::Mutex::new(Progress { done: 0, total: list.len() as u32, stopped: None });
    host.report(&p.lock().unwrap_or_else(|e| e.into_inner()));
    let (transcodes, remuxes): (Vec<_>, Vec<_>) = list.into_iter().partition(|c| c.plan == ProxyPlan::Transcode);
    std::thread::scope(|s| {
        if !transcodes.is_empty() {
            s.spawn(|| drain(host, &transcodes, &p));
        }
        drain(host, &remuxes, &p);
    });
    let end = p.into_inner().unwrap_or_else(|e| e.into_inner());
    host.report(&end);
    end
}

/// One drain of [`walk`]: `list` in order until it ends or either drain stops the walk.
fn drain(host: &impl Host, list: &[Candidate], p: &std::sync::Mutex<Progress>) {
    let progress = || p.lock().unwrap_or_else(|e| e.into_inner());
    let stopped = || progress().stopped.is_some();
    let stop = |why: Stopped| {
        progress().stopped.get_or_insert(why);
    };
    for c in list {
        while host.enabled() && host.user_busy() && !stopped() {
            host.pause();
        }
        if stopped() {
            return;
        }
        if !host.enabled() {
            return stop(Stopped::Off);
        }
        if !host.proxy_exists(c) {
            if !room_for(host.free_space(), c.estimate) {
                return stop(Stopped::NoSpace);
            }
            match host.make(c) {
                Made::Done | Made::Cancelled => {}
                Made::Failed => host.mark_failed(c),
            }
        }
        let mut g = progress();
        g.done += 1;
        host.report(&g);
    }
}

/// The walk in the app.
struct AppHost {
    app: AppHandle,
    root: PathBuf,
    store: DerivativeStore,
}

impl Host for AppHost {
    fn enabled(&self) -> bool {
        let state = self.app.state::<AppState>();
        let on = enabled(&state.settings.read().expect("settings lock").playback);
        on && state.library().is_ok_and(|l| l.root == self.root)
    }
    fn user_busy(&self) -> bool {
        busy(photos_jobs::user_busy(STEP_ASIDE))
    }
    fn pause(&self) {
        std::thread::sleep(Duration::from_millis(200));
    }
    fn free_space(&self) -> Option<u64> {
        crate::space::free_space(&self.root)
    }
    fn proxy_exists(&self, c: &Candidate) -> bool {
        self.store.proxy_path(&c.content_hash).is_file()
    }
    fn make(&self, c: &Candidate) -> Made {
        let _transcoding = (c.plan == ProxyPlan::Transcode).then(Transcoding::begin);
        match crate::derivatives::proxy_job(&self.app, c.id, photos_jobs::Lane::Maintenance, true) {
            Ok(None) => Made::Done,
            Ok(Some(job)) => {
                job.wait();
                if self.proxy_exists(c) {
                    Made::Done
                } else if job.cancel.is_cancelled() {
                    Made::Cancelled
                } else {
                    Made::Failed
                }
            }
            Err(e) => {
                tracing::debug!(asset = c.id, "video prep: {}", e.message);
                Made::Cancelled
            }
        }
    }
    fn mark_failed(&self, c: &Candidate) {
        self.store.mark_proxy_failed(&c.content_hash);
    }
    fn report(&self, p: &Progress) {
        *STATUS.lock().unwrap_or_else(|e| e.into_inner()) = Some(p.clone());
        ring(p);
        let _ = self.app.emit(crate::events::VIDEO_PREP_PROGRESS, p);
    }
}

/// The activity ring's *Preparing videos* line, while a walk has videos left (Round 9B's ring,
/// wired at the merge in Round 10): (handle, done so far).
static RING: std::sync::Mutex<Option<(photos_jobs::activity::Activity, u32)>> = std::sync::Mutex::new(None);

fn ring(p: &Progress) {
    let mut ring = RING.lock().unwrap_or_else(|e| e.into_inner());
    if p.stopped.is_some() || p.done >= p.total {
        *ring = None;
        return;
    }
    let (activity, done) = ring.get_or_insert_with(|| (photos_jobs::activity::start("video-prep", "Preparing videos", p.total as u64), 0));
    activity.advance(p.done.saturating_sub(*done) as u64);
    *done = p.done;
}

/// Set while the model bench times (Round 15): the walk starts no video meanwhile.
static BENCHING: AtomicBool = AtomicBool::new(false);
/// Set while the walk makes a 1080p version, so the bench can wait it out.
static TRANSCODING: AtomicBool = AtomicBool::new(false);

/// Whether the walk should step aside: the user is busy, or models are being timed.
fn busy(user: bool) -> bool {
    user || BENCHING.load(Ordering::SeqCst)
}

/// Whether a background transcode is running now.
pub fn transcoding() -> bool {
    TRANSCODING.load(Ordering::SeqCst)
}

/// Pauses the walk between videos until dropped (also on a panic).
pub struct BenchPause(());

impl BenchPause {
    pub fn begin() -> Self {
        BENCHING.store(true, Ordering::SeqCst);
        Self(())
    }
}

impl Drop for BenchPause {
    fn drop(&mut self) {
        BENCHING.store(false, Ordering::SeqCst);
    }
}

/// `TRANSCODING` for as long as it lives.
struct Transcoding(());

impl Transcoding {
    fn begin() -> Self {
        TRANSCODING.store(true, Ordering::SeqCst);
        Self(())
    }
}

impl Drop for Transcoding {
    fn drop(&mut self) {
        TRANSCODING.store(false, Ordering::SeqCst);
    }
}

static RUNNING: AtomicBool = AtomicBool::new(false);
static AGAIN: AtomicBool = AtomicBool::new(false);
/// The last walk's progress, for a screen opened after it started.
static STATUS: std::sync::Mutex<Option<Progress>> = std::sync::Mutex::new(None);
/// The disk was found full: said once, until a walk finds room again.
static SAID_FULL: AtomicBool = AtomicBool::new(false);

/// What the video prep is doing, if it has started.
pub fn status() -> Option<Progress> {
    STATUS.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

/// Walk the library's videos for proxies to make: after an import, when the WebView says what
/// it plays, when the setting is turned on, and on the maintenance tick. Coalesced: a call
/// while a walk runs makes it walk once more when it ends.
pub fn schedule(app: &AppHandle) {
    AGAIN.store(true, Ordering::SeqCst);
    if RUNNING.swap(true, Ordering::SeqCst) {
        return;
    }
    let app = app.clone();
    let spawned = std::thread::Builder::new().name("lumen-video-prep".into()).spawn(move || {
        photos_jobs::background_priority();
        loop {
            while AGAIN.swap(false, Ordering::SeqCst) {
                run_once(&app);
            }
            RUNNING.store(false, Ordering::SeqCst);
            // A call between the last swap and the store above would otherwise be lost.
            if !AGAIN.load(Ordering::SeqCst) || RUNNING.swap(true, Ordering::SeqCst) {
                break;
            }
        }
    });
    if spawned.is_err() {
        RUNNING.store(false, Ordering::SeqCst);
    }
}

fn run_once(app: &AppHandle) {
    let state = app.state::<AppState>();
    let Ok(lib) = state.library() else { return };
    if state.ffmpeg().is_none() || !enabled(&state.settings.read().expect("settings lock").playback) {
        return;
    }
    // What plays as it is is known once the WebView has said; until then `set_video_support`
    // calls this again.
    let Some(support) = *state.video_support.read().expect("video support lock") else { return };
    let mut rows = Vec::new();
    let mut before = None;
    loop {
        let Ok(page) = lib.db.with_conn(|c| assets::video_prep_rows(c, before, PAGE)) else { return };
        before = page.last().map(|r| r.id);
        let last = (page.len() as i64) < PAGE;
        rows.extend(page);
        if last {
            break;
        }
    }
    let store = DerivativeStore::new(&lib.root);
    let list = candidates(rows, std::env::consts::OS, Some(support), |h| store.proxy_path(h).is_file(), |h| store.proxy_failed(h));
    if list.is_empty() {
        return;
    }
    let (remux, transcode) = list.iter().fold((0, 0), |(r, t), c| if c.plan == ProxyPlan::Transcode { (r, t + 1) } else { (r + 1, t) });
    tracing::info!(videos = list.len(), remux, transcode, bytes = list.iter().map(|c| c.estimate).sum::<u64>(), "preparing videos");
    let host = AppHost { app: app.clone(), root: lib.root.clone(), store };
    let end = walk(&host, list);
    match end.stopped {
        Some(Stopped::NoSpace) => {
            if !SAID_FULL.swap(true, Ordering::SeqCst) {
                tracing::warn!(done = end.done, total = end.total, "not enough space to prepare videos (10 GB stays free)");
            }
        }
        _ => SAID_FULL.store(false, Ordering::SeqCst),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use photos_core::assets::VideoPrepRow;
    use photos_media::{ProxyPlan, VideoSupport};
    use std::sync::{Condvar, Mutex};
    use std::collections::HashSet;

    fn row(id: i64, captured_at: Option<i64>, size: i64, probe: &str) -> VideoPrepRow {
        VideoPrepRow {
            id,
            content_hash: format!("hash{id:04}"),
            relative_path: format!("2026/10/06/{id}.MOV"),
            metadata_json: (!probe.is_empty()).then(|| format!("{{\"width\":null,\"height\":null,\"orientation\":1,\"captured_at\":null,\"probe\":{probe}}}")),
            byte_size: size,
            captured_at,
            duration_ms: Some(10_000),
        }
    }

    const LIVE: &str = r#"{"video_codec":"hevc","pix_fmt":"yuvj420p","audio_codec":"pcm_s16le","container":"mov,mp4,m4a,3gp,3g2,mj2","rotation":0}"#;
    const PLAYS: &str = r#"{"video_codec":"h264","pix_fmt":"yuv420p","audio_codec":"aac","container":"mov,mp4,m4a,3gp,3g2,mj2","rotation":0}"#;
    const ALL: Option<VideoSupport> = Some(VideoSupport { hevc: true, hevc_main10: true });

    #[test]
    fn only_videos_that_cannot_play_and_have_no_proxy_newest_first() {
        let rows = vec![
            row(1, Some(100), 5_000, LIVE),
            row(2, Some(300), 5_000, PLAYS),
            row(3, None, 2_000_000_000, LIVE),
            row(4, Some(200), 5_000, LIVE),
            row(5, Some(400), 5_000, LIVE),
            row(6, Some(500), 5_000, LIVE),
            row(7, Some(600), 5_000, ""),
        ];
        let made: HashSet<String> = ["hash0005".to_string()].into();
        let failed: HashSet<String> = ["hash0006".to_string()].into();
        let list = candidates(rows, "windows", ALL, |h| made.contains(h), |h| failed.contains(h));
        assert_eq!(list.iter().map(|c| c.id).collect::<Vec<_>>(), vec![4, 1, 3], "newest first; undated last");
        assert_eq!(list[0].plan, ProxyPlan::Remux { copy_audio: false });
        assert_eq!(list[2].plan, ProxyPlan::Transcode, "2 GB: a 1080p version, not a copy");
        assert_eq!(list[0].original, std::path::PathBuf::from("2026/10/06/4.MOV"));
    }

    #[test]
    fn a_transcode_is_estimated_smaller_than_its_original() {
        assert_eq!(estimate_bytes(ProxyPlan::Remux { copy_audio: false }, 5_000, Some(10_000)), 5_000);
        // A 101 GB, 40-minute MOV: at most the ceiling's bytes.
        let huge = estimate_bytes(ProxyPlan::Transcode, 101_000_000_000, Some(40 * 60 * 1000));
        assert_eq!(huge, 40 * 60 * TRANSCODE_BYTES_PER_SECOND);
        assert_eq!(estimate_bytes(ProxyPlan::Transcode, 1_000, Some(60_000)), 1_000, "never more than the original");
        assert_eq!(estimate_bytes(ProxyPlan::Transcode, 8_000, None), 8_000, "unknown length: the original's size");
    }

    #[test]
    fn room_keeps_ten_gigabytes_free() {
        assert!(room_for(Some(MIN_FREE + 5_000), 5_000));
        assert!(!room_for(Some(MIN_FREE + 4_999), 5_000));
        assert!(!room_for(Some(3), 0));
        assert!(room_for(None, 5_000), "unknown: carry on (the import's rule)");
    }

    /// A host that records what the walk did. `Sync`: the walk runs two drains at once.
    #[derive(Default)]
    struct Fake {
        log: Mutex<Vec<String>>,
        busy_for: Mutex<u32>,
        free: Mutex<Option<u64>>,
        /// Turned off once this many proxies are made.
        off_after: Option<usize>,
        fails: HashSet<i64>,
        made_elsewhere: HashSet<i64>,
        reports: Mutex<Vec<Progress>>,
        /// A transcode waits for this gate (at most 2 s) before it finishes.
        gate: Option<(Mutex<bool>, Condvar)>,
        /// Opens the gate once this many remuxes are made.
        open_after_remuxes: usize,
        transcode_ms: u64,
    }

    impl Fake {
        fn log(&self) -> Vec<String> {
            self.log.lock().unwrap().clone()
        }
        fn made(&self) -> usize {
            self.log.lock().unwrap().iter().filter(|l| l.starts_with("make")).count()
        }
    }

    impl Host for Fake {
        fn enabled(&self) -> bool {
            self.off_after.is_none_or(|n| self.made() < n)
        }
        fn user_busy(&self) -> bool {
            let mut b = self.busy_for.lock().unwrap();
            if *b > 0 {
                *b -= 1;
                self.log.lock().unwrap().push("wait".into());
                true
            } else {
                false
            }
        }
        fn pause(&self) {}
        fn free_space(&self) -> Option<u64> {
            *self.free.lock().unwrap()
        }
        fn proxy_exists(&self, c: &Candidate) -> bool {
            self.made_elsewhere.contains(&c.id)
        }
        fn make(&self, c: &Candidate) -> Made {
            if !self.enabled() {
                self.log.lock().unwrap().push(format!("started {} while off", c.id));
            }
            if c.plan == ProxyPlan::Transcode {
                self.log.lock().unwrap().push(format!("start {}", c.id));
                std::thread::sleep(Duration::from_millis(self.transcode_ms));
                if let Some((open, cv)) = &self.gate {
                    let (open, timeout) = cv.wait_timeout_while(open.lock().unwrap(), Duration::from_secs(2), |o| !*o).unwrap();
                    if timeout.timed_out() && !*open {
                        self.log.lock().unwrap().push("gate timed out".into());
                    }
                }
            }
            {
                let mut log = self.log.lock().unwrap();
                log.push(format!("make {}", c.id));
                let remuxes = log.iter().filter(|l| l.starts_with("make") && !l.ends_with(" t")).count();
                if let Some((open, cv)) = &self.gate {
                    if c.plan != ProxyPlan::Transcode && remuxes >= self.open_after_remuxes {
                        *open.lock().unwrap() = true;
                        cv.notify_all();
                    }
                }
            }
            let mut free = self.free.lock().unwrap();
            *free = free.map(|f| f - c.estimate);
            if self.fails.contains(&c.id) {
                Made::Failed
            } else {
                Made::Done
            }
        }
        fn mark_failed(&self, c: &Candidate) {
            self.log.lock().unwrap().push(format!("failed {}", c.id));
        }
        fn report(&self, p: &Progress) {
            self.reports.lock().unwrap().push(p.clone());
        }
    }

    fn list(n: i64, size: u64) -> Vec<Candidate> {
        (1..=n)
            .map(|id| Candidate {
                id,
                content_hash: format!("h{id}"),
                original: format!("{id}.MOV").into(),
                plan: ProxyPlan::Remux { copy_audio: false },
                estimate: size,
            })
            .collect()
    }

    fn transcode(id: i64) -> Candidate {
        Candidate { plan: ProxyPlan::Transcode, ..list(id, 10).pop().unwrap() }
    }

    #[test]
    fn remuxes_never_wait_behind_a_transcode() {
        let fake = Fake { gate: Some((Mutex::new(false), Condvar::new())), open_after_remuxes: 3, ..Default::default() };
        let mut l = vec![transcode(1)];
        l.extend(list(4, 10).into_iter().skip(1));
        let end = walk(&fake, l);
        let log = fake.log();
        assert!(!log.contains(&"gate timed out".to_string()), "the remuxes waited behind the transcode: {log:?}");
        let at = |s: &str| log.iter().position(|l| l == s).unwrap_or_else(|| panic!("{s} in {log:?}"));
        assert!(at("make 4") < at("make 1"), "{log:?}");
        assert_eq!(end, Progress { done: 4, total: 4, stopped: None });
        let reports = fake.reports.lock().unwrap();
        assert!(reports.windows(2).all(|w| w[0].done <= w[1].done), "never goes back");
    }

    #[test]
    fn turning_prep_off_stops_both_drains() {
        let fake = Fake { off_after: Some(1), transcode_ms: 50, ..Default::default() };
        let mut l = vec![transcode(1)];
        l.extend(list(4, 10).into_iter().skip(1));
        let end = walk(&fake, l);
        let log = fake.log();
        assert_eq!(end.stopped, Some(Stopped::Off), "{log:?}");
        assert!(!log.iter().any(|l| l.contains("while off")), "a make started after it was turned off: {log:?}");
        assert!(fake.made() <= 2, "at most the one under way in each drain: {log:?}");
        assert!(end.done < end.total);
    }

    #[test]
    fn the_walk_steps_aside_counts_and_remembers_failures() {
        let fake = Fake { busy_for: Mutex::new(2), fails: [2].into(), made_elsewhere: [3].into(), ..Default::default() };
        let end = walk(&fake, list(3, 10));
        assert_eq!(fake.log(), vec!["wait", "wait", "make 1", "make 2", "failed 2"], "3 was made by the viewer meanwhile");
        assert_eq!(end, Progress { done: 3, total: 3, stopped: None });
        let reports = fake.reports.lock().unwrap();
        assert_eq!(reports.first(), Some(&Progress { done: 0, total: 3, stopped: None }));
        assert!(reports.windows(2).all(|w| w[0].done <= w[1].done), "never goes back");
    }

    #[test]
    fn the_walk_stops_when_the_disk_fills_or_it_is_turned_off() {
        let fake = Fake { free: Mutex::new(Some(MIN_FREE + 15)), ..Default::default() };
        let end = walk(&fake, list(3, 10));
        assert_eq!(fake.log(), vec!["make 1"], "the second would leave less than 10 GB");
        assert_eq!(end, Progress { done: 1, total: 3, stopped: Some(Stopped::NoSpace) });

        let fake = Fake { off_after: Some(2), ..Default::default() };
        let end = walk(&fake, list(3, 10));
        assert_eq!(fake.log(), vec!["make 1", "make 2"]);
        assert_eq!(end.stopped, Some(Stopped::Off));
    }

    /// The plan's replay: what the walk would make on a real library, by plan, and roughly
    /// how many bytes. Reads a COPY of `library.db`; with `LUMEN_LIBRARY` it also looks (only
    /// looks) for proxies already made. Writes nothing.
    ///
    /// LUMEN_DB=<copy> [LUMEN_LIBRARY=<library>] cargo test -p lumen --lib video_prep_on_a_copy -- --ignored --nocapture
    #[test]
    #[ignore = "needs a copy of a real library.db"]
    fn video_prep_on_a_copy() {
        let db = photos_core::db::Db::open(std::path::Path::new(&std::env::var("LUMEN_DB").unwrap())).unwrap();
        let mut rows = Vec::new();
        let mut before = None;
        loop {
            let page = db.with_conn(|c| photos_core::assets::video_prep_rows(c, before, PAGE)).unwrap();
            before = page.last().map(|r| r.id);
            let last = (page.len() as i64) < PAGE;
            rows.extend(page);
            if last {
                break;
            }
        }
        let store = std::env::var("LUMEN_LIBRARY").ok().map(|l| DerivativeStore::new(std::path::Path::new(&l)));
        let made = |h: &str| store.as_ref().is_some_and(|s| s.proxy_path(h).is_file());
        println!("{} videos", rows.len());
        let gb = |b: u64| b as f64 / 1e9;
        for (name, support) in [("HEVC + Main 10", ALL), ("HEVC 8-bit", Some(VideoSupport { hevc: true, hevc_main10: false })), ("no HEVC", Some(VideoSupport::default()))] {
            let original: std::collections::HashMap<i64, u64> = rows.iter().map(|r| (r.id, r.byte_size as u64)).collect();
            let list = candidates(rows.clone(), "windows", support, made, |_| false);
            let (mut remux, mut transcode, mut rb, mut tb, mut raw) = (0, 0, 0u64, 0u64, 0u64);
            for c in &list {
                raw += original[&c.id];
                match c.plan {
                    ProxyPlan::Transcode => (transcode, tb) = (transcode + 1, tb + c.estimate),
                    ProxyPlan::Remux { .. } => (remux, rb) = (remux + 1, rb + c.estimate),
                }
            }
            println!(
                "{name}: {} need a proxy ({:.1} GB of originals) — {remux} copied ≈ {:.1} GB, {transcode} at 1080p ≈ {:.1} GB; total ≈ {:.1} GB",
                list.len(),
                gb(raw),
                gb(rb),
                gb(tb),
                gb(rb + tb)
            );
        }
    }

    #[test]
    fn the_walk_pauses_while_models_are_timed() {
        assert!(!busy(false));
        let pause = BenchPause::begin();
        assert!(busy(false), "the model bench is timing: no video starts");
        drop(pause);
        assert!(!busy(false));
        assert!(busy(true));
    }

    #[test]
    fn the_setting_is_on_unless_turned_off() {
        let mut p = photos_core::settings::Settings::default().playback;
        assert!(enabled(&p));
        p.prepare_videos = Some(false);
        assert!(!enabled(&p));
    }
}
