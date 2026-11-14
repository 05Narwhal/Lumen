//! Asks for 600 MB in enumerate.

wit_bindgen::generate!({ world: "importer-plugin", path: "../../../src-tauri/crates/photos-plugins/wit" });

use exports::lumen::plugin::importer::Guest;
use lumen::plugin::types::{Extras, Item};


struct Fixture;

impl Guest for Fixture {
    fn enumerate() -> Result<Vec<Item>, String> {
        let big = vec![1u8; 600 << 20];
        Ok(vec![Item { key: "big".into(), name: "big".into(), size: std::hint::black_box(big).len() as u64, modified: None, path: None }])
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
