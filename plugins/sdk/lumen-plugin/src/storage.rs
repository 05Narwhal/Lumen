//! The plugin's own private storage (needs the `storage` permission; at most what the host's
//! quota allows). Names are flat; the host refuses path tricks.

#[cfg(all(target_arch = "wasm32", any(feature = "importer", feature = "tools")))]
mod imp {
    use crate::bindings::lumen::plugin::storage as s;

    /// `Ok(None)` when nothing is stored under `name`.
    pub fn read(name: &str) -> Result<Option<Vec<u8>>, String> {
        s::read(name)
    }
    pub fn write(name: &str, bytes: &[u8]) -> Result<(), String> {
        s::write(name, bytes)
    }
    pub fn remove(name: &str) -> Result<(), String> {
        s::remove(name)
    }
    pub fn names() -> Vec<String> {
        s::names()
    }
}

#[cfg(all(target_arch = "wasm32", any(feature = "importer", feature = "tools")))]
pub use imp::*;
