//! *Import → Repair* (Round 21 §6): a source is judged against the library so the person can bring
//! back what is missing or damaged.
//!
//! [`check_source`] classifies every media file the source lists:
//!
//! 1. **Identity** (no file is read): same file name (any case), same byte size and, when the
//!    asset remembers one, same source folder. An intact library original is `safe`; a flagged,
//!    missing or resized one is `damaged`; a trashed one is `in_trash`. A file with the same name
//!    and folder but another size is `damaged` when the library's copy is flagged unreadable.
//! 2. **Bytes**: only a file with no identity match is hashed (BLAKE3, as an import does) and looked
//!    up by `assets.content_hash`: in the library → `safe`, in Trash → `in_trash`; bytes a repair
//!    replaced (`replaced_originals`) → `safe`; else `new`.
//! 3. **Probing** (Round 22 §3): a `damaged` candidate's source copy is read once: when its bytes
//!    are the library copy's own, or it does not decode (the `probe` the caller passes), it is
//!    `source_damaged`: damaged on the source too, never offered for re-import.
//! 4. **Sidecars**: for a `safe`, `damaged` or `source_damaged` photo, each `.xmp` / `.aae` / `.thm` the source pairs
//!    with it (the import's own pairing rules) that `asset_sidecars` has no row of that kind for
//!    becomes one `sidecar` item. Told apart by name and kind: nothing is read.
//!
//! The check writes nothing, to the library or to the source. With `only_assets` it looks only at
//! files that identity-match those photos (the *Restore…* flow of §7) and hashes nothing but its
//! damaged candidates.
//!
//! [`apply`] brings in what the person ticked: `new` files through the normal import (one batch,
//! *Repair · <source>*), `sidecar` files through the import's sidecar copy, and a `damaged`
//! original by a **swap** that keeps the asset (id, albums, ratings, edits) and replaces only its
//! file, only when the copy differs from the damaged bytes and decodes. The damaged bytes go to
//! `<library>/kept-aside/<date>/`, never deleted, and are remembered in `replaced_originals`; any
//! stop leaves the library as it was. Every chosen item ends with one [`RepairOutcome`].

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use photos_core::{assets, batches, now_unix, sidecars};
use photos_core::Library;
use photos_jobs::CancelToken;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::source::{ImportSource, SourceItem};
use crate::ImportError;

/// Hashing is disk-bound; a few readers keep a card or a phone busy without starving the app.
const THREADS: usize = 4;
/// Progress is reported every this many identity-matched files (every hashed file reports).
const REPORT_EVERY: u64 = 64;

/// What the check made of one file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum RepairClass {
    /// In the library and intact. Counted, never listed.
    Safe,
    New,
    Damaged,
    /// The library's copy is damaged and the source's is the same bytes or does not decode
    /// either: *Damaged on <source> too*. Never ticked for re-import.
    SourceDamaged,
    Sidecar,
    InTrash,
}

/// Says whether the file at `path` decodes; `file_name` is its real name (the path may end `.part`).
/// `Err` carries the decoder's reason.
pub type Probe<'a> = &'a (dyn Fn(&Path, &str) -> Result<(), String> + Sync);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum RepairSourceKind {
    Folder,
    Volume,
    Iphone,
    MacDisk,
}

/// Names a source the Repair screen offered, or a folder the person chose.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RepairSourceRef {
    pub kind: RepairSourceKind,
    /// A folder's path, a volume's mount point, an iPhone's UDID, a Mac disk's device path.
    pub id: String,
}

/// One row of the source list (step 1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RepairSource {
    #[serde(rename = "ref")]
    #[ts(rename = "ref")]
    pub source_ref: RepairSourceRef,
    pub name: String,
    pub detail: String,
    /// `false` for an iPhone that is not paired yet.
    pub ready: bool,
    pub removable: bool,
}

/// A file that needs repair. `id` is its place in the result (1-based) and keys paging and
/// selection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RepairItem {
    #[ts(type = "number")]
    pub id: i64,
    pub name: String,
    /// The folder on the source, `/`-separated; empty at the root.
    pub dir: String,
    pub status: RepairClass,
    /// The library photo this concerns: the damaged or trashed one, or the one a sidecar belongs to.
    #[ts(type = "number | null")]
    pub asset_id: Option<i64>,
    #[ts(type = "number")]
    pub byte_size: u64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RepairCounts {
    /// Media files in the library and intact (not listed).
    pub safe: u32,
    pub new: u32,
    pub damaged: u32,
    pub source_damaged: u32,
    pub sidecar: u32,
    pub in_trash: u32,
}

impl RepairCounts {
    fn slot(&mut self, class: RepairClass) -> &mut u32 {
        match class {
            RepairClass::Safe => &mut self.safe,
            RepairClass::New => &mut self.new,
            RepairClass::Damaged => &mut self.damaged,
            RepairClass::SourceDamaged => &mut self.source_damaged,
            RepairClass::Sidecar => &mut self.sidecar,
            RepairClass::InTrash => &mut self.in_trash,
        }
    }

    fn add(&mut self, class: RepairClass) {
        *self.slot(class) += 1;
    }

