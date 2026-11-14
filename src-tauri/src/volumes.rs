//! Volume watcher (one thread, `sysinfo` every two seconds — costs nothing measurable and
//! behaves the same on every OS):
//!  * portable libraries: when a volume appears, look for known libraries on it and tell
//!    the frontend (`library://found`);
//!  * memory cards: any volume with a `DCIM` folder is a source; the snapshot with its
//!    new-item counts is pushed as `device://volumes` whenever it changes.

use std::collections::{HashMap, HashSet};
use std::path::{Component, Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use photos_core::seen::{self, SeenItem};
use photos_core::{Library, LibraryManifest};
use photos_import::{ImportSource, VolumeSource};
use tauri::{AppHandle, Emitter, Manager};

use crate::events::{LibraryFound, VolumeInfo, DEVICE_VOLUMES, LIBRARY_FOUND};
use crate::state::AppState;

const DEFAULT_LIBRARY_DIR: &str = "Lumen Library";
const TICK: Duration = Duration::from_secs(2);

/// Current DCIM volumes, keyed by mount. Read by `list_volumes`.
static SNAPSHOT: Mutex<Vec<VolumeInfo>> = Mutex::new(Vec::new());
/// Mounts whose counts must be recomputed on the next tick (after an import).
static DIRTY: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());

pub fn snapshot() -> Vec<VolumeInfo> {
    SNAPSHOT.lock().expect("volumes lock").clone()
}

/// Ask the watcher to recount `mount` (its new-item count changes after an import).
pub fn mark_dirty(mount: &Path) {
    DIRTY.lock().expect("dirty lock").push(mount.to_path_buf());
}

/// Where a library last seen at `entry_path` might live on `mount`: the same path below
/// the old root, then the default folder at the mount root. Deduplicated, in that order.
pub fn candidates(entry_path: &Path, mount: &Path) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    let tail: PathBuf = entry_path
        .components()
        .filter(|c| matches!(c, Component::Normal(_)))
        .collect();
    if tail.components().count() > 0 {
        out.push(mount.join(&tail));
    }
    let default = mount.join(DEFAULT_LIBRARY_DIR);
    if !out.iter().any(|p| p == &default) {
        out.push(default);
    }
    out
}

#[derive(Debug, Clone)]
struct Disk {
    mount: PathBuf,
    label: String,
    total_space: u64,
    removable: bool,
}

fn disks() -> Vec<Disk> {
    sysinfo::Disks::new_with_refreshed_list()
        .iter()
        .map(|d| Disk {
            mount: d.mount_point().to_path_buf(),
            label: d.name().to_string_lossy().into_owned(),
            total_space: d.total_space(),
            removable: d.is_removable(),
        })
        .collect()
}

fn scan_for_libraries(app: &AppHandle, new_mounts: &[PathBuf]) {
    let state = app.state::<AppState>();
    let entries: Vec<_> = state.registry.read().expect("registry lock").libraries.clone();
    for entry in entries.iter().filter(|e| !Library::exists_at(Path::new(&e.path))) {
        for mount in new_mounts {
            for c in candidates(Path::new(&entry.path), mount) {
                if !Library::exists_at(&c) {
                    continue;
                }
                match LibraryManifest::load(&c) {
                    Ok(Some(m)) if m.id == entry.id => {
                        tracing::info!(id = %entry.id, path = %c.display(), "found library on new volume");
                        let _ = app.emit(
                            LIBRARY_FOUND,
                            LibraryFound { id: entry.id.clone(), name: entry.name.clone(), path: c.display().to_string() },
                        );
                        break;
                    }
                    _ => {}
                }
            }
        }
    }
}

