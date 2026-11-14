//! A Develop recipe as an AI assistant reads and writes it (Round 16): a summary of what
//! differs from no edit, and a JSON merge-patch that is clamped to the sliders' ranges.
//!
//! Pure: the MCP server in the binary loads and saves recipes; this only turns them into
//! JSON and back. Masks are not patched here (the `mask` tool owns them), nor are the
//! version, a video's trim or the *Auto* flag.

use serde_json::{Map, Value};

use crate::edits::{LocalAdjust, Recipe, HSL_BANDS};

/// One slider: its path in the patch JSON (`hsl.<band>` stands for each band), its range,
/// and what it does to the picture. Clamping is `Recipe::normalized`'s; a test keeps this
/// table in step with it, and the `edit` tool's description is made from it.
pub struct Range {
    pub path: &'static str,
    pub min: f32,
    pub max: f32,
    pub meaning: &'static str,
}

const fn r(path: &'static str, min: f32, max: f32, meaning: &'static str) -> Range {
    Range { path, min, max, meaning }
}

pub const RANGES: &[Range] = &[
    r("develop.basic.temperature", -100.0, 100.0, "white balance, + warmer (yellow), − cooler (blue)"),
    r("develop.basic.tint", -100.0, 100.0, "white balance, + magenta, − green"),
    r("develop.basic.exposure", -5.0, 5.0, "brightness in stops; ±0.3 is a visible change"),
    r("develop.basic.contrast", -1.0, 1.0, "midtone contrast"),
    r("develop.basic.highlights", -1.0, 1.0, "bright areas; − recovers skies and skin shine"),
    r("develop.basic.shadows", -1.0, 1.0, "dark areas; + opens them"),
    r("develop.basic.whites", -1.0, 1.0, "white point; + until just before highlights clip"),
    r("develop.basic.blacks", -1.0, 1.0, "black point; − for depth"),
    r("develop.basic.texture", -1.0, 1.0, "fine detail; keep under 0.3 on skin"),
    r("develop.basic.clarity", -1.0, 1.0, "midtone local contrast; strong values look gritty"),
    r("develop.basic.dehaze", -1.0, 1.0, "removes (+) or adds (−) haze"),
    r("develop.basic.vibrance", -1.0, 1.0, "saturation of muted colours, spares skin"),
    r("develop.basic.saturation", -1.0, 1.0, "saturation of every colour"),
    r("develop.hsl.<band>.h", -1.0, 1.0, "hue shift of one colour band"),
    r("develop.hsl.<band>.s", -1.0, 1.0, "saturation of one colour band"),
    r("develop.hsl.<band>.l", -1.0, 1.0, "luminance of one colour band"),
    r("develop.grading.shadows.hue", 0.0, 360.0, "grading tint of the shadows, degrees"),
    r("develop.grading.shadows.sat", 0.0, 1.0, "strength of the shadows' tint; 0.1–0.3 is subtle"),
    r("develop.grading.shadows.lum", -1.0, 1.0, "shadows lighter or darker"),
    r("develop.grading.midtones.hue", 0.0, 360.0, "grading tint of the midtones, degrees"),
    r("develop.grading.midtones.sat", 0.0, 1.0, "strength of the midtones' tint"),
    r("develop.grading.midtones.lum", -1.0, 1.0, "midtones lighter or darker"),
    r("develop.grading.highlights.hue", 0.0, 360.0, "grading tint of the highlights, degrees"),
    r("develop.grading.highlights.sat", 0.0, 1.0, "strength of the highlights' tint"),
    r("develop.grading.highlights.lum", -1.0, 1.0, "highlights lighter or darker"),
    r("develop.grading.blending", 0.0, 1.0, "how far the three ranges overlap"),
    r("develop.grading.balance", -1.0, 1.0, "moves the shadow/highlight split"),
    r("develop.detail.sharpen.amount", 0.0, 1.5, "sharpening strength"),
    r("develop.detail.sharpen.radius", 0.5, 3.0, "sharpening radius in pixels"),
    r("develop.detail.sharpen.detail", 0.0, 1.0, "fine detail sharpened against edges"),
    r("develop.detail.sharpen.masking", 0.0, 1.0, "flat areas left unsharpened"),
    r("develop.detail.denoise.strength", 0.0, 1.0, "ML noise reduction blended in"),
    r("develop.effects.vignette.amount", -1.0, 1.0, "corners, − darker, + lighter"),
    r("develop.effects.vignette.midpoint", 0.0, 1.0, "how far in the vignette reaches"),
    r("develop.effects.vignette.feather", 0.0, 1.0, "softness of the vignette's edge"),
    r("develop.effects.grain.amount", 0.0, 1.0, "film grain"),
    r("develop.effects.grain.size", 0.0, 1.0, "grain size"),
    r("develop.effects.motion_blur.amount", 0.0, 1.0, "motion blur streak length"),
    r("develop.effects.motion_blur.angle", -90.0, 90.0, "motion blur direction in degrees, 0 horizontal"),
    r("develop.effects.glow.amount", 0.0, 1.0, "highlights bloom softly"),
    r("develop.effects.glow.size", 0.0, 1.0, "glow radius"),
    r("develop.effects.halation.amount", 0.0, 1.0, "film's red fringe round bright highlights"),
    r("develop.effects.halation.size", 0.0, 1.0, "halation radius"),
    r("straighten", -45.0, 45.0, "rotation in degrees"),
    r("rotate", 0.0, 270.0, "quarter turns: 0, 90, 180, 270"),
    r("crop.x", 0.0, 1.0, "crop {x,y,w,h} as fractions of the turned photo; null removes it"),
];

