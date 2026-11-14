//! Event names and payloads pushed to the webview. The frontend never polls.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::Serialize;
use tauri::{AppHandle, Emitter};
use ts_rs::TS;

pub const IMPORT_PROGRESS: &str = "import://progress";
pub const IMPORT_FINISHED: &str = "import://finished";
pub const JOBS_UPDATE: &str = "jobs://update";
/// Every named background activity (Round 9B, `photos_jobs::activity`).
pub const ACTIVITY_UPDATE: &str = "activity://update";
pub const LIBRARY_CHANGED: &str = "library://changed";
pub const LIBRARY_FOUND: &str = "library://found";
pub const DEVICE_VOLUMES: &str = "device://volumes";
pub const DEVICE_IPHONES: &str = "device://iphones";
pub const DEVICE_MACDISKS: &str = "device://macdisks";
pub const MODELS_PROGRESS: &str = "models://progress";
pub const MODELS_READY: &str = "models://ready";
pub const PROXY_PROGRESS: &str = "proxy://progress";
/// Round 9A's background video prep: `video_prep::Progress`.
pub const VIDEO_PREP_PROGRESS: &str = "video-prep://progress";
/// Round 12-6: staged progress and the end of one interactive AI action (`ai_task`).
pub const AI_PROGRESS: &str = "ai://progress";
pub const AI_FINISHED: &str = "ai://finished";
/// Round 15: what is stuck, sent whenever the set changes (an empty list: all clear).
pub const STALL: &str = "app://stall";

/// Where a stall is (Round 15 watchdog).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[ts(export)]
pub enum StallKind {
    MainThread,
    Database,
    Job,
}

/// One thing past its limit: the job key, the database caller, or the main thread.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Stall {
    pub what: String,
    pub kind: StallKind,
    #[ts(type = "number")]
    pub for_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct StallReport {
    pub stalls: Vec<Stall>,
}

/// What the iPhone poll found. `usbmuxd_available` false means Apple's drivers are
/// missing, which the UI explains instead of showing an empty list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct IphonesUpdate {
    pub usbmuxd_available: bool,
    pub devices: Vec<photos_import::IphoneInfo>,
}

/// A Mac-formatted disk that Windows cannot mount, read through its partition device.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct MacDiskInfo {
    /// `disk3-part2`
    pub id: String,
    pub name: String,
    pub fs: photos_import::macdisk::fs::MacFsKind,
    #[ts(type = "number")]
    pub total_bytes: u64,
    /// HFS+ only; the APFS parser does not know.
    #[ts(type = "number | null")]
    pub free_bytes: Option<u64>,
    /// Absolute paths of the `.photoslibrary` folders on it.
    pub libraries: Vec<String>,
}

/// A mounted volume with a `DCIM` folder (memory card, camera).
#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct VolumeInfo {
    pub mount: String,
    pub label: String,
    /// `import_seen.source_key`
    pub key: String,
    pub total_items: u32,
    pub new_items: u32,
    pub removable: bool,
}
pub const PROXY_FINISHED: &str = "proxy://finished";
pub const EXPORT_PROGRESS: &str = "export://progress";
pub const EXPORT_FINISHED: &str = "export://finished";
pub const SLIDESHOW_PROGRESS: &str = "slideshow://progress";
pub const SLIDESHOW_FINISHED: &str = "slideshow://finished";
/// The window was asked to close while an import runs; the frontend asks what to do.
pub const CLOSE_REQUESTED: &str = "app://close-requested";
/// Round 5: Develop edits are unsaved; the page writes them, then closes the window again if
/// the payload is `true` (Close) and stays if `false` (*Restart Lumen*, Round 15).
pub const FLUSH_REQUESTED: &str = "app://flush-requested";
/// *Restart Lumen* stayed in place (`RestartStep`): the backend answers again, or a new
/// Lumen could not be started (Round 15).
pub const RESTART: &str = "app://restart";
pub const DENOISE_PROGRESS: &str = "denoise://progress";
/// Round 4F: how far a memory's film has got while it is being made.
pub const FILM_PROGRESS: &str = "film://progress";

/// `stage` is `music` (choosing and fetching the track), `beat` (finding it) or `ready`.
#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct FilmProgress {
    #[ts(type = "number")]
    pub memory_id: i64,
    pub stage: String,
}

/// Round 4I: photos the face scan has left, after each batch (0 when it is done).
pub const PEOPLE_PROGRESS: &str = "people://progress";
/// Round 9A: the face radar moved faces after a correction ([`FacesRadar`]).
pub const FACES_RADAR: &str = "faces://radar";

/// The face scan's progress, for the activity ring (*Looking for people · 1,204 left*).
#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PeopleProgress {
    pub left: u32,
}
pub const DENOISE_FINISHED: &str = "denoise://finished";
/// The device browser re-sorted (dates read) or re-marked (an import finished) a session.
pub const DEVICE_BROWSER_CHANGED: &str = "device-browser://changed";

