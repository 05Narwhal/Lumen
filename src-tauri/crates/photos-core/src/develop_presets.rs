//! Develop presets (spec §1): six built in (defined here, never stored) and the user's own,
//! each setting only the panels it was saved with.

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::develop::Panel;
use crate::edits::{Basic, Curve, Develop, Effects, Grading, Grain, Vignette, Wheel};
use crate::error::CoreError;
use crate::Result;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Preset {
    /// Negative for the built-ins, 0 for a plugin's (those are told apart by `key`).
    #[ts(type = "number")]
    pub id: i64,
    /// Stable across restarts and unique in the list: `builtin:<n>`, `db:<id>` or
    /// `plugin:<plugin id>/<file name>` (Round 23 §7).
    pub key: String,
    pub name: String,
    pub develop: Develop,
    pub panels: Vec<Panel>,
    pub builtin: bool,
    pub source: PresetSource,
}

/// Where a preset comes from. A plugin's presets are read from its installed folder and are
/// read-only; `name` is the plugin's name, the title of its group in the list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum PresetSource {
    Builtin,
    Mine,
    Plugin { plugin: String, name: String },
}

fn builtin(id: i64, name: &str, develop: Develop, panels: &[Panel]) -> Preset {
    Preset { id, key: format!("builtin:{}", -id), name: name.into(), develop, panels: panels.to_vec(), builtin: true, source: PresetSource::Builtin }
}

fn mine(id: i64, name: String, develop: Develop, panels: Vec<Panel>) -> Preset {
    Preset { id, key: format!("db:{id}"), name, develop, panels, builtin: false, source: PresetSource::Mine }
}

/// The newest recipe version this Lumen reads (v3 carries masks, `edits::Recipe`).
pub const RECIPE_VERSION: u32 = 3;

/// A preset file a plugin ships (`presets/*.json`): Lumen's preset shape plus the recipe
/// version it was written for, and optionally the MCP *look* it describes (read by
/// `mcp::looks`, so it stays JSON here).
#[derive(Debug, Clone, PartialEq)]
pub struct PluginPresetFile {
    pub name: String,
    pub develop: Develop,
    pub panels: Vec<Panel>,
    pub look: Option<serde_json::Value>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PluginPresetWire {
    recipe_version: Option<u32>,
    name: String,
    #[serde(default)]
    develop: Develop,
    panels: Vec<Panel>,
    #[serde(default)]
    look: Option<serde_json::Value>,
}

/// Reads a plugin's preset JSON through the same checks as a Lumen preset: the recipe version
/// must be one this Lumen knows (never a silent partial apply of a newer file), a name and a
/// panel are needed, and the values are brought inside their ranges. Masks are kept.
pub fn parse_plugin_preset(text: &str) -> std::result::Result<PluginPresetFile, String> {
    let wire: PluginPresetWire = serde_json::from_str(text).map_err(|e| e.to_string())?;
    match wire.recipe_version {
        None => return Err("recipeVersion is missing".into()),
        Some(0) => return Err("recipeVersion 0 is not a recipe version".into()),
        Some(v) if v > RECIPE_VERSION => return Err(format!("recipeVersion {v} is newer than this Lumen reads (up to {RECIPE_VERSION})")),
        Some(_) => {}
    }
    let name = wire.name.trim().to_string();
    if name.is_empty() {
        return Err("a preset needs a name".into());
    }
    if wire.panels.is_empty() {
        return Err("a preset needs at least one panel".into());
    }
    Ok(PluginPresetFile { name, develop: wire.develop.normalized(), panels: wire.panels, look: wire.look })
}

/// The built-ins, in the order the panel lists them.
pub fn builtin_presets() -> Vec<Preset> {
    use Panel as P;
    vec![
        // A touch of snap, as a clean starting point.
        builtin(-1, "Natural", Develop { basic: Basic { contrast: 0.1, clarity: 0.1, vibrance: 0.15, ..Default::default() }, ..Default::default() }, &[P::Basic]),
        // Punchy colour and local contrast.
        builtin(
            -2,
            "Vivid",
            Develop { basic: Basic { contrast: 0.25, clarity: 0.15, dehaze: 0.05, vibrance: 0.4, saturation: 0.1, ..Default::default() }, ..Default::default() },
            &[P::Basic],
        ),
        // Lifted blacks, rolled-off whites, teal shadows and warm highlights, a little grain.
        builtin(
            -3,
            "Soft film",
            Develop {
                basic: Basic { contrast: -0.15, highlights: -0.2, shadows: 0.15, blacks: 0.15, vibrance: -0.1, ..Default::default() },
                curve: Curve { master: vec![[0.0, 0.06], [0.25, 0.24], [0.75, 0.76], [1.0, 0.95]], ..Default::default() },
                grading: Grading { shadows: Wheel { hue: 200.0, sat: 0.15, lum: 0.0 }, highlights: Wheel { hue: 45.0, sat: 0.12, lum: 0.0 }, ..Default::default() },
                effects: Effects { grain: Grain { amount: 0.25, size: 0.35 }, ..Default::default() },
                ..Default::default()
            },
            &[P::Basic, P::Curve, P::Grading, P::Effects],
        ),
        // Monochrome with a little more bite.
        builtin(-4, "B&W", Develop { basic: Basic { saturation: -1.0, contrast: 0.2, clarity: 0.15, ..Default::default() }, ..Default::default() }, &[P::Basic]),
        // Golden light, highlights held, edges darkened.
        builtin(
            -5,
            "Warm evening",
            Develop {
                basic: Basic { temperature: 25.0, tint: 5.0, highlights: -0.15, vibrance: 0.15, ..Default::default() },
                grading: Grading { highlights: Wheel { hue: 40.0, sat: 0.15, lum: 0.0 }, ..Default::default() },
                effects: Effects { vignette: Vignette { amount: -0.15, ..Default::default() }, ..Default::default() },
                ..Default::default()
            },
            &[P::Basic, P::Grading, P::Effects],
        ),
        // Clean, cool and a little brighter.
        builtin(
            -6,
            "Cool morning",
            Develop {
                basic: Basic { temperature: -20.0, tint: -3.0, exposure: 0.1, contrast: -0.05, shadows: 0.1, ..Default::default() },
                grading: Grading { shadows: Wheel { hue: 220.0, sat: 0.12, lum: 0.0 }, ..Default::default() },
                ..Default::default()
            },
            &[P::Basic, P::Grading],
        ),
    ]
}

/// Built-ins first, then the user's by name.
pub fn list_presets(conn: &Connection) -> Result<Vec<Preset>> {
    let mut out = builtin_presets();
    let mut stmt = conn.prepare("SELECT id, name, settings_json, panels_json FROM develop_presets WHERE builtin = 0 ORDER BY name COLLATE NOCASE, id")?;
    let rows = stmt.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?, r.get::<_, String>(3)?)))?;
    for row in rows {
        let (id, name, settings, panels) = row?;
        out.push(mine(id, name, serde_json::from_str(&settings)?, serde_json::from_str(&panels)?));
    }
    Ok(out)
}

