//! The ingest pipeline (architecture spec §6.1): discover → hash+stage → dedup →
//! metadata → place → device → insert → hand off for derivatives.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use photos_core::batches::{self, BATCH_CANCELLED, BATCH_COMPLETE, BATCH_FAILED, BATCH_INTERRUPTED};
use photos_core::seen::{self, SeenItem};
use photos_core::sidecars;
use photos_core::{assets, devices, import_journal, now_unix, Library};
use photos_jobs::CancelToken;
use photos_media::{hdr, kind_for_path, media_type_code, read_metadata_with, Ffmpeg, MediaKind, Metadata};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::source::{ImportSource, SourceItem};
use crate::ImportError;

#[derive(Debug, Clone)]
pub struct ImportOptions {
    pub skip_duplicates: bool,
    /// When present, videos are probed at ingest (size, duration, creation time).
    pub ffmpeg: Option<Ffmpeg>,
    /// `import_seen.source_key` for a card or phone: each file is remembered as seen when it
    /// is in the library (imported or already there), never merely because it was listed,
    /// so a file that failed, or that an unplugged device never sent, is offered again.
    pub seen_key: Option<String>,
    /// When present, the import stops before the library's drive would drop under
    /// `budget::IMPORT_FREE_BUFFER` (round 3).
    pub free_space: Option<crate::budget::FreeSpace>,
    /// Tests only: runs on each staged copy just before it is read back and verified, so a
    /// test can damage a copy the way a failing disk would.
    #[doc(hidden)]
    pub before_verify: Option<fn(&SourceItem, &Path)>,
}

