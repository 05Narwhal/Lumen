//! *Import → Repair* (Round 21 §6): runs `photos_import::repair::check_source` as one job on the
//! Maintenance lane, reports to the activity ring (`repair://progress`, `repair://finished`) and
//! keeps the last result in memory for the screen to list in pages. The check writes nothing, to
//! the library or to the source.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use photos_core::Library;
use photos_media::failure::UnreadableCause;
use photos_media::recover::{Recovered, RecoveredKind};
use serde::Deserialize;

use photos_import::repair::{self, RepairClass, RepairCounts, RepairItem, RepairOutcome, RepairOutcomeRow, RepairPhase, RepairResult, RepairSelection, RepairSource, RepairSourceKind, RepairSourceRef, RepairStatus};
use photos_import::macdisk::source::MacFolderSource;
use photos_import::{FolderSource, ImportSource, IphoneInfo, IphoneSource};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};
use ts_rs::TS;

use crate::error::{AppError, AppResult};
use crate::events::{MacDiskInfo, VolumeInfo};
use crate::state::AppState;

pub const REPAIR_PROGRESS: &str = "repair://progress";
pub const REPAIR_FINISHED: &str = "repair://finished";
/// A re-import is done: its `RepairResult` (*n imported · n repaired · n sidecars · n skipped*).
pub const REPAIR_IMPORTED: &str = "repair://imported";

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RepairFinished {
    pub cancelled: bool,
    pub error: Option<String>,
    /// How a re-import's items ended (all zero after a plain check).
    pub outcomes: OutcomeCounts,
}

/// The summary line of a run: how many items ended each way (§3 rule 6).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct OutcomeCounts {
    pub repaired: u32,
    pub imported: u32,
    pub sidecar: u32,
    pub source_damaged: u32,
    pub same_file: u32,
    pub duplicate: u32,
    pub failed: u32,
}

impl OutcomeCounts {
    pub fn of(rows: &[RepairOutcomeRow]) -> Self {
        let mut c = Self::default();
        for r in rows {
            *match r.outcome {
                RepairOutcome::Repaired => &mut c.repaired,
                RepairOutcome::Imported => &mut c.imported,
                RepairOutcome::Sidecar => &mut c.sidecar,
                RepairOutcome::SourceDamaged => &mut c.source_damaged,
                RepairOutcome::SameFile => &mut c.same_file,
                RepairOutcome::Duplicate => &mut c.duplicate,
                RepairOutcome::Failed => &mut c.failed,
            } += 1;
        }
        c
    }
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RepairItemsPage {
    pub items: Vec<RepairItem>,
    /// The `id` of the last item handed out; pass it as `after` for the next page.
    #[ts(type = "number | null")]
    pub next_cursor: Option<i64>,
}

/// The last check of the open library.
struct Last {
    library_id: String,
    /// What the screen titles the result with, and the batch is named after (*Repair · <name>*).
    source_name: String,
    source: RepairSourceRef,
    /// The photos the check was limited to (*Restore…*), checked the same way after a re-import.
    only_assets: Option<Vec<i64>>,
    status: RepairStatus,
    items: Vec<RepairItem>,
    /// How each item of the last re-import ended; kept across the checks that follow it.
    outcomes: Vec<RepairOutcomeRow>,
}

static LAST: Mutex<Option<Last>> = Mutex::new(None);

fn last() -> std::sync::MutexGuard<'static, Option<Last>> {
    LAST.lock().unwrap_or_else(|p| p.into_inner())
}

pub(crate) fn job_key(library_id: &str) -> String {
    format!("repair:{library_id}")
}

fn error(code: &str, message: &str) -> AppError {
    AppError { code: code.into(), message: message.into(), detail: None }
}

fn idle() -> RepairStatus {
    RepairStatus { phase: RepairPhase::Idle, done: 0, total: 0, counts: RepairCounts::default(), cancelled: false, current: None, bytes_done: 0, bytes_total: 0 }
}

// ---------------------------------------------------------------- sources

fn size_text(bytes: u64) -> String {
    const GB: f64 = 1_000_000_000.0;
    let b = bytes as f64;
    if b >= 1000.0 * GB {
        format!("{:.1} TB", b / (1000.0 * GB))
    } else {
        format!("{:.0} GB", b / GB)
    }
}

/// The rows of step 1 as plain data: every volume Lumen sees, each iPhone (an unpaired one is
/// listed but not ready) and each Apple-formatted disk. Folders are not listed: the person
/// chooses them with the OS dialog.
pub fn map_sources(volumes: &[VolumeInfo], iphones: &[IphoneInfo], disks: &[MacDiskInfo]) -> Vec<RepairSource> {
    let mut out = Vec::new();
    for v in volumes {
        out.push(RepairSource {
            source_ref: RepairSourceRef { kind: RepairSourceKind::Volume, id: v.mount.clone() },
            name: v.label.clone(),
            detail: v.mount.clone(),
            ready: true,
            removable: v.removable,
        });
    }
    for p in iphones {
        out.push(RepairSource {
            source_ref: RepairSourceRef { kind: RepairSourceKind::Iphone, id: p.udid.clone() },
            name: p.name.clone(),
            detail: if p.paired { p.model.clone().unwrap_or_else(|| "iPhone".into()) } else { "Unlock the phone and tap Trust".into() },
            ready: p.paired,
            removable: true,
        });
    }
    for d in disks {
        out.push(RepairSource {
            source_ref: RepairSourceRef { kind: RepairSourceKind::MacDisk, id: d.id.clone() },
            name: d.name.clone(),
            detail: format!("Apple-formatted · {}", size_text(d.total_bytes)),
            ready: true,
            removable: true,
        });
    }
    out
}

