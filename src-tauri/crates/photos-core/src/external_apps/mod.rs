//! External apps (Round 22 lane C, spec §7): which photo editors, RAW developers and video editors
//! are installed, found without launching anything. The pure parts live here so they are tested
//! against fake registries and folder trees; the binary's `external_apps` module supplies the real
//! lookups (registry, bundles, `.desktop` files), the icons and the launcher.

mod custom;
mod detect;
mod plugin_source;
mod profile;

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use ts_rs::TS;

pub use custom::{custom_app_for, custom_id, Custom};
pub use detect::{exec_launches, exec_program, natural_cmp, resolve_profile, Os};
pub use plugin_source::{is_shell_host, parse_plugin_apps, AppProfileOwned, PluginApps, WinRuleOwned, SHELL_HOSTS};
pub use profile::BuiltIn;

/// The name the built-in table gives an app id (a custom app has none here).
pub fn app_name(id: &str) -> Option<&'static str> {
    profile::TABLE.iter().find(|p| p.id == id).map(|p| p.name)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum AppKind {
    /// Saves over the file it is given (Photoshop).
    Editor,
    /// Keeps its own catalogue or sidecars (Lightroom, Capture One).
    Developer,
    /// Never writes its sources (Resolve).
    Video,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum Writes {
    InPlace,
    Sidecar,
    Never,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum Media {
    Photo,
    Raw,
    Video,
}

/// What a person chose for one app in Settings → External apps (absent = `Automatic`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum SendMode {
    #[default]
    Automatic,
    Rendered,
    OriginalFile,
}

/// The app each kind of media opens in on *Edit in …* (`externalApps.defaults`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DefaultApps {
    #[serde(default)]
    pub photo: Option<String>,
    #[serde(default)]
    pub raw: Option<String>,
    #[serde(default)]
    pub video: Option<String>,
}

/// An app the person picked with *Choose another app…* (`externalApps.custom`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct CustomApp {
    pub id: String,
    pub name: String,
    pub path: String,
}

/// What the page sees of an app: no paths, an icon URL served over `photos://`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ExternalApp {
    pub id: String,
    pub name: String,
    pub kind: AppKind,
    pub accepts: Vec<Media>,
    pub icon_url: String,
    pub custom: bool,
    pub send: SendMode,
    /// What the app does with a file it is given (Settings says whether a developer gets a copy).
    pub writes: Writes,
    /// The plugin that added it: the row reads *added by <plugin>*.
    pub added_by: Option<String>,
}

/// One row of the built-in table. Identifiers per OS; an empty list means "not on that OS".
///
/// The built-in table's rows borrow `'static` text; a plugin's rows (`plugin_source`) are built at
/// run time and lend theirs for one detection (`AppProfileOwned::with_profile`).
pub struct AppProfile<'a> {
    pub id: &'a str,
    pub name: &'a str,
    pub kind: AppKind,
    pub accepts: &'a [Media],
    pub reads_raw: bool,
    pub writes: Writes,
    pub win: &'a [WinRule<'a>],
    /// Bundle identifiers.
    pub mac: &'a [&'a str],
    /// `.desktop` ids (without the suffix).
    pub linux: &'a [&'a str],
}

