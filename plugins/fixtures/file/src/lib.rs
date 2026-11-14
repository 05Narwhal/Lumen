//! Tries to read the absolute path in its path setting and returns the error it got.

wit_bindgen::generate!({ world: "importer-plugin", path: "../../../src-tauri/crates/photos-plugins/wit" });

use exports::lumen::plugin::importer::Guest;
use lumen::plugin::types::{Extras, Item};
use lumen::plugin::host;

struct Fixture;

impl Guest for Fixture {
    fn enumerate() -> Result<Vec<Item>, String> {
        let raw = host::setting("path").ok_or("no path setting")?;
        let path = raw.trim_matches(char::from(34));
        match std::fs::read(path) {
            Ok(bytes) => Err(format!("read {} bytes: {}", bytes.len(), String::from_utf8_lossy(&bytes))),
            Err(e) => Err(format!("file: {e}")),
        }
    }

    fn read(_key: String, _offset: u64, _len: u32) -> Result<Vec<u8>, String> {
        Err("nothing to read".into())
    }

    fn extras(_key: String) -> Option<Extras> {
        None
    }

    fn sidecars(_key: String) -> Vec<Item> {
        Vec::new()
    }
}

export!(Fixture);
