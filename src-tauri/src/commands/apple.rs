//! Mac-formatted disks and Apple Photos libraries (architecture spec §8.4). Reading a disk or
//! copying a library's database is slow, so those commands run off the main thread.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use photos_core::seen::{self, SeenItem};
use photos_import::apple::files::{DirFiles, LibraryFiles, MacDiskFiles};
use photos_import::apple::source::{LibrarySummary, PhotosLibrarySource};
use photos_import::macdisk::source::{folder_listing, MacFolderListing, MacFolderSource};
use photos_import::{FilteredSource, ImportError, ImportSource, SourceItem};
use serde::Serialize;
use tauri::{AppHandle, Manager, State};
use ts_rs::TS;

use crate::commands::import::spawn_import;
use crate::connected::{DevicePreview, DeviceRef};
use crate::error::{AppError, AppResult};
use crate::events::MacDiskInfo;
use crate::macdisks;
use crate::state::AppState;

/// How many of a library's newest photos its card shows.
const NEWEST: usize = 12;

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PhotosLibraryInfo {
    pub summary: LibrarySummary,
    /// Files the import would bring: photos, videos, Live Photo videos, RAW alternates.
    pub total_files: u32,
    /// Of those, the ones not imported from this library before.
    pub new_items: u32,
    pub newest: Vec<DevicePreview>,
}

fn gone(id: &str) -> AppError {
    AppError { code: "device_gone".into(), message: "That Mac disk is no longer connected.".into(), detail: Some(id.into()) }
}

fn unreadable(e: ImportError) -> AppError {
    AppError { code: "mac_disk_unreadable".into(), message: "The disk could not be read.".into(), detail: Some(e.to_string()) }
}

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> AppResult<T> + Send + 'static) -> AppResult<T> {
    tauri::async_runtime::spawn_blocking(f)
        .await
        .map_err(|e| AppError { code: "internal".into(), message: "The work stopped unexpectedly.".into(), detail: Some(e.to_string()) })?
}

fn seen_items(items: &[SourceItem]) -> Vec<SeenItem<'_>> {
    items.iter().map(|i| SeenItem { rel_path: &i.rel_path, size: i.size, mtime: i.modified_at }).collect()
}

/// The library at `path` — on the Mac disk `disk`, or on this computer — opened once per
/// session. A cached library whose disk has since been re-plugged is opened again.
pub(crate) fn library(state: &AppState, disk: Option<&str>, path: &str) -> AppResult<Arc<PhotosLibrarySource>> {
    let key = format!("{}:{path}", disk.unwrap_or("local"));
    let fs = match disk {
        Some(id) => Some(macdisks::get(id).ok_or_else(|| gone(id))?.1),
        None => None,
    };
    if let Some((cached_fs, lib)) = state.photo_libraries.lock().expect("libraries lock").get(&key) {
        let same = match (cached_fs, &fs) {
            (Some(a), Some(b)) => Arc::ptr_eq(a, b),
            (None, None) => true,
            _ => false,
        };
        if same {
            return Ok(lib.clone());
        }
    }
    let files: Box<dyn LibraryFiles> = match &fs {
        Some(fs) => Box::new(MacDiskFiles { fs: fs.clone(), root: path.to_string() }),
        None => {
            if !Path::new(path).is_dir() {
                return Err(AppError { code: "not_a_folder".into(), message: "That Photos library could not be found.".into(), detail: Some(path.into()) });
            }
            Box::new(DirFiles(PathBuf::from(path)))
        }
    };
    let name = path.trim_end_matches(['/', '\\']).rsplit(['/', '\\']).next().unwrap_or("Photos Library");
    let name = name.strip_suffix(".photoslibrary").unwrap_or(name);
    let lib = PhotosLibrarySource::open(files, name).map(Arc::new).map_err(|e| AppError {
        code: "library_unreadable".into(),
        message: "That is not an Apple Photos library Lumen can read.".into(),
        detail: Some(e.to_string()),
    })?;
    state.photo_libraries.lock().expect("libraries lock").insert(key, (fs, lib.clone()));
    Ok(lib)
}

/// Mac disks the watcher has opened (Windows only; elsewhere they mount as volumes).
#[tauri::command(async)]
pub fn list_mac_disks() -> AppResult<Vec<MacDiskInfo>> {
    let mut list = macdisks::snapshot();
    list.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(list)
}

