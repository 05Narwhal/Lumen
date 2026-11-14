//! The looks an AI assistant can offer when the user gave no direction (Round 18 D1): a
//! hand-written catalogue from known grading practice, never learned from anyone's photos.
//! Each look is a merge-patch shaped like `get_recipe` that the assistant starts from and
//! adapts to the photo; `fit` scores how well each one suits a photo.

use photos_core::develop::Panel;
use photos_core::edits::Develop;
use serde::Deserialize;
use serde_json::{json, Value};

pub const LOOKS_URI: &str = "lumen://looks";

/// The tonal key a look wants: a dark, a middle or a bright photo.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    Low,
    Mid,
    High,
}

/// One look: the built-ins (from the table below) and the plugins' (`plugin_look`).
#[derive(Debug, Clone, PartialEq)]
pub struct Look {
    /// A built-in's slug, or `plugin:<plugin id>/<preset file>` (the preset's key).
    pub id: String,
    pub name: String,
    /// One line: what it does and what it suits.
    pub intent: String,
    /// OKLCh hue angles (degrees) the grade pushes toward; empty for mono, natural and
    /// looks that push every colour alike.
    pub hues: Vec<f32>,
    pub mono: bool,
    pub key: Key,
    /// Keeps skin believable.
    pub skin_safe: bool,
    pub likes_sky: bool,
    /// The natural-but-better look, always offered (exactly one, never a plugin's).
    pub safe: bool,
    /// The look's starting point, a merge-patch for `develop_mcp::patch`.
    pub patch: Value,
}

/// A built-in look as written in the table below.
struct LookDef {
    id: &'static str,
    name: &'static str,
    intent: &'static str,
    hues: &'static [f32],
    mono: bool,
    key: Key,
    skin_safe: bool,
    likes_sky: bool,
    safe: bool,
    patch: fn() -> Value,
}

impl LookDef {
    fn build(&self) -> Look {
        Look {
            id: self.id.into(),
            name: self.name.into(),
            intent: self.intent.into(),
            hues: self.hues.to_vec(),
            mono: self.mono,
            key: self.key,
            skin_safe: self.skin_safe,
            likes_sky: self.likes_sky,
            safe: self.safe,
            patch: (self.patch)(),
        }
    }
}

