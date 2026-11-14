//! The plugin types shared with the frontend (ts-rs) and the error every module returns.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// What a plugin adds (spec §1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum PluginKind {
    Theme,
    Presets,
    Apps,
    Prompts,
    Importer,
    Tools,
}

impl PluginKind {
    /// The plural name used in *<kind> isn't supported by this Lumen*.
    pub fn plural(&self) -> &'static str {
        match self {
            PluginKind::Theme => "Themes",
            PluginKind::Presets => "Presets",
            PluginKind::Apps => "Apps",
            PluginKind::Prompts => "Prompts",
            PluginKind::Importer => "Importers",
            PluginKind::Tools => "Tools",
        }
    }
}

/// API 1's four permissions (spec §3). Granted all or none.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum PluginPermission {
    ImportReadChosen,
    PhotosRead,
    DevelopWriteDrafts,
    Storage,
}

impl PluginPermission {
    /// The manifest's id for this permission.
    pub fn id(&self) -> &'static str {
        match self {
            PluginPermission::ImportReadChosen => "import.read-chosen",
            PluginPermission::PhotosRead => "photos.read",
            PluginPermission::DevelopWriteDrafts => "develop.write-drafts",
            PluginPermission::Storage => "storage",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        [
            PluginPermission::ImportReadChosen,
            PluginPermission::PhotosRead,
            PluginPermission::DevelopWriteDrafts,
            PluginPermission::Storage,
        ]
        .into_iter()
        .find(|p| p.id() == id)
    }

    /// How the install sheet shows it (spec §3, verbatim).
    pub fn label(&self) -> &'static str {
        match self {
            PluginPermission::ImportReadChosen => {
                "Read the folder or file you choose when you import with it"
            }
            PluginPermission::PhotosRead => {
                "See the photos in Develop's filmstrip: previews up to 1024 px, and their date, camera, lens, size, rating, label, flag and place"
            }
            PluginPermission::DevelopWriteDrafts => {
                "Change Develop drafts of those photos (every change can be undone)"
            }
            PluginPermission::Storage => "Keep its own files (up to 50 MB)",
        }
    }
}

/// Who made a plugin, as far as its signature proves (spec §8).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "camelCase")]
#[ts(export)]
pub enum PluginTrust {
    JlStudios,
    Author { name: String, fingerprint: String },
    Unsigned,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "camelCase")]
#[ts(export)]
pub enum PluginState {
    Ready,
    Off,
    NeedsNewerLumen,
    Problem { reason: String },
    Stopped { reason: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum SettingKind {
    Bool,
    Number,
    Text,
    Choice,
}

/// What an importer asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, rename = "PluginPick")]
pub enum Pick {
    Folder,
    File,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PluginSetting {
    pub key: String,
    pub label: String,
    pub kind: SettingKind,
    #[ts(type = "unknown")]
    pub value: serde_json::Value,
    pub choices: Vec<String>,
}

/// One `[[assets]]` entry, for Settings → Licences.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PluginAsset {
    pub path: String,
    pub license: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PluginInfo {
    pub id: String,
    pub name: String,
    pub version: String,
    pub author: String,
    pub description: String,
    pub license: String,
    pub trust: PluginTrust,
    pub kinds: Vec<PluginKind>,
    pub permissions: Vec<PluginPermission>,
    pub enabled: bool,
    pub dev: bool,
    pub state: PluginState,
    pub icon_url: Option<String>,
    pub settings: Vec<PluginSetting>,
    pub assets: Vec<PluginAsset>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PluginUpdate {
    pub from: String,
    pub new_permissions: Vec<PluginPermission>,
}

/// What the install sheet shows before anything is installed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PluginInspect {
    pub info: PluginInfo,
    pub adds: Vec<String>,
    pub problems: Vec<String>,
    pub notes: Vec<String>,
    pub update: Option<PluginUpdate>,
    pub older: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PluginImporter {
    pub plugin: String,
    pub label: String,
    pub pick: Pick,
    pub extensions: Vec<String>,
    pub hint: String,
}

/// Every failure in this crate: `code` is one of spec §11's `plugin_*` codes, copied into
/// `AppError.code` by the binary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginError {
    pub code: &'static str,
    pub message: String,
}

impl std::fmt::Display for PluginError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for PluginError {}

macro_rules! codes {
    ($($fn:ident => $code:literal),* $(,)?) => {
        impl PluginError {
            $(pub fn $fn(message: impl Into<String>) -> Self {
                PluginError { code: $code, message: message.into() }
            })*
        }
    };
}

codes! {
    invalid => "plugin_invalid",
    signature => "plugin_signature",
    key_changed => "plugin_key_changed",
    unsigned => "plugin_unsigned",
    licence => "plugin_licence",
    api => "plugin_api",
    permission => "plugin_permission",
    trap => "plugin_trap",
    timeout => "plugin_timeout",
    chosen_root => "plugin_chosen_root",
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: [PluginPermission; 4] = [
        PluginPermission::ImportReadChosen,
        PluginPermission::PhotosRead,
        PluginPermission::DevelopWriteDrafts,
        PluginPermission::Storage,
    ];

    #[test]
    fn permission_ids_round_trip() {
        for p in ALL {
            assert_eq!(PluginPermission::from_id(p.id()), Some(p));
        }
        assert_eq!(PluginPermission::from_id("network"), None);
    }

    #[test]
    fn permission_labels_are_spec_copy() {
        assert_eq!(
            PluginPermission::ImportReadChosen.label(),
            "Read the folder or file you choose when you import with it"
        );
        assert_eq!(
            PluginPermission::PhotosRead.label(),
            "See the photos in Develop's filmstrip: previews up to 1024 px, and their date, camera, lens, size, rating, label, flag and place"
        );
        assert_eq!(
            PluginPermission::DevelopWriteDrafts.label(),
            "Change Develop drafts of those photos (every change can be undone)"
        );
        assert_eq!(
            PluginPermission::Storage.label(),
            "Keep its own files (up to 50 MB)"
        );
    }

    #[test]
    fn ts_types_export() {
        let cfg = ts_rs::Config::from_env();
        for decl in [
            PluginKind::decl(&cfg),
            PluginPermission::decl(&cfg),
            PluginTrust::decl(&cfg),
            PluginState::decl(&cfg),
            SettingKind::decl(&cfg),
            Pick::decl(&cfg),
            PluginSetting::decl(&cfg),
            PluginInfo::decl(&cfg),
            PluginAsset::decl(&cfg),
            PluginUpdate::decl(&cfg),
            PluginInspect::decl(&cfg),
            PluginImporter::decl(&cfg),
        ] {
            assert!(decl.starts_with("type "), "{decl}");
        }
        assert!(PluginInfo::decl(&cfg).contains("iconUrl"));
        assert!(PluginTrust::decl(&cfg).contains("\"type\": \"author\""));
    }

    #[test]
    fn error_codes_are_the_spec_list() {
        let codes = [
            PluginError::invalid("x").code,
            PluginError::signature("x").code,
            PluginError::key_changed("x").code,
            PluginError::unsigned("x").code,
            PluginError::licence("x").code,
            PluginError::api("x").code,
            PluginError::permission("x").code,
            PluginError::trap("x").code,
            PluginError::timeout("x").code,
            PluginError::chosen_root("x").code,
        ];
        assert_eq!(
            codes,
            [
                "plugin_invalid",
                "plugin_signature",
                "plugin_key_changed",
                "plugin_unsigned",
                "plugin_licence",
                "plugin_api",
                "plugin_permission",
                "plugin_trap",
                "plugin_timeout",
                "plugin_chosen_root"
            ]
        );
    }
}
