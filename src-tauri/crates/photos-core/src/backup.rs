//! Library backup (Round 5A): an incremental mirror on another folder or drive.
//!
//! What is copied: `originals/`, `trash/`, `exports/` and `music/` file by file, then
//! `library.json`, then a snapshot of the database (`VACUUM INTO`, one consistent copy of a
//! live WAL database, never a raw file copy). `derivatives/`, `tmp/` and `logs/` are made
//! again from those, so they are skipped. The backup is itself a library: opening it restores.
//!
//! Safety, in order of importance:
//! - Nothing is ever written inside the library. The destination is refused when it is the
//!   library, inside it or around it, or holds another library or someone else's files.
//! - A file is copied to `<name>.lumen-part`, read back and compared, given its source's
//!   modified time, and only then renamed: an interrupted run leaves no half file under a
//!   real name, and the next run picks up where it stopped.
//! - A file gone from the library is not deleted from the backup: it moves to
//!   `.lumen-backup/removed/<date>/` and is kept [`KEEP_REMOVED_DAYS`] days.
//! - The database is snapshotted first and put in place last, so every file it names was
//!   in the library when the copying began.

use std::collections::HashSet;
use std::fs::{self, File};
use std::hash::Hasher;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::db::Db;
use crate::{CoreError, Result};

/// Folders mirrored file by file.
pub const MIRRORED_DIRS: [&str; 4] = ["originals", "trash", "exports", "music"];
/// Single files mirrored after the folders.
pub const MIRRORED_FILES: [&str; 1] = ["library.json"];
/// The backup's own folder: its marker and the files retired from the mirror.
pub const MARKER_DIR: &str = ".lumen-backup";
const MARKER_FILE: &str = "backup.json";
const PART: &str = ".lumen-part";
/// How long a file gone from the library stays in the backup.
pub const KEEP_REMOVED_DAYS: i64 = 30;
/// Modified times closer than this are the same: FAT keeps them to 2 s.
const MTIME_SLACK: Duration = Duration::from_secs(2);

/// Written into every backup: whose it is, so opening it can tell a restore from a mistake.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct BackupMarker {
    pub library_id: String,
    pub library_name: String,
    pub source_path: String,
    #[ts(type = "number")]
    pub finished_at: i64,
}

/// How a run ended.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct BackupReport {
    #[ts(type = "number")]
    pub copied: u64,
    #[ts(type = "number")]
    pub copied_bytes: u64,
    /// Files already in the backup, as they are.
    #[ts(type = "number")]
    pub unchanged: u64,
    /// Files gone from the library, retired to `.lumen-backup/removed`.
    #[ts(type = "number")]
    pub retired: u64,
    /// Files that could not be copied (unreadable, or changed while copying): tried next time.
    pub failed: Vec<String>,
    pub cancelled: bool,
}

/// Where a run is, for the activity ring.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct BackupProgress {
    #[ts(type = "number")]
    pub done_bytes: u64,
    #[ts(type = "number")]
    pub total_bytes: u64,
    #[ts(type = "number")]
    pub done_files: u64,
    #[ts(type = "number")]
    pub total_files: u64,
}

/// The backup of the library at `library_root` inside the folder the user chose: a folder
/// named like the library's, so one drive can hold several.
pub fn target_for(library_root: &Path, chosen: &Path) -> PathBuf {
    chosen.join(library_root.file_name().unwrap_or_else(|| std::ffi::OsStr::new("Lumen Library")))
}