// Grading wheel hues are Develop's (0 red, 30 orange, 60 yellow, 180 cyan, 210 blue,
// 300 magenta); `hues` are OKLCh (~30 red, ~55 orange/skin, ~95 yellow, ~140 green, ~200
// cyan, ~260 blue, ~330 magenta). Values are written as floats with at most two decimals
// so the summary reads each patch back unchanged.
static TABLE: &[LookDef] = &[
    LookDef {
        id: "natural",
        name: "Natural",
        intent: "The photo as it looked, only better: balanced light, open shadows, held highlights, a touch of colour. Suits anything.",
        hues: &[],
        mono: false,
        key: Key::Mid,
        skin_safe: true,
        likes_sky: false,
        safe: true,
        patch: || json!({ "develop": { "basic": { "contrast": 0.1, "highlights": -0.2, "shadows": 0.15, "clarity": 0.1, "vibrance": 0.15 } } }),
    },
    LookDef {
        id: "golden-film",
        name: "Golden film",
        intent: "Warm, slightly faded colour negative: golden highlights, cool lifted shadows, gentle grain. Suits late light, travel and everyday scenes.",
        hues: &[80.0],
        mono: false,
        key: Key::Mid,
        skin_safe: true,
        likes_sky: false,
        safe: false,
        patch: || {
            json!({ "develop": {
                "basic": { "temperature": 12.0, "contrast": 0.1, "highlights": -0.25, "vibrance": 0.1 },
                "curve": { "master": [[0.0, 0.06], [0.25, 0.24], [0.75, 0.8], [1.0, 0.96]] },
                "hsl": { "green": { "h": -0.25, "s": -0.15 }, "blue": { "s": -0.1 } },
                "grading": { "shadows": { "hue": 200.0, "sat": 0.12 }, "highlights": { "hue": 42.0, "sat": 0.25 } },
                "effects": { "grain": { "amount": 0.15 } }
            } })
        },
    },
    LookDef {
        id: "teal-orange",
        name: "Teal and orange",
        intent: "Cinematic split: teal shadows and sky against warm skin and highlights, punchy contrast. Suits people and warm subjects under blue sky or in blue light.",
        hues: &[55.0, 210.0],
        mono: false,
        key: Key::Mid,
        skin_safe: true,
        likes_sky: true,
        safe: false,
        patch: || {
            json!({ "develop": {
                "basic": { "contrast": 0.2, "highlights": -0.2, "shadows": 0.1, "clarity": 0.1, "vibrance": 0.2 },
                "hsl": { "aqua": { "h": 0.15, "s": 0.15 }, "blue": { "h": -0.15, "s": 0.1 }, "orange": { "s": 0.1 } },
                "grading": { "shadows": { "hue": 190.0, "sat": 0.35 }, "highlights": { "hue": 35.0, "sat": 0.3 }, "balance": 0.1 }
            } })
        },
    },
    LookDef {
        id: "cool-moody",
        name: "Cool and moody",
        intent: "Rich deep blue-teal grade, dense shadows, cool highlights, dark corners; colour stays full, never washed out. Suits overcast, forests, water, cities and empty places.",
        hues: &[250.0],
        mono: false,
        key: Key::Low,
        skin_safe: false,
        likes_sky: false,
        safe: false,
        patch: || {
            json!({ "develop": {
                "basic": { "temperature": -15.0, "exposure": -0.2, "contrast": 0.15, "highlights": -0.35, "vibrance": 0.2, "clarity": 0.15 },
                "curve": { "master": [[0.0, 0.0], [0.3, 0.22], [0.7, 0.7], [1.0, 0.97]] },
                "grading": { "shadows": { "hue": 215.0, "sat": 0.3, "lum": -0.1 }, "midtones": { "hue": 200.0, "sat": 0.1 } },
                "effects": { "vignette": { "amount": -0.25 } }
            } })
        },
    },
    LookDef {
        id: "low-key",
        name: "Low key",
        intent: "Chiaroscuro: most of the frame falls to near-black so the lit subject glows warm. Suits a bright subject on a dark ground, interiors, still life.",
        hues: &[60.0],
        mono: false,
        key: Key::Low,
        skin_safe: true,
        likes_sky: false,
        safe: false,
        patch: || {
            json!({ "develop": {
                "basic": { "exposure": -0.3, "contrast": 0.25, "highlights": -0.2, "blacks": -0.1, "clarity": 0.15 },
                "curve": { "master": [[0.0, 0.02], [0.25, 0.2], [0.75, 0.78], [1.0, 1.0]] },
                "grading": { "highlights": { "hue": 38.0, "sat": 0.18 } },
                "effects": { "vignette": { "amount": -0.3 } }
            } })
        },
    },
    LookDef {
        id: "high-key-airy",
        name: "High key, airy",
        intent: "Bright, soft and pastel: lifted shadows and blacks, low contrast, a hint of warmth. Suits light scenes, beaches, snow, children and flowers.",
        hues: &[80.0],
        mono: false,
        key: Key::High,
        skin_safe: true,
        likes_sky: true,
        safe: false,
        patch: || {
            json!({ "develop": {
                "basic": { "exposure": 0.6, "contrast": -0.2, "highlights": -0.2, "shadows": 0.4, "blacks": 0.2, "clarity": -0.1, "vibrance": 0.1, "saturation": -0.1 },
                "curve": { "master": [[0.0, 0.08], [0.5, 0.58], [1.0, 1.0]] },
                "grading": { "highlights": { "hue": 50.0, "sat": 0.08 } }
            } })
        },
    },
    LookDef {
        id: "faded-matte",
        name: "Faded matte",
        intent: "Gently matte blacks with a cool–warm split and light grain; colour keeps its body, never grey or washed out. Suits calm, nostalgic and lifestyle scenes.",
        hues: &[60.0, 230.0],
        mono: false,
        key: Key::Mid,
        skin_safe: true,
        likes_sky: false,
        safe: false,
        patch: || {
            json!({ "develop": {
                "basic": { "contrast": -0.05, "vibrance": 0.1 },
                "curve": { "master": [[0.0, 0.06], [0.25, 0.26], [0.75, 0.77], [1.0, 0.96]] },
                "grading": { "shadows": { "hue": 210.0, "sat": 0.12 }, "highlights": { "hue": 45.0, "sat": 0.12 } },
                "effects": { "grain": { "amount": 0.2 } }
            } })
        },
    },
    LookDef {
        id: "bw-contrast",
        name: "Black and white, contrast",
        intent: "Punchy monochrome: deep blacks, bright whites, darkened blue skies, crisp texture. Suits strong shapes, texture, architecture and dramatic light.",
        hues: &[],
        mono: true,
        key: Key::Mid,
        skin_safe: true,
        likes_sky: true,
        safe: false,
        patch: || {
            json!({ "develop": {
                "basic": { "contrast": 0.4, "whites": 0.2, "blacks": -0.25, "clarity": 0.25, "saturation": -1.0 },
                "curve": { "master": [[0.0, 0.0], [0.25, 0.18], [0.75, 0.84], [1.0, 1.0]] },
                "hsl": { "blue": { "l": -0.4 }, "aqua": { "l": -0.2 }, "orange": { "l": 0.1 } },
                "effects": { "vignette": { "amount": -0.2 }, "grain": { "amount": 0.12 } }
            } })
        },
    },
    LookDef {
        id: "bw-soft",
        name: "Black and white, soft",
        intent: "Gentle monochrome with open shadows, held highlights and a faint warm tone. Suits portraits, mist, soft light and quiet scenes.",
        hues: &[],
        mono: true,
        key: Key::Mid,
        skin_safe: true,
        likes_sky: false,
        safe: false,
        patch: || {
            json!({ "develop": {
                "basic": { "contrast": -0.15, "highlights": -0.2, "shadows": 0.25, "saturation": -1.0 },
                "curve": { "master": [[0.0, 0.06], [0.5, 0.52], [1.0, 0.96]] },
                "grading": { "midtones": { "hue": 40.0, "sat": 0.08 } },
                "effects": { "grain": { "amount": 0.15 } }
            } })
        },
    },
    LookDef {
        id: "vivid-pop",
        name: "Vivid pop",
        intent: "Rich, clean colour and crisp contrast without a tint: deep blues, lush greens. Suits colourful scenes, landscapes, food and festivals.",
        hues: &[],
        mono: false,
        key: Key::Mid,
        skin_safe: false,
        likes_sky: true,
        safe: false,
        patch: || {
            json!({ "develop": {
                "basic": { "contrast": 0.25, "clarity": 0.2, "dehaze": 0.1, "vibrance": 0.4, "saturation": 0.15 },
                "curve": { "master": [[0.0, 0.0], [0.25, 0.2], [0.75, 0.8], [1.0, 1.0]] },
                "hsl": { "blue": { "s": 0.2, "l": -0.1 }, "green": { "s": 0.1 } }
            } })
        },
    },
    LookDef {
        id: "warm-portrait",
        name: "Warm portrait",
        intent: "Flattering warmth: glowing, even skin, softened texture, gentle highlights. Suits people, faces and indoor light.",
        hues: &[50.0],
        mono: false,
        key: Key::Mid,
        skin_safe: true,
        likes_sky: false,
        safe: false,
        patch: || {
            json!({ "develop": {
                "basic": { "temperature": 10.0, "tint": 3.0, "exposure": 0.15, "highlights": -0.2, "shadows": 0.15, "texture": -0.15, "clarity": -0.1, "vibrance": 0.1 },
                "curve": { "master": [[0.0, 0.03], [0.5, 0.53], [1.0, 0.98]] },
                "hsl": { "orange": { "s": -0.1, "l": 0.1 }, "red": { "s": -0.05 } },
                "grading": { "shadows": { "hue": 25.0, "sat": 0.08 }, "highlights": { "hue": 40.0, "sat": 0.15 } }
            } })
        },
    },
    LookDef {
        id: "night-neon",
        name: "Night neon",
        intent: "Electric night: inky blues, glowing magentas and cyans, deep blacks. Suits city lights, festivals, light art and any night scene.",
        hues: &[330.0, 260.0],
        mono: false,
        key: Key::Low,
        skin_safe: false,
        likes_sky: false,
        safe: false,
        patch: || {
            json!({ "develop": {
                "basic": { "temperature": -10.0, "tint": 8.0, "contrast": 0.25, "highlights": -0.25, "blacks": -0.2, "dehaze": 0.15, "vibrance": 0.3, "saturation": 0.1 },
                "hsl": { "aqua": { "s": 0.2 }, "blue": { "h": 0.1, "s": 0.2 }, "purple": { "s": 0.3 }, "magenta": { "s": 0.25 } },
                "grading": { "shadows": { "hue": 230.0, "sat": 0.3 }, "highlights": { "hue": 320.0, "sat": 0.2 } }
            } })
        },
    },
];

