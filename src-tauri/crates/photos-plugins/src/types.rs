//! The plugin types shared with the frontend (ts-rs) and the error every module returns.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// What a plugin adds (spec §1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum PluginKind {
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
    // API 1.1 (Round 24 §8.4–§8.7).
    Models,
    LibraryRead,
    PlacesWrite,
    OriginalsGps,
    // API 1.3 (Round 25 P4b): proposals for the review screen.
    DatesWrite,
    TagsWrite,
    CaptionsWrite,
    PeopleWrite,
}

impl PluginPermission {
    pub const ALL: [PluginPermission; 12] = [
        PluginPermission::ImportReadChosen,
        PluginPermission::PhotosRead,
        PluginPermission::DevelopWriteDrafts,
        PluginPermission::Storage,
        PluginPermission::Models,
        PluginPermission::LibraryRead,
        PluginPermission::PlacesWrite,
        PluginPermission::OriginalsGps,
        PluginPermission::DatesWrite,
        PluginPermission::TagsWrite,
        PluginPermission::CaptionsWrite,
        PluginPermission::PeopleWrite,
    ];

    /// The permissions that let a plugin submit proposals (API 1.3), one per kind it may propose.
    pub const PROPOSING: [PluginPermission; 5] = [
        PluginPermission::PlacesWrite,
        PluginPermission::DatesWrite,
        PluginPermission::TagsWrite,
        PluginPermission::CaptionsWrite,
        PluginPermission::PeopleWrite,
    ];

    /// The manifest's id for this permission.
    pub fn id(&self) -> &'static str {
        match self {
            PluginPermission::ImportReadChosen => "import.read-chosen",
            PluginPermission::PhotosRead => "photos.read",
            PluginPermission::DevelopWriteDrafts => "develop.write-drafts",
            PluginPermission::Storage => "storage",
            PluginPermission::Models => "models",
            PluginPermission::LibraryRead => "library.read",
            PluginPermission::PlacesWrite => "places.write",
            PluginPermission::OriginalsGps => "originals.gps",
            PluginPermission::DatesWrite => "dates.write",
            PluginPermission::TagsWrite => "tags.write",
            PluginPermission::CaptionsWrite => "captions.write",
            PluginPermission::PeopleWrite => "people.write",
        }
    }

    /// The API minor version that introduced it: a manifest asking for it needs `api >= 1.<n>`.
    pub fn since_minor(&self) -> u32 {
        match self {
            PluginPermission::Models
            | PluginPermission::LibraryRead
            | PluginPermission::PlacesWrite
            | PluginPermission::OriginalsGps => 1,
            PluginPermission::DatesWrite
            | PluginPermission::TagsWrite
            | PluginPermission::CaptionsWrite
            | PluginPermission::PeopleWrite => 3,
            _ => 0,
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        PluginPermission::ALL
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
            PluginPermission::Models => "Use AI models it downloads",
            PluginPermission::LibraryRead => "Read dates, places and file names of your photos",
            PluginPermission::PlacesWrite => "Suggest places for photos",
            PluginPermission::OriginalsGps => "Add GPS to original files that have none",
            PluginPermission::DatesWrite => "Suggest dates for photos",
            PluginPermission::TagsWrite => "Suggest tags and labels for photos",
            PluginPermission::CaptionsWrite => "Suggest titles and captions for photos",
            PluginPermission::PeopleWrite => "Suggest names for faces Lumen found",
        }
    }
}

/// Lumen's file dialog kinds (Round 24 §8.1); a provider's `modes` maps each to the app's word.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, TS)]
#[serde(rename_all = "kebab-case")]
#[ts(export)]
pub enum PickKind {
    OpenFile,
    OpenMultiple,
    OpenFolder,
    Save,
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
    /// A heading on the plugin's page (§8.3).
    pub group: Option<String>,
    /// A `[[detect]]` id: disabled while that app isn't found.
    pub requires: Option<String>,
}

