//! `.lumentheme` packages (Round 26 goal 4): a zip with `theme.toml` (tokens, as plugin themes had),
//! an optional checked `theme.css`, fonts, images, an icon, a README and screenshots. No code, no
//! permissions, no signature (a store theme is still checked against the index's blake3). Installed
//! themes live in `themes/installed/<id>/` under app data, apart from plugins; *Try* unpacks to
//! `cache/theme-try/<hash>/` and nothing is installed until *Keep*.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use photos_core::ThemeId;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::package;
use crate::theme::{self, ThemeFile, Value};
use crate::theme_css;
use crate::PluginError;

pub const MAX_BYTES: u64 = 20 * 1024 * 1024;
/// The `data-plugin-theme` value is `<id>/theme`: one theme per package.
pub const THEME_SLOT: &str = "theme";
const PACKAGE_KEYS: &[&str] = &["version", "author", "description", "license", "lumen", "homepage", "listing"];

fn invalid(msg: impl Into<String>) -> PluginError {
    PluginError::invalid(msg)
}

/// The colours and sizes a miniature Lumen window is painted from (store tile, Appearance).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS, Default)]
#[ts(export)]
pub struct ThemeSwatch {
    pub bg: String,
    pub surface: String,
    pub sidebar: String,
    pub fg: String,
    pub fg2: String,
    pub accent: String,
    pub placeholder: String,
    pub radius: String,
    pub gap: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS, Default)]
#[ts(export)]
pub struct ThemeSwatches {
    pub light: ThemeSwatch,
    pub dark: ThemeSwatch,
}

/// An installed (or tried) theme, as Settings → Appearance and the store list it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct InstalledTheme {
    pub id: String,
    pub name: String,
    pub author: String,
    pub version: String,
    pub description: String,
    pub license: String,
    pub base: ThemeId,
    /// `light` or `dark`: applied the first time the theme is picked.
    pub default_scheme: String,
    /// The theme has a `theme.css` (it moves things, not only colours): the *Layout* badge.
    pub layout: bool,
    pub has_icon: bool,
    pub tokens: ThemeSwatches,
}

/// A theme package read and checked.
pub struct ThemePackage {
    pub root: PathBuf,
    pub file: ThemeFile,
    pub meta: InstalledTheme,
    pub files: BTreeMap<String, [u8; 32]>,
    /// Contrast failures (a problem outside Developer mode).
    pub contrast: Vec<String>,
}

struct Disk<'a>(&'a Path, &'a BTreeMap<String, [u8; 32]>);

impl theme_css::Files for Disk<'_> {
    fn head(&self, rel: &str) -> Option<(Vec<u8>, u64)> {
        if !self.1.contains_key(rel) {
            return None;
        }
        let path = self.0.join(rel);
        let size = fs::metadata(&path).ok()?.len();
        let mut head = vec![0u8; 8];
        let n = std::io::Read::read(&mut fs::File::open(&path).ok()?, &mut head).ok()?;
        head.truncate(n);
        Some((head, size))
    }
}

fn swatch(file: &ThemeFile, dark: bool) -> ThemeSwatch {
    let t = theme::effective_tokens(file, dark);
    let v = |k: &str| t.get(k).map(Value::css).unwrap_or_default();
    ThemeSwatch {
        bg: v("bg-base"),
        surface: v("bg-raised"),
        sidebar: v("bg-sidebar"),
        fg: v("fg-primary"),
        fg2: v("fg-muted"),
        accent: v("accent"),
        placeholder: v("bg-sunken"),
        radius: v("radius-md"),
        gap: v("tile-gap"),
    }
}

/// The swatches a theme file paints a miniature with (also what `add-theme.mjs` puts in the index).
pub fn swatches(file: &ThemeFile) -> ThemeSwatches {
    ThemeSwatches { light: swatch(file, false), dark: swatch(file, true) }
}

fn allowed_file(rel: &str) -> bool {
    let lower = rel.to_ascii_lowercase();
    let ext = |exts: &[&str]| exts.iter().any(|e| lower.ends_with(e));
    matches!(lower.as_str(), "theme.toml" | "theme.css" | "icon.png" | "readme.md" | "license" | "notices")
        || (lower.starts_with("fonts/") && ext(&[".woff2", ".txt"]))
        || ((lower.starts_with("images/") || lower.starts_with("screenshots/")) && ext(&[".png", ".jpg", ".jpeg", ".webp"]))
}

