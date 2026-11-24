//! The API 1.2 fixture (Round 25 P3): a settings page's `ui` export.
//! - `action("scan")` answers `scanned`, `action("setting")` the stored value of `depth`,
//!   anything else an error;
//! - `value("last_scan")` is plain text, `value("biggest")` a JSON list, anything else an error.

wit_bindgen::generate!({ world: "ui-plugin", path: "../../../src-tauri/crates/photos-plugins/wit" });

use exports::lumen::plugin::tools::Guest;
use exports::lumen::plugin::ui;
use lumen::plugin::host;
use lumen::plugin::types::Tool;

struct Fixture;

impl Guest for Fixture {
    fn tools() -> Vec<Tool> {
        Vec::new()
    }
    fn call(name: String, _args_json: String) -> Result<String, String> {
        Err(format!("no tool {name}"))
    }
}

impl ui::Guest for Fixture {
    fn action(name: String) -> Result<String, String> {
        match name.as_str() {
            "scan" => Ok("scanned".into()),
            "setting" => Ok(host::setting("depth").unwrap_or_default()),
            other => Err(format!("no action {other}")),
        }
    }
    fn value(key: String) -> Result<String, String> {
        match key.as_str() {
            "last_scan" => Ok("12 MB".into()),
            "biggest" => Ok("[\"a\",{\"text\":\"b\",\"detail\":\"1 GB\"}]".into()),
            other => Err(format!("no value {other}")),
        }
    }
}

export!(Fixture);
