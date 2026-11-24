//! `plugin.toml` (spec §4): parsed strictly, then checked field by field.

use std::collections::BTreeMap;

use serde::Deserialize;

use crate::{
    ContributionSlot, Pick, PickKind, PluginContribution, PluginError, PluginKind, PluginPermission, SettingKind,
    CONTRIBUTION_ICONS,
};

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FileRef {
    pub file: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DirRef {
    pub dir: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PromptDecl {
    pub name: String,
    pub description: String,
    pub file: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImporterDecl {
    pub label: String,
    pub pick: Pick,
    #[serde(default)]
    pub extensions: Vec<String>,
    #[serde(default)]
    pub hint: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolDecl {
    pub name: String,
    pub description: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AssetDecl {
    pub path: String,
    pub license: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SettingDecl {
    pub key: String,
    pub label: String,
    #[serde(rename = "type")]
    pub kind: SettingKind,
    pub default: serde_json::Value,
    #[serde(default)]
    pub choices: Vec<String>,
    /// A heading on the plugin's settings page (Round 24 §8.3).
    #[serde(default)]
    pub group: Option<String>,
    /// A `[[detect]]` id: the control is disabled while that app isn't found.
    #[serde(default)]
    pub requires: Option<String>,
    /// A `slider` / `number` control's bounds (API 1.2); a stored value outside them is refused.
    #[serde(skip)]
    pub range: Option<(f64, f64)>,
}

pub use crate::PluginControlKind as ControlKind;

/// One `[[ui.section.control]]`.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UiControl {
    #[serde(rename = "type")]
    pub kind: ControlKind,
    /// Required for a value control, `stat` and `list`.
    #[serde(default)]
    pub key: Option<String>,
    pub label: String,
    #[serde(default)]
    pub help: Option<String>,
    #[serde(default)]
    pub default: Option<serde_json::Value>,
    /// `select` and `segmented`.
    #[serde(default)]
    pub choices: Vec<String>,
    /// `slider` (both) and `number` (either).
    #[serde(default)]
    pub min: Option<f64>,
    #[serde(default)]
    pub max: Option<f64>,
    #[serde(default)]
    pub step: Option<f64>,
    /// Another value control's key: this control shows only while that one is on / non-empty.
    #[serde(default)]
    pub requires: Option<String>,
    /// A `[[detect]]` id: the control is disabled while that app isn't found.
    #[serde(default)]
    pub app: Option<String>,
    /// `button`: passed to the plugin's `ui.action`.
    #[serde(default)]
    pub action: Option<String>,
    /// `markdown`: the text.
    #[serde(default)]
    pub body: Option<String>,
}

/// One `[[ui.section]]`: a heading, a line of text and its controls. A flat `[[settings]]` list
/// reads as sections too (one per `group`, untitled when it has none).
#[derive(Debug, Clone, PartialEq, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UiSection {
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default, rename = "control")]
    pub controls: Vec<UiControl>,
}

/// One `[[ui.contribute]]`.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawContribution {
    slot: ContributionSlot,
    label: String,
    #[serde(default)]
    icon: Option<String>,
    action: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawUi {
    #[serde(default)]
    section: Vec<UiSection>,
    #[serde(default)]
    contribute: Vec<RawContribution>,
}

/// The longest label and action a contribution may have.
const MAX_CONTRIBUTION_LABEL: usize = 60;
const MAX_CONTRIBUTION_ACTION: usize = 64;
/// The action that opens a plugin's review page instead of calling its wasm.
pub const OPEN_REVIEW: &str = "open-review:";
/// The action that runs one of the plugin's `[[tools]]` on the slot's scope (selection, album or
/// library) instead of calling its `ui.action`.
pub const RUN_TOOL: &str = "tool:";

/// True when a contribution's action is answered by the plugin's `ui.action` (neither a review
/// page nor a tool), so `plugin.wasm` must export `ui`.
pub fn needs_ui_export(m: &Manifest) -> bool {
    m.contributions.iter().any(|c| !c.action.starts_with(OPEN_REVIEW) && !c.action.starts_with(RUN_TOOL))
}

/// Checks the `[[ui.contribute]]` tables.
fn check_contributions(raw: Vec<RawContribution>, has_code: bool, tools: &[ToolDecl]) -> Result<Vec<PluginContribution>, PluginError> {
    let mut out: Vec<PluginContribution> = Vec::new();
    for c in raw {
        let bad = |why: &str| invalid(format!("the contribution \"{}\" {why}", c.label));
        let label = c.label.trim();
        if label.is_empty() || label.chars().count() > MAX_CONTRIBUTION_LABEL {
            return Err(bad(&format!("needs a label of 1–{MAX_CONTRIBUTION_LABEL} characters")));
        }
        if c.action.is_empty() || c.action.chars().count() > MAX_CONTRIBUTION_ACTION {
            return Err(bad(&format!("needs an action of 1–{MAX_CONTRIBUTION_ACTION} characters")));
        }
        if let Some(i) = &c.icon {
            if !CONTRIBUTION_ICONS.contains(&i.as_str()) {
                return Err(bad(&format!("names the icon \"{i}\", which isn't on the list")));
            }
        }
        match c.action.strip_prefix(OPEN_REVIEW) {
            Some(id) if id.is_empty() || !id.chars().all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_') => {
                return Err(bad("opens a review whose id is empty or has odd characters"));
            }
            Some(_) => {}
            None if c.action.starts_with(RUN_TOOL) => {
                let name = &c.action[RUN_TOOL.len()..];
                if !tools.iter().any(|t| t.name == name) {
                    return Err(bad(&format!("runs the tool \"{name}\", which isn't one of its [[tools]]")));
                }
            }
            None if !has_code => return Err(bad("needs a plugin.wasm (a [[tools]] or [importer]) to answer its action")),
            None => {}
        }
        if out.iter().any(|o| o.slot == c.slot && o.label == label) {
            return Err(bad("is declared twice in the same slot"));
        }
        out.push(PluginContribution { slot: c.slot, label: label.to_string(), icon: c.icon, action: c.action });
    }
    Ok(out)
}

/// `api = 1` (API 1.0) or `api = "1.1"` / `"1.1.0"`. Ordered, so `m.api <= API_VERSION` works.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ApiVersion {
    pub major: u32,
    pub minor: u32,
}

impl std::fmt::Display for ApiVersion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}", self.major, self.minor)
    }
}

#[derive(Deserialize)]
#[serde(untagged)]
enum RawApi {
    Major(u32),
    Text(String),
}

impl RawApi {
    fn parse(&self) -> Option<ApiVersion> {
        match self {
            RawApi::Major(m) => Some(ApiVersion { major: *m, minor: 0 }),
            RawApi::Text(t) => {
                let mut it = t.split('.');
                let major = it.next()?.parse().ok()?;
                let minor = it.next().map_or(Some(0), |m| m.parse().ok())?;
                // A patch (`1.1.0`) is allowed and ignored; anything after it isn't.
                if it.next().is_some_and(|p| p.parse::<u32>().is_err()) || it.next().is_some() {
                    return None;
                }
                Some(ApiVersion { major, minor })
            }
        }
    }
}

/// What a declarative provider stands in for (Round 24 §8.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ProviderKind {
    FilePicker,
    Reveal,
}

/// How a file-picker provider answers: one JSON line `{"paths":[…],"filter":n}` on stdout.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ProviderOutput {
    #[default]
    JsonStdout,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Provider {
    pub kind: ProviderKind,
    /// A `[[detect]]` id: only the executable it resolves is ever launched.
    pub app: String,
    pub args: Vec<String>,
    #[serde(default)]
    pub modes: BTreeMap<PickKind, String>,
    #[serde(default)]
    pub output: ProviderOutput,
    /// A bool setting that switches this provider on and off.
    #[serde(default)]
    pub enabled_by: Option<String>,
}

