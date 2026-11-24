//! The first-party plugins under `plugins/first-party/` load and pass the install checks the way
//! Lumen runs them (Developer mode's *Load a folder…*).

use std::path::Path;

use photos_plugins::PluginState;

use super::Plugins;

/// Round 24 D4: the first-party Disk Atlas plugin (declarative, no code) loads and passes the checks.
#[test]
fn the_disk_atlas_plugin_loads_in_developer_mode() {
    let app = tempfile::tempdir().unwrap();
    let plugins = Plugins::load(app.path());
    plugins.write().set_developer_mode(true).unwrap();
    let ctx = plugins.ctx();
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("plugins").join("first-party").join("disk-atlas");
    let info = plugins.write().load_dev(&dir, &ctx).unwrap_or_else(|e| panic!("{e:?}"));
    assert!(matches!(info.state, PluginState::Ready | PluginState::NeedsNewerLumen), "{:?}", info.state);
    assert_eq!(info.id, "jl.disk-atlas");
    assert!(info.permissions.is_empty());
}
