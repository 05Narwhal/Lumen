//! Settings → Storage → *Clear temporary files* (Round 22 §6): what Repair, Recover and the
//! import leave in the library folder, and the DNG conversion cache in app data, measured and
//! removed on request.
//!
//! **Data safety.** Nothing is removed unless its resolved path lies inside the one folder its
//! kind owns (`inside`), and that folder is not itself a link. `kept-aside/` holds the only copies
//! of replaced originals, so it goes to the OS Recycle Bin (behind [`Bin`], so tests never touch
//! the real one); the rest is regenerable and is deleted. Nothing here opens `library.db`, and
//! nothing under `originals/` is touched except a `*.repair.part` or an import's `.part`, each
//! only once it is an hour old.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use photos_core::library::INCOMING_DIR;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::error::AppError;

/// Above this much in total the Storage page warns (the spec's one constant).
pub const TEMP_WARN_BYTES: u64 = 2 * 1024 * 1024 * 1024;

/// A half-written copy this young may still be growing: left alone.
const YOUNG: Duration = Duration::from_secs(60 * 60);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum TempKind {
    /// `<library>/kept-aside`: originals a repair swapped out.
    KeptAside,
    /// `*.repair.part` beside the originals and the import's `.incoming/*.part`.
    Unfinished,
    /// `<library>/recovered`: recoveries nobody kept.
    Recovered,
    /// `<app data>/cache/dng`: DNG conversions of HE NEFs and other stand-ins.
    DngCache,
}