/// Where a plugin may add an entry (API 1.2 `[[ui.contribute]]`). Lumen decides how each slot
/// is drawn; a contribution only names the slot, a label, an icon and an action.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum ContributionSlot {
    /// The map page's toolbar.
    #[serde(rename = "map.toolbar")]
    #[ts(rename = "map.toolbar")]
    MapToolbar,
    /// The menu of a photo or a selection in the grid.
    #[serde(rename = "library.selection-menu")]
    #[ts(rename = "library.selection-menu")]
    LibrarySelectionMenu,
    /// The menu of the photo open in the viewer.
    #[serde(rename = "viewer.menu")]
    #[ts(rename = "viewer.menu")]
    ViewerMenu,
    /// The command palette only (every contribution gets a palette entry anyway).
    #[serde(rename = "palette")]
    #[ts(rename = "palette")]
    Palette,
}

/// The Tabler icons a contribution may name (`IconMapPin` is `map-pin`). The page keeps the
/// same list in `src/lib/plugin-slots.ts`.
pub const CONTRIBUTION_ICONS: &[&str] = &[
    "map-pin", "map-search", "search", "sparkles", "wand", "photo", "calendar", "tag", "user", "world",
    "download", "refresh", "settings", "list-check", "check", "folder", "scan", "bolt",
];

/// One entry a plugin adds to a slot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PluginContribution {
    pub slot: ContributionSlot,
    pub label: String,
    /// A name from [`CONTRIBUTION_ICONS`].
    pub icon: Option<String>,
    /// `open-review:<id>` opens that review page; anything else is passed to the plugin's `ui.action`.
    pub action: String,
}

/// The kinds of control a plugin's settings page may declare (API 1.2). Lumen draws every one
/// with its own components; the plugin only describes them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "kebab-case")]
#[ts(export)]
pub enum PluginControlKind {
    Toggle,
    Select,
    Segmented,
    Slider,
    Number,
    Text,
    /// Runs the plugin's `ui.action(action)` when pressed.
    Button,
    /// Static text under the control's `label`, from `body` (Markdown, no HTML).
    Markdown,
    /// A read-only line whose text comes from the plugin's `ui.value(key)`.
    Stat,
    /// A read-only list whose rows come from the plugin's `ui.value(key)` (a JSON array).
    List,
}

impl PluginControlKind {
    /// A control that holds a value the person sets (stored in `plugins.json`).
    pub fn is_value(self) -> bool {
        matches!(self, Self::Toggle | Self::Select | Self::Segmented | Self::Slider | Self::Number | Self::Text)
    }

    /// A control that needs the plugin's wasm (`ui.action` / `ui.value`).
    pub fn needs_code(self) -> bool {
        matches!(self, Self::Button | Self::Stat | Self::List)
    }
}

/// One control on a plugin's settings page (API 1.2), with its current value.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PluginControl {
    pub kind: PluginControlKind,
    pub key: Option<String>,
    pub label: String,
    pub help: Option<String>,
    /// The stored value, else the default; `None` for button, markdown, stat and list.
    #[ts(type = "unknown")]
    pub value: Option<serde_json::Value>,
    pub choices: Vec<String>,
    pub min: Option<f64>,
    pub max: Option<f64>,
    pub step: Option<f64>,
    /// Another value control's key: shown only while that one is on / non-empty.
    pub requires: Option<String>,
    /// A `[[detect]]` id: disabled while that app isn't found.
    pub app: Option<String>,
    /// `button`: what `plugin_ui_action` is called with.
    pub action: Option<String>,
    /// `markdown`: the text.
    pub body: Option<String>,
}

/// A titled run of controls on a plugin's page.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PluginSection {
    pub title: Option<String>,
    pub description: Option<String>,
    pub controls: Vec<PluginControl>,
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
    /// The settings page: flat `[[settings]]` first, then `[[ui.section]]`, with current values.
    pub sections: Vec<PluginSection>,
    /// What it adds to the map toolbar, menus and palette (API 1.2). The page shows these only
    /// while the plugin is on and ready.
    pub contributions: Vec<PluginContribution>,
    /// The plugin's `homepage`, shown on its page.
    pub homepage: Option<String>,
    /// The author's link and the source's link (https only), shown on its page.
    pub author_url: Option<String>,
    pub repository: Option<String>,
    /// `photos://plugin-shot/<id>/<n>` for each `[listing]` screenshot, in the manifest's order.
    pub screenshots: Vec<String>,
    /// The package ships a `README.md` (its text comes from `plugin_readme`, not a URL).
    pub has_readme: bool,
    /// The ids of its `[[detect]]` entries: a plugin with any (or with settings) has its own page.
    pub detect_ids: Vec<String>,
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