fn clean_name(name: &str) -> Result<String> {
    let n = name.trim();
    if n.is_empty() {
        return Err(CoreError::Invalid("a preset needs a name".into()));
    }
    Ok(n.to_string())
}

pub fn save_preset(conn: &Connection, name: &str, develop: &Develop, panels: &[Panel], now: i64) -> Result<Preset> {
    let name = clean_name(name)?;
    // Masks belong to one photo's pixels, so a preset never carries them.
    let panels: Vec<Panel> = panels.iter().copied().filter(|p| *p != Panel::Masks).collect();
    if panels.is_empty() {
        return Err(CoreError::Invalid("a preset needs at least one panel".into()));
    }
    let develop = Develop { masks: Vec::new(), ..develop.clone().normalized() };
    conn.execute(
        "INSERT INTO develop_presets (name, settings_json, panels_json, builtin, created_at) VALUES (?1, ?2, ?3, 0, ?4)",
        params![name, serde_json::to_string(&develop)?, serde_json::to_string(&panels)?, now],
    )?;
    Ok(mine(conn.last_insert_rowid(), name, develop, panels))
}

/// What importing one Lightroom preset did (Round 11). `notes` are the settings it had that
/// a Lumen preset cannot carry.
#[derive(Debug, Clone, PartialEq)]
pub enum Imported {
    Saved { preset: Box<Preset>, notes: Vec<String> },
    /// A preset of that name exists (built in or the user's): it is not duplicated.
    SameName,
    /// Nothing in it is something a Lumen preset sets.
    Nothing { notes: Vec<String> },
}