/// Opens a `.lumentheme` file into `into` and checks it.
pub fn open_file(file: &Path, into: &Path, lumen: &semver::Version) -> Result<ThemePackage, PluginError> {
    if fs::metadata(file).map(|m| m.len()).unwrap_or(0) > MAX_BYTES {
        return Err(invalid("A theme can be at most 20 MB"));
    }
    let files = package::unzip_files(file, into)?;
    finish(into.to_path_buf(), files, lumen)
}

/// Checks an unpacked theme folder in place (Developer mode, and every installed theme).
pub fn open_folder(dir: &Path, lumen: &semver::Version) -> Result<ThemePackage, PluginError> {
    let files = package::folder_files(dir)?;
    finish(dir.to_path_buf(), files, lumen)
}

fn finish(root: PathBuf, files: BTreeMap<String, [u8; 32]>, lumen: &semver::Version) -> Result<ThemePackage, PluginError> {
    if let Some(bad) = files.keys().find(|f| !allowed_file(f)) {
        return Err(invalid(format!("{bad}: a theme holds only theme.toml, theme.css, fonts/, images/, screenshots/, icon.png, README.md and LICENSE (themes run no code)")));
    }
    let read = |rel: &str| fs::read(root.join(rel)).map_err(|e| invalid(format!("{rel}: {e}")));
    if !files.contains_key("theme.toml") {
        return Err(invalid("The package has no theme.toml"));
    }
    let text = String::from_utf8(read("theme.toml")?).map_err(|_| invalid("theme.toml isn't UTF-8 text"))?;
    let table: toml::Table = text.parse().map_err(|e: toml::de::Error| invalid(format!("theme.toml: {}", e.message())))?;
    let mut file = theme::parse_theme_table(&table, PACKAGE_KEYS).map_err(|e| invalid(format!("theme.toml: {e}")))?;
    let text_of = |k: &str, need: bool| -> Result<String, PluginError> {
        match table.get(k) {
            Some(toml::Value::String(s)) if !s.trim().is_empty() => Ok(s.clone()),
            None if !need => Ok(String::new()),
            _ => Err(invalid(format!("theme.toml: {k} is required text"))),
        }
    };
    if !crate::manifest::valid_id(&file.id) {
        return Err(invalid(format!("theme.toml: id \"{}\" must be 3–64 of a–z, 0–9, \".\" and \"-\"", file.id)));
    }
    let version = text_of("version", true)?;
    semver::Version::parse(&version).map_err(|e| invalid(format!("theme.toml: version: {e}")))?;
    let range = text_of("lumen", true)?;
    let range = semver::VersionReq::parse(&range).map_err(|e| invalid(format!("theme.toml: lumen: {e}")))?;
    // A pre-release Lumen (26.2.0-alpha) still matches `>=26.2.0`.
    let plain = semver::Version::new(lumen.major, lumen.minor, lumen.patch);
    if !range.matches(lumen) && !range.matches(&plain) {
        return Err(PluginError::api(format!("This theme needs Lumen {range}")));
    }
    let author = text_of("author", true)?;
    let description = text_of("description", false)?;
    if description.chars().count() > 140 {
        return Err(invalid("theme.toml: description is at most 140 characters"));
    }
    if file.tagline.is_empty() {
        file.tagline = description.clone();
    }
    let license = text_of("license", true)?;
    crate::licences::check(&license, files.contains_key("LICENSE")).map_err(PluginError::licence)?;
    let mut screenshots = Vec::new();
    match table.get("listing") {
        None => {}
        Some(toml::Value::Table(l)) => {
            for (k, v) in l {
                match (k.as_str(), v) {
                    ("screenshots", toml::Value::Array(a)) => {
                        for s in a {
                            let toml::Value::String(s) = s else { return Err(invalid("[listing] screenshots are paths")) };
                            screenshots.push(s.clone());
                        }
                    }
                    ("tags", toml::Value::Array(a)) if a.iter().all(|t| matches!(t.as_str(), Some("dark" | "light" | "layout" | "colourful"))) => {}
                    _ => return Err(invalid(format!("theme.toml: [listing] {k} isn't known or isn't written as expected"))),
                }
            }
        }
        Some(_) => return Err(invalid("theme.toml: [listing] must be a table")),
    }

    if files.contains_key("icon.png") {
        package::check_icon(&read("icon.png")?)?;
    }
    if files.contains_key("README.md") {
        package::check_readme(&read("README.md")?)?;
    }
    for s in &screenshots {
        if !files.contains_key(s) {
            return Err(invalid(format!("theme.toml names the screenshot {s}, which isn't in the package")));
        }
        package::check_screenshot(s, &read(s)?)?;
    }
    if let Some(font) = &file.font {
        let disk = Disk(&root, &files);
        match theme_css::Files::head(&disk, &font.file) {
            None => return Err(invalid(format!("[font] {} isn't in the package", font.file))),
            Some((_, size)) if size > theme_css::MAX_FONT_BYTES => return Err(invalid(format!("[font] {} is over 512 KB", font.file))),
            Some((head, _)) if !head.starts_with(b"wOF2") => return Err(invalid(format!("[font] {} isn't a woff2 font", font.file))),
            Some(_) => {}
        }
    }
    let layout = files.contains_key("theme.css");
    let pkg = ThemePackage {
        meta: InstalledTheme {
            id: file.id.clone(),
            name: file.name.clone(),
            author,
            version,
            description,
            license,
            base: file.base,
            default_scheme: file.default_scheme.clone(),
            layout,
            has_icon: files.contains_key("icon.png"),
            tokens: swatches(&file),
        },
        contrast: theme::contrast_failures(&file),
        root,
        file,
        files,
    };
    if layout {
        pkg.layout_css("http://check.invalid").map_err(|r| invalid(r.to_string()))?;
    }
    Ok(pkg)
}