    /// A file the probing pass moved from `from` to `to`.
    fn moved(&mut self, from: RepairClass, to: RepairClass) {
        let slot = self.slot(from);
        *slot = slot.saturating_sub(1);
        self.add(to);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum RepairPhase {
    Idle,
    Listing,
    Checking,
    /// Reading the damaged candidates' source copies (§3 rule 1).
    Probing,
    Copying,
    Verifying,
    Done,
}

/// Progress of a check, as the job reports it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RepairStatus {
    pub phase: RepairPhase,
    #[ts(type = "number")]
    pub done: u64,
    #[ts(type = "number")]
    pub total: u64,
    pub counts: RepairCounts,
    pub cancelled: bool,
    /// The file being looked at, when one is.
    pub current: Option<String>,
    /// Bytes of the current file copied so far, for a large file (0 otherwise).
    #[ts(type = "number")]
    pub bytes_done: u64,
    #[ts(type = "number")]
    pub bytes_total: u64,
}

/// The result of a check: what needs repair, and how many files are fine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepairCheck {
    pub items: Vec<RepairItem>,
    pub counts: RepairCounts,
    /// Stopped before every file was looked at: not a verdict.
    pub cancelled: bool,
}

/// What the catalogue says about one photo, for matching by identity.
struct AssetRow {
    id: i64,
    byte_size: i64,
    source_dir: Option<String>,
    relative_path: String,
    trashed: bool,
    unreadable: bool,
    /// Set once a metadata write changed the file (Round 24 §8.7): its size then differs from
    /// `byte_size` (the camera's), so the file is read and compared with this instead.
    file_hash: Option<String>,
}

fn dir_of(rel_path: &str) -> &str {
    rel_path.rsplit_once('/').map_or("", |(d, _)| d)
}

fn status(phase: RepairPhase, done: u64, total: u64, counts: RepairCounts, cancelled: bool) -> RepairStatus {
    RepairStatus { phase, done, total, counts, cancelled, current: None, bytes_done: 0, bytes_total: 0 }
}

/// BLAKE3 of the file at `path`, as an import computes it; `None` when it cannot be read.
fn hash_file(path: &Path) -> Option<String> {
    let mut file = std::fs::File::open(path).ok()?;
    let mut hasher = blake3::Hasher::new();
    hasher.update_reader(&mut file).ok()?;
    Some(hasher.finalize().to_hex().to_string())
}

/// Copies the source's `item` to `tmp`, hashing it, and says whether it can stand in for a damaged
/// original whose own bytes hash to `library_hash`: not the same bytes, and it decodes. `tmp` is
/// removed in every case.
fn probe_source(source: &dyn ImportSource, item: &SourceItem, tmp: &Path, library_hash: Option<&str>, probe: Probe) -> Result<(), String> {
    let staged = source.open(item).and_then(|reader| crate::ingest::hash_and_stage(reader, tmp));
    let verdict = match staged {
        Err(e) => Err(e.to_string()),
        Ok((hash, _)) if Some(hash.as_str()) == library_hash => Err("the same file as the library's".into()),
        Ok(_) => probe(tmp, &item.file_name),
    };
    let _ = std::fs::remove_file(tmp);
    verdict
}

type AssetIndex = HashMap<String, Vec<AssetRow>>;

/// Every asset by lower-case file name (only `only` when given), plus the sidecar kinds kept.
fn load_index(lib: &Library, only: Option<&[i64]>) -> Result<(AssetIndex, HashSet<(i64, String)>), ImportError> {
    let only: Option<HashSet<i64>> = only.map(|ids| ids.iter().copied().collect());
    let rows = lib.db.with_conn(|c| {
        let mut st = c.prepare("SELECT id, filename, byte_size, source_dir, relative_path, state, unreadable, file_hash FROM assets")?;
        let rows = st
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(1)?,
                    AssetRow {
                        id: r.get(0)?,
                        byte_size: r.get(2)?,
                        source_dir: r.get(3)?,
                        relative_path: r.get(4)?,
                        trashed: r.get::<_, i64>(5)? == 1,
                        unreadable: r.get::<_, i64>(6)? != 0,
                        file_hash: r.get(7)?,
                    },
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    })?;
    let mut by_name = AssetIndex::new();
    for (name, row) in rows {
        if only.as_ref().is_some_and(|o| !o.contains(&row.id)) {
            continue;
        }
        by_name.entry(name.to_lowercase()).or_default().push(row);
    }
    let kept = lib.db.with_conn(|c| {
        let mut st = c.prepare("SELECT asset_id, kind FROM asset_sidecars")?;
        let rows = st.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))?.collect::<Result<HashSet<_>, _>>()?;
        Ok(rows)
    })?;
    Ok((by_name, kept))
}

/// Whether the library's original is on disk at the catalogued size. A stat, not a read, except
/// for a file a metadata write changed: that one is read and compared with its `file_hash`.
fn original_intact(lib: &Library, a: &AssetRow) -> bool {
    if let Some(expected) = &a.file_hash {
        return hash_file(&lib.originals_dir().join(&a.relative_path)).as_ref() == Some(expected);
    }
    std::fs::metadata(lib.originals_dir().join(&a.relative_path)).is_ok_and(|m| m.is_file() && i64::try_from(m.len()) == Ok(a.byte_size))
}

/// Step 1: what name, size and folder settle without reading. `None` = identity does not settle it.
fn by_identity(lib: &Library, index: &AssetIndex, item: &SourceItem) -> Option<(RepairClass, i64)> {
    let candidates = index.get(&item.file_name.to_lowercase())?;
    let dir = dir_of(&item.rel_path);
    let same_dir = |a: &&AssetRow| a.source_dir.as_deref().is_none_or(|d| d == dir);
    let size = i64::try_from(item.size).ok();
    // Prefer a candidate that remembers this very folder over one that remembers none.
    let found = candidates
        .iter()
        .filter(same_dir)
        .filter(|a| Some(a.byte_size) == size)
        .max_by_key(|a| a.source_dir.is_some());
    if let Some(a) = found {
        let class = if a.trashed {
            RepairClass::InTrash
        } else if a.unreadable || !original_intact(lib, a) {
            RepairClass::Damaged
        } else {
            RepairClass::Safe
        };
        return Some((class, a.id));
    }
    // Same name and folder, another size: the source may be the good copy of a flagged original.
    candidates
        .iter()
        .find(|a| !a.trashed && a.unreadable && a.source_dir.as_deref() == Some(dir))
        .map(|a| (RepairClass::Damaged, a.id))
}