pub fn list_sources() -> Vec<RepairSource> {
    let phones = crate::iphones::current().devices;
    map_sources(&crate::volumes::snapshot(), &phones, &crate::macdisks::snapshot())
}

/// The `ImportSource` behind a reference, and the name the result is titled with.
fn open_source(r: &RepairSourceRef) -> AppResult<(Box<dyn ImportSource>, String)> {
    match r.kind {
        // A volume is checked whole, like a folder at its root.
        RepairSourceKind::Folder | RepairSourceKind::Volume => {
            let root = Path::new(&r.id);
            if !root.is_dir() {
                return Err(error("repair_no_folder", "That folder is not there."));
            }
            let source = FolderSource::new(root);
            let name = source.label();
            Ok((Box::new(source), name))
        }
        RepairSourceKind::Iphone => {
            let phone = crate::iphones::current()
                .devices
                .into_iter()
                .find(|p| p.udid == r.id)
                .ok_or_else(|| error("repair_source_gone", "That phone is no longer connected."))?;
            if !phone.paired {
                return Err(error("repair_not_paired", "Unlock the phone and tap Trust."));
            }
            Ok((Box::new(IphoneSource::new(&phone.udid, &phone.name)), phone.name))
        }
        RepairSourceKind::MacDisk => {
            let (info, fs) = crate::macdisks::get(&r.id).ok_or_else(|| error("repair_source_gone", "That disk is no longer connected."))?;
            Ok((Box::new(MacFolderSource::new(fs, &info.name, "/", true)), info.name))
        }
    }
}

// ---------------------------------------------------------------- the job

pub fn status() -> RepairStatus {
    last().as_ref().map_or_else(idle, |l| l.status.clone())
}

/// Starts a check of `source` against the open library. With `only_assets` it looks only at files
/// that identity-match those photos (*Restore…*, §7). One at a time.
pub fn start(app: &AppHandle, source: RepairSourceRef, only_assets: Option<Vec<i64>>) -> AppResult<()> {
    let state = app.state::<AppState>();
    let lib = state.library()?;
    let id = lib.manifest.id.clone();
    let key = job_key(&id);
    if state.jobs.is_inflight(&key) {
        return Err(error("repair_running", "A check is already running."));
    }
    let (opened, name) = open_source(&source)?;
    let outcomes = last().as_ref().filter(|l| l.library_id == id).map(|l| l.outcomes.clone()).unwrap_or_default();
    *last() = Some(Last {
        library_id: id.clone(),
        source_name: name,
        source: source.clone(),
        only_assets: only_assets.clone(),
        status: RepairStatus { phase: RepairPhase::Listing, ..idle() },
        items: Vec::new(),
        outcomes,
    });
    let a = app.clone();
    state.jobs.submit(&key, photos_jobs::Lane::Maintenance, move |cancel| {
        let finished = check_into_last(&a, &lib, &id, opened.as_ref(), only_assets.as_deref(), cancel);
        let _ = a.emit(REPAIR_FINISHED, finished);
    });
    Ok(())
}

/// A big file is a good file: lane A makes it open, so "too large" is not damage.
fn accept(reason: String) -> Result<(), String> {
    if photos_media::failure::cause(&reason) == photos_media::failure::UnreadableCause::TooLarge {
        Ok(())
    } else {
        Err(reason)
    }
}

/// Says whether the file at `path` decodes (Round 22 §3); `file_name` is its real name, since
/// `path` may end `.part`. The kind comes from the name, corrected by the bytes, as an import
/// does. No FFmpeg for a HEIC or a video, no RAW library, or a kind Lumen does not decode: it
/// cannot judge, and a missing tool is a state, so the file passes.
fn probe_with(path: &Path, file_name: &str, ffmpeg: Option<&photos_media::Ffmpeg>) -> Result<(), String> {
    use photos_media::MediaKind;
    let ext = Path::new(file_name).extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    let named = photos_media::kind_for_extension(&ext);
    let kind = if matches!(named, MediaKind::Image | MediaKind::Heic | MediaKind::Video) { photos_media::content_kind(path).unwrap_or(named) } else { named };
    match kind {
        MediaKind::Unknown => return Ok(()),
        MediaKind::Heic | MediaKind::Video if ffmpeg.is_none() => return Ok(()),
        _ => {}
    }
    match photos_media::decode_preview_with(path, kind, ffmpeg) {
        Ok(_) => Ok(()),
        Err(e) if photos_media::develop::raw::is_no_library(&e) => Ok(()),
        Err(e) => accept(e.to_string().lines().next().unwrap_or("it does not decode").trim().to_string()),
    }
}

