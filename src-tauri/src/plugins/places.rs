//! Estimated places and the generic Place review (Round 24 §8.6, `places.write`): a plugin only
//! *suggests*; the person confirms on the review screen; *Write N locations* is the only thing that
//! touches the library. Nothing here knows which plugin is suggesting.
//!
//! `place_source` is an integer column (0035), so an estimated place is `3`
//! ([`PLACE_SOURCE_ESTIMATED`]); the plugin's id is in `place_writes.plugin_id` and in the
//! file's `ESTIMATED:<id>` marker.

use std::collections::{BTreeMap, HashMap};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use photos_core::assets::PLACE_SOURCE_ESTIMATED;
use photos_core::Library;
use photos_plugins::host::{MoveTo, PlaceFlag, PlaceHit, PlaceState, PlacesHost, ReviewEvent, SplitAt, Suggestion};
use rusqlite::params;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::error::{AppError, AppResult};
use crate::originals_gps::{GpsWriter, UndoOutcome, WriteOutcome};

#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ReviewCandidate {
    pub lat: f64,
    pub lon: f64,
    pub confidence: f32,
    pub label: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "kebab-case")]
#[ts(export)]
pub enum PlaceKind {
    Anchor,
    Inferred,
    Guess,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "kebab-case")]
#[ts(export)]
pub enum ReviewFlag {
    FarFromGroup,
    ImpossibleSpeed,
    LowConfidence,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "kebab-case")]
#[ts(export)]
pub enum ReviewStatus {
    Pending,
    Confirmed,
    Rejected,
}

#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ReviewItem {
    #[ts(type = "number")]
    pub asset: i64,
    pub group: String,
    pub state: PlaceKind,
    pub flag: Option<ReviewFlag>,
    pub candidates: Vec<ReviewCandidate>,
    pub status: ReviewStatus,
}

impl ReviewItem {
    /// *Check these*: a guess or a flagged place nobody has looked at.
    pub fn needs_check(&self) -> bool {
        self.status == ReviewStatus::Pending && (self.state == PlaceKind::Guess || self.flag.is_some())
    }