/// `p` made comparable as text: absolute through its nearest existing ancestor (a destination
/// need not exist yet), `/` between parts and one at the end (so `/a/lib` is not taken for
/// `/a/library`), and on Windows without the `\\?\` prefix and without case.
fn normal(p: &Path) -> String {
    let mut rest = Vec::new();
    let mut at = p.to_path_buf();
    let base = loop {
        if let Ok(c) = fs::canonicalize(&at) {
            break c;
        }
        match (at.file_name().map(|n| n.to_os_string()), at.parent()) {
            (Some(name), Some(parent)) => {
                rest.push(name);
                at = parent.to_path_buf();
            }
            _ => break at,
        }
    };
    let mut full = base;
    for name in rest.into_iter().rev() {
        full.push(name);
    }
    let mut s = full.to_string_lossy().into_owned();
    if cfg!(windows) {
        s = s.strip_prefix(r"\\?\").unwrap_or(&s).replace('\\', "/").to_lowercase();
    }
    if !s.ends_with('/') {
        s.push('/');
    }
    s
}

/// The marker in `target`, if it is a backup.
pub fn read_marker(target: &Path) -> Option<BackupMarker> {
    serde_json::from_slice(&fs::read(target.join(MARKER_DIR).join(MARKER_FILE)).ok()?).ok()
}

/// Refuses a destination that would put the library at risk or overwrite something else.
pub fn check_destination(library_root: &Path, library_id: &str, target: &Path) -> Result<()> {
    let (lib, dest) = (normal(library_root), normal(target));
    if dest.starts_with(&lib) || lib.starts_with(&dest) {
        return Err(CoreError::Invalid("The backup cannot go inside the library, or the library inside the backup. Choose another folder or drive.".into()));
    }
    if !target.exists() {
        return Ok(());
    }
    match read_marker(target) {
        Some(m) if m.library_id == library_id => Ok(()),
        Some(m) => Err(CoreError::Invalid(format!("That folder holds the backup of another library (“{}”). Choose another folder.", m.library_name))),
        None if fs::read_dir(target)?.next().is_none() => Ok(()),
        None => Err(CoreError::Invalid(format!("“{}” already holds other files. Choose an empty folder or a drive.", target.display()))),
    }
}

/// A file to copy, relative to the library.
#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub rel: PathBuf,
    pub size: u64,
}

/// What a run will do: copy these, retire those.
#[derive(Debug, Default, PartialEq)]
pub struct Plan {
    pub copy: Vec<Entry>,
    pub unchanged: u64,
    pub retire: Vec<PathBuf>,
}

impl Plan {
    pub fn bytes(&self) -> u64 {
        self.copy.iter().map(|e| e.size).sum()
    }
}

/// Every file under `root/dir`, relative to `root`, skipping unfinished copies.
fn walk(root: &Path, dir: &str, out: &mut Vec<PathBuf>) -> Result<()> {
    let mut stack = vec![root.join(dir)];
    while let Some(d) = stack.pop() {
        let entries = match fs::read_dir(&d) {
            Ok(e) => e,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => return Err(e.into()),
        };
        for e in entries {
            let e = e?;
            let t = e.file_type()?;
            if t.is_dir() {
                stack.push(e.path());
            } else if t.is_file() && !e.file_name().to_string_lossy().ends_with(PART) {
                out.push(e.path().strip_prefix(root).expect("under root").to_path_buf());
            }
        }
    }
    Ok(())
}

fn same(src: &fs::Metadata, dst: &fs::Metadata) -> bool {
    if src.len() != dst.len() {
        return false;
    }
    match (src.modified(), dst.modified()) {
        (Ok(a), Ok(b)) => a.duration_since(b).or_else(|_| b.duration_since(a)).map(|d| d <= MTIME_SLACK).unwrap_or(false),
        _ => false,
    }
}

/// Compares the library with the backup: what is new or changed, and what has gone.
pub fn plan(library_root: &Path, target: &Path) -> Result<Plan> {
    let mut plan = Plan::default();
    let mut in_library = Vec::new();
    for dir in MIRRORED_DIRS {
        walk(library_root, dir, &mut in_library)?;
    }
    in_library.extend(MIRRORED_FILES.iter().map(PathBuf::from).filter(|f| library_root.join(f).is_file()));
    for rel in &in_library {
        let src = fs::metadata(library_root.join(rel))?;
        match fs::metadata(target.join(rel)) {
            Ok(dst) if same(&src, &dst) => plan.unchanged += 1,
            _ => plan.copy.push(Entry { rel: rel.clone(), size: src.len() }),
        }
    }
    let keep: HashSet<&PathBuf> = in_library.iter().collect();
    let mut in_backup = Vec::new();
    for dir in MIRRORED_DIRS {
        walk(target, dir, &mut in_backup)?;
    }
    plan.retire = in_backup.into_iter().filter(|rel| !keep.contains(rel)).collect();
    plan.retire.sort();
    Ok(plan)
}

const CHUNK: usize = 1 << 20;

