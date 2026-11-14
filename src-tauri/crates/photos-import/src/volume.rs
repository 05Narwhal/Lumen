//! Memory cards and camera drives: any mounted volume with a `DCIM` folder, or with the
//! folders camcorders keep their video in.

use std::io::Read;
use std::path::{Path, PathBuf};

use crate::source::{FolderSource, ImportSource, SourceItem};
use crate::ImportError;

pub const DCIM: &str = "DCIM";

/// Where AVCHD and XAVC camcorders write clips, outside `DCIM` (Round 19 C3). Their items
/// keep the whole path from the mount so two cards' files never share an `import_seen` key.
const VIDEO_FOLDERS: &[&str] = &["PRIVATE/AVCHD/BDMV/STREAM", "PRIVATE/M4ROOT/CLIP"];

/// A mounted volume whose root holds `DCIM/` (or a camcorder's video folder). The `key`
/// names it in `import_seen`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VolumeSource {
    mount: PathBuf,
    label: String,
    key: String,
    dcim: FolderSource,
    /// The mount itself, for the sidecars of camcorder clips (their paths start there).
    from_mount: FolderSource,
}

impl VolumeSource {
    /// `Some` when `<mount>/DCIM` or a camcorder video folder exists. The key is the label plus the total size, which
    /// is as stable as a volume gets without a serial number from `sysinfo`.
    pub fn detect(mount: &Path, label: &str, total_space: u64) -> Option<Self> {
        let dcim = mount.join(DCIM);
        if !dcim.is_dir() && !VIDEO_FOLDERS.iter().any(|f| mount.join(f).is_dir()) {
            return None;
        }
        let label = if label.trim().is_empty() {
            mount.display().to_string().trim_end_matches(['\\', '/']).to_string()
        } else {
            label.trim().to_string()
        };
        Some(Self { key: Self::key_for(&label, total_space), label, mount: mount.to_path_buf(), dcim: FolderSource::new(&dcim), from_mount: FolderSource::new(mount) })
    }

    pub fn key_for(label: &str, total_space: u64) -> String {
        format!("vol:{}:{total_space}", label.trim())
    }

    pub fn key(&self) -> &str {
        &self.key
    }

    pub fn mount(&self) -> &Path {
        &self.mount
    }

    pub fn volume_label(&self) -> &str {
        &self.label
    }

    /// Whether `item` came from a camcorder folder (its path starts at the mount).
    fn in_video_folder(&self, item: &SourceItem) -> bool {
        VIDEO_FOLDERS.iter().any(|f| item.rel_path.starts_with(&format!("{f}/"))) && !self.dcim.root().join(&item.rel_path).is_file()
    }
}

impl ImportSource for VolumeSource {
    fn label(&self) -> String {
        format!("{} (DCIM)", self.label)
    }

    fn kind(&self) -> i64 {
        photos_core::batches::SOURCE_VOLUME
    }

    /// What is under `DCIM/`, with paths relative to it (`100CANON/IMG_0001.CR3`), then the
    /// camcorder folders, with paths from the mount (`PRIVATE/M4ROOT/CLIP/C0001.MP4`).
    fn enumerate(&self) -> Result<Vec<SourceItem>, ImportError> {
        let mut items = if self.dcim.root().is_dir() { self.dcim.enumerate()? } else { Vec::new() };
        for folder in VIDEO_FOLDERS {
            let root = self.mount.join(folder);
            if !root.is_dir() {
                continue;
            }
            for mut item in FolderSource::new(&root).enumerate()? {
                item.rel_path = format!("{folder}/{}", item.rel_path);
                items.push(item);
            }
        }
        Ok(items)
    }

    fn open(&self, item: &SourceItem) -> Result<Box<dyn Read + Send>, ImportError> {
        if self.in_video_folder(item) {
            return Ok(Box::new(std::fs::File::open(self.mount.join(&item.rel_path))?));
        }
        self.dcim.open(item)
    }

    fn is_connected(&self) -> bool {
        self.mount.is_dir() && self.dcim.is_connected()
    }

    fn sidecars(&self, item: &SourceItem) -> Vec<SourceItem> {
        if self.in_video_folder(item) {
            self.from_mount.sidecars(item)
        } else {
            self.dcim.sidecars(item)
        }
    }

