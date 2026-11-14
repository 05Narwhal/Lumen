//! One read-only view over an HFS+ or APFS volume, whichever the partition holds.

use std::io::{Read, Seek, SeekFrom, Write};
use std::sync::Mutex;

use serde::Serialize;
use ts_rs::TS;

use super::aligned::AlignedReader;
use crate::ImportError;

pub trait ReadSeek: Read + Seek + Send {}
impl<T: Read + Seek + Send> ReadSeek for T {}

type Device = AlignedReader<Box<dyn ReadSeek>>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export)]
pub enum MacFsKind {
    Hfs,
    Apfs,
}

/// `H+`/`HX` at byte 1024 is an HFS+ volume header; `NXSB` at byte 32 is an APFS
/// container superblock. Needs the first 1026 bytes; shorter is `None`.
pub fn sniff(head: &[u8]) -> Option<MacFsKind> {
    if head.len() < 1026 {
        return None;
    }
    if matches!(&head[1024..1026], b"H+" | b"HX") {
        Some(MacFsKind::Hfs)
    } else if &head[32..36] == b"NXSB" {
        Some(MacFsKind::Apfs)
    } else {
        None
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct MacEntry {
    pub name: String,
    pub is_dir: bool,
    pub size: u64,
    /// Unix seconds.
    pub modified_at: Option<i64>,
}

enum Inner {
    Hfs(Box<hfsplus::HfsVolume<Device>>),
    Apfs(Box<apfs::ApfsVolume<Device>>),
}

/// A mounted-in-memory Mac volume. Both parsers need `&mut` for every read, and the
/// ingest pipeline opens files from several threads, so access goes through a mutex.
pub struct MacFs {
    kind: MacFsKind,
    volume_name: Option<String>,
    free_bytes: Option<u64>,
    inner: Mutex<Inner>,
}

/// Seconds between 1904-01-01 (HFS+) and 1970-01-01.
const HFS_EPOCH_OFFSET: i64 = 2_082_844_800;

fn hfs_time(t: u32) -> Option<i64> {
    (t != 0).then(|| t as i64 - HFS_EPOCH_OFFSET)
}

fn apfs_time(ns: i64) -> Option<i64> {
    (ns != 0).then_some(ns / 1_000_000_000)
}

fn failed(path: &str, e: impl std::fmt::Display) -> ImportError {
    ImportError::Source(format!("{path}: {e}"))
}

impl MacFs {
    /// Opens the volume on `device`, a partition of `len` bytes (whole sectors).
    pub fn open(device: Box<dyn ReadSeek>, len: u64) -> Result<Self, ImportError> {
        let mut reader = AlignedReader::new(device, len);
        let mut head = vec![0u8; 4096.min(len as usize)];
        reader.read_exact(&mut head)?;
        reader.seek(SeekFrom::Start(0))?;
        match sniff(&head) {
            Some(MacFsKind::Hfs) => {
                let vol = hfsplus::HfsVolume::open(reader).map_err(|e| failed("HFS+ volume", e))?;
                let h = vol.volume_header();
                let free = h.free_blocks as u64 * h.block_size as u64;
                Ok(Self { kind: MacFsKind::Hfs, volume_name: None, free_bytes: Some(free), inner: Mutex::new(Inner::Hfs(Box::new(vol))) })
            }
            Some(MacFsKind::Apfs) => {
                let vol = apfs::ApfsVolume::open(reader).map_err(|e| failed("APFS volume", e))?;
                let name = Some(vol.volume_info().name.clone()).filter(|n| !n.is_empty());
                Ok(Self { kind: MacFsKind::Apfs, volume_name: name, free_bytes: None, inner: Mutex::new(Inner::Apfs(Box::new(vol))) })
            }
            None => Err(ImportError::Source("not an HFS+ or APFS volume".into())),
        }
    }

    pub fn kind(&self) -> MacFsKind {
        self.kind
    }

    /// APFS's volume name. HFS+ keeps it in a catalog record the parser does not expose.
    pub fn volume_name(&self) -> Option<String> {
        self.volume_name.clone()
    }

    /// HFS+: free blocks × block size. APFS: unknown to the parser.
    pub fn free_bytes(&self) -> Option<u64> {
        self.free_bytes
    }

    /// Files and folders directly in `path` (`/`-separated, absolute). Symlinks are left out.
    pub fn list(&self, path: &str) -> Result<Vec<MacEntry>, ImportError> {
        let mut inner = self.inner.lock().expect("mac volume lock");
        let mut entries: Vec<MacEntry> = match &mut *inner {
            Inner::Hfs(v) => v
                .list_directory(path)
                .map_err(|e| failed(path, e))?
                .into_iter()
                .filter(|e| e.kind != hfsplus::EntryKind::Symlink)
                .map(|e| MacEntry {
                    is_dir: e.kind == hfsplus::EntryKind::Directory,
                    modified_at: hfs_time(e.modify_date),
                    size: e.size,
                    name: e.name,
                })
                .collect(),
            Inner::Apfs(v) => v
                .list_directory(path)
                .map_err(|e| failed(path, e))?
                .into_iter()
                .filter(|e| e.kind != apfs::EntryKind::Symlink)
                .map(|e| MacEntry {
                    is_dir: e.kind == apfs::EntryKind::Directory,
                    modified_at: apfs_time(e.modify_time),
                    size: e.size,
                    name: e.name,
                })
                .collect(),
        };
        entries.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(entries)
    }

    /// Streams the file at `path` into `out`; returns the bytes written.
    pub fn copy_to(&self, path: &str, mut out: &mut dyn Write) -> Result<u64, ImportError> {
        let mut inner = self.inner.lock().expect("mac volume lock");
        match &mut *inner {
            Inner::Hfs(v) => v.read_file_to(path, &mut out).map_err(|e| failed(path, e)),
            Inner::Apfs(v) => v.read_file_to(path, &mut out).map_err(|e| failed(path, e)),
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    #[test]
    fn sniffs_hfs_and_apfs_by_signature() {
        let mut head = vec![0u8; 4096];
        assert_eq!(sniff(&head), None);
        assert_eq!(sniff(&head[..100]), None, "too short to tell");
        head[1024..1026].copy_from_slice(b"H+");
        assert_eq!(sniff(&head), Some(MacFsKind::Hfs));
        head[1024..1026].copy_from_slice(b"HX");
        assert_eq!(sniff(&head), Some(MacFsKind::Hfs));
        let mut head = vec![0u8; 4096];
        head[32..36].copy_from_slice(b"NXSB");
        assert_eq!(sniff(&head), Some(MacFsKind::Apfs));
    }

    pub(crate) fn hfs_image(files: &[(&str, &[u8])]) -> (Vec<u8>, u64) {
        let mut b = hfsplus::testutil::HfsPlusImageBuilder::new();
        for (name, bytes) in files {
            b.add_file(name, bytes, 0o644);
        }
        let mut image = b.build();
        image.resize(image.len().div_ceil(512) * 512, 0); // a partition is whole sectors
        let len = image.len() as u64;
        (image, len)
    }

    #[test]
    fn lists_and_copies_files_from_an_hfs_image() {
        let (image, len) = hfs_image(&[("IMG_0001.JPG", b"jpeg bytes"), ("notes.txt", b"hi")]);
        let fs = MacFs::open(Box::new(std::io::Cursor::new(image)), len).unwrap();
        assert_eq!(fs.kind(), MacFsKind::Hfs);
        let entries = fs.list("/").unwrap();
        assert_eq!(entries.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(), ["IMG_0001.JPG", "notes.txt"]);
        assert!(!entries[0].is_dir);
        assert_eq!(entries[0].size, 10);
        let mut out = Vec::new();
        assert_eq!(fs.copy_to("/IMG_0001.JPG", &mut out).unwrap(), 10);
        assert_eq!(out, b"jpeg bytes");
        assert!(fs.free_bytes().is_some());
        assert!(fs.list("/missing").is_err());
    }

    #[test]
    fn refuses_what_is_not_a_mac_volume() {
        let zeros = vec![0u8; 64 * 1024];
        assert!(MacFs::open(Box::new(std::io::Cursor::new(zeros)), 64 * 1024).is_err());
    }
}