/// Saves a converted Lightroom preset as a user preset setting exactly the panels it touched.
/// Masks and the crop belong to one photo, so they are named in `notes` and left out.
pub fn import_converted(conn: &Connection, name: &str, converted: &crate::lightroom::Converted, now: i64) -> Result<Imported> {
    let name = clean_name(name)?;
    let taken = builtin_presets().iter().any(|p| p.name.eq_ignore_ascii_case(&name))
        || conn.query_row("SELECT 1 FROM develop_presets WHERE builtin = 0 AND name = ?1 COLLATE NOCASE", [&name], |_| Ok(())).optional()?.is_some();
    if taken {
        return Ok(Imported::SameName);
    }
    let mut notes = converted.skipped.clone();
    notes.extend(converted.develop.masks.iter().map(|m| format!("Mask: {}", m.name)));
    if converted.geometry.is_some() {
        notes.push("Crop and straighten".into());
    }
    if converted.panels.iter().all(|p| *p == Panel::Masks) {
        return Ok(Imported::Nothing { notes });
    }
    let preset = save_preset(conn, &name, &converted.develop, &converted.panels, now)?;
    Ok(Imported::Saved { preset: Box::new(preset), notes })
}

/// *Duplicate to my presets* (Round 23 §7): writes a plugin preset as the person's own row, exactly
/// as the plugin ships it (masks and panels included), so it outlives the plugin. Only a plugin's
/// preset is duplicated; the person's own and the built-ins have their own ways.
pub fn duplicate_preset(conn: &Connection, source: &Preset, now: i64) -> Result<Preset> {
    if !matches!(source.source, PresetSource::Plugin { .. }) {
        return Err(CoreError::Invalid("only a plugin's preset is duplicated".into()));
    }
    let name = clean_name(&source.name)?;
    conn.execute(
        "INSERT INTO develop_presets (name, settings_json, panels_json, builtin, created_at) VALUES (?1, ?2, ?3, 0, ?4)",
        params![name, serde_json::to_string(&source.develop)?, serde_json::to_string(&source.panels)?, now],
    )?;
    Ok(mine(conn.last_insert_rowid(), name, source.develop.clone(), source.panels.clone()))
}

fn user_preset(conn: &Connection, id: i64) -> Result<()> {
    if id < 0 {
        return Err(CoreError::Invalid("built-in presets cannot be changed".into()));
    }
    if id == 0 {
        return Err(CoreError::Invalid("a plugin's presets cannot be changed; duplicate one to change the copy".into()));
    }
    let found: Option<i64> = conn.query_row("SELECT id FROM develop_presets WHERE id = ?1 AND builtin = 0", [id], |r| r.get(0)).optional()?;
    found.map(|_| ()).ok_or_else(|| CoreError::NotFound(format!("preset {id}")))
}

pub fn rename_preset(conn: &Connection, id: i64, name: &str) -> Result<()> {
    user_preset(conn, id)?;
    conn.execute("UPDATE develop_presets SET name = ?2 WHERE id = ?1", params![id, clean_name(name)?])?;
    Ok(())
}

