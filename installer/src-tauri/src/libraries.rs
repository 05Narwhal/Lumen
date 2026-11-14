//! Deleting libraries the person chose, by typing, on screen 6b (spec *Amendment (2026-11-06)*).
//! Only Lumen's own parts of a library go ([`PARTS`]); anything else in the folder stays and the
//! folder goes only when that leaves it empty. Parts go to the Recycle Bin unless the person chose
//! *Delete for good* after being told they don't fit. Every rule is checked again at delete time.
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::removal::{is_link, norm};

/// What Lumen makes in a library (`photos-core::library`, the manifest, SQLite's side files and
/// the music folder backups mirror). Held by `the_parts_name_everything_a_new_library_has`.
pub const PARTS: &[&str] = &[
    "library.db",
    "library.db-wal",
    "library.db-shm",
    "library.db-journal",
    "library.json",
    "originals",
    "derivatives",
    "trash",
    "exports",
    "music",
    "logs",
];
const MARKERS: &[&str] = &["library.db", "library.json"];

/// Why a registered library is shown but can't be deleted from Setup.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "kebab-case")]
#[ts(export)]
pub enum Refusal {
    /// The folder is gone or holds no `library.db` / `library.json`.
    NotALibrary,
    /// A drive root or the user folder itself.
    TooHigh,
    /// It is, or holds, Lumen's app data or install folder.
    HoldsLumen,
    /// The folder itself is a link or junction.
    Link,
}

impl Refusal {
    pub fn sentence(self) -> &'static str {
        match self {
            Refusal::NotALibrary => "This folder no longer holds a Lumen library.",
            Refusal::TooHigh => "This library is a whole drive or user folder. Delete it yourself if you mean to.",
            Refusal::HoldsLumen => "Lumen's own files are inside this library. Delete it yourself after removing Lumen.",
            Refusal::Link => "This library's folder is a link to somewhere else. Delete it yourself if you mean to.",
        }
    }
}

/// Folders a library must never be, contain or sit inside of.
#[derive(Debug, Clone, Default)]
pub struct Protected {
    pub app_data: Option<PathBuf>,
    pub webview_data: Option<PathBuf>,
    pub install_dir: Option<PathBuf>,
    pub profile: Option<PathBuf>,
}

impl Protected {
    pub fn real(install_dir: Option<PathBuf>) -> Self {
        Self {
            app_data: crate::removal::app_data_dir(),
            webview_data: crate::removal::webview_data_dir(),
            install_dir,
            profile: std::env::var_os("USERPROFILE").map(PathBuf::from),
        }
    }
}

fn inside(a: &Path, b: &Path) -> bool {
    a.starts_with(b)
}

/// Whether `library` may be deleted at all. Called when listing and again before deleting.
pub fn check(library: &Path, protected: &Protected) -> Result<(), Refusal> {
    let meta = fs::symlink_metadata(library).map_err(|_| Refusal::NotALibrary)?;
    if is_link(&meta) {
        return Err(Refusal::Link);
    }
    if !meta.is_dir() || !MARKERS.iter().any(|m| fs::symlink_metadata(library.join(m)).is_ok_and(|m| m.is_file())) {
        return Err(Refusal::NotALibrary);
    }
    let lib = norm(library);
    if lib.parent().is_none() || lib.components().count() <= 1 || protected.profile.as_deref().is_some_and(|p| lib == norm(p)) {
        return Err(Refusal::TooHigh);
    }
    for p in [&protected.app_data, &protected.webview_data, &protected.install_dir].into_iter().flatten() {
        let p = norm(p);
        // Lumen's files inside the library, or the library inside them.
        if inside(&p, &lib) || inside(&lib, &p) {
            return Err(Refusal::HoldsLumen);
        }
    }
    Ok(())
}