/// Step 2: hashes `item` and looks the bytes up.
fn by_bytes(lib: &Library, source: &dyn ImportSource, item: &SourceItem) -> (RepairClass, Option<i64>) {
    let hash = source.open(item).ok().and_then(|mut reader| {
        let mut hasher = blake3::Hasher::new();
        hasher.update_reader(&mut reader).ok()?;
        Some(hasher.finalize().to_hex().to_string())
    });
    // A file that cannot be read is not in the library as far as anyone can tell.
    let Some(hash) = hash else { return (RepairClass::New, None) };
    let found: Option<(i64, bool)> = lib
        .db
        .with_conn(|c| {
            use rusqlite::OptionalExtension;
            Ok(c.query_row("SELECT id, state FROM assets WHERE content_hash = ?1", [&hash], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)? == 1))).optional()?)
        })
        .unwrap_or(None);
    match found {
        Some((id, false)) => (RepairClass::Safe, Some(id)),
        Some((id, true)) => (RepairClass::InTrash, Some(id)),
        // Bytes a repair set aside: the library already has this photo's good copy.
        None => match replaced_by(lib, &hash) {
            Some(id) => (RepairClass::Safe, Some(id)),
            None => (RepairClass::New, None),
        },
    }
}

/// The photo whose damaged bytes `hash` a repair replaced, if any.
fn replaced_by(lib: &Library, hash: &str) -> Option<i64> {
    lib.db
        .with_conn(|c| {
            use rusqlite::OptionalExtension;
            Ok(c.query_row("SELECT asset_id FROM replaced_originals WHERE content_hash = ?1", [hash], |r| r.get(0)).optional()?)
        })
        .unwrap_or(None)
}

/// Judges every media file `source` lists against `lib`. Reads a file only at step 2.
pub fn check_source(
    source: &dyn ImportSource,
    lib: &Library,
    only_assets: Option<&[i64]>,
    probe: Probe,
    progress: &(dyn Fn(RepairStatus) + Sync),
    cancel: &CancelToken,
) -> Result<RepairCheck, ImportError> {
    progress(status(RepairPhase::Listing, 0, 0, RepairCounts::default(), false));
    let mut listed = source.enumerate()?;
    listed.sort_by(|a, b| a.rel_path.cmp(&b.rel_path));
    let total = listed.len() as u64;
    let (index, kept) = load_index(lib, only_assets)?;
    let hashing_allowed = only_assets.is_none();

    // Pass 1: identity. `settled[i]` is the class and asset of file `i`, `None` while undecided.
    let mut settled: Vec<Option<(RepairClass, Option<i64>)>> = vec![None; listed.len()];
    let mut counts = RepairCounts::default();
    let mut to_hash: Vec<usize> = Vec::new();
    let mut done = 0u64;
    for (i, item) in listed.iter().enumerate() {
        if cancel.is_cancelled() {
            break;
        }
        match by_identity(lib, &index, item) {
            Some((class, asset)) => {
                settled[i] = Some((class, Some(asset)));
                counts.add(class);
                done += 1;
            }
            None if hashing_allowed => {
                to_hash.push(i);
                continue;
            }
            None => done += 1, // outside the photos asked for: not looked at
        }
        if done.is_multiple_of(REPORT_EVERY) {
            progress(status(RepairPhase::Checking, done, total, counts, false));
        }
    }
    let mut cancelled = cancel.is_cancelled();

    // Pass 2: the files identity could not place, hashed on a few threads.
    if !cancelled && !to_hash.is_empty() {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(THREADS)
            .build()
            .map_err(|e| ImportError::Source(format!("could not start the check: {e}")))?;
        let running = Mutex::new((done, counts));
        let hashed: Vec<Option<(usize, RepairClass, Option<i64>)>> = pool.install(|| {
            to_hash
                .par_iter()
                .map(|&i| {
                    if cancel.is_cancelled() {
                        return None;
                    }
                    let (class, asset) = by_bytes(lib, source, &listed[i]);
                    let mut guard = running.lock().unwrap_or_else(|p| p.into_inner());
                    guard.0 += 1;
                    guard.1.add(class);
                    progress(RepairStatus { current: Some(listed[i].file_name.clone()), ..status(RepairPhase::Checking, guard.0, total, guard.1, false) });
                    Some((i, class, asset))
                })
                .collect()
        });
        (done, counts) = running.into_inner().unwrap_or_else(|p| p.into_inner());
        for (i, class, asset) in hashed.into_iter().flatten() {
            settled[i] = Some((class, asset));
        }
        cancelled = cancel.is_cancelled();
    }

    // Pass 2b: a damaged candidate's source copy must differ from the library's bytes and decode
    // (§3 rule 1). Few files: only these are read, whatever `only_assets` says.
    let to_probe: Vec<usize> = settled.iter().enumerate().filter_map(|(i, s)| matches!(s, Some((RepairClass::Damaged, Some(_)))).then_some(i)).collect();
    if !cancelled && !to_probe.is_empty() {
        let by_id: HashMap<i64, &AssetRow> = index.values().flatten().map(|a| (a.id, a)).collect();
        let staging = tempfile::Builder::new()
            .prefix("lumen-repair-")
            .tempdir_in(std::env::temp_dir())
            .map_err(|e| ImportError::Source(format!("could not start the check: {e}")))?;
        let of = to_probe.len() as u64;
        for (n, &i) in to_probe.iter().enumerate() {
            if cancel.is_cancelled() {
                cancelled = true;
                break;
            }
            let item = &listed[i];
            let Some((_, Some(asset))) = settled[i] else { continue };
            progress(RepairStatus { current: Some(item.file_name.clone()), ..status(RepairPhase::Probing, n as u64, of, counts, false) });
            let original = by_id.get(&asset).map(|a| lib.originals_dir().join(&a.relative_path));
            let library_hash = original.as_deref().and_then(hash_file);
            let tmp = staging.path().join(format!("{n}-{}", item.file_name));
            if let Err(why) = probe_source(source, item, &tmp, library_hash.as_deref(), probe) {
                tracing::info!(file = %item.rel_path, "damaged on the source too: {why}");
                settled[i] = Some((RepairClass::SourceDamaged, Some(asset)));
                counts.moved(RepairClass::Damaged, RepairClass::SourceDamaged);
            }
        }
    }

    // Pass 3: what is listed, in the source's order, a photo's missing sidecars right after it.
    let mut items: Vec<RepairItem> = Vec::new();
    for (item, outcome) in listed.iter().zip(&settled) {
        let Some((class, asset_id)) = *outcome else { continue };
        let dir = dir_of(&item.rel_path).to_string();
        if matches!(class, RepairClass::New | RepairClass::Damaged | RepairClass::SourceDamaged | RepairClass::InTrash) {
            items.push(RepairItem { id: 0, name: item.file_name.clone(), dir: dir.clone(), status: class, asset_id, byte_size: item.size });
        }
        if let (RepairClass::Safe | RepairClass::Damaged | RepairClass::SourceDamaged, Some(asset)) = (class, asset_id) {
            for side in source.sidecars(item) {
                let kind = side.file_name.rsplit_once('.').and_then(|(_, ext)| sidecars::kind_for_extension(ext));
                let Some(kind) = kind else { continue };
                if kept.contains(&(asset, kind.to_string())) {
                    continue;
                }
                counts.add(RepairClass::Sidecar);
                items.push(RepairItem {
                    id: 0,
                    name: side.file_name.clone(),
                    dir: dir_of(&side.rel_path).to_string(),
                    status: RepairClass::Sidecar,
                    asset_id: Some(asset),
                    byte_size: side.size,
                });
            }
        }
    }
    for (n, item) in items.iter_mut().enumerate() {
        item.id = n as i64 + 1;
    }
    progress(status(RepairPhase::Done, done, total, counts, cancelled));
    Ok(RepairCheck { items, counts, cancelled })
}

