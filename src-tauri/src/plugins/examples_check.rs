//! Round 23 D1: every example under `plugins/examples/` loads and passes the install checks the
//! way Lumen runs them (Developer mode's *Load a folder…*). The two code examples need their
//! `plugin.wasm` (`node scripts/plugins/build-examples.mjs`, git-ignored); they are skipped with a
//! note when it isn't built. With `LUMEN_EXAMPLES_PACKED=<folder of .lumenplugin files>` each packed
//! file is also inspected (the install sheet's lines).

use std::path::{Path, PathBuf};

use photos_plugins::PluginState;

use super::Plugins;

fn examples() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("plugins").join("examples")
}

/// (folder, the Adds lines the sheet shows, the permissions it asks for)
const EXAMPLES: &[(&str, &[&str], &[&str])] = &[
    ("theme-nord", &["A theme: \"Nord\""], &[]),
    ("presets-film", &["3 Develop presets"], &[]),
    ("apps-gimp", &["An app: \"GIMP\""], &[]),
    ("prompts-film-look", &["A prompt for AI assistants: \"film-look\""], &[]),
    ("importer-folder-notes", &["An importer: \"Folder with notes\""], &["import.read-chosen"]),
    ("tools-warmth", &["1 tool for AI assistants"], &["photos.read", "develop.write-drafts"]),
];

#[test]
fn every_example_loads_in_developer_mode() {
    let app = tempfile::tempdir().unwrap();
    let plugins = Plugins::load(app.path());
    plugins.write().set_developer_mode(true).unwrap();
    let ctx = plugins.ctx();
    let mut loaded = 0;
    for (name, _, permissions) in EXAMPLES {
        let dir = examples().join(name);
        let code = matches!(*name, "importer-folder-notes" | "tools-warmth");
        if code && !dir.join("plugin.wasm").exists() {
            eprintln!("skipped {name}: plugin.wasm isn't built (node scripts/plugins/build-examples.mjs)");
            continue;
        }
        let info = plugins.write().load_dev(&dir, &ctx).unwrap_or_else(|e| panic!("{name}: {e:?}"));
        // The examples ask for Lumen >=26.2.0; a build still at 26.1.x loads them as "needs a newer Lumen" after every check passed.
        assert!(matches!(info.state, PluginState::Ready | PluginState::NeedsNewerLumen), "{name}: {:?}", info.state);
        assert!(info.dev && info.enabled, "{name}");
        assert_eq!(info.license, "MIT", "{name}");
        let asked: Vec<String> = info.permissions.iter().map(|p| p.id().to_string()).collect();
        assert_eq!(asked, *permissions, "{name}");
        loaded += 1;
    }
    assert!(loaded >= 4, "the four data examples always load");
}

#[test]
fn the_nord_theme_reads_and_keeps_the_contrast() {
    let dir = examples().join("theme-nord");
    let text = std::fs::read_to_string(dir.join("plugin.toml")).unwrap();
    let manifest = photos_plugins::manifest::parse(&text).unwrap();
    let themes = photos_plugins::theme::themes_of(&dir, &manifest).unwrap();
    assert_eq!(themes.len(), 1);
    let (meta, file) = &themes[0];
    assert_eq!((meta.id.as_str(), meta.default_scheme.as_str()), ("nord", "dark"));
    assert!(file.font.is_some() && !file.light.is_empty() && !file.dark.is_empty());
}

#[test]
fn packed_examples_show_their_sheet() {
    let Some(packed) = std::env::var_os("LUMEN_EXAMPLES_PACKED").map(PathBuf::from) else { return };
    let app = tempfile::tempdir().unwrap();
    let plugins = Plugins::load(app.path());
    let ctx = plugins.ctx();
    for (name, adds, _) in EXAMPLES {
        let file = packed.join(format!("{name}.lumenplugin"));
        // Signed with the test key: an author key, so no Developer mode is needed.
        let sheet = plugins.read().inspect(&file, &ctx).unwrap_or_else(|e| panic!("{name}: {e:?}"));
        assert_eq!(sheet.problems, Vec::<String>::new(), "{name}");
        assert_eq!(sheet.adds, *adds, "{name}");
    }
}
