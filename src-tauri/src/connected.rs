//! The connected-device page: what a memory card, drive or iPhone holds before it is
//! imported — its name, storage, how much is new, and a look at its newest photos.
//!
//! Previews are rendered on demand, one per request to `photos://device-thumb/{token}`:
//! the page asks for a dozen, and reading them off a phone is not free, so nothing is
//! rendered that is not on screen. Tokens map to (device, item); rendered bytes are kept up
//! to 64 MB, and the device browser (Round 4E) scrolls through thousands of them, so renders
//! queue newest first and those left behind by scrolling are dropped.

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};

use photos_core::seen::{self, SeenItem};
use photos_import::{preview, ImportSource, IphoneSource, SourceItem, VolumeSource};
use serde::Serialize;
use tauri::{AppHandle, Manager};
use ts_rs::TS;

use crate::error::{AppError, AppResult};
use crate::state::AppState;
use crate::volumes;

const NEWEST: usize = 12;
/// Rendered previews kept, by their JPEG bytes: a device browser scrolled back and forth
/// should not read the same files off a phone twice (~15 KB a tile, so a few thousand).
const KEEP_RENDERED_BYTES: usize = 64 * 1024 * 1024;
/// Renders at once: a card or a phone serves one file at a time well, a few at most.
const RENDER_SLOTS: usize = 3;
/// Requests waiting beyond this many newer ones are dropped (see `RenderGate`).
const MAX_WAITING: usize = 48;
/// A tile (`device-thumb/{token}`).
pub const PREVIEW_EDGE: u32 = 320;
/// A closer look (`device-thumb/{token}?s=1024`).
pub const LARGE_EDGE: u32 = 1024;

#[derive(Clone)]
pub enum DeviceRef {
    Volume { mount: PathBuf, label: String, total_space: u64 },
    Iphone { udid: String, name: String },
    /// A source opened once and kept (an Apple Photos library, whose database copy is slow).
    Shared(Arc<dyn ImportSource>),
}

impl DeviceRef {
    fn source(&self) -> Option<Box<dyn ImportSource>> {
        match self {
            DeviceRef::Volume { mount, label, total_space } => {
                VolumeSource::detect(mount, label, *total_space).map(|s| Box::new(s) as Box<dyn ImportSource>)
            }
            DeviceRef::Iphone { udid, name } => Some(Box::new(IphoneSource::new(udid, name))),
            DeviceRef::Shared(source) => Some(Box::new(source.clone())),
        }
    }
}

/// What `PreviewCache::render` came back with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Thumb {
    Jpeg(Arc<Vec<u8>>),
    /// Unknown token, or the file could not be rendered.
    Missing,
    /// The device has gone: the webview gets `410 Gone` and the page shows the gone state.
    Gone,
    /// Pushed out of the queue by newer requests (the tile scrolled away): `503`, not cached,
    /// so the tile asks again if it comes back.
    Dropped,
}

/// (token, long edge)
type ThumbKey = (u64, u32);

/// JPEG bytes by (token, long edge), least recently used first, capped by their total size.
pub struct ByteLru {
    cap: usize,
    used: usize,
    entries: VecDeque<(ThumbKey, Arc<Vec<u8>>)>,
}

impl ByteLru {
    pub fn new(cap: usize) -> Self {
        Self { cap, used: 0, entries: VecDeque::new() }
    }

    pub fn get(&mut self, key: (u64, u32)) -> Option<Arc<Vec<u8>>> {
        let at = self.entries.iter().position(|(k, _)| *k == key)?;
        let e = self.entries.remove(at)?;
        let bytes = e.1.clone();
        self.entries.push_back(e);
        Some(bytes)
    }

    pub fn put(&mut self, key: (u64, u32), bytes: Arc<Vec<u8>>) {
        if let Some(at) = self.entries.iter().position(|(k, _)| *k == key) {
            if let Some((_, old)) = self.entries.remove(at) {
                self.used -= old.len();
            }
        }
        self.used += bytes.len();
        self.entries.push_back((key, bytes));
        while self.used > self.cap {
            match self.entries.pop_front() {
                Some((_, old)) => self.used -= old.len(),
                None => break,
            }
        }
    }

