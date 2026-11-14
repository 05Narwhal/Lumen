//! Finding Mac partitions. Windows refuses an unelevated read of a whole disk
//! (`\\.\PhysicalDriveN`) but allows one of a partition device, so partitions are found by
//! opening `\\?\GLOBALROOT\Device\HarddiskN\PartitionM` and looking at their first bytes —
//! the partition table, which needs the whole disk, is never read. On macOS and Linux Mac
//! disks mount natively and appear as ordinary volumes, so there is nothing to probe.

use super::fs::{MacFs, MacFsKind};
use crate::ImportError;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PartitionRef {
    pub disk: u32,
    pub partition: u32,
}

impl PartitionRef {
    /// `\\?\GLOBALROOT\Device\Harddisk3\Partition2`
    pub fn device_path(&self) -> String {
        format!(r"\\?\GLOBALROOT\Device\Harddisk{}\Partition{}", self.disk, self.partition)
    }

    /// `disk3-part2`: stable while the disk stays plugged in; used in URLs.
    pub fn id(&self) -> String {
        format!("disk{}-part{}", self.disk, self.partition)
    }

    pub fn parse_id(id: &str) -> Option<Self> {
        let (disk, partition) = id.strip_prefix("disk")?.split_once("-part")?;
        Some(Self { disk: disk.parse().ok()?, partition: partition.parse().ok()? })
    }
}

#[derive(Debug, Clone)]
pub struct MacPartition {
    pub part: PartitionRef,
    pub kind: MacFsKind,
    pub len: u64,
    /// The disk's vendor and product, e.g. "ADATA HD710 PRO".
    pub disk_name: Option<String>,
}

/// Vendor and product from a `STORAGE_DEVICE_DESCRIPTOR`: NUL-terminated strings at the
/// offsets stored at bytes 12 and 16 (an offset of 0 means absent).
fn descriptor_name(d: &[u8]) -> Option<String> {
    let at = |field: usize| -> Option<String> {
        let off = u32::from_le_bytes(d.get(field..field + 4)?.try_into().ok()?) as usize;
        if off == 0 || off >= d.len() {
            return None;
        }
        let end = d[off..].iter().position(|&b| b == 0).map_or(d.len(), |n| off + n);
        let s = String::from_utf8_lossy(&d[off..end]).trim().to_string();
        (!s.is_empty()).then_some(s)
    };
    let parts: Vec<String> = [at(12), at(16)].into_iter().flatten().collect();
    (!parts.is_empty()).then(|| parts.join(" "))
}

/// Opens `p` read-only as a volume.
pub fn open(p: &MacPartition) -> Result<MacFs, ImportError> {
    let file = std::fs::File::open(p.part.device_path())?;
    MacFs::open(Box::new(file), p.len)
}

#[cfg(windows)]
mod platform {
    use std::fs::File;
    use std::io::{Read, Seek, SeekFrom};
    use std::os::windows::fs::OpenOptionsExt;
    use std::os::windows::io::AsRawHandle;

    use windows_sys::Win32::Storage::FileSystem::{FILE_SHARE_READ, FILE_SHARE_WRITE};
    use windows_sys::Win32::System::IO::DeviceIoControl;
    use windows_sys::Win32::System::Ioctl::{IOCTL_DISK_GET_LENGTH_INFO, IOCTL_STORAGE_QUERY_PROPERTY};

    use super::{descriptor_name, MacPartition, PartitionRef};
    use crate::macdisk::aligned::AlignedReader;
    use crate::macdisk::fs::sniff;

    const MAX_DISKS: u32 = 64;
    const MISSING_DISKS_BEFORE_STOPPING: u32 = 8;
    const MISSING_PARTITIONS_BEFORE_STOPPING: u32 = 4;