    /// What `DCIM/` holds that is not a photo or video (thumbnails of a camera's own, notes).
    fn leftovers(&self) -> Vec<(String, &'static str)> {
        if self.dcim.root().is_dir() { self.dcim.leftovers() } else { Vec::new() }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_dcim_and_enumerates_only_it() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(VolumeSource::detect(tmp.path(), "SDCARD", 32).is_none(), "no DCIM yet");
        std::fs::create_dir_all(tmp.path().join("DCIM/100CANON")).unwrap();
        std::fs::write(tmp.path().join("DCIM/100CANON/IMG_0001.jpg"), b"x").unwrap();
        std::fs::write(tmp.path().join("stray.jpg"), b"x").unwrap();
        let v = VolumeSource::detect(tmp.path(), "SDCARD", 32).unwrap();
        assert_eq!(v.key(), "vol:SDCARD:32");
        assert_eq!(v.label(), "SDCARD (DCIM)");
        assert_eq!(v.kind(), photos_core::batches::SOURCE_VOLUME);
        let items = v.enumerate().unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].rel_path, "100CANON/IMG_0001.jpg");
        let same = VolumeSource::detect(tmp.path(), " SDCARD ", 32).unwrap();
        assert_eq!(same.key(), v.key(), "stable across mounts");
        let unlabeled = VolumeSource::detect(tmp.path(), "", 32).unwrap();
        assert!(!unlabeled.volume_label().is_empty(), "falls back to the mount path");
    }

    /// Three 192-byte packets with the transport stream's sync byte, which is what tells a
    /// camera's `.MTS` from a TypeScript module.
    fn m2ts() -> Vec<u8> {
        let mut b = vec![0u8; 192 * 3];
        for i in 0..3 {
            b[4 + i * 192] = 0x47;
        }
        b
    }

    #[test]
    fn card_video_outside_dcim_is_listed_and_opens() {
        use std::io::Read as _;
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join("DCIM/100MSDCF")).unwrap();
        std::fs::write(tmp.path().join("DCIM/100MSDCF/DSC00001.jpg"), b"jpg").unwrap();
        std::fs::create_dir_all(tmp.path().join("PRIVATE/AVCHD/BDMV/STREAM")).unwrap();
        std::fs::write(tmp.path().join("PRIVATE/AVCHD/BDMV/STREAM/00000.MTS"), m2ts()).unwrap();
        std::fs::create_dir_all(tmp.path().join("PRIVATE/M4ROOT/CLIP")).unwrap();
        std::fs::write(tmp.path().join("PRIVATE/M4ROOT/CLIP/C0001.MP4"), b"xavc").unwrap();
        // The folders' index files are not media.
        std::fs::write(tmp.path().join("PRIVATE/M4ROOT/CLIP/C0001M01.XML"), b"<x/>").unwrap();
        let v = VolumeSource::detect(tmp.path(), "SONY", 64).unwrap();
        let items = v.enumerate().unwrap();
        let rels: Vec<_> = items.iter().map(|i| i.rel_path.as_str()).collect();
        assert_eq!(rels, ["100MSDCF/DSC00001.jpg", "PRIVATE/AVCHD/BDMV/STREAM/00000.MTS", "PRIVATE/M4ROOT/CLIP/C0001.MP4"]);
        for (item, want) in items.iter().zip([b"jpg".to_vec(), m2ts(), b"xavc".to_vec()]) {
            let mut got = Vec::new();
            v.open(item).unwrap().read_to_end(&mut got).unwrap();
            assert_eq!(got, want, "{}", item.rel_path);
        }
    }

    #[test]
    fn a_camcorder_card_without_dcim_is_a_volume() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join("PRIVATE/AVCHD/BDMV/STREAM")).unwrap();
        std::fs::write(tmp.path().join("PRIVATE/AVCHD/BDMV/STREAM/00001.MTS"), m2ts()).unwrap();
        let v = VolumeSource::detect(tmp.path(), "CAM", 8).unwrap();
        assert_eq!(v.enumerate().unwrap().len(), 1);
    }
}