    pub fn retain(&mut self, keep: impl Fn(u64) -> bool) {
        self.entries.retain(|((t, _), _)| keep(*t));
        self.used = self.entries.iter().map(|(_, b)| b.len()).sum();
    }

    #[cfg(test)]
    pub fn used(&self) -> usize {
        self.used
    }
}

/// Lets `slots` renders run at once, newest request first. Scrolling a grid asks for every
/// tile it passes; those still waiting when more than `max_waiting` newer ones have arrived
/// have scrolled away, and are dropped rather than read off the device. (The webview gives the
/// protocol no word when it abandons an image, so recency stands in for "still on screen".)
pub struct RenderGate {
    slots: usize,
    max_waiting: usize,
    state: Mutex<GateState>,
    wake: Condvar,
}

#[derive(Default)]
struct GateState {
    running: usize,
    next: u64,
    waiting: Vec<u64>,
    dropped: HashSet<u64>,
}

pub struct Permit<'a>(&'a RenderGate);

impl Drop for Permit<'_> {
    fn drop(&mut self) {
        self.0.state.lock().expect("gate").running -= 1;
        self.0.wake.notify_all();
    }
}

impl RenderGate {
    pub fn new(slots: usize, max_waiting: usize) -> Self {
        Self { slots, max_waiting, state: Mutex::new(GateState::default()), wake: Condvar::new() }
    }

    /// Waits for a slot; `None` when this request was dropped for newer ones.
    pub fn enter(&self) -> Option<Permit<'_>> {
        let mut s = self.state.lock().expect("gate");
        let ticket = s.next;
        s.next += 1;
        s.waiting.push(ticket);
        if s.waiting.len() > self.max_waiting {
            let oldest = s.waiting.remove(0);
            s.dropped.insert(oldest);
            self.wake.notify_all();
        }
        loop {
            if s.dropped.remove(&ticket) {
                return None;
            }
            if s.running < self.slots && s.waiting.last() == Some(&ticket) {
                s.waiting.pop();
                s.running += 1;
                return Some(Permit(self));
            }
            s = self.wake.wait(s).expect("gate");
        }
    }
}

pub struct PreviewCache {
    next: AtomicU64,
    items: Mutex<HashMap<u64, (DeviceRef, SourceItem)>>,
    rendered: Mutex<ByteLru>,
    gate: RenderGate,
}

impl Default for PreviewCache {
    fn default() -> Self {
        Self {
            next: AtomicU64::new(0),
            items: Mutex::new(HashMap::new()),
            rendered: Mutex::new(ByteLru::new(KEEP_RENDERED_BYTES)),
            gate: RenderGate::new(RENDER_SLOTS, MAX_WAITING),
        }
    }
}

impl PreviewCache {
    pub fn register(&self, device: &DeviceRef, item: &SourceItem) -> u64 {
        let token = self.next.fetch_add(1, Ordering::Relaxed) + 1;
        self.items.lock().expect("preview items").insert(token, (device.clone(), item.clone()));
        token
    }

    /// Drops `tokens` (a closed device browser's) and anything rendered for them.
    pub fn forget(&self, tokens: &[u64]) {
        let gone: HashSet<u64> = tokens.iter().copied().collect();
        self.items.lock().expect("preview items").retain(|t, _| !gone.contains(t));
        self.rendered.lock().expect("preview bytes").retain(|t| !gone.contains(&t));
    }

    /// The JPEG for `token` at most `edge` pixels long (`PREVIEW_EDGE` or `LARGE_EDGE`),
    /// rendering it on first request: from the camera's EXIF thumbnail when that is big
    /// enough, otherwise from the whole file.
    pub fn render(&self, token: u64, edge: u32, ffmpeg: Option<&photos_media::Ffmpeg>) -> Thumb {
        let key = (token, edge);
        if let Some(b) = self.rendered.lock().expect("preview bytes").get(key) {
            return Thumb::Jpeg(b);
        }
        let Some((device, item)) = self.items.lock().expect("preview items").get(&token).cloned() else { return Thumb::Missing };
        let Some(_permit) = self.gate.enter() else { return Thumb::Dropped };
        // The same tile may have been asked for twice; the first answer serves both.
        if let Some(b) = self.rendered.lock().expect("preview bytes").get(key) {
            return Thumb::Jpeg(b);
        }
        let Some(source) = device.source() else { return Thumb::Gone };
        let rendered = match preview::quick_preview(source.as_ref(), &item, edge) {
            Ok(Some(b)) => Ok(b),
            Ok(None) | Err(_) => preview::preview_jpeg(source.as_ref(), &item, ffmpeg, edge),
        };
        match rendered {
            Ok(b) => {
                let bytes = Arc::new(b);
                self.rendered.lock().expect("preview bytes").put(key, bytes.clone());
                Thumb::Jpeg(bytes)
            }
            Err(e) if !source.is_connected() => {
                tracing::info!(file = %item.rel_path, "device gone while previewing: {e}");
                Thumb::Gone
            }
            Err(e) => {
                tracing::warn!(file = %item.rel_path, "device preview failed: {e}");
                Thumb::Missing
            }
        }
    }
}

