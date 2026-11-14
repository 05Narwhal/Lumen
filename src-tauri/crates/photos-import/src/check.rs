//! *Check before deleting* (Round 19 §13, item 6): two read-only tools for the person who is
//! about to delete a source copy.
//!
//! * [`check_folder`] hashes every file under a folder or card and looks each one up by
//!   `assets.content_hash`; the library's own copy is read back from the disk too. Each file is
//!   **Safe** (in the library, both copies identical), **Not in the library** (with the reason
//!   when known) or an **Other file** (not media, not a sidecar).
//! * [`check_library`] re-hashes every original and sidecar against the catalogue and lists the
//!   missing or changed ones. It is the proof for sources that no longer exist.
//!
//! Neither writes a file or a row; the caller keeps the date of a full library check.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

use photos_core::sidecars;
use photos_core::Library;
use photos_jobs::CancelToken;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::verify::read_back_hash;
use crate::ImportError;

/// Hashing is disk-bound; a few readers keep a card or an SSD busy without starving the app.
const THREADS: usize = 4;
/// Library check: assets read per keyset page.
const PAGE: i64 = 500;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum CheckVerdict {
    Safe,
    NotInLibrary,
    Other,
}

/// One file of the checked folder.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct CheckItem {
    /// Relative to the checked folder, with `/` separators.
    pub path: String,
    pub status: CheckVerdict,
    /// Why a file is not in the library, as a stable code: `not_imported`, `in_trash`,
    /// `copy_missing`, `copy_differs`, `copy_unreadable`, `cannot_read`, `sidecar_not_kept`, or
    /// the reason an import recorded (`failed_verify`, `unreadable`, `burst_frame`, …).
    pub reason: Option<String>,
    #[ts(type = "number | null")]
    pub asset_id: Option<i64>,
    #[ts(type = "number")]
    pub byte_size: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct FolderCheck {
    pub folder: String,
    #[ts(type = "number")]
    pub checked_at: i64,
    pub items: Vec<CheckItem>,
    pub safe: u32,
    pub not_in_library: u32,
    pub other: u32,
    /// The check was stopped before every file was looked at: not a verdict.
    pub cancelled: bool,
}

impl FolderCheck {
    /// Every media file and sidecar is safe and the check ran to the end. *Other files* do not
    /// block this (Lumen keeps no other files); the report says how many there are.
    pub fn is_safe_to_delete(&self) -> bool {
        !self.cancelled && self.not_in_library == 0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum CheckProblem {
    Missing,
    Changed,
}

/// A file of the library that is not what the catalogue says it is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct CheckProblemItem {
    #[ts(type = "number")]
    pub asset_id: i64,
    pub filename: String,
    /// Under `originals/`.
    pub relative_path: String,
    /// `original`, or the sidecar's kind (`xmp`, `aae`, `thm`).
    pub kind: String,
    pub status: CheckProblem,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct LibraryCheck {
    #[ts(type = "number")]
    pub checked_at: i64,
    /// Originals and sidecars re-hashed.
    #[ts(type = "number")]
    pub checked: u64,
    #[ts(type = "number")]
    pub bytes: u64,
    pub problems: Vec<CheckProblemItem>,
    pub cancelled: bool,
}

impl LibraryCheck {
    pub fn is_intact(&self) -> bool {
        !self.cancelled && self.problems.is_empty()
    }
}

#[derive(Debug, Clone, Copy, Default, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct CheckProgress {
    #[ts(type = "number")]
    pub done_files: u64,
    #[ts(type = "number")]
    pub total_files: u64,
    #[ts(type = "number")]
    pub done_bytes: u64,
    #[ts(type = "number")]
    pub total_bytes: u64,
}

struct Counters {
    done_files: AtomicU64,
    done_bytes: AtomicU64,
    total_files: u64,
    total_bytes: u64,
}

impl Counters {
    fn new(total_files: u64, total_bytes: u64) -> Self {
        Self { done_files: AtomicU64::new(0), done_bytes: AtomicU64::new(0), total_files, total_bytes }
    }

