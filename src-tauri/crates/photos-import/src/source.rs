use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use photos_core::extras::ItemExtras;

use crate::ImportError;

/// One file offered by a source. `rel_path` is relative to the source root and is
/// only used for labels and failure reports.
#[derive(Debug, Clone)]
pub struct SourceItem {
    pub rel_path: String,
    pub file_name: String,
    pub size: u64,
    pub modified_at: Option<i64>,
    /// When the file was created, where the source's system records it (Round 8A).
    pub created_at: Option<i64>,
}

/// Folders, memory cards and iPhones all look like this to the ingest pipeline,
/// which is written once against `Read`.
pub trait ImportSource: Send + Sync {
    fn label(&self) -> String;
    /// `photos_core::batches::SOURCE_*`
    fn kind(&self) -> i64;
    fn enumerate(&self) -> Result<Vec<SourceItem>, ImportError>;
    fn open(&self, item: &SourceItem) -> Result<Box<dyn Read + Send>, ImportError>;
    /// Catalogue facts the source knows beyond the file (Apple Photos: favourite, albums,
    /// text, date, place). Applied to the asset, new or duplicate. Default: none.
    fn extras(&self, _item: &SourceItem) -> Option<ItemExtras> {
        None
    }
    /// Whether the source is still there. Asked when a file fails to open: a card pulled out
    /// or a phone unplugged stops the import as *interrupted* instead of failing every file
    /// that is left (round 3). Default: always there.
    fn is_connected(&self) -> bool {
        true
    }
    /// The `.xmp`, `.aae` and `.thm` files that belong to `item`, as items this source can
    /// `open` (Round 19 C3). Default: none.
    fn sidecars(&self, _item: &SourceItem) -> Vec<SourceItem> {
        Vec::new()
    }
    /// Files the source holds that are neither media nor a sidecar of a media file, with the
    /// reason they are not imported (`photos_core::sidecars::SKIP_*`). Default: none.
    fn leftovers(&self) -> Vec<(String, &'static str)> {
        Vec::new()
    }
}

/// A shared source is a source: an Apple library opened once per session (its database
/// copy is expensive) is imported and previewed through clones of one `Arc`.
impl<T: ImportSource + ?Sized> ImportSource for std::sync::Arc<T> {
    fn label(&self) -> String {
        (**self).label()
    }
    fn kind(&self) -> i64 {
        (**self).kind()
    }
    fn enumerate(&self) -> Result<Vec<SourceItem>, ImportError> {
        (**self).enumerate()
    }
    fn open(&self, item: &SourceItem) -> Result<Box<dyn Read + Send>, ImportError> {
        (**self).open(item)
    }
    fn extras(&self, item: &SourceItem) -> Option<ItemExtras> {
        (**self).extras(item)
    }
    fn is_connected(&self) -> bool {
        (**self).is_connected()
    }
    fn sidecars(&self, item: &SourceItem) -> Vec<SourceItem> {
        (**self).sidecars(item)
    }
    fn leftovers(&self) -> Vec<(String, &'static str)> {
        (**self).leftovers()
    }
}

/// The names in the folders a sidecar lookup has already listed (lower-cased name → real
/// name), so a folder of thousands of photos is read once, not once per photo.
#[derive(Default)]
struct DirNames(Mutex<HashMap<PathBuf, Arc<HashMap<String, String>>>>);

impl DirNames {
    fn of(&self, dir: &Path) -> Arc<HashMap<String, String>> {
        let mut cache = self.0.lock().unwrap_or_else(|e| e.into_inner());
        cache
            .entry(dir.to_path_buf())
            .or_insert_with(|| {
                let names = std::fs::read_dir(dir)
                    .into_iter()
                    .flatten()
                    .flatten()
                    .filter_map(|e| e.file_name().into_string().ok())
                    .map(|n| (n.to_lowercase(), n))
                    .collect();
                Arc::new(names)
            })
            .clone()
    }
}