/// Enumerate a DCIM volume and count what the current library has not imported yet.
fn describe(app: &AppHandle, disk: &Disk) -> Option<VolumeInfo> {
    let source = VolumeSource::detect(&disk.mount, &disk.label, disk.total_space)?;
    let items = match source.enumerate() {
        Ok(i) => i,
        Err(e) => {
            tracing::warn!(mount = %disk.mount.display(), "DCIM enumerate failed: {e}");
            return None;
        }
    };
    let seen_items: Vec<SeenItem<'_>> =
        items.iter().map(|i| SeenItem { rel_path: &i.rel_path, size: i.size, mtime: i.modified_at }).collect();
    let state = app.state::<AppState>();
    let new_items = match state.library() {
        Ok(lib) => lib.db.with_conn(|c| seen::count_unseen(c, source.key(), &seen_items)).unwrap_or(items.len()),
        Err(_) => items.len(),
    };
    Some(VolumeInfo {
        mount: disk.mount.display().to_string(),
        label: source.volume_label().to_string(),
        key: source.key().to_string(),
        total_items: items.len() as u32,
        new_items: new_items as u32,
        removable: disk.removable,
    })
}

pub fn start(app: AppHandle) {
    std::thread::Builder::new()
        .name("lumen-volumes".into())
        .spawn(move || {
            // Everything mounted at startup counts as new once, so a drive plugged in
            // before launch is found too.
            let mut known: HashSet<PathBuf> = HashSet::new();
            let mut cards: HashMap<PathBuf, VolumeInfo> = HashMap::new();
            loop {
                let now = disks();
                let mounts: HashSet<PathBuf> = now.iter().map(|d| d.mount.clone()).collect();
                let fresh: Vec<PathBuf> = mounts.difference(&known).cloned().collect();
                if !fresh.is_empty() {
                    scan_for_libraries(&app, &fresh);
                }
                let dirty: Vec<PathBuf> = std::mem::take(&mut *DIRTY.lock().expect("dirty lock"));
                let mut changed = false;
                // Gone volumes.
                let before = cards.len();
                cards.retain(|m, _| mounts.contains(m));
                changed |= cards.len() != before;
                // New or dirty volumes are (re)described; DCIM appearing on a known
                // mount is also caught because `cards` is keyed by mount only.
                for d in &now {
                    let needs = fresh.contains(&d.mount) || dirty.contains(&d.mount) || (!cards.contains_key(&d.mount) && d.mount.join(photos_import::volume::DCIM).is_dir());
                    if !needs {
                        continue;
                    }
                    match describe(&app, d) {
                        Some(info) => {
                            cards.insert(d.mount.clone(), info);
                            changed = true;
                        }
                        None => changed |= cards.remove(&d.mount).is_some(),
                    }
                }
                if changed {
                    let mut list: Vec<VolumeInfo> = cards.values().cloned().collect();
                    list.sort_by(|a, b| a.mount.cmp(&b.mount));
                    *SNAPSHOT.lock().expect("volumes lock") = list.clone();
                    let _ = app.emit(DEVICE_VOLUMES, &list);
                }
                known = mounts;
                std::thread::sleep(TICK);
            }
        })
        .expect("spawn volume watcher");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn candidates_keep_the_tail_and_add_the_default() {
        let c = candidates(Path::new("D:\\Photos\\Lumen Library"), Path::new("/Volumes/SSD"));
        assert_eq!(c[0], Path::new("/Volumes/SSD").join("Photos").join("Lumen Library"));
        assert_eq!(c[1], Path::new("/Volumes/SSD").join("Lumen Library"));
        assert_eq!(c.len(), 2);
        let c = candidates(Path::new("/Volumes/SSD/Lumen Library"), Path::new("E:\\"));
        assert_eq!(c, vec![Path::new("E:\\").join("Volumes").join("SSD").join("Lumen Library"), Path::new("E:\\").join("Lumen Library")]);
        let c = candidates(Path::new("/Lumen Library"), Path::new("/mnt/x"));
        assert_eq!(c, vec![Path::new("/mnt/x").join("Lumen Library")], "deduplicated");
    }
}