/// Keeps each status as the last one and tells the ring, four times a second at most; a change of
/// phase or file always goes out, so the last event of a phase is never lost.
fn reporter<'a>(a: &'a AppHandle, id: &'a str) -> impl Fn(RepairStatus) + Sync + 'a {
    let gate = Mutex::new((Instant::now() - Duration::from_secs(1), RepairPhase::Idle, None::<String>));
    move |s: RepairStatus| {
        if let Some(l) = last().as_mut().filter(|l| l.library_id == id) {
            l.status = s.clone();
        }
        let mut g = gate.lock().unwrap_or_else(|e| e.into_inner());
        let changed = g.1 != s.phase || g.2 != s.current;
        if changed || g.0.elapsed() >= Duration::from_millis(250) {
            *g = (Instant::now(), s.phase, s.current.clone());
            let _ = a.emit(REPAIR_PROGRESS, s);
        }
    }
}

/// Runs the check and keeps its result as the last one, reporting as it goes.
fn check_into_last(a: &AppHandle, lib: &photos_core::Library, id: &str, source: &dyn ImportSource, only_assets: Option<&[i64]>, cancel: &photos_jobs::CancelToken) -> RepairFinished {
    let report = reporter(a, id);
    let ffmpeg = a.state::<AppState>().ffmpeg();
    let probe = |path: &Path, name: &str| probe_with(path, name, ffmpeg.as_ref());
    tracing::info!("repair check started");
    let finished = match repair::check_source(source, lib, only_assets, &probe, &report, cancel) {
        Ok(check) => {
            tracing::info!(cancelled = check.cancelled, "repair check finished");
            let done = RepairStatus {
                phase: RepairPhase::Done,
                done: check.counts.safe as u64 + check.counts.new as u64 + check.counts.damaged as u64 + check.counts.source_damaged as u64 + check.counts.in_trash as u64,
                total: 0,
                counts: check.counts,
                cancelled: check.cancelled,
                ..idle()
            };
            let cancelled = check.cancelled;
            if let Some(l) = last().as_mut().filter(|l| l.library_id == id) {
                // The total is the number of files looked at.
                l.status = RepairStatus { total: done.done, ..done };
                l.items = check.items;
            }
            RepairFinished { cancelled, error: None, outcomes: OutcomeCounts::default() }
        }
        Err(e) => {
            tracing::warn!("repair check stopped: {e}");
            if let Some(l) = last().as_mut().filter(|l| l.library_id == id) {
                l.status = idle();
            }
            RepairFinished { cancelled: false, error: Some(e.to_string()), outcomes: OutcomeCounts::default() }
        }
    };
    if let Some(l) = last().as_ref().filter(|l| l.library_id == id) {
        let _ = a.emit(REPAIR_PROGRESS, &l.status);
    }
    finished
}

/// Brings in what the person ticked in the last check (§6 *Re-import*): new files as one
/// *Repair · <source>* import batch, missing sidecars, and damaged originals swapped for the
/// source's good copy. Then the source is checked again, so the list shows what is left.
/// Reports `repair://imported` (a `RepairResult`) and, after the new check, `repair://finished`.
pub fn start_import(app: &AppHandle, selection: RepairSelection) -> AppResult<()> {
    let state = app.state::<AppState>();
    let lib = state.library()?;
    let id = lib.manifest.id.clone();
    let key = job_key(&id);
    if state.jobs.is_inflight(&key) {
        return Err(error("repair_running", "A check is already running."));
    }
    let (source, name, only_assets, chosen) = {
        let held = last();
        let l = held.as_ref().filter(|l| l.library_id == id && l.status.phase == RepairPhase::Done).ok_or_else(|| error("repair_no_check", "Check a source first."))?;
        let chosen: Vec<RepairItem> = repair::select(&l.items, &selection).into_iter().cloned().collect();
        (l.source.clone(), l.source_name.clone(), l.only_assets.clone(), chosen)
    };
    if chosen.is_empty() {
        return Err(error("repair_nothing_selected", "Nothing is selected."));
    }
    let (opened, _) = open_source(&source)?;
    let ffmpeg = state.ffmpeg();
    let root = lib.root.clone();
    let free_space = Some(photos_import::FreeSpace::new(move || crate::space::free_space(&root)));
    let opts = photos_import::ImportOptions { skip_duplicates: true, ffmpeg, free_space, ..Default::default() };
    let a = app.clone();
    state.jobs.submit(&key, photos_jobs::Lane::Maintenance, move |cancel| {
        tracing::info!(files = chosen.len(), "repair re-import started");
        let (app_progress, app_ingest) = (a.clone(), a.clone());
        let report = reporter(&a, &id);
        let ffmpeg = a.state::<AppState>().ffmpeg();
        let probe = |path: &Path, name: &str| probe_with(path, name, ffmpeg.as_ref());
        let applied = repair::apply(
            opened.as_ref(),
            &lib,
            &name,
            &chosen,
            &opts,
            &probe,
            cancel,
            &mut |p| {
                let _ = app_progress.emit(crate::events::IMPORT_PROGRESS, p);
            },
            &mut |i| crate::derivatives::schedule_ingest(app_ingest.clone(), i.clone()),
            &mut |s| report(s.clone()),
        );
        let result = match applied {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!("repair re-import stopped: {e}");
                RepairResult { notes: vec![e.to_string()], skipped: chosen.len() as u32, ..Default::default() }
            }
        };
        tracing::info!(imported = result.imported, repaired = result.repaired, sidecars = result.sidecars, skipped = result.skipped, "repair re-import finished");
        // A swapped original is a new file under the same photo: its metadata is read again
        // (merged, never overwritten) and its pictures are made again.
        let ffmpeg = a.state::<AppState>().ffmpeg();
        for &asset in &result.repaired_ids {
            let rel = lib.db.with_conn(|c| Ok(c.query_row("SELECT relative_path FROM assets WHERE id = ?1", [asset], |r| r.get::<_, String>(0))?));
            if let Ok(rel) = rel {
                crate::maintenance::reread_metadata(&lib.db, asset, &lib.originals_dir().join(rel), ffmpeg.as_ref());
            }
        }
        crate::derivatives::retry_unreadable(&a, &result.repaired_ids);
        if result.imported > 0 {
            crate::maintenance::schedule_memories(&a);
            crate::embeddings::schedule(&a);
            crate::video_highlights::schedule(&a);
            crate::video_prep::schedule(&a);
            crate::places_online::wake();
        }
        if let Some(l) = last().as_mut().filter(|l| l.library_id == id) {
            l.outcomes = result.outcomes.clone();
        }
        let _ = a.emit(REPAIR_IMPORTED, &result);
        a.state::<AppState>().changes.changed(&a, true);
        let mut finished = check_into_last(&a, &lib, &id, opened.as_ref(), only_assets.as_deref(), cancel);
        finished.outcomes = OutcomeCounts::of(&result.outcomes);
        let _ = a.emit(REPAIR_FINISHED, finished);
    });
    Ok(())
}

