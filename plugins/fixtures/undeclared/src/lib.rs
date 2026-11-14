//! Calls storage, which an importer asking only for import.read-chosen did not ask for.

wit_bindgen::generate!({ world: "importer-plugin", path: "../../../src-tauri/crates/photos-plugins/wit" });

use exports::lumen::plugin::importer::Guest;
use lumen::plugin::types::{Extras, Item};
use lumen::plugin::storage;

struct Fixture;

impl Guest for Fixture {
    fn enumerate() -> Result<Vec<Item>, String> {
        Err(format!("names: {}", storage::names().len()))
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