/// One library on screen 6b.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct LibraryDetail {
    pub name: String,
    pub path: String,
    /// Files under `originals/` (not the import's half-copied `.incoming`).
    #[ts(type = "number")]
    pub photos: u64,
    /// Every byte of Lumen's parts: what would go.
    #[ts(type = "number")]
    pub bytes: u64,
    pub refused: Option<Refusal>,
}

/// Size and file count of a tree, never following a link (a link counts as nothing).
fn measure(p: &Path) -> (u64, u64) {
    let Ok(m) = fs::symlink_metadata(p) else { return (0, 0) };
    if is_link(&m) {
        return (0, 0);
    }
    if m.is_file() {
        return (m.len(), 1);
    }
    let mut total = (0, 0);
    if let Ok(rd) = fs::read_dir(p) {
        for e in rd.flatten() {
            let (b, n) = measure(&e.path());
            total = (total.0 + b, total.1 + n);
        }
    }
    total
}

pub fn detail(name: &str, library: &Path, protected: &Protected) -> LibraryDetail {
    let refused = check(library, protected).err();
    let (bytes, photos) = if refused.is_some() {
        (0, 0)
    } else {
        let bytes = PARTS.iter().map(|p| measure(&library.join(p)).0).sum();
        let (_, all) = measure(&library.join("originals"));
        let (_, incoming) = measure(&library.join("originals").join(".incoming"));
        (bytes, all - incoming)
    };
    LibraryDetail { name: name.to_string(), path: library.display().to_string(), photos, bytes, refused }
}

/// `(name, path)` of every registered library, as `libraries.json` has them.
pub fn registered(app_data: &Path) -> Vec<(String, PathBuf)> {
    let Ok(text) = fs::read_to_string(app_data.join(crate::removal::REGISTRY_FILE)) else { return Vec::new() };
    let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) else { return Vec::new() };
    v["libraries"]
        .as_array()
        .map(|a| a.iter().filter_map(|l| Some((l["name"].as_str().unwrap_or("").to_string(), PathBuf::from(l["path"].as_str()?)))).collect())
        .unwrap_or_default()
}

/// The Recycle Bin, behind a trait so tests never touch the real one.
pub trait Bin {
    /// How many bytes the Bin on `path`'s drive can hold; None when that drive has no Bin.
    fn capacity(&self, path: &Path) -> Option<u64>;
    /// Move these paths to the Bin.
    fn recycle(&self, paths: &[PathBuf]) -> io::Result<()>;
}

/// Screen 6b's question before anything is deleted.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct BinCheck {
    pub fits: bool,
    #[ts(type = "number")]
    pub bytes: u64,
    /// The smallest Bin among the drives involved (0 when one has none).
    #[ts(type = "number")]
    pub room: u64,
}

pub fn bin_check(libraries: &[PathBuf], protected: &Protected, bin: &dyn Bin) -> BinCheck {
    let mut bytes = 0;
    let mut room = u64::MAX;
    for l in libraries {
        let d = detail("", l, protected);
        bytes += d.bytes;
        room = room.min(bin.capacity(l).unwrap_or(0));
    }
    let room = if libraries.is_empty() { 0 } else { room };
    BinCheck { fits: bytes <= room, bytes, room }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "kebab-case")]
#[ts(export)]
pub enum Mode {
    Recycle,
    /// Only after the person was told the parts don't fit the Bin and chose *Delete for good*.
    Erase,
}

/// What happened to one library.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Deleted {
    pub path: String,
    /// The folder itself went (nothing else was in it).
    pub folder_removed: bool,
    /// Why it was not (fully) deleted, as a sentence; None when every part went.
    pub problem: Option<String>,
}

/// Remove a tree without following links: a link is removed as a link.
fn erase(p: &Path) -> io::Result<()> {
    let m = fs::symlink_metadata(p)?;
    if is_link(&m) {
        // A directory junction/symlink is removed with remove_dir, a file link with remove_file.
        return if m.is_dir() || fs::remove_file(p).is_err() { fs::remove_dir(p) } else { Ok(()) };
    }
    if m.is_dir() {
        for e in fs::read_dir(p)? {
            erase(&e?.path())?;
        }
        fs::remove_dir(p)
    } else {
        fs::remove_file(p)
    }
}