// A cache is not part of what a source is: clones start empty, and two sources are the same
// source when their folders are.
impl Clone for DirNames {
    fn clone(&self) -> Self {
        Self::default()
    }
}
impl PartialEq for DirNames {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}
impl Eq for DirNames {}
impl std::fmt::Debug for DirNames {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("DirNames")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FolderSource {
    root: PathBuf,
    recursive: bool,
    names: DirNames,
}

impl FolderSource {
    /// A folder and everything under it.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into(), recursive: true, names: DirNames::default() }
    }

    /// `false` imports the folder's own files and leaves its subfolders alone.
    pub fn recursive(mut self, recursive: bool) -> Self {
        self.recursive = recursive;
        self
    }

    pub fn root(&self) -> &Path {
        &self.root
    }
}

fn is_hidden(entry: &walkdir::DirEntry) -> bool {
    entry.file_name().to_str().is_some_and(|n| n.starts_with('.') || n.eq_ignore_ascii_case("@eaDir"))
}

impl ImportSource for FolderSource {
    fn label(&self) -> String {
        self.root.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| self.root.display().to_string())
    }

    fn kind(&self) -> i64 {
        photos_core::batches::SOURCE_FOLDER
    }

    fn enumerate(&self) -> Result<Vec<SourceItem>, ImportError> {
        if !self.root.is_dir() {
            return Err(ImportError::Source(format!("{} is not a folder", self.root.display())));
        }
        let mut items = Vec::new();
        let depth = if self.recursive { usize::MAX } else { 1 };
        let walker = walkdir::WalkDir::new(&self.root).follow_links(false).max_depth(depth).into_iter();
        // Hidden files and folders inside the chosen folder are skipped; the chosen folder
        // itself never is, even when its own name starts with a dot.
        for entry in walker.filter_entry(|e| e.depth() == 0 || !is_hidden(e)) {
            let entry = match entry {
                Ok(e) => e,
                Err(e) => {
                    tracing::warn!("skipping unreadable entry: {e}");
                    continue;
                }
            };
            if !entry.file_type().is_file() || !photos_media::is_media_file(entry.path()) {
                continue;
            }
            let meta = entry.metadata().ok();
            let rel = entry.path().strip_prefix(&self.root).unwrap_or(entry.path());
            items.push(SourceItem {
                rel_path: rel.to_string_lossy().replace('\\', "/"),
                file_name: entry.file_name().to_string_lossy().into_owned(),
                size: meta.as_ref().map(|m| m.len()).unwrap_or(0),
                modified_at: meta.as_ref().and_then(|m| unix_secs(m.modified())),
                created_at: meta.as_ref().and_then(|m| unix_secs(m.created())),
            });
        }
        items.sort_by(|a, b| a.rel_path.cmp(&b.rel_path));
        Ok(items)
    }

    fn open(&self, item: &SourceItem) -> Result<Box<dyn Read + Send>, ImportError> {
        let path = self.root.join(&item.rel_path);
        Ok(Box::new(std::fs::File::open(path)?))
    }

    /// A folder on a disk that was unplugged is gone.
    fn is_connected(&self) -> bool {
        self.root.is_dir()
    }

    /// `IMG_0001.xmp` and `IMG_0001.jpg.xmp` (Lightroom's and darktable's spelling) beside
    /// `IMG_0001.jpg`, one per kind, in any letter case.
    fn sidecars(&self, item: &SourceItem) -> Vec<SourceItem> {
        let path = self.root.join(&item.rel_path);
        let (Some(dir), Some(stem)) = (path.parent(), path.file_stem().and_then(|s| s.to_str())) else { return Vec::new() };
        let names = self.names.of(dir);
        let rel_dir = item.rel_path.rsplit_once('/').map_or("", |(d, _)| d);
        let mut found: Vec<(&'static str, SourceItem)> = Vec::new();
        for base in [stem, item.file_name.as_str()] {
            for ext in SIDECAR_EXTENSIONS {
                let Some(kind) = photos_core::sidecars::kind_for_extension(ext) else { continue };
                if found.iter().any(|(k, _)| *k == kind) {
                    continue;
                }
                let Some(real) = names.get(&format!("{base}.{ext}").to_lowercase()) else { continue };
                let rel = if rel_dir.is_empty() { real.clone() } else { format!("{rel_dir}/{real}") };
                if let Some(side) = sidecar_item(&self.root.join(&rel), rel) {
                    found.push((kind, side));
                }
            }
        }
        found.into_iter().map(|(_, s)| s).collect()
    }

    /// Every visible file that is not media and not the sidecar of a media file.
    fn leftovers(&self) -> Vec<(String, &'static str)> {
        let depth = if self.recursive { usize::MAX } else { 1 };
        let walker = walkdir::WalkDir::new(&self.root).follow_links(false).max_depth(depth).into_iter();
        let mut media: std::collections::HashSet<String> = std::collections::HashSet::new();
        let mut others: Vec<String> = Vec::new();
        for entry in walker.filter_entry(|e| e.depth() == 0 || !is_hidden(e)).flatten() {
            if !entry.file_type().is_file() {
                continue;
            }
            let rel = entry.path().strip_prefix(&self.root).unwrap_or(entry.path()).to_string_lossy().replace('\\', "/");
            if photos_media::is_media_file(entry.path()) {
                media.insert(rel.to_lowercase());
            } else {
                others.push(rel);
            }
        }
        let mut out: Vec<(String, &'static str)> = others
            .into_iter()
            .filter(|rel| !has_media_partner(rel, &media))
            .map(|rel| (rel, photos_core::sidecars::SKIP_UNSUPPORTED))
            .collect();
        out.sort();
        out
    }
}

const SIDECAR_EXTENSIONS: [&str; 3] = ["xmp", "aae", "thm"];

/// A sidecar file as an item: any regular file, whatever its type.
fn sidecar_item(path: &Path, rel_path: String) -> Option<SourceItem> {
    let meta = std::fs::metadata(path).ok().filter(|m| m.is_file())?;
    Some(SourceItem {
        rel_path,
        file_name: path.file_name()?.to_string_lossy().into_owned(),
        size: meta.len(),
        modified_at: unix_secs(meta.modified()),
        created_at: unix_secs(meta.created()),
    })
}

/// Whether `rel` is a sidecar whose media file is in `media` (lower-cased relative paths):
/// `a.xmp` beside `a.<anything media>`, or `a.jpg.xmp` beside `a.jpg`.
fn has_media_partner(rel: &str, media: &std::collections::HashSet<String>) -> bool {
    let lower = rel.to_lowercase();
    let Some((base, ext)) = lower.rsplit_once('.') else { return false };
    if photos_core::sidecars::kind_for_extension(ext).is_none() {
        return false;
    }
    media.contains(base) || media.iter().any(|m| m.rsplit_once('.').is_some_and(|(stem, _)| stem == base))
}

fn file_item(path: &Path, rel_path: String) -> Option<SourceItem> {
    let meta = std::fs::metadata(path).ok()?;
    if !meta.is_file() || !photos_media::is_media_file(path) {
        return None;
    }
    Some(SourceItem {
        rel_path,
        file_name: path.file_name()?.to_string_lossy().into_owned(),
        size: meta.len(),
        modified_at: unix_secs(meta.modified()),
        created_at: unix_secs(meta.created()),
    })
}

/// A file time as Unix seconds; `None` where the system does not keep it.
fn unix_secs(t: std::io::Result<std::time::SystemTime>) -> Option<i64> {
    t.ok()?.duration_since(std::time::UNIX_EPOCH).ok().map(|d| d.as_secs() as i64)
}

/// Files the user picked one by one, from anywhere. Anything that is not a photo or a
/// video, or no longer exists, is left out rather than failing the batch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FilesSource {
    files: Vec<PathBuf>,
}

