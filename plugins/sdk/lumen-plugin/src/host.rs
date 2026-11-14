//! The always-available `host` interface: logging and settings (raw JSON text).

#[cfg(all(target_arch = "wasm32", any(feature = "importer", feature = "tools")))]
mod imp {
    use crate::bindings::lumen::plugin::host;

    pub fn log(line: &str) {
        host::log(line)
    }
    pub fn setting(key: &str) -> Option<String> {
        host::setting(key)
    }
}

// Natively (unit tests, `cargo check` on the host) there is no host to call.
#[cfg(not(all(target_arch = "wasm32", any(feature = "importer", feature = "tools"))))]
mod imp {
    pub fn log(line: &str) {
        eprintln!("[plugin] {line}")
    }
    pub fn setting(_: &str) -> Option<String> {
        None
    }
}

pub use imp::{log, setting};
