//! Plugin themes in the app: the enabled plugins' themes as a list and as the CSS the page fills
//! `<style id="plugin-themes">` with (Round 23 lane B1). Rust writes every byte of that CSS.

use photos_plugins::theme::{self, PluginThemeMeta};

use super::Plugins;

/// Where the page fetches `plugin-asset/<id>/<path>` from: the `photos://` scheme's per-OS form
/// (WebView2 only serves custom schemes as `http://photos.localhost/…`).
pub fn asset_base() -> &'static str {
    if cfg!(windows) {
        "http://photos.localhost"
    } else {
        "photos://localhost"
    }
}

/// Every theme of every enabled, ready plugin. A plugin whose theme files no longer read is
/// left out and logged, never an error for the page.
fn enabled_themes(plugins: &Plugins) -> Vec<(PluginThemeMeta, String, theme::ThemeFile)> {
    let store = plugins.read();
    let mut out = Vec::new();
    for inst in store.enabled().into_iter().filter(|i| !i.manifest.themes.is_empty()) {
        match theme::themes_of(&inst.dir, &inst.manifest) {
            Ok(themes) => {
                let version = inst.manifest.version.to_string();
                out.extend(themes.into_iter().map(|(m, f)| (m, version.clone(), f)));
            }
            Err(e) => tracing::warn!(plugin = %inst.manifest.id, error = %e, "a plugin's themes couldn't be read"),
        }
    }
    out
}

/// The enabled plugin themes, for the theme picker.
pub fn list(plugins: &Plugins) -> Vec<PluginThemeMeta> {
    enabled_themes(plugins).into_iter().map(|(meta, _, _)| meta).collect()
}

/// The CSS for the enabled plugin themes.
pub fn css(plugins: &Plugins) -> String {
    let themes: Vec<_> = enabled_themes(plugins).into_iter().map(|(meta, v, file)| (meta.plugin, v, file)).collect();
    theme::css(&themes, asset_base())
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;
    use std::time::Instant;

    use super::*;

    const THEME: &str = "id = \"nord\"\nname = \"Nord\"\ntagline = \"Arctic blues.\"\nbase = \"darkroom\"\ndefault_scheme = \"dark\"\n\
        [light]\nbg-base = \"oklch(96% 0.01 240)\"\nradius-md = \"10px\"\n[dark]\nbg-base = \"oklch(16% 0.004 60)\"\nshadow-md = \"0 6px 16px #00000080\"\n";

    /// An app folder with installed theme plugins `(id, enabled)`, written as the store keeps them.
    fn app_with(plugins: &[(&str, bool)]) -> tempfile::TempDir {
        let t = tempfile::tempdir().unwrap();
        let dir = t.path().join("plugins");
        let mut state = serde_json::Map::new();
        for (id, enabled) in plugins {
            let folder = dir.join("installed").join(id);
            fs::create_dir_all(folder.join("themes")).unwrap();
            fs::write(
                folder.join("plugin.toml"),
                format!(
                    "id = \"{id}\"\nname = \"P\"\nversion = \"1.0.0\"\nauthor = \"T\"\ndescription = \"d\"\nlicense = \"MIT\"\nlumen = \">=26.0.0\"\napi = 1\n\n[[themes]]\nfile = \"themes/nord.toml\"\n"
                ),
            )
            .unwrap();
            fs::write(folder.join("themes/nord.toml"), THEME).unwrap();
            state.insert(
                (*id).to_string(),
                serde_json::json!({
                    "version": "1.0.0", "enabled": enabled, "granted": [], "key": "A1B2C3D4E5F60708",
                    "trust": {"type": "author", "name": "T", "fingerprint": "A1B2-C3D4"},
                    "installedAt": "2026-11-14T00:00:00Z", "dev": false, "settings": {}
                }),
            );
        }
        fs::write(dir.join("plugins.json"), serde_json::to_vec(&serde_json::json!({ "plugins": state })).unwrap())
            .unwrap();
        t
    }

    #[test]
    fn themes_from_enabled_only() {
        let t = app_with(&[("com.test.on", true), ("com.test.off", false)]);
        let plugins = Plugins::load(t.path());
        let list = list(&plugins);
        assert_eq!(list.len(), 1, "{list:?}");
        assert_eq!((list[0].plugin.as_str(), list[0].id.as_str()), ("com.test.on", "nord"));
        let css = css(&plugins);
        assert!(css.contains("com.test.on/nord"), "{css}");
        assert!(!css.contains("com.test.off"), "{css}");
    }

    #[test]
    fn a_theme_file_that_stopped_reading_is_left_out() {
        let t = app_with(&[("com.test.on", true), ("com.test.bad", true)]);
        fs::write(t.path().join("plugins/installed/com.test.bad/themes/nord.toml"), "id = 3").unwrap();
        let plugins = Plugins::load(t.path());
        let ids: Vec<String> = list(&plugins).into_iter().map(|m| m.plugin).collect();
        assert_eq!(ids, vec!["com.test.on".to_string()]);
    }

    #[test]
    fn asset_base_is_the_os_form() {
        let base = asset_base();
        assert!(base == "http://photos.localhost" || base == "photos://localhost");
        assert_eq!(base.starts_with("http://"), cfg!(windows));
    }

    #[test]
    fn theme_boot_budget() {
        let ids: Vec<String> = (0..10).map(|n| format!("com.test.p{n}")).collect();
        let t = app_with(&ids.iter().map(|i| (i.as_str(), true)).collect::<Vec<_>>());
        let plugins = Plugins::load(Path::new(t.path()));
        let mut times: Vec<_> = (0..5)
            .map(|_| {
                let start = Instant::now();
                let css = css(&plugins);
                let took = start.elapsed();
                assert_eq!(css.matches("[data-plugin-theme=").count(), 20, "light and dark rule per theme");
                took
            })
            .collect();
        times.sort();
        assert!(times[2].as_millis() < 10, "median {:?} of {times:?}", times[2]);
    }
}