    fn step(&self, bytes: u64, report: &(dyn Fn(CheckProgress) + Sync)) {
        let done_files = self.done_files.fetch_add(1, Ordering::Relaxed) + 1;
        let done_bytes = self.done_bytes.fetch_add(bytes, Ordering::Relaxed) + bytes;
        report(CheckProgress { done_files, total_files: self.total_files, done_bytes, total_bytes: self.total_bytes });
    }
}

fn pool() -> Result<rayon::ThreadPool, ImportError> {
    rayon::ThreadPoolBuilder::new()
        .num_threads(THREADS)
        .build()
        .map_err(|e| ImportError::Source(format!("could not start the check: {e}")))
}

// -- Check a folder ----------------------------------------------------------------------------

enum Kind {
    Media,
    Sidecar,
    Other,
}

struct Entry {
    rel: String,
    abs: PathBuf,
    kind: Kind,
    size: u64,
}

/// What reading the library's own copy found.
#[derive(Clone, Copy, PartialEq, Eq)]
enum CopyState {
    Intact,
    Missing,
    Differs,
    Unreadable,
}

impl CopyState {
    fn code(self) -> &'static str {
        match self {
            CopyState::Intact => unreachable!("an intact copy has no reason"),
            CopyState::Missing => "copy_missing",
            CopyState::Differs => "copy_differs",
            CopyState::Unreadable => "copy_unreadable",
        }
    }
}

/// Reads `copy` back from the disk and compares it with the catalogued hash.
fn read_copy(copy: &Path, expected: &str) -> CopyState {
    match read_back_hash(copy) {
        Ok(h) if h == expected => CopyState::Intact,
        Ok(_) => CopyState::Differs,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => CopyState::Missing,
        Err(_) => CopyState::Unreadable,
    }
}

fn base_name(path: &str) -> String {
    path.rsplit(['/', '\\']).next().unwrap_or(path).to_lowercase()
}

/// Reasons the imports recorded, by lower-case file name; the newest record wins.
fn known_skips(lib: &Library) -> HashMap<String, String> {
    let rows: Vec<(String, String)> = lib
        .db
        .with_conn(|c| {
            let mut st = c.prepare("SELECT path, reason FROM import_skips ORDER BY id")?;
            let rows = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        })
        .unwrap_or_default();
    rows.into_iter().map(|(p, r)| (base_name(&p), r)).collect()
}

fn walk(folder: &Path) -> (Vec<Entry>, Vec<CheckItem>) {
    let mut entries = Vec::new();
    let mut unreadable = Vec::new();
    for found in walkdir::WalkDir::new(folder).follow_links(false).sort_by_file_name() {
        let dir_entry = match found {
            Ok(e) => e,
            Err(e) => {
                // A folder that cannot be opened hides files: say so instead of skipping it.
                let path = e.path().map(|p| rel_of(folder, p)).unwrap_or_default();
                unreadable.push(CheckItem { path, status: CheckVerdict::NotInLibrary, reason: Some("cannot_read".into()), asset_id: None, byte_size: 0 });
                continue;
            }
        };
        if !dir_entry.file_type().is_file() {
            continue;
        }
        let abs = dir_entry.path().to_path_buf();
        let kind = if abs.extension().and_then(|e| e.to_str()).and_then(sidecars::kind_for_extension).is_some() {
            Kind::Sidecar
        } else if photos_media::is_media_file(&abs) {
            Kind::Media
        } else {
            Kind::Other
        };
        let size = dir_entry.metadata().map(|m| m.len()).unwrap_or(0);
        entries.push(Entry { rel: rel_of(folder, &abs), abs, kind, size });
    }
    (entries, unreadable)
}

fn rel_of(folder: &Path, path: &Path) -> String {
    path.strip_prefix(folder).unwrap_or(path).to_string_lossy().replace('\\', "/")
}