/// Placeholders a provider's `args` may use; Lumen fills them, never a shell.
pub const PLACEHOLDERS: [&str; 8] =
    ["mode", "title", "start", "name", "filters...", "parent", "path", "folder"];

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DetectWin {
    pub exe: String,
    #[serde(default)]
    pub paths: Vec<String>,
    #[serde(default)]
    pub path_env: bool,
    /// `HKCU/…` or `HKLM/…` keys whose default value is the exe's path (App Paths).
    #[serde(default)]
    pub registry: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DetectUnix {
    pub exe: String,
    #[serde(default)]
    pub paths: Vec<String>,
    #[serde(default)]
    pub path_env: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DetectMac {
    pub app: String,
    #[serde(default)]
    pub paths: Vec<String>,
}

/// An app the plugin works with, found by the host (Round 24 §8.2).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Detect {
    pub id: String,
    pub name: String,
    pub windows: Option<DetectWin>,
    pub linux: Option<DetectUnix>,
    pub macos: Option<DetectMac>,
    /// Arguments that print the version; the first line of stdout is shown.
    pub version: Option<Vec<String>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ModelKind {
    #[default]
    Onnx,
    /// A header + f16 matrix + f32 payload rows file; top-k runs in the host.
    Vectors,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum InputKind {
    /// The host decodes, resizes and normalises a photo into it (`run-on-asset`).
    Image,
    /// The plugin passes f32 values itself (`run`).
    Tensor,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Layout {
    Nchw,
    Nhwc,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InputSpec {
    pub kind: InputKind,
    pub size: Option<u32>,
    #[serde(default)]
    pub mean: Vec<f32>,
    #[serde(default)]
    pub std: Vec<f32>,
    pub layout: Option<Layout>,
}

/// A model the host downloads and runs for the plugin (Round 24 §8.4).
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelDecl {
    pub id: String,
    pub url: String,
    pub blake3: String,
    pub size: u64,
    pub licence: String,
    #[serde(default)]
    pub kind: ModelKind,
    #[serde(default)]
    pub inputs: BTreeMap<String, InputSpec>,
}

impl SettingDecl {
    /// Whether `v` is a value of this setting's type (a choice must be one of `choices`).
    pub fn accepts(&self, v: &serde_json::Value) -> bool {
        match self.kind {
            SettingKind::Bool => v.is_boolean(),
            SettingKind::Number => {
                v.is_number() && self.range.is_none_or(|(lo, hi)| v.as_f64().is_some_and(|n| n >= lo && n <= hi))
            }
            SettingKind::Text => v.is_string(),
            SettingKind::Choice => v
                .as_str()
                .is_some_and(|d| self.choices.iter().any(|c| c == d)),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Manifest {
    pub id: String,
    pub name: String,
    pub version: semver::Version,
    pub author: String,
    pub description: String,
    pub license: String,
    pub homepage: Option<String>,
    /// A link on the plugin's page from the author's name (https only).
    pub author_url: Option<String>,
    /// Where the source lives (https only).
    pub repository: Option<String>,
    /// `[listing] screenshots`, in order: paths under `screenshots/` the page shows.
    pub screenshots: Vec<String>,
    pub lumen: semver::VersionReq,
    pub api: ApiVersion,
    pub permissions: Vec<PluginPermission>,
    pub presets: Option<DirRef>,
    pub apps: Option<FileRef>,
    pub prompts: Vec<PromptDecl>,
    pub importer: Option<ImporterDecl>,
    pub tools: Vec<ToolDecl>,
    pub assets: Vec<AssetDecl>,
    pub settings: Vec<SettingDecl>,
    /// The settings page: flat `[[settings]]` first (a section per `group`), then `[[ui.section]]`
    /// (API 1.2). Every value control also appears in `settings`.
    pub sections: Vec<UiSection>,
    /// `[[ui.contribute]]` (API 1.2): entries in the map toolbar, menus and palette.
    pub contributions: Vec<PluginContribution>,
    // API 1.1 (Round 24 §8.1–§8.4).
    pub providers: Vec<Provider>,
    pub detect: Vec<Detect>,
    pub models: Vec<ModelDecl>,
}

/// `[listing]`: what the plugin's page and the store show before and after install.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Listing {
    #[serde(default)]
    screenshots: Vec<String>,
}

/// The most screenshots a plugin's page shows.
pub const MAX_SCREENSHOTS: usize = 6;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Raw {
    id: String,
    name: String,
    version: String,
    author: String,
    description: String,
    license: String,
    homepage: Option<String>,
    author_url: Option<String>,
    repository: Option<String>,
    listing: Option<Listing>,
    lumen: String,
    api: RawApi,
    #[serde(default)]
    permissions: Vec<String>,
    #[serde(default)]
    providers: Vec<Provider>,
    #[serde(default)]
    detect: Vec<Detect>,
    #[serde(default)]
    models: Vec<ModelDecl>,
    presets: Option<DirRef>,
    apps: Option<FileRef>,
    #[serde(default)]
    prompts: Vec<PromptDecl>,
    importer: Option<ImporterDecl>,
    #[serde(default)]
    tools: Vec<ToolDecl>,
    #[serde(default)]
    assets: Vec<AssetDecl>,
    #[serde(default)]
    settings: Vec<SettingDecl>,
    ui: Option<RawUi>,
}

/// A flat `[[settings]]` entry as a control, so one renderer draws both forms.
fn control_of(d: &SettingDecl) -> UiControl {
    let (kind, number) = match d.kind {
        SettingKind::Bool => (ControlKind::Toggle, false),
        SettingKind::Number => (ControlKind::Number, true),
        SettingKind::Text => (ControlKind::Text, false),
        SettingKind::Choice => (ControlKind::Select, false),
    };
    UiControl {
        kind,
        key: Some(d.key.clone()),
        label: d.label.clone(),
        help: None,
        default: Some(d.default.clone()),
        choices: d.choices.clone(),
        min: number.then(|| d.range.map(|r| r.0)).flatten(),
        max: number.then(|| d.range.map(|r| r.1)).flatten(),
        step: None,
        requires: None,
        app: d.requires.clone(),
        action: None,
        body: None,
    }
}

/// The flat settings as sections: one per run of the same `group`.
fn legacy_sections(settings: &[SettingDecl]) -> Vec<UiSection> {
    let mut out: Vec<UiSection> = Vec::new();
    for d in settings {
        if out.last().is_none_or(|s| s.title != d.group) {
            out.push(UiSection { title: d.group.clone(), ..UiSection::default() });
        }
        out.last_mut().expect("pushed above").controls.push(control_of(d));
    }
    out
}

/// Checks the `[ui]` tables (API 1.2) and returns the settings their value controls declare.
fn check_ui(
    sections: &[UiSection],
    taken: &mut std::collections::BTreeSet<String>,
    detect: &[Detect],
    has_code: bool,
) -> Result<Vec<SettingDecl>, PluginError> {
    let mut out = Vec::new();
    // `requires` may point forward, so collect every value key first.
    let value_keys: std::collections::BTreeSet<&str> = sections
        .iter()
        .flat_map(|s| &s.controls)
        .filter(|c| c.kind.is_value())
        .filter_map(|c| c.key.as_deref())
        .chain(taken.iter().map(String::as_str))
        .collect();
    let mut new_keys = Vec::new();
    for s in sections {
        let heading = s.title.as_deref().unwrap_or("untitled");
        for c in &s.controls {
            let name = c.key.as_deref().unwrap_or(c.label.as_str());
            let bad = |why: &str| invalid(format!("the control \"{name}\" in section \"{heading}\" {why}"));
            if c.label.trim().is_empty() {
                return Err(bad("needs a label"));
            }
            if c.kind.needs_code() && !has_code {
                return Err(bad("needs a plugin.wasm (a [[tools]] or [importer]) to answer it"));
            }
            if c.kind.is_value() || matches!(c.kind, ControlKind::Stat | ControlKind::List) {
                let key = c.key.as_deref().filter(|k| !k.is_empty()).ok_or_else(|| bad("needs a key"))?;
                if taken.contains(key) || new_keys.contains(&key) {
                    return Err(invalid(format!("the setting key \"{key}\" is empty or declared twice")));
                }
                new_keys.push(key);
            }
            if c.kind == ControlKind::Button && c.action.as_deref().is_none_or(str::is_empty) {
                return Err(bad("needs an action"));
            }
            if c.kind == ControlKind::Markdown && c.body.as_deref().is_none_or(|b| b.trim().is_empty()) {
                return Err(bad("needs a body"));
            }
            if let Some(r) = &c.requires {
                if c.key.as_deref() == Some(r.as_str()) || !value_keys.contains(r.as_str()) {
                    return Err(bad(&format!("requires \"{r}\", which isn't another control's key")));
                }
            }
            if let Some(a) = &c.app {
                if !detect.iter().any(|d| &d.id == a) {
                    return Err(bad(&format!("names the app \"{a}\", which no [[detect]] declares")));
                }
            }
            if !c.kind.is_value() {
                continue;
            }
            let (kind, range) = match c.kind {
                ControlKind::Toggle => (SettingKind::Bool, None),
                ControlKind::Text => (SettingKind::Text, None),
                ControlKind::Select | ControlKind::Segmented => {
                    if c.choices.is_empty() {
                        return Err(bad("needs choices"));
                    }
                    (SettingKind::Choice, None)
                }
                ControlKind::Slider => match (c.min, c.max) {
                    (Some(lo), Some(hi)) if lo < hi => (SettingKind::Number, Some((lo, hi))),
                    _ => return Err(bad("needs a min below its max")),
                },
                _ => match (c.min, c.max) {
                    (Some(lo), Some(hi)) if lo > hi => return Err(bad("has a min above its max")),
                    (lo, hi) => (SettingKind::Number, Some((lo.unwrap_or(f64::MIN), hi.unwrap_or(f64::MAX)))),
                },
            };
            let default = c.default.clone().ok_or_else(|| bad("needs a default"))?;
            let decl = SettingDecl {
                key: c.key.clone().unwrap_or_default(),
                label: c.label.clone(),
                kind,
                default,
                choices: c.choices.clone(),
                group: s.title.clone(),
                requires: c.app.clone(),
                range,
            };
            if !decl.accepts(&decl.default) {
                return Err(bad("has a default that doesn't match its type or bounds"));
            }
            out.push(decl);
        }
    }
    taken.extend(new_keys.into_iter().map(str::to_string));
    Ok(out)
}

impl Manifest {
    /// The kinds this manifest declares, in [`PluginKind`] order.
    pub fn kinds(&self) -> Vec<PluginKind> {
        let mut k = Vec::new();
        if self.presets.is_some() {
            k.push(PluginKind::Presets);
        }
        if self.apps.is_some() {
            k.push(PluginKind::Apps);
        }
        if !self.prompts.is_empty() {
            k.push(PluginKind::Prompts);
        }
        if self.importer.is_some() {
            k.push(PluginKind::Importer);
        }
        if !self.tools.is_empty() {
            k.push(PluginKind::Tools);
        }
        k
    }

    /// The last segment of the id (`studio.jl.takeout` → `takeout`): the MCP tool prefix.
    pub fn short_name(&self) -> &str {
        self.id.rsplit('.').next().unwrap_or(&self.id)
    }

    /// True when the manifest declares a kind that runs code (`plugin.wasm`).
    pub fn has_code(&self) -> bool {
        self.importer.is_some() || !self.tools.is_empty()
    }

    /// The install sheet's *Adds* lines for `[[providers]]` (Round 24 §8.1): *Open Disk Atlas
    /// for file dialogs*, *Open Disk Atlas for Show in folder*.
    pub fn provider_adds(&self) -> Vec<String> {
        self.providers
            .iter()
            .map(|p| {
                let app = self.detect.iter().find(|d| d.id == p.app).map_or(p.app.as_str(), |d| d.name.as_str());
                match p.kind {
                    ProviderKind::FilePicker => format!("Open {app} for file dialogs"),
                    ProviderKind::Reveal => format!("Open {app} for Show in folder"),
                }
            })
            .collect()
    }
}

fn invalid(msg: impl Into<String>) -> PluginError {
    PluginError::invalid(format!("plugin.toml: {}", msg.into()))
}

pub(crate) fn valid_id(id: &str) -> bool {
    (3..=64).contains(&id.len())
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'.' || b == b'-')
}

fn valid_tool_name(name: &str) -> bool {
    (1..=40).contains(&name.len())
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}

/// A bare file name: no folder, no `..`.
fn bare_file_name(s: &str) -> bool {
    !s.is_empty() && !s.contains(['/', '\\', ':']) && s != "." && s != ".."
}

fn check_link(key: &str, url: &Option<String>) -> Result<(), PluginError> {
    match url {
        Some(u) if !(u.starts_with("https://") && u.len() > "https://".len() && !u.contains(char::is_whitespace)) => {
            Err(invalid(format!("{key} must be an https:// link")))
        }
        _ => Ok(()),
    }
}

fn check_screenshots(paths: &[String]) -> Result<(), PluginError> {
    if paths.len() > MAX_SCREENSHOTS {
        return Err(invalid(format!("[listing] screenshots has {}; at most {MAX_SCREENSHOTS}", paths.len())));
    }
    let mut seen = std::collections::BTreeSet::new();
    for p in paths {
        let name = p.strip_prefix("screenshots/").unwrap_or("");
        let ext = name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase());
        if !bare_file_name(name) || !matches!(ext.as_deref(), Some("png" | "jpg" | "jpeg")) || !seen.insert(p) {
            return Err(invalid(format!(
                "the screenshot \"{p}\" must be a .png or .jpg file directly under screenshots/, listed once"
            )));
        }
    }
    Ok(())
}

fn check_detect(detect: &[Detect]) -> Result<(), PluginError> {
    let mut ids = std::collections::BTreeSet::new();
    for d in detect {
        if !valid_id(&d.id) || !ids.insert(d.id.as_str()) {
            return Err(invalid(format!(
                "the detect id \"{}\" must be 3 to 64 characters of a-z, 0-9, '.' and '-', declared once",
                d.id
            )));
        }
        let exes = [
            d.windows.as_ref().map(|w| w.exe.as_str()),
            d.linux.as_ref().map(|l| l.exe.as_str()),
            d.macos.as_ref().map(|m| m.app.as_str()),
        ];
        if exes.iter().all(Option::is_none) {
            return Err(invalid(format!("the detect \"{}\" names no app for any system", d.id)));
        }
        if let Some(bad) = exes.into_iter().flatten().find(|e| !bare_file_name(e)) {
            return Err(invalid(format!(
                "the detect \"{}\" names \"{bad}\": give a file name, the folders go in paths",
                d.id
            )));
        }
    }
    Ok(())
}

/// Every `{…}` in an argument must be a known placeholder, and `{filters...}` stands alone.
fn check_arg(arg: &str) -> Result<(), String> {
    let mut rest = arg;
    while let Some(open) = rest.find('{') {
        let after = &rest[open + 1..];
        let close = after.find('}').ok_or_else(|| format!("\"{arg}\" opens a placeholder it never closes"))?;
        let name = &after[..close];
        if !PLACEHOLDERS.contains(&name) {
            return Err(format!("\"{{{name}}}\" isn't a placeholder Lumen fills"));
        }
        if name == "filters..." && arg != "{filters...}" {
            return Err("\"{filters...}\" must be an argument of its own".into());
        }
        rest = &after[close + 1..];
    }
    Ok(())
}

fn check_providers(providers: &[Provider], detect: &[Detect], settings: &[SettingDecl]) -> Result<(), PluginError> {
    for p in providers {
        if !detect.iter().any(|d| d.id == p.app) {
            return Err(invalid(format!(
                "a provider uses the app \"{}\", which no [[detect]] declares",
                p.app
            )));
        }
        for a in &p.args {
            check_arg(a).map_err(|m| invalid(format!("a provider's args: {m}")))?;
        }
        if p.kind == ProviderKind::FilePicker && p.modes.is_empty() {
            return Err(invalid("a file-picker provider needs modes"));
        }
        if p.modes.values().any(|w| w.is_empty()) {
            return Err(invalid("a provider's mode can't be empty"));
        }
        if let Some(key) = &p.enabled_by {
            if !settings.iter().any(|s| &s.key == key && s.kind == SettingKind::Bool) {
                return Err(invalid(format!(
                    "a provider is enabled by \"{key}\", which isn't a bool [[settings]] key"
                )));
            }
        }
    }
    Ok(())
}

fn check_models(models: &[ModelDecl], permissions: &[PluginPermission]) -> Result<(), PluginError> {
    if !models.is_empty() && !permissions.contains(&PluginPermission::Models) {
        return Err(PluginError::permission("[[models]] needs the permission \"models\""));
    }
    let mut ids = std::collections::BTreeSet::new();
    for m in models {
        if !valid_tool_name(&m.id) || !ids.insert(m.id.as_str()) {
            return Err(invalid(format!(
                "the model id \"{}\" must match [a-z0-9_]{{1,40}} and be declared once",
                m.id
            )));
        }
        if !m.url.starts_with("https://") {
            return Err(invalid(format!("the model \"{}\" must be downloaded over https", m.id)));
        }
        if m.blake3.len() != 64 || !m.blake3.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(invalid(format!("the model \"{}\" needs a blake3 of 64 hex digits", m.id)));
        }
        if m.licence.trim().is_empty() {
            return Err(invalid(format!("the model \"{}\" needs a licence", m.id)));
        }
        if m.kind == ModelKind::Vectors && !m.inputs.is_empty() {
            return Err(invalid(format!("the vectors model \"{}\" takes no inputs", m.id)));
        }
        for (name, i) in &m.inputs {
            let rgb = |v: &Vec<f32>| v.is_empty() || v.len() == 3;
            if i.kind == InputKind::Image && (i.size.is_none_or(|s| s == 0) || !rgb(&i.mean) || !rgb(&i.std)) {
                return Err(invalid(format!(
                    "the image input \"{name}\" of model \"{}\" needs a size and three-value mean / std",
                    m.id
                )));
            }
            if i.std.contains(&0.0) {
                return Err(invalid(format!("the input \"{name}\" has a std of 0")));
            }
        }
    }
    Ok(())
}

pub fn parse(text: &str) -> Result<Manifest, PluginError> {
    // API 1.4 (Round 26): themes are `.lumentheme` files, never part of a plugin.
    if text.lines().any(|l| l.trim_start().starts_with("[[themes]]")) {
        return Err(invalid(
            "[[themes]] left plugins in API 1.4: make the theme a .lumentheme (see docs/THEMES.md)",
        ));
    }
    let raw: Raw = toml::from_str(text).map_err(|e| invalid(e.message().to_string()))?;
    if !valid_id(&raw.id) {
        return Err(invalid(format!(
            "the id \"{}\" must be 3 to 64 characters of a-z, 0-9, '.' and '-'",
            raw.id
        )));
    }
    let version = semver::Version::parse(&raw.version)
        .map_err(|e| invalid(format!("version \"{}\" is not semver: {e}", raw.version)))?;
    let lumen = semver::VersionReq::parse(&raw.lumen).map_err(|e| {
        invalid(format!(
            "lumen \"{}\" is not a version range: {e}",
            raw.lumen
        ))
    })?;
    let api = raw.api.parse().ok_or_else(|| {
        invalid("api must be a number (1) or a version like \"1.1\"")
    })?;
    let mut permissions = Vec::new();
    for p in &raw.permissions {
        let perm = PluginPermission::from_id(p)
            .ok_or_else(|| PluginError::permission(format!("Unknown permission \"{p}\"")))?;
        if api.major == 1 && api.minor < perm.since_minor() {
            return Err(invalid(format!(
                "the permission \"{p}\" needs api = \"1.{}\"",
                perm.since_minor()
            )));
        }
        if !permissions.contains(&perm) {
            permissions.push(perm);
        }
    }
    let v11 = api >= ApiVersion { major: 1, minor: 1 };
    if !v11 && !(raw.providers.is_empty() && raw.detect.is_empty() && raw.models.is_empty()) {
        return Err(invalid("[[providers]], [[detect]] and [[models]] need api = \"1.1\""));
    }
    check_link("author_url", &raw.author_url)?;
    check_link("repository", &raw.repository)?;
    let screenshots = raw.listing.map(|l| l.screenshots).unwrap_or_default();
    check_screenshots(&screenshots)?;
    check_detect(&raw.detect)?;
    if raw.ui.is_some() && api < (ApiVersion { major: 1, minor: 2 }) {
        return Err(invalid("[ui] needs api = \"1.2\""));
    }
    check_models(&raw.models, &permissions)?;
    let mut names = std::collections::BTreeSet::new();
    for t in &raw.tools {
        if !valid_tool_name(&t.name) {
            return Err(invalid(format!(
                "the tool name \"{}\" must match [a-z0-9_]{{1,40}}",
                t.name
            )));
        }
        if !names.insert(t.name.as_str()) {
            return Err(invalid(format!(
                "the tool \"{}\" is declared twice",
                t.name
            )));
        }
    }
    let mut keys = std::collections::BTreeSet::new();
    for s in &raw.settings {
        if s.key.is_empty() || !keys.insert(s.key.as_str()) {
            return Err(invalid(format!(
                "the setting key \"{}\" is empty or declared twice",
                s.key
            )));
        }
        if !s.accepts(&s.default) {
            return Err(invalid(format!(
                "the default of setting \"{}\" doesn't match its type (a choice needs choices that include it)",
                s.key
            )));
        }
        if let Some(r) = &s.requires {
            if !raw.detect.iter().any(|d| &d.id == r) {
                return Err(invalid(format!(
                    "the setting \"{}\" requires \"{r}\", which no [[detect]] declares",
                    s.key
                )));
            }
        }
    }
    let mut taken: std::collections::BTreeSet<String> = keys.iter().map(|k| k.to_string()).collect();
    let mut settings = raw.settings;
    let mut sections = legacy_sections(&settings);
    let mut contributions = Vec::new();
    if let Some(ui) = raw.ui {
        let has_code = raw.importer.is_some() || !raw.tools.is_empty();
        settings.extend(check_ui(&ui.section, &mut taken, &raw.detect, has_code)?);
        sections.extend(ui.section);
        contributions = check_contributions(ui.contribute, has_code, &raw.tools)?;
    }
    check_providers(&raw.providers, &raw.detect, &settings)?;
    Ok(Manifest {
        id: raw.id,
        name: raw.name,
        version,
        author: raw.author,
        description: raw.description,
        license: raw.license,
        homepage: raw.homepage,
        author_url: raw.author_url,
        repository: raw.repository,
        screenshots,
        lumen,
        api,
        permissions,
        providers: raw.providers,
        detect: raw.detect,
        models: raw.models,
        presets: raw.presets,
        apps: raw.apps,
        prompts: raw.prompts,
        importer: raw.importer,
        tools: raw.tools,
        assets: raw.assets,
        settings,
        sections,
        contributions,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{PluginKind, PluginPermission};

    pub(crate) const FULL: &str = r#"
id = "studio.jl.takeout"
name = "Google Takeout"
version = "1.0.0"
author = "JL Studios"
description = "Imports a Google Photos Takeout with its dates, places and albums."
license = "MIT"
homepage = "https://example.com"
lumen = ">=26.2.0"
api = 1
permissions = ["import.read-chosen"]


[presets]
dir = "presets"

[apps]
file = "apps.toml"

[[prompts]]
name = "film-look"
description = "Retouch toward a warm film look"
file = "prompts/film-look.md"

[importer]
label = "Google Takeout"
pick = "folder"
extensions = ["zip"]
hint = "Choose the folder Google Takeout made"

[[tools]]
name = "level_horizon"
description = "Levels the horizon"

[[assets]]
path = "assets/fonts/Lora.woff2"
license = "OFL-1.1"

[[settings]]
key = "albums_from_folders"
label = "Use Takeout's folders as albums"
type = "bool"
default = true
"#;

    const MIN: &str = r#"
id = "a.b"
name = "A"
version = "1.0.0"
author = "Me"
description = "d"
license = "MIT"
lumen = ">=26.0.0"
api = 1
"#;

    fn with(base: &str, key: &str, value: &str) -> String {
        base.lines()
            .map(|l| {
                if l.starts_with(&format!("{key} =")) {
                    format!("{key} = {value}")
                } else {
                    l.to_string()
                }
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn parses_spec_example() {
        let m = parse(FULL).unwrap();
        assert_eq!(
            m.kinds(),
            vec![
                PluginKind::Presets,
                PluginKind::Apps,
                PluginKind::Prompts,
                PluginKind::Importer,
                PluginKind::Tools
            ]
        );
        assert_eq!(m.short_name(), "takeout");
        assert_eq!(m.permissions, vec![PluginPermission::ImportReadChosen]);
        assert_eq!(m.version, semver::Version::new(1, 0, 0));
        assert_eq!(m.settings[0].default, serde_json::json!(true));
        assert_eq!(m.importer.as_ref().unwrap().pick, crate::Pick::Folder);
        assert!(parse(MIN).unwrap().kinds().is_empty());
    }

    #[test]
    fn unknown_top_level_key_errors() {
        // Before the first table, or TOML would read it as the table's key.
        let e = parse(&format!("colour = 1\n{FULL}")).unwrap_err();
        assert_eq!(e.code, "plugin_invalid");
        assert!(e.message.contains("colour"), "{}", e.message);
    }

    #[test]
    fn id_rules() {
        let long = format!("\"{}\"", "a".repeat(65));
        for bad in ["\"ab\"", "\"Studio.X\"", "\"a b.c\"", long.as_str()] {
            let e = parse(&with(MIN, "id", bad)).unwrap_err();
            assert_eq!(e.code, "plugin_invalid", "{bad}");
        }
        assert!(parse(&with(MIN, "id", "\"a.b\"")).is_ok());
    }

    #[test]
    fn tool_name_rules() {
        let long = "a".repeat(41);
        for bad in ["Level", "a-b", long.as_str()] {
            let t = format!("{MIN}\n[[tools]]\nname = \"{bad}\"\ndescription = \"x\"\n");
            assert_eq!(parse(&t).unwrap_err().code, "plugin_invalid", "{bad}");
        }
        assert!(parse(&format!(
            "{MIN}\n[[tools]]\nname = \"a_1\"\ndescription = \"x\"\n"
        ))
        .is_ok());
    }

    #[test]
    fn bad_semver_and_range() {
        assert_eq!(
            parse(&with(MIN, "version", "\"1\"")).unwrap_err().code,
            "plugin_invalid"
        );
        assert_eq!(
            parse(&with(MIN, "lumen", "\"soon\"")).unwrap_err().code,
            "plugin_invalid"
        );
    }

    #[test]
    fn unknown_permission_errors() {
        let e = parse(&format!("permissions = [\"network\"]\n{MIN}")).unwrap_err();
        assert_eq!(e.code, "plugin_permission");
        assert!(e.message.contains("network"));
    }

    #[test]
    fn setting_default_matches_kind() {
        let s = |body: &str| format!("{MIN}\n[[settings]]\nkey = \"k\"\nlabel = \"K\"\n{body}\n");
        assert_eq!(
            parse(&s("type = \"bool\"\ndefault = \"yes\""))
                .unwrap_err()
                .code,
            "plugin_invalid"
        );
        assert_eq!(
            parse(&s("type = \"choice\"\ndefault = \"a\""))
                .unwrap_err()
                .code,
            "plugin_invalid"
        );
        assert_eq!(
            parse(&s(
                "type = \"choice\"\ndefault = \"c\"\nchoices = [\"a\", \"b\"]"
            ))
            .unwrap_err()
            .code,
            "plugin_invalid"
        );
        assert!(parse(&s(
            "type = \"choice\"\ndefault = \"a\"\nchoices = [\"a\", \"b\"]"
        ))
        .is_ok());
        assert!(parse(&s("type = \"number\"\ndefault = 2.5")).is_ok());
        assert!(parse(&s("type = \"text\"\ndefault = \"\"")).is_ok());
    }

    // ---- API 1.1 (Round 24 §8) ----

    const MIN11: &str = r#"
id = "jl.disk-atlas"
name = "Disk Atlas"
version = "1.0.0"
author = "JL Studios"
description = "d"
license = "MIT"
lumen = ">=26.2.0"
api = "1.1.0"
"#;

    const DETECT: &str = r#"
[[detect]]
id = "disk-atlas"
name = "Disk Atlas"
windows = { exe = "disk-atlas.exe", paths = ["%LOCALAPPDATA%/Programs/Disk Atlas", "%ProgramFiles%/Disk Atlas"], path_env = true, registry = ["HKCU/Software/Microsoft/Windows/CurrentVersion/App Paths/disk-atlas.exe"] }
linux   = { exe = "disk-atlas", path_env = true }
macos   = { app = "Disk Atlas.app" }
version = ["--version"]
"#;

    const PICKER: &str = r#"
[[providers]]
kind = "file-picker"
app = "disk-atlas"
args = ["--pick", "{mode}", "--title", "{title}", "--start", "{start}", "--name", "{name}", "{filters...}", "--parent", "{parent}"]
modes = { open-file = "open", open-multiple = "open-multiple", open-folder = "folder", save = "save" }
output = "json-stdout"
enabled_by = "use_picker"

[[providers]]
kind = "reveal"
app = "disk-atlas"
args = ["--select", "{path}"]
enabled_by = "use_picker"

[[settings]]
key = "use_picker"
label = "Use Disk Atlas for file dialogs"
type = "bool"
default = true
requires = "disk-atlas"
group = "Disk Atlas"
"#;

    const MODELS: &str = r#"
permissions = ["models", "library.read", "places.write", "originals.gps"]
"#;

    const MODEL_DECLS: &str = r#"
[[models]]
id = "image"
url = "https://example.com/wayfinder-image-fp16.onnx"
blake3 = "0000000000000000000000000000000000000000000000000000000000000000"
size = 0
licence = "CC-BY-NC-4.0"
inputs = { pixels = { kind = "image", size = 224, mean = [0.481, 0.458, 0.408], std = [0.269, 0.261, 0.276], layout = "nchw" } }

[[models]]
id = "gallery"
url = "https://example.com/wayfinder-gallery.bin"
blake3 = "1111111111111111111111111111111111111111111111111111111111111111"
size = 205000000
licence = "CC-BY-NC-4.0"
kind = "vectors"
"#;

    #[test]
    fn parses_providers() {
        let m = parse(&format!("{MIN11}{DETECT}{PICKER}")).unwrap();
        assert_eq!(m.api, ApiVersion { major: 1, minor: 1 });
        assert_eq!(m.providers.len(), 2);
        let p = &m.providers[0];
        assert_eq!(p.kind, ProviderKind::FilePicker);
        assert_eq!(p.output, ProviderOutput::JsonStdout);
        assert_eq!(p.modes[&PickKind::OpenFolder], "folder");
        assert_eq!(p.modes.len(), 4);
        assert_eq!(p.enabled_by.as_deref(), Some("use_picker"));
        assert_eq!(m.providers[1].kind, ProviderKind::Reveal);
        assert!(m.providers[1].modes.is_empty());
        assert!(!m.has_code());
        assert_eq!(m.provider_adds(), ["Open Disk Atlas for file dialogs", "Open Disk Atlas for Show in folder"]);
    }

    #[test]
    fn refuses_unknown_placeholder() {
        for bad in ["{shell}", "--x={home}", "{path", "x{filters...}"] {
            let t = format!(
                "{MIN11}{DETECT}\n[[providers]]\nkind = \"reveal\"\napp = \"disk-atlas\"\nargs = [\"{bad}\"]\n"
            );
            assert_eq!(parse(&t).unwrap_err().code, "plugin_invalid", "{bad}");
        }
        let ok = format!(
            "{MIN11}{DETECT}\n[[providers]]\nkind = \"reveal\"\napp = \"disk-atlas\"\nargs = [\"--in={{folder}}\"]\n"
        );
        assert!(parse(&ok).is_ok());
    }

    #[test]
    fn provider_needs_detect() {
        let e = parse(&format!("{MIN11}{}", PICKER.replace("requires = \"disk-atlas\"\n", ""))).unwrap_err();
        assert_eq!(e.code, "plugin_invalid");
        assert!(e.message.contains("disk-atlas"), "{}", e.message);
        // enabled_by must name a bool setting.
        let t = format!("{MIN11}{DETECT}{}", PICKER.replace("enabled_by = \"use_picker\"\n\n[[providers]]", "enabled_by = \"nope\"\n\n[[providers]]"));
        assert_eq!(parse(&t).unwrap_err().code, "plugin_invalid");
        // A file-picker needs modes.
        let t = format!("{MIN11}{DETECT}\n[[providers]]\nkind = \"file-picker\"\napp = \"disk-atlas\"\nargs = []\n");
        assert_eq!(parse(&t).unwrap_err().code, "plugin_invalid");
    }

    #[test]
    fn parses_detect() {
        let m = parse(&format!("{MIN11}{DETECT}")).unwrap();
        let d = &m.detect[0];
        assert_eq!(d.id, "disk-atlas");
        let w = d.windows.as_ref().unwrap();
        assert_eq!(w.exe, "disk-atlas.exe");
        assert_eq!(w.paths.len(), 2);
        assert!(w.path_env);
        assert_eq!(w.registry.len(), 1);
        assert!(d.linux.as_ref().unwrap().path_env);
        assert_eq!(d.macos.as_ref().unwrap().app, "Disk Atlas.app");
        assert_eq!(d.version.as_deref(), Some(&["--version".to_string()][..]));
        // An exe with a folder in it is refused.
        let bad = DETECT.replace("exe = \"disk-atlas\",", "exe = \"bin/disk-atlas\",");
        assert_eq!(parse(&format!("{MIN11}{bad}")).unwrap_err().code, "plugin_invalid");
        // Detect needs api 1.1.
        assert_eq!(parse(&format!("{MIN}{DETECT}")).unwrap_err().code, "plugin_invalid");
    }

    #[test]
    fn parses_models_onnx_and_vectors() {
        let m = parse(&format!("{MODELS}{MIN11}{MODEL_DECLS}")).unwrap();
        assert_eq!(m.models[0].kind, ModelKind::Onnx);
        let px = &m.models[0].inputs["pixels"];
        assert_eq!((px.kind, px.size, px.layout), (InputKind::Image, Some(224), Some(Layout::Nchw)));
        assert_eq!(px.mean.len(), 3);
        assert_eq!(m.models[1].kind, ModelKind::Vectors);
        assert_eq!(m.models[1].size, 205_000_000);
        // Without the permission, a bad hash, plain http.
        assert_eq!(parse(&format!("{MIN11}{MODEL_DECLS}")).unwrap_err().code, "plugin_permission");
        let short = MODEL_DECLS.replacen("0000000000000000", "", 1);
        assert_eq!(parse(&format!("{MODELS}{MIN11}{short}")).unwrap_err().code, "plugin_invalid");
        let http = MODEL_DECLS.replacen("https://", "http://", 1);
        assert_eq!(parse(&format!("{MODELS}{MIN11}{http}")).unwrap_err().code, "plugin_invalid");
    }

    #[test]
    fn settings_requires_detect() {
        let t = format!("{MIN11}\n[[settings]]\nkey = \"k\"\nlabel = \"K\"\ntype = \"bool\"\ndefault = true\nrequires = \"disk-atlas\"\n");
        assert_eq!(parse(&t).unwrap_err().code, "plugin_invalid");
        let m = parse(&format!("{MIN11}{DETECT}{PICKER}")).unwrap();
        assert_eq!(m.settings[0].requires.as_deref(), Some("disk-atlas"));
        assert_eq!(m.settings[0].group.as_deref(), Some("Disk Atlas"));
    }

    #[test]
    fn new_permissions_parse() {
        let m = parse(&format!("{MODELS}{MIN11}")).unwrap();
        assert_eq!(
            m.permissions,
            vec![
                PluginPermission::Models,
                PluginPermission::LibraryRead,
                PluginPermission::PlacesWrite,
                PluginPermission::OriginalsGps
            ]
        );
        for p in PluginPermission::ALL {
            assert_eq!(PluginPermission::from_id(p.id()), Some(p));
            assert!(!p.label().is_empty());
        }
        // A 1.0 manifest can't ask for them.
        let e = parse(&format!("permissions = [\"models\"]\n{MIN}")).unwrap_err();
        assert_eq!(e.code, "plugin_invalid");
        assert!(e.message.contains("1.1"), "{}", e.message);
    }

    #[test]
    fn api_versions() {
        for (text, want) in [("1", (1, 0)), ("\"1.1\"", (1, 1)), ("\"1.1.0\"", (1, 1)), ("2", (2, 0))] {
            let m = parse(&with(MIN, "api", text)).unwrap();
            assert_eq!((m.api.major, m.api.minor), want, "{text}");
        }
        for bad in ["\"one\"", "\"1.x\"", "\"1.1.0.0\"", "\"1.1.beta\""] {
            assert_eq!(parse(&with(MIN, "api", bad)).unwrap_err().code, "plugin_invalid", "{bad}");
        }
        assert!(ApiVersion { major: 1, minor: 0 } < crate::API_VERSION);
        assert!(ApiVersion { major: 2, minor: 0 } > crate::API_VERSION);
    }

    #[test]
    fn api_1_0_manifest_unchanged() {
        let m = parse(FULL).unwrap();
        assert_eq!(m.api, ApiVersion { major: 1, minor: 0 });
        assert!(m.providers.is_empty() && m.detect.is_empty() && m.models.is_empty());
        assert_eq!((m.settings[0].group.clone(), m.settings[0].requires.clone()), (None, None));
    }

    #[test]
    fn listing_fields_parse_and_default_to_nothing() {
        let bare = parse(MIN).unwrap();
        assert_eq!((bare.author_url, bare.repository, bare.screenshots), (None, None, vec![]));
        let m = parse(&format!(
            "{MIN}author_url = \"https://example.com/me\"\nrepository = \"https://github.com/me/x\"\n\
             \n[listing]\nscreenshots = [\"screenshots/a.png\", \"screenshots/b.jpg\"]\n"
        ))
        .unwrap();
        assert_eq!(m.author_url.as_deref(), Some("https://example.com/me"));
        assert_eq!(m.repository.as_deref(), Some("https://github.com/me/x"));
        assert_eq!(m.screenshots, ["screenshots/a.png", "screenshots/b.jpg"]);
    }

    // API 1.2: declarative settings pages (Round 25 P3).

    const MIN12: &str = r#"
id = "jl.ui"
name = "Ui"
version = "1.0.0"
author = "JL Studios"
description = "d"
license = "MIT"
lumen = ">=26.4.0"
api = "1.2"
"#;

    const SECTIONS: &str = r#"
[[ui.section]]
title = "Scanning"
description = "How a scan runs."

[[ui.section.control]]
type = "toggle"
key = "deep"
label = "Deep scan"
help = "Reads every file."
default = false

[[ui.section.control]]
type = "slider"
key = "depth"
label = "Depth"
default = 3
min = 1
max = 8
step = 1
requires = "deep"

[[ui.section.control]]
type = "segmented"
key = "unit"
label = "Units"
default = "mb"
choices = ["mb", "gb"]

[[ui.section]]
title = "Tools"

[[ui.section.control]]
type = "stat"
key = "last_scan"
label = "Last scan"

[[ui.section.control]]
type = "button"
label = "Scan now"
action = "scan"

[[ui.section.control]]
type = "markdown"
label = "About"
body = "Reads **only** the folder you pick."
"#;

    fn ui_manifest(extra: &str) -> String {
        format!("{MIN12}\n[[tools]]\nname = \"t\"\ndescription = \"x\"\n{extra}")
    }

    #[test]
    fn api_1_2_parses_sections_and_controls() {
        let m = parse(&ui_manifest(SECTIONS)).unwrap();
        assert_eq!(m.api, ApiVersion { major: 1, minor: 2 });
        assert_eq!(m.sections.len(), 2);
        let s = &m.sections[0];
        assert_eq!((s.title.as_deref(), s.description.as_deref()), (Some("Scanning"), Some("How a scan runs.")));
        assert_eq!(s.controls[0].kind, ControlKind::Toggle);
        assert_eq!(s.controls[0].help.as_deref(), Some("Reads every file."));
        assert_eq!(s.controls[1].requires.as_deref(), Some("deep"));
        assert_eq!(s.controls[1].min, Some(1.0));
        let tools = &m.sections[1].controls;
        assert_eq!(tools.iter().map(|c| c.kind).collect::<Vec<_>>(), [ControlKind::Stat, ControlKind::Button, ControlKind::Markdown]);
        assert_eq!(tools[1].action.as_deref(), Some("scan"));
        assert_eq!(tools[2].body.as_deref(), Some("Reads **only** the folder you pick."));
    }

    #[test]
    fn value_controls_become_settings_with_their_defaults() {
        let m = parse(&ui_manifest(SECTIONS)).unwrap();
        let keys: Vec<_> = m.settings.iter().map(|d| (d.key.as_str(), d.kind)).collect();
        assert_eq!(
            keys,
            [("deep", SettingKind::Bool), ("depth", SettingKind::Number), ("unit", SettingKind::Choice)]
        );
        let depth = &m.settings[1];
        assert!(depth.accepts(&serde_json::json!(4)));
        assert!(!depth.accepts(&serde_json::json!(9)), "above max");
        assert!(!depth.accepts(&serde_json::json!(0)), "below min");
        assert_eq!(m.settings[0].group.as_deref(), Some("Scanning"));
    }

    #[test]
    fn flat_settings_read_as_one_untitled_section() {
        let m = parse(FULL).unwrap();
        assert_eq!(m.sections.len(), 1);
        let s = &m.sections[0];
        assert_eq!((s.title.clone(), s.description.clone()), (None, None));
        assert_eq!(s.controls.len(), 1);
        assert_eq!(s.controls[0].kind, ControlKind::Toggle);
        assert_eq!(s.controls[0].key.as_deref(), Some("albums_from_folders"));
        assert_eq!(s.controls[0].default, Some(serde_json::json!(true)));
        // No settings, no sections.
        assert!(parse(MIN).unwrap().sections.is_empty());
    }

    #[test]
    fn flat_settings_group_into_titled_sections_and_keep_their_app() {
        let m = parse(&format!("{MIN11}{DETECT}{PICKER}")).unwrap();
        assert_eq!(m.sections[0].title.as_deref(), Some("Disk Atlas"));
        assert_eq!(m.sections[0].controls[0].app.as_deref(), Some("disk-atlas"));
    }

    #[test]
    fn flat_settings_come_before_ui_sections() {
        let flat = "\n[[settings]]\nkey = \"old\"\nlabel = \"Old\"\ntype = \"bool\"\ndefault = true\n";
        let m = parse(&ui_manifest(&format!("{flat}{SECTIONS}"))).unwrap();
        assert_eq!(m.sections.len(), 3);
        assert_eq!(m.sections[0].title, None);
        assert_eq!(m.sections[1].title.as_deref(), Some("Scanning"));
        assert_eq!(m.settings[0].key, "old");
    }

    #[test]
    fn ui_needs_api_1_2() {
        for api in ["1", "\"1.1\""] {
            let t = format!("{}{SECTIONS}", with(&ui_manifest(""), "api", api));
            let e = parse(&t).unwrap_err();
            assert_eq!(e.code, "plugin_invalid", "{api}");
            assert!(e.message.contains("1.2"), "{}", e.message);
        }
    }

    #[test]
    fn ui_controls_are_checked() {
        let ctl = |body: &str| ui_manifest(&format!("\n[[ui.section]]\n[[ui.section.control]]\n{body}\n"));
        let bad = [
            // a value control needs a key and a default
            "type = \"toggle\"\nlabel = \"L\"\ndefault = true",
            "type = \"toggle\"\nkey = \"k\"\nlabel = \"L\"",
            // the default must match the type
            "type = \"toggle\"\nkey = \"k\"\nlabel = \"L\"\ndefault = \"yes\"",
            // choices
            "type = \"select\"\nkey = \"k\"\nlabel = \"L\"\ndefault = \"a\"",
            "type = \"segmented\"\nkey = \"k\"\nlabel = \"L\"\ndefault = \"c\"\nchoices = [\"a\", \"b\"]",
            // a slider needs min < max and a default inside
            "type = \"slider\"\nkey = \"k\"\nlabel = \"L\"\ndefault = 1",
            "type = \"slider\"\nkey = \"k\"\nlabel = \"L\"\ndefault = 5\nmin = 0\nmax = 3",
            "type = \"slider\"\nkey = \"k\"\nlabel = \"L\"\ndefault = 1\nmin = 3\nmax = 3",
            // a button needs an action, a markdown block a body, a stat / list a key
            "type = \"button\"\nlabel = \"L\"",
            "type = \"markdown\"\nlabel = \"L\"",
            "type = \"stat\"\nlabel = \"L\"",
            "type = \"list\"\nlabel = \"L\"",
            // an unknown kind or field
            "type = \"dial\"\nkey = \"k\"\nlabel = \"L\"\ndefault = 1",
            "type = \"toggle\"\nkey = \"k\"\nlabel = \"L\"\ndefault = true\ncolour = \"red\"",
            // requires must name another value control
            "type = \"toggle\"\nkey = \"k\"\nlabel = \"L\"\ndefault = true\nrequires = \"nope\"",
            "type = \"toggle\"\nkey = \"k\"\nlabel = \"L\"\ndefault = true\nrequires = \"k\"",
            // app must name a [[detect]]
            "type = \"toggle\"\nkey = \"k\"\nlabel = \"L\"\ndefault = true\napp = \"nope\"",
        ];
        for b in bad {
            assert_eq!(parse(&ctl(b)).unwrap_err().code, "plugin_invalid", "{b}");
        }
        // A key may not repeat, across sections or with a flat [[settings]].
        let two = "type = \"toggle\"\nkey = \"k\"\nlabel = \"L\"\ndefault = true";
        assert_eq!(parse(&ctl(&format!("{two}\n[[ui.section.control]]\n{two}"))).unwrap_err().code, "plugin_invalid");
        let flat = format!("\n[[settings]]\nkey = \"k\"\nlabel = \"K\"\ntype = \"bool\"\ndefault = true\n{}", "");
        assert_eq!(parse(&ui_manifest(&format!("{flat}\n[[ui.section]]\n[[ui.section.control]]\n{two}"))).unwrap_err().code, "plugin_invalid");
    }

    #[test]
    fn buttons_stats_and_lists_need_code() {
        // A providers-only plugin has no wasm to answer a button.
        let t = format!("{MIN12}{DETECT}\n[[ui.section]]\n[[ui.section.control]]\ntype = \"button\"\nlabel = \"L\"\naction = \"go\"\n");
        let e = parse(&t).unwrap_err();
        assert_eq!(e.code, "plugin_invalid");
        assert!(e.message.contains("tools") || e.message.contains("importer"), "{}", e.message);
        // Toggles, selects and text are fine without code.
        let t = format!(
            "{MIN12}{DETECT}\n[[ui.section]]\n[[ui.section.control]]\ntype = \"text\"\nkey = \"k\"\nlabel = \"L\"\ndefault = \"\"\napp = \"disk-atlas\"\n"
        );
        assert!(parse(&t).is_ok());
    }

    #[test]
    fn a_provider_can_be_enabled_by_a_toggle_control() {
        let t = format!(
            "{MIN12}{DETECT}\n[[ui.section]]\n[[ui.section.control]]\ntype = \"toggle\"\nkey = \"use_picker\"\nlabel = \"L\"\ndefault = true\n\
             \n[[providers]]\nkind = \"reveal\"\napp = \"disk-atlas\"\nargs = [\"--in={{folder}}\"]\nenabled_by = \"use_picker\"\n"
        );
        assert!(parse(&t).is_ok());
    }

    // API 1.2: contribution slots (Round 25 P4a).

    const CONTRIB: &str = r#"
[[ui.contribute]]
slot = "map.toolbar"
label = "Find places"
icon = "map-pin"
action = "open-review:places"

[[ui.contribute]]
slot = "library.selection-menu"
label = "Check these"
action = "check"

[[ui.contribute]]
slot = "palette"
label = "Scan everything"
icon = "search"
action = "scan"
"#;

    #[test]
    fn contributions_parse_with_their_slot_label_icon_and_action() {
        let m = parse(&ui_manifest(CONTRIB)).unwrap();
        let c = &m.contributions;
        assert_eq!(c.len(), 3);
        assert_eq!(c[0].slot, ContributionSlot::MapToolbar);
        assert_eq!((c[0].label.as_str(), c[0].icon.as_deref(), c[0].action.as_str()), ("Find places", Some("map-pin"), "open-review:places"));
        assert_eq!((c[1].slot, c[1].icon.clone()), (ContributionSlot::LibrarySelectionMenu, None));
        assert_eq!(c[2].slot, ContributionSlot::Palette);
        // A manifest with no contributions has none, and `[ui]` with only contributions is fine.
        assert!(parse(&ui_manifest(SECTIONS)).unwrap().contributions.is_empty());
        assert!(m.sections.is_empty());
    }

    #[test]
    fn every_slot_name_is_known_and_a_viewer_slot_exists() {
        for (name, slot) in [
            ("map.toolbar", ContributionSlot::MapToolbar),
            ("library.selection-menu", ContributionSlot::LibrarySelectionMenu),
            ("viewer.menu", ContributionSlot::ViewerMenu),
            ("palette", ContributionSlot::Palette),
        ] {
            let t = ui_manifest(&format!("\n[[ui.contribute]]\nslot = \"{name}\"\nlabel = \"L\"\naction = \"a\"\n"));
            assert_eq!(parse(&t).unwrap().contributions[0].slot, slot, "{name}");
        }
    }

    #[test]
    fn contributions_are_checked() {
        let c = |body: &str| ui_manifest(&format!("\n[[ui.contribute]]\n{body}\n"));
        let bad = [
            // an unknown slot, a missing or empty slot / label / action
            "slot = \"sidebar\"\nlabel = \"L\"\naction = \"a\"",
            "label = \"L\"\naction = \"a\"",
            "slot = \"palette\"\nlabel = \"  \"\naction = \"a\"",
            "slot = \"palette\"\nlabel = \"L\"\naction = \"\"",
            "slot = \"palette\"\nlabel = \"L\"",
            // an icon outside the allowlist, a path, a URL
            "slot = \"palette\"\nlabel = \"L\"\naction = \"a\"\nicon = \"skull\"",
            "slot = \"palette\"\nlabel = \"L\"\naction = \"a\"\nicon = \"https://x.test/a.svg\"",
            // an unknown field
            "slot = \"palette\"\nlabel = \"L\"\naction = \"a\"\nshortcut = \"Mod+K\"",
            // an overlong label or action
            &format!("slot = \"palette\"\nlabel = \"{}\"\naction = \"a\"", "x".repeat(61)),
            &format!("slot = \"palette\"\nlabel = \"L\"\naction = \"{}\"", "x".repeat(65)),
            // an open-review with no id, or an id with a slash
            "slot = \"palette\"\nlabel = \"L\"\naction = \"open-review:\"",
            "slot = \"palette\"\nlabel = \"L\"\naction = \"open-review:../x\"",
        ];
        for b in bad {
            assert_eq!(parse(&c(b)).unwrap_err().code, "plugin_invalid", "{b}");
        }
        // The same label twice in one slot is a mistake; in two slots it is not.
        let one = "slot = \"palette\"\nlabel = \"L\"\naction = \"a\"";
        assert_eq!(parse(&c(&format!("{one}\n[[ui.contribute]]\n{one}"))).unwrap_err().code, "plugin_invalid");
        let other = "slot = \"viewer.menu\"\nlabel = \"L\"\naction = \"a\"";
        assert!(parse(&c(&format!("{one}\n[[ui.contribute]]\n{other}"))).is_ok());
    }

    #[test]
    fn a_wasm_action_needs_code_but_open_review_does_not() {
        let t = |action: &str| format!("{MIN12}{DETECT}\n[[ui.contribute]]\nslot = \"palette\"\nlabel = \"L\"\naction = \"{action}\"\n");
        let e = parse(&t("scan")).unwrap_err();
        assert_eq!(e.code, "plugin_invalid");
        assert!(e.message.contains("tools") || e.message.contains("importer"), "{}", e.message);
        assert!(parse(&t("open-review:places")).is_ok());
    }

    #[test]
    fn a_tool_action_names_a_declared_tool() {
        let t = |action: &str, tools: &str| format!("{MIN12}{DETECT}{tools}\n[[ui.contribute]]\nslot = \"map.toolbar\"\nlabel = \"L\"\naction = \"{action}\"\n");
        let tool = "\n[[tools]]\nname = \"find\"\ndescription = \"d\"\n";
        let c = parse(&t("tool:find", tool)).unwrap().contributions;
        assert_eq!(c[0].action, "tool:find");
        for bad in ["tool:", "tool:other"] {
            let e = parse(&t(bad, tool)).unwrap_err();
            assert_eq!(e.code, "plugin_invalid", "{bad}");
            assert!(e.message.contains("[[tools]]"), "{}", e.message);
        }
        // a tool action needs no `ui` export (it runs the tool, not `ui.action`)
        assert!(!needs_ui_export(&parse(&t("tool:find", tool)).unwrap()));
        assert!(needs_ui_export(&parse(&t("scan", tool)).unwrap()));
    }

    #[test]
    fn contributions_need_api_1_2() {
        let t = format!("{}{CONTRIB}", with(&ui_manifest(""), "api", "\"1.1\""));
        let e = parse(&t).unwrap_err();
        assert_eq!(e.code, "plugin_invalid");
        assert!(e.message.contains("1.2"), "{}", e.message);
    }

    /// API 1.3 (Round 25 P4b): the proposal permissions need api 1.3, and parse once it is.
    #[test]
    fn proposal_permissions_need_api_1_3() {
        let ids = ["dates.write", "tags.write", "captions.write", "people.write"];
        for id in ids {
            let old = MIN11.replace("api = \"1.1.0\"", "api = \"1.2\"");
            let e = parse(&format!("{old}permissions = [\"{id}\"]\n")).unwrap_err();
            assert!(e.message.contains("1.3"), "{id}: {}", e.message);
        }
        let new = MIN11.replace("api = \"1.1.0\"", "api = \"1.3\"");
        let list = ids.map(|i| format!("\"{i}\"")).join(", ");
        let m = parse(&format!("{new}permissions = [{list}]\n")).unwrap();
        assert_eq!(
            m.permissions,
            [PluginPermission::DatesWrite, PluginPermission::TagsWrite, PluginPermission::CaptionsWrite, PluginPermission::PeopleWrite]
        );
        assert!(crate::API_VERSION >= ApiVersion { major: 1, minor: 3 });
    }

    #[test]
    fn api_1_0_and_1_1_still_parse_with_ui_free_manifests() {
        assert!(parse(FULL).is_ok());
        assert!(parse(&format!("{MIN11}{DETECT}{PICKER}")).is_ok());
        assert!(crate::API_VERSION >= ApiVersion { major: 1, minor: 2 });
    }

    #[test]
    fn links_must_be_https() {
        for key in ["author_url", "repository"] {
            for bad in ["http://example.com", "javascript:alert(1)", "example.com", "file:///c:/x", "https://"] {
                let e = parse(&format!("{MIN}{key} = \"{bad}\"\n")).unwrap_err();
                assert_eq!(e.code, "plugin_invalid", "{key} {bad}");
                assert!(e.message.contains(key), "{}", e.message);
            }
        }
    }

    #[test]
    fn listing_screenshots_are_few_and_under_screenshots() {
        let l = |paths: &str| format!("{MIN}\n[listing]\nscreenshots = [{paths}]\n");
        let seven: Vec<String> = (0..7).map(|i| format!("\"screenshots/{i}.png\"")).collect();
        for bad in [
            seven.join(", "),
            "\"icon.png\"".into(),
            "\"screenshots/../icon.png\"".into(),
            "\"screenshots/\"".into(),
            "\"screenshots/a.gif\"".into(),
            "\"screenshots/a.png\", \"screenshots/a.png\"".into(),
            "\"screenshots\\\\a.png\"".into(),
        ] {
            assert_eq!(parse(&l(&bad)).unwrap_err().code, "plugin_invalid", "{bad}");
        }
        assert!(parse(&l("")).unwrap().screenshots.is_empty());
        let six: Vec<String> = (0..6).map(|i| format!("\"screenshots/{i}.png\"")).collect();
        assert_eq!(parse(&l(&six.join(", "))).unwrap().screenshots.len(), 6);
    }
}
