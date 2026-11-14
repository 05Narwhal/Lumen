use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use ts_rs::TS;

use crate::external_apps::{CustomApp, DefaultApps, SendMode};
use crate::Result;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS, Default)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum ThemeId {
    #[default]
    Gallery,
    Darkroom,
    Paper,
    Glass,
}

/// The theme the person picked: one of the four built in (a plain string in `settings.json`, as
/// it always was) or a theme a plugin adds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(untagged)]
#[ts(export)]
pub enum ThemeChoice {
    Builtin(ThemeId),
    Plugin { plugin: String, theme: String },
}

impl Default for ThemeChoice {
    fn default() -> Self {
        ThemeChoice::Builtin(ThemeId::default())
    }
}

impl ThemeChoice {
    /// The built-in theme the page is drawn on. A plugin theme's own base when `lookup` knows it
    /// (the enabled plugin themes); a plugin that is off or gone falls back to the default theme.
    pub fn base(&self, lookup: impl Fn(&str, &str) -> Option<ThemeId>) -> ThemeId {
        match self {
            ThemeChoice::Builtin(id) => *id,
            ThemeChoice::Plugin { plugin, theme } => lookup(plugin, theme).unwrap_or_default(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS, Default)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum ColorScheme {
    Light,
    Dark,
    #[default]
    Auto,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS, Default)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum TileSize {
    S,
    #[default]
    M,
    L,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS, Default)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum DuplicateHandling {
    #[default]
    Skip,
    ImportAnyway,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Appearance {
    pub theme: ThemeChoice,
    pub scheme: ColorScheme,
    pub tile_size: TileSize,
    /// How the Library timeline is cut into sections: by day, month or year.
    #[serde(default)]
    pub library_group_by: crate::layout::GroupBy,
    /// Settings → Library: a RAW file the camera also saved as a JPEG is left out of every
    /// view but the Trash, so only the JPEG shows.
    #[serde(default)]
    pub hide_raw_with_jpeg: bool,
    #[serde(default)]
    pub reduce_motion: bool,
    #[serde(default)]
    pub sidebar_collapsed: bool,
    #[serde(flatten)]
    #[ts(skip)]
    pub extra: Map<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ImportSettings {
    pub duplicates: DuplicateHandling,
    pub prompt_on_volume: bool,
    #[serde(flatten)]
    #[ts(skip)]
    pub extra: Map<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Playback {
    pub loop_videos: bool,
    pub mute_by_default: bool,
    /// Settings → Playback & codecs: export slideshows with the graphics card's H.264 encoder
    /// when there is one that works (Round 4F). Absent means on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub hardware_encoding: Option<bool>,
    /// Settings → Playback & codecs: prepare videos this computer cannot play as they are in
    /// the background, at import and once for the library (Round 9A). Absent means on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub prepare_videos: Option<bool>,
    #[serde(flatten)]
    #[ts(skip)]
    pub extra: Map<String, Value>,
}

/// Which base map the Map screen draws when online detail is on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS, Default)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum MapStyle {
    #[default]
    Map,
    Satellite,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct MapSettings {
    /// Street-level detail and satellite imagery from online tile servers. Off, the map is
    /// the bundled world outline only and makes no network request.
    #[serde(default = "yes")]
    pub online: bool,
    #[serde(default)]
    pub style: MapStyle,
    #[serde(flatten)]
    #[ts(skip)]
    pub extra: Map<String, Value>,
}

fn yes() -> bool {
    true
}

impl Default for MapSettings {
    fn default() -> Self {
        Self { online: true, style: MapStyle::Map, extra: Map::new() }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct WindowState {
    pub width: u32,
    pub height: u32,
    pub x: Option<i32>,
    pub y: Option<i32>,
    pub maximized: bool,
    /// Where *Restart Lumen* left off (route + query), for the next launch to open once
    /// (Round 15). Read and cleared by `take_resume`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub resume: Option<String>,
}

/// The music browser is the only part of Lumen that reaches the network, and it stays
/// shut until a free Jamendo client id is entered here.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct MusicSettings {
    #[serde(default)]
    pub jamendo_client_id: Option<String>,
    /// Anything a later version writes here survives a round trip through this one.
    #[serde(flatten)]
    #[ts(skip)]
    pub extra: Map<String, Value>,
}

/// On-device models (Phase 7). On by default: the models are fetched automatically
/// (ML design §2.3), and this is the switch that stops both fetching and inference.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SmartSettings {
    #[serde(default = "yes")]
    pub enabled: bool,
    /// Denoise on the graphics card (DirectML, Windows) when it works (Phase 8d).
    #[serde(default = "yes")]
    pub gpu: bool,
    /// Trained models behind switches, off by default (Round 12).
    #[serde(default)]
    pub experimental: Experimental,
    #[serde(flatten)]
    #[ts(skip)]
    pub extra: Map<String, Value>,
}

impl Default for SmartSettings {
    fn default() -> Self {
        Self { enabled: true, gpu: true, experimental: Experimental::default(), extra: Map::new() }
    }
}

/// Settings → Smart features → Experimental (Round 12). Each switch uses the model in
/// the dev models folder; both are off by default and old settings files read as off.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Experimental {
    /// Auto starts from the trained model's tone and colour (Round 12).
    #[serde(default)]
    pub auto_model: bool,
    /// The Retouch button in Develop (Round 12).
    #[serde(default)]
    pub retouch: bool,
    /// *Read text in photos* on the Glyph row (Round 13): the text scan runs and search uses it.
    #[serde(default)]
    pub read_text: bool,
    /// AI masks on the Facet row (Phase 8f): Sky, Subject, Background and Person in Develop's
    /// Masks panel. Off, the model is never fetched and the items are hidden.
    #[serde(default)]
    pub ai_masks: bool,
    /// The model and performance tier per task (Round 14, Settings → Experimental).
    #[serde(default)]
    pub tasks: ExperimentalTasks,
}

/// A task's performance tier: `Auto` follows the tier Lumen recommends for this computer.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export)]
pub enum TierChoice {
    #[default]
    Auto,
    Low,
    High,
    Ultra,
}

/// The model (its id; `None` is the newest) and tier a task runs.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct TaskChoice {
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub tier: TierChoice,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ExperimentalTasks {
    #[serde(default)]
    pub auto: TaskChoice,
    #[serde(default)]
    pub retouch: TaskChoice,
}

/// Settings → Export (Round 8A).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ExportSettings {
    /// Lumen's mark at the bottom right of every video Lumen makes (slideshows, memory films).
    /// Never on exported photos or the library's own videos.
    #[serde(default = "yes")]
    pub watermark: bool,
    #[serde(flatten)]
    #[ts(skip)]
    pub extra: Map<String, Value>,
}

impl Default for ExportSettings {
    fn default() -> Self {
        Self { watermark: true, extra: Map::new() }
    }
}

/// Which way round a date written in numbers is read and shown (Round 11): `4.6.2026` is
/// 4 June under `Dmy`, 6 April under `Mdy`, and `2026-06-04` under `Ymd`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS, Default)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum DateOrder {
    #[default]
    Dmy,
    Mdy,
    Ymd,
}

/// Settings → Search & dates: the date order, or `Auto` for the system's.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS, Default)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum DateOrderSetting {
    #[default]
    Auto,
    Dmy,
    Mdy,
    Ymd,
}

impl DateOrderSetting {
    /// The order this setting means on a system whose locale is `locale` (a BCP 47 tag).
    pub fn resolve(self, locale: &str) -> DateOrder {
        match self {
            DateOrderSetting::Auto => order_for_locale(locale),
            DateOrderSetting::Dmy => DateOrder::Dmy,
            DateOrderSetting::Mdy => DateOrder::Mdy,
            DateOrderSetting::Ymd => DateOrder::Ymd,
        }
    }
}

/// The date order a locale writes in: month first in the US and the few places that follow
/// it, year first in East Asia, Hungary, Lithuania, Mongolia, Sweden and South Africa, day
/// first everywhere else (and for a tag that names no region, such as `C`).
pub fn order_for_locale(tag: &str) -> DateOrder {
    let tag = tag.split(['.', '@']).next().unwrap_or("").replace('_', "-").to_ascii_lowercase();
    let mut parts = tag.split('-');
    let lang = parts.next().unwrap_or("");
    // The region is the first two-letter (or three-digit) subtag after the language.
    let region = parts.find(|p| p.len() == 2 || (p.len() == 3 && p.bytes().all(|b| b.is_ascii_digit()))).unwrap_or("");
    match (lang, region) {
        ("en", "us" | "ph" | "ca" | "bz" | "fm" | "mh" | "pw") | ("fil", _) => DateOrder::Mdy,
        ("zh" | "ja" | "ko" | "hu" | "lt" | "mn", _) | ("en", "za") | ("sv", "se" | "") => DateOrder::Ymd,
        _ => DateOrder::Dmy,
    }
}

/// Settings → Search & dates (Round 11).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SearchSettings {
    #[serde(default)]
    pub date_order: DateOrderSetting,
    #[serde(flatten)]
    #[ts(skip)]
    pub extra: Map<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Settings {
    pub version: u32,
    #[serde(default)]
    pub appearance: Appearance,
    #[serde(default)]
    pub import: ImportSettings,
    #[serde(default)]
    pub playback: Playback,
    #[serde(default)]
    pub map: MapSettings,
    #[serde(default)]
    pub music: MusicSettings,
    #[serde(default)]
    pub smart: SmartSettings,
    #[serde(default)]
    pub export: ExportSettings,
    #[serde(default)]
    pub search: SearchSettings,
    #[serde(default)]
    pub window: WindowState,
    #[serde(default)]
    pub ai_assistants: AiAssistants,
    #[serde(default)]
    pub updates: UpdateSettings,
    #[serde(default)]
    pub discord: DiscordStatus,
    /// Round 22 lane C: the apps photos open in (*Edit in…*, *Open with*).
    #[serde(default)]
    pub external_apps: ExternalAppsSettings,
    /// Exact place names (plan 2026-11-08): send each photo's position, never the photo, to
    /// OpenStreetMap to name its town and village. Off by default; off, the atlas names them.
    #[serde(default)]
    pub exact_places: bool,
    /// The person's own keys by action id (Round 19 §3). A missing id keeps its default; an
    /// empty list unbinds it.
    #[serde(default)]
    pub keybinds: BTreeMap<String, Vec<String>>,
    #[serde(flatten)]
    #[ts(skip)]
    pub extra: Map<String, Value>,
}

/// Which builds a person takes (Round 17). A channel takes its own stability and anything more
/// stable: Stable takes only `-stable`, Alpha takes everything.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum UpdateChannel {
    #[default]
    Stable,
    Beta,
    Alpha,
}

/// Settings → Updates (Round 17).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct UpdateSettings {
    /// Download in the background and install on close. Off, nothing happens until *Check*.
    #[serde(default = "default_true")]
    pub automatic: bool,
    #[serde(default)]
    pub channel: UpdateChannel,
    #[serde(flatten)]
    #[ts(skip)]
    pub extra: Map<String, Value>,
}