pub fn delete(library: &Path, protected: &Protected, mode: Mode, bin: &dyn Bin) -> Deleted {
    let path = library.display().to_string();
    if let Err(r) = check(library, protected) {
        return Deleted { path, folder_removed: false, problem: Some(r.sentence().to_string()) };
    }
    let parts: Vec<PathBuf> = PARTS.iter().map(|p| library.join(p)).filter(|p| fs::symlink_metadata(p).is_ok()).collect();
    let result = match mode {
        Mode::Recycle => bin.recycle(&parts),
        Mode::Erase => parts.iter().try_for_each(|p| erase(p)),
    };
    let problem = result.err().map(|e| format!("Some of this library could not be deleted ({e}). What is left is still in its folder."));
    // Never a recursive delete of the library folder: only when Lumen's parts were all it held.
    let folder_removed = problem.is_none() && fs::remove_dir(library).is_ok();
    Deleted { path, folder_removed, problem }
}

/// When app data stays, the deleted libraries leave `libraries.json` (written to a sibling, then
/// renamed). `current` moves to the first library left, or becomes null.
pub fn forget(app_data: &Path, gone: &[PathBuf]) -> io::Result<()> {
    let file = app_data.join(crate::removal::REGISTRY_FILE);
    let text = fs::read_to_string(&file)?;
    let mut v: serde_json::Value = serde_json::from_str(&text).map_err(io::Error::other)?;
    let gone: Vec<PathBuf> = gone.iter().map(|g| norm(g)).collect();
    let Some(list) = v["libraries"].as_array_mut() else { return Ok(()) };
    list.retain(|l| l["path"].as_str().is_none_or(|p| !gone.contains(&norm(Path::new(p)))));
    let ids: Vec<serde_json::Value> = list.iter().map(|l| l["id"].clone()).collect();
    if !ids.contains(&v["current"]) {
        v["current"] = ids.first().cloned().unwrap_or(serde_json::Value::Null);
    }
    let tmp = app_data.join(format!("{}.tmp", crate::removal::REGISTRY_FILE));
    fs::write(&tmp, serde_json::to_string_pretty(&v).map_err(io::Error::other)?)?;
    fs::rename(&tmp, &file)
}

/// The real Recycle Bin (Windows shell).
#[cfg(windows)]
pub struct ShellBin;

#[cfg(windows)]
fn wide(s: &std::ffi::OsStr) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;
    s.encode_wide().collect()
}

#[cfg(windows)]
impl Bin for ShellBin {
    fn capacity(&self, path: &Path) -> Option<u64> {
        use windows_sys::Win32::Storage::FileSystem::{GetDiskFreeSpaceExW, GetDriveTypeW, GetVolumeNameForVolumeMountPointW, GetVolumePathNameW};
        const DRIVE_FIXED: u32 = 3;
        let mut root = vec![0u16; 1024];
        let p: Vec<u16> = wide(path.as_os_str()).into_iter().chain([0]).collect();
        // SAFETY: both buffers are NUL-terminated and sized as passed.
        if unsafe { GetVolumePathNameW(p.as_ptr(), root.as_mut_ptr(), root.len() as u32) } == 0 {
            return None;
        }
        if unsafe { GetDriveTypeW(root.as_ptr()) } != DRIVE_FIXED {
            return None;
        }
        let mut guid = vec![0u16; 64];
        // SAFETY: `root` is the NUL-terminated mount point from above; `guid` is 64 wide chars.
        let guid = if unsafe { GetVolumeNameForVolumeMountPointW(root.as_ptr(), guid.as_mut_ptr(), guid.len() as u32) } != 0 {
            let s = String::from_utf16_lossy(&guid[..guid.iter().position(|&c| c == 0).unwrap_or(0)]);
            s.find('{').and_then(|a| s.find('}').map(|b| s[a..=b].to_string()))
        } else {
            None
        };
        let key = guid.and_then(|g| {
            winreg::RegKey::predef(winreg::enums::HKEY_CURRENT_USER).open_subkey(format!(r"Software\Microsoft\Windows\CurrentVersion\Explorer\BitBucket\Volume\{g}")).ok()
        });
        if key.as_ref().and_then(|k| k.get_value::<u32, _>("NukeOnDelete").ok()) == Some(1) {
            return None;
        }
        if let Some(mb) = key.as_ref().and_then(|k| k.get_value::<u32, _>("MaxCapacity").ok()) {
            return Some(u64::from(mb) * 1024 * 1024);
        }
        // No setting stored: Windows' default is about 5 % of the drive; assume that.
        let mut total = 0u64;
        // SAFETY: `root` is NUL-terminated; only the total is asked for.
        unsafe { GetDiskFreeSpaceExW(root.as_ptr(), std::ptr::null_mut(), &mut total, std::ptr::null_mut()) };
        Some(total / 20)
    }

