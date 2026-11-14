//! `lumen:plugin/storage` (spec §3): a plugin's own named files in `plugins/data/<id>/`, up to
//! 50 MB in all. A name is one plain file name, never a path.

use std::fs;
use std::path::PathBuf;

use super::bindings::lumen::plugin::storage::Host;
use super::ctx::HostState;

pub(crate) const QUOTA: u64 = 50 << 20;

pub(crate) struct Storage {
    dir: PathBuf,
}

/// Names Windows opens as devices (`NUL`, `COM1`, `con.txt`…) in any folder, never as files.
fn is_device(name: &str) -> bool {
    let stem = name.split('.').next().unwrap_or(name).to_ascii_uppercase();
    matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || (stem.len() == 4
            && (stem.starts_with("COM") || stem.starts_with("LPT"))
            && stem.as_bytes()[3].is_ascii_digit())
}

fn check_name(name: &str) -> Result<(), String> {
    let ok = !name.is_empty()
        && name.len() <= 128
        && name != "."
        && name != ".."
        // Windows drops a trailing dot, so `a.` would name `a`.
        && !name.ends_with('.')
        && !is_device(name)
        && name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'));
    if ok {
        Ok(())
    } else {
        Err(format!("\"{name}\" isn't a storage name (letters, digits, '.', '-', '_')"))
    }
}

impl Storage {
    pub(crate) fn new(dir: PathBuf) -> Storage {
        Storage { dir }
    }

    fn used_without(&self, name: &str) -> u64 {
        fs::read_dir(&self.dir)
            .map(|r| {
                r.flatten()
                    .filter(|e| e.file_name() != name)
                    .filter_map(|e| e.metadata().ok())
                    .filter(|m| m.is_file())
                    .map(|m| m.len())
                    .sum()
            })
            .unwrap_or(0)
    }

    pub(crate) fn read(&self, name: &str) -> Result<Option<Vec<u8>>, String> {
        check_name(name)?;
        match fs::read(self.dir.join(name)) {
            Ok(b) => Ok(Some(b)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.to_string()),
        }
    }

    pub(crate) fn write(&self, name: &str, bytes: &[u8]) -> Result<(), String> {
        check_name(name)?;
        if self.used_without(name) + bytes.len() as u64 > QUOTA {
            return Err("storage is full (50 MB)".into());
        }
        fs::create_dir_all(&self.dir).map_err(|e| e.to_string())?;
        let tmp = self.dir.join(format!(".{name}.tmp"));
        fs::write(&tmp, bytes).map_err(|e| e.to_string())?;
        fs::rename(&tmp, self.dir.join(name)).map_err(|e| {
            let _ = fs::remove_file(&tmp);
            e.to_string()
        })
    }

    pub(crate) fn remove(&self, name: &str) -> Result<(), String> {
        check_name(name)?;
        match fs::remove_file(self.dir.join(name)) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.to_string()),
            _ => Ok(()),
        }
    }

    pub(crate) fn names(&self) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(&self.dir)
            .map(|r| {
                r.flatten()
                    .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
                    .map(|e| e.file_name().to_string_lossy().into_owned())
                    .filter(|n| check_name(n).is_ok() && !n.ends_with(".tmp"))
                    .collect()
            })
            .unwrap_or_default();
        names.sort();
        names
    }
}

impl Host for HostState {
    fn read(&mut self, name: String) -> Result<Option<Vec<u8>>, String> {
        self.storage.read(&name)
    }

    fn write(&mut self, name: String, bytes: Vec<u8>) -> Result<(), String> {
        self.storage.write(&name, &bytes)
    }

    fn remove(&mut self, name: String) -> Result<(), String> {
        self.storage.remove(&name)
    }

    fn names(&mut self) -> Vec<String> {
        self.storage.names()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn storage_quota_and_names() {
        let dir = tempfile::tempdir().unwrap();
        let s = Storage::new(dir.path().join("data"));
        assert_eq!(s.read("a").unwrap(), None);
        s.write("big", &vec![0u8; 49 << 20]).unwrap();
        assert_eq!(s.write("more", &vec![0u8; 2 << 20]).unwrap_err(), "storage is full (50 MB)");
        // Replacing a file counts only the new size.
        s.write("big", &vec![0u8; 49 << 20]).unwrap();
        s.write("small.json", b"{}").unwrap();
        assert!(dir.path().join("data/small.json").is_file());
        for bad in ["../x", "a/b", "a\\b", "..", "", "C:x", "CON", "nul", "com1", "LPT9.txt", "aux.json", "a."] {
            assert!(s.write(bad, b"x").is_err(), "{bad}");
            assert!(s.read(bad).is_err(), "{bad}");
        }
        // Device-like but not devices.
        for ok in ["console", "com10", "nul_", "lpt"] {
            s.write(ok, b"x").unwrap();
            s.remove(ok).unwrap();
        }
        assert_eq!(s.names(), vec!["big".to_string(), "small.json".to_string()]);
        s.remove("big").unwrap();
        s.remove("big").unwrap();
        assert_eq!(s.names(), vec!["small.json".to_string()]);
    }
}