/// One level of the folder browser on a Mac disk.
#[tauri::command]
pub async fn list_mac_folder(disk: String, path: String) -> AppResult<MacFolderListing> {
    blocking(move || {
        let (_, fs) = macdisks::get(&disk).ok_or_else(|| gone(&disk))?;
        folder_listing(&fs, &path).map_err(unreadable)
    })
    .await
}

/// Imports a folder from a Mac disk, with its subfolders unless `recursive` is false.
#[tauri::command(async)]
pub fn start_mac_folder_import(app: AppHandle, state: State<'_, AppState>, disk: String, path: String, recursive: Option<bool>) -> AppResult<i64> {
    let (info, fs) = macdisks::get(&disk).ok_or_else(|| gone(&disk))?;
    let source = MacFolderSource::new(fs, &info.name, &path, recursive.unwrap_or(true));
    spawn_import(&app, &state, Box::new(source), Some(format!("{}: {path}", info.name)), None, None)
}

/// A library's summary, how much of it is new, and its newest photos as preview tokens.
#[tauri::command]
pub async fn get_photos_library(app: AppHandle, disk: Option<String>, path: String) -> AppResult<PhotosLibraryInfo> {
    blocking(move || {
        let state = app.state::<AppState>();
        let lib = library(&state, disk.as_deref(), &path)?;
        let items = lib.enumerate().map_err(unreadable)?;
        let new_items = match state.library() {
            Ok(l) => l.db.with_conn(|c| seen::count_unseen(c, &lib.key(), &seen_items(&items))).unwrap_or(items.len()),
            Err(_) => items.len(),
        };
        let device = DeviceRef::Shared(lib.clone());
        let newest = lib
            .newest(NEWEST)
            .into_iter()
            .map(|item| DevicePreview {
                token: state.device_previews.register(&device, item).to_string(),
                file_name: item.file_name.clone(),
                modified_at: item.modified_at,
                video: photos_media::kind_for_extension(item.file_name.rsplit('.').next().unwrap_or("")) == photos_media::MediaKind::Video,
            })
            .collect();
        Ok(PhotosLibraryInfo { summary: lib.summary().clone(), total_files: items.len() as u32, new_items: new_items as u32, newest })
    })
    .await
}

/// Imports what is new in a library, remembering each file as seen as it lands.
#[tauri::command]
pub async fn start_photos_library_import(app: AppHandle, disk: Option<String>, path: String) -> AppResult<i64> {
    blocking(move || {
        let state = app.state::<AppState>();
        let lib = library(&state, disk.as_deref(), &path)?;
        let catalogue = state.library()?;
        let key = lib.key();
        let items = lib.enumerate().map_err(unreadable)?;
        let unseen = catalogue.db.with_conn(|c| seen::unseen_indexes(c, &key, &seen_items(&items)))?;
        if unseen.is_empty() {
            return Err(AppError { code: "nothing_new".into(), message: "Everything in this library is already imported.".into(), detail: None });
        }
        let allow: HashSet<String> = unseen.iter().map(|&i| items[i].rel_path.clone()).collect();
        let source = FilteredSource::new(lib.clone(), allow);
        spawn_import(&app, &state, Box::new(source), Some(path.clone()), Some(key), None)
    })
    .await
}

/// An Apple Photos library Lumen found by itself (Round 9B): the Import screen offers Apple
/// Photos only when there is one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct FoundPhotosLibrary {
    /// The `.photoslibrary` folder's path, on this computer or on the Mac disk.
    pub path: String,
    /// "Photos Library", the folder's name without its extension.
    pub name: String,
    /// The Mac disk it is on (`/connected/mac/{disk}`), or `None` on this computer.
    pub disk: Option<String>,
    /// Where it is, for a person: "Pictures", "D:", "Macintosh HD".
    pub place: String,
}

/// Whether `dir` is a Photos library: the name says so and it has the database Photos keeps.
fn is_photos_library(dir: &Path) -> bool {
    dir.extension().is_some_and(|e| e.eq_ignore_ascii_case("photoslibrary")) && dir.join("database").join("Photos.sqlite").is_file()
}

/// Photos libraries directly inside `root` and inside its `Pictures` folder: where Photos puts
/// one, and where people copy one to. Two shallow listings per root, nothing deeper.
pub fn libraries_under(root: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    for dir in [root.to_path_buf(), root.join("Pictures")] {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for e in entries.flatten() {
            let p = e.path();
            if e.file_type().is_ok_and(|t| t.is_dir()) && is_photos_library(&p) {
                found.push(p);
            }
        }
    }
    found.sort();
    found.dedup();
    found
}