/// How each item of the last re-import ended, in the order they were done.
pub fn outcomes() -> Vec<RepairOutcomeRow> {
    last().as_ref().map(|l| l.outcomes.clone()).unwrap_or_default()
}

pub fn cancel(state: &AppState) -> AppResult<()> {
    let lib = state.library()?;
    state.jobs.cancel(&job_key(&lib.manifest.id));
    Ok(())
}

/// The next `limit` items of one class after `after` (an item `id`). Ids ascend in the result, so
/// a page never repeats or skips even while the screen scrolls.
fn page_of(items: &[RepairItem], status: RepairClass, after: Option<i64>, limit: u32) -> RepairItemsPage {
    let limit = limit.clamp(1, 500) as usize;
    let mut mine = items.iter().filter(|i| i.status == status && after.is_none_or(|a| i.id > a));
    let page: Vec<RepairItem> = mine.by_ref().take(limit).cloned().collect();
    let next_cursor = if mine.next().is_some() { page.last().map(|i| i.id) } else { None };
    RepairItemsPage { items: page, next_cursor }
}

pub fn items(state: &AppState, status: RepairClass, after: Option<i64>, limit: u32) -> AppResult<RepairItemsPage> {
    let lib = state.library()?;
    let held = last();
    let Some(l) = held.as_ref().filter(|l| l.library_id == lib.manifest.id) else {
        return Ok(RepairItemsPage { items: vec![], next_cursor: None });
    };
    Ok(page_of(&l.items, status, after, limit))
}

// ---------------------------------------------------------------- recovered (Round 22 §4)

/// A recovery is done: how many photos gave something and how many nothing.
pub const REPAIR_RECOVERED: &str = "repair://recovered";

/// A recovery waiting in `<library>/recovered/` for *Keep* or *Discard*.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RecoveredRow {
    #[ts(type = "number")]
    pub asset_id: i64,
    pub name: String,
    pub kind: RecoveredKind,
    pub percent: u8,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RecoverFinished {
    pub recovered: u32,
    /// Photos with nothing readable: their only way out is *Remove*.
    pub nothing: u32,
    pub cancelled: bool,
}

/// What *Keep* or its undo did, per photo.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RecoveredChanges {
    #[ts(type = "number[]")]
    pub done: Vec<i64>,
    pub failed: Vec<RecoveredFailure>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RecoveredFailure {
    #[ts(type = "number")]
    pub asset_id: i64,
    pub reason: String,
}

/// `<id>.json` beside a staged recovery (no table, §4): the figures and the photo's name. After
/// *Keep* it is renamed `<id>.kept.json`, so the undo can stage the recovery again.
#[derive(Serialize, Deserialize)]
struct StagedNote {
    name: String,
    #[serde(flatten)]
    recovered: Recovered,
}

const STAGED_EXTENSIONS: [&str; 2] = ["jpg", "mp4"];

fn recovered_dir(root: &Path) -> PathBuf {
    root.join("recovered")
}

/// Every path in `recovered/` is built from an asset id, never from a name a caller passed.
fn note_path(root: &Path, id: i64) -> PathBuf {
    recovered_dir(root).join(format!("{id}.json"))
}

fn kept_note_path(root: &Path, id: i64) -> PathBuf {
    recovered_dir(root).join(format!("{id}.kept.json"))
}

/// The staged recovery of photo `id`, if there is one.
pub fn staged_file(root: &Path, id: i64) -> Option<PathBuf> {
    STAGED_EXTENSIONS.iter().map(|e| recovered_dir(root).join(format!("{id}.{e}"))).find(|p| p.is_file())
}

