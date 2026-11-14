//! Importing from a Mac disk: any folder, with or without its subfolders, and the folder
//! browser that picks it.

use std::io::{Read, Write};
use std::path::Path;
use std::sync::Arc;

use serde::Serialize;
use ts_rs::TS;

use super::fs::MacFs;
use crate::source::{ImportSource, SourceItem};
use crate::ImportError;

/// A file copied off a device into a temporary file, read from there. The copy is deleted
/// when the reader is dropped. Parsers and phones cannot hand out a reader that outlives
/// their lock, and the ingest pipeline reads from its own threads.
pub(crate) struct TempRead {
    _file: tempfile::NamedTempFile,
    reader: std::fs::File,
}

impl TempRead {
    pub(crate) fn copy(fill: impl FnOnce(&mut dyn Write) -> Result<u64, ImportError>) -> Result<Self, ImportError> {
        let mut file = tempfile::NamedTempFile::new()?;
        {
            let mut out = std::io::BufWriter::with_capacity(1 << 20, file.as_file_mut());
            fill(&mut out)?;
            out.flush()?;
        }
        let reader = file.reopen()?;
        Ok(Self { _file: file, reader })
    }
}

impl Read for TempRead {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.reader.read(buf)
    }
}

/// Dot-files, macOS's own folders (`.Spotlight-V100`, `.fseventsd`, `.Trashes`) and HFS+'s
/// private metadata folders, whose names start with NUL bytes.
fn is_hidden(name: &str) -> bool {
    name.starts_with('.') || name.chars().any(char::is_control)
}

fn is_library(name: &str) -> bool {
    name.to_ascii_lowercase().ends_with(".photoslibrary")
}

fn join(dir: &str, name: &str) -> String {
    if dir.ends_with('/') {
        format!("{dir}{name}")
    } else {
        format!("{dir}/{name}")
    }
}

pub struct MacFolderSource {
    fs: Arc<MacFs>,
    disk_label: String,
    root: String,
    recursive: bool,
}

impl MacFolderSource {
    /// `root` is an absolute path on the volume (`/` for all of it).
    pub fn new(fs: Arc<MacFs>, disk_label: &str, root: &str, recursive: bool) -> Self {
        Self { fs, disk_label: disk_label.to_string(), root: root.to_string(), recursive }
    }
}

impl ImportSource for MacFolderSource {
    fn label(&self) -> String {
        self.root.rsplit('/').find(|s| !s.is_empty()).map(str::to_string).unwrap_or_else(|| self.disk_label.clone())
    }

    fn kind(&self) -> i64 {
        photos_core::batches::SOURCE_MACDISK
    }

    fn enumerate(&self) -> Result<Vec<SourceItem>, ImportError> {
        let mut items = Vec::new();
        let mut stack = vec![self.root.clone()];
        while let Some(dir) = stack.pop() {
            let entries = match self.fs.list(&dir) {
                Ok(e) => e,
                // The chosen folder must be readable; a subfolder that is not is skipped.
                Err(e) if dir == self.root => return Err(e),
                Err(e) => {
                    tracing::warn!("skipping unreadable folder: {e}");
                    continue;
                }
            };
            for e in entries {
                if is_hidden(&e.name) {
                    continue;
                }
                let path = join(&dir, &e.name);
                if e.is_dir {
                    // A Photos library is imported as a library, never as loose files.
                    if self.recursive && !is_library(&e.name) {
                        stack.push(path);
                    }
                } else if photos_media::is_supported(Path::new(&e.name)) {
                    let rel = path.strip_prefix(&self.root).unwrap_or(&path).trim_start_matches('/').to_string();
                    items.push(SourceItem { rel_path: rel, file_name: e.name, size: e.size, modified_at: e.modified_at, created_at: None });
                }
            }
        }
        items.sort_by(|a, b| a.rel_path.cmp(&b.rel_path));
        Ok(items)
    }

    fn open(&self, item: &SourceItem) -> Result<Box<dyn Read + Send>, ImportError> {
        let path = join(&self.root, &item.rel_path);
        Ok(Box::new(TempRead::copy(|out| self.fs.copy_to(&path, out))?))
    }
}

/// One level of the folder browser.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct MacFolderListing {
    pub path: String,
    /// Visible sub-folders, sorted; Photos libraries are left out (they have their own card).
    pub folders: Vec<String>,
    /// Photos and videos directly in `path`.
    pub media_here: u32,
}

pub fn folder_listing(fs: &MacFs, path: &str) -> Result<MacFolderListing, ImportError> {
    let mut folders = Vec::new();
    let mut media_here = 0;
    for e in fs.list(path)? {
        if is_hidden(&e.name) {
            continue;
        }
        if e.is_dir {
            if !is_library(&e.name) {
                folders.push(e.name);
            }
        } else if photos_media::is_supported(Path::new(&e.name)) {
            media_here += 1;
        }
    }
    folders.sort_by_key(|f| f.to_lowercase());
    Ok(MacFolderListing { path: path.to_string(), folders, media_here })
}

/// `.photoslibrary` folders in the top `depth` levels of the volume, as absolute paths.
pub fn find_photos_libraries(fs: &MacFs, depth: usize) -> Vec<String> {
    let mut found = Vec::new();
    let mut level = vec!["/".to_string()];
    for _ in 0..depth {
        let mut next = Vec::new();
        for dir in level {
            let Ok(entries) = fs.list(&dir) else { continue };
            for e in entries.into_iter().filter(|e| e.is_dir && !is_hidden(&e.name)) {
                let path = join(&dir, &e.name);
                if is_library(&e.name) {
                    found.push(path);
                } else {
                    next.push(path);
                }
            }
        }
        level = next;
    }
    found.sort();
    found
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::macdisk::fs::tests::hfs_image;

    fn image_fs() -> Arc<MacFs> {
        let (image, len) = hfs_image(&[("IMG_0001.JPG", b"one"), ("IMG_0002.HEIC", b"two"), (".DS_Store", b"x"), ("notes.txt", b"n")]);
        Arc::new(MacFs::open(Box::new(std::io::Cursor::new(image)), len).unwrap())
    }

    #[test]
    fn imports_the_photos_in_a_folder_and_nothing_else() {
        let src = MacFolderSource::new(image_fs(), "ADATA", "/", true);
        let items = src.enumerate().unwrap();
        assert_eq!(items.iter().map(|i| i.rel_path.as_str()).collect::<Vec<_>>(), ["IMG_0001.JPG", "IMG_0002.HEIC"]);
        let mut bytes = Vec::new();
        src.open(&items[1]).unwrap().read_to_end(&mut bytes).unwrap();
        assert_eq!(bytes, b"two");
        assert_eq!(src.kind(), photos_core::batches::SOURCE_MACDISK);
        assert_eq!(src.label(), "ADATA");
        assert_eq!(MacFolderSource::new(image_fs(), "ADATA", "/Pictures/2019", false).label(), "2019");
    }

    #[test]
    fn the_browser_counts_media_and_hides_system_entries() {
        let l = folder_listing(&image_fs(), "/").unwrap();
        assert_eq!(l.media_here, 2);
        assert!(l.folders.is_empty());
        assert!(is_hidden(".Spotlight-V100"));
        assert!(is_hidden("\u{0}\u{0}\u{0}\u{0}HFS+ Private Data"));
        assert!(!is_hidden("mami photos"));
        assert!(is_library("Julian's Photo Library.photoslibrary"));
    }

    #[test]
    fn a_missing_folder_fails_the_import_instead_of_importing_nothing() {
        assert!(MacFolderSource::new(image_fs(), "ADATA", "/gone", true).enumerate().is_err());
    }
}
