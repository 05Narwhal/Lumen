//! `lumen:plugin/chosen` (spec §3): list, stat and read below the folder or file the person
//! chose for an import, and nowhere else. The root is canonicalised once; every path a plugin
//! names is canonicalised too (resolving symlinks and junctions) and must stay below it.

use std::fs;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Component, Path, PathBuf};
use std::time::UNIX_EPOCH;

use super::bindings::lumen::plugin::chosen::Host;
use super::bindings::lumen::plugin::types::Entry;
use super::ctx::HostState;
use crate::{Pick, PluginError};

/// The most one `read` returns.
const MAX_READ: u32 = 16 << 20;

/// The folder or file an import may read.
#[derive(Debug, Clone)]
pub struct ChosenRoot {
    root: PathBuf,
    pick: Pick,
}

fn outside(rel: &str) -> PluginError {
    PluginError::chosen_root(format!("{rel} is outside the chosen folder"))
}

impl ChosenRoot {
    pub fn new(root: &Path, pick: Pick) -> Result<ChosenRoot, PluginError> {
        let canon = fs::canonicalize(root)
            .map_err(|e| PluginError::chosen_root(format!("{}: {e}", root.display())))?;
        let ok = match pick {
            Pick::Folder => canon.is_dir(),
            Pick::File => canon.is_file(),
        };
        if !ok {
            return Err(PluginError::chosen_root(format!(
                "{} isn't a {}",
                root.display(),
                if pick == Pick::Folder { "folder" } else { "file" }
            )));
        }
        Ok(ChosenRoot { root: canon, pick })
    }

    /// The canonical folder or file the person chose.
    pub fn path(&self) -> &Path {
        &self.root
    }

    fn file_name(&self) -> String {
        self.root.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
    }

    /// The real path of `rel` (forward or back slashes), refused if it leaves the root.
    pub fn resolve(&self, rel: &str) -> Result<PathBuf, PluginError> {
        let rel_path = Path::new(rel);
        if rel_path.components().any(|c| !matches!(c, Component::Normal(_) | Component::CurDir))
            || rel.starts_with(['/', '\\'])
            || rel.contains(':')
        {
            return Err(outside(rel));
        }
        let joined = match self.pick {
            Pick::Folder => self.root.join(rel_path),
            // A picked file is its own root: only its own name (or "") names it.
            Pick::File if rel.is_empty() || rel == self.file_name() => self.root.clone(),
            Pick::File => return Err(outside(rel)),
        };
        let canon = fs::canonicalize(&joined)
            .map_err(|e| PluginError::invalid(format!("{rel}: {e}")))?;
        if !canon.starts_with(&self.root) {
            return Err(outside(rel));
        }
        Ok(canon)
    }

    pub(crate) fn list(&self, dir: &str) -> Result<Vec<Entry>, PluginError> {
        if self.pick == Pick::File {
            return if dir.is_empty() { Ok(vec![self.stat(&self.file_name())?]) } else { Err(outside(dir)) };
        }
        let path = self.resolve(dir)?;
        let read = fs::read_dir(&path).map_err(|e| PluginError::invalid(format!("{dir}: {e}")))?;
        let mut out = Vec::new();
        for e in read.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            let rel = if dir.is_empty() { name.clone() } else { format!("{}/{name}", dir.trim_end_matches('/')) };
            // An entry that resolves outside (a link out of the root) isn't listed.
            if let Ok(real) = self.resolve(&rel) {
                if let Ok(meta) = fs::metadata(&real) {
                    out.push(entry(name, &meta));
                }
            }
        }
        out.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(out)
    }

    pub(crate) fn stat(&self, rel: &str) -> Result<Entry, PluginError> {
        let path = self.resolve(rel)?;
        let meta = fs::metadata(&path).map_err(|e| PluginError::invalid(format!("{rel}: {e}")))?;
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        Ok(entry(name, &meta))
    }

    pub(crate) fn read(&self, rel: &str, offset: u64, len: u32) -> Result<Vec<u8>, PluginError> {
        let path = self.resolve(rel)?;
        let io = |e: std::io::Error| PluginError::invalid(format!("{rel}: {e}"));
        let mut f = fs::File::open(&path).map_err(io)?;
        f.seek(SeekFrom::Start(offset)).map_err(io)?;
        let mut buf = Vec::with_capacity(len.min(MAX_READ) as usize);
        f.take(u64::from(len.min(MAX_READ))).read_to_end(&mut buf).map_err(io)?;
        Ok(buf)
    }
}

