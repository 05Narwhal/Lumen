//! The sandbox (Round 23 A2 Task 4): only granted interfaces are linked, WASI reaches no file
//! and no socket, settings arrive as JSON text and every log line carries the plugin id.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use photos_plugins::host::{ChosenRoot, Engine, HostCtx};
use photos_plugins::{Pick, PluginPermission};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("tests/wasm/{name}.wasm"))
}

fn engine(dir: &Path) -> Engine {
    Engine::new(&dir.join("cache"), Arc::new(|_, _| {})).unwrap()
}

type Lines = Arc<Mutex<Vec<String>>>;

fn ctx(dir: &Path, granted: Vec<PluginPermission>, settings: serde_json::Value) -> (HostCtx, Lines) {
    let lines: Lines = Arc::default();
    let sink = lines.clone();
    let ctx = HostCtx {
        plugin: "com.test.x".into(),
        granted,
        settings: settings.as_object().cloned().unwrap_or_default(),
        data_dir: dir.join("data"),
        log: Arc::new(move |l: &str| sink.lock().unwrap().push(l.to_string())),
        cancel: None,
        chosen: None,
        photos: None,
        drafts: None,
    };
    (ctx, lines)
}

#[test]
fn socket_fails() {
    let dir = tempfile::tempdir().unwrap();
    let (c, _) = ctx(dir.path(), vec![PluginPermission::ImportReadChosen], serde_json::json!({}));
    let mut inst = engine(dir.path()).importer_at(&fixture("socket"), c).unwrap();
    let err = inst.enumerate().unwrap_err();
    assert!(err.message.starts_with("socket:"), "{err}");
}

#[test]
fn file_fails() {
    let dir = tempfile::tempdir().unwrap();
    let secret = dir.path().join("secret.txt");
    std::fs::write(&secret, "the secret").unwrap();
    let path = secret.to_string_lossy().replace('\\', "/");
    let (c, _) = ctx(dir.path(), vec![PluginPermission::ImportReadChosen], serde_json::json!({ "path": path }));
    let mut inst = engine(dir.path()).importer_at(&fixture("file"), c).unwrap();
    let err = inst.enumerate().unwrap_err();
    assert!(err.message.starts_with("file:"), "{err}");
    assert!(!err.message.contains("the secret"), "{err}");
}

#[test]
fn only_granted_interfaces_linked() {
    let dir = tempfile::tempdir().unwrap();
    let (c, _) = ctx(dir.path(), vec![], serde_json::json!({}));
    let err = engine(dir.path()).importer_at(&fixture("importer_ok"), c).err().unwrap();
    assert_eq!(err.code, "plugin_permission", "{err}");
    assert!(err.message.contains("lumen:plugin/chosen"), "{err}");
}

#[test]
fn setting_reads_json() {
    let dir = tempfile::tempdir().unwrap();
    let (c, _) = ctx(
        dir.path(),
        vec![PluginPermission::ImportReadChosen],
        serde_json::json!({ "mode": "setting", "k": true }),
    );
    let mut inst = engine(dir.path()).importer_at(&fixture("importer_ok"), c).unwrap();
    let err = inst.enumerate().unwrap_err();
    assert_eq!(err.message, "k=Some(\"true\")");
}

#[test]
fn log_lines_reach_sink() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("root");
    std::fs::create_dir(&root).unwrap();
    std::fs::write(root.join("a.jpg"), b"JPEGDATA").unwrap();
    let (mut c, lines) = ctx(dir.path(), vec![PluginPermission::ImportReadChosen], serde_json::json!({}));
    c.chosen = Some(ChosenRoot::new(&root, Pick::Folder).unwrap());
    let mut inst = engine(dir.path()).importer_at(&fixture("importer_ok"), c).unwrap();
    let items = inst.enumerate().unwrap();
    assert_eq!(items.len(), 2);
    assert_eq!(items[0].path.as_deref(), Some("a.jpg"));
    assert_eq!(inst.read("b", 0, 4).unwrap(), b"JPEG");
    assert_eq!(inst.extras("a").unwrap().unwrap().albums, vec!["Trip".to_string()]);
    drop(inst);
    let lines = lines.lock().unwrap();
    assert!(lines.contains(&"plugin=com.test.x enumerating".to_string()), "{lines:?}");
    assert!(lines.contains(&"plugin=com.test.x hello from stdout".to_string()), "{lines:?}");
}