impl ThemePackage {
    /// `theme.css` checked and written back for `asset_base` (`<base>/theme-asset/<id>/<path>`).
    pub fn layout_css(&self, asset_base: &str) -> Result<String, theme_css::Refusal> {
        if !self.files.contains_key("theme.css") {
            return Ok(String::new());
        }
        let src = fs::read(self.root.join("theme.css")).unwrap_or_default();
        let Ok(src) = String::from_utf8(src) else {
            return Err(theme_css::Refusal { line: 1, rule: "syntax", message: "theme.css isn't UTF-8 text".into() });
        };
        let id = &self.meta.id;
        let v = &self.meta.version;
        let url = |p: &str| format!("{asset_base}/theme-asset/{id}/{p}?v={v}");
        let scope = format!("{id}/{THEME_SLOT}");
        Ok(theme_css::check(&src, &scope, id, &Disk(&self.root, &self.files), &url)?.css)
    }

    /// Why this theme can't be installed or tried, if it can't (contrast fails outside Developer mode).
    pub fn problem(&self, dev: bool) -> Option<String> {
        (!dev && !self.contrast.is_empty()).then(|| format!("Text would be hard to read: {}", self.contrast.join("; ")))
    }
}

/// The token CSS and the checked layout CSS for `themes` (installed, then a tried one last so it wins).
pub fn css(themes: &[ThemePackage], asset_base: &str) -> String {
    let tokens: Vec<(String, String, ThemeFile)> = themes
        .iter()
        .map(|p| {
            let mut f = p.file.clone();
            f.id = THEME_SLOT.into();
            (p.meta.id.clone(), p.meta.version.clone(), f)
        })
        .collect();
    let mut out = theme::css(&tokens, asset_base);
    for p in themes {
        match p.layout_css(asset_base) {
            Ok(c) => out.push_str(&c),
            Err(e) => tracing::warn!(theme = %p.meta.id, error = %e, "an installed theme's CSS stopped passing the check"),
        }
    }
    out
}

/// The theme store under app data: `themes/installed/<id>/` and the *Try* cache.
pub struct ThemeStore {
    root: PathBuf,
    try_root: PathBuf,
}

impl ThemeStore {
    /// `app` is Lumen's app data folder.
    pub fn new(app: &Path) -> Self {
        ThemeStore { root: app.join("themes"), try_root: app.join("cache").join("theme-try") }
    }

    fn installed_dir(&self) -> PathBuf {
        self.root.join("installed")
    }