// ---- Re-import (§6 *Re-import*) ---------------------------------------------------------------

/// What the Repair list has ticked: the rows of a filter but a few, or rows by id
/// (`src/lib/repair-selection.ts`).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(untagged)]
pub enum RepairSelection {
    All { all: bool, status: RepairClass, except: Vec<i64> },
    Ids { ids: Vec<i64> },
}

/// The ticked items of a check, in its order. Only `new`, `damaged` and `sidecar` rows can be
/// taken: a trashed photo is put back from Trash, not imported again, and a `source_damaged` one
/// has no good copy to bring.
pub fn select<'a>(items: &'a [RepairItem], selection: &RepairSelection) -> Vec<&'a RepairItem> {
    let takeable = |i: &&RepairItem| matches!(i.status, RepairClass::New | RepairClass::Damaged | RepairClass::Sidecar);
    match selection {
        RepairSelection::All { all, status, except } => {
            items.iter().filter(takeable).filter(|i| *all && i.status == *status && !except.contains(&i.id)).collect()
        }
        RepairSelection::Ids { ids } => {
            let ids: HashSet<i64> = ids.iter().copied().collect();
            items.iter().filter(takeable).filter(|i| ids.contains(&i.id)).collect()
        }
    }
}

/// What a re-import did: *n imported · n repaired · n sidecars · n skipped*.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RepairResult {
    pub imported: u32,
    pub repaired: u32,
    pub sidecars: u32,
    pub skipped: u32,
    /// The *Repair · <source>* batch, when there were new files.
    #[ts(type = "number | null")]
    pub batch_id: Option<i64>,
    /// The photos whose file was replaced: their derivatives are made again.
    #[ts(type = "number[]")]
    pub repaired_ids: Vec<i64>,
    /// One line per skipped file: its name and why.
    pub notes: Vec<String>,
    pub cancelled: bool,
    /// One row per chosen item, in the order they were done.
    pub outcomes: Vec<RepairOutcomeRow>,
}

/// How one chosen item of a run ended (§3 rule 6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum RepairOutcome {
    Repaired,
    Imported,
    Sidecar,
    /// The source's copy does not decode either.
    SourceDamaged,
    /// The source's copy is the library's damaged bytes.
    SameFile,
    /// The source's copy is another photo already in the library.
    Duplicate,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RepairOutcomeRow {
    pub name: String,
    pub dir: String,
    pub outcome: RepairOutcome,
    pub reason: Option<String>,
    #[ts(type = "number | null")]
    pub asset_id: Option<i64>,
}

fn outcome_row(item: &RepairItem, outcome: RepairOutcome, reason: Option<String>, asset_id: Option<i64>) -> RepairOutcomeRow {
    RepairOutcomeRow { name: item.name.clone(), dir: item.dir.clone(), outcome, reason, asset_id }
}

/// A file over this many bytes reports its bytes as it copies (§3 rule 5).
const LARGE_FILE: u64 = 100 * 1024 * 1024;