/// The folders to look in: the user's Pictures and home, and the top of every mounted drive.
pub(crate) fn search_roots(app: &AppHandle) -> Vec<(PathBuf, String)> {
    use tauri::path::BaseDirectory;
    let mut roots = Vec::new();
    if let Ok(p) = app.path().resolve("", BaseDirectory::Picture) {
        roots.push((p, "Pictures".to_string()));
    }
    if let Ok(p) = app.path().home_dir() {
        roots.push((p, "Home folder".to_string()));
    }
    #[cfg(windows)]
    for letter in b'A'..=b'Z' {
        let root = PathBuf::from(format!("{}:\\", letter as char));
        if root.is_dir() {
            roots.push((root, format!("{}:", letter as char)));
        }
    }
    #[cfg(not(windows))]
    {
        let user = std::env::var("USER").unwrap_or_default();
        for parent in ["/Volumes".to_string(), format!("/media/{user}"), format!("/run/media/{user}"), "/mnt".to_string()] {
            for e in std::fs::read_dir(&parent).into_iter().flatten().flatten() {
                let name = e.file_name().to_string_lossy().into_owned();
                roots.push((e.path(), name));
            }
        }
    }
    roots
}

/// Apple Photos libraries on this computer, its drives and connected Mac disks. Looked for
/// again when the drives change, else kept for a minute.
#[tauri::command(async)]
pub fn find_apple_photos_libraries(app: AppHandle) -> AppResult<Vec<FoundPhotosLibrary>> {
    use std::sync::Mutex;
    use std::time::{Duration, Instant};
    /// The roots looked in, when, and what was found there.
    type Found = (Vec<PathBuf>, Instant, Vec<FoundPhotosLibrary>);
    static CACHE: Mutex<Option<Found>> = Mutex::new(None);
    let roots = search_roots(&app);
    let key: Vec<PathBuf> = roots.iter().map(|(p, _)| p.clone()).collect();
    let mut local = match CACHE.lock().expect("library cache lock").as_ref() {
        Some((k, at, found)) if *k == key && at.elapsed() < Duration::from_secs(60) => Some(found.clone()),
        _ => None,
    };
    if local.is_none() {
        let mut found: Vec<FoundPhotosLibrary> = Vec::new();
        for (root, place) in &roots {
            for p in libraries_under(root) {
                let path = p.display().to_string();
                if found.iter().any(|f| f.path == path) {
                    continue;
                }
                let name = p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "Photos Library".into());
                found.push(FoundPhotosLibrary { path, name, disk: None, place: place.clone() });
            }
        }
        *CACHE.lock().expect("library cache lock") = Some((key, Instant::now(), found.clone()));
        local = Some(found);
    }
    let mut all = local.unwrap_or_default();
    for disk in macdisks::snapshot() {
        for path in &disk.libraries {
            let name = path.rsplit('/').next().unwrap_or(path).trim_end_matches(".photoslibrary").to_string();
            all.push(FoundPhotosLibrary { path: path.clone(), name, disk: Some(disk.id.clone()), place: disk.name.clone() });
        }
    }
    Ok(all)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn library(at: &Path) {
        std::fs::create_dir_all(at.join("database")).unwrap();
        std::fs::write(at.join("database").join("Photos.sqlite"), b"").unwrap();
    }

    #[test]
    fn finds_libraries_at_the_top_and_in_pictures_only() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        library(&root.join("Photos Library.photoslibrary"));
        library(&root.join("Pictures").join("Family.photoslibrary"));
        // Too deep, a name alone, and a plain folder: none of these.
        library(&root.join("Backups").join("Old.photoslibrary"));
        std::fs::create_dir_all(root.join("Empty.photoslibrary")).unwrap();
        std::fs::create_dir_all(root.join("Pictures").join("Holiday")).unwrap();
        let found: Vec<String> = libraries_under(root).iter().map(|p| p.file_name().unwrap().to_string_lossy().into_owned()).collect();
        assert_eq!(found, ["Photos Library.photoslibrary", "Family.photoslibrary"]);
        assert!(libraries_under(&root.join("missing")).is_empty());
    }
}
