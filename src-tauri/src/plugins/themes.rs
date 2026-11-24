//! Installed `.lumentheme` themes in the app (Round 26 goal 4): the list, the CSS the page fills
//! `<style id="plugin-themes">` with, install / remove, and the files `photos://theme-asset/` serves.
//! Rust writes every byte of that CSS. Themes are apart from plugins (`themes/installed/`).

use std::path::{Path, PathBuf};

use photos_plugins::theme_pkg::{self, InstalledTheme, ThemePackage, ThemeStore, THEME_SLOT};
use photos_plugins::PluginError;

use super::state::Served;

/// Where the page fetches `theme-asset/<id>/<path>` from: the `photos://` scheme's per-OS form
/// (WebView2 only serves custom schemes as `http://photos.localhost/…`).
pub fn asset_base() -> &'static str {
    if cfg!(windows) {
        "http://photos.localhost"
    } else {
        "photos://localhost"
    }
}

pub struct Themes {
    store: ThemeStore,
    lumen: semver::Version,
}

impl Themes {
    pub fn load(app_dir: &Path) -> Themes {
        let store = ThemeStore::new(app_dir);
        // A *Try* never outlives a launch.
        store.try_end();
        Themes { store, lumen: env!("CARGO_PKG_VERSION").parse().expect("Lumen's version is semver") }
    }

    /// The installed themes, then the one being tried (so its rules come last and win).
    fn packages(&self) -> Vec<ThemePackage> {
        let mut out = self.store.list(&self.lumen);
        if let Some(t) = self.store.tried(&self.lumen) {
            out.retain(|p| p.meta.id != t.meta.id);
            out.push(t);
        }
        out
    }

    pub fn list(&self) -> Vec<InstalledTheme> {
        self.store.list(&self.lumen).into_iter().map(|p| p.meta).collect()
    }

    pub fn css(&self) -> String {
        theme_pkg::css(&self.packages(), asset_base())
    }

    pub fn install(&self, file: &Path, dev: bool) -> Result<InstalledTheme, PluginError> {
        self.store.install(file, &self.lumen, dev)
    }

    /// A package's meta and why it can't be installed, without installing it.
    pub fn inspect(&self, file: &Path, dev: bool) -> Result<(InstalledTheme, Option<String>), PluginError> {
        self.store.inspect(file, &self.lumen, dev)
    }

    /// The theme being tried, if any (the page needs its base while it is shown).
    pub fn tried(&self) -> Option<InstalledTheme> {
        self.store.tried(&self.lumen).map(|p| p.meta)
    }

    pub fn remove(&self, id: &str) -> Result<(), PluginError> {
        self.store.remove(id)
    }

    /// *Try*: a downloaded, hash-checked package shown without installing.
    /// The file's own blake3 must be `hash` (the index's), whatever handed it over.
    pub fn try_file(&self, file: &Path, hash: &str) -> Result<InstalledTheme, PluginError> {
        let bytes = std::fs::read(file).map_err(|e| PluginError::invalid(format!("The theme couldn't be read: {e}")))?;
        if !blake3::hash(&bytes).to_hex().eq_ignore_ascii_case(hash) {
            return Err(PluginError::invalid("The theme doesn't match the store's checksum"));
        }
        self.store.try_open(file, hash, &self.lumen).map(|p| p.meta)
    }

    pub fn try_end(&self) {
        self.store.try_end();
    }

    /// *Keep* after *Try*: installs the tried theme.
    pub fn keep(&self) -> Result<InstalledTheme, PluginError> {
        let tried = self.store.tried(&self.lumen).ok_or_else(|| PluginError::invalid("No theme is being tried"))?;
        let meta = self.store.install_folder(&tried.root, &self.lumen, false)?;
        self.store.try_end();
        Ok(meta)
    }

    pub fn is_installed(&self, id: &str) -> bool {
        self.store.get(id, &self.lumen).is_some()
    }

    /// Installed, or the one being tried: what `theme_apply` may pick.
    pub fn is_available(&self, id: &str) -> bool {
        self.is_installed(id) || self.store.tried(&self.lumen).is_some_and(|t| t.meta.id == id)
    }

    /// The file behind `theme-asset/<id>/<path>` (percent-encoded path), installed or tried.
    pub fn serve_asset(&self, id: &str, encoded: &str) -> Option<Served> {
        let rel = super::state::percent_decode(encoded)?;
        let path: PathBuf = self.store.asset_path(id, &rel, &self.lumen)?;
        Some(Served { path, dev: false })
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    const TOML: &str = "id = \"studio.test.dusk\"\nname = \"Dusk\"\nversion = \"1.0.0\"\nauthor = \"T\"\ndescription = \"d\"\n\
        license = \"MIT\"\nlumen = \">=26.0.0\"\nbase = \"darkroom\"\n[dark]\nbg-base = \"oklch(16% 0.004 60)\"\n";

    /// A theme as the store keeps it once installed: unpacked under `themes/installed/<id>/`.
    fn installed(app: &Path) {
        let dir = app.join("themes/installed/studio.test.dusk");
        fs::create_dir_all(dir.join("images")).unwrap();
        fs::write(dir.join("theme.toml"), TOML).unwrap();
        fs::write(dir.join("images/g.png"), b"PNG").unwrap();
    }

    #[test]
    fn installed_themes_reach_the_picker_the_css_and_the_route() {
        let t = tempfile::tempdir().unwrap();
        let themes = Themes::load(t.path());
        installed(t.path());
        let listed = themes.list();
        assert_eq!(listed[0].id, "studio.test.dusk");
        assert!(themes.css().contains("[data-plugin-theme=\"studio.test.dusk/theme\"]"));
        assert!(themes.serve_asset("studio.test.dusk", "images/g.png").is_some());
        assert!(themes.serve_asset("studio.test.dusk", "..%2Ftheme.toml").is_none());
        themes.remove("studio.test.dusk").unwrap();
        assert!(themes.list().is_empty() && themes.css().is_empty());
    }

    #[test]
    fn try_refuses_a_file_whose_blake3_isnt_the_index_s() {
        let t = tempfile::tempdir().unwrap();
        let themes = Themes::load(t.path());
        let file = t.path().join("dusk.lumentheme");
        fs::write(&file, b"not the indexed bytes").unwrap();
        let e = themes.try_file(&file, &"0".repeat(64)).unwrap_err();
        assert!(e.message.contains("checksum"), "{}", e.message);
        assert!(!t.path().join("cache/theme-try").exists() && !themes.is_available("studio.test.dusk"));
    }
}