impl Default for ImportOptions {
    fn default() -> Self {
        Self { skip_duplicates: true, ffmpeg: None, seen_key: None, free_space: None, before_verify: None }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ImportProgress {
    #[ts(type = "number")]
    pub batch_id: i64,
    #[ts(type = "number")]
    pub done: u64,
    #[ts(type = "number")]
    pub total: u64,
    #[ts(type = "number")]
    pub imported: u64,
    #[ts(type = "number")]
    pub skipped: u64,
    #[ts(type = "number")]
    pub failed: u64,
    pub current: Option<String>,
    /// Bytes read from the source so far, and in all (round 3).
    #[ts(type = "number")]
    pub bytes_done: u64,
    #[ts(type = "number")]
    pub bytes_total: u64,
    /// Time left, once there is enough to say (see `eta::Eta`).
    #[ts(type = "number | null")]
    pub eta_ms: Option<u64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ImportSummary {
    #[ts(type = "number")]
    pub batch_id: i64,
    #[ts(type = "number")]
    pub imported: u64,
    #[ts(type = "number")]
    pub skipped: u64,
    #[ts(type = "number")]
    pub failed: u64,
    pub cancelled: bool,
    /// The source went away part-way (round 3); what came in is kept.
    pub interrupted: bool,
    /// What was imported from, as the history names it ("Julian's iPhone").
    pub label: String,
    /// How many files of the source were not imported and are listed (`import_skips`):
    /// duplicates, files of a type Lumen does not take, files that could not be read.
    #[ts(type = "number")]
    pub skip_records: u64,
}

/// A source with thousands of foreign files (a project folder) lists the first of them.
const MAX_LEFTOVERS: usize = 2000;

/// What the derivative pass needs for one freshly catalogued file.
#[derive(Debug, Clone)]
pub struct Ingested {
    pub asset_id: i64,
    pub content_hash: String,
    pub original: PathBuf,
    pub kind: MediaKind,
    pub orientation: u16,
    /// What EXIF recorded as the stored size, if anything. Compared with the size the
    /// decoder actually returns to tell whether the pixels were already turned.
    pub exif_size: Option<(u32, u32)>,
    /// `photos_media::hdr::HdrKind::code`
    pub hdr: i64,
}

const HASH_CHUNK: usize = 32;

/// Threads an import hashes and copies with: half the cores, so the rest of the app stays
/// quick while it runs (round 3).
pub fn import_threads(cores: usize) -> usize {
    (cores / 2).max(1)
}

/// The import's own pool (rayon's global pool is shared with rendering and layout).
fn import_pool() -> &'static rayon::ThreadPool {
    static POOL: std::sync::OnceLock<rayon::ThreadPool> = std::sync::OnceLock::new();
    POOL.get_or_init(|| {
        let cores = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(2);
        rayon::ThreadPoolBuilder::new()
            .num_threads(import_threads(cores))
            .thread_name(|i| format!("lumen-import-hash-{i}"))
            .build()
            .expect("import thread pool")
    })
}
const INSERT_BATCH: usize = 50;
const PROGRESS_EVERY: Duration = Duration::from_millis(100);

/// What an import tells the history when its source disappears, once rather than per file.
pub const DISCONNECTED: &str = "The device was disconnected. Plug it back in and import again to finish.";

/// A catalogued file waiting for its row: `journal` names its placement in `originals/`.
struct Pending {
    row: assets::NewAsset,
    staged: Staged,
    meta: Metadata,
    kind: MediaKind,
    journal: i64,
}

/// A staged file: hashed and copied into `.incoming`, not yet catalogued.
struct Staged {
    item: SourceItem,
    hash: String,
    size: u64,
    tmp: PathBuf,
}

/// Creates the batch row, then runs it. Callers that need the batch id before the
/// work starts (to report progress against it) use `run_batch` directly.
pub fn run_import(
    lib: &Library,
    source: &dyn ImportSource,
    opts: &ImportOptions,
    cancel: &CancelToken,
    on_progress: &mut dyn FnMut(&ImportProgress),
    on_ingested: &mut dyn FnMut(&Ingested),
) -> Result<ImportSummary, ImportError> {
    let batch_id = lib.db.with_conn(|c| {
        batches::start_batch(c, source.kind(), &source.label(), source_path_label(source).as_deref(), now_unix())
    })?;
    run_batch(lib, source, batch_id, opts, cancel, on_progress, on_ingested)
}

/// Runs an already-created batch to completion (or cancellation).
pub fn run_batch(
    lib: &Library,
    source: &dyn ImportSource,
    batch_id: i64,
    opts: &ImportOptions,
    cancel: &CancelToken,
    on_progress: &mut dyn FnMut(&ImportProgress),
    on_ingested: &mut dyn FnMut(&Ingested),
) -> Result<ImportSummary, ImportError> {
    let items = match source.enumerate() {
        Ok(items) => items,
        Err(e) => {
            lib.db.with_conn(|c| {
                batches::record_failure(c, batch_id, &source.label(), &e.to_string())?;
                batches::finish_batch(c, batch_id, BATCH_FAILED, 0, 0, now_unix())
            })?;
            return Err(e);
        }
    };

    let incoming = lib.incoming_dir();
    std::fs::create_dir_all(&incoming)?;
    let mut progress = ImportProgress {
        batch_id,
        total: items.len() as u64,
        bytes_total: items.iter().map(|i| i.size).sum(),
        ..Default::default()
    };
    let mut eta = crate::eta::Eta::new(Instant::now());
    let mut last_emit = Instant::now() - PROGRESS_EVERY;
    let mut pending: Vec<Pending> = Vec::new();
    let (mut cancelled, mut interrupted) = (false, false);
    let mut late_duplicates: Vec<(String, String)> = Vec::new();
    let seen_key = opts.seen_key.as_deref();

    for chunk in items.chunks(HASH_CHUNK) {
        if cancel.is_cancelled() {
            cancelled = true;
            break;
        }
        // Room first: only as much of this chunk as keeps the buffer free, and then stop.
        let mut chunk = chunk;
        let mut out_of_room = false;
        if let Some(free) = opts.free_space.as_ref().and_then(|f| f.free()) {
            let allowed = crate::budget::budget_for(free);
            let (mut sum, mut n) = (0u64, 0usize);
            for it in chunk {
                if sum + it.size > allowed {
                    break;
                }
                sum += it.size;
                n += 1;
            }
            if n < chunk.len() {
                chunk = &chunk[..n];
                out_of_room = true;
            }
        }
        // Hash and stage in parallel: the source is read exactly once per file.
        let staged: Vec<Result<Staged, (SourceItem, ImportError)>> = import_pool().install(|| {
            chunk
                .par_iter()
                .map(|item| {
                    let tmp = incoming.join(format!("{}.part", uid(item)));
                    let result = source
                        .open(item)
                        .and_then(|reader| hash_and_stage(reader, &tmp))
                        .map(|(hash, size)| Staged { item: item.clone(), hash, size, tmp: tmp.clone() });
                    result.map_err(|e| {
                        let _ = std::fs::remove_file(&tmp);
                        (item.clone(), e)
                    })
                })
                .collect()
        });

        // A file that fails with the device gone is not a failure of that file: the rest of
        // this chunk failed the same way, and the import stops here as interrupted.
        let mut gone = false;
        for result in staged {
            progress.done += 1;
            let size = match &result {
                Ok(s) => s.item.size,
                Err((item, _)) => item.size,
            };
            progress.bytes_done += size;
            eta.record(size, Instant::now());
            match result {
                Err((item, e)) => {
                    if gone || !source.is_connected() {
                        gone = true;
                        continue;
                    }
                    progress.failed += 1;
                    lib.db.with_conn(|c| batches::record_failure(c, batch_id, &item.rel_path, &e.to_string()))?;
                    record_skip(lib, batch_id, &item.rel_path, sidecars::SKIP_UNREADABLE, None);
                }
                Ok(staged) => {
                    progress.current = Some(staged.item.file_name.clone());
                    let item = staged.item.clone();
                    let hash = staged.hash.clone();
                    match catalogue(lib, batch_id, opts, staged, &mut pending) {
                        Ok(Catalogued::Queued) => progress.imported += 1,
                        Ok(Catalogued::Duplicate(existing)) => {
                            progress.skipped += 1;
                            // Already in the library: nothing left to bring in from here.
                            mark_seen(lib, seen_key, &item);
                            // A duplicate still takes what the source knows about it.
                            if let Some(id) = existing {
                                apply_extras(lib, source, id, &item);
                                crate::sidecar::marks_for_duplicate(lib, source, id, &item);
                            }
                            match existing {
                                Some(_) => record_skip(lib, batch_id, &item.rel_path, sidecars::SKIP_DUPLICATE, existing),
                                // A copy of a file queued in this batch: its row exists after the flush.
                                None => late_duplicates.push((item.rel_path.clone(), hash)),
                            }
                        }
                        Err((rel, e)) => {
                            progress.failed += 1;
                            lib.db.with_conn(|c| batches::record_failure(c, batch_id, &rel, &e.to_string()))?;
                            let reason = if matches!(e, ImportError::Verify(_)) { sidecars::SKIP_FAILED_VERIFY } else { sidecars::SKIP_UNREADABLE };
                            record_skip(lib, batch_id, &rel, reason, None);
                        }
                    }
                }
            }
            if pending.len() >= INSERT_BATCH {
                flush(lib, source, &mut pending, on_ingested, seen_key)?;
            }
            if last_emit.elapsed() >= PROGRESS_EVERY {
                progress.eta_ms = eta.remaining_ms(progress.bytes_total.saturating_sub(progress.bytes_done), Instant::now());
                on_progress(&progress);
                last_emit = Instant::now();
            }
        }
        if gone {
            interrupted = true;
            progress.failed += 1;
            lib.db.with_conn(|c| batches::record_failure(c, batch_id, &source.label(), DISCONNECTED))?;
            break;
        }
        if out_of_room {
            interrupted = true;
            progress.failed += 1;
            lib.db.with_conn(|c| batches::record_failure(c, batch_id, &source.label(), crate::budget::OUT_OF_ROOM))?;
            break;
        }
    }
    flush(lib, source, &mut pending, on_ingested, seen_key)?;
    progress.current = None;
    on_progress(&progress);
    for (path, hash) in late_duplicates {
        let twin = lib.db.with_conn(|c| assets::find_by_hash(c, &hash)).ok().flatten();
        record_skip(lib, batch_id, &path, sidecars::SKIP_DUPLICATE, twin);
    }
    // What the source holds that was not even offered: foreign files, sidecars with no photo.
    if !cancelled && !interrupted {
        for (path, reason) in source.leftovers().into_iter().take(MAX_LEFTOVERS) {
            record_skip(lib, batch_id, &path, reason, None);
        }
    }

    let state = if cancelled {
        BATCH_CANCELLED
    } else if interrupted {
        BATCH_INTERRUPTED
    } else {
        BATCH_COMPLETE
    };
    lib.db.with_conn(|c| {
        // Live Photos arrive as a still plus a MOV; pair them before the batch closes.
        match assets::pair_live_photos(c, batch_id) {
            Ok(0) => {}
            Ok(n) => tracing::info!(batch_id, n, "paired live photos"),
            Err(e) => tracing::warn!(batch_id, "live photo pairing failed: {e}"),
        }
        batches::finish_batch(c, batch_id, state, progress.imported as i64, progress.skipped as i64, now_unix())
    })?;
    let _ = std::fs::remove_dir(&incoming);
    let skip_records = lib.db.with_conn(|c| sidecars::skips_of(c, batch_id)).map_or(0, |s| s.len() as u64);
    Ok(ImportSummary {
        skip_records,
        batch_id,
        imported: progress.imported,
        skipped: progress.skipped,
        failed: progress.failed,
        cancelled,
        interrupted,
        label: source.label(),
    })
}

/// Lists a file that was not imported. A failure costs a line in the list, never a file.
fn record_skip(lib: &Library, batch_id: i64, path: &str, reason: &str, asset_id: Option<i64>) {
    if let Err(e) = lib.db.with_conn(|c| sidecars::record_skip(c, batch_id, path, reason, asset_id, now_unix())) {
        tracing::warn!(file = %path, "could not list the skipped file: {e}");
    }
}

/// Remembers `item` as seen on the source, now that it is in the library. A failure costs a
/// file being offered again, never a file, so it is logged.
fn mark_seen(lib: &Library, key: Option<&str>, item: &SourceItem) {
    let Some(key) = key else { return };
    let seen = [SeenItem { rel_path: &item.rel_path, size: item.size, mtime: item.modified_at }];
    if let Err(e) = lib.db.with_conn(|c| seen::mark_seen(c, key, &seen, now_unix())) {
        tracing::warn!(file = %item.rel_path, "could not remember the file as imported: {e}");
    }
}

fn source_path_label(source: &dyn ImportSource) -> Option<String> {
    // Only folder sources carry a path; other kinds report their label.
    if source.kind() == batches::SOURCE_FOLDER {
        Some(source.label())
    } else {
        None
    }
}

fn uid(item: &SourceItem) -> String {
    let mut h = blake3::Hasher::new();
    h.update(item.rel_path.as_bytes());
    h.update(&item.size.to_le_bytes());
    h.finalize().to_hex()[..16].to_string()
}

/// Streams the source once: BLAKE3 over 1 MB chunks while writing the staging copy.
/// The catalogue's key for a file's bytes, exactly as an import computes it.
pub fn content_hash(path: &Path) -> Result<String, ImportError> {
    let mut hasher = blake3::Hasher::new();
    hasher.update_reader(std::fs::File::open(path)?)?;
    Ok(hasher.finalize().to_hex().to_string())
}

pub(crate) fn hash_and_stage(reader: Box<dyn Read + Send>, staging: &Path) -> Result<(String, u64), ImportError> {
    hash_and_stage_with(reader, staging, &mut |_| {})
}

/// [`hash_and_stage`], telling `on_bytes` how many bytes are staged so far after each chunk
/// (Round 22 B2: a large file's progress in Repair).
pub(crate) fn hash_and_stage_with(mut reader: Box<dyn Read + Send>, staging: &Path, on_bytes: &mut dyn FnMut(u64)) -> Result<(String, u64), ImportError> {
    let mut hasher = blake3::Hasher::new();
    let mut out = std::io::BufWriter::new(std::fs::File::create(staging)?);
    let mut buf = vec![0u8; 1 << 20];
    let mut total = 0u64;
    loop {
        let n = reader.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        out.write_all(&buf[..n])?;
        total += n as u64;
        on_bytes(total);
    }
    out.flush()?;
    out.get_ref().sync_all()?;
    Ok((hasher.finalize().to_hex().to_string(), total))
}

enum Catalogued {
    Queued,
    /// Skipped: the content is already catalogued (`Some(id)`) or queued in this batch.
    Duplicate(Option<i64>),
}

/// Writes the source's extras for `item` onto `asset_id`. A failure costs the extras, never
/// the photo, so it is logged rather than recorded as a failed file.
fn apply_extras(lib: &Library, source: &dyn ImportSource, asset_id: i64, item: &SourceItem) {
    let Some(x) = source.extras(item) else { return };
    if let Err(e) = lib.db.with_conn(|c| photos_core::extras::apply(c, asset_id, &x, now_unix())) {
        tracing::warn!(file = %item.rel_path, "could not apply the source's extras: {e}");
    }
}

/// Dedup, read metadata, place the file, resolve the device, queue the row.
fn catalogue(
    lib: &Library,
    batch_id: i64,
    opts: &ImportOptions,
    staged: Staged,
    pending: &mut Vec<Pending>,
) -> Result<Catalogued, (String, ImportError)> {
    let rel = staged.item.rel_path.clone();
    let wrap = |e: ImportError| (rel.clone(), e);

    let existing = lib.db.with_conn(|c| assets::find_by_hash(c, &staged.hash)).map_err(|e| wrap(e.into()))?;
    let dup = existing.is_some() || pending.iter().any(|p| p.staged.hash == staged.hash);
    if dup && opts.skip_duplicates {
        let _ = std::fs::remove_file(&staged.tmp);
        return Ok(Catalogued::Duplicate(existing));
    }
    // Verify after copy (Round 19 §13): the copy is read back from the disk before anything
    // names it. Duplicates are skipped above, so a re-import of a card reads nothing twice.
    if let Some(hook) = opts.before_verify {
        hook(&staged.item, &staged.tmp);
    }
    if let Err(e) = crate::verify::verify_staged(&staged.tmp, &staged.hash) {
        let _ = std::fs::remove_file(&staged.tmp);
        return Err(wrap(e));
    }

    let kind = kind_for_path(Path::new(&staged.item.file_name));
    // A HEIF saved under another image's name (`IMG_1855.JPG`) is read as what it is (Round 8A).
    let heif_named_otherwise = kind == MediaKind::Image && photos_media::is_heif_file(&staged.tmp);
    let kind = if heif_named_otherwise { MediaKind::Heic } else { kind };
    let meta = read_metadata_with(&staged.tmp, kind, opts.ffmpeg.as_ref());
    let now = now_unix();
    // Screenshots, scans and exports carry no EXIF time; the file's modified time is a far
    // better timeline position than the moment of import.
    let captured_at = meta.captured_at.or(staged.item.modified_at);
    // Which tag the date came from, so the re-dating pass knows this row is already read.
    let capture_source = meta
        .capture_source
        .map(|s| s.as_str().to_string())
        .or_else(|| captured_at.map(|_| photos_media::CaptureSource::FileTime.as_str().to_string()));
    let date_ts = captured_at.unwrap_or(now);
    let day_dir = chrono::DateTime::from_timestamp(date_ts, 0)
        .unwrap_or_default()
        .format("%Y/%m/%d")
        .to_string();
    let ext = if heif_named_otherwise { "heic".to_string() } else { photos_media::extension_of(Path::new(&staged.item.file_name)) };
    let relative_path = unique_relative_path(&lib.originals_dir(), &day_dir, &staged.hash, &staged.item.file_name);
    let final_path = lib.originals_dir().join(&relative_path);
    if let Some(p) = final_path.parent() {
        std::fs::create_dir_all(p).map_err(|e| wrap(e.into()))?;
    }
    // Journal first: a crash between the move and the row leaves a record naming exactly
    // this file, and the next open settles it (`recover`).
    let journal = lib
        .db
        .with_conn(|c| import_journal::record(c, batch_id, &staged.hash, &relative_path, now))
        .map_err(|e| wrap(e.into()))?;
    if let Err(e) = std::fs::rename(&staged.tmp, &final_path) {
        let _ = lib.db.with_conn(|c| import_journal::clear(c, &[journal]));
        return Err(wrap(e.into()));
    }

    let device_id = lib
        .db
        .with_conn(|c| devices::upsert_device(c, meta.make.as_deref(), meta.model.as_deref(), meta.body_serial.as_deref(), now))
        .map_err(|e| wrap(e.into()))?;

    let row = assets::NewAsset {
        content_hash: staged.hash.clone(),
        relative_path,
        filename: staged.item.file_name.clone(),
        extension: ext,
        media_type: media_type_code(kind),
        byte_size: staged.size as i64,
        width: meta.width.map(i64::from),
        height: meta.height.map(i64::from),
        orientation: meta.orientation as i64,
        captured_at,
        captured_at_utc: meta.captured_at.zip(meta.tz_offset_minutes).map(|(t, off)| t - off as i64 * 60),
        capture_source,
        tz_offset_minutes: meta.tz_offset_minutes.map(i64::from),
        imported_at: now,
        file_modified_at: staged.item.modified_at,
        file_created_at: staged.item.created_at,
        device_id,
        import_batch_id: Some(batch_id),
        gps_lat: meta.gps_lat,
        gps_lon: meta.gps_lon,
        gps_altitude: meta.gps_altitude,
        metadata_json: serde_json::to_string(&meta).ok(),
        duration_ms: meta.duration_ms,
        // Read from the staged copy, before it is moved into place.
        hdr: hdr::detect(&final_path, kind, meta.probe.as_ref()).code(),
    };
    pending.push(Pending { row, staged, meta, kind, journal });
    Ok(Catalogued::Queued)
}

fn unique_relative_path(originals: &Path, day_dir: &str, hash: &str, file_name: &str) -> String {
    let stem = Path::new(file_name).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "file".into());
    let ext = Path::new(file_name).extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
    let base = format!("{day_dir}/{}_{stem}", &hash[..8]);
    let mut candidate = format!("{base}{ext}");
    let mut n = 2;
    while originals.join(&candidate).exists() {
        candidate = format!("{base}_{n}{ext}");
        n += 1;
    }
    candidate
}

/// The folder part of a source path (`''` for a file at the root).
fn source_dir_of(rel_path: &str) -> &str {
    rel_path.rsplit_once('/').map_or("", |(dir, _)| dir)
}

fn flush(
    lib: &Library,
    source: &dyn ImportSource,
    pending: &mut Vec<Pending>,
    on_ingested: &mut dyn FnMut(&Ingested),
    seen_key: Option<&str>,
) -> Result<(), ImportError> {
    if pending.is_empty() {
        return Ok(());
    }
    let rows: Vec<assets::NewAsset> = pending.iter().map(|p| p.row.clone()).collect();
    let ids = lib.db.with_conn(|c| assets::insert_assets(c, &rows))?;
    // Where each file sat on its source, so a Live Photo pairs only inside one folder.
    lib.db.with_conn(|c| {
        for (id, p) in ids.iter().zip(pending.iter()) {
            assets::set_source_dir(c, *id, source_dir_of(&p.staged.item.rel_path))?;
        }
        Ok(())
    })?;
    // The rows exist: the placements are settled.
    let journal: Vec<i64> = pending.iter().map(|p| p.journal).collect();
    lib.db.with_conn(|c| import_journal::clear(c, &journal))?;
    for (id, Pending { row, staged, meta, kind, .. }) in ids.into_iter().zip(pending.drain(..)) {
        mark_seen(lib, seen_key, &staged.item);
        apply_extras(lib, source, id, &staged.item);
        crate::sidecar::bring_sidecars(lib, source, id, &staged.item, &row.relative_path);
        on_ingested(&Ingested {
            asset_id: id,
            content_hash: staged.hash,
            original: lib.originals_dir().join(&row.relative_path),
            kind,
            orientation: meta.orientation,
            exif_size: match (meta.width, meta.height) {
                (Some(w), Some(h)) if w > 0 && h > 0 => Some((w, h)),
                _ => None,
            },
            hdr: row.hdr,
        });
    }
    Ok(())
}

#[cfg(test)]
mod pool_tests {
    use super::*;

    /// Hashing and copying use at most half the cores, so the viewer, the grid and the
    /// Interactive lane keep the rest while an import runs (round 3).
    #[test]
    fn an_import_leaves_half_the_machine_to_everything_else() {
        assert_eq!(import_threads(24), 12);
        assert_eq!(import_threads(4), 2);
        assert_eq!(import_threads(1), 1);
        assert_eq!(import_threads(3), 1);
    }
}
