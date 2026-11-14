//! The compile cache (spec §2): `<cache>/<blake3(wasm)>-<engine hash>.cwasm` with the blake3 of
//! that file beside it in `.b3`. A missing, truncated or mismatched file is compiled again and
//! never handed to `Component::deserialize`.

use std::fs;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

use wasmtime::component::Component;

/// The cache key's second half: wasmtime's own compatibility hash (its version and every
/// setting that changes compiled code), so a Lumen with another wasmtime never reuses a file.
pub(crate) fn engine_key(engine: &wasmtime::Engine) -> String {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    engine.precompile_compatibility_hash().hash(&mut h);
    format!("{:016x}", h.finish())
}

fn paths(dir: &Path, wasm: &[u8], key: &str) -> (PathBuf, PathBuf) {
    let stem = format!("{}-{key}", blake3::hash(wasm).to_hex());
    (dir.join(format!("{stem}.cwasm")), dir.join(format!("{stem}.b3")))
}

/// Reads a cached component whose `.b3` matches its bytes.
pub(crate) fn load(engine: &wasmtime::Engine, dir: &Path, wasm: &[u8], key: &str) -> Option<Component> {
    let (cwasm, b3) = paths(dir, wasm, key);
    let bytes = fs::read(&cwasm).ok()?;
    let want = fs::read_to_string(&b3).ok()?;
    if blake3::hash(&bytes).to_hex().as_str() != want.trim() {
        return None;
    }
    // SAFETY: the bytes are exactly what `Component::serialize` wrote for this engine
    // configuration (the `.b3` beside them matches), and wasmtime re-checks the header and
    // its compatibility hash, refusing anything else.
    unsafe { Component::deserialize(engine, &bytes) }.ok()
}

/// Writes a freshly compiled component; a failed write only costs a recompile next time.
pub(crate) fn store(component: &Component, dir: &Path, wasm: &[u8], key: &str) {
    let Ok(bytes) = component.serialize() else { return };
    let (cwasm, b3) = paths(dir, wasm, key);
    let _ = fs::create_dir_all(dir);
    let _ = fs::remove_file(&b3);
    let tmp = cwasm.with_extension("cwasm.tmp");
    if fs::write(&tmp, &bytes).is_ok() && fs::rename(&tmp, &cwasm).is_ok() {
        let _ = fs::write(&b3, blake3::hash(&bytes).to_hex().as_str());
    } else {
        let _ = fs::remove_file(&tmp);
    }
}
