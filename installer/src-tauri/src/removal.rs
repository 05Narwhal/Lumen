//! Removing Lumen's app data when the person ticks *Also remove Lumen's settings, caches and
//! downloaded models* (spec *Uninstall*). Only the entries on [`ALLOW_LIST`] go; anything else is
//! kept. Nothing that equals, lies inside or contains a registered library is deleted, nor any
//! folder holding `library.db` / `library.json`, nor `backups/pre-migration/` (always kept), and
//! links are never followed. Folders are emptied file by file and removed only once empty, so the
//! app-data root is never deleted recursively. [`execute`] checks every path again at delete time.
use serde::Deserialize;
use std::fs::{self, Metadata};
use std::path::{Component, Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    File,
    Folder,
}

/// One thing Lumen creates, relative to its root (named from `photos_core::paths::AppPaths`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Entry {
    pub path: &'static str,
    pub kind: Kind,
}

const fn file(path: &'static str) -> Entry {
    Entry { path, kind: Kind::File }
}
const fn folder(path: &'static str) -> Entry {
    Entry { path, kind: Kind::Folder }
}

/// Under `config_dir/JL-Studios/Lumen/` (`AppPaths` and the writers that join onto `app_data`).
/// `.json.tmp` is the atomic-save sibling a crash can leave.
pub const ALLOW_LIST: &[Entry] = &[
    file("settings.json"),
    file("settings.json.tmp"),
    file("libraries.json"),
    file("libraries.json.tmp"),
    file("backups.json"),
    file("backups.json.tmp"),
    file("model-bench.json"),
    file("model-bench.json.tmp"),
    folder("logs"),
    folder("models"),
    folder("updates"),
    folder("cache/dng"),
];
/// Folders the list empties, removed when nothing else is in them; then the root itself.
const EMPTY_PARENTS: &[&str] = &["cache"];
/// Under `%LOCALAPPDATA%\<identifier>`: Tauri's WebView2 user data folder.
pub const WEBVIEW_ALLOW_LIST: &[Entry] = &[folder("EBWebView")];
pub const SAFETY_COPIES: &str = "backups/pre-migration";

/// `<root>\backups\pre-migration` in the system's separators (screen 6 shows it).
pub fn safety_dir(root: &Path) -> PathBuf {
    SAFETY_COPIES.split('/').fold(root.to_path_buf(), |p, part| p.join(part))
}
pub const REGISTRY_FILE: &str = "libraries.json";
const MARKERS: &[&str] = &["library.db", "library.json"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeepReason {
    /// Not something Lumen is known to create.
    NotOnList,
    /// A registered library, or inside one.
    IsLibrary,
    /// A folder with a registered library somewhere inside.
    ContainsLibrary,
    /// It, or a folder around it, holds `library.db` or `library.json`.
    HoldsLibrary,
    SafetyCopies,
    /// A symlink or junction: neither it nor what it points at is touched.
    Link,
    /// A folder that still held something at delete time.
    NotEmpty,
    /// `libraries.json` could not be read, so no library can be ruled out.
    RegistryUnreadable,
    OutsideRoot,
    Unreadable,
}

#[derive(Debug, Clone, Default)]
pub struct RemovalPlan {
    /// Files, then folders deepest first (removed only when empty), each root last.
    pub delete: Vec<PathBuf>,
    pub kept: Vec<(PathBuf, KeepReason)>,
    /// `backups/pre-migration/` when it holds anything.
    pub safety_copies: Option<PathBuf>,
    /// The roots `delete` lies under, and the libraries it was planned against; [`execute`]
    /// checks against them again.
    pub roots: Vec<PathBuf>,
    pub libraries: Vec<PathBuf>,
}

#[derive(Debug, Default)]
pub struct Report {
    pub deleted: Vec<PathBuf>,
    pub kept: Vec<(PathBuf, KeepReason)>,
    pub failed: Vec<(PathBuf, String)>,
}

/// What gets compared: the resolved path (links resolved as far as it exists, the rest appended),
/// without `\\?\`, and on Windows with `\` and in lower case, as NTFS ignores case.
pub(crate) fn norm(p: &Path) -> PathBuf {
    let abs = std::path::absolute(p).unwrap_or_else(|_| p.to_path_buf());
    let (mut cur, mut rest, mut resolved) = (abs.as_path(), Vec::new(), None);
    loop {
        if let Ok(c) = fs::canonicalize(cur) {
            resolved = Some(c);
            break;
        }
        match (cur.parent(), cur.file_name()) {
            (Some(parent), Some(name)) => {
                rest.push(name.to_owned());
                cur = parent;
            }
            _ => break,
        }
    }
    let mut out = match resolved {
        Some(c) => c,
        None => {
            rest.clear();
            abs.clone()
        }
    };
    for name in rest.iter().rev() {
        out.push(name);
    }
    let s = out.to_string_lossy();
    let s = if let Some(r) = s.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{r}")
    } else if let Some(r) = s.strip_prefix(r"\\?\") {
        r.to_string()
    } else {
        s.into_owned()
    };
    #[cfg(windows)]
    let s = s.replace('/', "\\").to_lowercase();
    PathBuf::from(s)
}