/// Point curves (`develop.curve.{master,r,g,b}`) and flips take no range; the description
/// names them by hand.
pub const OTHER_FIELDS: &str = "develop.curve.{master|r|g|b}: [[x,y],…] in 0–1, [] is straight; flip_h, flip_v: bool";

/// A refused patch: an unknown or forbidden field, or a value of the wrong type.
#[derive(Debug, Clone, PartialEq)]
pub struct PatchError(pub String);

impl std::fmt::Display for PatchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// The recipe as JSON with every field present: HSL keyed by band name, an empty Develop
/// and the default denoise when there are none, masks, trim, version and *Auto* left out.
fn full_json(recipe: &Recipe) -> Value {
    let mut r = recipe.clone();
    let mut d = r.develop.take().unwrap_or_default();
    d.detail.denoise.get_or_insert_with(Default::default);
    r.develop = Some(d);
    let mut v = serde_json::to_value(&r).expect("a recipe serialises");
    let o = v.as_object_mut().expect("a recipe is an object");
    for k in ["version", "trim"] {
        o.remove(k);
    }
    let dev = o.get_mut("develop").and_then(Value::as_object_mut).expect("develop is set");
    dev.remove("masks");
    dev.remove("auto");
    if let Some(Value::Array(bands)) = dev.remove("hsl") {
        let named: Map<String, Value> = HSL_BANDS.iter().map(|b| b.to_string()).zip(bands).collect();
        dev.insert("hsl".into(), Value::Object(named));
    }
    v
}

fn round2(v: &Value) -> Value {
    match v {
        Value::Number(n) => n.as_f64().map(|f| serde_json::json!((f * 100.0).round() / 100.0)).unwrap_or_else(|| v.clone()),
        Value::Array(a) => Value::Array(a.iter().map(round2).collect()),
        Value::Object(o) => Value::Object(o.iter().map(|(k, x)| (k.clone(), round2(x))).collect()),
        _ => v.clone(),
    }
}

/// What differs between `v` and `base`, rounded; `None` when nothing does.
fn diff(v: &Value, base: &Value) -> Option<Value> {
    match (v, base) {
        (Value::Object(o), Value::Object(b)) => {
            let m: Map<String, Value> = o.iter().filter_map(|(k, x)| diff(x, b.get(k).unwrap_or(&Value::Null)).map(|d| (k.clone(), d))).collect();
            (!m.is_empty()).then_some(Value::Object(m))
        }
        _ => {
            let (a, b) = (round2(v), round2(base));
            (a != b).then_some(a)
        }
    }
}

/// Only the fields that differ from no edit, rounded to 2 dp; `{}` for an unedited photo.
/// The number of masks is added as `masks` when there are any.
pub fn summary(recipe: &Recipe) -> Value {
    let recipe = recipe.clone().normalized();
    let mut out = diff(&full_json(&recipe), &full_json(&Recipe::identity())).unwrap_or_else(|| Value::Object(Map::new()));
    let masks = recipe.develop.as_ref().map_or(0, |d| d.masks.len());
    if masks > 0 {
        out.as_object_mut().expect("an object").insert("masks".into(), masks.into());
    }
    out
}