    /// Every installed theme that still reads (one that doesn't is logged and left out).
    pub fn list(&self, lumen: &semver::Version) -> Vec<ThemePackage> {
        let Ok(rd) = fs::read_dir(self.installed_dir()) else { return Vec::new() };
        let mut out: Vec<ThemePackage> = rd
            .filter_map(Result::ok)
            .filter(|e| e.path().is_dir())
            .filter_map(|e| match open_folder(&e.path(), lumen) {
                Ok(p) if e.file_name().to_str() == Some(p.meta.id.as_str()) => Some(p),
                Ok(_) => None,
                Err(err) => {
                    tracing::warn!(dir = %e.path().display(), error = %err.message, "an installed theme couldn't be read");
                    None
                }
            })
            .collect();
        out.sort_by_key(|p| p.meta.name.to_lowercase());
        out
    }

    /// The installed theme `id`, if any.
    pub fn get(&self, id: &str, lumen: &semver::Version) -> Option<ThemePackage> {
        if !crate::manifest::valid_id(id) {
            return None;
        }
        open_folder(&self.installed_dir().join(id), lumen).ok().filter(|p| p.meta.id == id)
    }

    /// Installs (or updates) a `.lumentheme`. `dev` lets contrast failures through.
    pub fn install(&self, file: &Path, lumen: &semver::Version, dev: bool) -> Result<InstalledTheme, PluginError> {
        let stage = self.root.join("staging").join(format!("{}", std::process::id()) + &unique());
        let _ = fs::remove_dir_all(&stage);
        let result = open_file(file, &stage, lumen).and_then(|p| self.place(p, &stage, dev));
        let _ = fs::remove_dir_all(&stage);
        result
    }

    /// Reads and checks a `.lumentheme` without installing it (the install sheet): its meta and
    /// the reason it can't be installed, if any. The unpacked copy is removed again.
    pub fn inspect(&self, file: &Path, lumen: &semver::Version, dev: bool) -> Result<(InstalledTheme, Option<String>), PluginError> {
        let stage = self.root.join("staging").join(format!("inspect-{}", unique()));
        let _ = fs::remove_dir_all(&stage);
        let result = open_file(file, &stage, lumen).map(|p| {
            let problem = p.problem(dev);
            (p.meta, problem)
        });
        let _ = fs::remove_dir_all(&stage);
        result
    }

    /// Installs a theme already unpacked and checked (*Keep* after *Try*).
    pub fn install_folder(&self, dir: &Path, lumen: &semver::Version, dev: bool) -> Result<InstalledTheme, PluginError> {
        let stage = self.root.join("staging").join(unique());
        copy_dir(dir, &stage).map_err(|e| invalid(format!("Couldn't copy the theme: {e}")))?;
        let result = open_folder(&stage, lumen).and_then(|p| self.place(p, &stage, dev));
        let _ = fs::remove_dir_all(&stage);
        result
    }

    fn place(&self, p: ThemePackage, stage: &Path, dev: bool) -> Result<InstalledTheme, PluginError> {
        if let Some(problem) = p.problem(dev) {
            return Err(invalid(problem));
        }
        let dest = self.installed_dir().join(&p.meta.id);
        let old = self.root.join("staging").join(format!("old-{}", unique()));
        fs::create_dir_all(self.installed_dir()).map_err(|e| invalid(format!("Couldn't install: {e}")))?;
        if dest.exists() {
            fs::rename(&dest, &old).map_err(|e| invalid(format!("Couldn't replace the old version: {e}")))?;
        }
        if let Err(e) = fs::rename(stage, &dest) {
            let _ = fs::rename(&old, &dest);
            return Err(invalid(format!("Couldn't install: {e}")));
        }
        let _ = fs::remove_dir_all(&old);
        Ok(p.meta)
    }

    /// Removes an installed theme (app data only).
    pub fn remove(&self, id: &str) -> Result<(), PluginError> {
        if !crate::manifest::valid_id(id) {
            return Err(invalid("Not a theme id"));
        }
        let dir = self.installed_dir().join(id);
        if !dir.exists() {
            return Err(invalid(format!("The theme {id} isn't installed")));
        }
        fs::remove_dir_all(&dir).map_err(|e| invalid(format!("Couldn't remove the theme: {e}")))
    }

