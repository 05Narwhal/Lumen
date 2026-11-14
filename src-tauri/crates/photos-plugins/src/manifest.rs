//! `plugin.toml` (spec §4): parsed strictly, then checked field by field.

use serde::Deserialize;

use crate::{Pick, PluginError, PluginKind, PluginPermission, SettingKind};

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
}

impl SettingDecl {
    /// Whether `v` is a value of this setting's type (a choice must be one of `choices`).
    pub fn accepts(&self, v: &serde_json::Value) -> bool {
        match self.kind {
            SettingKind::Bool => v.is_boolean(),
            SettingKind::Number => v.is_number(),
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
    pub lumen: semver::VersionReq,
    pub api: u32,
    pub permissions: Vec<PluginPermission>,
    pub themes: Vec<FileRef>,
    pub presets: Option<DirRef>,
    pub apps: Option<FileRef>,
    pub prompts: Vec<PromptDecl>,
    pub importer: Option<ImporterDecl>,
    pub tools: Vec<ToolDecl>,
    pub assets: Vec<AssetDecl>,
    pub settings: Vec<SettingDecl>,
}

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
    lumen: String,
    api: u32,
    #[serde(default)]
    permissions: Vec<String>,
    #[serde(default)]
    themes: Vec<FileRef>,
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
}

impl Manifest {
    /// The kinds this manifest declares, in [`PluginKind`] order.
    pub fn kinds(&self) -> Vec<PluginKind> {
        let mut k = Vec::new();
        if !self.themes.is_empty() {
            k.push(PluginKind::Theme);
        }
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
}

fn invalid(msg: impl Into<String>) -> PluginError {
    PluginError::invalid(format!("plugin.toml: {}", msg.into()))
}

fn valid_id(id: &str) -> bool {
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

pub fn parse(text: &str) -> Result<Manifest, PluginError> {
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
    let mut permissions = Vec::new();
    for p in &raw.permissions {
        let perm = PluginPermission::from_id(p)
            .ok_or_else(|| PluginError::permission(format!("Unknown permission \"{p}\"")))?;
        if !permissions.contains(&perm) {
            permissions.push(perm);
        }
    }
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
    }
    Ok(Manifest {
        id: raw.id,
        name: raw.name,
        version,
        author: raw.author,
        description: raw.description,
        license: raw.license,
        homepage: raw.homepage,
        lumen,
        api: raw.api,
        permissions,
        themes: raw.themes,
        presets: raw.presets,
        apps: raw.apps,
        prompts: raw.prompts,
        importer: raw.importer,
        tools: raw.tools,
        assets: raw.assets,
        settings: raw.settings,
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

[[themes]]
file = "themes/nord.toml"

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
                PluginKind::Theme,
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
}
