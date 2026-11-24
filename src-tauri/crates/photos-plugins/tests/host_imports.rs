//! The install-time imports check (Round 23 A2 Task 3).

use std::fs;
use std::path::Path;

use photos_plugins::host::ImportsCheck;
use photos_plugins::package::{open_folder, KindCheck, Report};

const BASE: &str = r#"id = "com.test.x"
name = "X"
version = "1.0.0"
author = "Tester"
description = "A test plugin."
license = "MIT"
lumen = ">=26.0.0"
api = 1
"#;

const IMPORTER: &str = r#"
[importer]
label = "Test Source"
pick = "folder"
"#;

/// Runs the check over a folder holding `fixture` as `plugin.wasm` and a manifest with
/// `permissions` and `extra`.
fn check(fixture: &str, permissions: &[&str], extra: &str) -> Report {
    let dir = tempfile::tempdir().unwrap();
    let perms: Vec<String> = permissions.iter().map(|p| format!("\"{p}\"")).collect();
    let toml = format!("{BASE}permissions = [{}]\n{extra}", perms.join(", "));
    fs::write(dir.path().join("plugin.toml"), toml).unwrap();
    fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("tests/wasm/{fixture}.wasm")),
        dir.path().join("plugin.wasm"),
    )
    .unwrap();
    let pkg = open_folder(dir.path()).unwrap();
    let mut report = Report::default();
    ImportsCheck.check(&pkg, true, &mut report);
    report
}

#[test]
fn claims_the_code_kinds() {
    use photos_plugins::PluginKind;
    assert_eq!(ImportsCheck.kinds(), &[PluginKind::Importer, PluginKind::Tools]);
}

#[test]
fn std_wasi_imports_allowed() {
    let r = check("importer_ok", &["import.read-chosen"], IMPORTER);
    assert!(r.problems.is_empty(), "{:?}", r.problems);
    assert_eq!(r.adds, vec!["An importer: \"Test Source\"".to_string()]);
}

#[test]
fn undeclared_import_refused() {
    let r = check("undeclared", &["import.read-chosen"], IMPORTER);
    assert!(
        r.problems
            .iter()
            .any(|p| p.contains("uses lumen:plugin/storage without asking for it")),
        "{:?}",
        r.problems
    );
}

#[test]
fn network_import_refused() {
    let r = check("socket", &["import.read-chosen"], IMPORTER);
    assert!(
        r.problems.iter().any(|p| p.contains("uses the network")),
        "{:?}",
        r.problems
    );
}

#[test]
fn exports_match_manifest() {
    let r = check("tools_ok", &["photos.read", "storage"], IMPORTER);
    assert!(
        r.problems
            .iter()
            .any(|p| p.contains("declares an importer but plugin.wasm doesn't export one")),
        "{:?}",
        r.problems
    );
}

const UI_PAGE: &str = r#"
[[tools]]
name = "scan"
description = "Scans."

[[ui.section]]
title = "Scan"

[[ui.section.control]]
type = "button"
label = "Scan now"
action = "scan"
"#;

/// `check` with the manifest on API 1.2, where `[ui]` is allowed.
fn check12(fixture: &str, permissions: &[&str], extra: &str) -> Report {
    let dir = tempfile::tempdir().unwrap();
    let perms: Vec<String> = permissions.iter().map(|p| format!("\"{p}\"")).collect();
    let toml = format!("{}permissions = [{}]\n{extra}", BASE.replace("api = 1", "api = \"1.2\""), perms.join(", "));
    fs::write(dir.path().join("plugin.toml"), toml).unwrap();
    fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("tests/wasm/{fixture}.wasm")),
        dir.path().join("plugin.wasm"),
    )
    .unwrap();
    let pkg = open_folder(dir.path()).unwrap();
    let mut report = Report::default();
    ImportsCheck.check(&pkg, true, &mut report);
    report
}

#[test]
fn a_contribution_action_needs_the_ui_export_but_open_review_does_not() {
    let c = |action: &str| format!("\n[[tools]]\nname = \"scan\"\ndescription = \"Scans.\"\n\n[[ui.contribute]]\nslot = \"palette\"\nlabel = \"Go\"\naction = \"{action}\"\n");
    let r = check12("tools_ok", &["photos.read", "storage"], &c("scan"));
    assert!(r.problems.iter().any(|p| p.contains("contribution") && p.contains("doesn't export `ui`")), "{:?}", r.problems);
    let r = check12("tools_ok", &["photos.read", "storage"], &c("open-review:places"));
    assert!(!r.problems.iter().any(|p| p.contains("contribution")), "{:?}", r.problems);
}

#[test]
fn buttons_need_the_ui_export() {
    let r = check12("tools_ok", &["photos.read", "storage"], UI_PAGE);
    assert!(
        r.problems.iter().any(|p| p.contains("buttons, stats or lists") && p.contains("doesn't export `ui`")),
        "{:?}",
        r.problems
    );
}

#[test]
fn a_plugin_that_exports_ui_passes() {
    let r = check12("ui_ok", &["storage"], UI_PAGE);
    assert!(r.problems.is_empty(), "{:?}", r.problems);
}