/// Copies `src` to `dst` through `dst.lumen-part`: hashed on the way, read back and compared,
/// given `src`'s modified time, then renamed into place.
pub fn copy_verified(src: &Path, dst: &Path, cancelled: &dyn Fn() -> bool) -> Result<u64> {
    if let Some(dir) = dst.parent() {
        fs::create_dir_all(dir)?;
    }
    let part = PathBuf::from(format!("{}{PART}", dst.display()));
    let result = (|| -> Result<u64> {
        let source_meta = fs::metadata(src)?;
        let mut from = File::open(src)?;
        let mut to = File::create(&part)?;
        let mut written = std::collections::hash_map::DefaultHasher::new();
        let mut buf = vec![0u8; CHUNK];
        let mut n_total = 0u64;
        loop {
            if cancelled() {
                return Err(CoreError::Invalid("cancelled".into()));
            }
            let n = from.read(&mut buf)?;
            if n == 0 {
                break;
            }
            written.write(&buf[..n]);
            to.write_all(&buf[..n])?;
            n_total += n as u64;
        }
        to.sync_all()?;
        drop(to);
        if n_total != source_meta.len() || fs::metadata(src)?.len() != n_total {
            return Err(CoreError::Invalid(format!("{} changed while it was copied", src.display())));
        }
        let mut read_back = std::collections::hash_map::DefaultHasher::new();
        let mut back = File::open(&part)?;
        loop {
            let n = back.read(&mut buf)?;
            if n == 0 {
                break;
            }
            read_back.write(&buf[..n]);
        }
        drop(back);
        if read_back.finish() != written.finish() {
            return Err(CoreError::Invalid(format!("the copy of {} did not read back the same", src.display())));
        }
        if let Ok(t) = source_meta.modified() {
            File::options().write(true).open(&part)?.set_modified(t)?;
        }
        fs::rename(&part, dst)?;
        Ok(n_total)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&part);
    }
    result
}

fn removed_dir(target: &Path) -> PathBuf {
    target.join(MARKER_DIR).join("removed")
}

/// Moves a file gone from the library out of the mirror, into today's removed folder.
pub fn retire(target: &Path, rel: &Path, today: chrono::NaiveDate) -> Result<()> {
    let to = removed_dir(target).join(today.format("%Y-%m-%d").to_string()).join(rel);
    if let Some(dir) = to.parent() {
        fs::create_dir_all(dir)?;
    }
    fs::rename(target.join(rel), to)?;
    Ok(())
}

/// Deletes the removed folders older than [`KEEP_REMOVED_DAYS`]. Only ever inside
/// `.lumen-backup/removed`, and only folders named as a date.
pub fn purge_removed(target: &Path, today: chrono::NaiveDate) -> Result<usize> {
    let mut n = 0;
    let Ok(days) = fs::read_dir(removed_dir(target)) else { return Ok(0) };
    for d in days.flatten() {
        let name = d.file_name().to_string_lossy().into_owned();
        let Ok(day) = chrono::NaiveDate::parse_from_str(&name, "%Y-%m-%d") else { continue };
        if (today - day).num_days() > KEEP_REMOVED_DAYS && d.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            fs::remove_dir_all(d.path())?;
            n += 1;
        }
    }
    Ok(n)
}

/// What a run needs to know about the library.
pub struct Source<'a> {
    pub root: &'a Path,
    pub id: &'a str,
    pub name: &'a str,
    pub db: &'a Db,
}