fn read_note(path: &Path) -> Option<StagedNote> {
    serde_json::from_slice(&std::fs::read(path).ok()?).ok()
}

fn remove_staged(root: &Path, id: i64) {
    for e in STAGED_EXTENSIONS {
        let _ = std::fs::remove_file(recovered_dir(root).join(format!("{id}.{e}")));
    }
    let _ = std::fs::remove_file(note_path(root, id));
}

/// The ids among `ids` still flagged and not in Trash: a recovery is only ever kept in place of a
/// file that does not open, never over one that was repaired meanwhile.
fn still_flagged(lib: &Library, ids: &[i64]) -> AppResult<HashSet<i64>> {
    let flagged: Vec<i64> = lib.db.with_conn(|c| {
        let mut stmt = c.prepare("SELECT id FROM assets WHERE unreadable = 1 AND state = 0")?;
        let rows = stmt.query_map([], |r| r.get::<_, i64>(0))?.collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    })?;
    let wanted: HashSet<i64> = ids.iter().copied().collect();
    Ok(flagged.into_iter().filter(|id| wanted.contains(id)).collect())
}

/// The photos *Recover what's readable* works on: `ids`, or every flagged photo not in Trash,
/// as (id, file name, path under `originals/`). A photo too large to open is not damaged: skipped.
fn to_recover(lib: &Library, ids: Option<&[i64]>) -> AppResult<Vec<(i64, String, String)>> {
    let rows: Vec<(i64, String, String, String)> = lib.db.with_conn(|c| {
        let mut stmt = c.prepare("SELECT id, filename, relative_path, content_hash FROM assets WHERE unreadable = 1 AND state = 0 ORDER BY id")?;
        let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?.collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    })?;
    let wanted: Option<HashSet<i64>> = ids.map(|i| i.iter().copied().collect());
    let store = photos_media::DerivativeStore::new(&lib.root);
    Ok(rows
        .into_iter()
        .filter(|(id, ..)| wanted.as_ref().is_none_or(|w| w.contains(id)))
        .filter(|(.., hash)| crate::library_health::cause_of(&store, hash) != UnreadableCause::TooLarge)
        .map(|(id, name, rel, _)| (id, name, rel))
        .collect())
}

/// Recovers photo `id` into `recovered/<id>.<jpg|mp4>` with its note, replacing an earlier attempt.
/// `None` when nothing is readable; then nothing is staged.
pub fn recover_one(lib: &Library, id: i64, name: &str, rel: &str, ffmpeg: &photos_media::Ffmpeg, exiftool: Option<&photos_media::ExifTool>) -> Option<RecoveredRow> {
    remove_staged(&lib.root, id);
    let stem = recovered_dir(&lib.root).join(id.to_string());
    let (file, r) = photos_media::recover::recover(&lib.originals_dir().join(rel), name, &stem, ffmpeg, exiftool)?;
    let note = StagedNote { name: name.to_string(), recovered: r };
    let written = serde_json::to_vec(&note).ok().and_then(|json| std::fs::write(note_path(&lib.root, id), json).ok());
    if written.is_none() {
        let _ = std::fs::remove_file(file);
        return None;
    }
    Some(RecoveredRow { asset_id: id, name: note.name, kind: r.kind, percent: r.percent, width: r.width, height: r.height })
}

/// The staged recoveries of photos still flagged, by id.
pub fn recovered_rows(lib: &Library) -> AppResult<Vec<RecoveredRow>> {
    let Ok(entries) = std::fs::read_dir(recovered_dir(&lib.root)) else { return Ok(vec![]) };
    let mut rows: Vec<RecoveredRow> = entries
        .flatten()
        .filter_map(|e| e.file_name().to_str()?.strip_suffix(".json")?.parse::<i64>().ok())
        .filter(|&id| staged_file(&lib.root, id).is_some())
        .filter_map(|id| {
            let n = read_note(&note_path(&lib.root, id))?;
            Some(RecoveredRow { asset_id: id, name: n.name, kind: n.recovered.kind, percent: n.recovered.percent, width: n.recovered.width, height: n.recovered.height })
        })
        .collect();
    let ids: Vec<i64> = rows.iter().map(|r| r.asset_id).collect();
    let flagged = still_flagged(lib, &ids)?;
    rows.retain(|r| flagged.contains(&r.asset_id));
    rows.sort_by_key(|r| r.asset_id);
    Ok(rows)
}

/// *Discard*: the staged files go; the library is not touched.
pub fn discard(lib: &Library, ids: &[i64]) {
    for &id in ids {
        remove_staged(&lib.root, id);
    }
}

/// *Keep*: each staged recovery takes the damaged original's place (`photos_import`'s
/// `keep_recovered`, the original kept aside); its note becomes `<id>.kept.json` for the undo.
pub fn keep(lib: &Library, ids: &[i64], probe: repair::Probe) -> AppResult<RecoveredChanges> {
    let flagged = still_flagged(lib, ids)?;
    let mut out = RecoveredChanges::default();
    for &id in ids {
        let fail = |reason: &str| RecoveredFailure { asset_id: id, reason: reason.to_string() };
        if !flagged.contains(&id) {
            out.failed.push(fail("This photo is not damaged any more."));
            continue;
        }
        let (Some(note), Some(staged)) = (read_note(&note_path(&lib.root, id)), staged_file(&lib.root, id)) else {
            out.failed.push(fail("Nothing was recovered for this photo."));
            continue;
        };
        match repair::keep_recovered(lib, id, &staged, note.recovered.percent, probe) {
            Ok(()) => {
                let _ = std::fs::rename(note_path(&lib.root, id), kept_note_path(&lib.root, id));
                let _ = std::fs::remove_file(&staged);
                out.done.push(id);
            }
            Err(e) => out.failed.push(fail(&e.to_string())),
        }
    }
    Ok(out)
}