/// Settings → External apps (Round 22 lane C §7): the person's own apps, the app each kind of
/// media opens in, and what is sent to an app (absent = automatic).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ExternalAppsSettings {
    #[serde(default)]
    pub custom: Vec<CustomApp>,
    #[serde(default)]
    pub defaults: DefaultApps,
    #[serde(default)]
    pub send: BTreeMap<String, SendMode>,
    #[serde(flatten)]
    #[ts(skip)]
    pub extra: Map<String, Value>,
}

/// The opt-in Discord status (Round 17): *Competing in Lumen* and one coarse activity, never
/// anything about the photos. Off, no pipe is opened.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DiscordStatus {
    #[serde(default)]
    pub enabled: bool,
    /// Say what Lumen is doing (browsing, developing…); off leaves the line out.
    #[serde(default = "default_true")]
    pub show_activity: bool,
    #[serde(default = "default_true")]
    pub show_elapsed: bool,
    #[serde(flatten)]
    #[ts(skip)]
    pub extra: Map<String, Value>,
}

fn default_true() -> bool {
    true
}

impl Default for UpdateSettings {
    fn default() -> Self {
        Self { automatic: true, channel: UpdateChannel::Stable, extra: Map::new() }
    }
}

impl Default for DiscordStatus {
    fn default() -> Self {
        Self { enabled: false, show_activity: true, show_elapsed: true, extra: Map::new() }
    }
}

