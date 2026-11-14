//! The well-behaved importer. Its `mode` setting picks what `enumerate` does, so the host
//! tests can drive one fixture through several cases:
//! - (none): lists the chosen root, reads the first file through `chosen.read`, logs a line
//!   (and prints one to stdout), returns two items (one with `path`, one read by key);
//! - `setting`: fails with the text of the `k` setting;
//! - `progress`: calls `progress` every 0.5 s for `secs` seconds;
//! - `progress-forever`: calls `progress` until it fails and returns that error;
//! - `spec` (Round 23 A3): lists the items of the text setting `spec` (`key,name,path;...`, an
//!   empty path means the item is read through the plugin); `dir`: every file of the chosen
//!   folder as an item with a `path`. In both, `read` logs `read <key>` and reads the file
//!   named `key` (or traps when the setting `trap_reads` is `true`), `extras` returns a favourite
//!   in the album `Trip` titled by the key (captured at the text setting `captured`), and
//!   `sidecars` lists `<key>.xmp` (read through the plugin) when the root has one.

wit_bindgen::generate!({ world: "importer-plugin", path: "../../../src-tauri/crates/photos-plugins/wit" });

use exports::lumen::plugin::importer::Guest;
use lumen::plugin::types::{Extras, Item};
use lumen::plugin::{chosen, host};

struct Fixture;

/// A text setting without the JSON quotes.
fn text(key: &str) -> Option<String> {
    let raw = host::setting(key)?;
    Some(raw.trim_matches('"').to_string()).filter(|s| !s.is_empty())
}

fn spec_mode() -> bool {
    matches!(text("mode").as_deref(), Some("spec") | Some("dir"))
}

fn size_of(path: &str) -> u64 {
    chosen::stat(path).map(|e| e.size).unwrap_or(0)
}

fn sleep_half() {
    std::thread::sleep(std::time::Duration::from_millis(500));
}

impl Guest for Fixture {
    fn enumerate() -> Result<Vec<Item>, String> {
        match host::setting("mode").as_deref() {
            Some("\"setting\"") => return Err(format!("k={:?}", host::setting("k"))),
            Some("\"progress\"") => {
                let secs: u64 = host::setting("secs").and_then(|s| s.parse().ok()).unwrap_or(3);
                for i in 0..secs * 2 {
                    sleep_half();
                    chosen::progress(i, None)?;
                }
                return Ok(Vec::new());
            }
            // Blocks in a WASI sleep, where epochs can't interrupt it.
            Some("\"sleep\"") => {
                std::thread::sleep(std::time::Duration::from_secs(3600));
                return Ok(Vec::new());
            }
            Some("\"progress-forever\"") => {
                let mut i = 0;
                loop {
                    chosen::progress(i, None)?;
                    i += 1;
                    std::thread::sleep(std::time::Duration::from_millis(20));
                }
            }
            Some("\"spec\"") => {
                let spec = text("spec").unwrap_or_default();
                let mut items = Vec::new();
                for entry in spec.split(';').filter(|e| !e.is_empty()) {
                    let mut f = entry.split(',');
                    let key = f.next().unwrap_or_default().to_string();
                    let name = f.next().unwrap_or_default().to_string();
                    let path = f.next().filter(|p| !p.is_empty()).map(str::to_string);
                    let size = size_of(path.as_deref().unwrap_or(&key));
                    items.push(Item { key, name, size, modified: None, path });
                }
                return Ok(items);
            }
            Some("\"dir\"") => {
                let mut items = Vec::new();
                for e in chosen::list("")?.into_iter().filter(|e| !e.is_dir) {
                    items.push(Item { key: e.name.clone(), name: e.name.clone(), size: e.size, modified: e.modified, path: Some(e.name) });
                }
                return Ok(items);
            }
            _ => {}
        }
        host::log("enumerating");
        println!("hello from stdout");
        let entries = chosen::list("")?;
        let first = entries.iter().find(|e| !e.is_dir).ok_or("no files")?;
        let bytes = chosen::read(&first.name, 0, 4)?;
        Ok(vec![
            Item {
                key: "a".into(),
                name: first.name.clone(),
                size: first.size,
                modified: first.modified,
                path: Some(first.name.clone()),
            },
            Item {
                key: "b".into(),
                name: "b.bin".into(),
                size: bytes.len() as u64,
                modified: None,
                path: None,
            },
        ])
    }

    fn read(key: String, offset: u64, len: u32) -> Result<Vec<u8>, String> {
        if spec_mode() {
            host::log(&format!("read {key}"));
            if text("trap_reads").as_deref() == Some("true") {
                unreachable!("trap_reads");
            }
            return chosen::read(&key, offset, len);
        }
        if key != "b" {
            return Err("no such item".into());
        }
        let entries = chosen::list("")?;
        let first = entries.iter().find(|e| !e.is_dir).ok_or("no files")?;
        chosen::read(&first.name, offset, len)
    }

    fn extras(key: String) -> Option<Extras> {
        if spec_mode() {
            return Some(Extras {
                favourite: true,
                albums: vec!["Trip".into()],
                title: Some(key),
                caption: None,
                keywords: Vec::new(),
                captured_at: text("captured"),
                lat: None,
                lon: None,
            });
        }
        (key == "a").then(|| Extras {
            favourite: true,
            albums: vec!["Trip".into()],
            title: None,
            caption: None,
            keywords: Vec::new(),
            captured_at: None,
            lat: None,
            lon: None,
        })
    }

    fn sidecars(key: String) -> Vec<Item> {
        if spec_mode() {
            let name = format!("{key}.xmp");
            if let Ok(e) = chosen::stat(&name) {
                return vec![Item { key: name.clone(), name, size: e.size, modified: e.modified, path: None }];
            }
        }
        Vec::new()
    }
}

export!(Fixture);