/// RFC 7386 merge of `patch` into `target`, refusing a key `target` does not have (`crop`
/// is the one optional object: null in `target`, it takes any of x, y, w, h).
fn merge(target: &mut Value, patch: &Value, path: &str) -> Result<(), PatchError> {
    let Value::Object(p) = patch else {
        return Err(PatchError(format!("{path} must be an object")));
    };
    for (k, pv) in p {
        let at = if path.is_empty() { k.clone() } else { format!("{path}.{k}") };
        let t = target.as_object_mut().expect("merged into an object");
        if at == "crop" {
            match pv {
                Value::Null => {
                    t.insert(k.clone(), Value::Null);
                }
                Value::Object(_) => {
                    let slot = t.entry(k.clone()).or_insert(Value::Null);
                    if slot.is_null() {
                        *slot = serde_json::json!({ "x": 0.0, "y": 0.0, "w": 1.0, "h": 1.0 });
                    }
                    merge(slot, pv, &at)?;
                }
                _ => return Err(PatchError("crop must be {x,y,w,h} or null".into())),
            }
            continue;
        }
        // The effects left out of a recipe while off: filled in with their defaults.
        if path == "develop.effects" && !t.contains_key(k) {
            let off = serde_json::to_value(crate::edits::Effects::default()).expect("effects serialise");
            let fill = match k.as_str() {
                "motion_blur" => serde_json::to_value(crate::edits::MotionBlur::default()).ok(),
                "glow" => serde_json::to_value(crate::edits::Glow::default()).ok(),
                "halation" => serde_json::to_value(crate::edits::Halation::default()).ok(),
                _ => off.get(k).cloned(),
            };
            if let Some(v) = fill {
                t.insert(k.clone(), v);
            }
        }
        let Some(slot) = t.get_mut(k) else {
            return Err(PatchError(format!("unknown field {at}")));
        };
        match (slot.is_object(), pv) {
            (true, Value::Object(_)) => merge(slot, pv, &at)?,
            (true, _) => return Err(PatchError(format!("{at} must be an object"))),
            (false, Value::Object(_)) => return Err(PatchError(format!("{at} is not an object"))),
            (false, Value::Null) => return Err(PatchError(format!("{at} cannot be null"))),
            (false, _) => {
                let same_kind = matches!((&*slot, pv), (Value::Number(_), Value::Number(_)) | (Value::Bool(_), Value::Bool(_)) | (Value::Array(_), Value::Array(_)));
                if !same_kind {
                    return Err(PatchError(format!("{at} has the wrong type")));
                }
                *slot = pv.clone();
            }
        }
    }
    Ok(())
}

/// `recipe` with `patch` merged in (the shape of `summary`'s output), every field then
/// clamped to its slider's range. Masks, trim and *Auto* are kept as they were.
pub fn patch(recipe: &Recipe, patch: &Value) -> Result<Recipe, PatchError> {
    let mut v = full_json(recipe);
    merge(&mut v, patch, "")?;
    let o = v.as_object_mut().expect("an object");
    let dev = o.get_mut("develop").and_then(Value::as_object_mut).expect("develop is set");
    if let Some(Value::Object(named)) = dev.remove("hsl") {
        let bands: Vec<Value> = HSL_BANDS.iter().map(|b| named[*b].clone()).collect();
        dev.insert("hsl".into(), Value::Array(bands));
    }
    let old = recipe.develop.clone().unwrap_or_default();
    dev.insert("masks".into(), serde_json::to_value(&old.masks).map_err(|e| PatchError(e.to_string()))?);
    dev.insert("auto".into(), old.auto.into());
    o.insert("version".into(), 3.into());
    o.insert("trim".into(), serde_json::to_value(&recipe.trim).map_err(|e| PatchError(e.to_string()))?);
    let mut out: Recipe = serde_json::from_value(v).map_err(|e| PatchError(e.to_string()))?;
    // A denoise the patch did not touch stays absent, as it was.
    if recipe.develop.as_ref().is_none_or(|d| d.detail.denoise.is_none()) {
        if let Some(d) = &mut out.develop {
            if d.detail.denoise.as_ref().is_some_and(|n| *n == Default::default()) {
                d.detail.denoise = None;
            }
        }
    }
    Ok(out.normalized())
}