/// One backup run into `target` (from [`target_for`]). `free` is the destination's free space
/// when known: a run that cannot fit stops before copying anything.
pub fn run(source: &Source, target: &Path, free: Option<u64>, now: i64, cancelled: &dyn Fn() -> bool, progress: &mut dyn FnMut(BackupProgress)) -> Result<BackupReport> {
    check_destination(source.root, source.id, target)?;
    fs::create_dir_all(target.join(MARKER_DIR))?;
    // Claimed before anything is copied, so a run stopped halfway is resumed, not refused as
    // someone else's folder. `finished_at` stays the last finished run's.
    let started = BackupMarker {
        library_id: source.id.into(),
        library_name: source.name.into(),
        source_path: source.root.to_string_lossy().into_owned(),
        finished_at: read_marker(target).map(|m| m.finished_at).unwrap_or(0),
    };
    fs::write(target.join(MARKER_DIR).join(MARKER_FILE), serde_json::to_vec_pretty(&started)?)?;
    let today = chrono::DateTime::from_timestamp(now, 0).unwrap_or_default().date_naive();

    // 1. The database, first: every file it names is in the library from here on.
    let db_part = target.join(format!("library.db{PART}"));
    let _ = fs::remove_file(&db_part);
    let snapshot = db_part.to_string_lossy().into_owned();
    source.db.with_conn(|c| Ok(c.execute("VACUUM INTO ?1", [&snapshot])?))?;

    let result = (|| -> Result<BackupReport> {
        // 2. What to copy, and whether it fits.
        let plan = plan(source.root, target)?;
        let need = plan.bytes();
        if let Some(free) = free {
            if need > free {
                return Err(CoreError::Invalid(format!("The backup needs {} more, and the destination has {} free.", gb(need), gb(free))));
            }
        }
        let mut report = BackupReport { unchanged: plan.unchanged, ..Default::default() };
        let mut p = BackupProgress { done_bytes: 0, total_bytes: need, done_files: 0, total_files: plan.copy.len() as u64 };
        progress(p);

        // 3. The files.
        for e in &plan.copy {
            if cancelled() {
                report.cancelled = true;
                return Ok(report);
            }
            match copy_verified(&source.root.join(&e.rel), &target.join(&e.rel), cancelled) {
                Ok(n) => {
                    report.copied += 1;
                    report.copied_bytes += n;
                }
                Err(_) if cancelled() => {
                    report.cancelled = true;
                    return Ok(report);
                }
                Err(err) => {
                    tracing::warn!(file = %e.rel.display(), "backup: {err}");
                    report.failed.push(e.rel.to_string_lossy().into_owned());
                }
            }
            p.done_bytes += e.size;
            p.done_files += 1;
            progress(p);
        }

        // 4. What left the library leaves the mirror, kept aside for a while.
        for rel in &plan.retire {
            match retire(target, rel, today) {
                Ok(()) => report.retired += 1,
                Err(err) => tracing::warn!(file = %rel.display(), "backup: could not retire: {err}"),
            }
        }
        purge_removed(target, today)?;
        Ok(report)
    })();

    let report = match result {
        Ok(r) if !r.cancelled => r,
        other => {
            let _ = fs::remove_file(&db_part);
            return other;
        }
    };

    // 5. The database in place, last, without a stale WAL beside it, and the marker.
    for stale in ["library.db-wal", "library.db-shm"] {
        let _ = fs::remove_file(target.join(stale));
    }
    fs::rename(&db_part, target.join("library.db"))?;
    let marker = BackupMarker { library_id: source.id.into(), library_name: source.name.into(), source_path: source.root.to_string_lossy().into_owned(), finished_at: now };
    fs::write(target.join(MARKER_DIR).join(MARKER_FILE), serde_json::to_vec_pretty(&marker)?)?;
    Ok(report)
}

/// How often a backup runs by itself (when its destination is there).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum Schedule {
    #[default]
    Off,
    Daily,
    Weekly,
}

impl Schedule {
    fn every(self) -> Option<i64> {
        match self {
            Schedule::Off => None,
            Schedule::Daily => Some(86_400),
            Schedule::Weekly => Some(7 * 86_400),
        }
    }
}

/// One library's backup settings and its last run.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct BackupConfig {
    /// The folder or drive chosen; the backup is [`target_for`] inside it.
    pub destination: Option<String>,
    #[serde(default)]
    pub schedule: Schedule,
    /// When the last run that finished did (seconds).
    #[serde(default)]
    #[ts(type = "number | null")]
    pub last_success_at: Option<i64>,
    /// The last run, finished or not.
    #[serde(default)]
    pub last: Option<BackupRun>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct BackupRun {
    #[ts(type = "number")]
    pub at: i64,
    /// Why it stopped, when it did not finish (no space, destination missing, refused).
    pub error: Option<String>,
    pub report: Option<BackupReport>,
}

impl BackupConfig {
    /// Whether a scheduled run is due at `now` (the destination's presence is the caller's to check).
    pub fn due(&self, now: i64) -> bool {
        match (self.destination.as_ref(), self.schedule.every()) {
            (Some(_), Some(every)) => self.last_success_at.is_none_or(|t| now - t >= every),
            _ => false,
        }
    }
}

/// Every library's backup, by library id, kept in `backups.json` beside `libraries.json`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct BackupConfigs {
    pub libraries: std::collections::BTreeMap<String, BackupConfig>,
}

impl BackupConfigs {
    pub fn load(path: &Path) -> Self {
        fs::read(path).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
    }

    /// Written through a temporary file, so a crash never leaves half a file.
    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)?;
        }
        let tmp = path.with_extension("json.tmp");
        fs::write(&tmp, serde_json::to_vec_pretty(self)?)?;
        fs::rename(tmp, path)?;
        Ok(())
    }
}

