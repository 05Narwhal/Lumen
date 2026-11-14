//! Tries to reach the network from enumerate and returns the error it got.

wit_bindgen::generate!({ world: "importer-plugin", path: "../../../src-tauri/crates/photos-plugins/wit" });

use exports::lumen::plugin::importer::Guest;
use lumen::plugin::types::{Extras, Item};


struct Fixture;

impl Guest for Fixture {
    fn enumerate() -> Result<Vec<Item>, String> {
        match std::net::TcpStream::connect("1.1.1.1:80") {
            Ok(_) => Ok(Vec::new()),
            Err(e) => Err(format!("socket: {e}")),
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
