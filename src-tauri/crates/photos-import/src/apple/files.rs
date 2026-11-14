//! Where a library's files come from: a folder this computer can open (a mounted disk, a
//! copied library, `~/Pictures` on a Mac), or a Mac disk read through [`MacFs`].

use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;

use crate::macdisk::fs::MacFs;
use crate::ImportError;

/// Paths are relative to the `.photoslibrary` folder and use `/`.
pub trait LibraryFiles: Send + Sync {
    /// Files (not folders) directly in `dir`, with their sizes.
    fn list(&self, dir: &str) -> Result<Vec<(String, u64)>, ImportError>;
    fn copy_to(&self, rel: &str, out: &mut dyn Write) -> Result<u64, ImportError>;
}

pub struct DirFiles(pub PathBuf);

impl LibraryFiles for DirFiles {
    fn list(&self, dir: &str) -> Result<Vec<(String, u64)>, ImportError> {
        let mut out = Vec::new();
        for entry in std::fs::read_dir(self.0.join(dir))? {
            let entry = entry?;
            let meta = entry.metadata()?;
            if meta.is_file() {
                out.push((entry.file_name().to_string_lossy().into_owned(), meta.len()));
            }
        }
        Ok(out)
    }

    fn copy_to(&self, rel: &str, out: &mut dyn Write) -> Result<u64, ImportError> {
        let mut file = std::fs::File::open(self.0.join(rel))?;
        Ok(std::io::copy(&mut file, out)?)
    }
}

pub struct MacDiskFiles {
    pub fs: Arc<MacFs>,
    /// The library's absolute path on the volume.
    pub root: String,
}

impl MacDiskFiles {
    fn path(&self, rel: &str) -> String {
        format!("{}/{}", self.root.trim_end_matches('/'), rel)
    }
}

impl LibraryFiles for MacDiskFiles {
    fn list(&self, dir: &str) -> Result<Vec<(String, u64)>, ImportError> {
        Ok(self.fs.list(&self.path(dir))?.into_iter().filter(|e| !e.is_dir).map(|e| (e.name, e.size)).collect())
    }

    fn copy_to(&self, rel: &str, out: &mut dyn Write) -> Result<u64, ImportError> {
        self.fs.copy_to(&self.path(rel), out)
    }
}