/// The source path of an item: its folder and name.
fn rel_of(item: &RepairItem) -> String {
    if item.dir.is_empty() { item.name.clone() } else { format!("{}/{}", item.dir, item.name) }
}

/// The new files of `inner` only, under the repair's own label. Lists no leftovers: the
/// person picked files, not the source.
struct OnlyThese<'a> {
    inner: &'a dyn ImportSource,
    allow: HashSet<String>,
    label: String,
}

impl ImportSource for OnlyThese<'_> {
    fn label(&self) -> String {
        self.label.clone()
    }
    fn kind(&self) -> i64 {
        self.inner.kind()
    }
    fn enumerate(&self) -> Result<Vec<SourceItem>, ImportError> {
        Ok(self.inner.enumerate()?.into_iter().filter(|i| self.allow.contains(&i.rel_path)).collect())
    }
    fn open(&self, item: &SourceItem) -> Result<Box<dyn std::io::Read + Send>, ImportError> {
        self.inner.open(item)
    }
    fn extras(&self, item: &SourceItem) -> Option<photos_core::extras::ItemExtras> {
        self.inner.extras(item)
    }
    fn is_connected(&self) -> bool {
        self.inner.is_connected()
    }
    fn sidecars(&self, item: &SourceItem) -> Vec<SourceItem> {
        self.inner.sidecars(item)
    }
}

/// Brings in the `chosen` items of a check of `source` (named `source_name` in the history).
/// `new` → one import batch; `sidecar` → the import's sidecar copy; `damaged` → [`swap`].
#[allow(clippy::too_many_arguments)]
pub fn apply(
    source: &dyn ImportSource,
    lib: &Library,
    source_name: &str,
    chosen: &[RepairItem],
    opts: &crate::ImportOptions,
    probe: Probe,
    cancel: &CancelToken,
    on_progress: &mut dyn FnMut(&crate::ImportProgress),
    on_ingested: &mut dyn FnMut(&crate::Ingested),
    on_status: &mut dyn FnMut(&RepairStatus),
) -> Result<RepairResult, ImportError> {
    let mut out = RepairResult::default();
    let total = chosen.len() as u64;
    let status = |phase: RepairPhase, done: u64, current: Option<String>, bytes_done: u64, bytes_total: u64| RepairStatus {
        phase,
        done,
        total,
        counts: RepairCounts::default(),
        cancelled: false,
        current,
        bytes_done,
        bytes_total,
    };
    let listed = source.enumerate()?;
    let by_rel: HashMap<&str, &SourceItem> = listed.iter().map(|i| (i.rel_path.as_str(), i)).collect();
    let skip = |out: &mut RepairResult, item: &RepairItem, outcome: RepairOutcome, why: &str| {
        tracing::warn!(file = %rel_of(item), "repair skipped: {why}");
        out.skipped += 1;
        out.notes.push(format!("{}: {why}", item.name));
        out.outcomes.push(outcome_row(item, outcome, Some(why.to_string()), item.asset_id));
    };

    // 1. New files: the normal import (copy, verify, sidecars), one batch.
    let new: HashSet<String> = chosen.iter().filter(|i| i.status == RepairClass::New).map(rel_of).collect();
    if !new.is_empty() && !cancel.is_cancelled() {
        let only = OnlyThese { inner: source, allow: new, label: format!("Repair · {source_name}") };
        let batch_id = lib.db.with_conn(|c| batches::start_batch(c, only.kind(), &only.label, None, now_unix()))?;
        let summary = crate::run_batch(
            lib,
            &only,
            batch_id,
            opts,
            cancel,
            &mut |p| {
                on_progress(p);
                on_status(&status(RepairPhase::Copying, p.done, p.current.clone(), 0, 0));
            },
            on_ingested,
        )?;
        out.batch_id = Some(batch_id);
        out.imported += summary.imported as u32;
        out.skipped += (summary.skipped + summary.failed) as u32;
        out.cancelled |= summary.cancelled;
        for item in chosen.iter().filter(|i| i.status == RepairClass::New) {
            let asset: Option<i64> = lib.db.with_conn(|c| {
                use rusqlite::OptionalExtension;
                Ok(c
                    .query_row(
                        "SELECT id FROM assets WHERE import_batch_id = ?1 AND filename = ?2 COLLATE NOCASE AND (source_dir IS NULL OR source_dir = ?3)",
                        rusqlite::params![batch_id, item.name, item.dir],
                        |r| r.get(0),
                    )
                    .optional()?)
            })?;
            out.outcomes.push(match asset {
                Some(id) => outcome_row(item, RepairOutcome::Imported, None, Some(id)),
                None => {
                    let why = if summary.cancelled { "cancelled" } else { "it was not imported" };
                    outcome_row(item, RepairOutcome::Failed, Some(why.into()), None)
                }
            });
        }
    }

    // 2. Sidecars the catalogue lacks, beside their photo's original.
    for item in chosen.iter().filter(|i| i.status == RepairClass::Sidecar) {
        if cancel.is_cancelled() {
            out.cancelled = true;
            break;
        }
        let Some(asset) = item.asset_id else { continue };
        on_status(&status(RepairPhase::Copying, out.outcomes.len() as u64, Some(item.name.clone()), 0, 0));
        let Some(side) = find_sidecar(source, &listed, item) else {
            skip(&mut out, item, RepairOutcome::Failed, "no longer on the source");
            continue;
        };
        let original_rel: Option<String> = lib
            .db
            .with_conn(|c| {
                use rusqlite::OptionalExtension;
                Ok(c.query_row("SELECT relative_path FROM assets WHERE id = ?1", [asset], |r| r.get(0)).optional()?)
            })?;
        let Some(original_rel) = original_rel else {
            skip(&mut out, item, RepairOutcome::Failed, "its photo is no longer in the library");
            continue;
        };
        match crate::sidecar::bring_one(lib, source, asset, &side, &original_rel) {
            Ok(true) => {
                out.sidecars += 1;
                out.outcomes.push(outcome_row(item, RepairOutcome::Sidecar, None, Some(asset)));
            }
            Ok(false) => skip(&mut out, item, RepairOutcome::Failed, "a different file is already in its place"),
            Err(e) => skip(&mut out, item, RepairOutcome::Failed, &e.to_string()),
        }
    }

    // 3. Damaged originals: swapped for the source's good copy.
    for item in chosen.iter().filter(|i| i.status == RepairClass::Damaged) {
        if cancel.is_cancelled() {
            out.cancelled = true;
            break;
        }
        let (Some(asset), Some(found)) = (item.asset_id, by_rel.get(rel_of(item).as_str())) else {
            skip(&mut out, item, RepairOutcome::Failed, "no longer on the source");
            continue;
        };
        let done = out.outcomes.len() as u64;
        let name = item.name.clone();
        let size = found.size;
        let mut report = |phase: RepairPhase, bytes_done: u64| {
            let large = size > LARGE_FILE;
            on_status(&status(phase, done, Some(name.clone()), if large { bytes_done } else { 0 }, if large { size } else { 0 }));
        };
        match swap(lib, source, asset, found, probe, &mut report) {
            Ok(()) => {
                out.repaired += 1;
                out.repaired_ids.push(asset);
                out.outcomes.push(outcome_row(item, RepairOutcome::Repaired, None, Some(asset)));
            }
            Err(SwapStop::SameFile) => skip(&mut out, item, RepairOutcome::SameFile, "the source holds the same damaged file"),
            Err(SwapStop::SourceDamaged(why)) => skip(&mut out, item, RepairOutcome::SourceDamaged, &why),
            Err(SwapStop::Duplicate) => skip(&mut out, item, RepairOutcome::Duplicate, "already in the library as another photo"),
            Err(SwapStop::Failed(why)) => skip(&mut out, item, RepairOutcome::Failed, &why),
        }
    }
    Ok(out)
}