impl FilesSource {
    pub fn new(files: Vec<PathBuf>) -> Self {
        Self { files }
    }
}

impl ImportSource for FilesSource {
    fn label(&self) -> String {
        let n = self.files.iter().filter(|f| f.is_file() && photos_media::is_media_file(f)).count();
        if n == 1 { "1 file".into() } else { format!("{n} files") }
    }

    fn kind(&self) -> i64 {
        photos_core::batches::SOURCE_FOLDER
    }

    fn enumerate(&self) -> Result<Vec<SourceItem>, ImportError> {
        // The full path is the item's identity: two picked files may share a name.
        Ok(self.files.iter().filter_map(|f| file_item(f, f.to_string_lossy().replace('\\', "/"))).collect())
    }

    fn open(&self, item: &SourceItem) -> Result<Box<dyn Read + Send>, ImportError> {
        Ok(Box::new(std::fs::File::open(PathBuf::from(&item.rel_path))?))
    }
}

/// A source restricted to the relative paths in `allow` (incremental card and phone
/// imports: only what `import_seen` has not recorded). Labels and kind pass through.
pub struct FilteredSource<S: ImportSource> {
    inner: S,
    allow: std::collections::HashSet<String>,
}

impl<S: ImportSource> FilteredSource<S> {
    pub fn new(inner: S, allow: std::collections::HashSet<String>) -> Self {
        Self { inner, allow }
    }