/// A mask's adjustments as JSON with every field present, in the photo's shape: HSL keyed by
/// band name, its own sharpening at the defaults when it has none.
fn full_local_json(a: &LocalAdjust) -> Value {
    use crate::edits::{Curve, Glow, Grading, Grain, Halation, Hsl, LocalSharpen, MotionBlur};
    let to = |v: Result<Value, serde_json::Error>| v.expect("a mask's adjustments serialise");
    let mut v = to(serde_json::to_value(a));
    let o = v.as_object_mut().expect("an object");
    let defaults = [
        ("vibrance", Value::from(0.0)),
        ("curve", to(serde_json::to_value(Curve::default()))),
        ("hsl", to(serde_json::to_value([Hsl::default(); 8]))),
        ("grading", to(serde_json::to_value(Grading::default()))),
        ("sharpen", to(serde_json::to_value(LocalSharpen::default()))),
        ("grain", to(serde_json::to_value(Grain::default()))),
        ("glow", to(serde_json::to_value(Glow::default()))),
        ("halation", to(serde_json::to_value(Halation::default()))),
        ("motion_blur", to(serde_json::to_value(MotionBlur::default()))),
    ];
    for (k, d) in defaults {
        if o.get(k).is_none_or(Value::is_null) {
            o.insert(k.into(), d);
        }
    }
    if let Some(Value::Array(bands)) = o.remove("hsl") {
        let named: Map<String, Value> = HSL_BANDS.iter().map(|b| b.to_string()).zip(bands).collect();
        o.insert("hsl".into(), Value::Object(named));
    }
    v
}

/// A mask's adjustments with `patch` merged in (the shape of [`local_summary`]'s output), as
/// [`patch`] does for the photo: unknown fields refused, objects merged. The caller clamps.
/// Sharpening of its own stays absent unless the patch gives some.
pub fn patch_local(a: &LocalAdjust, patch: &Value) -> Result<LocalAdjust, PatchError> {
    let mut v = full_local_json(a);
    merge(&mut v, patch, "adjust")?;
    let o = v.as_object_mut().expect("an object");
    if let Some(Value::Object(named)) = o.remove("hsl") {
        let bands: Vec<Value> = HSL_BANDS.iter().map(|b| named[*b].clone()).collect();
        o.insert("hsl".into(), Value::Array(bands));
    }
    if a.sharpen.is_none() && patch.get("sharpen").is_none() {
        o.remove("sharpen");
    }
    serde_json::from_value(v).map_err(|e| PatchError(e.to_string()))
}

