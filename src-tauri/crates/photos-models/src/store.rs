use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::manifest::ModelEntry;

/// Where the downloaded models are, and which of them are usable.
#[derive(Debug, Clone)]
pub struct ModelStore {
    pub dir: PathBuf,
}

impl ModelStore {
    pub fn path_of(&self, e: &ModelEntry) -> PathBuf {
        self.dir.join(&e.file)
    }

    /// Present at its full size. The download verified the hash before the atomic rename
    /// that put it here, so a file at this path at this size is the file the manifest names.
    pub fn is_ready(&self, e: &ModelEntry) -> bool {
        std::fs::metadata(self.path_of(e)).map(|m| m.len() == e.bytes).unwrap_or(false)
    }
}

/// Where a download is written until its hash checks out: beside the target, so the rename
/// that publishes it is on one volume and atomic.
pub fn part_path(p: &Path) -> PathBuf {
    let mut s = p.as_os_str().to_owned();
    s.push(".part");
    PathBuf::from(s)
}

/// Whether `path` is exactly the file `entry` names: its size, then its SHA-256.
pub fn verify(path: &Path, entry: &ModelEntry) -> bool {
    let Ok(meta) = std::fs::metadata(path) else { return false };
    if meta.len() != entry.bytes {
        return false;
    }
    let Ok(mut f) = std::fs::File::open(path) else { return false };
    let mut h = Sha256::new();
    if std::io::copy(&mut f, &mut h).is_err() {
        return false;
    }
    format!("{:x}", h.finalize()) == entry.sha256
}

/// The `Range` to ask for when `have` bytes of `total` are already on disk: nothing to ask
/// when the file is complete, and no range for a fresh start.
pub fn range_from(have: u64, total: u64) -> Option<Option<String>> {
    match have {
        0 => Some(None),
        h if h < total => Some(Some(format!("bytes={h}-"))),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(bytes: u64, sha: &str) -> ModelEntry {
        serde_json::from_str(&format!(
            r#"{{ "id": "t", "role": "x", "file": "t.onnx", "bytes": {bytes}, "sha256": "{sha}", "licence": "MIT" }}"#
        ))
        .unwrap()
    }

    #[test]
    fn verify_checks_size_then_hash() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("t.onnx");
        std::fs::write(&p, b"abc").unwrap();
        let good = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
        assert!(verify(&p, &entry(3, good)));
        assert!(!verify(&p, &entry(4, good)));
        assert!(!verify(&p, &entry(3, &"0".repeat(64))));
        assert!(!verify(&dir.path().join("missing.onnx"), &entry(3, good)));
        assert_eq!(part_path(&p), dir.path().join("t.onnx.part"));
    }

    #[test]
    fn range_from_asks_for_the_rest() {
        assert_eq!(range_from(0, 100), Some(None), "a fresh start asks for everything");
        assert_eq!(range_from(40, 100), Some(Some("bytes=40-".into())));
        assert_eq!(range_from(100, 100), None, "complete: nothing to ask for");
    }
}