/// A symlink, junction or any other reparse point: never followed, never deleted.
pub(crate) fn is_link(m: &Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
        if m.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return true;
        }
    }
    m.file_type().is_symlink()
}

fn holds_marker(dir: &Path) -> bool {
    MARKERS.iter().any(|m| fs::symlink_metadata(dir.join(m)).is_ok())
}

fn has_anything(dir: &Path) -> bool {
    fs::read_dir(dir).map(|mut r| r.next().is_some()).unwrap_or(false)
}

/// One root's rules, used when planning and again when deleting.
struct Guard {
    raw: PathBuf,
    root: PathBuf,
    safety: PathBuf,
    libraries: Vec<PathBuf>,
}

impl Guard {
    fn new(raw: &Path, libraries: &[PathBuf]) -> Self {
        Self { raw: raw.to_path_buf(), root: norm(raw), safety: norm(&raw.join(SAFETY_COPIES)), libraries: libraries.iter().map(|l| norm(l)).collect() }
    }

    /// Why `path` must stay, whatever it turns out to be.
    fn refuse(&self, path: &Path) -> Option<KeepReason> {
        let Ok(rel) = path.strip_prefix(&self.raw) else { return Some(KeepReason::OutsideRoot) };
        if rel.components().any(|c| !matches!(c, Component::Normal(_))) {
            return Some(KeepReason::OutsideRoot);
        }
        // Resolved, so a link anywhere above `path` that leads out is caught here.
        let p = norm(path);
        if !p.starts_with(&self.root) {
            return Some(KeepReason::OutsideRoot);
        }
        if p.starts_with(&self.safety) {
            return Some(KeepReason::SafetyCopies);
        }
        if self.libraries.iter().any(|l| p.starts_with(l)) {
            return Some(KeepReason::IsLibrary);
        }
        // The root and every folder between it and `path`.
        let mut dir = self.raw.clone();
        if holds_marker(&dir) {
            return Some(KeepReason::HoldsLibrary);
        }
        let parts: Vec<_> = rel.components().collect();
        for part in parts.iter().take(parts.len().saturating_sub(1)) {
            dir.push(part);
            if holds_marker(&dir) {
                return Some(KeepReason::HoldsLibrary);
            }
        }
        None
    }

    fn contains_library(&self, path: &Path) -> bool {
        let p = norm(path);
        self.libraries.iter().any(|l| l.starts_with(&p))
    }
}

fn walk(g: &Guard, path: &Path, plan: &mut RemovalPlan) {
    let Ok(meta) = fs::symlink_metadata(path) else { return };
    let keep = |plan: &mut RemovalPlan, r| plan.kept.push((path.to_path_buf(), r));
    if is_link(&meta) {
        return keep(plan, KeepReason::Link);
    }
    if let Some(r) = g.refuse(path) {
        return keep(plan, r);
    }
    if !meta.is_dir() {
        return plan.delete.push(path.to_path_buf());
    }
    if holds_marker(path) {
        return keep(plan, KeepReason::HoldsLibrary);
    }
    let Ok(read) = fs::read_dir(path) else { return keep(plan, KeepReason::Unreadable) };
    let mut children: Vec<PathBuf> = read.flatten().map(|e| e.path()).collect();
    children.sort();
    for child in &children {
        walk(g, child, plan);
    }
    if g.contains_library(path) {
        keep(plan, KeepReason::ContainsLibrary);
    } else {
        plan.delete.push(path.to_path_buf());
    }
}

