//! Settings → Storage: every disk on the computer (internal and removable, a memory card
//! included) with its free space, and what the open library and Lumen itself take up.
//! Read-only: sizes come from directory metadata, nothing is opened or written.

use std::path::{Path, PathBuf};

use serde::Serialize;
use tauri::State;
use ts_rs::TS;

use crate::error::AppResult;
use crate::state::AppState;
use crate::temp_files::{self, TempKind, TempUsage};

#[derive(Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct StorageDisk {
    pub mount: String,
    /// The volume's name, empty when it has none.
    pub label: String,
    pub file_system: String,
    pub removable: bool,
    #[ts(type = "number")]
    pub total: u64,
    #[ts(type = "number")]
    pub available: u64,
    /// The open library lives on it.
    pub holds_library: bool,
    /// It has a `DCIM` folder: a camera card or a phone mounted as a disk.
    pub has_photos: bool,
}

/// One slice of what Lumen takes up.
#[derive(Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct StoragePart {
    pub label: String,
    #[ts(type = "number")]
    pub bytes: u64,
}

#[derive(Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct StorageOverview {
    pub disks: Vec<StorageDisk>,
    pub library_path: Option<String>,
    /// The library's folders (originals, previews, catalogue, trash, …), largest first.
    pub library: Vec<StoragePart>,
    /// Lumen's own data outside the library (models, safety copies, logs, updates).
    pub app: Vec<StoragePart>,
}

#[tauri::command(async)]
pub fn storage_overview(state: State<'_, AppState>) -> AppResult<StorageOverview> {
    let root: Option<PathBuf> = state.library.read().expect("library lock").as_ref().map(|l| l.root.clone());
    let mut disks: Vec<StorageDisk> = sysinfo::Disks::new_with_refreshed_list()
        .iter()
        .map(|d| {
            let mount = d.mount_point();
            StorageDisk {
                mount: mount.display().to_string(),
                label: d.name().to_string_lossy().into_owned(),
                file_system: d.file_system().to_string_lossy().into_owned(),
                removable: d.is_removable(),
                total: d.total_space(),
                available: d.available_space(),
                holds_library: false,
                has_photos: mount.join("DCIM").is_dir(),
            }
        })
        .filter(|d| d.total > 0)
        .collect();
    // The library's disk is the one with the longest mount that prefixes its path.
    if let Some(root) = &root {
        if let Some(i) = disks
            .iter()
            .enumerate()
            .filter(|(_, d)| root.starts_with(&d.mount))
            .max_by_key(|(_, d)| d.mount.len())
            .map(|(i, _)| i)
        {
            disks[i].holds_library = true;
        }
    }
    disks.sort_by_key(|d| (!d.holds_library, d.removable, d.mount.clone()));

    let library = root.as_deref().map(library_parts).unwrap_or_default();
    let app = app_parts(&state.paths.app_data);
    Ok(StorageOverview { disks, library_path: root.map(|r| r.display().to_string()), library, app })
}

/// What Repair, Recover and the import left behind, and the conversion cache (Round 22 §6).
#[tauri::command(async)]
pub fn get_temp_usage(state: State<'_, AppState>) -> AppResult<TempUsage> {
    let lib = state.library()?;
    Ok(temp_files::usage(&lib.root, &state.paths.app_data))
}

/// *Clear temporary files* for `kinds`; returns the bytes freed. Refused with `busy` while a
/// repair or a recovery runs.
#[tauri::command(async)]
pub fn clear_temp(state: State<'_, AppState>, kinds: Vec<TempKind>) -> AppResult<u64> {
    let lib = state.library()?;
    let busy = state.jobs.is_inflight(&crate::repair::job_key(&lib.manifest.id));
    temp_files::clear(&lib.root, &state.paths.app_data, &kinds, busy, &temp_files::SystemBin)
}

fn library_parts(root: &Path) -> Vec<StoragePart> {
    let mut parts = vec![
        part("Photos and videos", size_of(&root.join("originals"))),
        part("Previews and video copies", size_of(&root.join("derivatives"))),
        part("Catalogue", ["library.db", "library.db-wal", "library.db-shm"].iter().map(|f| file_size(&root.join(f))).sum()),
        part("Trash", size_of(&root.join("trash"))),
        part("Music", size_of(&root.join("music"))),
    ];
    let named = ["originals", "derivatives", "trash", "music", "library.db", "library.db-wal", "library.db-shm"];
    let other: u64 = entries(root).filter(|p| !named.iter().any(|n| p.file_name().is_some_and(|f| f == *n))).map(|p| size_of(&p)).sum();
    parts.push(part("Other", other));
    tidy(parts)
}

fn app_parts(app_data: &Path) -> Vec<StoragePart> {
    let parts = vec![
        part("Smart-feature models", size_of(&app_data.join("models"))),
        part("Safety copies before upgrades", size_of(&app_data.join("backups"))),
        part("Downloaded updates", size_of(&app_data.join("updates"))),
        part("Caches", size_of(&app_data.join("cache"))),
        part("Logs", size_of(&app_data.join("logs"))),
    ];
    tidy(parts)
}

fn part(label: &str, bytes: u64) -> StoragePart {
    StoragePart { label: label.into(), bytes }
}

fn tidy(mut parts: Vec<StoragePart>) -> Vec<StoragePart> {
    parts.retain(|p| p.bytes > 0);
    parts.sort_by_key(|p| std::cmp::Reverse(p.bytes));
    parts
}

fn entries(dir: &Path) -> impl Iterator<Item = PathBuf> {
    std::fs::read_dir(dir).into_iter().flatten().flatten().map(|e| e.path())
}

fn file_size(p: &Path) -> u64 {
    std::fs::symlink_metadata(p).map(|m| if m.is_file() { m.len() } else { 0 }).unwrap_or(0)
}

/// Bytes under `p`, never following links (a link out of the library would count twice).
fn size_of(p: &Path) -> u64 {
    let Ok(meta) = std::fs::symlink_metadata(p) else { return 0 };
    if meta.is_file() {
        return meta.len();
    }
    if !meta.is_dir() {
        return 0;
    }
    let mut total = 0;
    let mut stack = vec![p.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
            let Ok(ft) = entry.file_type() else { continue };
            if ft.is_dir() {
                stack.push(entry.path());
            } else if ft.is_file() {
                total += entry.metadata().map(|m| m.len()).unwrap_or(0);
            }
        }
    }
    total
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn library_parts_count_each_folder_largest_first() {
        let t = tempfile::tempdir().unwrap();
        let r = t.path();
        std::fs::create_dir_all(r.join("originals/2024/05")).unwrap();
        std::fs::write(r.join("originals/2024/05/a.jpg"), vec![0; 300]).unwrap();
        std::fs::create_dir_all(r.join("derivatives")).unwrap();
        std::fs::write(r.join("derivatives/t.webp"), vec![0; 100]).unwrap();
        std::fs::write(r.join("library.db"), vec![0; 50]).unwrap();
        std::fs::write(r.join("library.json"), vec![0; 5]).unwrap();
        let parts = library_parts(r);
        let got: Vec<(&str, u64)> = parts.iter().map(|p| (p.label.as_str(), p.bytes)).collect();
        assert_eq!(got, [("Photos and videos", 300), ("Previews and video copies", 100), ("Catalogue", 50), ("Other", 5)]);
    }
}