/// A denoise run's tiles done (Phase 8d).
#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DenoiseProgress {
    #[ts(type = "number")]
    pub asset_id: i64,
    pub done: u32,
    pub total: u32,
}

/// A denoise run ended: built (`ok`), failed or cancelled (`error`).
#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DenoiseFinished {
    #[ts(type = "number")]
    pub asset_id: i64,
    pub ok: bool,
    pub model: String,
    pub on_gpu: bool,
    pub error: Option<String>,
}

/// How far one slideshow export has come: the stills first, then FFmpeg's own progress.
#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SlideshowProgress {
    #[ts(type = "number")]
    pub id: i64,
    /// `id` is a memory's: its film is exported without a slideshow (Round 6).
    pub memory: bool,
    /// 0..=1
    pub progress: f32,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SlideshowFinished {
    #[ts(type = "number")]
    pub id: i64,
    /// `id` is a memory's (Round 6).
    pub memory: bool,
    pub ok: bool,
    /// The finished MP4, for *Reveal*.
    pub path: Option<String>,
    pub error: Option<String>,
}

/// A model file arriving (ML design §5).
#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ModelsProgress {
    pub file: String,
    #[ts(type = "number")]
    pub received: u64,
    #[ts(type = "number")]
    pub total: u64,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ExportProgress {
    #[ts(type = "number")]
    pub batch: u64,
    pub done: u32,
    pub failed: u32,
    pub total: u32,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ExportFinished {
    #[ts(type = "number")]
    pub batch: u64,
    pub done: u32,
    pub failed: u32,
    /// Where the files went, for *Reveal*.
    pub dir: String,
    pub metadata: photos_core::export::MetadataOutcome,
    /// Files whose read-back found something missing (spec §5); empty when all is well.
    pub losses: Vec<photos_core::export::ExportLoss>,
    /// Set when the batch went into the library rather than a folder.
    pub library: Option<AddedToLibrary>,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AddedToLibrary {
    /// New photos, each linked to the one it was exported from.
    pub added: u32,
    /// Files whose bytes the library already had.
    pub duplicates: u32,
    /// The album they were also put in, by name (Round 5).
    #[ts(optional)]
    pub album: Option<String>,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ProxyProgress {
    #[ts(type = "number")]
    pub asset_id: i64,
    /// 0..=1
    pub progress: f32,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ProxyFinished {
    #[ts(type = "number")]
    pub asset_id: i64,
    pub ok: bool,
    /// Made by the background video prep, not asked for by the viewer (Round 9A): a failure
    /// is not announced.
    pub background: bool,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct JobsUpdate {
    pub running: u32,
    pub queued: u32,
    /// Per lane, so the activity ring can name the work rather than reporting one total.
    pub interactive: u32,
    pub prefetch: u32,
    pub import: u32,
    pub maintenance: u32,
    /// Worker threads in the pool.
    pub workers: u32,
}

/// One background activity as the ring lists it: a label a person understands and how far
/// it got. `done` and `total` only grow while it runs.
#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ActivityLine {
    pub id: String,
    pub label: String,
    #[ts(type = "number")]
    pub done: u64,
    #[ts(type = "number")]
    pub total: u64,
    pub detail: Option<String>,
}

/// Every running activity (Round 9B), sent when any changes; an empty list is idle.
#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ActivityUpdate {
    pub items: Vec<ActivityLine>,
}

impl From<photos_jobs::activity::ActivityItem> for ActivityLine {
    fn from(i: photos_jobs::activity::ActivityItem) -> Self {
        Self { id: i.id, label: i.label, done: i.done, total: i.total, detail: i.detail }
    }
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct LibraryChanged {
    #[ts(type = "number")]
    pub version: u64,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct LibraryFound {
    pub id: String,
    pub name: String,
    pub path: String,
}

const MIN_INTERVAL: Duration = Duration::from_millis(250);
/// How often work in the background (a thumbnail finishing) is announced. During an import
/// hundreds finish a second; announcing each at `MIN_INTERVAL` refetched every grid four
/// times a second and made scrolling stutter (round 3).
pub const BACKGROUND_INTERVAL: Duration = Duration::from_secs(2);

/// Coalesces `library://changed`: at most four per second for what the user did, every two
/// seconds for background work, and never lost — the jobs ticker flushes anything held back.
pub struct ChangeEmitter {
    pub version: AtomicU64,
    /// A user change held back by `MIN_INTERVAL` (its version is already bumped).
    dirty: AtomicBool,
    /// Background changes not announced yet (the version is bumped when they are, so the
    /// layout cache survives until then).
    background: AtomicBool,
    last: Mutex<Instant>,
    last_background: Mutex<Instant>,
}

impl Default for ChangeEmitter {
    fn default() -> Self {
        let long_ago = Instant::now() - BACKGROUND_INTERVAL;
        Self {
            version: AtomicU64::new(1),
            dirty: AtomicBool::new(false),
            background: AtomicBool::new(false),
            last: Mutex::new(long_ago),
            last_background: Mutex::new(long_ago),
        }
    }
}

impl ChangeEmitter {
    /// Bump the version and emit now if the interval allows, otherwise mark dirty.
    pub fn changed(&self, app: &AppHandle, force: bool) {
        if let Some(version) = self.note_at(force, Instant::now()) {
            let _ = app.emit(LIBRARY_CHANGED, LibraryChanged { version });
        }
    }

    /// Background work changed the library (a thumbnail, a photo's size): announced every
    /// `BACKGROUND_INTERVAL` at most.
    pub fn changed_background(&self, app: &AppHandle) {
        if let Some(version) = self.note_background_at(Instant::now()) {
            let _ = app.emit(LIBRARY_CHANGED, LibraryChanged { version });
        }
    }

    /// Called by the ticker: emits if a change was held back and is due.
    pub fn flush(&self, app: &AppHandle) {
        if let Some(version) = self.take_flush_at(Instant::now()) {
            let _ = app.emit(LIBRARY_CHANGED, LibraryChanged { version });
        }
    }

    /// A user change: the version to announce now, if any. Anything held back goes with it.
    fn note_at(&self, force: bool, now: Instant) -> Option<u64> {
        let version = self.version.fetch_add(1, Ordering::SeqCst) + 1;
        let mut last = self.last.lock().unwrap();
        if force || now.saturating_duration_since(*last) >= MIN_INTERVAL {
            *last = now;
            self.dirty.store(false, Ordering::SeqCst);
            if self.background.swap(false, Ordering::SeqCst) {
                *self.last_background.lock().unwrap() = now;
            }
            Some(version)
        } else {
            self.dirty.store(true, Ordering::SeqCst);
            None
        }
    }

    fn note_background_at(&self, now: Instant) -> Option<u64> {
        self.background.store(true, Ordering::SeqCst);
        self.announce_background_if_due(now)
    }

    fn announce_background_if_due(&self, now: Instant) -> Option<u64> {
        let mut last = self.last_background.lock().unwrap();
        if !self.background.load(Ordering::SeqCst) || now.saturating_duration_since(*last) < BACKGROUND_INTERVAL {
            return None;
        }
        *last = now;
        self.background.store(false, Ordering::SeqCst);
        self.dirty.store(false, Ordering::SeqCst);
        *self.last.lock().unwrap() = now;
        Some(self.version.fetch_add(1, Ordering::SeqCst) + 1)
    }

    fn take_flush_at(&self, now: Instant) -> Option<u64> {
        if let Some(v) = self.announce_background_if_due(now) {
            return Some(v);
        }
        if self.dirty.swap(false, Ordering::SeqCst) {
            *self.last.lock().unwrap() = now;
            return Some(self.version.load(Ordering::SeqCst));
        }
        None
    }

    pub fn current(&self) -> u64 {
        self.version.load(Ordering::SeqCst)
    }
}

#[cfg(test)]
mod change_tests {
    use super::*;

    /// A thumbnail finishing is a background change: during an import hundreds arrive a
    /// second, and announcing each (at the old 250 ms) refetched every grid four times a
    /// second and made scrolling stutter.
    #[test]
    fn background_changes_are_announced_every_two_seconds_at_most() {
        let e = ChangeEmitter::default();
        let t0 = Instant::now();
        assert!(e.note_background_at(t0).is_some(), "the first is announced");
        assert_eq!(e.note_background_at(t0 + Duration::from_millis(300)), None);
        assert_eq!(e.note_background_at(t0 + Duration::from_millis(1500)), None);
        // Held back, not lost: the ticker announces it once it is due.
        assert_eq!(e.take_flush_at(t0 + Duration::from_millis(1600)), None, "not due yet");
        assert!(e.take_flush_at(t0 + BACKGROUND_INTERVAL).is_some());
        assert_eq!(e.take_flush_at(t0 + BACKGROUND_INTERVAL * 2), None, "nothing left");
    }

    #[test]
    fn what_the_user_did_is_announced_at_once() {
        let e = ChangeEmitter::default();
        let t0 = Instant::now();
        e.note_background_at(t0);
        e.note_background_at(t0 + Duration::from_millis(10));
        // A forced change goes now and carries the held-back background ones with it.
        let v = e.note_at(true, t0 + Duration::from_millis(20)).unwrap();
        assert_eq!(v, e.current());
        assert_eq!(e.take_flush_at(t0 + BACKGROUND_INTERVAL * 3), None, "already announced");
    }
}

/// `faces://radar`: *Fixed 7 more photos of Anna · Undo* (Round 9A).
#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct FacesRadar {
    #[ts(type = "number")]
    pub person: i64,
    pub name: Option<String>,
    pub photos: u32,
    pub undo: photos_core::people::radar::RadarUndo,
}