/// The source's sidecar file a `sidecar` item names, found through the photo it pairs with.
fn find_sidecar(source: &dyn ImportSource, listed: &[SourceItem], item: &RepairItem) -> Option<SourceItem> {
    let rel = rel_of(item);
    let stem = item.name.split('.').next().unwrap_or("").to_lowercase();
    listed
        .iter()
        .filter(|m| dir_of(&m.rel_path) == item.dir && m.file_name.to_lowercase().starts_with(&stem))
        .flat_map(|m| source.sidecars(m))
        .find(|s| s.rel_path == rel)
}

/// Why a swap stopped. The library is as it was and nothing was set aside.
#[derive(Debug)]
enum SwapStop {
    /// The source's copy is the damaged original's own bytes.
    SameFile,
    /// The source's copy does not decode: the decoder's reason.
    SourceDamaged(String),
    /// The source's copy is another photo of the library.
    Duplicate,
    Failed(String),
}

impl From<std::io::Error> for SwapStop {
    fn from(e: std::io::Error) -> Self {
        SwapStop::Failed(format!("could not copy it: {e}"))
    }
}

impl From<ImportError> for SwapStop {
    fn from(e: ImportError) -> Self {
        SwapStop::Failed(e.to_string())
    }
}

impl From<photos_core::CoreError> for SwapStop {
    fn from(e: photos_core::CoreError) -> Self {
        SwapStop::Failed(e.to_string())
    }
}

/// `<library>/kept-aside/<YYYY-MM-DD>/<rel>`, numbered when that name is taken.
fn kept_aside_path(lib: &Library, rel: &str) -> PathBuf {
    let day = chrono::DateTime::from_timestamp(now_unix(), 0).unwrap_or_default().format("%Y-%m-%d").to_string();
    let first = lib.root.join("kept-aside").join(day).join(rel);
    if !first.exists() {
        return first;
    }
    let stem = first.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let ext = first.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
    (2..).map(|n| first.with_file_name(format!("{stem} ({n}){ext}"))).find(|p| !p.exists()).expect("a free name")
}

/// Replaces asset `asset`'s original by the source's `item`, in the spec's order (§3 rule 2), each
/// step checked before the next:
/// 1. copy to a temp name in the original's folder, hashing while copying, then read back;
///    1a. the damaged original's own bytes: stop (`SameFile`);
///    1b. a copy that does not decode: stop (`SourceDamaged`);
/// 2. bytes that belong to another asset: stop (`Duplicate`);
/// 3. the damaged original (if there) moves to `kept-aside/` — never deleted;
/// 4. the temp file takes the original's name; in one transaction hash, size and `unreadable`
///    (the probe passed) are updated and the replaced bytes recorded in `replaced_originals`.
///
/// Every stop removes the temp file; a failure at 4 undoes 3.
fn swap(lib: &Library, source: &dyn ImportSource, asset: i64, item: &SourceItem, probe: Probe, report: &mut dyn FnMut(RepairPhase, u64)) -> Result<(), SwapStop> {
    let target = Target::load(lib, asset)?;
    let dest = lib.originals_dir().join(&target.rel);
    let folder = dest.parent().ok_or_else(|| SwapStop::Failed("the original has no folder".into()))?;
    std::fs::create_dir_all(folder)?;
    let file_name = dest.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let tmp = folder.join(format!("{file_name}.repair.part"));

    // 1. Copy and verify (the import's own read-back).
    report(RepairPhase::Copying, 0);
    let staged = source
        .open(item)
        .and_then(|reader| crate::ingest::hash_and_stage_with(reader, &tmp, &mut |n| report(RepairPhase::Copying, n)))
        .and_then(|(hash, size)| {
            report(RepairPhase::Verifying, size);
            crate::verify::verify_staged(&tmp, &hash).map(|()| (hash, size))
        });
    let (hash, size) = match staged {
        Ok(s) => s,
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            return Err(e.into());
        }
    };
    put_in_place(lib, asset, &target, &tmp, &hash, size, &target.rel, None, probe)?;
    tracing::info!(asset, original = %target.rel, "repaired a damaged original");
    Ok(())
}