/// Undo of *Keep*: the damaged original returns, the recovery is staged again with its note.
/// Only a photo whose last replacement was a kept recovery (its `<id>.kept.json` exists).
pub fn undo_keep(lib: &Library, ids: &[i64]) -> RecoveredChanges {
    let mut out = RecoveredChanges::default();
    for &id in ids {
        let kept = kept_note_path(&lib.root, id);
        if !kept.is_file() {
            out.failed.push(RecoveredFailure { asset_id: id, reason: "There is no kept recovery to undo.".into() });
            continue;
        }
        match repair::undo_replace(lib, id) {
            Ok(_) => {
                let _ = std::fs::rename(&kept, note_path(&lib.root, id));
                out.done.push(id);
            }
            Err(e) => out.failed.push(RecoveredFailure { asset_id: id, reason: e.to_string() }),
        }
    }
    out
}

/// *Recover what's readable*: one job on the repair key, phase `Probing` with the photo's name.
/// Ends with `repair://recovered`. Needs FFmpeg: without it nothing can be decoded.
pub fn start_recover(app: &AppHandle, ids: Option<Vec<i64>>) -> AppResult<()> {
    let state = app.state::<AppState>();
    let lib = state.library()?;
    let key = job_key(&lib.manifest.id);
    if state.jobs.is_inflight(&key) {
        return Err(error("repair_running", "A check is already running."));
    }
    let Some(ffmpeg) = state.ffmpeg() else {
        return Err(error("ffmpeg_missing", "Recovering needs FFmpeg."));
    };
    let todo = to_recover(&lib, ids.as_deref())?;
    let a = app.clone();
    state.jobs.submit(&key, photos_jobs::Lane::Maintenance, move |cancel| {
        let exiftool = a.state::<AppState>().exiftool();
        let total = todo.len() as u64;
        let mut finished = RecoverFinished::default();
        tracing::info!(photos = total, "recovery started");
        for (done, (id, name, rel)) in todo.iter().enumerate() {
            if cancel.is_cancelled() {
                finished.cancelled = true;
                break;
            }
            let status = RepairStatus { phase: RepairPhase::Probing, done: done as u64, total, current: Some(name.clone()), ..idle() };
            let _ = a.emit(REPAIR_PROGRESS, status);
            match recover_one(&lib, *id, name, rel, &ffmpeg, exiftool.as_ref()) {
                Some(_) => finished.recovered += 1,
                None => finished.nothing += 1,
            }
        }
        tracing::info!(recovered = finished.recovered, nothing = finished.nothing, cancelled = finished.cancelled, "recovery finished");
        let _ = a.emit(REPAIR_PROGRESS, RepairStatus { phase: RepairPhase::Done, done: total, total, cancelled: finished.cancelled, ..idle() });
        let _ = a.emit(REPAIR_RECOVERED, finished);
    });
    Ok(())
}

/// *Keep* from the screen: then, as after a swap, the new file's metadata is read again and its
/// pictures are made.
pub fn keep_and_refresh(app: &AppHandle, ids: &[i64]) -> AppResult<RecoveredChanges> {
    let state = app.state::<AppState>();
    let lib = state.library()?;
    let ffmpeg = state.ffmpeg();
    let probe = |path: &Path, name: &str| probe_with(path, name, ffmpeg.as_ref());
    let changes = keep(&lib, ids, &probe)?;
    for &asset in &changes.done {
        let rel = lib.db.with_conn(|c| Ok(c.query_row("SELECT relative_path FROM assets WHERE id = ?1", [asset], |r| r.get::<_, String>(0))?));
        if let Ok(rel) = rel {
            crate::maintenance::reread_metadata(&lib.db, asset, &lib.originals_dir().join(rel), ffmpeg.as_ref());
        }
    }
    crate::derivatives::retry_unreadable(app, &changes.done);
    if !changes.done.is_empty() {
        state.changes.changed(app, true);
    }
    Ok(changes)
}

pub fn undo_keep_and_refresh(app: &AppHandle, ids: &[i64]) -> AppResult<RecoveredChanges> {
    let state = app.state::<AppState>();
    let lib = state.library()?;
    let changes = undo_keep(&lib, ids);
    if !changes.done.is_empty() {
        state.changes.changed(app, true);
    }
    Ok(changes)
}

#[cfg(test)]
mod recovered_tests {
    use super::*;
    use photos_core::assets::{insert_assets, NewAsset};