fn plan_root(root: &Path, list: &[Entry], parents: &[&str], libraries: &[PathBuf], plan: &mut RemovalPlan) {
    let Ok(meta) = fs::symlink_metadata(root) else { return };
    let keep = |plan: &mut RemovalPlan, r| plan.kept.push((root.to_path_buf(), r));
    if is_link(&meta) {
        return keep(plan, KeepReason::Link);
    }
    if !meta.is_dir() || !root.is_absolute() || root.parent().is_none() {
        return keep(plan, KeepReason::OutsideRoot);
    }
    let g = Guard::new(root, libraries);
    if let Some(r) = g.refuse(root) {
        return keep(plan, r);
    }
    plan.roots.push(root.to_path_buf());
    for entry in list {
        let path = entry.path.split('/').fold(root.to_path_buf(), |p, part| p.join(part));
        let Ok(m) = fs::symlink_metadata(&path) else { continue };
        let fits = match entry.kind {
            Kind::File => m.is_file(),
            Kind::Folder => m.is_dir(),
        };
        if fits || is_link(&m) {
            walk(&g, &path, plan);
        } else {
            plan.kept.push((path, KeepReason::NotOnList));
        }
    }
    let firsts: Vec<&str> = list.iter().filter_map(|e| e.path.split('/').next()).collect();
    if let Ok(read) = fs::read_dir(root) {
        let mut names: Vec<PathBuf> = read.flatten().map(|e| e.path()).collect();
        names.sort();
        for p in names {
            if p.file_name().and_then(|n| n.to_str()).is_some_and(|n| firsts.contains(&n)) {
                continue;
            }
            let copies = safety_dir(root);
            if copies.parent() == Some(p.as_path()) && copies.exists() {
                plan.kept.push((copies, KeepReason::SafetyCopies));
            } else {
                plan.kept.push((p, KeepReason::NotOnList));
            }
        }
    }
    for parent in parents {
        let path = root.join(parent);
        if fs::symlink_metadata(&path).is_ok_and(|m| m.is_dir() && !is_link(&m)) && g.refuse(&path).is_none() {
            plan.delete.push(path);
        }
    }
    if g.contains_library(root) {
        keep(plan, KeepReason::ContainsLibrary);
    } else {
        plan.delete.push(root.to_path_buf());
    }
}

/// What the box would take, read-only. Unticked, `delete` is empty; `safety_copies` is reported
/// either way. `libraries` are every registered library ([`read_libraries`]).
pub fn plan_removal(app_data: &Path, webview_data: &Path, libraries: &[PathBuf], remove_data: bool) -> RemovalPlan {
    let copies = safety_dir(app_data);
    let mut plan = RemovalPlan { libraries: libraries.to_vec(), safety_copies: has_anything(&copies).then_some(copies), ..Default::default() };
    if remove_data {
        plan_root(app_data, ALLOW_LIST, EMPTY_PARENTS, libraries, &mut plan);
        plan_root(webview_data, WEBVIEW_ALLOW_LIST, &[], libraries, &mut plan);
    }
    plan
}