    /// Unpacks and checks a downloaded `.lumentheme` for *Try* (`hash` is its blake3, already
    /// checked against the index). Nothing is installed.
    pub fn try_open(&self, file: &Path, hash: &str, lumen: &semver::Version) -> Result<ThemePackage, PluginError> {
        if hash.len() != 64 || !hash.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(invalid("Not a package hash"));
        }
        self.try_end();
        let dir = self.try_root.join(hash.to_ascii_lowercase());
        let p = open_file(file, &dir, lumen)?;
        if let Some(problem) = p.problem(false) {
            self.try_end();
            return Err(invalid(problem));
        }
        Ok(p)
    }

    /// The theme being tried, if any.
    pub fn tried(&self, lumen: &semver::Version) -> Option<ThemePackage> {
        let dir = fs::read_dir(&self.try_root).ok()?.filter_map(Result::ok).find(|e| e.path().is_dir())?;
        open_folder(&dir.path(), lumen).ok()
    }

    /// Ends *Try*: the cache folder goes.
    pub fn try_end(&self) {
        let _ = fs::remove_dir_all(&self.try_root);
    }

    /// Where a theme's file is, installed or tried, for `photos://theme-asset/<id>/<path>`.
    pub fn asset_path(&self, id: &str, rel: &str, lumen: &semver::Version) -> Option<PathBuf> {
        if !crate::manifest::valid_id(id) || !theme::safe_rel_path(rel) {
            return None;
        }
        let tried = self.tried(lumen).filter(|p| p.meta.id == id);
        let pkg = tried.or_else(|| self.get(id, lumen))?;
        pkg.files.contains_key(rel).then(|| pkg.root.join(rel))
    }
}

fn unique() -> String {
    let n = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    format!("{n:x}")
}

