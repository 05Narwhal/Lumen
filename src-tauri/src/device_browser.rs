//! The device browser (Round 4E): everything on a card, an iPhone, a folder on a Mac disk or
//! an Apple Photos library, held in memory while the browser is open and paged over
//! `invoke`, so the user can pick exactly what to import.
//!
//! A session is the source's `enumerate()` grouped into what the import treats as one item
//! (a Live Photo's still and video, a RAW and its JPEG), sorted newest first. Nothing is
//! written anywhere: a device is a read-only source. Items keep their `index` for the life of
//! the session, so a selection survives filters and the background date pass re-sorting them.
//! Pages are keyed by position in the filtered order (no SQL; the list is in memory).

use std::collections::{HashMap, HashSet, VecDeque};
use std::io::Read;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

use chrono::{Local, Offset, TimeZone};
use photos_import::{ImportSource, SourceItem};
use photos_media::MediaKind;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// The most a page may hold; the frontend asks for ~200.
pub const PAGE_MAX: u32 = 500;
/// Browsers open at once. Opening another closes the oldest, so a webview reload that never
/// said goodbye cannot pile sessions up.
const MAX_SESSIONS: usize = 4;
/// How much of a file the date pass reads: EXIF in a JPEG or TIFF-based RAW and the XMP
/// packet are near the front.
const HEAD_BYTES: u64 = 256 * 1024;
/// The date pass publishes what it found this often, so the grid re-sorts in steps rather
/// than once per file.
const PUBLISH_EVERY: Duration = Duration::from_millis(800);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export)]
pub enum ItemKind {
    Photo,
    Video,
    Raw,
    Live,
}