pub fn delete_preset(conn: &Connection, id: i64) -> Result<()> {
    user_preset(conn, id)?;
    conn.execute("DELETE FROM develop_presets WHERE id = ?1", [id])?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::develop::tests::library;

    #[test]
    fn built_ins_come_first_and_cannot_be_changed() {
        let db = library();
        db.with_conn(|c| {
            let names: Vec<String> = list_presets(c)?.into_iter().map(|p| p.name).collect();
            assert_eq!(names, ["Natural", "Vivid", "Soft film", "B&W", "Warm evening", "Cool morning"]);
            assert!(matches!(delete_preset(c, -1), Err(CoreError::Invalid(_))));
            assert!(matches!(rename_preset(c, -2, "Mine"), Err(CoreError::Invalid(_))));
            for p in builtin_presets() {
                assert_eq!(p.develop.clone().normalized(), p.develop, "{} is inside every range", p.name);
                assert!(!p.panels.is_empty());
            }
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn presets_never_carry_masks() {
        let db = library();
        db.with_conn(|c| {
            let masked = crate::edits::tests::masked(crate::edits::LocalAdjust { exposure: 1.0, ..Default::default() }).develop.unwrap();
            let d = Develop { basic: Basic { exposure: 0.7, ..Default::default() }, ..masked };
            let p = save_preset(c, "Mine", &d, &[Panel::Basic, Panel::Masks], 0)?;
            assert!(p.develop.masks.is_empty() && p.panels == [Panel::Basic], "{p:?}");
            assert!(matches!(save_preset(c, "Only masks", &d, &[Panel::Masks], 0), Err(CoreError::Invalid(_))), "no panel left");
            let stored = list_presets(c)?.into_iter().find(|q| q.id == p.id).unwrap();
            assert!(stored.develop.masks.is_empty() && !stored.panels.contains(&Panel::Masks));
            Ok(())
        })
        .unwrap();
    }

    fn converted(settings: &[(&str, f64)]) -> crate::lightroom::Converted {
        let s = settings.iter().map(|(k, v)| (k.to_string(), crate::lightroom::Value::Num(*v))).collect();
        crate::lightroom::convert(&s, None)
    }

    #[test]
    fn a_lightroom_preset_sets_only_the_panels_it_touched() {
        let db = library();
        db.with_conn(|c| {
            let only = converted(&[("Vibrance", 15.0)]);
            let Imported::Saved { preset, notes } = import_converted(c, "Pop", &only, 0)? else { panic!() };
            assert_eq!(preset.panels, vec![Panel::Basic]);
            assert_eq!(preset.develop.basic.vibrance, 0.15);
            assert!(notes.is_empty(), "{notes:?}");
            // The same name again is skipped, whatever its case; a built-in's name too.
            assert!(matches!(import_converted(c, "pop", &only, 0)?, Imported::SameName));
            assert!(matches!(import_converted(c, "Vivid", &only, 0)?, Imported::SameName));
            assert_eq!(list_presets(c)?.iter().filter(|p| !p.builtin).count(), 1);
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn a_lightroom_presets_masks_and_crop_are_named_not_kept() {
        let db = library();
        db.with_conn(|c| {
            let mut conv = converted(&[("Exposure2012", 0.5), ("HasCrop", 1.0), ("CropLeft", 0.1), ("CropRight", 0.9), ("CropTop", 0.0), ("CropBottom", 1.0)]);
            conv.skipped.push("LensProfileEnable".into());
            conv.develop.masks = crate::edits::tests::masked(crate::edits::LocalAdjust { exposure: 1.0, ..Default::default() }).develop.unwrap().masks;
            conv.develop.masks[0].name = "Sky".into();
            conv.panels.push(Panel::Masks);
            let Imported::Saved { preset, notes } = import_converted(c, "Graduated", &conv, 0)? else { panic!() };
            assert!(preset.develop.masks.is_empty());
            assert_eq!(preset.panels, vec![Panel::Basic]);
            assert_eq!(notes, vec!["LensProfileEnable".to_string(), "Mask: Sky".into(), "Crop and straighten".into()]);
            // Nothing Lumen can use is no preset.
            let none = converted(&[("LensProfileEnable", 1.0)]);
            assert!(matches!(import_converted(c, "Lens only", &none, 0)?, Imported::Nothing { notes } if notes == vec!["LensProfileEnable".to_string()]));
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn user_presets_round_trip() {
        let db = library();
        db.with_conn(|c| {
            let d = Develop { basic: Basic { exposure: 0.7, ..Default::default() }, ..Default::default() };
            assert!(matches!(save_preset(c, "Bright", &d, &[], 0), Err(CoreError::Invalid(_))));
            let p = save_preset(c, " Bright ", &d, &[Panel::Basic], 0)?;
            assert_eq!(p.name, "Bright");
            save_preset(c, "Also", &d, &[Panel::Basic, Panel::Effects], 0)?;
            let all = list_presets(c)?;
            assert_eq!(all.len(), 8);
            assert_eq!(all[6].name, "Also", "user presets by name");
            assert_eq!(all[7].develop.basic.exposure, 0.7);
            rename_preset(c, p.id, "Brighter")?;
            delete_preset(c, p.id)?;
            assert_eq!(list_presets(c)?.len(), 7);
            assert!(matches!(delete_preset(c, p.id), Err(CoreError::NotFound(_))));
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn keys_are_stable() {
        let db = library();
        db.with_conn(|c| {
            let d = Develop { basic: Basic { exposure: 0.7, ..Default::default() }, ..Default::default() };
            let p = save_preset(c, "Bright", &d, &[Panel::Basic], 0)?;
            assert_eq!(p.key, format!("db:{}", p.id));
            assert_eq!(p.source, PresetSource::Mine);
            let all = list_presets(c)?;
            let natural = all.iter().find(|q| q.name == "Natural").unwrap();
            assert_eq!(natural.key, "builtin:1");
            assert_eq!(natural.source, PresetSource::Builtin);
            assert_eq!(all.iter().find(|q| q.name == "Cool morning").unwrap().key, "builtin:6");
            assert_eq!(all.iter().find(|q| q.id == p.id).unwrap().key, p.key);
            let keys: std::collections::HashSet<_> = all.iter().map(|q| q.key.clone()).collect();
            assert_eq!(keys.len(), all.len(), "keys are unique");
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn parses_plugin_preset() {
        let text = r#"{
            "recipeVersion": 3,
            "name": "Kodak warm",
            "develop": { "basic": { "temperature": 12.0, "contrast": 0.1 } },
            "panels": ["basic"],
            "look": { "intent": "warm film", "hues": [30, 45], "mono": false, "key": "mid", "skinSafe": true, "likesSky": false }
        }"#;
        let f = parse_plugin_preset(text).unwrap();
        assert_eq!(f.name, "Kodak warm");
        assert_eq!(f.develop.basic.temperature, 12.0);
        assert_eq!(f.panels, vec![Panel::Basic]);
        let look = f.look.unwrap();
        assert_eq!(look["intent"], "warm film");
        assert_eq!(look["skinSafe"], true);
        // Without a look, `look` is None; unknown top-level keys are refused.
        assert!(parse_plugin_preset(r#"{"recipeVersion":2,"name":"x","develop":{},"panels":["basic"]}"#).unwrap().look.is_none());
        assert!(parse_plugin_preset(r#"{"recipeVersion":2,"name":"x","develop":{},"panels":["basic"],"extra":1}"#).is_err());
    }

    #[test]
    fn future_recipe_version_refused() {
        let err = parse_plugin_preset(r#"{"recipeVersion":4,"name":"x","develop":{},"panels":["basic"]}"#).unwrap_err();
        assert!(err.contains("recipeVersion 4") && err.contains("newer"), "{err}");
        assert!(parse_plugin_preset(r#"{"name":"x","develop":{},"panels":["basic"]}"#).is_err(), "a preset states its recipe version");
        assert!(parse_plugin_preset(r#"{"recipeVersion":0,"name":"x","develop":{},"panels":["basic"]}"#).is_err());
        assert!(parse_plugin_preset(r#"{"recipeVersion":2,"name":" ","develop":{},"panels":["basic"]}"#).is_err(), "a name is needed");
        assert!(parse_plugin_preset(r#"{"recipeVersion":2,"name":"x","develop":{},"panels":[]}"#).is_err(), "a panel is needed");
    }

    #[test]
    fn masks_allowed() {
        let masked = crate::edits::tests::masked(crate::edits::LocalAdjust { exposure: 1.0, ..Default::default() }).develop.unwrap();
        let text = serde_json::json!({ "recipeVersion": 3, "name": "Sky", "develop": masked, "panels": ["basic", "masks"] }).to_string();
        let f = parse_plugin_preset(&text).unwrap();
        assert_eq!(f.develop.masks.len(), 1);
        assert_eq!(f.panels, vec![Panel::Basic, Panel::Masks]);
    }

    #[test]
    fn duplicate_writes_a_row() {
        let db = library();
        db.with_conn(|c| {
            let masked = crate::edits::tests::masked(crate::edits::LocalAdjust { exposure: 1.0, ..Default::default() }).develop.unwrap();
            let develop = Develop { basic: Basic { exposure: 0.4, ..Default::default() }, ..masked };
            let plugin = Preset {
                id: 0,
                key: "plugin:com.x/warm.json".into(),
                name: "Warm".into(),
                develop,
                panels: vec![Panel::Basic, Panel::Masks],
                builtin: false,
                source: PresetSource::Plugin { plugin: "com.x".into(), name: "X".into() },
            };
            let copy = duplicate_preset(c, &plugin, 7)?;
            assert_eq!(copy.source, PresetSource::Mine);
            assert!(copy.id > 0 && copy.key == format!("db:{}", copy.id) && !copy.builtin);
            assert_eq!((copy.name.as_str(), &copy.develop, &copy.panels), ("Warm", &plugin.develop, &plugin.panels), "the copy is exact, masks included");
            let stored = list_presets(c)?.into_iter().find(|p| p.id == copy.id).unwrap();
            assert_eq!(stored, copy);
            // Only a plugin's preset is duplicated: the person's own and the built-ins have their own ways.
            assert!(matches!(duplicate_preset(c, &stored, 7), Err(CoreError::Invalid(_))));
            assert!(matches!(duplicate_preset(c, &builtin_presets()[0], 7), Err(CoreError::Invalid(_))));
            // A plugin's preset has id 0: it is never renamed or deleted through the row commands.
            assert!(matches!(rename_preset(c, 0, "Mine"), Err(CoreError::Invalid(_))));
            assert!(matches!(delete_preset(c, 0), Err(CoreError::Invalid(_))));
            Ok(())
        })
        .unwrap();
    }
}