pub enum WinRule<'a> {
    /// `HKLM|HKCU\…\App Paths\<exe>`.
    AppPath(&'a str),
    /// An Add/Remove Programs entry whose name starts so; the app is `exe` (relative) inside its
    /// install location, and only counts if that file exists.
    Uninstall { name_starts: &'a str, exe: &'a str },
    /// A path pattern with `*` and `?` in its segments and `%ProgramFiles%`-style variables; the
    /// match that sorts last (newest version) wins.
    Glob(&'a str),
    /// A Microsoft Store app by package family name (`Microsoft.Paint_8wekyb3d8bbwe`). Its folder
    /// is locked, so the target is `shell:AppsFolder\<family>!<app id>`, started through the shell.
    Store { family: &'a str },
}

pub struct DesktopEntry {
    pub exec: String,
    pub icon: Option<String>,
    pub path: PathBuf,
}

/// Everything detection asks the machine. The binary implements it per OS; tests use maps.
pub trait Lookup {
    /// The executable `App Paths\<exe>` names, if it exists on disk.
    fn app_path(&self, exe: &str) -> Option<PathBuf>;
    /// Add/Remove Programs: (DisplayName, InstallLocation).
    fn uninstall(&self) -> Vec<(String, Option<PathBuf>)>;
    /// Files matching a pattern (variables expanded by the lookup).
    fn glob(&self, pattern: &str) -> Vec<PathBuf>;
    /// The bundle with this identifier.
    fn bundle(&self, id: &str) -> Option<PathBuf>;
    fn desktop(&self, id: &str) -> Option<DesktopEntry>;
    /// The `shell:AppsFolder\<family>!<app id>` target of an installed Store app (newest version).
    fn store_app(&self, _family: &str) -> Option<PathBuf> {
        None
    }
    fn exists(&self, path: &Path) -> bool;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstalledApp {
    pub id: String,
    pub name: String,
    pub kind: AppKind,
    pub accepts: Vec<Media>,
    pub reads_raw: bool,
    pub writes: Writes,
    /// The executable, the `.app` bundle, the `.desktop` file or a Store app's `shell:AppsFolder\…`.
    pub target: PathBuf,
    /// A `.desktop` file's `Exec` line.
    pub exec: Option<String>,
    pub custom: bool,
    /// The plugin that added it (*added by <plugin>*), `None` for the built-in table and the
    /// person's own apps.
    pub added_by: Option<String>,
}

/// Where apps come from: the built-in table and the person's own list now, plugins later.
pub trait AppSource: Send + Sync {
    fn apps(&self, lookup: &dyn Lookup) -> Vec<InstalledApp>;
}

pub trait Launcher {
    fn launch(&self, app: &InstalledApp, files: &[PathBuf]) -> Result<(), String>;
}

/// The key a copy made for an app is reused by (spec §8 *Used again*): the source's content hash,
/// the recipe it was rendered with, what was sent and the bit depth. Hex blake3.
pub fn made_key_hex(source_hash: &str, recipe_json: &str, sent: &str, bits: u8) -> String {
    let mut h = blake3::Hasher::new();
    for part in [source_hash, recipe_json, sent] {
        h.update(&(part.len() as u64).to_le_bytes());
        h.update(part.as_bytes());
    }
    h.update(&[bits]);
    h.finalize().to_hex().to_string()
}

/// Every source's apps, ids unique (the first source wins), by name.
pub fn installed(sources: &[&dyn AppSource], lookup: &dyn Lookup) -> Vec<InstalledApp> {
    let mut all: Vec<InstalledApp> = Vec::new();
    for source in sources {
        for app in source.apps(lookup) {
            if !all.iter().any(|a| a.id == app.id) {
                all.push(app);
            }
        }
    }
    all.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()).then_with(|| a.id.cmp(&b.id)));
    all
}

#[cfg(test)]
pub(crate) mod fake {
    use std::collections::{BTreeMap, BTreeSet};

    use super::*;

    /// A registry and folder tree held in maps.
    #[derive(Default)]
    pub struct FakeLookup {
        pub app_paths: BTreeMap<String, PathBuf>,
        pub uninstall: Vec<(String, Option<PathBuf>)>,
        pub globs: BTreeMap<String, Vec<PathBuf>>,
        pub bundles: BTreeMap<String, PathBuf>,
        pub desktops: BTreeMap<String, PathBuf>,
        pub files: BTreeSet<PathBuf>,
    }

    impl Lookup for FakeLookup {
        fn app_path(&self, exe: &str) -> Option<PathBuf> {
            self.app_paths.get(exe).cloned()
        }
        fn uninstall(&self) -> Vec<(String, Option<PathBuf>)> {
            self.uninstall.clone()
        }
        fn glob(&self, pattern: &str) -> Vec<PathBuf> {
            self.globs.get(pattern).cloned().unwrap_or_default()
        }
        fn bundle(&self, id: &str) -> Option<PathBuf> {
            self.bundles.get(id).cloned()
        }
        fn desktop(&self, id: &str) -> Option<DesktopEntry> {
            self.desktops.get(id).map(|p| DesktopEntry {
                exec: format!("{id} %U"),
                icon: Some(id.to_string()),
                path: p.clone(),
            })
        }
        fn exists(&self, path: &Path) -> bool {
            self.files.contains(path)
        }
    }
}