    /// Will be written: confirmed, or an anchor / inference nobody flagged and nobody rejected.
    pub fn is_done(&self) -> bool {
        self.status == ReviewStatus::Confirmed || (self.status == ReviewStatus::Pending && !self.needs_check())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ReviewGroup {
    pub id: String,
    pub label: String,
    pub count: u32,
    /// Photos in it that still need a look.
    pub check: u32,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ReviewCounts {
    pub check: u32,
    pub moments: u32,
    pub done: u32,
    pub rejected: u32,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ReviewState {
    pub groups: Vec<ReviewGroup>,
    pub items: Vec<ReviewItem>,
    pub counts: ReviewCounts,
}

/// What the person did on the screen (the page's side of `review-event`).
#[derive(Debug, Clone, PartialEq, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "camelCase")]
#[ts(export)]
pub enum ReviewAction {
    Confirm { #[ts(type = "Array<number>")] assets: Vec<i64> },
    Reject { #[ts(type = "Array<number>")] assets: Vec<i64> },
    #[serde(rename_all = "camelCase")]
    Move { #[ts(type = "Array<number>")] assets: Vec<i64>, lat: f64, lon: f64 },
    #[serde(rename_all = "camelCase")]
    Split { group: String, #[ts(type = "number")] at_asset: i64 },
}

impl ReviewAction {
    pub fn to_event(&self) -> ReviewEvent {
        let ids = |a: &[i64]| a.iter().map(|&i| i as u64).collect::<Vec<_>>();
        match self {
            ReviewAction::Confirm { assets } => ReviewEvent::Confirm(ids(assets)),
            ReviewAction::Reject { assets } => ReviewEvent::Reject(ids(assets)),
            ReviewAction::Move { assets, lat, lon } => ReviewEvent::Move(MoveTo { assets: ids(assets), lat: *lat, lon: *lon }),
            ReviewAction::Split { group, at_asset } => ReviewEvent::Split(SplitAt { group: group.clone(), at_asset: *at_asset as u64 }),
        }
    }
}

fn item_of(s: &Suggestion, status: ReviewStatus) -> ReviewItem {
    ReviewItem {
        asset: s.asset as i64,
        group: s.group.clone(),
        state: match s.state {
            PlaceState::Anchor => PlaceKind::Anchor,
            PlaceState::Inferred => PlaceKind::Inferred,
            PlaceState::Guess => PlaceKind::Guess,
        },
        flag: s.flag.map(|f| match f {
            PlaceFlag::FarFromGroup => ReviewFlag::FarFromGroup,
            PlaceFlag::ImpossibleSpeed => ReviewFlag::ImpossibleSpeed,
            PlaceFlag::LowConfidence => ReviewFlag::LowConfidence,
        }),
        candidates: s.candidates.iter().take(5).map(|c| ReviewCandidate { lat: c.lat, lon: c.lon, confidence: c.confidence, label: c.label.clone() }).collect(),
        status,
    }
}

/// One plugin's current suggestions: asset → item, plus each group's label.
#[derive(Default)]
struct Review {
    items: BTreeMap<i64, ReviewItem>,
    labels: HashMap<String, String>,
}

impl Review {
    fn merge(&mut self, batch: &[Suggestion]) {
        for s in batch {
            let id = s.asset as i64;
            // A person's answer stands unless the plugin now puts the photo somewhere else.
            let status = match self.items.get(&id) {
                Some(old) if old.status != ReviewStatus::Confirmed || old.candidates.first().map(|c| (c.lat, c.lon)) == s.candidates.first().map(|c| (c.lat, c.lon)) => old.status,
                _ => ReviewStatus::Pending,
            };
            self.labels.insert(s.group.clone(), s.group_label.clone());
            self.items.insert(id, item_of(s, status));
        }
    }

    fn out(&self) -> ReviewState {
        let mut groups: BTreeMap<String, ReviewGroup> = BTreeMap::new();
        let mut counts = ReviewCounts::default();
        let mut items: Vec<ReviewItem> = self.items.values().cloned().collect();
        for it in &items {
            if it.status == ReviewStatus::Rejected {
                counts.rejected += 1;
                continue;
            }
            let g = groups.entry(it.group.clone()).or_insert_with(|| ReviewGroup {
                id: it.group.clone(),
                label: self.labels.get(&it.group).cloned().unwrap_or_default(),
                count: 0,
                check: 0,
            });
            g.count += 1;
            if it.needs_check() {
                g.check += 1;
                counts.check += 1;
            } else if it.is_done() {
                counts.done += 1;
            }
        }
        counts.moments = groups.len() as u32;
        items.sort_by_key(|i| i.asset);
        ReviewState { groups: groups.into_values().collect(), items, counts }
    }

    /// The places *Write* commits: `(asset, lat, lon)` of each done item's best candidate.
    fn to_write(&self) -> Vec<(i64, f64, f64)> {
        self.items.values().filter(|i| i.is_done()).filter_map(|i| i.candidates.first().map(|c| (i.asset, c.lat, c.lon))).collect()
    }
}

/// Every plugin's review, in memory (a suggestion set is rebuilt by the plugin's next run).
#[derive(Default)]
pub struct ReviewStore {
    sets: Mutex<HashMap<String, Review>>,
    /// A running write's cancel flag, by plugin id.
    writing: Mutex<HashMap<String, Arc<AtomicBool>>>,
}

impl ReviewStore {
    fn sets(&self) -> std::sync::MutexGuard<'_, HashMap<String, Review>> {
        self.sets.lock().unwrap_or_else(|p| p.into_inner())
    }

    pub fn suggest(&self, plugin: &str, batch: &[Suggestion]) {
        self.sets().entry(plugin.to_string()).or_default().merge(batch);
    }

    pub fn state(&self, plugin: &str) -> ReviewState {
        self.sets().get(plugin).map(Review::out).unwrap_or_default()
    }

    /// Applies what the person did, forwards it to the plugin (`forward` is its `on-review`; a
    /// plugin without one answers nothing) and merges the suggestions it changed.
    pub fn event(&self, plugin: &str, action: &ReviewAction, forward: impl FnOnce(&ReviewEvent) -> Result<Vec<Suggestion>, String>) -> Result<ReviewState, String> {
        {
            let mut sets = self.sets();
            let review = sets.entry(plugin.to_string()).or_default();
            let mark = |review: &mut Review, assets: &[i64], status| {
                for a in assets {
                    if let Some(i) = review.items.get_mut(a) {
                        i.status = status;
                    }
                }
            };
            match action {
                ReviewAction::Confirm { assets } => mark(review, assets, ReviewStatus::Confirmed),
                ReviewAction::Reject { assets } => mark(review, assets, ReviewStatus::Rejected),
                ReviewAction::Move { assets, lat, lon } => {
                    for a in assets {
                        if let Some(i) = review.items.get_mut(a) {
                            i.candidates.insert(0, ReviewCandidate { lat: *lat, lon: *lon, confidence: 1.0, label: None });
                            i.candidates.truncate(5);
                            i.status = ReviewStatus::Confirmed;
                        }
                    }
                }
                ReviewAction::Split { .. } => {}
            }
        }
        // The plugin runs without the lock: it may call `suggest` itself.
        let changed = forward(&action.to_event())?;
        let mut sets = self.sets();
        let review = sets.entry(plugin.to_string()).or_default();
        review.merge(&changed);
        Ok(review.out())
    }

    fn to_write(&self, plugin: &str) -> Vec<(i64, f64, f64)> {
        self.sets().get(plugin).map(Review::to_write).unwrap_or_default()
    }

    /// Drops what was written from the review, so a second *Write* has nothing left to write twice.
    fn written(&self, plugin: &str, assets: &[i64]) {
        if let Some(r) = self.sets().get_mut(plugin) {
            for a in assets {
                r.items.remove(a);
            }
        }
    }

    /// Marks a write as running; `None` when one already is.
    pub fn begin_write(&self, plugin: &str) -> Option<Arc<AtomicBool>> {
        let mut w = self.writing.lock().unwrap_or_else(|p| p.into_inner());
        if w.contains_key(plugin) {
            return None;
        }
        let flag = Arc::new(AtomicBool::new(false));
        w.insert(plugin.to_string(), flag.clone());
        Some(flag)
    }

    pub fn end_write(&self, plugin: &str) {
        self.writing.lock().unwrap_or_else(|p| p.into_inner()).remove(plugin);
    }

    pub fn cancel_write(&self, plugin: &str) {
        if let Some(f) = self.writing.lock().unwrap_or_else(|p| p.into_inner()).get(plugin) {
            f.store(true, Ordering::Relaxed);
        }
    }
}

/// `lumen:plugin/places` for the app: suggestions go to the [`ReviewStore`], names come from the
/// offline atlas.
pub struct AppPlaces {
    pub store: Arc<ReviewStore>,
}

impl PlacesHost for AppPlaces {
    fn suggest(&self, plugin: &str, batch: Vec<Suggestion>) -> Result<(), String> {
        self.store.suggest(plugin, &batch);
        Ok(())
    }

    fn lookup(&self, name: &str) -> Vec<PlaceHit> {
        photos_core::places::search(name, 8)
            .into_iter()
            .map(|(label, lat, lon)| {
                let country = label.split_once(", ").map(|(_, c)| c.to_string());
                PlaceHit { name: label, lat, lon, country }
            })
            .collect()
    }
}

// ----------------------------------------------------------------------------- write and undo

/// What puts GPS into (and takes it out of) a file: [`GpsWriter`], or a test's own.
pub trait FileGps {
    fn write(&mut self, path: &Path, lat: f64, lon: f64, marker: &str) -> AppResult<WriteOutcome>;
    fn remove(&mut self, path: &Path, expected_hash: &str) -> AppResult<UndoOutcome>;
}

impl FileGps for GpsWriter {
    fn write(&mut self, path: &Path, lat: f64, lon: f64, marker: &str) -> AppResult<WriteOutcome> {
        self.write_gps(path, lat, lon, marker)
    }
    fn remove(&mut self, path: &Path, expected_hash: &str) -> AppResult<UndoOutcome> {
        self.remove_gps(path, expected_hash)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct WriteReport {
    #[ts(type = "number")]
    pub write_id: i64,
    /// Places now in the library.
    pub locations: u32,
    /// Of those, written into the original file too.
    pub files: u32,
    /// *Location saved in Lumen only*: a format Lumen doesn't write, or no permission to.
    pub lumen_only: u32,
    /// The file refused or failed (asset id, why); the location is in Lumen only.
    #[ts(type = "Array<[number, string]>")]
    pub refused: Vec<(i64, String)>,
    /// Gained a location of its own meanwhile, or is gone: not written.
    pub skipped: u32,
    pub cancelled: bool,
    /// The assets whose place went into the library, for the review to forget.
    #[serde(skip)]
    #[ts(skip)]
    pub written: Vec<i64>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct UndoReport {
    pub reverted: u32,
    /// Files whose GPS stayed (changed since, or unreachable): *N files kept their location*.
    #[ts(type = "Array<[number, string]>")]
    pub kept_in_file: Vec<(i64, String)>,
}

fn db_err(e: impl std::fmt::Display) -> AppError {
    AppError { code: "places_write".into(), message: e.to_string(), detail: None }
}

/// *Write N locations*: one `place_writes` row, then per place the library row (only for a photo
/// that has no location yet, named from the atlas, `place_source = 3`) and, with `files` and a
/// format that takes it, the original file (§8.7). `cancel` is looked at between files.
pub fn write_job(
    lib: &Library,
    plugin: &str,
    places: &[(i64, f64, f64)],
    mut files: Option<&mut dyn FileGps>,
    cancel: &AtomicBool,
    mut progress: impl FnMut(usize),
) -> AppResult<WriteReport> {
    let write_id = lib.db.with_conn(|c| {
        c.execute("INSERT INTO place_writes (plugin_id, created_at) VALUES (?1, ?2)", params![plugin, photos_core::now_unix()])?;
        Ok(c.last_insert_rowid())
    })?;
    let mut report = WriteReport { write_id, ..Default::default() };
    let mut named: Vec<i64> = Vec::new();
    for (n, &(id, lat, lon)) in places.iter().enumerate() {
        progress(n);
        if cancel.load(Ordering::Relaxed) {
            report.cancelled = true;
            break;
        }
        let (label, locality) = photos_core::places::reverse_geocode(lat, lon).map(|p| (p.label(), p.label())).unwrap_or((None, None));
        let prev: Option<i64> = lib.db.with_conn(|c| {
            let prev = c.query_row("SELECT place_source FROM assets WHERE id = ?1 AND gps_lat IS NULL AND gps_lon IS NULL AND state = 0", [id], |r| r.get(0)).ok();
            let Some(prev): Option<i64> = prev else { return Ok(None) };
            c.execute(
                "UPDATE assets SET gps_lat = ?2, gps_lon = ?3, place_name = ?4, locality = ?5, place_source = ?6 WHERE id = ?1",
                params![id, lat, lon, label, locality, PLACE_SOURCE_ESTIMATED],
            )?;
            c.execute(
                "INSERT INTO place_write_items (write_id, asset_id, lat, lon, wrote_file, prev_place_source) VALUES (?1, ?2, ?3, ?4, 0, ?5)",
                params![write_id, id, lat, lon, prev.to_string()],
            )?;
            Ok(Some(prev))
        })?;
        if prev.is_none() {
            report.skipped += 1;
            continue;
        }
        report.locations += 1;
        named.push(id);
        let Some(files) = files.as_deref_mut() else {
            report.lumen_only += 1;
            continue;
        };
        let outcome = match lib.original_path(id) {
            Ok(path) => files.write(&path, lat, lon, plugin),
            Err(e) => Err(db_err(e)),
        };
        match outcome {
            Ok(WriteOutcome::Written { file_hash }) => {
                crate::originals_gps::mark_file_written(lib, id, &file_hash)?;
                lib.db.with_conn(|c| Ok(c.execute("UPDATE place_write_items SET wrote_file = 1 WHERE write_id = ?1 AND asset_id = ?2", params![write_id, id])?))?;
                report.files += 1;
            }
            Ok(WriteOutcome::DbOnly) => report.lumen_only += 1,
            Ok(WriteOutcome::Refused(why)) => report.refused.push((id, why)),
            Err(e) => report.refused.push((id, e.message)),
        }
    }
    lib.db.with_conn(|c| photos_core::fts::index_ids(c, &named))?;
    report.written = named;
    Ok(report)
}

/// One undo for a whole write: library rows always revert (when the place is still the one
/// written); a file's GPS is removed only if the file is still the bytes Lumen wrote.
pub fn undo_write(lib: &Library, write_id: i64, mut files: Option<&mut dyn FileGps>) -> AppResult<UndoReport> {
    type Item = (i64, f64, f64, bool, Option<String>);
    let items: Vec<Item> = lib.db.with_conn(|c| {
        let mut s = c.prepare("SELECT asset_id, lat, lon, wrote_file, prev_place_source FROM place_write_items WHERE write_id = ?1 ORDER BY asset_id")?;
        let rows = s.query_map([write_id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get::<_, i64>(3)? != 0, r.get(4)?)))?.collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    })?;
    let mut report = UndoReport::default();
    for (id, lat, lon, wrote_file, prev) in items {
        if wrote_file {
            let hash: Option<String> = lib.db.with_conn(|c| Ok(c.query_row("SELECT COALESCE(file_hash, content_hash) FROM assets WHERE id = ?1", [id], |r| r.get(0)).ok()))?;
            let outcome = match (files.as_deref_mut(), hash, lib.original_path(id)) {
                (Some(f), Some(h), Ok(path)) => f.remove(&path, &h),
                (None, ..) => Ok(UndoOutcome::Skipped("no_writer".into())),
                _ => Ok(UndoOutcome::Skipped("unreachable".into())),
            };
            match outcome {
                Ok(UndoOutcome::Removed { file_hash }) => crate::originals_gps::mark_file_written(lib, id, &file_hash)?,
                Ok(UndoOutcome::Skipped(why)) => report.kept_in_file.push((id, why)),
                Err(e) => report.kept_in_file.push((id, e.message)),
            }
        }
        // Back to what it was, unless the person has since put the photo somewhere else.
        let back = prev.and_then(|p| p.parse::<i64>().ok()).unwrap_or(0);
        let n = lib.db.with_conn(|c| {
            Ok(c.execute(
                "UPDATE assets SET gps_lat = NULL, gps_lon = NULL, place_name = NULL, locality = NULL, place_source = ?2
                 WHERE id = ?1 AND place_source = ?3 AND abs(gps_lat - ?4) < 1e-9 AND abs(gps_lon - ?5) < 1e-9",
                params![id, back, PLACE_SOURCE_ESTIMATED, lat, lon],
            )?)
        })?;
        report.reverted += n as u32;
    }
    lib.db.with_conn(|c| {
        let ids: Vec<i64> = c.prepare("SELECT asset_id FROM place_write_items WHERE write_id = ?1")?.query_map([write_id], |r| r.get(0))?.collect::<Result<_, _>>()?;
        photos_core::fts::index_ids(c, &ids)?;
        c.execute("DELETE FROM place_writes WHERE id = ?1", [write_id])?;
        Ok(())
    })?;
    Ok(report)
}

/// The places the review would write now (for the *Write N* button's count and the job).
pub fn pending_writes(store: &ReviewStore, plugin: &str) -> Vec<(i64, f64, f64)> {
    store.to_write(plugin)
}

/// After a write, the places that went in leave the review.
pub fn forget_written(store: &ReviewStore, plugin: &str, places: &[(i64, f64, f64)]) {
    store.written(plugin, &places.iter().map(|p| p.0).collect::<Vec<_>>());
}

#[cfg(test)]
mod tests {
    use super::*;
    use photos_plugins::host::Candidate;

    fn sug(asset: u64, group: &str, state: PlaceState, flag: Option<PlaceFlag>, lat: f64, lon: f64) -> Suggestion {
        Suggestion {
            asset,
            candidates: vec![Candidate { lat, lon, confidence: 0.9, label: None }],
            state,
            group: group.into(),
            group_label: format!("{group} label"),
            flag,
        }
    }

    #[test]
    fn suggest_then_event_round_trip() {
        let store = ReviewStore::default();
        let host = AppPlaces { store: Arc::new(ReviewStore::default()) };
        host.suggest("jl.t", vec![sug(1, "g1", PlaceState::Anchor, None, 46.0, 14.0)]).unwrap();
        // The host stores per plugin; use the store directly for the rest.
        store.suggest("jl.t", &[
            sug(1, "g1", PlaceState::Anchor, None, 46.0, 14.0),
            sug(2, "g1", PlaceState::Guess, None, 46.1, 14.1),
            sug(3, "g2", PlaceState::Inferred, Some(PlaceFlag::FarFromGroup), 10.0, 10.0),
        ]);
        let s = store.state("jl.t");
        assert_eq!((s.counts.check, s.counts.moments, s.counts.done), (2, 2, 1));
        assert_eq!(store.to_write("jl.t"), vec![(1, 46.0, 14.0)]);

        // Confirm 2 → done; the plugin propagates to photo 3 (a fake `on-review`).
        let mut seen = None;
        let s = store
            .event("jl.t", &ReviewAction::Confirm { assets: vec![2] }, |e| {
                seen = Some(format!("{e:?}"));
                Ok(vec![sug(3, "g1", PlaceState::Inferred, None, 46.1, 14.1)])
            })
            .unwrap();
        assert!(seen.unwrap().contains("Confirm"));
        assert_eq!((s.counts.check, s.counts.done, s.counts.moments), (0, 3, 1));
        assert_eq!(s.items.iter().find(|i| i.asset == 3).unwrap().group, "g1");

        // Reject 3 → out of the review and the write; Move 1 → confirmed at the new pin.
        let s = store.event("jl.t", &ReviewAction::Reject { assets: vec![3] }, |_| Ok(vec![])).unwrap();
        assert_eq!((s.counts.rejected, s.counts.done), (1, 2));
        let s = store.event("jl.t", &ReviewAction::Move { assets: vec![1], lat: 1.0, lon: 2.0 }, |_| Ok(vec![])).unwrap();
        assert_eq!(s.items[0].candidates[0].lat, 1.0);
        assert_eq!(store.to_write("jl.t"), vec![(1, 1.0, 2.0), (2, 46.1, 14.1)]);

        // A plugin that fails keeps the host's side of the action but reports the error.
        assert!(store.event("jl.t", &ReviewAction::Split { group: "g1".into(), at_asset: 2 }, |_| Err("boom".into())).is_err());
        // The atlas answers `lookup`.
        assert!(!host.lookup("Ljubljana").is_empty());
    }

    struct Fake {
        mode: &'static str,
        removed: Vec<i64>,
    }

    impl FileGps for Fake {
        fn write(&mut self, path: &Path, _: f64, _: f64, _: &str) -> AppResult<WriteOutcome> {
            let name = path.file_name().unwrap().to_string_lossy().to_string();
            Ok(if name.ends_with(".mov") {
                WriteOutcome::DbOnly
            } else if self.mode == "refuse" {
                WriteOutcome::Refused("has_gps: EXIF".into())
            } else {
                WriteOutcome::Written { file_hash: format!("hash-of-{name}") }
            })
        }
        fn remove(&mut self, path: &Path, expected: &str) -> AppResult<UndoOutcome> {
            let name = path.file_name().unwrap().to_string_lossy().to_string();
            if expected != format!("hash-of-{name}") {
                return Ok(UndoOutcome::Skipped("changed".into()));
            }
            self.removed.push(1);
            Ok(UndoOutcome::Removed { file_hash: format!("content-{name}") })
        }
    }

    fn library() -> (tempfile::TempDir, Library) {
        let dir = tempfile::tempdir().unwrap();
        let lib = Library::create(dir.path()).unwrap();
        lib.db
            .with_conn(|c| {
                for (id, file, lat) in [(1, "a.jpg", None), (2, "b.mov", None), (3, "c.jpg", None), (4, "d.jpg", Some(1.5f64))] {
                    c.execute(
                        "INSERT INTO assets (id, content_hash, relative_path, filename, extension, media_type, byte_size, imported_at, gps_lat, gps_lon)
                         VALUES (?1, ?2, ?3, ?3, 'jpg', 0, 1, 1, ?4, ?4)",
                        params![id, format!("content-{file}"), file, lat],
                    )?;
                }
                Ok(())
            })
            .unwrap();
        (dir, lib)
    }

    fn row(lib: &Library, id: i64) -> (Option<f64>, Option<String>, i64, Option<String>) {
        lib.db
            .with_conn(|c| Ok(c.query_row("SELECT gps_lat, place_name, place_source, file_hash FROM assets WHERE id = ?1", [id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?))
            .unwrap()
    }

    #[test]
    fn write_job_db_and_files() {
        let (_d, lib) = library();
        let mut fake = Fake { mode: "ok", removed: vec![] };
        let places = [(1, 46.05, 14.5), (2, 46.05, 14.5), (4, 10.0, 10.0), (3, 46.05, 14.5)];
        let r = write_job(&lib, "jl.wayfinder", &places, Some(&mut fake as &mut dyn FileGps), &AtomicBool::new(false), |_| {}).unwrap();
        // 1 and 3 are JPEGs written, 2 (a movie) is Lumen-only, 4 already had a location.
        assert_eq!((r.locations, r.files, r.lumen_only, r.skipped), (3, 2, 1, 1));
        let (lat, name, src, hash) = row(&lib, 1);
        assert_eq!((lat, src, hash.as_deref()), (Some(46.05), PLACE_SOURCE_ESTIMATED, Some("hash-of-a.jpg")));
        assert!(name.is_some(), "named from the atlas");
        assert_eq!(row(&lib, 2).2, PLACE_SOURCE_ESTIMATED);
        assert_eq!(row(&lib, 4).0, Some(1.5), "a photo with its own location is left alone");

        // Without the permission everything is Lumen-only; a refusing file stays DB-only with a reason.
        let (_d2, lib2) = library();
        let r = write_job(&lib2, "jl.wayfinder", &[(1, 1.0, 2.0)], None, &AtomicBool::new(false), |_| {}).unwrap();
        assert_eq!((r.locations, r.files, r.lumen_only), (1, 0, 1));
        let (_d3, lib3) = library();
        let mut refuse = Fake { mode: "refuse", removed: vec![] };
        let r = write_job(&lib3, "jl.wayfinder", &[(1, 1.0, 2.0)], Some(&mut refuse as &mut dyn FileGps), &AtomicBool::new(false), |_| {}).unwrap();
        assert_eq!((r.locations, r.files, r.refused.len()), (1, 0, 1));
        assert_eq!(row(&lib3, 1).0, Some(1.0));

        // Cancelled before the second file: the first stays, the second is never touched.
        let (_d4, lib4) = library();
        let cancel = AtomicBool::new(false);
        let r = write_job(&lib4, "jl.wayfinder", &[(1, 1.0, 2.0), (3, 1.0, 2.0)], None, &cancel, |n| if n == 1 { cancel.store(true, Ordering::Relaxed) }).unwrap();
        assert!(r.cancelled && r.locations == 1);
        assert_eq!(row(&lib4, 3).0, None);
    }

    #[test]
    fn undo_whole_write() {
        let (_d, lib) = library();
        let mut fake = Fake { mode: "ok", removed: vec![] };
        let r = write_job(&lib, "jl.wayfinder", &[(1, 46.05, 14.5), (2, 46.05, 14.5), (3, 46.05, 14.5)], Some(&mut fake as &mut dyn FileGps), &AtomicBool::new(false), |_| {}).unwrap();
        // File 3 changes after the write (not the bytes Lumen wrote).
        lib.db.with_conn(|c| Ok(c.execute("UPDATE assets SET file_hash = 'someone-edited-it' WHERE id = 3", [])?)).unwrap();
        let u = undo_write(&lib, r.write_id, Some(&mut fake as &mut dyn FileGps)).unwrap();
        // Library rows always revert; the changed file keeps its GPS and is listed.
        assert_eq!(u.reverted, 3);
        assert_eq!(u.kept_in_file, vec![(3, "changed".to_string())]);
        for id in [1, 2, 3] {
            let (lat, name, src, _) = row(&lib, id);
            assert_eq!((lat, name, src), (None, None, 0), "asset {id}");
        }
        assert_eq!(row(&lib, 1).3, None, "file_hash back to the import's");
        let left: i64 = lib.db.with_conn(|c| Ok(c.query_row("SELECT count(*) FROM place_writes", [], |r| r.get(0))?)).unwrap();
        assert_eq!(left, 0);
    }
}