/// Delete `plan.delete` in order, each path checked again first (a library or `library.db` may
/// have appeared since, or the plan may be wrong). Files are removed one by one and folders only
/// when empty: nothing here deletes recursively.
pub fn execute(plan: &RemovalPlan) -> Report {
    let guards: Vec<Guard> = plan.roots.iter().map(|r| Guard::new(r, &plan.libraries)).collect();
    let mut report = Report::default();
    for path in &plan.delete {
        let Some(g) = guards.iter().filter(|g| path.starts_with(&g.raw)).max_by_key(|g| g.raw.components().count()) else {
            report.kept.push((path.clone(), KeepReason::OutsideRoot));
            continue;
        };
        let Ok(meta) = fs::symlink_metadata(path) else { continue };
        let dir = meta.is_dir();
        let why = if is_link(&meta) {
            Some(KeepReason::Link)
        } else if let Some(r) = g.refuse(path) {
            Some(r)
        } else if dir && holds_marker(path) {
            Some(KeepReason::HoldsLibrary)
        } else if dir && g.contains_library(path) {
            Some(KeepReason::ContainsLibrary)
        } else if dir && has_anything(path) {
            Some(KeepReason::NotEmpty)
        } else {
            None
        };
        if let Some(r) = why {
            report.kept.push((path.clone(), r));
            continue;
        }
        let done = if dir { fs::remove_dir(path) } else { fs::remove_file(path) };
        match done {
            Ok(()) => report.deleted.push(path.clone()),
            Err(e) => report.failed.push((path.clone(), e.to_string())),
        }
    }
    report
}

#[derive(Deserialize)]
struct RegistryFile {
    libraries: Vec<RegistryEntry>,
}

#[derive(Deserialize)]
struct RegistryEntry {
    path: String,
}

fn parse_registry(text: &str) -> serde_json::Result<Vec<PathBuf>> {
    Ok(serde_json::from_str::<RegistryFile>(text)?.libraries.into_iter().map(|e| PathBuf::from(e.path)).collect())
}

/// Every library `libraries.json` names (`photos_core::registry::LibraryRegistry`), plus any in a
/// `libraries.json.tmp` an interrupted save left. No file is no libraries; a file that can't be
/// read or parsed is an error, and then nothing may be deleted.
pub fn read_libraries(app_data: &Path) -> Result<Vec<PathBuf>, String> {
    let main = app_data.join(REGISTRY_FILE);
    let mut out = match fs::read_to_string(&main) {
        Ok(text) => parse_registry(&text).map_err(|e| format!("{}: {e}", main.display()))?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
        Err(e) => return Err(format!("{}: {e}", main.display())),
    };
    if let Some(more) = fs::read_to_string(app_data.join("libraries.json.tmp")).ok().and_then(|t| parse_registry(&t).ok()) {
        for p in more {
            if !out.contains(&p) {
                out.push(p);
            }
        }
    }
    Ok(out)
}

/// [`plan_removal`] against the registered libraries; an unreadable registry plans nothing.
pub fn plan_for(app_data: &Path, webview_data: &Path, remove_data: bool) -> RemovalPlan {
    match read_libraries(app_data) {
        Ok(libraries) => plan_removal(app_data, webview_data, &libraries, remove_data),
        Err(_) => {
            let mut plan = plan_removal(app_data, webview_data, &[], false);
            plan.kept.push((app_data.join(REGISTRY_FILE), KeepReason::RegistryUnreadable));
            plan
        }
    }
}

/// What screen 6 shows, read-only.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Summary {
    pub libraries: Vec<PathBuf>,
    /// Bytes of the files the box would remove.
    pub app_data_bytes: u64,
    pub safety_copies: Option<PathBuf>,
}

pub fn summary(app_data: &Path, webview_data: &Path) -> Summary {
    let plan = plan_for(app_data, webview_data, true);
    let app_data_bytes = plan.delete.iter().filter_map(|p| fs::symlink_metadata(p).ok()).filter(|m| m.is_file()).map(|m| m.len()).sum();
    Summary { libraries: read_libraries(app_data).unwrap_or_default(), app_data_bytes, safety_copies: plan.safety_copies }
}

/// Lumen's app data as `AppPaths::resolve` finds it: `LUMEN_APP_DATA_DIR`, a test build's baked
/// folder, else `%APPDATA%\JL-Studios\Lumen`.
pub fn app_data_dir() -> Option<PathBuf> {
    std::env::var_os("LUMEN_APP_DATA_DIR")
        .or_else(|| option_env!("LUMEN_BAKED_APP_DATA_DIR").map(Into::into))
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("APPDATA").map(|a| PathBuf::from(a).join("JL-Studios").join("Lumen")))
}

