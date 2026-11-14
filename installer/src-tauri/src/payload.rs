//! The NSIS installer a full Setup carries.
use std::io;
use std::path::{Path, PathBuf};

/// Write the embedded installer into `dir` and return its path. A slim Setup (built without the
/// `payload` feature) carries none.
/// The embedded installer, one place for both `write` and `len`.
#[cfg(feature = "payload")]
static PAYLOAD: &[u8] = include_bytes!(env!("LUMEN_SETUP_PAYLOAD"));

/// The installer's size in bytes; 0 for a slim Setup.
#[cfg(feature = "payload")]
pub fn len() -> u64 {
    PAYLOAD.len() as u64
}

#[cfg(not(feature = "payload"))]
pub fn len() -> u64 {
    0
}

#[cfg(feature = "payload")]
pub fn write(dir: &Path) -> io::Result<PathBuf> {
    // The release build names the signed installer: LUMEN_SETUP_PAYLOAD=<absolute path>.
    let path = dir.join("lumen-installer.exe");
    std::fs::write(&path, PAYLOAD)?;
    Ok(path)
}

#[cfg(not(feature = "payload"))]
pub fn write(_dir: &Path) -> io::Result<PathBuf> {
    Err(io::Error::new(io::ErrorKind::NotFound, "this Setup carries no installer"))
}