    fn jpeg(w: u32, h: u32) -> Vec<u8> {
        let mut out = Vec::new();
        image::DynamicImage::new_rgb8(w, h).write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Jpeg).unwrap();
        out
    }

    /// A library with photos `a.jpg` (flagged) and `b.jpg` (fine), their originals on disk.
    fn library() -> (tempfile::TempDir, Library, i64, i64) {
        let tmp = tempfile::tempdir().unwrap();
        let lib = Library::create(&tmp.path().join("lib")).unwrap();
        let photo = |name: &str, hash: &str| NewAsset {
            content_hash: hash.into(),
            relative_path: format!("2026/{name}"),
            filename: name.into(),
            extension: "jpg".into(),
            media_type: 0,
            byte_size: 3,
            captured_at: Some(100),
            imported_at: 1,
            ..Default::default()
        };
        let ids = lib.db.with_conn(|c| insert_assets(c, &[photo("a.jpg", "aa"), photo("b.jpg", "bb")])).unwrap();
        lib.db.with_conn(|c| photos_core::unreadable::set_unreadable(c, ids[0], true)).unwrap();
        std::fs::create_dir_all(lib.originals_dir().join("2026")).unwrap();
        std::fs::write(lib.originals_dir().join("2026/a.jpg"), b"BAD").unwrap();
        std::fs::write(lib.originals_dir().join("2026/b.jpg"), jpeg(8, 8)).unwrap();
        (tmp, lib, ids[0], ids[1])
    }

    fn stage(lib: &Library, id: i64, name: &str) {
        std::fs::create_dir_all(recovered_dir(&lib.root)).unwrap();
        std::fs::write(recovered_dir(&lib.root).join(format!("{id}.jpg")), jpeg(8, 6)).unwrap();
        let note = StagedNote { name: name.into(), recovered: Recovered { kind: RecoveredKind::Partial, percent: 62, width: 8, height: 6 } };
        std::fs::write(note_path(&lib.root, id), serde_json::to_vec(&note).unwrap()).unwrap();
    }

    #[test]
    fn only_flagged_photos_list_their_recovery() {
        let (_tmp, lib, a, b) = library();
        stage(&lib, a, "a.jpg");
        stage(&lib, b, "b.jpg");
        let rows = recovered_rows(&lib).unwrap();
        assert_eq!(rows, [RecoveredRow { asset_id: a, name: "a.jpg".into(), kind: RecoveredKind::Partial, percent: 62, width: 8, height: 6 }]);
    }

    #[test]
    fn discard_removes_only_the_staged_files() {
        let (_tmp, lib, a, _) = library();
        stage(&lib, a, "a.jpg");
        discard(&lib, &[a]);
        assert!(staged_file(&lib.root, a).is_none() && !note_path(&lib.root, a).exists());
        assert_eq!(std::fs::read(lib.originals_dir().join("2026/a.jpg")).unwrap(), b"BAD");
        assert!(recovered_rows(&lib).unwrap().is_empty());
    }

    #[test]
    fn keep_then_undo_stages_the_recovery_again() {
        let (_tmp, lib, a, _) = library();
        stage(&lib, a, "a.jpg");
        let ok = |_: &Path, _: &str| Ok(());
        let kept = keep(&lib, &[a], &ok).unwrap();
        assert_eq!((kept.done.as_slice(), kept.failed.len()), ([a].as_slice(), 0));
        assert!(staged_file(&lib.root, a).is_none() && kept_note_path(&lib.root, a).is_file());
        let pct: Option<i64> = lib.db.with_conn(|c| Ok(c.query_row("SELECT recovered_pct FROM assets WHERE id = ?1", [a], |r| r.get(0))?)).unwrap();
        assert_eq!(pct, Some(62));
        assert!(recovered_rows(&lib).unwrap().is_empty());

        let undone = undo_keep(&lib, &[a]);
        assert_eq!(undone.done, [a]);
        assert_eq!(std::fs::read(lib.originals_dir().join("2026/a.jpg")).unwrap(), b"BAD");
        assert_eq!(recovered_rows(&lib).unwrap().len(), 1);
        // Nothing kept any more: a second undo refuses.
        assert_eq!(undo_keep(&lib, &[a]).failed.len(), 1);
    }

    #[test]
    fn keep_refuses_a_photo_that_opens_and_one_with_nothing_staged() {
        let (_tmp, lib, a, b) = library();
        stage(&lib, b, "b.jpg");
        let ok = |_: &Path, _: &str| Ok(());
        let out = keep(&lib, &[a, b], &ok).unwrap();
        assert!(out.done.is_empty());
        assert_eq!(out.failed.iter().map(|f| f.asset_id).collect::<Vec<_>>(), [a, b]);
        assert!(std::fs::read(lib.originals_dir().join("2026/b.jpg")).unwrap().starts_with(&[0xFF, 0xD8]));
    }

    #[test]
    fn a_too_large_photo_is_not_recovered() {
        let (_tmp, lib, a, b) = library();
        lib.db.with_conn(|c| photos_core::unreadable::set_unreadable(c, b, true)).unwrap();
        photos_media::DerivativeStore::new(&lib.root).record_failure("bb", "26.1.0-alpha", "Memory limit exceeded");
        assert_eq!(to_recover(&lib, None).unwrap().iter().map(|r| r.0).collect::<Vec<_>>(), [a]);
        assert!(to_recover(&lib, Some(&[a + 100])).unwrap().is_empty());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use photos_import::macdisk::fs::MacFsKind;

    #[test]
    fn a_too_large_error_counts_as_a_good_file() {
        assert_eq!(accept("Memory limit exceeded".into()), Ok(()));
        assert_eq!(accept("Invalid data found when processing input".into()), Err("Invalid data found when processing input".into()));
    }

    #[test]
    fn the_probe_judges_the_bytes_by_the_files_real_name() {
        let dir = tempfile::tempdir().unwrap();
        // The name decides the kind even when the path ends `.part`.
        let good = dir.path().join("a.png.repair.part");
        let mut png = Vec::new();
        image::DynamicImage::new_rgb8(4, 4).write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Jpeg).unwrap();
        std::fs::write(&good, &png).unwrap();
        assert_eq!(probe_with(&good, "a.jpg", None), Ok(()));
        let bad = dir.path().join("b.jpg.repair.part");
        std::fs::write(&bad, b"BAD bytes that are not a picture").unwrap();
        assert!(probe_with(&bad, "b.jpg", None).is_err());
        // What cannot be judged passes: a HEIC without FFmpeg, a kind Lumen does not decode.
        assert_eq!(probe_with(&bad, "c.heic", None), Ok(()));
        assert_eq!(probe_with(&bad, "notes.txt", None), Ok(()));
    }

    #[test]
    fn outcomes_are_counted_per_kind() {
        let row = |outcome| RepairOutcomeRow { name: "a".into(), dir: String::new(), outcome, reason: None, asset_id: None };
        let c = OutcomeCounts::of(&[row(RepairOutcome::Repaired), row(RepairOutcome::Repaired), row(RepairOutcome::SameFile), row(RepairOutcome::Failed)]);
        assert_eq!((c.repaired, c.same_file, c.failed, c.imported), (2, 1, 1, 0));
    }

    fn volume(mount: &str, label: &str, removable: bool) -> VolumeInfo {
        VolumeInfo { mount: mount.into(), label: label.into(), key: "k".into(), total_items: 3, new_items: 1, removable }
    }

    fn phone(udid: &str, paired: bool) -> IphoneInfo {
        IphoneInfo { udid: udid.into(), name: "Julian's iPhone".into(), model: Some("iPhone 15".into()), paired }
    }

    fn disk(id: &str) -> MacDiskInfo {
        MacDiskInfo { id: id.into(), name: "Backup".into(), fs: MacFsKind::Hfs, total_bytes: 2_000_000_000_000, free_bytes: None, libraries: vec![] }
    }

    #[test]
    fn the_source_list_has_volumes_then_phones_then_mac_disks() {
        let list = map_sources(&[volume("E:\\", "CARD", true), volume("D:\\", "Data", false)], &[phone("u1", true), phone("u2", false)], &[disk("disk3-part2")]);
        let kinds: Vec<_> = list.iter().map(|s| s.source_ref.kind).collect();
        assert_eq!(
            kinds,
            [RepairSourceKind::Volume, RepairSourceKind::Volume, RepairSourceKind::Iphone, RepairSourceKind::Iphone, RepairSourceKind::MacDisk]
        );
        assert!(list[0].removable && !list[1].removable);
        assert_eq!(list[0].source_ref.id, "E:\\");
    }

    #[test]
    fn an_unpaired_iphone_is_listed_but_not_ready() {
        let list = map_sources(&[], &[phone("u1", true), phone("u2", false)], &[]);
        assert!(list[0].ready);
        assert_eq!(list[0].detail, "iPhone 15");
        assert!(!list[1].ready);
        assert_eq!(list[1].detail, "Unlock the phone and tap Trust");
    }

    #[test]
    fn a_mac_disk_names_its_size_and_is_ready() {
        let list = map_sources(&[], &[], &[disk("disk3-part2")]);
        assert_eq!(list[0].source_ref, RepairSourceRef { kind: RepairSourceKind::MacDisk, id: "disk3-part2".into() });
        assert_eq!(list[0].detail, "Apple-formatted · 2.0 TB");
        assert!(list[0].ready);
    }

    #[test]
    fn nothing_plugged_in_lists_nothing() {
        assert!(map_sources(&[], &[], &[]).is_empty());
    }

    fn item(id: i64, status: RepairClass) -> RepairItem {
        RepairItem { id, name: format!("{id}.jpg"), dir: String::new(), status, asset_id: None, byte_size: 1 }
    }

    #[test]
    fn pages_hold_one_class_and_walk_by_id_without_repeating_or_skipping() {
        let all: Vec<RepairItem> = (1..=7).map(|i| item(i, if i % 2 == 0 { RepairClass::Damaged } else { RepairClass::New })).collect();
        let first = page_of(&all, RepairClass::New, None, 3);
        assert_eq!(first.items.iter().map(|i| i.id).collect::<Vec<_>>(), [1, 3, 5]);
        assert_eq!(first.next_cursor, Some(5));
        let second = page_of(&all, RepairClass::New, first.next_cursor, 3);
        assert_eq!(second.items.iter().map(|i| i.id).collect::<Vec<_>>(), [7]);
        assert_eq!(second.next_cursor, None);
        assert!(page_of(&all, RepairClass::InTrash, None, 3).items.is_empty());
    }
}