    pub fn inner(&self) -> &S {
        &self.inner
    }
}

impl<S: ImportSource> ImportSource for FilteredSource<S> {
    fn label(&self) -> String {
        self.inner.label()
    }

    fn kind(&self) -> i64 {
        self.inner.kind()
    }

    fn enumerate(&self) -> Result<Vec<SourceItem>, ImportError> {
        Ok(self.inner.enumerate()?.into_iter().filter(|i| self.allow.contains(&i.rel_path)).collect())
    }

    fn open(&self, item: &SourceItem) -> Result<Box<dyn Read + Send>, ImportError> {
        self.inner.open(item)
    }

    fn extras(&self, item: &SourceItem) -> Option<ItemExtras> {
        self.inner.extras(item)
    }

    fn is_connected(&self) -> bool {
        self.inner.is_connected()
    }

    fn sidecars(&self, item: &SourceItem) -> Vec<SourceItem> {
        self.inner.sidecars(item)
    }

    fn leftovers(&self) -> Vec<(String, &'static str)> {
        self.inner.leftovers()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let r = dir.path();
        std::fs::create_dir_all(r.join("2024/summer")).unwrap();
        for f in ["top.jpg", "notes.txt", "2024/a.jpg", "2024/summer/b.heic"] {
            std::fs::write(r.join(f), b"x").unwrap();
        }
        dir
    }

    fn names(items: Vec<SourceItem>) -> Vec<String> {
        items.into_iter().map(|i| i.rel_path).collect()
    }

    #[test]
    fn a_folder_imports_its_subfolders_unless_asked_not_to() {
        let dir = tree();
        assert_eq!(names(FolderSource::new(dir.path()).enumerate().unwrap()), ["2024/a.jpg", "2024/summer/b.heic", "top.jpg"]);
        assert_eq!(names(FolderSource::new(dir.path()).recursive(false).enumerate().unwrap()), ["top.jpg"]);
    }

    #[test]
    fn chosen_files_import_exactly_those_and_skip_what_is_not_media() {
        let dir = tree();
        let r = dir.path();
        let src = FilesSource::new(vec![r.join("top.jpg"), r.join("2024/summer/b.heic"), r.join("notes.txt"), r.join("missing.jpg")]);
        let got = src.enumerate().unwrap();
        assert_eq!(got.iter().map(|i| i.file_name.as_str()).collect::<Vec<_>>(), ["top.jpg", "b.heic"]);
        let mut bytes = Vec::new();
        src.open(&got[1]).unwrap().read_to_end(&mut bytes).unwrap();
        assert_eq!(bytes, b"x");
        assert_eq!(src.label(), "2 files");
    }

    /// Round 8A: the source file's creation time travels with it, where the system keeps one.
    #[test]
    fn a_file_brings_when_it_was_created() {
        let dir = tree();
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() as i64;
        let items = FolderSource::new(dir.path()).enumerate().unwrap();
        let picked = FilesSource::new(vec![dir.path().join("top.jpg")]).enumerate().unwrap();
        for item in items.iter().chain(&picked) {
            if cfg!(any(windows, target_os = "macos")) {
                let at = item.created_at.unwrap_or_else(|| panic!("{} has no creation time", item.rel_path));
                assert!((at - now).abs() < 120, "{at} vs {now}");
            }
        }
    }
}