fn copy_dir(from: &Path, to: &Path) -> std::io::Result<()> {
    fs::create_dir_all(to)?;
    for e in fs::read_dir(from)? {
        let e = e?;
        let t = e.file_type()?;
        if t.is_dir() {
            copy_dir(&e.path(), &to.join(e.file_name()))?;
        } else if t.is_file() {
            fs::copy(e.path(), to.join(e.file_name()))?;
        }
    }
    Ok(())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::package::fixture::pack_to;

    pub const TOML: &str = "id = \"studio.test.dusk\"\nname = \"Dusk\"\nversion = \"1.0.0\"\nauthor = \"Test\"\n\
        description = \"Violet dusk.\"\nlicense = \"MIT\"\nlumen = \">=26.2.0\"\nbase = \"darkroom\"\ndefault_scheme = \"dark\"\n\
        [light]\naccent = \"oklch(50% 0.15 300)\"\n[dark]\nbg-base = \"oklch(16% 0.02 300)\"\n[listing]\ntags = [\"dark\", \"layout\"]\n";

    fn lumen() -> semver::Version {
        "26.2.0-alpha".parse().unwrap()
    }

    fn make(dir: &Path, entries: &[(&str, &[u8])]) -> PathBuf {
        let f = dir.join("t.lumentheme");
        pack_to(&f, entries);
        f
    }

    #[test]
    fn installs_lists_serves_and_removes() {
        let t = tempfile::tempdir().unwrap();
        let layout = b"[data-part=\"shell\"] { grid-template-areas: \"titlebar\" \"sidebar\" \"main\"; }\n[data-part=\"panel\"] { background: url(images/g.png) }";
        let f = make(t.path(), &[("theme.toml", TOML.as_bytes()), ("theme.css", layout), ("images/g.png", b"\x89PNG\r\n\x1a\nxx")]);
        let store = ThemeStore::new(&t.path().join("app"));
        let meta = store.install(&f, &lumen(), false).unwrap();
        assert_eq!((meta.id.as_str(), meta.layout, meta.base), ("studio.test.dusk", true, ThemeId::Darkroom));
        assert!(meta.tokens.light.accent.starts_with("oklch(50%"), "{:?}", meta.tokens);
        let list = store.list(&lumen());
        assert_eq!(list.len(), 1);
        let out = css(&list, "http://photos.localhost");
        assert!(out.contains("[data-plugin-theme=\"studio.test.dusk/theme\"]"), "{out}");
        assert!(out.contains("http://photos.localhost/theme-asset/studio.test.dusk/images/g.png?v=1.0.0"), "{out}");
        assert!(store.asset_path("studio.test.dusk", "images/g.png", &lumen()).is_some());
        assert!(store.asset_path("studio.test.dusk", "../x", &lumen()).is_none());
        assert!(store.asset_path("studio.test.dusk", "theme.toml", &lumen()).is_some());
        store.remove("studio.test.dusk").unwrap();
        assert!(store.list(&lumen()).is_empty());
        assert!(!t.path().join("app/themes/staging").read_dir().map(|mut d| d.next().is_some()).unwrap_or(false));
    }

    #[test]
    fn refusals() {
        let t = tempfile::tempdir().unwrap();
        let store = ThemeStore::new(&t.path().join("app"));
        let toml = |from: &str, to: &str| TOML.replace(from, to).into_bytes();
        let t1 = TOML.as_bytes().to_vec();
        type Case<'a> = (Vec<(&'a str, Vec<u8>)>, &'a str);
        let cases: Vec<Case> = vec![
            (vec![("theme.toml", t1.clone()), ("plugin.wasm", b"\0asm".to_vec())], "run no code"),
            (vec![("theme.toml", t1.clone()), ("theme.css", b"[data-part=\"dialog\"] { color: red }".to_vec())], "line 1"),
            (vec![("theme.css", Vec::new())], "no theme.toml"),
            (vec![("theme.toml", toml(">=26.2.0", ">=27.0.0"))], "needs Lumen"),
            (vec![("theme.toml", toml("license = \"MIT\"\n", ""))], "license"),
            (vec![("theme.toml", toml("[light]", "permissions = []\n[light]"))], "unknown key permissions"),
            (vec![("theme.toml", toml("[dark]", "[dark]\nfg-primary = \"oklch(18% 0.02 300)\""))], "hard to read"),
            (vec![("theme.toml", format!("{TOML}[font]\nfile = \"fonts/a.woff2\"\n").into_bytes()), ("fonts/a.woff2", b"PK..".to_vec())], "woff2"),
        ];
        for (entries, want) in &cases {
            let entries: Vec<(&str, &[u8])> = entries.iter().map(|(n, b)| (*n, b.as_slice())).collect();
            let f = make(t.path(), &entries);
            let e = store.install(&f, &lumen(), false).unwrap_err();
            assert!(e.message.contains(want), "{want}: {}", e.message);
            fs::remove_file(f).unwrap();
        }
        assert!(store.list(&lumen()).is_empty());
    }

    #[test]
    fn try_never_installs() {
        let t = tempfile::tempdir().unwrap();
        let f = make(t.path(), &[("theme.toml", TOML.as_bytes())]);
        let store = ThemeStore::new(&t.path().join("app"));
        let hash = blake3::hash(&fs::read(&f).unwrap()).to_hex().to_string();
        let p = store.try_open(&f, &hash, &lumen()).unwrap();
        assert_eq!(store.tried(&lumen()).unwrap().meta.id, p.meta.id);
        assert!(store.list(&lumen()).is_empty());
        let kept = store.install_folder(&p.root, &lumen(), false).unwrap();
        assert_eq!(kept.id, "studio.test.dusk");
        store.try_end();
        assert!(store.tried(&lumen()).is_none());
        assert_eq!(store.list(&lumen()).len(), 1);
    }

    /// What `plugins/store/add-theme.mjs` fills a swatch from when theme.toml doesn't set it:
    /// each base's swatches, as `swatches` computes them.
    #[test]
    fn base_swatches_golden() {
        let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../plugins/store/base-swatches.json");
        let mut want = serde_json::Map::new();
        for base in ["gallery", "darkroom", "paper", "glass"] {
            let f = theme::parse_theme(&format!("id = \"x\"
name = \"x\"
base = \"{base}\"
")).unwrap();
            want.insert(base.into(), serde_json::to_value(swatches(&f)).unwrap());
        }
        let want = serde_json::Value::Object(want);
        if std::env::var("LUMEN_REGEN_GOLDEN").is_ok() {
            fs::write(&path, serde_json::to_string_pretty(&want).unwrap() + "
").unwrap();
        }
        let have: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&path).expect("run with LUMEN_REGEN_GOLDEN=1")).unwrap();
        assert_eq!(have, want, "base-swatches.json is stale: run with LUMEN_REGEN_GOLDEN=1");
    }

    /// The example in `themes/examples/nord/` (the old Nord plugin theme) passes install's checks.
    #[test]
    fn nord_example_is_a_valid_theme() {
        let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../themes/examples/nord");
        let p = open_folder(&dir, &lumen()).unwrap();
        assert_eq!(p.problem(false), None);
        // `#88c0d0`, as Rust writes colours.
        assert!(p.meta.tokens.dark.accent.starts_with("oklch(77.46"), "{}", p.meta.tokens.dark.accent);
    }
}