pub fn catalogue() -> &'static [Look] {
    static BUILT: std::sync::OnceLock<Vec<Look>> = std::sync::OnceLock::new();
    BUILT.get_or_init(|| TABLE.iter().map(LookDef::build).collect())
}

#[cfg(test)]
pub fn by_id(id: &str) -> Option<&'static Look> {
    catalogue().iter().find(|l| l.id == id)
}

/// The built-ins, then the plugins' looks. A plugin look never takes a built-in's id (nor an
/// earlier plugin look's) and is never the `safe` look, whatever it says.
pub fn with_plugins(extra: Vec<Look>) -> Vec<Look> {
    let mut all: Vec<Look> = catalogue().to_vec();
    for look in extra {
        if !all.iter().any(|l| l.id == look.id) {
            all.push(Look { safe: false, ..look });
        }
    }
    all
}

/// The `lumen://looks` resource: every look in `looks` (the built-ins and the plugins', from
/// [`with_plugins`]) with its starting patch.
pub fn resource_of(looks: &[Look]) -> Value {
    json!({ "looks": looks.iter().map(|l| json!({ "id": l.id, "name": l.name, "intent": l.intent, "patch": l.patch })).collect::<Vec<_>>() })
}

/// A preset's `look = { intent, hues, mono, key, skinSafe, likesSky }` (spec §7), as a look whose
/// patch is the preset's own panels (never its masks: those belong to one photo). `safe` is not
/// a field, so a plugin can't claim the natural look's place.
pub fn plugin_look(id: &str, name: &str, look: &Value, develop: &Develop, panels: &[Panel]) -> Result<Look, String> {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    struct Wire {
        intent: String,
        #[serde(default)]
        hues: Vec<f32>,
        #[serde(default)]
        mono: bool,
        #[serde(default = "mid")]
        key: String,
        #[serde(default = "yes")]
        skin_safe: bool,
        #[serde(default)]
        likes_sky: bool,
    }
    fn mid() -> String {
        "mid".into()
    }
    fn yes() -> bool {
        true
    }
    let w: Wire = serde_json::from_value(look.clone()).map_err(|e| e.to_string())?;
    let intent = w.intent.trim().to_string();
    if intent.is_empty() || intent.chars().count() > 300 {
        return Err("look.intent is one line of 1-300 characters".into());
    }
    let key = match w.key.as_str() {
        "low" => Key::Low,
        "mid" => Key::Mid,
        "high" => Key::High,
        other => return Err(format!("look.key is low, mid or high, not \"{other}\"")),
    };
    if w.hues.len() > 4 || w.hues.iter().any(|h| !h.is_finite() || !(0.0..360.0).contains(h)) {
        return Err("look.hues are up to four angles from 0 to 359".into());
    }
    if w.mono && !w.hues.is_empty() {
        return Err("a mono look has no hues".into());
    }
    let wanted: Vec<&str> = panels.iter().filter(|p| **p != Panel::Masks).map(panel_key).collect();
    if wanted.is_empty() {
        return Err("a look needs a panel besides masks".into());
    }
    let Value::Object(all) = serde_json::to_value(develop).map_err(|e| e.to_string())? else {
        return Err("the preset's settings can't be read".into());
    };
    let develop_patch: serde_json::Map<String, Value> = all.into_iter().filter(|(k, _)| wanted.contains(&k.as_str())).collect();
    let patch_value = json!({ "develop": Value::Object(develop_patch) });
    photos_core::develop_mcp::patch(&photos_core::edits::Recipe::identity(), &patch_value).map_err(|e| format!("the preset isn't a valid starting point: {e}"))?;
    Ok(Look { id: id.into(), name: name.into(), intent, hues: w.hues, mono: w.mono, key, skin_safe: w.skin_safe, likes_sky: w.likes_sky, safe: false, patch: patch_value })
}

