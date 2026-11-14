//! Round 17: one version, everywhere. `scripts/release/set-version.mjs` writes these fields; this
//! fails if they ever disagree or leave the scheme (`26.X.Y-alpha|beta|stable`).

use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).parent().expect("the repo root").to_path_buf()
}

fn json(rel: &str) -> serde_json::Value {
    serde_json::from_slice(&std::fs::read(root().join(rel)).unwrap_or_else(|e| panic!("{rel}: {e}"))).unwrap_or_else(|e| panic!("{rel}: {e}"))
}

/// `26.X.Y` when `v` follows the scheme.
fn numeric_part(v: &str) -> Option<&str> {
    let (numeric, channel) = v.split_once('-')?;
    let parts: Vec<&str> = numeric.split('.').collect();
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    (parts.len() == 3 && parts[0] == "26" && digits(parts[1]) && digits(parts[2]) && matches!(channel, "alpha" | "beta" | "stable")).then_some(numeric)
}

#[test]
fn every_version_field_agrees() {
    let cargo = env!("CARGO_PKG_VERSION");
    let package = json("package.json");
    let tauri = json("src-tauri/tauri.conf.json");
    assert_eq!(package["version"].as_str(), Some(cargo), "package.json");
    assert_eq!(tauri["version"].as_str(), Some(cargo), "tauri.conf.json");
    let numeric = numeric_part(cargo).unwrap_or_else(|| panic!("{cargo} is not 26.X.Y-alpha|beta|stable"));
    assert_eq!(tauri["bundle"]["windows"]["wix"]["version"].as_str(), Some(numeric), "the WiX version is the numeric part");
}

#[test]
fn lumen_setup_says_the_version_it_installs() {
    let cargo = env!("CARGO_PKG_VERSION");
    assert_eq!(json("installer/package.json")["version"].as_str(), Some(cargo), "installer/package.json");
    assert_eq!(json("installer/src-tauri/tauri.conf.json")["version"].as_str(), Some(cargo), "installer/src-tauri/tauri.conf.json");
    let toml = std::fs::read_to_string(root().join("installer/src-tauri/Cargo.toml")).unwrap();
    let line = toml.lines().find(|l| l.starts_with("version = ")).expect("Setup's Cargo.toml version");
    assert_eq!(line.trim_start_matches("version = ").trim_matches('"'), cargo, "installer/src-tauri/Cargo.toml");
}

#[test]
fn the_scheme_refuses_what_may_never_be_published() {
    for bad in ["26.1.0", "2026.1.0", "26.1-alpha", "26.1.0-rc", "27.1.0-alpha", "26.1.x-alpha"] {
        assert_eq!(numeric_part(bad), None, "{bad}");
    }
    assert_eq!(numeric_part("26.12.3-beta"), Some("26.12.3"));
}

#[test]
fn the_browser_shim_says_no_old_version() {
    let shim = std::fs::read_to_string(root().join("src/dev/browser-shim.ts")).unwrap();
    assert!(!shim.contains("2026.1.0"), "the shim still says 2026.1.0");
}
