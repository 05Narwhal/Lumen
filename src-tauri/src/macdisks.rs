//! Mac-formatted disks (HFS+, APFS) under *Connected*. On Windows they get no drive letter,
//! so a watcher probes partition devices every five seconds (access-0 opens, no admin) and
//! opens the Mac ones read-only; see architecture spec §8.4. macOS mounts these disks, and
//! Linux mounts HFS+, so there the volume watcher already sees them and this does nothing.

use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use photos_import::macdisk::fs::MacFs;
use photos_import::macdisk::probe::{self, MacPartition};
use photos_import::macdisk::source::find_photos_libraries;
use tauri::{AppHandle, Emitter};

use crate::events::{MacDiskInfo, DEVICE_MACDISKS};

/// Libraries are looked for this many folders deep: `/Pictures/Photos Library.photoslibrary`
/// and `/Backups/2023/…` are found, a walk of the whole disk is not made.
const LIBRARY_DEPTH: usize = 3;

static DISKS: Mutex<Vec<(MacDiskInfo, Arc<MacFs>)>> = Mutex::new(Vec::new());

/// APFS names its volumes; HFS+ keeps the name where the parser cannot reach it, so the
/// disk's own product name stands in.
pub fn display_name(volume: Option<&str>, disk: Option<&str>) -> String {
    volume.or(disk).map(str::to_string).unwrap_or_else(|| "Mac disk".into())
}

pub fn snapshot() -> Vec<MacDiskInfo> {
    DISKS.lock().expect("mac disks lock").iter().map(|(i, _)| i.clone()).collect()
}

pub fn get(id: &str) -> Option<(MacDiskInfo, Arc<MacFs>)> {
    DISKS.lock().expect("mac disks lock").iter().find(|(i, _)| i.id == id).cloned()
}

fn describe(p: &MacPartition) -> Option<(MacDiskInfo, Arc<MacFs>)> {
    let fs = match probe::open(p) {
        Ok(fs) => Arc::new(fs),
        Err(e) => {
            tracing::warn!(partition = %p.part.id(), "Mac disk could not be opened: {e}");
            return None;
        }
    };
    let volume = fs.volume_name();
    let info = MacDiskInfo {
        id: p.part.id(),
        name: display_name(volume.as_deref(), p.disk_name.as_deref()),
        fs: p.kind,
        total_bytes: p.len,
        free_bytes: fs.free_bytes(),
        libraries: find_photos_libraries(&fs, LIBRARY_DEPTH),
    };
    tracing::info!(id = %info.id, name = %info.name, libraries = info.libraries.len(), "Mac disk connected");
    Some((info, fs))
}

pub fn start(app: AppHandle) {
    if !cfg!(windows) {
        return;
    }
    std::thread::Builder::new()
        .name("lumen-macdisks".into())
        .spawn(move || {
            // Partitions probed and found not to be Mac, so they are not read every tick.
            let mut not_mac: HashSet<String> = HashSet::new();
            loop {
                let present = probe::partitions();
                let ids: HashSet<String> = present.iter().map(|p| p.id()).collect();
                not_mac.retain(|id| ids.contains(id));
                let known: HashSet<String> = DISKS.lock().expect("mac disks lock").iter().map(|(i, _)| i.id.clone()).collect();
                let mut changed = false;
                {
                    let mut disks = DISKS.lock().expect("mac disks lock");
                    let before = disks.len();
                    disks.retain(|(i, _)| ids.contains(&i.id));
                    changed |= disks.len() != before;
                }
                let fresh: Vec<_> = present.iter().filter(|p| !known.contains(&p.id()) && !not_mac.contains(&p.id())).collect();
                for p in fresh {
                    match probe::probe(p).and_then(|m| describe(&m)) {
                        Some(disk) => {
                            DISKS.lock().expect("mac disks lock").push(disk);
                            changed = true;
                        }
                        None => {
                            not_mac.insert(p.id());
                        }
                    }
                }
                if changed {
                    let mut list = snapshot();
                    list.sort_by(|a, b| a.id.cmp(&b.id));
                    let _ = app.emit(DEVICE_MACDISKS, &list);
                }
                std::thread::sleep(std::time::Duration::from_secs(5));
            }
        })
        .expect("spawn Mac disk watcher");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_a_disk_as_well_as_it_can() {
        assert_eq!(display_name(Some("Photos SSD"), Some("Samsung T7")), "Photos SSD");
        assert_eq!(display_name(None, Some("ADATA HD710 PRO")), "ADATA HD710 PRO");
        assert_eq!(display_name(None, None), "Mac disk");
    }
}