/// Judges every file under `folder` against `lib`. Reads only.
pub fn check_folder(
    lib: &Library,
    folder: &Path,
    cancel: &CancelToken,
    report: &(dyn Fn(CheckProgress) + Sync),
) -> Result<FolderCheck, ImportError> {
    if !folder.is_dir() {
        return Err(ImportError::Source("That folder is not there.".into()));
    }
    let (entries, mut items) = walk(folder);
    let total_bytes = entries.iter().filter(|e| !matches!(e.kind, Kind::Other)).map(|e| e.size).sum();
    let total_files = entries.iter().filter(|e| !matches!(e.kind, Kind::Other)).count() as u64;
    let counters = Counters::new(total_files, total_bytes);
    let skips = known_skips(lib);
    let copies: Mutex<HashMap<(bool, i64), CopyState>> = Mutex::new(HashMap::new());

    let judged: Vec<Option<CheckItem>> = pool()?.install(|| {
        entries
            .par_iter()
            .map(|e| {
                if cancel.is_cancelled() {
                    return None;
                }
                let item = match e.kind {
                    Kind::Other => CheckItem { path: e.rel.clone(), status: CheckVerdict::Other, reason: None, asset_id: None, byte_size: e.size },
                    Kind::Media | Kind::Sidecar => {
                        let item = judge(lib, e, &skips, &copies);
                        counters.step(e.size, report);
                        item
                    }
                };
                Some(item)
            })
            .collect()
    });
    let cancelled = cancel.is_cancelled() || judged.iter().any(Option::is_none);
    items.extend(judged.into_iter().flatten());

    let count = |s: CheckVerdict| items.iter().filter(|i| i.status == s).count() as u32;
    Ok(FolderCheck {
        folder: folder.to_string_lossy().into_owned(),
        checked_at: photos_core::now_unix(),
        safe: count(CheckVerdict::Safe),
        not_in_library: count(CheckVerdict::NotInLibrary),
        other: count(CheckVerdict::Other),
        items,
        cancelled,
    })
}

fn judge(lib: &Library, e: &Entry, skips: &HashMap<String, String>, copies: &Mutex<HashMap<(bool, i64), CopyState>>) -> CheckItem {
    let item = |status, reason: Option<&str>, asset_id| CheckItem { path: e.rel.clone(), status, reason: reason.map(str::to_string), asset_id, byte_size: e.size };
    let hash = match read_back_hash(&e.abs) {
        Ok(h) => h,
        Err(_) => return item(CheckVerdict::NotInLibrary, Some("cannot_read"), None),
    };
    let sidecar = matches!(e.kind, Kind::Sidecar);

    // (asset id, path under originals/, trashed) of the library's file with these bytes.
    let found: Option<(i64, String, bool)> = lib
        .db
        .with_conn(|c| {
            use rusqlite::OptionalExtension;
            let row = if sidecar {
                c.query_row(
                    "SELECT s.asset_id, s.relative_path, a.state FROM asset_sidecars s JOIN assets a ON a.id = s.asset_id WHERE s.content_hash = ?1 ORDER BY s.id LIMIT 1",
                    [&hash],
                    |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, i64>(2)? == 1)),
                )
                .optional()?
            } else {
                c.query_row("SELECT id, relative_path, state FROM assets WHERE content_hash = ?1", [&hash], |r| {
                    Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, i64>(2)? == 1))
                })
                .optional()?
            };
            Ok(row)
        })
        .unwrap_or(None);

    let Some((asset_id, relative, trashed)) = found else {
        let reason = if sidecar {
            "sidecar_not_kept".to_string()
        } else {
            skips.get(&base_name(&e.rel)).cloned().unwrap_or_else(|| "not_imported".to_string())
        };
        return item(CheckVerdict::NotInLibrary, Some(&reason), None);
    };
    if trashed {
        return item(CheckVerdict::NotInLibrary, Some("in_trash"), Some(asset_id));
    }
    // The library's copy is read back too: a catalogue row says nothing about the bytes.
    let key = (sidecar, asset_id);
    let known = copies.lock().unwrap_or_else(|p| p.into_inner()).get(&key).copied();
    let copy = known.unwrap_or_else(|| {
        let c = read_copy(&lib.originals_dir().join(&relative), &hash);
        copies.lock().unwrap_or_else(|p| p.into_inner()).insert(key, c);
        c
    });
    match copy {
        CopyState::Intact => item(CheckVerdict::Safe, None, Some(asset_id)),
        bad => item(CheckVerdict::NotInLibrary, Some(bad.code()), Some(asset_id)),
    }
}

// -- Check the library -------------------------------------------------------------------------

/// Copies made for an external editor are the editor's to change (Round 22 §8): *Check the
/// library* leaves the paths `external_edits` names alone.
const NOT_AN_EDITED_COPY: &str = "relative_path NOT IN (SELECT relative_path FROM external_edits)";