/// Tauri's WebView2 data: `%LOCALAPPDATA%\<identifier>` (`EBWebView` inside). A test build bakes
/// its identifier in `LUMEN_SETUP_IDENTIFIER`.
pub fn webview_data_dir() -> Option<PathBuf> {
    let identifier = option_env!("LUMEN_SETUP_IDENTIFIER").unwrap_or("com.jlstudios.lumen");
    std::env::var_os("LOCALAPPDATA").map(|l| PathBuf::from(l).join(identifier))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    struct Tree {
        _t: tempfile::TempDir,
        base: PathBuf,
        app: PathBuf,
        web: PathBuf,
    }

    fn tree() -> Tree {
        let t = tempfile::tempdir().unwrap();
        let base = fs::canonicalize(t.path()).unwrap();
        let (app, web) = (base.join("Roaming/JL-Studios/Lumen"), base.join("Local/com.jlstudios.lumen"));
        fs::create_dir_all(&app).unwrap();
        fs::create_dir_all(&web).unwrap();
        Tree { _t: t, base, app, web }
    }

    fn put(root: &Path, files: &[&str]) {
        for f in files {
            let p = root.join(f);
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            fs::write(&p, format!("bytes of {f}")).unwrap();
        }
    }

    /// Every file under `dir` with its contents, to prove a library kept every byte.
    fn snapshot(dir: &Path) -> Vec<(PathBuf, Vec<u8>)> {
        let mut out = Vec::new();
        let mut stack = vec![dir.to_path_buf()];
        while let Some(d) = stack.pop() {
            for e in fs::read_dir(&d).unwrap().flatten() {
                let m = fs::symlink_metadata(e.path()).unwrap();
                if m.is_dir() {
                    stack.push(e.path());
                } else {
                    out.push((e.path(), fs::read(e.path()).unwrap()));
                }
            }
        }
        out.sort();
        out
    }

    fn lumen_files(t: &Tree) {
        put(&t.app, &[
            "settings.json", "settings.json.tmp", "libraries.json", "backups.json", "model-bench.json",
            "logs/lumen-20261106-01.log", "models/clip-vit-b32.onnx", "models/dev/retouch.json",
            "updates/pending.json", "updates/update-26.1.1-alpha.bin.part", "cache/dng/abc.dng",
        ]);
        put(&t.web, &["EBWebView/Default/Cache/data_0", "EBWebView/Local State"]);
    }

    fn registry(app: &Path, libraries: &[&Path]) {
        let entries: Vec<_> = libraries
            .iter()
            .enumerate()
            .map(|(i, p)| serde_json::json!({ "id": format!("lib-{i}"), "name": "L", "path": p.to_string_lossy(), "lastOpenedAt": 0 }))
            .collect();
        fs::write(app.join(REGISTRY_FILE), serde_json::json!({ "version": 2, "libraries": entries, "current": null }).to_string()).unwrap();
    }

    fn library(at: &Path) {
        put(at, &["library.db", "library.json", "originals/2026/IMG_0001.HEIC", "derivatives/thumbs/ab/x.jpg"]);
    }

    fn run(t: &Tree, libraries: &[PathBuf]) -> (RemovalPlan, Report) {
        let plan = plan_removal(&t.app, &t.web, libraries, true);
        let report = execute(&plan);
        (plan, report)
    }

    #[test]
    fn only_allow_listed_entries_are_deleted() {
        let t = tree();
        lumen_files(&t);
        let (_, report) = run(&t, &[]);
        assert!(report.failed.is_empty(), "{:?}", report.failed);
        assert!(!t.app.exists(), "nothing but Lumen's own entries was there, so the root is empty and goes");
        assert!(!t.web.exists());
        assert!(t.app.parent().unwrap().is_dir(), "JL-Studios stays: other apps share it");
    }

    #[test]
    fn unknown_files_are_kept() {
        let t = tree();
        lumen_files(&t);
        put(&t.app, &["notes.txt", "Other/thing.bin", "cache/other/x.bin", "backups/elsewhere.json"]);
        put(&t.web, &["Other/y"]);
        let (plan, _) = run(&t, &[]);
        for f in ["notes.txt", "Other/thing.bin", "cache/other/x.bin", "backups/elsewhere.json"] {
            assert!(t.app.join(f).is_file(), "{f} is not Lumen's and stays");
        }
        assert!(t.web.join("Other/y").is_file());
        assert!(!t.app.join("settings.json").exists() && !t.app.join("models").exists() && !t.web.join("EBWebView").exists());
        assert!(plan.kept.iter().any(|(p, r)| p == &t.app.join("notes.txt") && *r == KeepReason::NotOnList));
    }

    #[test]
    fn library_inside_app_data_survives() {
        let t = tree();
        lumen_files(&t);
        let (top, in_models, unregistered) = (t.app.join("My Library"), t.app.join("models/Lib2"), t.app.join("logs/Old"));
        library(&top);
        library(&in_models);
        put(&unregistered, &["library.json", "originals/a.jpg"]);
        // Registered but holding no library.db yet (being created): only the registry protects it.
        let bare = t.app.join("updates/Bare");
        put(&bare, &["originals/b.jpg"]);
        registry(&t.app, &[&top, &in_models, &bare]);
        let before = [snapshot(&top), snapshot(&in_models), snapshot(&unregistered), snapshot(&bare)];
        let libs = read_libraries(&t.app).unwrap();
        assert_eq!(libs.len(), 3);
        let (plan, _) = run(&t, &libs);
        assert_eq!(before, [snapshot(&top), snapshot(&in_models), snapshot(&unregistered), snapshot(&bare)]);
        assert!(!t.app.join("updates/pending.json").exists());
        assert!(!t.app.join("models/clip-vit-b32.onnx").exists(), "the rest of models still goes");
        assert!(!t.app.join("logs/lumen-20261106-01.log").exists());
        assert!(t.app.join("models").is_dir() && t.app.is_dir());
        assert!(plan.kept.iter().any(|(p, r)| p == &t.app.join("models") && *r == KeepReason::ContainsLibrary));
        assert!(plan.kept.iter().any(|(p, r)| p == &unregistered && *r == KeepReason::HoldsLibrary));
    }

    #[test]
    fn app_data_inside_library_is_untouched() {
        let t = tree();
        let lib = t.base.join("Pictures/Lumen Library");
        library(&lib);
        let app = lib.join("appdata");
        let web = lib.join("webview");
        put(&app, &["settings.json", "libraries.json", "logs/a.log", "models/m.onnx"]);
        put(&web, &["EBWebView/x"]);
        // Registered as the person typed it: another case and forward slashes on Windows.
        let typed = if cfg!(windows) { PathBuf::from(lib.to_string_lossy().to_uppercase().replace('\\', "/")) } else { lib.clone() };
        let before = snapshot(&lib);
        let plan = plan_removal(&app, &web, &[typed], true);
        assert!(plan.delete.is_empty(), "{:?}", plan.delete);
        execute(&plan);
        assert_eq!(before, snapshot(&lib));
    }

    #[test]
    fn library_db_in_a_cache_folder_keeps_that_folder() {
        let t = tree();
        put(&t.app, &["cache/dng/sub/library.db", "cache/dng/sub/a.dng", "cache/dng/b.dng"]);
        put(&t.web, &["EBWebView/inner/library.json", "EBWebView/inner/z"]);
        run(&t, &[]);
        assert!(t.app.join("cache/dng/sub/library.db").is_file() && t.app.join("cache/dng/sub/a.dng").is_file());
        assert!(!t.app.join("cache/dng/b.dng").exists());
        assert!(t.web.join("EBWebView/inner/z").is_file());
    }

    #[test]
    fn pre_migration_copies_are_always_kept() {
        let t = tree();
        lumen_files(&t);
        put(&t.app, &["backups/pre-migration/lib-1/library-0032.db"]);
        let copies = t.app.join(SAFETY_COPIES);
        assert_eq!(plan_removal(&t.app, &t.web, &[], false).safety_copies, Some(copies.clone()));
        let (plan, report) = run(&t, &[]);
        assert_eq!(plan.safety_copies, Some(copies.clone()));
        assert!(!plan.delete.iter().any(|p| p.starts_with(&copies)));
        assert!(copies.join("lib-1/library-0032.db").is_file());
        assert!(!t.app.join("backups.json").exists(), "the backup schedules go with the box");
        assert!(t.app.is_dir() && report.failed.is_empty());
        let left: Vec<_> = fs::read_dir(&t.app).unwrap().flatten().map(|e| e.file_name()).collect();
        assert_eq!(left, ["backups"], "app data stays with only the safety copies");
    }

    #[test]
    fn registry_file_goes_only_with_the_box() {
        let t = tree();
        lumen_files(&t);
        let lib = t.base.join("Pictures/Lumen Library");
        library(&lib);
        registry(&t.app, &[&lib]);
        let off = plan_removal(&t.app, &t.web, std::slice::from_ref(&lib), false);
        assert!(off.delete.is_empty());
        execute(&off);
        assert!(t.app.join(REGISTRY_FILE).is_file() && t.app.join("models/clip-vit-b32.onnx").is_file());
        let before = snapshot(&lib);
        execute(&plan_removal(&t.app, &t.web, std::slice::from_ref(&lib), true));
        assert!(!t.app.join(REGISTRY_FILE).exists());
        assert_eq!(before, snapshot(&lib), "the library it pointed at stays where it is");
    }

    #[test]
    fn the_root_is_never_deleted_recursively() {
        let t = tree();
        lumen_files(&t);
        put(&t.app, &["mine.txt"]);
        let plan = plan_removal(&t.app, &t.web, &[], true);
        // Lumen (or anyone) writes after the plan was made: a planned folder is no longer empty.
        put(&t.app, &["logs/written-after.log"]);
        let report = execute(&plan);
        assert!(t.app.join("mine.txt").is_file() && t.app.join("logs/written-after.log").is_file());
        assert!(report.kept.iter().any(|(p, r)| p == &t.app.join("logs") && *r == KeepReason::NotEmpty));
        assert!(report.kept.iter().any(|(p, r)| p == &t.app && *r == KeepReason::NotEmpty));
    }

    fn link_dir(link: &Path, target: &Path) {
        #[cfg(windows)]
        {
            let ok = std::process::Command::new("cmd").arg("/C").arg("mklink").arg("/J").arg(link).arg(target).output().unwrap();
            assert!(ok.status.success(), "{}", String::from_utf8_lossy(&ok.stderr));
        }
        #[cfg(unix)]
        std::os::unix::fs::symlink(target, link).unwrap();
    }

    #[test]
    fn a_symlink_or_junction_out_of_app_data_is_not_followed() {
        let t = tree();
        lumen_files(&t);
        let (outside, outside2) = (t.base.join("Elsewhere"), t.base.join("Elsewhere2"));
        put(&outside, &["precious.jpg", "deep/more.jpg"]);
        put(&outside2, &["also.jpg"]);
        link_dir(&t.app.join("models/linked"), &outside);
        fs::remove_dir_all(t.app.join("updates")).unwrap();
        link_dir(&t.app.join("updates"), &outside2);
        let before = (snapshot(&outside), snapshot(&outside2));
        let (plan, _) = run(&t, &[]);
        assert_eq!(before, (snapshot(&outside), snapshot(&outside2)));
        for l in [t.app.join("models/linked"), t.app.join("updates")] {
            assert!(plan.kept.contains(&(l.clone(), KeepReason::Link)), "{l:?}");
            assert!(fs::symlink_metadata(&l).is_ok(), "the link itself is left too");
        }
        assert!(!t.app.join("models/clip-vit-b32.onnx").exists());
    }

    #[test]
    fn execute_checks_each_path_again_at_delete_time() {
        let t = tree();
        lumen_files(&t);
        let lib = t.base.join("Lib");
        library(&lib);
        let mut plan = plan_removal(&t.app, &t.web, std::slice::from_ref(&lib), true);
        // A library appears in a planned folder after the plan, and a forged entry names a library file.
        put(&t.app, &["cache/dng/library.db"]);
        plan.delete.insert(0, lib.join("library.db"));
        plan.delete.insert(0, lib.join("originals/2026/IMG_0001.HEIC"));
        let before = snapshot(&lib);
        let report = execute(&plan);
        assert_eq!(before, snapshot(&lib));
        assert!(t.app.join("cache/dng/abc.dng").is_file() && t.app.join("cache/dng/library.db").is_file());
        assert!(report.kept.iter().any(|(p, r)| p == &t.app.join("cache/dng/abc.dng") && *r == KeepReason::HoldsLibrary));
        assert!(report.kept.iter().any(|(p, r)| p == &lib.join("library.db") && *r == KeepReason::OutsideRoot));
    }

    #[test]
    fn an_unreadable_registry_deletes_nothing() {
        let t = tree();
        lumen_files(&t);
        fs::write(t.app.join(REGISTRY_FILE), "{ not json").unwrap();
        assert!(read_libraries(&t.app).is_err());
        let plan = plan_for(&t.app, &t.web, true);
        assert!(plan.delete.is_empty());
        assert!(plan.kept.iter().any(|(_, r)| *r == KeepReason::RegistryUnreadable));
    }

    #[test]
    fn a_half_written_registry_copy_counts_too() {
        let t = tree();
        let lib = t.app.join("New Library");
        put(&lib, &["originals/a.jpg"]); // not yet holding library.db: only the registry names it
        registry(&t.app, &[]);
        let tmp = t.app.join("libraries.json.tmp");
        fs::write(&tmp, serde_json::json!({ "version": 2, "libraries": [{ "id": "n", "name": "N", "path": lib.to_string_lossy(), "lastOpenedAt": 0 }], "current": "n" }).to_string()).unwrap();
        assert_eq!(read_libraries(&t.app).unwrap(), vec![lib.clone()]);
    }

    #[test]
    fn the_summary_sizes_what_would_go() {
        let t = tree();
        put(&t.app, &["settings.json", "notes.txt", "backups/pre-migration/k/library-1.db"]);
        let s = summary(&t.app, &t.web);
        assert_eq!(s.app_data_bytes, "bytes of settings.json".len() as u64);
        assert_eq!(s.safety_copies, Some(t.app.join(SAFETY_COPIES)));
        assert!(s.libraries.is_empty());
        assert!(t.app.join("settings.json").is_file(), "a summary deletes nothing");
    }

    #[test]
    fn the_list_names_every_file_app_paths_names() {
        let root = Path::new("/a");
        let p = photos_core::paths::AppPaths::new(root, "/lib");
        let on = |path: &Path, kind: Kind| {
            let rel = path.strip_prefix(root).unwrap().to_string_lossy().replace('\\', "/");
            ALLOW_LIST.iter().any(|e| e.path == rel && e.kind == kind)
        };
        for f in [p.settings_file(), p.registry_file(), p.backups_file()] {
            assert!(on(&f, Kind::File), "{f:?}");
            assert!(on(&f.with_extension("json.tmp"), Kind::File), "{f:?}'s atomic-save sibling");
        }
        for d in [p.logs_dir(), p.models_dir(), p.updates_dir()] {
            assert!(on(&d, Kind::Folder), "{d:?}");
        }
        assert_eq!(p.pre_migration_dir(), root.join(SAFETY_COPIES));
        assert_eq!(p.registry_file(), root.join(REGISTRY_FILE));
    }

    #[test]
    fn the_registry_is_read_as_lumen_writes_it() {
        let t = tree();
        let mut reg = photos_core::registry::LibraryRegistry::default();
        reg.upsert("id-1", "Lumen Library", &t.base.join("Pictures/Lumen Library"), 1);
        reg.save(&t.app.join(REGISTRY_FILE)).unwrap();
        assert_eq!(read_libraries(&t.app).unwrap(), vec![t.base.join("Pictures/Lumen Library")]);
    }
}