    fn recycle(&self, paths: &[PathBuf]) -> io::Result<()> {
        use windows_sys::Win32::UI::Shell::{SHFileOperationW, FOF_ALLOWUNDO, FOF_NOCONFIRMATION, FOF_NOERRORUI, FOF_SILENT, FOF_WANTNUKEWARNING, FO_DELETE, SHFILEOPSTRUCTW};
        if paths.is_empty() {
            return Ok(());
        }
        // Double-NUL-terminated list of full paths.
        let mut from: Vec<u16> = Vec::new();
        for p in paths {
            from.extend(wide(p.as_os_str()));
            from.push(0);
        }
        from.push(0);
        let mut op = SHFILEOPSTRUCTW {
            hwnd: std::ptr::null_mut(),
            wFunc: FO_DELETE,
            pFrom: from.as_ptr(),
            pTo: std::ptr::null(),
            // FOF_WANTNUKEWARNING: should something not fit after all, Windows asks before erasing it.
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    struct FakeBin {
        capacity: Option<u64>,
        recycled: RefCell<Vec<PathBuf>>,
    }

    impl FakeBin {
        fn new(capacity: Option<u64>) -> Self {
            Self { capacity, recycled: RefCell::new(Vec::new()) }
        }
    }

    impl Bin for FakeBin {
        fn capacity(&self, _: &Path) -> Option<u64> {
            self.capacity
        }
        /// Moves each path into a "bin" folder beside the test tree, like the real Bin would.
        fn recycle(&self, paths: &[PathBuf]) -> io::Result<()> {
            for p in paths {
                let bin = p.parent().unwrap().parent().unwrap().join("bin");
                fs::create_dir_all(&bin)?;
                fs::rename(p, bin.join(format!("{}-{}", self.recycled.borrow().len(), p.file_name().unwrap().to_string_lossy())))?;
                self.recycled.borrow_mut().push(p.clone());
            }
            Ok(())
        }
    }

    fn put(root: &Path, files: &[(&str, usize)]) {
        for (rel, n) in files {
            let p = root.join(rel);
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            fs::write(p, vec![b'x'; *n]).unwrap();
        }
    }

    /// A temp tree with a library `Photos` holding Lumen's parts and one file of the person's own.
    fn library() -> (tempfile::TempDir, PathBuf) {
        let t = tempfile::tempdir().unwrap();
        let lib = t.path().join("Photos");
        put(&lib, &[("library.db", 100), ("library.db-wal", 10), ("library.json", 5), ("originals/2026/01/a.jpg", 1000), ("originals/2026/01/b.jpg", 2000), ("originals/.incoming/c.part", 50), ("derivatives/thumbs/a.jpg", 30), ("trash/old.jpg", 7), ("exports/x.mp4", 8)]);
        (t, lib)
    }

    fn protected(t: &tempfile::TempDir) -> Protected {
        Protected { app_data: Some(t.path().join("AppData")), webview_data: None, install_dir: Some(t.path().join("Install")), profile: Some(t.path().to_path_buf()) }
    }

    #[test]
    fn the_parts_name_everything_a_new_library_has() {
        let t = tempfile::tempdir().unwrap();
        let root = t.path().join("New");
        drop(photos_core::library::Library::create(&root).unwrap());
        for e in fs::read_dir(&root).unwrap() {
            let name = e.unwrap().file_name().to_string_lossy().into_owned();
            assert!(PARTS.contains(&name.as_str()), "{name} is made by Lumen but would be left behind");
        }
    }

    #[test]
    fn detail_counts_photos_and_every_part() {
        let (t, lib) = library();
        let d = detail("Photos", &lib, &protected(&t));
        assert_eq!(d.refused, None);
        assert_eq!(d.photos, 2, "the half-copied .incoming file is not a photo");
        assert_eq!(d.bytes, 100 + 10 + 5 + 1000 + 2000 + 50 + 30 + 7 + 8);
    }

    #[test]
    fn only_lumens_parts_go_and_the_persons_own_files_stay() {
        let (t, lib) = library();
        put(&lib, &[("my notes.txt", 3), ("Holiday edits/final.psd", 9)]);
        let bin = FakeBin::new(Some(u64::MAX));
        let r = delete(&lib, &protected(&t), Mode::Recycle, &bin);
        assert_eq!(r.problem, None);
        assert!(!r.folder_removed, "the folder still holds the person's files");
        assert_eq!(fs::read(lib.join("my notes.txt")).unwrap(), b"xxx");
        assert!(lib.join("Holiday edits/final.psd").is_file());
        for p in ["library.db", "library.json", "originals", "derivatives", "trash", "exports"] {
            assert!(!lib.join(p).exists(), "{p}");
        }
    }

    #[test]
    fn recycling_moves_the_parts_to_the_bin_and_then_the_empty_folder_goes() {
        let (t, lib) = library();
        let bin = FakeBin::new(Some(u64::MAX));
        let r = delete(&lib, &protected(&t), Mode::Recycle, &bin);
        assert!(r.folder_removed);
        assert!(!lib.exists());
        assert_eq!(bin.recycled.borrow().len(), 7, "each part is one item in the Bin");
        assert!(t.path().join("bin").read_dir().unwrap().count() == 7, "nothing was erased, all of it is in the Bin");
    }

    #[test]
    fn erasing_goes_only_when_asked_and_never_follows_a_link() {
        let (t, lib) = library();
        let outside = t.path().join("Outside");
        put(&outside, &[("keep.jpg", 4)]);
        #[cfg(windows)]
        let linked = std::os::windows::fs::symlink_dir(&outside, lib.join("originals").join("linked")).is_ok()
            || std::process::Command::new("cmd").args(["/c", "mklink", "/J"]).arg(lib.join("originals").join("linked")).arg(&outside).output().is_ok_and(|o| o.status.success());
        #[cfg(not(windows))]
        let linked = std::os::unix::fs::symlink(&outside, lib.join("originals").join("linked")).is_ok();
        let bin = FakeBin::new(Some(0));
        let r = delete(&lib, &protected(&t), Mode::Erase, &bin);
        assert_eq!(r.problem, None);
        assert!(bin.recycled.borrow().is_empty(), "Erase never goes through the Bin");
        assert!(!lib.exists());
        assert!(linked, "the test could not make a link");
        assert_eq!(fs::read(outside.join("keep.jpg")).unwrap(), b"xxxx", "the link's target is untouched");
    }

    #[test]
    fn a_folder_without_a_library_is_refused_and_untouched() {
        let t = tempfile::tempdir().unwrap();
        let dir = t.path().join("Pictures");
        put(&dir, &[("originals/a.jpg", 1), ("trash/b.jpg", 1)]);
        let r = delete(&dir, &protected(&t), Mode::Erase, &FakeBin::new(Some(u64::MAX)));
        assert!(r.problem.is_some());
        assert!(dir.join("originals/a.jpg").is_file() && dir.join("trash/b.jpg").is_file());
        assert_eq!(check(&dir, &protected(&t)), Err(Refusal::NotALibrary));
    }

    #[test]
    fn a_library_holding_or_inside_lumens_own_folders_is_refused() {
        let (t, lib) = library();
        let mut p = protected(&t);
        p.app_data = Some(lib.join("originals").join("AppData"));
        assert_eq!(check(&lib, &p), Err(Refusal::HoldsLumen), "app data inside the library");
        let mut p = protected(&t);
        p.install_dir = Some(t.path().to_path_buf());
        assert_eq!(check(&lib, &p), Err(Refusal::HoldsLumen), "the library inside the install folder");
        let r = delete(&lib, &p, Mode::Erase, &FakeBin::new(Some(u64::MAX)));
        assert!(r.problem.is_some() && lib.join("library.db").is_file());
    }

    #[test]
    fn the_profile_folder_or_a_drive_root_is_refused() {
        let (t, lib) = library();
        let mut p = protected(&t);
        p.profile = Some(lib.clone());
        assert_eq!(check(&lib, &p), Err(Refusal::TooHigh));
    }

    #[test]
    fn a_library_folder_that_is_a_link_is_refused() {
        let (t, lib) = library();
        let link = t.path().join("Link to Photos");
        #[cfg(windows)]
        let ok = std::os::windows::fs::symlink_dir(&lib, &link).is_ok()
            || std::process::Command::new("cmd").args(["/c", "mklink", "/J"]).arg(&link).arg(&lib).output().is_ok_and(|o| o.status.success());
        #[cfg(not(windows))]
        let ok = std::os::unix::fs::symlink(&lib, &link).is_ok();
        assert!(ok);
        assert_eq!(check(&link, &protected(&t)), Err(Refusal::Link));
        assert!(delete(&link, &protected(&t), Mode::Erase, &FakeBin::new(Some(u64::MAX))).problem.is_some());
        assert!(lib.join("library.db").is_file());
    }

    #[test]
    fn the_bin_check_says_whether_everything_fits() {
        let (t, lib) = library();
        let total = detail("", &lib, &protected(&t)).bytes;
        let one = std::slice::from_ref(&lib);
        assert_eq!(bin_check(one, &protected(&t), &FakeBin::new(Some(total))), BinCheck { fits: true, bytes: total, room: total });
        assert!(!bin_check(one, &protected(&t), &FakeBin::new(Some(total - 1))).fits);
        assert_eq!(bin_check(&[lib], &protected(&t), &FakeBin::new(None)), BinCheck { fits: false, bytes: total, room: 0 }, "a drive without a Bin");
    }

    #[test]
    fn forget_drops_deleted_libraries_and_moves_current() {
        let t = tempfile::tempdir().unwrap();
        let app = t.path().join("AppData");
        fs::create_dir_all(&app).unwrap();
        let reg = r#"{"version":2,"libraries":[{"id":"a","name":"A","path":"C:\\A","x":1},{"id":"b","name":"B","path":"C:\\B"}],"current":"a","extra":true}"#;
        fs::write(app.join("libraries.json"), reg).unwrap();
        forget(&app, &[PathBuf::from("c:/a")]).unwrap();
        let v: serde_json::Value = serde_json::from_str(&fs::read_to_string(app.join("libraries.json")).unwrap()).unwrap();
        assert_eq!(v["libraries"].as_array().unwrap().len(), 1);
        assert_eq!(v["libraries"][0]["id"], "b");
        assert_eq!(v["current"], "b");
        assert_eq!(v["extra"], true, "fields Setup doesn't know stay");
        assert!(!app.join("libraries.json.tmp").exists());
        assert_eq!(registered(&app), vec![("B".to_string(), PathBuf::from("C:\\B"))]);
    }
}