struct Row {
    asset_id: i64,
    filename: String,
    relative_path: String,
    kind: String,
    hash: String,
    size: u64,
}

/// One keyset page: assets after `after`, then the sidecars of those assets.
fn page(lib: &Library, after: i64) -> Result<Vec<Row>, ImportError> {
    Ok(lib.db.with_conn(|c| {
        let mut rows = Vec::new();
        let mut st = c.prepare(&format!(
            "SELECT id, filename, relative_path, content_hash, byte_size FROM assets WHERE id > ?1 AND {NOT_AN_EDITED_COPY} ORDER BY id LIMIT ?2"
        ))?;
        let assets = st
            .query_map(rusqlite::params![after, PAGE], |r| {
                Ok(Row { asset_id: r.get(0)?, filename: r.get(1)?, relative_path: r.get(2)?, kind: "original".into(), hash: r.get(3)?, size: r.get::<_, i64>(4)?.max(0) as u64 })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        if let (Some(first), Some(last)) = (assets.first().map(|r| r.asset_id), assets.last().map(|r| r.asset_id)) {
            let mut st = c.prepare("SELECT asset_id, relative_path, kind, content_hash, byte_size, source_name FROM asset_sidecars WHERE asset_id BETWEEN ?1 AND ?2 ORDER BY id")?;
            let sidecar_rows = st
                .query_map([first, last], |r| {
                    let rel: String = r.get(1)?;
                    Ok(Row { asset_id: r.get(0)?, filename: r.get(5)?, relative_path: rel, kind: r.get(2)?, hash: r.get(3)?, size: r.get::<_, i64>(4)?.max(0) as u64 })
                })?
                .collect::<Result<Vec<_>, _>>()?;
            rows.extend(sidecar_rows);
        }
        rows.extend(assets);
        Ok(rows)
    })?)
}

/// Re-hashes every original and sidecar of `lib` against the catalogue. Reads only.
pub fn check_library(lib: &Library, cancel: &CancelToken, report: &(dyn Fn(CheckProgress) + Sync)) -> Result<LibraryCheck, ImportError> {
    let (total_files, total_bytes): (u64, u64) = lib.db.with_conn(|c| {
        let (a, ab): (i64, i64) = c.query_row(
            &format!("SELECT COUNT(*), COALESCE(SUM(byte_size), 0) FROM assets WHERE {NOT_AN_EDITED_COPY}"),
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        let (s, sb): (i64, i64) = c.query_row("SELECT COUNT(*), COALESCE(SUM(byte_size), 0) FROM asset_sidecars", [], |r| Ok((r.get(0)?, r.get(1)?)))?;
        Ok(((a + s).max(0) as u64, (ab + sb).max(0) as u64))
    })?;
    let counters = Counters::new(total_files, total_bytes);
    let pool = pool()?;
    let originals = lib.originals_dir();
    let mut problems = Vec::new();
    let mut checked = 0u64;
    let mut bytes = 0u64;
    let mut cancelled = false;
    let mut after = 0i64;
    loop {
        let rows = page(lib, after)?;
        let Some(last_asset) = rows.iter().filter(|r| r.kind == "original").map(|r| r.asset_id).max() else { break };
        after = last_asset;
        let results: Vec<Option<Option<CheckProblem>>> = pool.install(|| {
            rows.par_iter()
                .map(|r| {
                    if cancel.is_cancelled() {
                        return None;
                    }
                    let verdict = match read_copy(&originals.join(&r.relative_path), &r.hash) {
                        CopyState::Intact => None,
                        CopyState::Missing => Some(CheckProblem::Missing),
                        CopyState::Differs | CopyState::Unreadable => Some(CheckProblem::Changed),
                    };
                    counters.step(r.size, report);
                    Some(verdict)
                })
                .collect()
        });
        for (row, result) in rows.into_iter().zip(results) {
            match result {
                None => cancelled = true,
                Some(verdict) => {
                    checked += 1;
                    bytes += row.size;
                    if let Some(status) = verdict {
                        problems.push(CheckProblemItem { asset_id: row.asset_id, filename: row.filename, relative_path: row.relative_path, kind: row.kind, status });
                    }
                }
            }
        }
        if cancelled {
            break;
        }
    }
    Ok(LibraryCheck { checked_at: photos_core::now_unix(), checked, bytes, problems, cancelled })
}