/// What an asset's row says about its original before a replacement.
struct Target {
    rel: String,
    extension: String,
    hash: String,
    size: i64,
}

impl Target {
    fn load(lib: &Library, asset: i64) -> Result<Target, SwapStop> {
        Ok(lib.db.with_conn(|c| {
            Ok(c.query_row("SELECT relative_path, extension, content_hash, byte_size FROM assets WHERE id = ?1", [asset], |r| {
                Ok(Target { rel: r.get(0)?, extension: r.get(1)?, hash: r.get(2)?, size: r.get(3)? })
            })?)
        })?)
    }
}

fn is_video_ext(ext: &str) -> bool {
    photos_media::kind_for_extension(ext) == photos_media::MediaKind::Video
}

/// Steps 1a–4 of [`swap`] for `tmp`, already staged and read back in the original's folder,
/// which takes the name `new_rel` (the original's own, or the same with another extension).
/// `recovered` is the figure a kept recovery records; `None` for a whole good copy.
#[allow(clippy::too_many_arguments)]
fn put_in_place(
    lib: &Library,
    asset: i64,
    target: &Target,
    tmp: &Path,
    hash: &str,
    size: u64,
    new_rel: &str,
    recovered: Option<u8>,
    probe: Probe,
) -> Result<(), SwapStop> {
    let stop = |why: SwapStop| {
        let _ = std::fs::remove_file(tmp);
        Err(why)
    };
    let old_dest = lib.originals_dir().join(&target.rel);
    let new_dest = lib.originals_dir().join(new_rel);
    let file_name = new_dest.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();

    // 1a. The damaged original's own bytes (read from disk: the catalogued hash may be the bytes
    // it had at import, before it was damaged).
    let (old_hash, old_size) = match (old_dest.is_file().then(|| hash_file(&old_dest)).flatten(), std::fs::metadata(&old_dest)) {
        (Some(h), Ok(m)) => (h, m.len() as i64),
        _ => (target.hash.clone(), target.size),
    };
    if old_dest.is_file() && old_hash == hash {
        return stop(SwapStop::SameFile);
    }

    // 1b. It must decode.
    if let Err(why) = probe(tmp, &file_name) {
        return stop(SwapStop::SourceDamaged(why));
    }

    // 2. Another photo's bytes: not this photo's good copy.
    match lib.db.with_conn(|c| assets::find_by_hash(c, hash)) {
        Ok(Some(other)) if other != asset => return stop(SwapStop::Duplicate),
        Ok(_) => {}
        Err(e) => return stop(e.into()),
    }
    if new_rel != target.rel && new_dest.exists() {
        return stop(SwapStop::Failed(format!("{file_name} already exists beside the original")));
    }

    // 3. The damaged original steps aside.
    let kept = if old_dest.exists() {
        let kept = kept_aside_path(lib, &target.rel);
        let moved = kept.parent().map_or(Ok(()), std::fs::create_dir_all).and_then(|()| std::fs::rename(&old_dest, &kept));
        if let Err(e) = moved {
            return stop(SwapStop::Failed(format!("could not set the damaged file aside: {e}")));
        }
        Some(kept)
    } else {
        None
    };
    let undo_aside = |kept: &Option<PathBuf>| {
        if let Some(k) = kept {
            if let Err(e) = std::fs::rename(k, &old_dest) {
                tracing::error!(kept = %k.display(), original = %old_dest.display(), "could not put the damaged original back: {e}");
            }
        }
    };

    // 4. The good copy takes its place; the catalogue follows in one transaction.
    if let Err(e) = std::fs::rename(tmp, &new_dest) {
        undo_aside(&kept);
        return stop(SwapStop::Failed(format!("could not put the good copy in place: {e}")));
    }
    let kept_rel = kept.as_deref().and_then(|k| k.strip_prefix(&lib.root).ok()).map(|p| p.to_string_lossy().replace('\\', "/"));
    let new_ext = Path::new(new_rel).extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
    let updated = lib.db.with_conn(|c| {
        let tx = c.unchecked_transaction()?;
        tx.execute(
            "UPDATE assets SET content_hash = ?2, byte_size = ?3, unreadable = 0, relative_path = ?4, recovered_pct = ?5, file_hash = NULL, metadata_written_at = NULL WHERE id = ?1",
            rusqlite::params![asset, hash, size as i64, new_rel, recovered],
        )?;
        if new_rel != target.rel {
            assets::set_container(&tx, asset, &new_ext, is_video_ext(&new_ext))?;
        }
        if old_hash != hash {
            tx.execute(
                "INSERT OR REPLACE INTO replaced_originals
                   (content_hash, asset_id, kept_rel, old_relative_path, old_extension, old_byte_size, replaced_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                rusqlite::params![old_hash, asset, kept_rel, target.rel, target.extension, old_size, now_unix()],
            )?;
        }
        tx.commit()?;
        Ok(())
    });
    if let Err(e) = updated {
        let _ = std::fs::remove_file(&new_dest);
        undo_aside(&kept);
        return Err(e.into());
    }
    Ok(())
}

impl SwapStop {
    fn into_error(self) -> ImportError {
        ImportError::Source(match self {
            SwapStop::SameFile => "it is the damaged file's own bytes".into(),
            SwapStop::SourceDamaged(why) => format!("it does not open: {why}"),
            SwapStop::Duplicate => "it is another photo of the library".into(),
            SwapStop::Failed(why) => why,
        })
    }
}

/// Keeps a recovery (§4): `staged` (a file under `<library>/recovered/`) replaces asset `asset`'s
/// damaged original by [`swap`]'s steps, the original's name taking the staged file's extension.
/// The asset keeps its id, albums, rating, date and edits; `recovered_pct` records `percent`. The
/// staged file is only read: the caller removes it.
pub fn keep_recovered(lib: &Library, asset: i64, staged: &Path, percent: u8, probe: Probe) -> Result<(), ImportError> {
    let target = Target::load(lib, asset).map_err(SwapStop::into_error)?;
    let ext = staged.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
    let new_rel = Path::new(&target.rel).with_extension(&ext).to_string_lossy().replace('\\', "/");
    let new_dest = lib.originals_dir().join(&new_rel);
    let folder = new_dest.parent().ok_or_else(|| ImportError::Source("the original has no folder".into()))?;
    std::fs::create_dir_all(folder)?;
    let tmp = folder.join(format!("{}.repair.part", new_dest.file_name().map(|n| n.to_string_lossy()).unwrap_or_default()));
    let staged_copy = std::fs::File::open(staged)
        .map_err(ImportError::from)
        .and_then(|f| crate::ingest::hash_and_stage(Box::new(f), &tmp))
        .and_then(|(hash, size)| crate::verify::verify_staged(&tmp, &hash).map(|()| (hash, size)));
    let (hash, size) = match staged_copy {
        Ok(s) => s,
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            return Err(e);
        }
    };
    put_in_place(lib, asset, &target, &tmp, &hash, size, &new_rel, Some(percent.clamp(1, 100)), probe).map_err(SwapStop::into_error)?;
    tracing::info!(asset, original = %new_rel, percent, "kept a recovery of a damaged original");
    Ok(())
}

