//! API 1.2 (Round 25 P3): the `ui` export answers a settings page's buttons, stats and lists;
//! plugins without it (1.0, 1.1) say so and keep working.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use photos_plugins::host::{Engine, HostCtx, ToolsInstance};
use photos_plugins::PluginPermission;

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("tests/wasm/{name}.wasm"))
}

fn tools(name: &str, depth: Option<i64>) -> (tempfile::TempDir, Arc<ToolsInstance>) {
    let dir = tempfile::tempdir().unwrap();
    let engine = Engine::new(&dir.path().join("cache"), Arc::new(|_, _| {})).unwrap();
    let data = dir.path().join("data");
    let ctx = move || {
        let mut settings = serde_json::Map::new();
        if let Some(d) = depth {
            settings.insert("depth".into(), d.into());
        }
        HostCtx {
            plugin: "com.test.x".into(),
            granted: vec![PluginPermission::PhotosRead, PluginPermission::Storage],
            settings,
            data_dir: data.clone(),
            log: Arc::new(|_: &str| {}),
            cancel: None,
            chosen: None,
            photos: None,
            drafts: None,
            models: None,
            library: None,
            places: None,
            proposals: None,
        }
    };
    let t = engine.tools_at("com.test.x", &fixture(name), ctx).unwrap();
    (dir, t)
}

#[test]
fn ui_actions_and_values_answer() {
    let (_d, t) = tools("ui_ok", Some(5));
    assert!(t.has_ui());
    assert_eq!(t.ui_action("scan").unwrap(), "scanned");
    assert_eq!(t.ui_action("setting").unwrap(), "5");
    assert_eq!(t.ui_value("last_scan").unwrap(), "12 MB");
    assert!(t.ui_value("biggest").unwrap().starts_with('['));
}

#[test]
fn ui_errors_are_the_plugins_words() {
    let (_d, t) = tools("ui_ok", None);
    assert_eq!(t.ui_action("nope").unwrap_err().message, "no action nope");
    assert_eq!(t.ui_value("nope").unwrap_err().message, "no value nope");
    // The plugin is still usable after an error it returned itself.
    assert_eq!(t.ui_action("scan").unwrap(), "scanned");
}

#[test]
fn a_plugin_without_ui_says_so() {
    for name in ["api10_tools_ok", "tools_ok"] {
        let (_d, t) = tools(name, None);
        assert!(!t.has_ui(), "{name}");
        let e = t.ui_action("scan").unwrap_err();
        assert_eq!(e.code, "plugin_invalid", "{name}");
        assert!(t.ui_value("x").is_err());
    }
}