/// A panel's key in `Develop`'s JSON.
fn panel_key(p: &Panel) -> &'static str {
    match p {
        Panel::Basic => "basic",
        Panel::Curve => "curve",
        Panel::Hsl => "hsl",
        Panel::Grading => "grading",
        Panel::Detail => "detail",
        Panel::Effects => "effects",
        Panel::Masks => "masks",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use photos_core::develop_mcp::{patch, summary};
    use photos_core::edits::Recipe;
    use photos_core::develop::Panel;

    #[test]
    fn the_catalogue_is_valid() {
        let ids: std::collections::BTreeSet<_> = catalogue().iter().map(|l| l.id.clone()).collect();
        assert_eq!(ids.len(), 12, "12 unique ids");
        for id in ["natural", "golden-film", "teal-orange", "cool-moody", "low-key", "high-key-airy", "faded-matte", "bw-contrast", "bw-soft", "vivid-pop", "warm-portrait", "night-neon"] {
            assert!(by_id(id).is_some(), "{id}");
        }
        let safe: Vec<_> = catalogue().iter().filter(|l| l.safe).map(|l| l.id.as_str()).collect();
        assert_eq!(safe, ["natural"]);
        for l in catalogue() {
            let p = l.patch.clone();
            let r = patch(&Recipe::identity(), &p).unwrap_or_else(|e| panic!("{}: {e}", l.id));
            assert_eq!(summary(&r), p, "{}: a value was clamped or is a default", l.id);
            let d = &p["develop"];
            if l.id != "natural" {
                let styled = ["grading", "curve", "hsl"].iter().any(|k| d.get(k).is_some()) || d["basic"].get("saturation").is_some();
                assert!(styled, "{} sets grading, curve, hsl or saturation", l.id);
            }
            if l.mono {
                assert_eq!(d["basic"]["saturation"], json!(-1.0), "{} is grey", l.id);
                assert!(l.hues.is_empty(), "{}", l.id);
            }
            assert!(!l.intent.is_empty() && !l.name.is_empty());
        }
    }

    #[test]
    fn low_key_darkens_without_crushing_a_dark_scene() {
        // A dark forest already sits in the lower quarter: stacking exposure, shadows, blacks and a
        // dropped curve flattened it to black. The shadows keep detail: the curve lifts the
        // quarter tone, and neither shadows nor blacks go down by much.
        let p = by_id("low-key").unwrap().patch.clone();
        let b = &p["develop"]["basic"];
        assert!(b["exposure"].as_f64().unwrap() >= -0.3);
        assert!(b["shadows"].is_null(), "no extra shadow drop");
        assert!(b["blacks"].as_f64().unwrap() >= -0.1);
        let curve = p["develop"]["curve"]["master"].as_array().unwrap();
        assert!(curve[1][1].as_f64().unwrap() >= curve[1][0].as_f64().unwrap() - 0.06, "the quarter tone stays near where it was");
        assert!(curve[0][1].as_f64().unwrap() >= 0.02, "the black point is lifted a little");
    }

    #[test]
    fn the_resource_lists_every_look_with_its_patch() {
        let r = resource_of(catalogue());
        let looks = r["looks"].as_array().unwrap();
        assert_eq!(looks.len(), 12);
        assert!(looks.iter().all(|l| l["id"].is_string() && l["intent"].is_string() && l["patch"]["develop"].is_object()));
    }

    // Plugin looks (Round 23 B2)

    fn teal_json() -> Value {
        let b = by_id("teal-orange").unwrap();
        json!({ "intent": b.intent, "hues": b.hues, "mono": false, "key": "mid", "skinSafe": true, "likesSky": true })
    }

    fn contrast(v: f32) -> photos_core::edits::Develop {
        photos_core::edits::Develop { basic: photos_core::edits::Basic { contrast: v, ..Default::default() }, ..Default::default() }
    }

    fn plugin_teal(id: &str) -> Look {
        plugin_look(id, "Plugin teal", &teal_json(), &contrast(0.2), &[Panel::Basic]).unwrap()
    }

    fn colours() -> Vec<crate::mcp::fit::PhotoColour> {
        use crate::mcp::fit::PhotoColour;
        let mut warm_sky = [0.0; 12];
        (warm_sky[1], warm_sky[7], warm_sky[4]) = (0.4, 0.4, 0.2);
        let mut forest = [0.0; 12];
        (forest[7], forest[8], forest[0]) = (0.7, 0.2, 0.1);
        vec![
            PhotoColour { hues: warm_sky, chroma_share: 0.6, key: 0.5, contrast: 0.8, sat: 0.35, skin: true, sky: 0.3 },
            PhotoColour { hues: forest, chroma_share: 0.1, key: 0.35, contrast: 0.85, sat: 0.05, skin: false, sky: 0.0 },
            PhotoColour { key: 0.5, contrast: 0.6, ..Default::default() },
        ]
    }

    #[test]
    fn plugin_look_scored_like_builtins() {
        let builtin = by_id("teal-orange").unwrap();
        let plugin = plugin_teal("plugin:com.x/teal.json");
        for c in colours() {
            let (a, b) = (crate::mcp::fit::score(builtin, &c), crate::mcp::fit::score(&plugin, &c));
            assert_eq!(a.score, b.score, "{c:?}");
            assert_eq!(a.reasons, b.reasons);
        }
        assert_eq!(plugin.id, "plugin:com.x/teal.json");
        assert_eq!(plugin.patch, json!({ "develop": { "basic": serde_json::to_value(contrast(0.2).basic).unwrap() } }));
        // The patch is a valid starting point for the assistant.
        patch(&Recipe::identity(), &plugin.patch).unwrap();
    }

    #[test]
    fn plugin_look_never_safe() {
        assert!(!plugin_teal("plugin:com.x/a.json").safe);
        let mut with_safe = teal_json();
        with_safe["safe"] = json!(true);
        let err = plugin_look("plugin:com.x/a.json", "A", &with_safe, &contrast(0.2), &[Panel::Basic]).unwrap_err();
        assert!(err.contains("safe"), "{err}");
        // Even a hand-made look that says it is safe is not, once it joins the catalogue.
        let sneaky = Look { safe: true, ..plugin_teal("plugin:com.x/b.json") };
        let all = with_plugins(vec![sneaky]);
        assert_eq!(all.iter().filter(|l| l.safe).map(|l| l.id.as_str()).collect::<Vec<_>>(), ["natural"]);
        // A plugin look can't take a built-in's id.
        assert_eq!(with_plugins(vec![Look { id: "natural".into(), ..plugin_teal("x") }]).len(), 12);
    }

    #[test]
    fn plugin_look_fields_are_checked() {
        let panels = [Panel::Basic];
        let try_with = |j: Value| plugin_look("plugin:com.x/a.json", "A", &j, &contrast(0.2), &panels);
        assert!(try_with(json!({ "intent": "" })).is_err(), "an intent is needed");
        assert!(try_with(json!({ "intent": "x", "key": "huge" })).is_err());
        assert!(try_with(json!({ "intent": "x", "hues": [400.0] })).is_err());
        assert!(try_with(json!({ "intent": "x", "colour": 1 })).is_err(), "unknown keys are refused");
        assert!(try_with(json!({ "intent": "x", "mono": true, "hues": [30.0] })).is_err(), "a mono look has no hues");
        let minimal = try_with(json!({ "intent": "Soft and warm" })).unwrap();
        assert!(minimal.hues.is_empty() && !minimal.mono && minimal.key == Key::Mid && minimal.skin_safe && !minimal.likes_sky);
        // Masks belong to one photo: they never go into a look's patch.
        let masked = plugin_look("plugin:com.x/m.json", "M", &json!({ "intent": "x" }), &contrast(0.2), &[Panel::Basic, Panel::Masks]).unwrap();
        assert!(masked.patch["develop"].get("masks").is_none());
        assert!(plugin_look("plugin:com.x/n.json", "N", &json!({ "intent": "x" }), &contrast(0.2), &[Panel::Masks]).is_err(), "nothing but masks is no look");
    }

    #[test]
    fn offer_limit_still_four() {
        use crate::mcp::fit::{offer, score};
        let plugin: Vec<Look> = (0..10).map(|n| plugin_teal(&format!("plugin:com.x/{n}.json"))).collect();
        let all = with_plugins(plugin);
        assert_eq!(all.len(), 22);
        for c in colours() {
            let fits: Vec<_> = all.iter().map(|l| score(l, &c)).collect();
            let offered = offer(&fits);
            assert!(offered.len() <= 4, "{offered:?}");
            assert_eq!(offered[0], "natural");
        }
        let c = &colours()[0];
        let fits: Vec<_> = all.iter().map(|l| score(l, c)).collect();
        assert_eq!(offer(&fits).len(), 4, "plenty of good fits still gives four");
    }
}