#[cfg(test)]
mod cache_tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    #[test]
    fn the_cache_evicts_by_bytes_least_recent_first() {
        let mut lru = ByteLru::new(100);
        lru.put((1, 320), Arc::new(vec![0; 40]));
        lru.put((2, 320), Arc::new(vec![0; 40]));
        assert!(lru.get((1, 320)).is_some(), "1 is now the most recent");
        lru.put((3, 320), Arc::new(vec![0; 40]));
        assert!(lru.get((2, 320)).is_none(), "2 was the least recent");
        assert!(lru.get((1, 320)).is_some() && lru.get((3, 320)).is_some());
        assert_eq!(lru.used(), 80);
        lru.put((4, 1024), Arc::new(vec![0; 200]));
        assert_eq!(lru.used(), 0, "something bigger than the cap is not kept");
        lru.put((5, 320), Arc::new(vec![0; 10]));
        lru.retain(|t| t != 5);
        assert_eq!(lru.used(), 0);
    }

    #[test]
    fn requests_that_fall_behind_are_dropped_and_never_render() {
        let gate = Arc::new(RenderGate::new(1, 2));
        let rendered = Arc::new(AtomicUsize::new(0));
        let hold = gate.enter().expect("the one slot");
        let mut waiting = Vec::new();
        for n in 0..4 {
            let (g, r) = (gate.clone(), rendered.clone());
            waiting.push(std::thread::spawn(move || match g.enter() {
                Some(_p) => {
                    r.fetch_add(1, Ordering::SeqCst);
                    Some(n)
                }
                None => None,
            }));
            // Arrive in order.
            std::thread::sleep(std::time::Duration::from_millis(30));
        }
        drop(hold);
        let outcomes: Vec<Option<i32>> = waiting.into_iter().map(|h| h.join().unwrap()).collect();
        assert_eq!(outcomes, [None, None, Some(2), Some(3)], "the two oldest scrolled away");
        assert_eq!(rendered.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn the_newest_request_is_served_first() {
        let gate = Arc::new(RenderGate::new(1, 10));
        let order = Arc::new(Mutex::new(Vec::new()));
        let hold = gate.enter().unwrap();
        let mut hs = Vec::new();
        for n in 0..3 {
            let (g, o) = (gate.clone(), order.clone());
            hs.push(std::thread::spawn(move || {
                let _p = g.enter().unwrap();
                o.lock().unwrap().push(n);
            }));
            std::thread::sleep(std::time::Duration::from_millis(30));
        }
        drop(hold);
        for h in hs {
            h.join().unwrap();
        }
        assert_eq!(*order.lock().unwrap(), [2, 1, 0]);
    }

    #[test]
    fn an_unknown_token_is_missing_without_waiting() {
        let cache = PreviewCache::default();
        assert_eq!(cache.render(42, PREVIEW_EDGE, None), Thumb::Missing);
    }
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DeviceStorage {
    #[ts(type = "number")]
    pub total_bytes: u64,
    #[ts(type = "number")]
    pub free_bytes: u64,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DevicePreview {
    /// For `photos://device-thumb/{token}`.
    pub token: String,
    pub file_name: String,
    #[ts(type = "number | null")]
    pub modified_at: Option<i64>,
    pub video: bool,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ConnectedDevice {
    /// `volume` or `iphone`.
    pub kind: String,
    pub id: String,
    pub name: String,
    /// `None` when the device will not say (a locked phone).
    pub storage: Option<DeviceStorage>,
    pub total_items: u32,
    pub new_items: u32,
    #[ts(type = "number")]
    pub total_bytes: u64,
    pub newest: Vec<DevicePreview>,
    /// For a card or drive: whether the OS treats it as removable.
    pub removable: bool,
}

fn gone(what: &str, id: &str) -> AppError {
    AppError { code: "device_gone".into(), message: format!("That {what} is no longer connected."), detail: Some(id.into()) }
}

/// Everything the device page shows. Slow on a phone (a USB listing of its whole camera
/// roll), so it runs off the main thread.
pub fn describe(app: &AppHandle, kind: &str, id: &str) -> AppResult<ConnectedDevice> {
    let state = app.state::<AppState>();
    let (device, name, storage, removable, key) = match kind {
        "volume" => {
            let info = volumes::snapshot().into_iter().find(|v| v.mount == id).ok_or_else(|| gone("card or drive", id))?;
            let total_space: u64 = info.key.rsplit(':').next().and_then(|s| s.parse().ok()).unwrap_or(0);
            let storage = sysinfo::Disks::new_with_refreshed_list()
                .iter()
                .find(|d| d.mount_point().to_string_lossy() == info.mount)
                .map(|d| DeviceStorage { total_bytes: d.total_space(), free_bytes: d.available_space() });
            let device = DeviceRef::Volume { mount: PathBuf::from(&info.mount), label: info.label.clone(), total_space };
            (device, info.label, storage, info.removable, info.key)
        }
        "iphone" => {
            let phone = photos_import::iphone::list_iphones()?.into_iter().find(|p| p.udid == id).ok_or_else(|| gone("iPhone", id))?;
            if !phone.paired {
                return Err(photos_import::IphoneError::TrustRequired.into());
            }
            let source = IphoneSource::new(&phone.udid, &phone.name);
            let storage = source.storage().ok().map(|(total_bytes, free_bytes)| DeviceStorage { total_bytes, free_bytes });
            let key = source.key();
            (DeviceRef::Iphone { udid: phone.udid, name: phone.name.clone() }, phone.name, storage, true, key)
        }
        "mac" => {
            // A Mac disk's page lists its libraries and folders itself; this is its header.
            let (info, _) = crate::macdisks::get(id).ok_or_else(|| gone("Mac disk", id))?;
            let storage = info.free_bytes.map(|free_bytes| DeviceStorage { total_bytes: info.total_bytes, free_bytes });
            return Ok(ConnectedDevice {
                kind: kind.into(),
                id: id.into(),
                name: info.name,
                storage,
                total_items: 0,
                new_items: 0,
                total_bytes: 0,
                newest: Vec::new(),
                removable: true,
            });
        }
        other => return Err(AppError { code: "bad_device_kind".into(), message: "Unknown kind of device.".into(), detail: Some(other.into()) }),
    };

    let source = device.source().ok_or_else(|| gone("card or drive", id))?;
    let items = source.enumerate().map_err(|e| AppError {
        code: "device_unreadable".into(),
        message: "The device's photos could not be read.".into(),
        detail: Some(e.to_string()),
    })?;
    let new_items = match state.library() {
        Ok(lib) => {
            let seen_items: Vec<SeenItem<'_>> = items.iter().map(|i| SeenItem { rel_path: &i.rel_path, size: i.size, mtime: i.modified_at }).collect();
            lib.db.with_conn(|c| seen::unseen_indexes(c, &key, &seen_items)).map(|u| u.len()).unwrap_or(items.len())
        }
        Err(_) => items.len(),
    };
    let newest = preview::newest(&items, NEWEST)
        .into_iter()
        .map(|item| DevicePreview {
            token: state.device_previews.register(&device, item).to_string(),
            file_name: item.file_name.clone(),
            modified_at: item.modified_at,
            video: photos_media::kind_for_extension(item.file_name.rsplit('.').next().unwrap_or("")) == photos_media::MediaKind::Video,
        })
        .collect();
    Ok(ConnectedDevice {
        kind: kind.into(),
        id: id.into(),
        name,
        storage,
        total_items: items.len() as u32,
        new_items: new_items as u32,
        total_bytes: items.iter().map(|i| i.size).sum(),
        newest,
        removable,
    })
}