/// Undoes the last replacement of asset `asset`'s original (a kept recovery, §4): the file now in
/// its place moves to `<library>/recovered/<asset>.<ext>`, the kept-aside original comes back, and
/// the row is as it was (path, extension, size, hash, `unreadable = 1`, no `recovered_pct`).
/// Returns where the replacing file went.
pub fn undo_replace(lib: &Library, asset: i64) -> Result<PathBuf, ImportError> {
    let (rel, ext): (String, String) =
        lib.db.with_conn(|c| Ok(c.query_row("SELECT relative_path, extension FROM assets WHERE id = ?1", [asset], |r| Ok((r.get(0)?, r.get(1)?)))?))?;
    type Replaced = (String, Option<String>, String, String, i64);
    let (old_hash, kept_rel, old_rel, old_ext, old_size): Replaced = lib.db.with_conn(|c| {
        Ok(c.query_row(
            "SELECT content_hash, kept_rel, old_relative_path, old_extension, old_byte_size FROM replaced_originals
              WHERE asset_id = ?1 ORDER BY replaced_at DESC, rowid DESC LIMIT 1",
            [asset],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
        )?)
    })?;
    let current = lib.originals_dir().join(&rel);
    let restored = lib.originals_dir().join(&old_rel);
    let back_to = lib.root.join("recovered").join(format!("{asset}.{}", ext.to_ascii_lowercase()));
    std::fs::create_dir_all(lib.root.join("recovered"))?;
    if restored.exists() && restored != current {
        return Err(ImportError::Source(format!("{} is in the way", restored.display())));
    }

    std::fs::rename(&current, &back_to)?;
    let put_back = |kept: &Path| -> std::io::Result<()> {
        if let Some(p) = restored.parent() {
            std::fs::create_dir_all(p)?;
        }
        std::fs::rename(kept, &restored)
    };
    let kept = kept_rel.as_deref().map(|k| lib.root.join(k));
    if let Some(k) = &kept {
        if let Err(e) = put_back(k) {
            let _ = std::fs::rename(&back_to, &current);
            return Err(e.into());
        }
    }
    let updated = lib.db.with_conn(|c| {
        let tx = c.unchecked_transaction()?;
        tx.execute(
            "UPDATE assets SET relative_path = ?2, content_hash = ?3, byte_size = ?4, unreadable = 1, recovered_pct = NULL WHERE id = ?1",
            rusqlite::params![asset, old_rel, old_hash, old_size],
        )?;
        assets::set_container(&tx, asset, &old_ext, is_video_ext(&old_ext))?;
        tx.execute("DELETE FROM replaced_originals WHERE content_hash = ?1", [&old_hash])?;
        tx.commit()?;
        Ok(())
    });
    if let Err(e) = updated {
        if let Some(k) = &kept {
            let _ = std::fs::rename(&restored, k);
        }
        let _ = std::fs::rename(&back_to, &current);
        return Err(e.into());
    }
    remove_empty_parents(kept.as_deref(), &lib.root.join("kept-aside"));
    tracing::info!(asset, original = %old_rel, "undid the replacement of an original");
    Ok(back_to)
}

/// Removes the folders `kept-aside/<date>/…` a put-back file leaves empty, up to `stop`.
fn remove_empty_parents(file: Option<&Path>, stop: &Path) {
    let mut dir = file.and_then(Path::parent);
    while let Some(d) = dir {
        if !d.starts_with(stop) || std::fs::remove_dir(d).is_err() {
            break;
        }
        dir = d.parent();
    }
}