    /// Whether the device exists, asking for no access at all: no read, no admin.
    fn exists(path: &str) -> bool {
        std::fs::OpenOptions::new().access_mode(0).share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE).open(path).is_ok()
    }

    /// One `DeviceIoControl` with byte buffers in and out; the bytes returned, or `None`.
    fn ioctl(file: &File, code: u32, input: &[u8], out: &mut [u8]) -> Option<usize> {
        let mut returned = 0u32;
        // SAFETY: the handle is open for the duration of the call; both buffers are valid
        // for the lengths passed; no OVERLAPPED, so the call completes synchronously.
        let ok = unsafe {
            DeviceIoControl(
                file.as_raw_handle(),
                code,
                input.as_ptr().cast(),
                input.len() as u32,
                out.as_mut_ptr().cast(),
                out.len() as u32,
                &mut returned,
                std::ptr::null_mut(),
            )
        };
        (ok != 0).then_some(returned as usize)
    }

    pub fn partitions() -> Vec<PartitionRef> {
        let mut out = Vec::new();
        let mut missing_disks = 0;
        for disk in 0..MAX_DISKS {
            // Partition0 is the whole disk: its existence says whether the disk does.
            if !exists(&PartitionRef { disk, partition: 0 }.device_path()) {
                missing_disks += 1;
                if missing_disks >= MISSING_DISKS_BEFORE_STOPPING {
                    break;
                }
                continue;
            }
            missing_disks = 0;
            let mut missing = 0;
            for partition in 1..=128 {
                let p = PartitionRef { disk, partition };
                if exists(&p.device_path()) {
                    missing = 0;
                    out.push(p);
                } else {
                    missing += 1;
                    if missing >= MISSING_PARTITIONS_BEFORE_STOPPING {
                        break;
                    }
                }
            }
        }
        out
    }

    pub fn probe(p: &PartitionRef) -> Option<MacPartition> {
        let file = File::open(p.device_path()).ok()?;
        let mut len = [0u8; 8];
        ioctl(&file, IOCTL_DISK_GET_LENGTH_INFO, &[], &mut len)?;
        let len = u64::from_le_bytes(len);
        if len < 4096 {
            return None;
        }
        let mut reader = AlignedReader::new(file, len);
        let mut head = vec![0u8; 4096];
        reader.seek(SeekFrom::Start(0)).ok()?;
        reader.read_exact(&mut head).ok()?;
        let kind = sniff(&head)?;
        let file = File::open(p.device_path()).ok()?;
        // STORAGE_PROPERTY_QUERY { StorageDeviceProperty, PropertyStandardQuery }: all zero.
        let mut descriptor = vec![0u8; 1024];
        let disk_name = ioctl(&file, IOCTL_STORAGE_QUERY_PROPERTY, &[0u8; 12], &mut descriptor)
            .and_then(|n| descriptor_name(&descriptor[..n]));
        Some(MacPartition { part: p.clone(), kind, len, disk_name })
    }
}

/// Partitions that exist right now, found with access-0 opens: cheap enough for a poll.
/// Empty on macOS and Linux, where Mac disks mount natively.
pub fn partitions() -> Vec<PartitionRef> {
    #[cfg(windows)]
    return platform::partitions();
    #[cfg(not(windows))]
    Vec::new()
}

/// Reads the first 4 KiB of `p`; `Some` when it holds HFS+ or APFS.
pub fn probe(p: &PartitionRef) -> Option<MacPartition> {
    #[cfg(windows)]
    return platform::probe(p);
    #[cfg(not(windows))]
    {
        let _ = p;
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_round_trip() {
        let p = PartitionRef { disk: 3, partition: 2 };
        assert_eq!(p.device_path(), r"\\?\GLOBALROOT\Device\Harddisk3\Partition2");
        // Counted, not compared: a literal can lose a backslash on both sides of `==`.
        assert_eq!(p.device_path().chars().take_while(|&c| c == '\\').count(), 2);
        assert_eq!(p.id(), "disk3-part2");
        assert_eq!(PartitionRef::parse_id("disk3-part2"), Some(p));
        assert_eq!(PartitionRef::parse_id("disk3"), None);
        assert_eq!(PartitionRef::parse_id("disk3-part"), None);
        assert_eq!(PartitionRef::parse_id("../etc"), None);
    }

    #[test]
    fn names_come_from_the_descriptor_strings() {
        // A STORAGE_DEVICE_DESCRIPTOR: vendor offset at 12, product offset at 16.
        let mut d = vec![0u8; 64];
        d[12..16].copy_from_slice(&40u32.to_le_bytes());
        d[16..20].copy_from_slice(&48u32.to_le_bytes());
        d[40..46].copy_from_slice(b"ADATA\0");
        d[48..63].copy_from_slice(b"HD710 PRO     \0");
        assert_eq!(descriptor_name(&d), Some("ADATA HD710 PRO".into()));
        let mut d = vec![0u8; 64];
        d[16..20].copy_from_slice(&48u32.to_le_bytes());
        d[48..53].copy_from_slice(b"Disk\0");
        assert_eq!(descriptor_name(&d), Some("Disk".into()), "no vendor");
        assert_eq!(descriptor_name(&[0u8; 64]), None);
    }
}