fn gb(bytes: u64) -> String {
    format!("{:.1} GB", bytes as f64 / 1e9)
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_790_000_000; // 2026-09-21
    fn day(offset: i64) -> chrono::NaiveDate {
        chrono::DateTime::from_timestamp(NOW + offset * 86_400, 0).unwrap().date_naive()
    }

    /// A small library on disk: a database, two originals, an export and its manifest.
    fn library() -> (tempfile::TempDir, Db) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        for (rel, body) in [("originals/2026/01/aa_one.jpg", "one"), ("originals/2026/02/bb_two.heic", "two two"), ("exports/film.mp4", "film"), ("library.json", "{}"), ("derivatives/thumbs/x.jpg", "thumb")] {
            fs::create_dir_all(root.join(rel).parent().unwrap()).unwrap();
            fs::write(root.join(rel), body).unwrap();
        }
        let db = Db::open(&root.join("library.db")).unwrap();
        (dir, db)
    }

    fn back_up(root: &Path, db: &Db, target: &Path, now: i64) -> BackupReport {
        let source = Source { root, id: "lib-1", name: "Lumen Library", db };
        run(&source, target, None, now, &|| false, &mut |_| {}).unwrap()
    }

    #[test]
    fn a_first_run_copies_the_library_and_a_second_only_what_changed() {
        let (lib, db) = library();
        let dest = tempfile::tempdir().unwrap();
        let target = target_for(lib.path(), dest.path());

        let first = back_up(lib.path(), &db, &target, NOW);
        assert_eq!((first.copied, first.unchanged, first.failed.len()), (4, 0, 0));
        assert_eq!(fs::read_to_string(target.join("originals/2026/02/bb_two.heic")).unwrap(), "two two");
        assert!(!target.join("derivatives").exists(), "derivatives are made again, never copied");
        assert!(target.join("library.db").is_file());
        assert_eq!(read_marker(&target).unwrap().library_id, "lib-1");
        // The snapshot is a database that opens, with the library's schema.
        let copy = Db::open(&target.join("library.db")).unwrap();
        assert_eq!(copy.schema_version().unwrap(), db.schema_version().unwrap());
        drop(copy);

        fs::write(lib.path().join("originals/2026/03/cc_three.jpg").as_path(), "3").unwrap_or_else(|_| {
            fs::create_dir_all(lib.path().join("originals/2026/03")).unwrap();
            fs::write(lib.path().join("originals/2026/03/cc_three.jpg"), "3").unwrap();
        });
        let second = back_up(lib.path(), &db, &target, NOW + 60);
        assert_eq!((second.copied, second.unchanged), (1, 4));
        assert!(fs::read_dir(&target).unwrap().all(|e| !e.unwrap().file_name().to_string_lossy().ends_with(PART)), "no part left behind");
    }

    #[test]
    fn a_file_gone_from_the_library_is_kept_aside_for_thirty_days() {
        let (lib, db) = library();
        let dest = tempfile::tempdir().unwrap();
        let target = target_for(lib.path(), dest.path());
        back_up(lib.path(), &db, &target, NOW);

        fs::remove_file(lib.path().join("originals/2026/01/aa_one.jpg")).unwrap();
        let r = back_up(lib.path(), &db, &target, NOW);
        assert_eq!(r.retired, 1);
        assert!(!target.join("originals/2026/01/aa_one.jpg").exists());
        let kept = target.join(MARKER_DIR).join("removed").join(day(0).format("%Y-%m-%d").to_string()).join("originals/2026/01/aa_one.jpg");
        assert_eq!(fs::read_to_string(&kept).unwrap(), "one");

        // Still there after 30 days; gone after 31.
        back_up(lib.path(), &db, &target, NOW + 30 * 86_400);
        assert!(kept.exists());
        back_up(lib.path(), &db, &target, NOW + 31 * 86_400);
        assert!(!kept.exists());
    }

    #[test]
    fn a_destination_that_would_put_the_library_at_risk_is_refused() {
        let (lib, db) = library();
        let source = Source { root: lib.path(), id: "lib-1", name: "Lumen Library", db: &db };
        let go = |target: &Path| run(&source, target, None, NOW, &|| false, &mut |_| {});
        // Inside the library, or the library itself.
        assert!(matches!(go(&lib.path().join("backup")), Err(CoreError::Invalid(_))));
        assert!(matches!(go(lib.path()), Err(CoreError::Invalid(_))));
        assert!(!lib.path().join("backup").exists(), "nothing written into the library");
        // A folder already holding other things.
        let busy = tempfile::tempdir().unwrap();
        fs::write(busy.path().join("tax.pdf"), "x").unwrap();
        assert!(matches!(go(busy.path()), Err(CoreError::Invalid(_))));
        assert!(!busy.path().join("library.db").exists());
        // Another library's backup.
        let other = tempfile::tempdir().unwrap();
        fs::create_dir_all(other.path().join(MARKER_DIR)).unwrap();
        let marker = BackupMarker { library_id: "lib-2".into(), library_name: "Work".into(), source_path: "/w".into(), finished_at: 1 };
        fs::write(other.path().join(MARKER_DIR).join(MARKER_FILE), serde_json::to_vec(&marker).unwrap()).unwrap();
        assert!(matches!(go(other.path()), Err(CoreError::Invalid(m)) if m.contains("Work")));
    }

    #[test]
    fn a_backup_that_does_not_fit_copies_nothing() {
        let (lib, db) = library();
        let dest = tempfile::tempdir().unwrap();
        let target = target_for(lib.path(), dest.path());
        let source = Source { root: lib.path(), id: "lib-1", name: "Lumen Library", db: &db };
        assert!(matches!(run(&source, &target, Some(3), NOW, &|| false, &mut |_| {}), Err(CoreError::Invalid(_))));
        assert!(!target.join("originals").exists());
        assert!(!target.join("library.db").exists());
        assert!(!target.join(format!("library.db{PART}")).exists());
    }

    #[test]
    fn a_cancelled_run_leaves_no_database_and_resumes_next_time() {
        let (lib, db) = library();
        let dest = tempfile::tempdir().unwrap();
        let target = target_for(lib.path(), dest.path());
        let source = Source { root: lib.path(), id: "lib-1", name: "Lumen Library", db: &db };
        let calls = std::cell::Cell::new(0);
        // Cancel after the first file.
        let r = run(&source, &target, None, NOW, &|| calls.get() > 0, &mut |p| calls.set(p.done_files)).unwrap();
        assert!(r.cancelled);
        assert!(!target.join("library.db").exists(), "an unfinished backup has no database to open");
        let again = back_up(lib.path(), &db, &target, NOW);
        assert_eq!(again.copied + again.unchanged, 4);
        assert!(target.join("library.db").is_file());
    }

    #[test]
    fn a_sibling_whose_name_starts_like_the_library_is_not_inside_it() {
        let parent = tempfile::tempdir().unwrap();
        let lib = parent.path().join("Lumen Library");
        fs::create_dir_all(&lib).unwrap();
        assert!(check_destination(&lib, "lib-1", &parent.path().join("Lumen Library 2")).is_ok());
        assert!(check_destination(&lib, "lib-1", &parent.path().join("LUMEN LIBRARY").join("b")).is_err() || !cfg!(windows), "Windows paths ignore case");
    }

    #[test]
    fn a_scheduled_backup_is_due_once_its_interval_has_passed() {
        let mut c = BackupConfig { destination: Some("E:/".into()), schedule: Schedule::Daily, ..Default::default() };
        assert!(c.due(NOW), "never run");
        c.last_success_at = Some(NOW);
        assert!(!c.due(NOW + 3_600));
        assert!(c.due(NOW + 86_400));
        c.schedule = Schedule::Weekly;
        assert!(!c.due(NOW + 86_400));
        c.schedule = Schedule::Off;
        assert!(!c.due(NOW + 30 * 86_400));
        assert!(!BackupConfig { schedule: Schedule::Daily, ..Default::default() }.due(NOW), "nowhere to go");
    }

    #[test]
    fn the_configs_survive_a_round_trip_and_a_missing_file_is_empty() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("backups.json");
        assert_eq!(BackupConfigs::load(&path), BackupConfigs::default());
        let mut all = BackupConfigs::default();
        all.libraries.insert("lib-1".into(), BackupConfig { destination: Some("E:/".into()), schedule: Schedule::Weekly, last_success_at: Some(NOW), last: None });
        all.save(&path).unwrap();
        assert_eq!(BackupConfigs::load(&path), all);
    }

    #[test]
    fn the_backup_folder_is_named_like_the_library() {
        assert_eq!(target_for(Path::new("C:/Pictures/Lumen Library"), Path::new("E:/")), Path::new("E:/").join("Lumen Library"));
    }
}