const ALL: [TempKind; 4] = [TempKind::KeptAside, TempKind::Unfinished, TempKind::Recovered, TempKind::DngCache];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct TempUsageRow {
    pub kind: TempKind,
    #[ts(type = "number")]
    pub bytes: u64,
    #[ts(type = "number")]
    pub files: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct TempUsage {
    /// Only kinds that hold something, in the page's order.
    pub rows: Vec<TempUsageRow>,
    #[ts(type = "number")]
    pub total: u64,
    /// The total is above [`TEMP_WARN_BYTES`].
    pub warn: bool,
}

/// The Recycle Bin, behind a trait so tests never touch the real one.
pub trait Bin {
    /// Moves these paths to the Bin. All or an error; nothing is erased for good.
    fn recycle(&self, paths: &[PathBuf]) -> io::Result<()>;
}

/// One thing a kind can remove, with its size.
struct Item {
    path: PathBuf,
    bytes: u64,
    files: u64,
}

/// The only folder a kind may remove from.
fn owner(kind: TempKind, lib_root: &Path, app_data: &Path) -> PathBuf {
    match kind {
        TempKind::KeptAside => lib_root.join("kept-aside"),
        TempKind::Unfinished => lib_root.join("originals"),
        TempKind::Recovered => lib_root.join("recovered"),
        TempKind::DngCache => app_data.join("cache").join("dng"),
    }
}

/// `candidate` really lies inside `root`: both exist, both resolve (links followed) and `root`
/// is not itself a link. False on any error, so a missing or unreadable path is never removed.
fn inside(root: &Path, candidate: &Path) -> bool {
    let Ok(meta) = fs::symlink_metadata(root) else { return false };
    if meta.file_type().is_symlink() || !meta.is_dir() {
        return false;
    }
    match (fs::canonicalize(root), fs::canonicalize(candidate)) {
        (Ok(r), Ok(c)) => c.starts_with(&r),
        _ => false,
    }
}

/// (bytes, files) of `path`; links are counted as nothing and never followed.
fn size_of(path: &Path) -> (u64, u64) {
    let Ok(meta) = fs::symlink_metadata(path) else { return (0, 0) };
    let ft = meta.file_type();
    if ft.is_symlink() {
        (0, 0)
    } else if ft.is_dir() {
        let Ok(rd) = fs::read_dir(path) else { return (0, 0) };
        rd.flatten().map(|e| size_of(&e.path())).fold((0, 0), |a, b| (a.0 + b.0, a.1 + b.1))
    } else {
        (meta.len(), 1)
    }
}

fn item(path: PathBuf) -> Item {
    let (bytes, files) = size_of(&path);
    Item { path, bytes, files }
}

/// Everything directly inside `dir`.
fn children(dir: &Path) -> Vec<Item> {
    fs::read_dir(dir).map(|rd| rd.flatten().map(|e| item(e.path())).collect()).unwrap_or_default()
}

fn old_enough(path: &Path, now: SystemTime) -> bool {
    fs::symlink_metadata(path).and_then(|m| m.modified()).ok().and_then(|t| now.duration_since(t).ok()).is_some_and(|age| age >= YOUNG)
}

/// `*.repair.part` anywhere under `originals/` and `.incoming/*.part`, each an hour old.
fn unfinished(originals: &Path, now: SystemTime) -> Vec<Item> {
    fn walk(dir: &Path, incoming: &Path, now: SystemTime, out: &mut Vec<Item>) {
        let Ok(rd) = fs::read_dir(dir) else { return };
        for e in rd.flatten() {
            let Ok(ft) = e.file_type() else { continue };
            let p = e.path();
            if ft.is_dir() {
                walk(&p, incoming, now, out);
            } else if ft.is_file() {
                let name = e.file_name().to_string_lossy().to_ascii_lowercase();
                let staged = name.ends_with(".part") && p.parent() == Some(incoming);
                if (name.ends_with(".repair.part") || staged) && old_enough(&p, now) {
                    out.push(item(p));
                }
            }
        }
    }
    let incoming = originals.parent().map(|r| r.join(INCOMING_DIR)).unwrap_or_default();
    let mut out = Vec::new();
    walk(originals, &incoming, now, &mut out);
    out
}

fn items(kind: TempKind, lib_root: &Path, app_data: &Path, now: SystemTime) -> Vec<Item> {
    let owner = owner(kind, lib_root, app_data);
    match kind {
        TempKind::Unfinished => unfinished(&owner, now),
        _ => children(&owner),
    }
}

/// The page's rows: what each kind holds right now.
pub fn usage(lib_root: &Path, app_data: &Path) -> TempUsage {
    usage_at(lib_root, app_data, SystemTime::now())
}

fn usage_at(lib_root: &Path, app_data: &Path, now: SystemTime) -> TempUsage {
    let rows = ALL
        .iter()
        .map(|&kind| {
            let list = items(kind, lib_root, app_data, now);
            TempUsageRow { kind, bytes: list.iter().map(|i| i.bytes).sum(), files: list.iter().map(|i| i.files).sum() }
        })
        .collect();
    summarise(rows)
}

/// Drops the empty rows and adds the total and the warning.
fn summarise(rows: Vec<TempUsageRow>) -> TempUsage {
    let rows: Vec<TempUsageRow> = rows.into_iter().filter(|r| r.files > 0 || r.bytes > 0).collect();
    let total = rows.iter().map(|r| r.bytes).sum();
    TempUsage { rows, total, warn: total > TEMP_WARN_BYTES }
}

/// Removes what `kinds` hold and returns the bytes freed. `busy` is a repair or a recovery
/// running: refused with code `busy` and nothing removed. `kept-aside/` goes first and to the Bin;
/// if the Bin refuses, nothing else is touched either.
pub fn clear(lib_root: &Path, app_data: &Path, kinds: &[TempKind], busy: bool, bin: &dyn Bin) -> Result<u64, AppError> {
    clear_at(lib_root, app_data, kinds, busy, bin, SystemTime::now())
}

fn clear_at(lib_root: &Path, app_data: &Path, kinds: &[TempKind], busy: bool, bin: &dyn Bin, now: SystemTime) -> Result<u64, AppError> {
    if busy {
        return Err(AppError { code: "busy".into(), message: "Wait for Repair to finish.".into(), detail: None });
    }
    let mut freed = 0;
    for kind in ALL.into_iter().filter(|k| kinds.contains(k)) {
        let root = owner(kind, lib_root, app_data);
        freed += remove_listed(&root, items(kind, lib_root, app_data, now), if kind == TempKind::KeptAside { Some(bin) } else { None })?;
    }
    Ok(freed)
}

/// Removes each item that lies inside `root` (to the Bin when one is given, else for good) and
/// returns the bytes it freed. An item outside `root` is skipped, never removed.
fn remove_listed(root: &Path, list: Vec<Item>, bin: Option<&dyn Bin>) -> Result<u64, AppError> {
    let safe: Vec<Item> = list
        .into_iter()
        .filter(|i| {
            let ok = inside(root, &i.path);
            if !ok {
                tracing::warn!(path = %i.path.display(), root = %root.display(), "temporary file outside its folder left alone");
            }
            ok
        })
        .collect();
    if safe.is_empty() {
        return Ok(0);
    }
    match bin {
        Some(bin) => {
            let paths: Vec<PathBuf> = safe.iter().map(|i| i.path.clone()).collect();
            bin.recycle(&paths).map_err(|e| AppError {
                code: "bin".into(),
                message: "Lumen couldn't move the replaced originals to the Recycle Bin, so it left them where they are.".into(),
                detail: Some(e.to_string()),
            })?;
            Ok(safe.iter().map(|i| i.bytes).sum())
        }
        None => {
            let mut freed = 0;
            for i in &safe {
                // A link is removed as a link; a folder with its contents.
                let removed = match fs::symlink_metadata(&i.path) {
                    Ok(m) if m.is_dir() && !m.file_type().is_symlink() => fs::remove_dir_all(&i.path),
                    Ok(_) => fs::remove_file(&i.path),
                    Err(e) => Err(e),
                };
                match removed {
                    Ok(()) => freed += i.bytes,
                    Err(e) => tracing::warn!(path = %i.path.display(), error = %e, "temporary file could not be removed"),
                }
            }
            Ok(freed)
        }
    }
}

// ---------------------------------------------------------------- the real Recycle Bin

/// Windows' shell Recycle Bin (the call Lumen Setup's library removal makes).
#[cfg(windows)]
pub struct SystemBin;

#[cfg(windows)]
impl Bin for SystemBin {
    fn recycle(&self, paths: &[PathBuf]) -> io::Result<()> {
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::UI::Shell::{SHFileOperationW, FOF_ALLOWUNDO, FOF_NOCONFIRMATION, FOF_NOERRORUI, FOF_SILENT, FOF_WANTNUKEWARNING, FO_DELETE, SHFILEOPSTRUCTW};
        if paths.is_empty() {
            return Ok(());
        }
        // A double-NUL-terminated list of full paths.
        let mut from: Vec<u16> = Vec::new();
        for p in paths {
            from.extend(p.as_os_str().encode_wide());
            from.push(0);
        }
        from.push(0);
        let mut op = SHFILEOPSTRUCTW {
            hwnd: std::ptr::null_mut(),
            wFunc: FO_DELETE,
            pFrom: from.as_ptr(),
            pTo: std::ptr::null(),
            // FOF_WANTNUKEWARNING: should something not fit, Windows refuses rather than erase it.
            fFlags: (FOF_ALLOWUNDO | FOF_NOCONFIRMATION | FOF_SILENT | FOF_NOERRORUI | FOF_WANTNUKEWARNING) as u16,
            fAnyOperationsAborted: 0,
            hNameMappings: std::ptr::null_mut(),
            lpszProgressTitle: std::ptr::null(),
        };
        // SAFETY: `op` points at `from`, which outlives the call.
        let code = unsafe { SHFileOperationW(&mut op) };
        if code != 0 {
            return Err(io::Error::other(format!("the Recycle Bin refused it, code {code}")));
        }
        if op.fAnyOperationsAborted != 0 {
            return Err(io::Error::other("it was stopped before the end"));
        }
        Ok(())
    }
}

/// macOS: into `~/.Trash` by a rename (a move across disks is refused, and nothing is erased).
#[cfg(target_os = "macos")]
pub struct SystemBin;

#[cfg(target_os = "macos")]
impl Bin for SystemBin {
    fn recycle(&self, paths: &[PathBuf]) -> io::Result<()> {
        let trash = dirs::home_dir().ok_or_else(|| io::Error::other("no home folder"))?.join(".Trash");
        fs::create_dir_all(&trash)?;
        for p in paths {
            fs::rename(p, free_name(&trash, p))?;
        }
        Ok(())
    }
}

/// Linux: the freedesktop trash, `$XDG_DATA_HOME/Trash/{files,info}` (a move across disks is
/// refused, and nothing is erased).
#[cfg(all(unix, not(target_os = "macos")))]
pub struct SystemBin;

#[cfg(all(unix, not(target_os = "macos")))]
impl Bin for SystemBin {
    fn recycle(&self, paths: &[PathBuf]) -> io::Result<()> {
        let trash = dirs::data_dir().ok_or_else(|| io::Error::other("no data folder"))?.join("Trash");
        let (files, info) = (trash.join("files"), trash.join("info"));
        fs::create_dir_all(&files)?;
        fs::create_dir_all(&info)?;
        for p in paths {
            let dest = free_name(&files, p);
            let name = dest.file_name().unwrap_or_default().to_string_lossy().into_owned();
            let encoded: String = p.to_string_lossy().bytes().map(|b| if b.is_ascii_alphanumeric() || b"/-_.~".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") }).collect();
            let when = chrono::Local::now().format("%Y-%m-%dT%H:%M:%S");
            fs::write(info.join(format!("{name}.trashinfo")), format!("[Trash Info]\nPath={encoded}\nDeletionDate={when}\n"))?;
            if let Err(e) = fs::rename(p, &dest) {
                let _ = fs::remove_file(info.join(format!("{name}.trashinfo")));
                return Err(e);
            }
        }
        Ok(())
    }
}

/// `dir/<name>`, numbered while that name is taken.
#[cfg(unix)]
fn free_name(dir: &Path, from: &Path) -> PathBuf {
    let name = from.file_name().unwrap_or_default().to_string_lossy().into_owned();
    let mut dest = dir.join(&name);
    let mut n = 1;
    while dest.exists() {
        dest = dir.join(format!("{name} {n}"));
        n += 1;
    }
    dest
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    /// Moves into a folder of its own and remembers what it was given.
    struct FakeBin {
        dir: PathBuf,
        got: RefCell<Vec<PathBuf>>,
        fail: bool,
    }
    impl Bin for FakeBin {
        fn recycle(&self, paths: &[PathBuf]) -> io::Result<()> {
            if self.fail {
                return Err(io::Error::other("full"));
            }
            for (n, p) in paths.iter().enumerate() {
                fs::rename(p, self.dir.join(format!("{}-{n}", self.got.borrow().len())))?;
                self.got.borrow_mut().push(p.clone());
            }
            Ok(())
        }
    }

    struct Fixture {
        _t: tempfile::TempDir,
        lib: PathBuf,
        app: PathBuf,
        bin: FakeBin,
    }

    fn put(p: &Path, bytes: usize) {
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, vec![7u8; bytes]).unwrap();
    }

    fn age(p: &Path, by: Duration) {
        let f = fs::OpenOptions::new().write(true).open(p).unwrap();
        f.set_modified(SystemTime::now() - by).unwrap();
    }

    /// A library with a few bytes of each kind: kept-aside 100 + 50, an old repair part 10 and an
    /// old import part 5, recovered 20 + 1, DNG cache 40, plus originals and a database that
    /// must never be touched.
    fn fixture() -> Fixture {
        let t = tempfile::tempdir().unwrap();
        let (lib, app) = (t.path().join("Lumen Library"), t.path().join("app"));
        put(&lib.join("kept-aside/2026-11-13/2021/Čaj ü.heic"), 100);
        put(&lib.join("kept-aside/2026-11-14/b.jpg"), 50);
        put(&lib.join("originals/2021/a.jpg.repair.part"), 10);
        put(&lib.join("originals/.incoming/x.part"), 5);
        put(&lib.join("originals/2021/a.jpg"), 1000);
        put(&lib.join("recovered/3.jpg"), 20);
        put(&lib.join("recovered/3.json"), 1);
        put(&app.join("cache/dng/abc.dng"), 40);
        put(&lib.join("library.db"), 9);
        let long = Duration::from_secs(2 * 60 * 60);
        age(&lib.join("originals/2021/a.jpg.repair.part"), long);
        age(&lib.join("originals/.incoming/x.part"), long);
        let bin_dir = t.path().join("bin");
        fs::create_dir_all(&bin_dir).unwrap();
        Fixture { lib, app, bin: FakeBin { dir: bin_dir, got: RefCell::new(vec![]), fail: false }, _t: t }
    }

    fn row(u: &TempUsage, k: TempKind) -> Option<(u64, u64)> {
        u.rows.iter().find(|r| r.kind == k).map(|r| (r.bytes, r.files))
    }

    #[test]
    fn usage_counts_each_kind() {
        let f = fixture();
        let u = usage(&f.lib, &f.app);
        assert_eq!(row(&u, TempKind::KeptAside), Some((150, 2)));
        assert_eq!(row(&u, TempKind::Unfinished), Some((15, 2)));
        assert_eq!(row(&u, TempKind::Recovered), Some((21, 2)));
        assert_eq!(row(&u, TempKind::DngCache), Some((40, 1)));
        assert_eq!(u.total, 226);
        assert!(!u.warn);
    }

    #[test]
    fn a_kind_with_nothing_has_no_row() {
        let t = tempfile::tempdir().unwrap();
        let u = usage(&t.path().join("lib"), &t.path().join("app"));
        assert!(u.rows.is_empty());
        assert_eq!(u.total, 0);
    }

    #[test]
    fn clear_removes_only_the_chosen_kinds() {
        let f = fixture();
        let freed = clear(&f.lib, &f.app, &[TempKind::Recovered, TempKind::DngCache], false, &f.bin).unwrap();
        assert_eq!(freed, 61);
        assert!(!f.lib.join("recovered/3.jpg").exists() && !f.app.join("cache/dng/abc.dng").exists());
        // Everything else stays: kept-aside, the unfinished copies, an original, the database.
        assert!(f.lib.join("kept-aside/2026-11-14/b.jpg").is_file());
        assert!(f.lib.join("originals/2021/a.jpg.repair.part").is_file());
        assert!(f.lib.join("originals/2021/a.jpg").is_file());
        assert!(f.lib.join("library.db").is_file());
        assert!(f.bin.got.borrow().is_empty(), "caches never go through the Bin");
    }

    #[test]
    fn kept_aside_goes_to_the_bin_not_to_nothing() {
        let f = fixture();
        let freed = clear(&f.lib, &f.app, &[TempKind::KeptAside], false, &f.bin).unwrap();
        assert_eq!(freed, 150);
        assert_eq!(f.bin.got.borrow().len(), 2, "one item per date folder");
        let kept: u64 = fs::read_dir(&f.bin.dir).unwrap().flatten().map(|e| size_of(&e.path()).0).sum();
        assert_eq!(kept, 150, "the bytes are in the Bin, not gone");
        assert!(!f.lib.join("kept-aside/2026-11-13").exists());
    }

    #[test]
    fn a_bin_that_refuses_leaves_everything() {
        let mut f = fixture();
        f.bin.fail = true;
        let e = clear(&f.lib, &f.app, &[TempKind::KeptAside, TempKind::Recovered], false, &f.bin).unwrap_err();
        assert_eq!(e.code, "bin");
        assert!(f.lib.join("kept-aside/2026-11-14/b.jpg").is_file());
        assert!(f.lib.join("recovered/3.jpg").is_file(), "kept-aside is first, so nothing after it ran");
    }

    #[test]
    fn unfinished_removes_only_the_part_files() {
        let f = fixture();
        let freed = clear(&f.lib, &f.app, &[TempKind::Unfinished], false, &f.bin).unwrap();
        assert_eq!(freed, 15);
        assert!(!f.lib.join("originals/2021/a.jpg.repair.part").exists());
        assert!(!f.lib.join("originals/.incoming/x.part").exists());
        assert!(f.lib.join("originals/2021/a.jpg").is_file(), "an original is never an unfinished copy");
    }

    #[test]
    fn a_young_part_file_is_left_alone() {
        let f = fixture();
        put(&f.lib.join("originals/2022/new.heic.repair.part"), 8);
        put(&f.lib.join("originals/.incoming/live.part"), 8);
        let u = usage(&f.lib, &f.app);
        assert_eq!(row(&u, TempKind::Unfinished), Some((15, 2)), "only the two old ones count");
        clear(&f.lib, &f.app, &[TempKind::Unfinished], false, &f.bin).unwrap();
        assert!(f.lib.join("originals/2022/new.heic.repair.part").is_file());
        assert!(f.lib.join("originals/.incoming/live.part").is_file());
    }

    #[test]
    fn a_part_file_anywhere_else_is_not_unfinished() {
        let f = fixture();
        // `.part` outside `.incoming` and not a repair copy (an editor's, a sidecar's): not ours.
        put(&f.lib.join("originals/2021/other.part"), 3);
        clear(&f.lib, &f.app, &[TempKind::Unfinished], false, &f.bin).unwrap();
        assert!(f.lib.join("originals/2021/other.part").is_file());
    }

    #[test]
    fn a_path_outside_is_refused() {
        let f = fixture();
        let outside = f.lib.parent().unwrap().join("Pictures/IMG_1.heic");
        put(&outside, 500);
        let root = f.lib.join("recovered");
        // `..`, an absolute path elsewhere, and a path that does not exist.
        let dotdot = root.join("..").join("..").join("Pictures").join("IMG_1.heic");
        assert!(!inside(&root, &dotdot));
        assert!(!inside(&root, &outside));
        assert!(!inside(&root, &root.join("missing.jpg")));
        assert!(inside(&root, &root.join("3.jpg")));
        let freed = remove_listed(&root, vec![item(dotdot), item(outside.clone())], None).unwrap();
        assert_eq!(freed, 0);
        assert!(outside.is_file(), "nothing outside the folder was removed");
    }

    #[test]
    fn a_link_pointing_out_is_refused() {
        let f = fixture();
        let outside = f.lib.parent().unwrap().join("Pictures");
        put(&outside.join("IMG_2.heic"), 500);
        let link = f.lib.join("recovered/out");
        #[cfg(windows)]
        let made = std::os::windows::fs::symlink_dir(&outside, &link);
        #[cfg(unix)]
        let made = std::os::unix::fs::symlink(&outside, &link);
        if made.is_err() {
            eprintln!("SKIPPED: no privilege to make a link");
            return; // Windows without the privilege to make a link: the `..` test above covers `inside`.
        }
        assert!(!inside(&f.lib.join("recovered"), &link));
        clear(&f.lib, &f.app, &[TempKind::Recovered], false, &f.bin).unwrap();
        assert!(outside.join("IMG_2.heic").is_file(), "the link's target is untouched");
    }

    #[test]
    fn a_folder_that_is_a_link_is_not_a_root() {
        let f = fixture();
        let target = f.lib.parent().unwrap().join("Elsewhere");
        put(&target.join("keep.jpg"), 5);
        fs::remove_dir_all(f.lib.join("recovered")).unwrap();
        #[cfg(windows)]
        let made = std::os::windows::fs::symlink_dir(&target, f.lib.join("recovered"));
        #[cfg(unix)]
        let made = std::os::unix::fs::symlink(&target, f.lib.join("recovered"));
        if made.is_err() {
            eprintln!("SKIPPED: no privilege to make a link");
            return;
        }
        assert!(!inside(&f.lib.join("recovered"), &f.lib.join("recovered").join("keep.jpg")));
        clear(&f.lib, &f.app, &[TempKind::Recovered], false, &f.bin).unwrap();
        assert!(target.join("keep.jpg").is_file());
    }

    #[test]
    fn clear_is_refused_while_a_repair_runs() {
        let f = fixture();
        let e = clear(&f.lib, &f.app, &ALL, true, &f.bin).unwrap_err();
        assert_eq!(e.code, "busy");
        assert_eq!(usage(&f.lib, &f.app).total, 226, "nothing was removed");
        assert!(f.bin.got.borrow().is_empty());
    }

    #[test]
    fn the_warning_starts_above_two_gibibytes() {
        let rows = |bytes| vec![TempUsageRow { kind: TempKind::KeptAside, bytes, files: 1 }];
        assert!(!summarise(rows(TEMP_WARN_BYTES)).warn);
        assert!(summarise(rows(TEMP_WARN_BYTES + 1)).warn);
        let two = vec![TempUsageRow { kind: TempKind::KeptAside, bytes: TEMP_WARN_BYTES, files: 1 }, TempUsageRow { kind: TempKind::DngCache, bytes: 1, files: 1 }];
        let u = summarise(two);
        assert_eq!(u.total, TEMP_WARN_BYTES + 1);
        assert!(u.warn, "the total counts, not one row");
    }
}
