//! Verify after copy (Round 19 §13): before an import places a staged copy in `originals/`,
//! the copy is read back from the disk and hashed again, and a copy whose BLAKE3 differs from
//! the bytes the source gave is not imported.
//!
//! The staging write ends with `sync_all`, so the bytes are on the disk; a plain read-back
//! would most likely be served from the page cache and prove only that the cache is right.
//! On Windows the read-back is unbuffered (`FILE_FLAG_NO_BUFFERING`), so it comes from the
//! device. A volume that refuses unbuffered reads (a share, an odd sector size) falls back to
//! a plain read, as do the other systems.

use std::io::Read;
use std::path::Path;

use crate::ImportError;

/// What the history says about a copy that did not read back as the source's bytes.
pub const MISMATCH: &str =
    "The copy in the library did not match the original, so it was not imported. The original is untouched; import it again.";

/// Reads `staged` back from the disk and fails unless its BLAKE3 is `expected`.
pub fn verify_staged(staged: &Path, expected: &str) -> Result<(), ImportError> {
    let actual = read_back_hash(staged)?;
    if actual == expected {
        Ok(())
    } else {
        tracing::warn!(file = %staged.display(), expected, actual, "staged copy did not read back as the source");
        Err(ImportError::Verify(MISMATCH.into()))
    }
}

/// The file's BLAKE3, read past the cache where the system allows it.
pub fn read_back_hash(path: &Path) -> std::io::Result<String> {
    #[cfg(windows)]
    match unbuffered::hash(path) {
        Ok(Some(hash)) => return Ok(hash),
        Ok(None) => {}
        Err(e) => return Err(e),
    }
    plain_hash(path)
}

fn plain_hash(path: &Path) -> std::io::Result<String> {
    let mut hasher = blake3::Hasher::new();
    hasher.update_reader(std::fs::File::open(path)?)?;
    Ok(hasher.finalize().to_hex().to_string())
}

#[cfg(windows)]
mod unbuffered {
    use super::*;
    use std::os::windows::fs::OpenOptionsExt;
    use windows_sys::Win32::Storage::FileSystem::FILE_FLAG_NO_BUFFERING;

    /// Sector alignment for the buffer, offsets and lengths: 4 KiB covers 512-byte and
    /// 4K-native disks.
    const ALIGN: usize = 4096;
    const CHUNK: usize = 1 << 20;

    /// `Ok(None)` when the volume refuses unbuffered reads (the caller reads plainly).
    pub(super) fn hash(path: &Path) -> std::io::Result<Option<String>> {
        let Ok(mut file) = std::fs::OpenOptions::new().read(true).custom_flags(FILE_FLAG_NO_BUFFERING).open(path) else {
            return Ok(None);
        };
        let len = file.metadata()?.len();
        let mut backing = vec![0u8; CHUNK + ALIGN];
        let start = backing.as_ptr().align_offset(ALIGN);
        let buf = &mut backing[start..start + CHUNK];
        let mut hasher = blake3::Hasher::new();
        let mut total = 0u64;
        // Every read starts at a multiple of CHUNK, so offsets stay aligned; the last read
        // is short and ends the loop before a read from an unaligned end of file.
        while total < len {
            let n = match file.read(buf) {
                Ok(0) => return Err(std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "the copy is shorter than it was")),
                Ok(n) => n,
                // ERROR_INVALID_PARAMETER: the sector size is not a divisor of ALIGN.
                Err(e) if e.kind() == std::io::ErrorKind::InvalidInput && total == 0 => return Ok(None),
                Err(e) => return Err(e),
            };
            hasher.update(&buf[..n]);
            total += n as u64;
            if n < CHUNK {
                break;
            }
        }
        if total != len {
            return Err(std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "the copy is shorter than it was"));
        }
        Ok(Some(hasher.finalize().to_hex().to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Sizes around the chunk and sector edges read back to the same hash as a plain read.
    #[test]
    fn read_back_matches_a_plain_hash_at_every_edge() {
        let dir = tempfile::tempdir().unwrap();
        for len in [0usize, 1, 4095, 4096, 4097, (1 << 20) - 1, 1 << 20, (1 << 20) + 1, 3 * (1 << 20) + 17] {
            let p = dir.path().join(format!("f{len}"));
            let bytes: Vec<u8> = (0..len).map(|i| (i * 31 % 251) as u8).collect();
            std::fs::write(&p, &bytes).unwrap();
            assert_eq!(read_back_hash(&p).unwrap(), blake3::hash(&bytes).to_hex().to_string(), "len {len}");
        }
    }

    #[test]
    fn a_changed_byte_fails_verification() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("f");
        std::fs::write(&p, b"hello lumen").unwrap();
        let expected = blake3::hash(b"hello lumen").to_hex().to_string();
        assert!(verify_staged(&p, &expected).is_ok());
        std::fs::write(&p, b"hello lumeN").unwrap();
        assert!(matches!(verify_staged(&p, &expected), Err(ImportError::Verify(_))));
    }

    /// The unbuffered path is the one taken on a local NTFS temp dir, not the fallback.
    #[cfg(windows)]
    #[test]
    fn local_disks_take_the_unbuffered_read() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("f");
        std::fs::write(&p, vec![7u8; 10_000]).unwrap();
        assert!(unbuffered::hash(&p).unwrap().is_some());
    }
}