fn entry(name: String, meta: &fs::Metadata) -> Entry {
    Entry {
        name,
        is_dir: meta.is_dir(),
        size: if meta.is_dir() { 0 } else { meta.len() },
        // Unix seconds.
        modified: meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|d| d.as_secs() as i64),
    }
}

impl HostState {
    fn chosen(&self) -> Result<&ChosenRoot, String> {
        self.ctx.chosen.as_ref().ok_or_else(|| "nothing was chosen for this plugin".to_string())
    }
}

impl Host for HostState {
    fn list(&mut self, dir: String) -> Result<Vec<Entry>, String> {
        self.chosen()?.list(&dir).map_err(|e| e.message)
    }

    fn stat(&mut self, path: String) -> Result<Entry, String> {
        self.chosen()?.stat(&path).map_err(|e| e.message)
    }

    fn read(&mut self, path: String, offset: u64, len: u32) -> Result<Vec<u8>, String> {
        self.chosen()?.read(&path, offset, len).map_err(|e| e.message)
    }

    /// Restarts the call's clock (spec §2) unless the import was cancelled.
    fn progress(&mut self, _done: u64, _total: Option<u64>) -> Result<(), String> {
        if self.cancelled() {
            return Err("cancelled".into());
        }
        self.started = std::time::Instant::now();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("root");
        fs::create_dir_all(root.join("a")).unwrap();
        fs::write(root.join("a/b.jpg"), b"jpeg").unwrap();
        fs::write(dir.path().join("x"), b"outside").unwrap();
        (dir, root)
    }

    #[test]
    fn chosen_root_confines() {
        let (_d, root) = tree();
        let c = ChosenRoot::new(&root, Pick::Folder).unwrap();
        assert!(c.resolve("a/b.jpg").is_ok());
        assert!(c.resolve("a\\b.jpg").is_ok());
        for bad in ["../x", "a/../../x", "/etc/x", "C:/x", "\\x"] {
            assert_eq!(c.resolve(bad).unwrap_err().code, "plugin_chosen_root", "{bad}");
        }
        assert_eq!(c.read("a/b.jpg", 1, 10).unwrap(), b"peg");
        let names: Vec<_> = c.list("").unwrap().into_iter().map(|e| (e.name, e.is_dir)).collect();
        assert_eq!(names, vec![("a".to_string(), true)]);
    }

    #[cfg(windows)]
    #[test]
    fn junction_escape_refused() {
        let (d, root) = tree();
        let out = d.path().join("elsewhere");
        fs::create_dir(&out).unwrap();
        fs::write(out.join("secret.txt"), b"s").unwrap();
        let link = root.join("j");
        let status = std::process::Command::new("cmd")
            .args(["/c", "mklink", "/J"])
            .arg(&link)
            .arg(&out)
            .output()
            .unwrap();
        assert!(status.status.success(), "{status:?}");
        let c = ChosenRoot::new(&root, Pick::Folder).unwrap();
        assert_eq!(c.resolve("j/secret.txt").unwrap_err().code, "plugin_chosen_root");
        assert!(c.list("").unwrap().iter().all(|e| e.name != "j"));
    }

    #[cfg(unix)]
    #[test]
    fn symlink_escape_refused() {
        let (d, root) = tree();
        std::os::unix::fs::symlink(d.path().join("x"), root.join("l")).unwrap();
        let c = ChosenRoot::new(&root, Pick::Folder).unwrap();
        assert_eq!(c.resolve("l").unwrap_err().code, "plugin_chosen_root");
    }

    #[test]
    fn file_pick_root_is_the_file() {
        let (_d, root) = tree();
        let c = ChosenRoot::new(&root.join("a/b.jpg"), Pick::File).unwrap();
        let list = c.list("").unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].name, "b.jpg");
        assert_eq!(c.read("b.jpg", 0, 4).unwrap(), b"jpeg");
        assert_eq!(c.resolve("other").unwrap_err().code, "plugin_chosen_root");
        assert!(ChosenRoot::new(&root, Pick::File).is_err());
    }
}