/// The local MCP server for AI assistants in Develop (Round 16). Off, no port is open.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AiAssistants {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_mcp_port")]
    pub port: u16,
    /// The bearer token clients send; made by Rust on first enable, empty until then.
    #[serde(default)]
    pub token: String,
    #[serde(flatten)]
    #[ts(skip)]
    pub extra: Map<String, Value>,
}

fn default_mcp_port() -> u16 {
    47820
}

impl Default for AiAssistants {
    fn default() -> Self {
        Self { enabled: false, port: default_mcp_port(), token: String::new(), extra: Map::new() }
    }
}

impl Default for Appearance {
    fn default() -> Self {
        Self {
            theme: ThemeChoice::Builtin(ThemeId::Gallery),
            scheme: ColorScheme::Auto,
            tile_size: TileSize::M,
            library_group_by: crate::layout::GroupBy::Month,
            hide_raw_with_jpeg: false,
            reduce_motion: false,
            sidebar_collapsed: false,
            extra: Map::new(),
        }
    }
}

impl Default for ImportSettings {
    fn default() -> Self {
        Self { duplicates: DuplicateHandling::Skip, prompt_on_volume: true, extra: Map::new() }
    }
}

impl Default for Playback {
    fn default() -> Self {
        Self { loop_videos: false, mute_by_default: false, hardware_encoding: None, prepare_videos: None, extra: Map::new() }
    }
}