/// Only what a mask's adjustments change, rounded to 2 dp, in the photo's shape; `{}` for
/// none. Sharpening of its own shows whenever it is set, defaults or not.
pub fn local_summary(a: &LocalAdjust) -> Value {
    let mut out = diff(&full_local_json(a), &full_local_json(&LocalAdjust::default())).unwrap_or_else(|| Value::Object(Map::new()));
    if let Some(s) = &a.sharpen {
        let own = round2(&serde_json::to_value(s).expect("sharpening serialises"));
        out.as_object_mut().expect("an object").entry("sharpen").or_insert(own);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn an_unedited_photo_summarises_as_nothing() {
        assert_eq!(summary(&Recipe::identity()), json!({}));
    }

    #[test]
    fn a_patch_reads_back_in_the_summary_and_leaves_other_fields() {
        let a = patch(&Recipe::identity(), &json!({ "develop": { "basic": { "exposure": 0.333, "temperature": 12 } } })).unwrap();
        let b = patch(&a, &json!({ "develop": { "basic": { "shadows": 0.2 }, "hsl": { "orange": { "s": -0.1 } } } })).unwrap();
        assert_eq!(
            summary(&b),
            json!({ "develop": { "basic": { "exposure": 0.33, "temperature": 12.0, "shadows": 0.2 }, "hsl": { "orange": { "s": -0.1 } } } })
        );
        assert_eq!(b.develop.unwrap().detail.denoise, None, "an untouched denoise stays off");
    }

    #[test]
    fn values_are_clamped_to_the_sliders() {
        let a = patch(&Recipe::identity(), &json!({ "develop": { "basic": { "exposure": 9, "texture": -3 } }, "straighten": 80 })).unwrap();
        assert_eq!(a.basic().exposure, 5.0);
        assert_eq!(a.basic().texture, -1.0);
        assert_eq!(a.straighten, 45.0);
    }

    #[test]
    fn the_ranges_table_matches_the_clamping() {
        for range in RANGES.iter().filter(|r| !r.path.contains('<') && !r.path.contains("hue") && !r.path.starts_with("crop") && r.path != "rotate") {
            let nest = |v: f32| range.path.rsplit('.').fold(json!(v), |acc, k| json!({ k: acc }));
            let hi = full_json(&patch(&Recipe::identity(), &nest(range.max + 10.0)).unwrap());
            let lo = full_json(&patch(&Recipe::identity(), &nest(range.min - 10.0)).unwrap());
            let at = |v: &Value| range.path.split('.').fold(v.clone(), |acc, k| acc[k].clone()).as_f64().map(|f| f as f32);
            // An optional effect clamped to off is left out of the recipe.
            let low = at(&lo).or(range.path.starts_with("develop.effects.").then_some(range.min));
            assert_eq!((low, at(&hi)), (Some(range.min), Some(range.max)), "{}", range.path);
        }
    }

    #[test]
    fn crop_is_set_and_removed() {
        let a = patch(&Recipe::identity(), &json!({ "crop": { "x": 0.1, "w": 0.8 } })).unwrap();
        assert_eq!(summary(&a)["crop"], json!({ "x": 0.1, "y": 0.0, "w": 0.8, "h": 1.0 }));
        assert_eq!(patch(&a, &json!({ "crop": null })).unwrap().crop, None);
    }

    #[test]
    fn masks_and_unknown_fields_are_refused() {
        for bad in [
            json!({ "develop": { "masks": [] } }),
            json!({ "develop": { "auto": true } }),
            json!({ "version": 3 }),
            json!({ "develop": { "basic": { "glow": 1 } } }),
            json!({ "develop": { "basic": { "exposure": "bright" } } }),
            json!({ "develop": { "basic": 1 } }),
            json!([1]),
        ] {
            assert!(patch(&Recipe::identity(), &bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn masks_survive_a_patch() {
        let mut r = Recipe::identity();
        let mask: crate::edits::Mask = serde_json::from_value(json!({
            "id": 1, "name": "Face", "shapes": [{ "kind": "radial", "add": true, "center": [0.5, 0.5], "radius": [0.2, 0.2], "angle": 0, "feather": 0.5, "inside": true }],
            "adjust": { "exposure": 0.5 }
        }))
        .unwrap();
        r.develop = Some(crate::edits::Develop { masks: vec![mask], ..Default::default() });
        let out = patch(&r, &json!({ "develop": { "basic": { "exposure": 0.1 } } })).unwrap();
        assert_eq!(out.develop.as_ref().unwrap().masks.len(), 1);
        assert_eq!(summary(&out)["masks"], json!(1));
    }

    #[test]
    fn a_masks_adjust_takes_every_local_tool_in_the_photos_shape() {
        use crate::edits::{Glow, LocalAdjust, LocalSharpen};
        let a = LocalAdjust { exposure: 0.5, glow: Glow { amount: 0.2, size: 0.4 }, ..Default::default() };
        let out = patch_local(
            &a,
            &json!({ "vibrance": 0.3, "curve": { "master": [[0.0, 0.1], [1.0, 0.9]] }, "hsl": { "blue": { "s": -0.4 } }, "grading": { "shadows": { "hue": 210, "sat": 0.3 } }, "sharpen": { "radius": 2.0 }, "glow": { "size": 0.8 }, "grain": { "amount": 0.3 } }),
        )
        .unwrap();
        assert_eq!(out.exposure, 0.5, "left out, kept");
        assert_eq!(out.vibrance, 0.3);
        assert_eq!(out.curve.master, vec![[0.0, 0.1], [1.0, 0.9]]);
        let blue = HSL_BANDS.iter().position(|b| *b == "blue").unwrap();
        assert_eq!(out.hsl[blue].s, -0.4);
        assert_eq!((out.grading.shadows.hue, out.grading.shadows.sat), (210.0, 0.3));
        assert_eq!(out.sharpen, Some(LocalSharpen { radius: 2.0, ..Default::default() }));
        assert_eq!(out.glow, Glow { amount: 0.2, size: 0.8 }, "merged, not replaced");
        assert_eq!(out.grain.amount, 0.3);
        // Untouched, the sharpening of its own stays absent.
        assert_eq!(patch_local(&a, &json!({ "exposure": 1.0 })).unwrap().sharpen, None);
        assert!(patch_local(&a, &json!({ "warmth": 1.0 })).unwrap_err().0.contains("warmth"));
        assert!(patch_local(&a, &json!({ "glow": 1.0 })).is_err());
        // The summary: only what differs, in the same shape.
        let s = local_summary(&out);
        assert_eq!(s["exposure"], json!(0.5));
        assert_eq!(s["hsl"], json!({ "blue": { "s": -0.4 } }));
        assert_eq!(s["glow"], json!({ "amount": 0.2, "size": 0.8 }));
        assert_eq!(s["sharpen"]["radius"], json!(2.0));
        assert!(s.get("tint").is_none() && s.get("halation").is_none());
        let own = LocalAdjust { sharpness: 0.4, sharpen: Some(LocalSharpen::default()), ..Default::default() };
        assert!(local_summary(&own).get("sharpen").is_some(), "its own sharpening shows even at the defaults");
        assert_eq!(local_summary(&LocalAdjust::default()), json!({}));
    }
}
