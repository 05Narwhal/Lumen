//! The well-behaved tools plugin: `tools()` lists `echo` and `record`.
//! - `echo` returns `{"args":<args>,"scope":<photos.scope() length>}` and logs a line;
//! - `record` appends `in;` to the `calls` storage file, sleeps 200 ms, appends `out;`, so two
//!   overlapping calls would show as `in;in;`.

wit_bindgen::generate!({ world: "tools-plugin", path: "../../../src-tauri/crates/photos-plugins/wit" });

use exports::lumen::plugin::tools::Guest;
use lumen::plugin::types::Tool;
use lumen::plugin::{host, photos, storage};

struct Fixture;

fn append(text: &str) -> Result<(), String> {
    let mut bytes = storage::read("calls")?.unwrap_or_default();
    bytes.extend_from_slice(text.as_bytes());
    storage::write("calls", &bytes)
}

impl Guest for Fixture {
    fn tools() -> Vec<Tool> {
        vec![
            Tool { name: "echo".into(), input_schema: "{\"type\":\"object\"}".into() },
            Tool { name: "record".into(), input_schema: "{\"type\":\"object\"}".into() },
        ]
    }

    fn call(name: String, args_json: String) -> Result<String, String> {
        match name.as_str() {
            "echo" => {
                host::log("echo called");
                Ok(format!("{{\"args\":{},\"scope\":{}}}", args_json, photos::scope().len()))
            }
            "record" => {
                append("in;")?;
                std::thread::sleep(std::time::Duration::from_millis(200));
                append("out;")?;
                Ok("{}".into())
            }
            other => Err(format!("no tool {other}")),
        }
    }
}

export!(Fixture);