impl Default for WindowState {
    fn default() -> Self {
        Self { width: 1280, height: 800, x: None, y: None, maximized: false, resume: None }
    }
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            version: 1,
            appearance: Default::default(),
            import: Default::default(),
            playback: Default::default(),
            map: Default::default(),
            music: Default::default(),
            smart: Default::default(),
            export: Default::default(),
            search: Default::default(),
            window: Default::default(),
            ai_assistants: Default::default(),
            updates: Default::default(),
            discord: Default::default(),
            external_apps: Default::default(),
            exact_places: false,
            keybinds: BTreeMap::new(),
            extra: Map::new(),
        }
    }
}

impl Settings {
    /// Missing file → defaults. A corrupted file is an error, not silently reset.
    pub fn load(path: &Path) -> Result<Self> {
        match std::fs::read_to_string(path) {
            Ok(text) => Ok(serde_json::from_str(&text)?),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e.into()),
        }
    }

    /// Atomic: write `<path>.tmp`, then rename over `path`.
    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_string_pretty(self)?)?;
        std::fs::rename(&tmp, path)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn settings_from_before_8d_use_the_gpu() {
        let s: super::Settings = serde_json::from_str(r#"{"version":1,"smart":{"enabled":true}}"#).unwrap();
        assert!(s.smart.gpu);
    }

    #[test]
    fn settings_from_before_phase_7_have_smart_features_on() {
        let s: Settings = serde_json::from_str(r#"{"version":1}"#).unwrap();
        assert!(s.smart.enabled, "models are fetched automatically unless switched off");
        let off: Settings = serde_json::from_str(r#"{"version":1,"smart":{"enabled":false}}"#).unwrap();
        assert!(!off.smart.enabled);
        let blank: Settings = serde_json::from_str(r#"{"version":1,"smart":{}}"#).unwrap();
        assert!(blank.smart.enabled);
    }

    use super::*;

    #[test]
    fn the_locale_says_which_way_round_a_date_is_read() {
        let cases = [
            ("en-US", DateOrder::Mdy),
            ("en-GB", DateOrder::Dmy),
            ("de-DE", DateOrder::Dmy),
            ("sl-SI", DateOrder::Dmy),
            ("ja-JP", DateOrder::Ymd),
            ("zh-CN", DateOrder::Ymd),
            ("hu-HU", DateOrder::Ymd),
            ("en-CA", DateOrder::Mdy),
            ("fil-PH", DateOrder::Mdy),
            ("en_US.UTF-8", DateOrder::Mdy),
            ("ko", DateOrder::Ymd),
            ("sv-SE", DateOrder::Ymd),
            ("en-ZA", DateOrder::Ymd),
            ("en-PH", DateOrder::Mdy),
            ("", DateOrder::Dmy),
            ("C", DateOrder::Dmy),
        ];
        for (tag, order) in cases {
            assert_eq!(order_for_locale(tag), order, "{tag}");
        }
    }

    #[test]
    fn settings_from_before_round_11_read_dates_in_the_systems_order() {
        let s: Settings = serde_json::from_str(r#"{"version":1}"#).unwrap();
        assert_eq!(s.search.date_order, DateOrderSetting::Auto);
        let mdy: Settings = serde_json::from_str(r#"{"version":1,"search":{"dateOrder":"mdy"}}"#).unwrap();
        assert_eq!(mdy.search.date_order, DateOrderSetting::Mdy);
        assert_eq!(mdy.search.date_order.resolve("de-DE"), DateOrder::Mdy);
        assert_eq!(DateOrderSetting::Auto.resolve("ja-JP"), DateOrder::Ymd);
    }

    #[test]
    fn videos_lumen_makes_carry_the_mark_unless_switched_off() {
        let s: Settings = serde_json::from_str(r#"{"version":1}"#).unwrap();
        assert!(s.export.watermark, "on for settings written before Round 8A");
        let off: Settings = serde_json::from_str(r#"{"version":1,"export":{"watermark":false}}"#).unwrap();
        assert!(!off.export.watermark);
        assert_eq!(serde_json::to_value(&off).unwrap()["export"]["watermark"], false);
    }

    #[test]
    fn settings_written_before_the_map_section_load_with_online_detail() {
        let old = r#"{"version":1,"appearance":{"theme":"glass","scheme":"auto","tileSize":"m"}}"#;
        let s: Settings = serde_json::from_str(old).unwrap();
        assert!(s.map.online);
        assert_eq!(s.map.style, MapStyle::Map);
        let back = serde_json::to_value(&s).unwrap();
        assert_eq!(back["map"]["style"], "map");
    }

    /// Round 16: settings from before AI assistants load with the server off and no token.
    #[test]
    fn old_settings_update_automatically_on_stable() {
        let old: Settings = serde_json::from_str(r#"{"version":1}"#).unwrap();
        assert!(old.updates.automatic);
        assert_eq!(old.updates.channel, UpdateChannel::Stable);
        assert_eq!(Settings::default().updates, old.updates);
        let chosen: Settings = serde_json::from_str(r#"{"version":1,"updates":{"channel":"alpha"}}"#).unwrap();
        assert!(chosen.updates.automatic, "a missing field keeps its default");
        assert_eq!(chosen.updates.channel, UpdateChannel::Alpha);
        let off: Settings = serde_json::from_str(r#"{"version":1,"updates":{"automatic":false}}"#).unwrap();
        assert!(!off.updates.automatic);
    }

    #[test]
    fn old_settings_have_ai_assistants_off() {
        let old = r#"{"version":1,"aiAssistants":{"port":5000}}"#;
        let s: Settings = serde_json::from_str(old).unwrap();
        assert_eq!((s.ai_assistants.enabled, s.ai_assistants.port, s.ai_assistants.token.as_str()), (false, 5000, ""));
        let none: Settings = serde_json::from_str(r#"{"version":1}"#).unwrap();
        assert_eq!(none.ai_assistants, AiAssistants::default());
        assert_eq!(none.ai_assistants.port, 47820);
    }

    /// Round 17: settings from before the Discord status load with it off and both details on.
    #[test]
    fn old_settings_have_discord_off() {
        let none: Settings = serde_json::from_str(r#"{"version":1}"#).unwrap();
        assert_eq!((none.discord.enabled, none.discord.show_activity, none.discord.show_elapsed), (false, true, true));
        assert_eq!(none.discord, DiscordStatus::default());
        let on: Settings = serde_json::from_str(r#"{"version":1,"discord":{"enabled":true}}"#).unwrap();
        assert_eq!((on.discord.enabled, on.discord.show_activity, on.discord.show_elapsed), (true, true, true));
        let back = serde_json::to_value(&on).unwrap();
        assert_eq!(back["discord"]["showActivity"], true);
    }

    /// Round 19 §3: a person's own keys are stored by action id; an empty list means unbound.
    #[test]
    fn keybind_overrides_round_trip_and_old_settings_have_none() {
        let none: Settings = serde_json::from_str(r#"{"version":1}"#).unwrap();
        assert!(none.keybinds.is_empty());
        let s: Settings = serde_json::from_str(r#"{"version":1,"keybinds":{"go-library":["Mod+L"],"grid.open":[]}}"#).unwrap();
        assert_eq!(s.keybinds["go-library"], vec!["Mod+L".to_string()]);
        assert!(s.keybinds["grid.open"].is_empty());
        let back = serde_json::to_value(&s).unwrap();
        assert_eq!(back["keybinds"]["grid.open"], serde_json::json!([]));
        assert!(back.get("extra").is_none());
    }

    #[test]
    fn old_settings_theme_string_reads() {
        let s: Settings = serde_json::from_str(r#"{"version":1,"appearance":{"theme":"glass","scheme":"dark","tileSize":"l","reduceMotion":false,"sidebarCollapsed":false}}"#).unwrap();
        assert_eq!(s.appearance.theme, ThemeChoice::Builtin(ThemeId::Glass));
        let back = serde_json::to_value(&s).unwrap();
        assert_eq!(back["appearance"]["theme"], "glass");
    }

    #[test]
    fn plugin_choice_round_trips() {
        let text = r#"{"version":1,"appearance":{"theme":{"plugin":"a.b","theme":"nord"},"scheme":"dark","tileSize":"l","reduceMotion":false,"sidebarCollapsed":false}}"#;
        let s: Settings = serde_json::from_str(text).unwrap();
        assert_eq!(s.appearance.theme, ThemeChoice::Plugin { plugin: "a.b".into(), theme: "nord".into() });
        let back = serde_json::to_value(&s).unwrap();
        assert_eq!(back["appearance"]["theme"], serde_json::json!({"plugin":"a.b","theme":"nord"}));
        let again: Settings = serde_json::from_value(back).unwrap();
        assert_eq!(again, s);
    }

    #[test]
    fn missing_plugin_choice_uses_base() {
        let choice = ThemeChoice::Plugin { plugin: "a.b".into(), theme: "nord".into() };
        // Not among the enabled themes (off, removed): the settings default.
        assert_eq!(choice.base(|_, _| None), ThemeId::Gallery);
        // Known: its own base.
        let found = choice.base(|p, t| (p == "a.b" && t == "nord").then_some(ThemeId::Darkroom));
        assert_eq!(found, ThemeId::Darkroom);
        assert_eq!(ThemeChoice::Builtin(ThemeId::Paper).base(|_, _| None), ThemeId::Paper);
    }

    #[test]
    fn default_matches_ux_spec() {
        let s = Settings::default();
        assert_eq!(s.version, 1);
        assert_eq!(s.appearance.theme, ThemeChoice::Builtin(ThemeId::Gallery));
        assert_eq!(s.appearance.scheme, ColorScheme::Auto);
        assert_eq!(s.appearance.tile_size, TileSize::M);
        assert_eq!(s.window.width, 1280);
        assert_eq!(s.window.height, 800);
    }

    #[test]
    fn missing_file_loads_default() {
        let tmp = tempfile::tempdir().unwrap();
        let s = Settings::load(&tmp.path().join("settings.json")).unwrap();
        assert_eq!(s, Settings::default());
    }

    #[test]
    fn round_trips_and_preserves_unknown_keys() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("settings.json");
        std::fs::write(&path, r#"{"version":1,"appearance":{"theme":"paper","scheme":"dark","tileSize":"l","reduceMotion":true,"sidebarCollapsed":false,"futureKey":5},"futureSection":{"a":1}}"#).unwrap();
        let mut s = Settings::load(&path).unwrap();
        assert_eq!(s.appearance.theme, ThemeChoice::Builtin(ThemeId::Paper));
        assert!(s.appearance.reduce_motion);
        s.appearance.theme = ThemeChoice::Builtin(ThemeId::Glass);
        s.save(&path).unwrap();
        let raw: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(raw["appearance"]["theme"], "glass");
        assert_eq!(raw["appearance"]["futureKey"], 5);
        assert_eq!(raw["futureSection"]["a"], 1);
        assert!(!tmp.path().join("settings.json.tmp").exists());
    }
    #[test]
    fn old_smart_settings_have_experimental_switches_off() {
        let s: super::SmartSettings = serde_json::from_str(r#"{"enabled":true,"gpu":true}"#).unwrap();
        assert!(!s.experimental.auto_model);
        assert!(!s.experimental.retouch);
        assert!(!s.experimental.read_text);
        assert_eq!(super::SmartSettings::default().experimental, super::Experimental::default());
    }

    /// Round 13: *Read text in photos* is off in old files and kept when on.
    #[test]
    fn read_text_switch_round_trips() {
        let s: super::SmartSettings = serde_json::from_str(r#"{"experimental":{"readText":true}}"#).unwrap();
        assert!(s.experimental.read_text);
        assert!(!s.experimental.retouch);
        assert_eq!(serde_json::to_value(&s).unwrap()["experimental"]["readText"], true);
    }

    #[test]
    fn experimental_switches_keep_unknown_keys() {
        let s: super::SmartSettings =
            serde_json::from_str(r#"{"experimental":{"retouch":true},"futureKey":1}"#).unwrap();
        assert!(s.experimental.retouch);
        assert!(!s.experimental.auto_model);
        let raw = serde_json::to_value(&s).unwrap();
        assert_eq!(raw["futureKey"], 1);
        assert_eq!(raw["experimental"]["retouch"], true);
    }

    #[test]
    fn old_experimental_settings_get_default_task_choices() {
        let s: super::SmartSettings = serde_json::from_str(r#"{"experimental":{"retouch":true}}"#).unwrap();
        assert_eq!(s.experimental.tasks, super::ExperimentalTasks::default());
        assert_eq!(s.experimental.tasks.auto.tier, super::TierChoice::Auto);
        assert_eq!(s.experimental.tasks.retouch.model, None);
    }

    #[test]
    fn task_choices_round_trip() {
        let s: super::SmartSettings = serde_json::from_str(
            r#"{"experimental":{"autoModel":true,"tasks":{"auto":{"model":"lumen-ray-1.0","tier":"ultra"},"retouch":{"tier":"low"}}}}"#,
        )
        .unwrap();
        assert_eq!(s.experimental.tasks.auto, super::TaskChoice { model: Some("lumen-ray-1.0".into()), tier: super::TierChoice::Ultra });
        assert_eq!(s.experimental.tasks.retouch.tier, super::TierChoice::Low);
        let raw = serde_json::to_value(&s).unwrap();
        assert_eq!(raw["experimental"]["tasks"]["auto"]["tier"], "ultra");
        let back: super::SmartSettings = serde_json::from_value(raw).unwrap();
        assert_eq!(back.experimental, s.experimental);
    }
}