/// Where an item's date came from: the photo's own EXIF or XMP, Apple Photos' catalogue, or
/// the file's modified time (on a card, often the moment it was copied there).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export)]
pub enum DateSource {
    Exif,
    Photos,
    Modified,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct BrowserItem {
    /// Stable for the session: selections are kept by it.
    pub index: u32,
    /// For `photos://device-thumb/{token}`.
    pub token: String,
    pub file_name: String,
    /// Every file of the item (a Live Photo's video and a RAW's JPEG included).
    #[ts(type = "number")]
    pub size: u64,
    pub files: u32,
    /// Local wall time as unix seconds, like `captured_at`: format it as UTC.
    #[ts(type = "number | null")]
    pub date: Option<i64>,
    pub date_source: Option<DateSource>,
    pub kind: ItemKind,
    /// Already brought into the library from this source.
    pub imported: bool,
}

#[derive(Debug, Clone, Default, Deserialize, TS)]
#[serde(rename_all = "camelCase", default)]
#[ts(export)]
pub struct BrowserFilter {
    /// Inclusive, in the same wall-time seconds as `BrowserItem.date`.
    #[ts(type = "number | null")]
    pub from: Option<i64>,
    #[ts(type = "number | null")]
    pub to: Option<i64>,
    /// Empty means every kind.
    pub kinds: Vec<ItemKind>,
    pub new_only: bool,
    /// Whether items with no date at all stay in view when a date range is set.
    pub include_undated: bool,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct BrowserPage {
    pub items: Vec<BrowserItem>,
    pub next_cursor: Option<u32>,
    /// How many items match the filter.
    pub matching: u32,
    /// Moves whenever the order or the *imported* marks change; a page from an older
    /// generation is refetched.
    pub generation: u32,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct BrowserSession {
    pub id: u32,
    pub name: String,
    pub total: u32,
    /// Items not imported from this source before.
    pub new: u32,
    #[ts(type = "number")]
    pub total_bytes: u64,
    #[ts(type = "number | null")]
    pub date_min: Option<i64>,
    #[ts(type = "number | null")]
    pub date_max: Option<i64>,
    /// The source can say what was imported from it before (`import_seen`); a folder on a
    /// Mac disk cannot, and the *New only* switch is hidden for it.
    pub knows_imported: bool,
    /// Dates are still being read from the files; the order may change.
    pub dating: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct BrowserSelection {
    pub count: u32,
    pub files: u32,
    #[ts(type = "number")]
    pub bytes: u64,
    /// Of `count`, the items already in the library (ingest skips their content).
    pub imported: u32,
}

/// `device-browser://changed`: dates were read or an import finished; refetch what is shown.
#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct BrowserChanged {
    pub session: u32,
    pub generation: u32,
    pub dating: bool,
}

/// One tile: the files the import treats as one photo, the shown one first.
#[derive(Debug, Clone)]
pub struct Entry {
    pub files: Vec<SourceItem>,
    pub kind: ItemKind,
    pub date: Option<i64>,
    pub date_source: Option<DateSource>,
    pub imported: bool,
}

impl Entry {
    pub fn primary(&self) -> &SourceItem {
        &self.files[0]
    }

    fn size(&self) -> u64 {
        self.files.iter().map(|f| f.size).sum()
    }
}

fn media_kind(item: &SourceItem) -> MediaKind {
    photos_media::kind_for_extension(item.file_name.rsplit('.').next().unwrap_or(""))
}

/// Shown first: a still the webview-free preview path renders best, then a RAW, then video.
fn rank(kind: MediaKind) -> u8 {
    match kind {
        MediaKind::Image | MediaKind::Heic | MediaKind::Jxl => 0,
        MediaKind::Raw => 1,
        MediaKind::Video => 2,
        MediaKind::Unknown => 3,
    }
}

fn stem(name: &str) -> &str {
    name.rsplit_once('.').map_or(name, |(s, _)| s)
}

fn dir(rel_path: &str) -> &str {
    rel_path.rsplit_once('/').map_or("", |(d, _)| d)
}

/// The pairing key on a card or phone: the same name in the same folder, any case
/// (`IMG_0001.HEIC` + `IMG_0001.MOV`). A RAW is never paired: a camera shooting RAW + JPEG
/// shows both files, as the library does, rather than one tile hiding the other.
pub fn pair_by_name(item: &SourceItem) -> String {
    let raw = if media_kind(item) == MediaKind::Raw { "\0raw" } else { "" };
    format!("{}/{}{raw}", dir(&item.rel_path).to_ascii_lowercase(), stem(&item.file_name).to_ascii_lowercase())
}

/// The pairing key in an Apple Photos library: a companion is `<UUID>_3.mov` (Live video) or
/// `<UUID>_4.<raw>` (RAW alternate) beside `<UUID>.<ext>`. Two photos with the same original
/// name stay two items, because their UUIDs differ.
pub fn pair_by_uuid(item: &SourceItem) -> String {
    let s = stem(&item.rel_path);
    let s = s.strip_suffix("_3").or_else(|| s.strip_suffix("_4")).unwrap_or(s);
    s.to_ascii_lowercase()
}

/// Whether the kind chip `k` shows `e`. A RAW is a photo as well as a RAW: *Photos* (the usual
/// pick) shows every still, *RAW* narrows to the RAWs.
fn shows_as(e: &Entry, k: ItemKind) -> bool {
    e.kind == k || (k == ItemKind::Photo && e.kind == ItemKind::Raw)
}

/// Groups `items` into entries, the shown file first, and decides each one's kind.
/// `date` dates an entry by its shown file.
pub fn group(items: Vec<SourceItem>, key: impl Fn(&SourceItem) -> String, date: impl Fn(&SourceItem) -> (Option<i64>, Option<DateSource>)) -> Vec<Entry> {
    let mut at: HashMap<String, usize> = HashMap::new();
    let mut groups: Vec<Vec<SourceItem>> = Vec::new();
    for item in items {
        match at.get(&key(&item)) {
            Some(&i) => groups[i].push(item),
            None => {
                at.insert(key(&item), groups.len());
                groups.push(vec![item]);
            }
        }
    }
    groups
        .into_iter()
        .map(|mut files| {
            files.sort_by(|a, b| rank(media_kind(a)).cmp(&rank(media_kind(b))).then(a.rel_path.cmp(&b.rel_path)));
            let kinds: Vec<MediaKind> = files.iter().map(media_kind).collect();
            let still = kinds.iter().any(|k| rank(*k) <= 1);
            let video = kinds.contains(&MediaKind::Video);
            let kind = if still && video {
                ItemKind::Live
            } else if kinds.contains(&MediaKind::Raw) {
                ItemKind::Raw
            } else if video {
                ItemKind::Video
            } else {
                ItemKind::Photo
            };
            let (date, date_source) = date(&files[0]);
            Entry { files, kind, date, date_source, imported: false }
        })
        .collect()
}

/// A file's modified time (UTC) as the wall time it was on this computer, so it sorts and
/// filters with EXIF dates, which are wall times.
pub fn wall_time(utc: i64) -> i64 {
    match Local.timestamp_opt(utc, 0).single() {
        Some(t) => utc + t.offset().fix().local_minus_utc() as i64,
        None => utc,
    }
}

/// The entries of a session in their current order.
pub struct Catalogue {
    pub entries: Vec<Entry>,
    /// Indexes into `entries`, newest first, undated last.
    order: Vec<u32>,
    pub generation: u32,
}

impl Catalogue {
    pub fn new(entries: Vec<Entry>) -> Self {
        let mut c = Self { order: (0..entries.len() as u32).collect(), entries, generation: 0 };
        c.resort();
        c
    }

    fn resort(&mut self) {
        let e = &self.entries;
        self.order.sort_by(|&a, &b| {
            let (a, b) = (&e[a as usize], &e[b as usize]);
            b.date.unwrap_or(i64::MIN).cmp(&a.date.unwrap_or(i64::MIN)).then_with(|| b.primary().rel_path.cmp(&a.primary().rel_path))
        });
    }

    fn matches(&self, e: &Entry, f: &BrowserFilter) -> bool {
        if f.new_only && e.imported {
            return false;
        }
        if !f.kinds.is_empty() && !f.kinds.iter().any(|&k| shows_as(e, k)) {
            return false;
        }
        match e.date {
            Some(d) => f.from.is_none_or(|from| d >= from) && f.to.is_none_or(|to| d <= to),
            None => f.include_undated || (f.from.is_none() && f.to.is_none()),
        }
    }

    /// Indexes matching `f`, in order.
    pub fn ids(&self, f: &BrowserFilter) -> Vec<u32> {
        self.order.iter().copied().filter(|&i| self.matches(&self.entries[i as usize], f)).collect()
    }

    /// Up to `limit` indexes from position `cursor` of the filtered order, the next cursor,
    /// and how many match in all.
    pub fn page(&self, f: &BrowserFilter, cursor: Option<u32>, limit: u32) -> (Vec<u32>, Option<u32>, u32) {
        let all = self.ids(f);
        let start = (cursor.unwrap_or(0) as usize).min(all.len());
        let end = (start + limit.clamp(1, PAGE_MAX) as usize).min(all.len());
        let next = (end < all.len()).then_some(end as u32);
        (all[start..end].to_vec(), next, all.len() as u32)
    }

    pub fn selection(&self, indexes: &[u32]) -> BrowserSelection {
        let mut s = BrowserSelection::default();
        let unique: HashSet<u32> = indexes.iter().copied().collect();
        for e in unique.iter().filter_map(|&i| self.entries.get(i as usize)) {
            s.count += 1;
            s.files += e.files.len() as u32;
            s.bytes += e.size();
            s.imported += e.imported as u32;
        }
        s
    }

    /// Dates read from the files themselves; re-sorts and moves the generation on.
    pub fn set_dates(&mut self, found: &[(u32, i64)]) {
        if found.is_empty() {
            return;
        }
        for &(i, date) in found {
            if let Some(e) = self.entries.get_mut(i as usize) {
                e.date = Some(date);
                e.date_source = Some(DateSource::Exif);
            }
        }
        self.resort();
        self.generation += 1;
    }

    /// An entry is imported when every one of its files is.
    pub fn mark_imported(&mut self, seen: impl Fn(&SourceItem) -> bool) {
        for e in &mut self.entries {
            e.imported = e.files.iter().all(&seen);
        }
        self.generation += 1;
    }

    pub fn new_count(&self) -> u32 {
        self.entries.iter().filter(|e| !e.imported).count() as u32
    }

    pub fn date_range(&self) -> (Option<i64>, Option<i64>) {
        let dates = self.entries.iter().filter_map(|e| e.date);
        let (mut lo, mut hi) = (None::<i64>, None::<i64>);
        for d in dates {
            lo = Some(lo.map_or(d, |l| l.min(d)));
            hi = Some(hi.map_or(d, |h| h.max(d)));
        }
        (lo, hi)
    }

    /// One item per picked tile, sized as all its files, for the space check: the dialog
    /// counts tiles, as the browser does.
    pub fn as_items(&self, indexes: &[u32]) -> Vec<SourceItem> {
        let unique: HashSet<u32> = indexes.iter().copied().collect();
        self.order
            .iter()
            .filter(|i| unique.contains(i))
            .map(|&i| {
                let e = &self.entries[i as usize];
                SourceItem { size: e.size(), modified_at: e.date, ..e.primary().clone() }
            })
            .collect()
    }

    /// The source items to import for `indexes`: whole entries, so a Live Photo's video and
    /// a RAW's JPEG come along with the tile that was picked. With `limit`, the newest
    /// entries whose files fit in that many bytes.
    pub fn files_for(&self, indexes: &[u32], limit: Option<u64>) -> Vec<SourceItem> {
        let unique: HashSet<u32> = indexes.iter().copied().collect();
        let picked: Vec<&Entry> = self.order.iter().filter(|i| unique.contains(i)).map(|&i| &self.entries[i as usize]).collect();
        let mut room = limit.unwrap_or(u64::MAX);
        let mut out = Vec::new();
        // `order` is newest first, which is the order the space check keeps.
        for e in picked {
            let size = e.size();
            if size > room {
                continue;
            }
            room -= size;
            out.extend(e.files.iter().cloned());
        }
        out
    }
}

/// Everything a session needs beyond the catalogue: the source and how to import from it.
pub struct Session {
    pub id: u32,
    pub name: String,
    pub source: Arc<dyn ImportSource>,
    /// `import_seen` key; `None` for a folder on a Mac disk.
    pub seen_key: Option<String>,
    /// What the import history shows as the batch's path.
    pub path_label: Option<String>,
    /// A card's mount, re-counted by the volume watcher after an import.
    pub mount: Option<PathBuf>,
    pub catalogue: RwLock<Catalogue>,
    /// index → `device-thumb` token, handed out as pages are read.
    pub tokens: Mutex<HashMap<u32, u64>>,
    pub closed: AtomicBool,
    pub dating: AtomicBool,
}

impl Session {
    pub fn summary(&self) -> BrowserSession {
        let c = self.catalogue.read().expect("catalogue");
        let (date_min, date_max) = c.date_range();
        BrowserSession {
            id: self.id,
            name: self.name.clone(),
            total: c.entries.len() as u32,
            new: c.new_count(),
            total_bytes: c.entries.iter().map(Entry::size).sum(),
            date_min,
            date_max,
            knows_imported: self.seen_key.is_some(),
            dating: self.dating.load(Ordering::Relaxed),
        }
    }

    /// A page of items; `token` registers each shown file with the preview cache once.
    pub fn page(&self, filter: &BrowserFilter, cursor: Option<u32>, limit: u32, mut token: impl FnMut(&SourceItem) -> u64) -> BrowserPage {
        let c = self.catalogue.read().expect("catalogue");
        let (ids, next_cursor, matching) = c.page(filter, cursor, limit);
        let mut tokens = self.tokens.lock().expect("tokens");
        let items = ids
            .into_iter()
            .map(|i| {
                let e = &c.entries[i as usize];
                let t = *tokens.entry(i).or_insert_with(|| token(e.primary()));
                BrowserItem {
                    index: i,
                    token: t.to_string(),
                    file_name: e.primary().file_name.clone(),
                    size: e.size(),
                    files: e.files.len() as u32,
                    date: e.date,
                    date_source: e.date_source,
                    kind: e.kind,
                    imported: e.imported,
                }
            })
            .collect();
        BrowserPage { items, next_cursor, matching, generation: c.generation }
    }

    pub fn changed(&self) -> BrowserChanged {
        BrowserChanged {
            session: self.id,
            generation: self.catalogue.read().expect("catalogue").generation,
            dating: self.dating.load(Ordering::Relaxed),
        }
    }
}

/// The open browsers.
#[derive(Default)]
pub struct Browsers {
    next: AtomicU32,
    sessions: Mutex<VecDeque<Arc<Session>>>,
}

impl Browsers {
    pub fn next_id(&self) -> u32 {
        self.next.fetch_add(1, Ordering::Relaxed) + 1
    }

    /// Keeps `session`, returning any closed to make room (their tokens are to be forgotten).
    pub fn insert(&self, session: Arc<Session>) -> Vec<Arc<Session>> {
        let mut all = self.sessions.lock().expect("browsers");
        all.push_back(session);
        let mut evicted = Vec::new();
        while all.len() > MAX_SESSIONS {
            if let Some(old) = all.pop_front() {
                old.closed.store(true, Ordering::Relaxed);
                evicted.push(old);
            }
        }
        evicted
    }

    pub fn get(&self, id: u32) -> Option<Arc<Session>> {
        self.sessions.lock().expect("browsers").iter().find(|s| s.id == id).cloned()
    }

    pub fn remove(&self, id: u32) -> Option<Arc<Session>> {
        let mut all = self.sessions.lock().expect("browsers");
        let at = all.iter().position(|s| s.id == id)?;
        let s = all.remove(at)?;
        s.closed.store(true, Ordering::Relaxed);
        Some(s)
    }
}

/// Reads the capture date from the head of every photo dated only by its file time, newest
/// first, and hands what it found to the catalogue in batches (`publish` is told each new
/// generation). Stops as soon as the session closes. Videos keep their file time: their
/// date is deep in the container, and a card's videos are rarely re-copied alone.
pub fn date_pass(session: &Session, mut publish: impl FnMut(&Session)) {
    let todo: Vec<(u32, SourceItem)> = {
        let c = session.catalogue.read().expect("catalogue");
        c.order
            .iter()
            .map(|&i| (i, &c.entries[i as usize]))
            .filter(|(_, e)| e.date_source != Some(DateSource::Exif) && e.date_source != Some(DateSource::Photos) && e.kind != ItemKind::Video)
            .map(|(i, e)| (i, e.primary().clone()))
            .collect()
    };
    session.dating.store(true, Ordering::Relaxed);
    let mut found: Vec<(u32, i64)> = Vec::new();
    let mut last = Instant::now();
    for (i, item) in todo {
        if session.closed.load(Ordering::Relaxed) {
            break;
        }
        let mut head = Vec::new();
        let read = session.source.open(&item).and_then(|r| r.take(HEAD_BYTES).read_to_end(&mut head).map_err(Into::into));
        if read.is_err() {
            if !session.source.is_connected() {
                break;
            }
            continue;
        }
        if let Some(c) = photos_media::capture_in_head(&head) {
            found.push((i, c.local));
        }
        if !found.is_empty() && last.elapsed() >= PUBLISH_EVERY {
            session.catalogue.write().expect("catalogue").set_dates(&found);
            found.clear();
            last = Instant::now();
            publish(session);
        }
    }
    session.catalogue.write().expect("catalogue").set_dates(&found);
    session.dating.store(false, Ordering::Relaxed);
    publish(session);
}

#[cfg(test)]
mod tests {
    use super::*;
    use photos_import::ImportError;

    fn item(rel: &str, size: u64, at: Option<i64>) -> SourceItem {
        let name = rel.rsplit('/').next().unwrap().to_string();
        SourceItem { rel_path: rel.into(), file_name: name, size, modified_at: at, created_at: None }
    }

    fn plain(i: &SourceItem) -> (Option<i64>, Option<DateSource>) {
        (i.modified_at, i.modified_at.map(|_| DateSource::Modified))
    }

    /// A card of `n` JPEGs one minute apart, the first undated every 50.
    fn card(n: usize) -> Catalogue {
        let items = (0..n).map(|k| item(&format!("100CANON/IMG_{k:04}.JPG"), 1000, (k % 50 != 0).then_some(k as i64 * 60))).collect();
        Catalogue::new(group(items, pair_by_name, plain))
    }

    fn all() -> BrowserFilter {
        BrowserFilter { include_undated: true, ..Default::default() }
    }

    #[test]
    fn pages_cover_every_item_once_newest_first() {
        let c = card(1000);
        let (mut seen, mut cursor, mut dates) = (Vec::new(), None, Vec::new());
        loop {
            let (ids, next, matching) = c.page(&all(), cursor, 200);
            assert_eq!(matching, 1000);
            dates.extend(ids.iter().map(|&i| c.entries[i as usize].date));
            seen.extend(ids);
            match next {
                Some(n) => cursor = Some(n),
                None => break,
            }
        }
        assert_eq!(seen.len(), 1000);
        assert_eq!(seen.iter().collect::<HashSet<_>>().len(), 1000);
        let dated: Vec<i64> = dates.iter().map_while(|d| *d).collect();
        assert!(dated.windows(2).all(|w| w[0] >= w[1]), "newest first");
        assert_eq!(dated.len(), 980, "the undated come last");
        assert_eq!(c.ids(&all()), seen, "ids are the pages joined");
    }

    #[test]
    fn a_date_range_is_inclusive_and_can_drop_the_undated() {
        let c = card(1000);
        let f = BrowserFilter { from: Some(60 * 60), to: Some(60 * 120), include_undated: true, ..Default::default() };
        let got = c.ids(&f);
        let dated: Vec<i64> = got.iter().filter_map(|&i| c.entries[i as usize].date).collect();
        // Minutes 60..=120, less the undated 100th.
        assert_eq!(dated.len(), 60);
        assert_eq!((dated.iter().min(), dated.iter().max()), (Some(&3600), Some(&7200)));
        assert_eq!(got.len(), 80, "the 20 undated are kept");
        let f = BrowserFilter { include_undated: false, ..f };
        assert_eq!(c.ids(&f).len(), 60);
        // Without a range, nothing is hidden for having no date.
        assert_eq!(c.ids(&BrowserFilter::default()).len(), 1000);
    }

    #[test]
    fn new_only_and_kinds_filter() {
        let mut c = card(10);
        c.mark_imported(|i| i.rel_path.as_str() < "100CANON/IMG_0004");
        assert_eq!(c.new_count(), 6);
        assert_eq!(c.ids(&BrowserFilter { new_only: true, ..all() }).len(), 6);
        assert_eq!(c.ids(&BrowserFilter { kinds: vec![ItemKind::Video], ..all() }).len(), 0);
        assert_eq!(c.ids(&BrowserFilter { kinds: vec![ItemKind::Photo], ..all() }).len(), 10);
        let s = c.selection(&[0, 1, 9, 9]);
        assert_eq!(s, BrowserSelection { count: 3, files: 3, bytes: 3000, imported: 2 });
    }

    #[test]
    fn a_live_pair_is_one_item_and_a_raw_beside_its_jpeg_is_its_own() {
        let items = vec![
            item("DCIM/100APPLE/IMG_0001.HEIC", 3, Some(10)),
            item("DCIM/100APPLE/IMG_0001.MOV", 5, Some(10)),
            item("DCIM/100APPLE/IMG_0002.MOV", 7, Some(20)),
            item("DCIM/101NIKON/DSC_1.NEF", 20, Some(30)),
            item("DCIM/101NIKON/dsc_1.jpg", 4, Some(30)),
            // Same name, another folder: another photo.
            item("DCIM/102NIKON/DSC_1.JPG", 4, Some(40)),
        ];
        let c = Catalogue::new(group(items, pair_by_name, plain));
        let kinds: Vec<(ItemKind, &str, usize)> =
            c.ids(&all()).iter().map(|&i| &c.entries[i as usize]).map(|e| (e.kind, e.primary().file_name.as_str(), e.files.len())).collect();
        assert_eq!(
            kinds,
            [
                (ItemKind::Photo, "DSC_1.JPG", 1),
                (ItemKind::Photo, "dsc_1.jpg", 1),
                (ItemKind::Raw, "DSC_1.NEF", 1),
                (ItemKind::Video, "IMG_0002.MOV", 1),
                (ItemKind::Live, "IMG_0001.HEIC", 2)
            ]
        );
        // *Photos* shows every still, RAWs included; *RAW* only the RAW.
        let photos = BrowserFilter { kinds: vec![ItemKind::Photo], ..all() };
        assert_eq!(c.ids(&photos).len(), 3);
        assert_eq!(c.ids(&BrowserFilter { kinds: vec![ItemKind::Raw], ..all() }).len(), 1);
        // Picking the Live tile brings its video.
        let live = c.ids(&all())[4];
        let files: Vec<String> = c.files_for(&[live], None).into_iter().map(|i| i.rel_path).collect();
        assert_eq!(files, ["DCIM/100APPLE/IMG_0001.HEIC", "DCIM/100APPLE/IMG_0001.MOV"]);
    }

    #[test]
    fn photos_library_companions_pair_by_uuid_not_by_name() {
        let named = |rel: &str, name: &str| SourceItem { rel_path: rel.into(), file_name: name.into(), size: 1, modified_at: Some(1), created_at: None };
        let items = vec![
            named("originals/A/AAA.heic", "IMG_0001.HEIC"),
            named("originals/A/AAA_3.mov", "IMG_0001.MOV"),
            named("originals/A/AAA_4.dng", "IMG_0001.DNG"),
            named("originals/A/ABB.heic", "IMG_0001.HEIC"),
        ];
        let c = Catalogue::new(group(items, pair_by_uuid, plain));
        let mut sizes: Vec<usize> = c.entries.iter().map(|e| e.files.len()).collect();
        sizes.sort();
        assert_eq!(sizes, [1, 3]);
        assert!(c.entries.iter().any(|e| e.kind == ItemKind::Live && e.primary().rel_path == "originals/A/AAA.heic"));
    }

    #[test]
    fn the_space_limit_keeps_whole_items_newest_first() {
        let items = vec![
            item("a/IMG_1.HEIC", 3, Some(10)),
            item("a/IMG_1.MOV", 5, Some(10)),
            item("a/IMG_2.JPG", 4, Some(20)),
            item("a/IMG_3.JPG", 4, Some(30)),
        ];
        let c = Catalogue::new(group(items, pair_by_name, plain));
        let every: Vec<u32> = c.ids(&all());
        let names = |v: Vec<SourceItem>| v.into_iter().map(|i| i.file_name).collect::<Vec<_>>();
        assert_eq!(names(c.files_for(&every, Some(10))), ["IMG_3.JPG", "IMG_2.JPG"], "the Live pair does not fit and is not split");
        assert_eq!(names(c.files_for(&every, None)).len(), 4);
        let tiles = c.as_items(&every);
        assert_eq!(tiles.iter().map(|t| t.size).collect::<Vec<_>>(), [4, 4, 8], "one item per tile, sized with its video");
    }

    /// Files whose head holds an XMP capture date; the file time says otherwise.
    struct Card {
        items: Vec<SourceItem>,
    }

    impl ImportSource for Card {
        fn label(&self) -> String {
            "Card".into()
        }
        fn kind(&self) -> i64 {
            photos_core::batches::SOURCE_VOLUME
        }
        fn enumerate(&self) -> Result<Vec<SourceItem>, ImportError> {
            Ok(self.items.clone())
        }
        fn open(&self, item: &SourceItem) -> Result<Box<dyn Read + Send>, ImportError> {
            // IMG_k was taken on day k of 2020.
            let k: i64 = item.file_name[4..8].parse().unwrap();
            let day = chrono::NaiveDate::from_ymd_opt(2020, 1, 1).unwrap() + chrono::Duration::days(k);
            let xmp = format!(r#"<x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:Description xmp:CreateDate="{day}T12:00:00"/></x:xmpmeta>"#);
            Ok(Box::new(std::io::Cursor::new(xmp.into_bytes())))
        }
    }

    #[test]
    fn the_date_pass_reorders_by_the_photos_own_dates() {
        // Copied to the card in reverse: the file times say IMG_0009 is the oldest.
        let items: Vec<SourceItem> = (0..10).map(|k| item(&format!("100/IMG_{k:04}.JPG"), 1, Some(1_000_000 - k))).collect();
        let source = Arc::new(Card { items: items.clone() });
        let session = Session {
            id: 1,
            name: "Card".into(),
            source,
            seen_key: None,
            path_label: None,
            mount: None,
            catalogue: RwLock::new(Catalogue::new(group(items, pair_by_name, plain))),
            tokens: Mutex::new(HashMap::new()),
            closed: AtomicBool::new(false),
            dating: AtomicBool::new(false),
        };
        let before = session.page(&all(), None, 10, |_| 0).items;
        assert_eq!(before[0].file_name, "IMG_0000.JPG");
        let mut published = 0;
        date_pass(&session, |_| published += 1);
        assert!(published >= 1);
        let after = session.page(&all(), None, 10, |_| 0);
        assert_eq!(after.items[0].file_name, "IMG_0009.JPG", "the newest photograph leads");
        assert!(after.items.iter().all(|i| i.date_source == Some(DateSource::Exif)));
        assert!(after.generation > 0);
        assert!(!session.dating.load(Ordering::Relaxed));
    }

    #[test]
    fn a_token_is_handed_out_once_per_item() {
        let c = card(3);
        let session = Session {
            id: 1,
            name: "Card".into(),
            source: Arc::new(Card { items: Vec::new() }),
            seen_key: Some("vol:x".into()),
            path_label: None,
            mount: None,
            catalogue: RwLock::new(c),
            tokens: Mutex::new(HashMap::new()),
            closed: AtomicBool::new(false),
            dating: AtomicBool::new(false),
        };
        let mut n = 0;
        let a = session.page(&all(), None, 3, |_| {
            n += 1;
            n
        });
        let b = session.page(&all(), None, 3, |_| 99);
        assert_eq!(a.items.iter().map(|i| &i.token).collect::<Vec<_>>(), b.items.iter().map(|i| &i.token).collect::<Vec<_>>());
    }

    /// Task 4: what was picked, and only that, comes in and is remembered as seen; a Live
    /// Photo picked by its still brings its video.
    #[test]
    fn an_import_of_a_selection_brings_exactly_it() {
        use photos_import::{run_import, FilteredSource, FolderSource, ImportOptions};

        let tmp = tempfile::tempdir().unwrap();
        let card = tmp.path().join("DCIM");
        std::fs::create_dir_all(&card).unwrap();
        for (i, name) in ["A.JPG", "B.JPG", "C.JPG", "IMG_1.JPG"].iter().enumerate() {
            image::RgbImage::from_fn(48, 32, |x, y| image::Rgb([(x as u8).wrapping_mul(i as u8 + 3), y as u8, 40 * i as u8])).save(card.join(name)).unwrap();
        }
        std::fs::write(card.join("IMG_1.MOV"), b"not really a movie").unwrap();
        let source = Arc::new(FolderSource::new(&card));
        let cat = Catalogue::new(group(source.enumerate().unwrap(), pair_by_name, plain));
        let pick = |name: &str| cat.entries.iter().position(|e| e.primary().file_name == name).unwrap() as u32;
        let files = cat.files_for(&[pick("A.JPG"), pick("C.JPG"), pick("IMG_1.JPG")], None);
        let allow: HashSet<String> = files.into_iter().map(|f| f.rel_path).collect();
        assert_eq!(allow.len(), 4, "the Live Photo's video rides along");

        let lib = photos_core::Library::create(&tmp.path().join("lib")).unwrap();
        let opts = ImportOptions { seen_key: Some("vol:card".into()), ..Default::default() };
        let filtered = FilteredSource::new(source.clone(), allow);
        run_import(&lib, &filtered, &opts, &photos_jobs::CancelToken::new(), &mut |_| {}, &mut |_| {}).unwrap();

        let mut seen: Vec<String> = lib
            .db
            .with_conn(|c| {
                let mut st = c.prepare("SELECT rel_path FROM import_seen WHERE source_key = 'vol:card'")?;
                let rows = st.query_map([], |r| r.get::<_, String>(0))?.collect::<Result<Vec<_>, _>>()?;
                Ok(rows)
            })
            .unwrap();
        seen.sort();
        assert_eq!(seen, ["A.JPG", "C.JPG", "IMG_1.JPG", "IMG_1.MOV"]);
    }

    #[test]
    fn opening_a_fifth_browser_closes_the_oldest() {
        let browsers = Browsers::default();
        let mut evicted = Vec::new();
        for _ in 0..5 {
            let id = browsers.next_id();
            let s = Arc::new(Session {
                id,
                name: String::new(),
                source: Arc::new(Card { items: Vec::new() }),
                seen_key: None,
                path_label: None,
                mount: None,
                catalogue: RwLock::new(Catalogue::new(Vec::new())),
                tokens: Mutex::new(HashMap::new()),
                closed: AtomicBool::new(false),
                dating: AtomicBool::new(false),
            });
            evicted.extend(browsers.insert(s));
        }
        assert_eq!(evicted.len(), 1);
        assert!(evicted[0].closed.load(Ordering::Relaxed));
        assert!(browsers.get(evicted[0].id).is_none());
        assert!(browsers.remove(5).is_some());
        assert!(browsers.get(5).is_none());
    }
}
