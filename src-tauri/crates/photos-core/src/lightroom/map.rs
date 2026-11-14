//! Camera Raw settings → Lumen's `Develop`, by the Round 11 spec's mapping table.

use std::cell::RefCell;
use std::collections::{BTreeSet, HashSet};

use super::{Converted, Geometry, Settings, Value};
use crate::develop::Panel;
use crate::edits::{Crop, Denoise, Develop, LocalAdjust, Mask, MaskShape};

/// Lightroom's HSL bands, in Lumen's order (`edits::HSL_BANDS`).
const BANDS: [&str; 8] = ["Red", "Orange", "Yellow", "Green", "Aqua", "Blue", "Purple", "Magenta"];

/// Settings that say nothing about the picture: what the preset is, which version wrote it.
const QUIET: [&str; 27] = [
    "Version", "ProcessVersion", "Name", "Group", "UUID", "Cluster", "PresetType", "Copyright", "ContactInfo", "Description", "HasSettings", "AlreadyApplied",
    "RawFileName", "CameraModelRestriction", "CompatibleVersion", "ToneCurveName", "ToneCurveName2010", "ToneCurveName2012", "CameraProfileDigest", "OverrideLookVignette",
    "SortName", "ShortName", "ProfileName", "Amount", "Look", "LookTable", "AutoGrayscaleMix",
];

/// Settings whose default is not zero: at their default they change nothing.
const DEFAULTS: [(&str, f64); 14] = [
    ("PerspectiveScale", 100.0),
    ("DefringePurpleHueLo", 30.0),
    ("DefringePurpleHueHi", 70.0),
    ("DefringeGreenHueLo", 40.0),
    ("DefringeGreenHueHi", 60.0),
    ("VignetteMidpoint", 50.0),
    ("ColorNoiseReduction", 25.0),
    ("ColorNoiseReductionDetail", 50.0),
    ("ColorNoiseReductionSmoothness", 50.0),
    ("LuminanceNoiseReductionDetail", 50.0),
    ("GrainFrequency", 50.0),
    ("PostCropVignetteStyle", 1.0),
    ("PostCropVignetteRoundness", 0.0),
    ("ColorGradeGlobalHue", 0.0),
];

/// Camera Raw settings → Lumen's Develop. `as_shot_kelvin` is the photo's own white balance,
/// for a Kelvin temperature (5500 K when unknown).
pub fn convert(s: &Settings, as_shot_kelvin: Option<f32>) -> Converted {
    let r = Reader { s, used: RefCell::new(BTreeSet::new()) };
    let mut d = Develop::default();
    let mut touched: HashSet<Panel> = HashSet::new();
    let mut skipped: Vec<String> = Vec::new();

    basic(&r, &mut d, &mut touched, &mut skipped, as_shot_kelvin);
    curve(&r, &mut d, &mut touched);
    for (i, band) in BANDS.iter().enumerate() {
        let h = &mut d.hsl[i];
        let any = [r.set(&format!("HueAdjustment{band}"), 0.01, &mut h.h), r.set(&format!("SaturationAdjustment{band}"), 0.01, &mut h.s), r.set(&format!("LuminanceAdjustment{band}"), 0.01, &mut h.l)];
        if any.contains(&true) {
            touched.insert(Panel::Hsl);
        }
    }
    grading(&r, &mut d, &mut touched, &mut skipped);
    detail(&r, &mut d, &mut touched);
    let e = &mut d.effects;
    let effects = [
        r.set("PostCropVignetteAmount", 0.01, &mut e.vignette.amount),
        r.set("PostCropVignetteMidpoint", 0.01, &mut e.vignette.midpoint),
        r.set("PostCropVignetteFeather", 0.01, &mut e.vignette.feather),
        r.set("GrainAmount", 0.01, &mut e.grain.amount),
        r.set("GrainSize", 0.01, &mut e.grain.size),
    ];
    if effects.contains(&true) {
        touched.insert(Panel::Effects);
    }
    let geometry = geometry(&r);
    d.masks = masks(&r, &mut skipped);
    if !d.masks.is_empty() {
        touched.insert(Panel::Masks);
    }

    // Whatever is left has no Lumen equivalent, unless it is at a value that does nothing.
    let used = r.used.into_inner();
    for (k, v) in s {
        if !used.contains(k.as_str()) && !QUIET.contains(&k.as_str()) && does_something(k, v) {
            skipped.push(k.clone());
        }
    }
    let mut seen = BTreeSet::new();
    skipped.retain(|x| seen.insert(x.clone()));
    let text = |k: &str| s.get(k).and_then(|v| v.str().map(str::to_string).or_else(|| v.num().map(|n| n.to_string())));
    Converted {
        develop: d.normalized(),
        geometry,
        panels: Panel::ALL.into_iter().filter(|p| touched.contains(p)).collect(),
        skipped,
        name: text("Name").filter(|n| !n.is_empty()),
        group: text("Group").filter(|n| !n.is_empty()),
    }
}

/// Reads settings and remembers which it read.
struct Reader<'a> {
    s: &'a Settings,
    used: RefCell<BTreeSet<&'a str>>,
}

impl<'a> Reader<'a> {
    fn get(&self, key: &str) -> Option<&'a Value> {
        let (k, v) = self.s.get_key_value(key)?;
        self.used.borrow_mut().insert(k.as_str());
        Some(v)
    }

    fn num(&self, key: &str) -> Option<f32> {
        self.get(key).and_then(Value::num).map(|n| n as f32)
    }

    fn bool(&self, key: &str) -> Option<bool> {
        self.get(key).and_then(Value::bool)
    }

    /// `*into = value · scale` when the setting is there; whether it was.
    fn set(&self, key: &str, scale: f32, into: &mut f32) -> bool {
        match self.num(key) {
            Some(v) => {
                *into = if scale == 0.01 { v / 100.0 } else { v * scale };
                true
            }
            None => false,
        }
    }
}

fn basic(r: &Reader, d: &mut Develop, touched: &mut HashSet<Panel>, skipped: &mut Vec<String>, as_shot: Option<f32>) {
    let version = r.get("ProcessVersion").and_then(Value::num).unwrap_or(11.0);
    let old = version < 10.0;
    if old {
        skipped.push(if version < 6.0 { "Process version 2003 (approximated)" } else { "Process version 2010 (approximated)" }.into());
    }
    let b = &mut d.basic;
    let mut any = [
        r.set("Exposure2012", 1.0, &mut b.exposure),
        r.set("Contrast2012", 0.01, &mut b.contrast),
        r.set("Highlights2012", 0.01, &mut b.highlights),
        r.set("Shadows2012", 0.01, &mut b.shadows),
        r.set("Whites2012", 0.01, &mut b.whites),
        r.set("Blacks2012", 0.01, &mut b.blacks),
        r.set("Texture", 0.01, &mut b.texture),
        r.set("Clarity2012", 0.01, &mut b.clarity),
        r.set("Dehaze", 0.01, &mut b.dehaze),
        r.set("Vibrance", 0.01, &mut b.vibrance),
        r.set("Saturation", 0.01, &mut b.saturation),
    ]
    .contains(&true);
    if old {
        // Process version 2010's sliders, near enough: Contrast centred on 25, Recovery pulls
        // highlights down, Fill Light lifts shadows, its "Shadows" are the blacks (default 5).
        let pv2010 = [
            r.set("Exposure", 1.0, &mut b.exposure),
            r.num("Contrast").map(|v| b.contrast = (v - 25.0) / 100.0).is_some(),
            r.num("Recovery").map(|v| b.highlights = -v / 100.0).is_some(),
            r.set("FillLight", 0.01, &mut b.shadows),
            r.num("Shadows").map(|v| b.blacks = -(v - 5.0) / 100.0).is_some(),
            r.set("Clarity", 0.01, &mut b.clarity),
        ];
        any |= pv2010.contains(&true);
    }
    // White balance: relative on a JPEG, in Kelvin on a RAW; "As Shot" is no change.
    let wb = r.get("WhiteBalance").and_then(Value::str);
    let incremental = [r.set("IncrementalTemperature", 1.0, &mut b.temperature), r.set("IncrementalTint", 1.0, &mut b.tint)].contains(&true);
    match wb {
        Some("As Shot") => {
            r.get("Temperature");
            r.get("Tint");
            any = true;
        }
        Some("Auto") => {
            r.get("Temperature");
            r.get("Tint");
            skipped.push("White balance Auto".into());
        }
        _ if !incremental => {
            if let Some(k) = r.num("Temperature").filter(|k| *k > 0.0) {
                let shot = as_shot.filter(|k| *k > 0.0).unwrap_or(5500.0);
                b.temperature = (100.0 * (k / shot).log2() / 1.2).clamp(-100.0, 100.0);
                skipped.push("Temperature (approximated)".into());
                any = true;
            }
            if let Some(t) = r.num("Tint") {
                b.tint = (t / 1.5).clamp(-100.0, 100.0);
                skipped.push("Tint (approximated)".into());
                any = true;
            }
        }
        _ => {}
    }
    if wb.is_some() || incremental {
        any = true;
    }
    if r.bool("ConvertToGrayscale") == Some(true) {
        b.saturation = -1.0;
        b.vibrance = 0.0;
        skipped.push("Black & white (approximated)".into());
        any = true;
    }
    if any {
        touched.insert(Panel::Basic);
    }
}

/// A Lightroom curve's points (x, y out of 255): a flat list of numbers, or "x, y" texts.
fn points(v: &Value) -> Option<Vec<[f32; 2]>> {
    let Value::List(items) = v else { return None };
    let mut out = Vec::new();
    if items.iter().all(|i| matches!(i, Value::Num(_))) {
        for pair in items.chunks(2) {
            if let [x, y] = pair {
                out.push([x.num()? as f32 / 255.0, y.num()? as f32 / 255.0]);
            }
        }
    } else {
        for item in items {
            let (x, y) = item.str()?.split_once(',')?;
            out.push([x.trim().parse::<f32>().ok()? / 255.0, y.trim().parse::<f32>().ok()? / 255.0]);
        }
    }
    Some(out)
}

/// `y` of a piecewise-linear curve through `p` (sorted by x) at `x`; the identity when empty.
fn at(p: &[[f32; 2]], x: f32) -> f32 {
    match p {
        [] => x,
        [only] => only[1],
        _ => {
            let i = p.iter().position(|q| q[0] >= x).unwrap_or(p.len() - 1).max(1);
            let (a, b) = (p[i - 1], p[i]);
            if (b[0] - a[0]).abs() < 1e-6 {
                b[1]
            } else {
                a[1] + (b[1] - a[1]) * ((x - a[0]) / (b[0] - a[0])).clamp(0.0, 1.0)
            }
        }
    }
}

fn curve(r: &Reader, d: &mut Develop, touched: &mut HashSet<Panel>) {
    let c = &mut d.curve;
    for (key, old, into) in [("ToneCurvePV2012", "ToneCurve", &mut c.master), ("ToneCurvePV2012Red", "", &mut c.r), ("ToneCurvePV2012Green", "", &mut c.g), ("ToneCurvePV2012Blue", "", &mut c.b)] {
        let found = r.get(key).or_else(|| (!old.is_empty()).then(|| r.get(old)).flatten());
        if let Some(p) = found.and_then(points) {
            *into = p;
            touched.insert(Panel::Curve);
        }
    }
    // The parametric sliders (−100..100) over their four regions, split at 25 / 50 / 75 by
    // default: five points, laid over the point curve.
    let keys = ["ParametricShadows", "ParametricDarks", "ParametricLights", "ParametricHighlights"];
    let amounts: Vec<Option<f32>> = keys.iter().map(|k| r.num(k)).collect();
    let splits = [r.num("ParametricShadowSplit").unwrap_or(25.0), r.num("ParametricMidtoneSplit").unwrap_or(50.0), r.num("ParametricHighlightSplit").unwrap_or(75.0)].map(|v| (v / 100.0).clamp(0.01, 0.99));
    if amounts.iter().any(Option::is_some) {
        touched.insert(Panel::Curve);
    }
    let a: Vec<f32> = amounts.iter().map(|v| v.unwrap_or(0.0) / 100.0).collect();
    if a.iter().all(|v| *v == 0.0) {
        return;
    }
    const LIFT: f32 = 0.2;
    let bend = |x: f32, k: f32| (x + LIFT * k).clamp(0.0, 1.0);
    let param = [[0.0, 0.0], [splits[0], bend(splits[0], (a[0] + a[1]) / 2.0)], [splits[1], bend(splits[1], (a[1] + a[2]) / 2.0)], [splits[2], bend(splits[2], (a[2] + a[3]) / 2.0)], [1.0, 1.0]];
    if c.master.is_empty() {
        c.master = param.to_vec();
    } else {
        // The point curve's own points, and the parametric ones not crowding them.
        let mut xs: Vec<f32> = c.master.iter().map(|p| p[0]).collect();
        let near = |x: f32| xs.iter().any(|m| (m - x).abs() < 0.02);
        let extra: Vec<f32> = param.iter().map(|p| p[0]).filter(|x| !near(*x)).collect();
        xs.extend(extra);
        xs.sort_by(f32::total_cmp);
        let master = c.master.clone();
        c.master = xs.into_iter().map(|x| [x, at(&param, at(&master, x))]).collect();
    }
}

fn grading(r: &Reader, d: &mut Develop, touched: &mut HashSet<Panel>, skipped: &mut Vec<String>) {
    let g = &mut d.grading;
    let any = [
        r.set("SplitToningShadowHue", 1.0, &mut g.shadows.hue),
        r.set("SplitToningShadowSaturation", 0.01, &mut g.shadows.sat),
        r.set("SplitToningHighlightHue", 1.0, &mut g.highlights.hue),
        r.set("SplitToningHighlightSaturation", 0.01, &mut g.highlights.sat),
        r.set("SplitToningBalance", 0.01, &mut g.balance),
        r.set("ColorGradeShadowLum", 0.01, &mut g.shadows.lum),
        r.set("ColorGradeHighlightLum", 0.01, &mut g.highlights.lum),
        r.set("ColorGradeMidtoneHue", 1.0, &mut g.midtones.hue),
        r.set("ColorGradeMidtoneSat", 0.01, &mut g.midtones.sat),
        r.set("ColorGradeMidtoneLum", 0.01, &mut g.midtones.lum),
        r.set("ColorGradeBlending", 0.01, &mut g.blending),
    ];
    if any.contains(&true) {
        touched.insert(Panel::Grading);
    }
    let global = [r.num("ColorGradeGlobalSat"), r.num("ColorGradeGlobalLum")];
    r.get("ColorGradeGlobalHue");
    if global.iter().any(|v| v.is_some_and(|v| v != 0.0)) {
        skipped.push("Global colour grading".into());
    }
}

fn detail(r: &Reader, d: &mut Develop, touched: &mut HashSet<Panel>) {
    let s = &mut d.detail.sharpen;
    let mut any = [
        r.set("Sharpness", 0.01, &mut s.amount),
        r.set("SharpenRadius", 1.0, &mut s.radius),
        r.set("SharpenDetail", 0.01, &mut s.detail),
        r.set("SharpenEdgeMasking", 0.01, &mut s.masking),
    ]
    .contains(&true);
    if let Some(v) = r.num("LuminanceSmoothing") {
        d.detail.denoise = (v > 0.0).then(|| Denoise { strength: v / 100.0, ..Denoise::default() });
        any = true;
    }
    if any {
        touched.insert(Panel::Detail);
    }
}

fn geometry(r: &Reader) -> Option<Geometry> {
    let has = r.bool("HasCrop");
    let [left, top, right, bottom, angle] = ["CropLeft", "CropTop", "CropRight", "CropBottom", "CropAngle"].map(|k| r.num(k));
    r.get("CropConstrainToWarp");
    if has != Some(true) {
        return None;
    }
    let x = left.unwrap_or(0.0).clamp(0.0, 1.0);
    let y = top.unwrap_or(0.0).clamp(0.0, 1.0);
    let w = (right.unwrap_or(1.0).clamp(0.0, 1.0) - x).max(0.0);
    let h = (bottom.unwrap_or(1.0).clamp(0.0, 1.0) - y).max(0.0);
    let whole = x == 0.0 && y == 0.0 && w >= 0.9999 && h >= 0.9999;
    let crop = (!whole && w > 0.0 && h > 0.0).then_some(Crop { x, y, w, h });
    let straighten = angle.unwrap_or(0.0).clamp(-45.0, 45.0);
    (crop.is_some() || straighten != 0.0).then_some(Geometry { crop, straighten })
}

/// The `Local*` sliders Lumen's masks have.
fn local_adjust(c: &Settings, skipped: &mut Vec<String>) -> LocalAdjust {
    let n = |k: &str| c.get(k).and_then(Value::num).map(|v| v as f32);
    let either = |a: &str, b: &str| n(a).or_else(|| n(b)).unwrap_or(0.0);
    // -1..1 in Lightroom, ±100 in Lumen; multiplied before rounding to f32.
    let hundred = |k: &str| c.get(k).and_then(Value::num).map_or(0.0, |v| (v * 100.0) as f32);
    let adjust = LocalAdjust {
        temperature: hundred("LocalTemperature"),
        tint: hundred("LocalTint"),
        exposure: either("LocalExposure2012", "LocalExposure"),
        contrast: either("LocalContrast2012", "LocalContrast"),
        highlights: n("LocalHighlights2012").unwrap_or(0.0),
        shadows: n("LocalShadows2012").unwrap_or(0.0),
        whites: n("LocalWhites2012").unwrap_or(0.0),
        blacks: n("LocalBlacks2012").unwrap_or(0.0),
        texture: n("LocalTexture").unwrap_or(0.0),
        clarity: either("LocalClarity2012", "LocalClarity"),
        dehaze: n("LocalDehaze").unwrap_or(0.0),
        saturation: n("LocalSaturation").unwrap_or(0.0),
        sharpness: n("LocalSharpness").unwrap_or(0.0),
        ..LocalAdjust::default()
    };
    const KNOWN: [&str; 17] = [
        "LocalTemperature", "LocalTint", "LocalExposure2012", "LocalExposure", "LocalContrast2012", "LocalContrast", "LocalHighlights2012", "LocalShadows2012", "LocalWhites2012",
        "LocalBlacks2012", "LocalTexture", "LocalClarity2012", "LocalClarity", "LocalDehaze", "LocalSaturation", "LocalSharpness", "LocalBrightness",
    ];
    for (k, v) in c {
        if k.starts_with("Local") && !KNOWN.contains(&k.as_str()) && does_something(k, v) {
            skipped.push(format!("Mask {k}"));
        }
    }
    adjust
}

/// Gradients and radial gradients become Lumen masks; brushes, AI and range masks are named
/// in `skipped`, as are spot removal and red eye.
fn masks(r: &Reader, skipped: &mut Vec<String>) -> Vec<Mask> {
    let mut out: Vec<Mask> = Vec::new();
    for (key, label) in [("RetouchAreas", "Spot removal"), ("RedEyeInfo", "Red eye")] {
        if r.get(key).is_some_and(|v| does_something(key, v)) {
            skipped.push(label.into());
        }
    }
    for key in ["GradientBasedCorrections", "CircularGradientBasedCorrections", "PaintBasedCorrections", "MaskGroupBasedCorrections"] {
        let Some(Value::List(corrections)) = r.get(key) else { continue };
        for c in corrections {
            let Value::Table(c) = c else { continue };
            let mut shapes = Vec::new();
            let mut kinds = Vec::new();
            for m in mask_list(c) {
                let n = |k: &str| m.get(k).and_then(Value::num).map(|v| v as f32);
                let inverted = m.get("MaskInverted").and_then(Value::bool).unwrap_or(false);
                let add = match n("MaskBlendMode") {
                    None | Some(0.0) => true,
                    Some(_) => {
                        skipped.push("Mask intersect or subtract (approximated)".into());
                        false
                    }
                };
                match m.get("What").and_then(Value::str).unwrap_or("") {
                    "Mask/Gradient" => {
                        let (full, zero) = ([n("FullX").unwrap_or(0.5), n("FullY").unwrap_or(0.0)], [n("ZeroX").unwrap_or(0.5), n("ZeroY").unwrap_or(1.0)]);
                        let (from, to) = if inverted { (zero, full) } else { (full, zero) };
                        shapes.push(MaskShape::Linear { add, from, to });
                        kinds.push("Gradient");
                    }
                    "Mask/CircularGradient" => {
                        let (t, l, b, rt) = (n("Top").unwrap_or(0.25), n("Left").unwrap_or(0.25), n("Bottom").unwrap_or(0.75), n("Right").unwrap_or(0.75));
                        let flipped = m.get("Flipped").and_then(Value::bool).unwrap_or(false);
                        shapes.push(MaskShape::Radial {
                            add,
                            center: [(l + rt) / 2.0, (t + b) / 2.0],
                            radius: [((rt - l) / 2.0).abs(), ((b - t) / 2.0).abs()],
                            angle: n("Angle").unwrap_or(0.0),
                            feather: n("Feather").unwrap_or(50.0) / 100.0,
                            inside: flipped != inverted,
                        });
                        kinds.push("Radial");
                    }
                    "Mask/Paint" | "Mask/Brush" => skipped.push("Brush mask".into()),
                    "Mask/Image" => skipped.push("AI mask (subject, sky…)".into()),
                    w if w.starts_with("Mask/Range") || w.contains("RangeMask") => skipped.push("Range mask".into()),
                    "" => {}
                    w => skipped.push(format!("Mask ({})", w.trim_start_matches("Mask/"))),
                }
            }
            if shapes.is_empty() {
                continue;
            }
            let adjust = local_adjust(c, skipped);
            let id = out.len() as u32 + 1;
            let nth = out.iter().filter(|m| m.name.starts_with(kinds[0])).count() + 1;
            let name = c.get("CorrectionName").and_then(Value::str).filter(|n| !n.is_empty()).map(str::to_string).unwrap_or_else(|| format!("{} {nth}", kinds[0]));
            out.push(Mask {
                id,
                name,
                shapes,
                invert: false,
                opacity: c.get("CorrectionAmount").and_then(Value::num).map_or(1.0, |v| v as f32),
                adjust,
                visible: c.get("CorrectionActive").and_then(Value::bool).unwrap_or(true),
            });
        }
    }
    out
}

/// A correction's masks: `CorrectionMasks`, and inside a mask group the masks it holds.
fn mask_list(c: &Settings) -> Vec<&Settings> {
    let mut out = Vec::new();
    if let Some(Value::List(ms)) = c.get("CorrectionMasks") {
        for m in ms {
            if let Value::Table(m) = m {
                match m.get("Masks") {
                    Some(Value::List(inner)) => out.extend(inner.iter().filter_map(|x| if let Value::Table(t) = x { Some(t) } else { None })),
                    _ => out.push(m),
                }
            }
        }
    }
    out
}

/// Whether a setting at this value changes the picture: not zero, not its default, not empty.
fn does_something(key: &str, v: &Value) -> bool {
    if key.starts_with("Upright") || key.starts_with("Supports") || (key.starts_with("LensProfile") && key != "LensProfileEnable") {
        return false;
    }
    if let Some((_, default)) = DEFAULTS.iter().find(|(k, _)| *k == key) {
        return v.num() != Some(*default);
    }
    match v {
        Value::Num(n) => *n != 0.0,
        Value::Bool(b) => *b,
        Value::Str(s) => {
            let s = s.trim();
            match key {
                "CameraProfile" => !matches!(s, "" | "Adobe Standard" | "Adobe Color" | "Embedded"),
                "WhiteBalance" => s != "As Shot",
                _ => !s.is_empty() && s.parse::<f64>().is_ok_and(|n| n != 0.0) || s.parse::<f64>().is_err() && !s.eq_ignore_ascii_case("false"),
            }
        }
        Value::List(l) => !l.is_empty(),
        Value::Table(t) => !t.is_empty(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::edits::{Crop, MaskShape};
    use crate::lightroom::Value;

    fn set(pairs: &[(&str, Value)]) -> Settings {
        pairs.iter().map(|(k, v)| (k.to_string(), v.clone())).collect()
    }
    fn n(v: f64) -> Value {
        Value::Num(v)
    }
    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-3
    }

    #[test]
    fn exposure_in_stops_sets_only_basic() {
        let c = convert(&set(&[("Exposure2012", n(1.0))]), None);
        assert_eq!(c.develop.basic.exposure, 1.0);
        assert_eq!(c.panels, vec![Panel::Basic]);
        assert!(c.skipped.is_empty(), "{:?}", c.skipped);
        assert!(c.geometry.is_none());
    }

    #[test]
    fn basic_sliders_are_hundredths() {
        let c = convert(
            &set(&[
                ("Contrast2012", n(50.0)),
                ("Highlights2012", n(-40.0)),
                ("Shadows2012", n(30.0)),
                ("Whites2012", n(10.0)),
                ("Blacks2012", n(-5.0)),
                ("Texture", n(20.0)),
                ("Clarity2012", n(15.0)),
                ("Dehaze", n(8.0)),
                ("Vibrance", n(25.0)),
                ("Saturation", n(-10.0)),
            ]),
            None,
        );
        let b = c.develop.basic;
        assert_eq!((b.contrast, b.highlights, b.shadows, b.whites, b.blacks), (0.5, -0.4, 0.3, 0.1, -0.05));
        assert_eq!((b.texture, b.clarity, b.dehaze, b.vibrance, b.saturation), (0.2, 0.15, 0.08, 0.25, -0.1));
    }

    #[test]
    fn a_preset_with_only_vibrance_touches_only_basic() {
        let c = convert(&set(&[("Vibrance", n(15.0)), ("ProcessVersion", Value::Str("11.0".into())), ("Name", Value::Str("Pop".into()))]), None);
        assert_eq!(c.panels, vec![Panel::Basic]);
        assert_eq!(c.develop.basic.vibrance, 0.15);
        assert_eq!(c.name.as_deref(), Some("Pop"));
        assert!(c.skipped.is_empty(), "{:?}", c.skipped);
    }

    #[test]
    fn white_balance_incremental_and_in_kelvin() {
        let c = convert(&set(&[("IncrementalTemperature", n(20.0)), ("IncrementalTint", n(-10.0))]), None);
        assert_eq!((c.develop.basic.temperature, c.develop.basic.tint), (20.0, -10.0));
        let k = convert(&set(&[("WhiteBalance", Value::Str("Custom".into())), ("Temperature", n(6600.0))]), Some(5500.0));
        assert!(close(k.develop.basic.temperature, 21.92), "{}", k.develop.basic.temperature);
        assert!(k.skipped.contains(&"Temperature (approximated)".to_string()), "{:?}", k.skipped);
        // Without the photo's own, 5500 K.
        let guess = convert(&set(&[("Temperature", n(6600.0))]), None);
        assert!(close(guess.develop.basic.temperature, 21.92));
        // As shot is no change at all.
        let shot = convert(&set(&[("WhiteBalance", Value::Str("As Shot".into())), ("Temperature", n(5250.0)), ("Tint", n(12.0))]), Some(5250.0));
        assert_eq!((shot.develop.basic.temperature, shot.develop.basic.tint), (0.0, 0.0));
        assert!(shot.skipped.is_empty(), "{:?}", shot.skipped);
    }

    #[test]
    fn tone_curves_are_points_out_of_255() {
        let c = convert(&set(&[("ToneCurvePV2012", Value::List([0.0, 0.0, 64.0, 50.0, 255.0, 255.0].map(Value::Num).to_vec()))]), None);
        let m = &c.develop.curve.master;
        assert_eq!(m.len(), 3);
        assert!(close(m[1][0], 0.251) && close(m[1][1], 0.196), "{m:?}");
        assert_eq!((m[0], m[2]), ([0.0, 0.0], [1.0, 1.0]));
        assert_eq!(c.panels, vec![Panel::Curve]);
        // XMP writes each point as "x, y".
        let xmp = convert(&set(&[("ToneCurvePV2012Red", Value::List(vec![Value::Str("0, 20".into()), Value::Str("255, 255".into())]))]), None);
        assert!(close(xmp.develop.curve.r[0][1], 20.0 / 255.0));
    }

    #[test]
    fn parametric_sliders_bend_the_master_curve() {
        let c = convert(
            &set(&[
                ("ParametricShadows", n(0.0)),
                ("ParametricDarks", n(0.0)),
                ("ParametricLights", n(40.0)),
                ("ParametricHighlights", n(0.0)),
                ("ParametricShadowSplit", n(25.0)),
                ("ParametricMidtoneSplit", n(50.0)),
                ("ParametricHighlightSplit", n(75.0)),
            ]),
            None,
        );
        let m = &c.develop.curve.master;
        assert_eq!(m.len(), 5, "{m:?}");
        let at = |x: f32| m.iter().find(|p| close(p[0], x)).unwrap()[1];
        assert!(at(0.5) > 0.5 && at(0.75) > 0.75, "lights lift the upper middle: {m:?}");
        assert!(close(at(0.25), 0.25), "{m:?}");
        assert_eq!(c.panels, vec![Panel::Curve]);
    }

    #[test]
    fn hsl_bands_are_hundredths() {
        let c = convert(&set(&[("HueAdjustmentBlue", n(-30.0)), ("SaturationAdjustmentRed", n(20.0)), ("LuminanceAdjustmentMagenta", n(-100.0))]), None);
        assert_eq!(c.develop.hsl[5].h, -0.3);
        assert_eq!(c.develop.hsl[0].s, 0.2);
        assert_eq!(c.develop.hsl[7].l, -1.0);
        assert_eq!(c.panels, vec![Panel::Hsl]);
    }

    #[test]
    fn split_toning_and_colour_grading_become_wheels() {
        let c = convert(&set(&[("SplitToningShadowHue", n(200.0)), ("SplitToningShadowSaturation", n(40.0)), ("SplitToningBalance", n(-20.0))]), None);
        let g = c.develop.grading;
        assert_eq!((g.shadows.hue, g.shadows.sat, g.balance), (200.0, 0.4, -0.2));
        assert_eq!(c.panels, vec![Panel::Grading]);
        let cg = convert(&set(&[("ColorGradeMidtoneHue", n(30.0)), ("ColorGradeMidtoneSat", n(10.0)), ("ColorGradeMidtoneLum", n(-5.0)), ("ColorGradeBlending", n(80.0)), ("ColorGradeHighlightLum", n(12.0))]), None);
        let g = cg.develop.grading;
        assert_eq!((g.midtones.hue, g.midtones.sat, g.midtones.lum, g.blending, g.highlights.lum), (30.0, 0.1, -0.05, 0.8, 0.12));
    }

    #[test]
    fn detail_and_noise() {
        let c = convert(&set(&[("Sharpness", n(40.0)), ("SharpenRadius", n(1.2)), ("SharpenDetail", n(30.0)), ("SharpenEdgeMasking", n(60.0)), ("LuminanceSmoothing", n(35.0))]), None);
        let d = &c.develop.detail;
        assert_eq!((d.sharpen.amount, d.sharpen.radius, d.sharpen.detail, d.sharpen.masking), (0.4, 1.2, 0.3, 0.6));
        assert_eq!(d.denoise.as_ref().map(|x| x.strength), Some(0.35));
        assert_eq!(c.panels, vec![Panel::Detail]);
        let off = convert(&set(&[("LuminanceSmoothing", n(0.0))]), None);
        assert!(off.develop.detail.denoise.is_none());
    }

    #[test]
    fn vignette_and_grain() {
        let c = convert(&set(&[("PostCropVignetteAmount", n(-25.0)), ("PostCropVignetteMidpoint", n(40.0)), ("PostCropVignetteFeather", n(70.0)), ("GrainAmount", n(20.0)), ("GrainSize", n(30.0))]), None);
        let e = c.develop.effects;
        assert_eq!((e.vignette.amount, e.vignette.midpoint, e.vignette.feather), (-0.25, 0.4, 0.7));
        assert_eq!((e.grain.amount, e.grain.size), (0.2, 0.3));
        assert_eq!(c.panels, vec![Panel::Effects]);
    }

    #[test]
    fn a_crop_and_its_angle_are_geometry() {
        let c = convert(
            &set(&[("HasCrop", Value::Bool(true)), ("CropLeft", n(0.1)), ("CropTop", n(0.05)), ("CropRight", n(0.9)), ("CropBottom", n(0.95)), ("CropAngle", n(2.5))]),
            None,
        );
        let g = c.geometry.unwrap();
        let k = g.crop.unwrap();
        assert!(close(k.x, 0.1) && close(k.y, 0.05) && close(k.w, 0.8) && close(k.h, 0.9), "{k:?}");
        assert_eq!(g.straighten, 2.5);
        assert!(c.panels.is_empty(), "geometry is no Develop panel");
        // Without HasCrop the crop values say nothing; a crop past the edge is clamped.
        assert!(convert(&set(&[("HasCrop", Value::Bool(false)), ("CropLeft", n(0.2))]), None).geometry.is_none());
        let wide = convert(&set(&[("HasCrop", Value::Bool(true)), ("CropLeft", n(-0.1)), ("CropTop", n(0.0)), ("CropRight", n(1.2)), ("CropBottom", n(0.5))]), None);
        assert_eq!(wide.geometry.unwrap().crop, Some(Crop { x: 0.0, y: 0.0, w: 1.0, h: 0.5 }));
    }

    fn correction(locals: &[(&str, Value)], mask: &[(&str, Value)]) -> Value {
        let mut c = set(locals);
        c.insert("What".into(), Value::Str("Correction".into()));
        c.insert("CorrectionMasks".into(), Value::List(vec![Value::Table(set(mask))]));
        Value::Table(c)
    }

    #[test]
    fn gradients_become_linear_and_radial_masks() {
        let linear = correction(
            &[("LocalExposure2012", n(-0.5)), ("LocalContrast2012", n(0.2)), ("LocalTemperature", n(0.3)), ("CorrectionAmount", n(1.0))],
            &[("What", Value::Str("Mask/Gradient".into())), ("ZeroX", n(0.5)), ("ZeroY", n(0.6)), ("FullX", n(0.5)), ("FullY", n(0.2))],
        );
        let radial = correction(
            &[("LocalSaturation", n(-0.4)), ("CorrectionAmount", n(0.5))],
            &[
                ("What", Value::Str("Mask/CircularGradient".into())),
                ("Top", n(0.2)),
                ("Left", n(0.3)),
                ("Bottom", n(0.6)),
                ("Right", n(0.7)),
                ("Angle", n(15.0)),
                ("Feather", n(50.0)),
                ("Flipped", Value::Bool(true)),
            ],
        );
        let c = convert(&set(&[("GradientBasedCorrections", Value::List(vec![linear])), ("CircularGradientBasedCorrections", Value::List(vec![radial]))]), None);
        assert_eq!(c.develop.masks.len(), 2, "{:?}", c.skipped);
        let l = &c.develop.masks[0];
        assert_eq!(l.shapes, vec![MaskShape::Linear { add: true, from: [0.5, 0.2], to: [0.5, 0.6] }]);
        assert_eq!((l.adjust.exposure, l.adjust.contrast, l.adjust.temperature), (-0.5, 0.2, 30.0));
        let r = &c.develop.masks[1];
        let MaskShape::Radial { center, radius, angle, feather, inside, .. } = r.shapes[0].clone() else { panic!() };
        assert!(close(center[0], 0.5) && close(center[1], 0.4) && close(radius[0], 0.2) && close(radius[1], 0.2), "{center:?} {radius:?}");
        assert_eq!((angle, feather, inside), (15.0, 0.5, true));
        assert_eq!((r.adjust.saturation, r.opacity), (-0.4, 0.5));
        assert_eq!(c.panels, vec![Panel::Masks]);
    }

    #[test]
    fn what_lumen_cannot_do_is_named_and_changes_nothing() {
        let brush = correction(&[("LocalExposure2012", n(0.3))], &[("What", Value::Str("Mask/Paint".into())), ("Radius", n(0.1))]);
        let c = convert(
            &set(&[
                ("CameraProfile", Value::Str("Adobe Portrait".into())),
                ("LensProfileEnable", n(1.0)),
                ("PerspectiveVertical", n(12.0)),
                ("PaintBasedCorrections", Value::List(vec![brush])),
            ]),
            None,
        );
        assert_eq!(c.develop, Develop::default().normalized());
        assert!(c.panels.is_empty(), "{:?}", c.panels);
        for name in ["CameraProfile", "LensProfileEnable", "PerspectiveVertical", "Brush mask"] {
            assert!(c.skipped.contains(&name.to_string()), "{name} in {:?}", c.skipped);
        }
        // Defaults that do nothing are not worth a line.
        let quiet = convert(&set(&[("LensProfileEnable", n(0.0)), ("CameraProfile", Value::Str("Adobe Standard".into())), ("PerspectiveVertical", n(0.0))]), None);
        assert!(quiet.skipped.is_empty(), "{:?}", quiet.skipped);
    }

    #[test]
    fn process_version_2010_is_approximated() {
        let c = convert(&set(&[("ProcessVersion", Value::Str("6.7".into())), ("Exposure", n(0.7)), ("Contrast", n(25.0)), ("FillLight", n(20.0)), ("Clarity", n(10.0))]), None);
        assert_eq!(c.develop.basic.exposure, 0.7);
        assert_eq!(c.develop.basic.shadows, 0.2);
        assert_eq!(c.develop.basic.clarity, 0.1);
        assert!(c.skipped.contains(&"Process version 2010 (approximated)".to_string()), "{:?}", c.skipped);
    }

    #[test]
    fn every_value_is_in_range() {
        let c = convert(&set(&[("Exposure2012", n(9.0)), ("Contrast2012", n(400.0)), ("IncrementalTemperature", n(-300.0))]), None);
        assert_eq!((c.develop.basic.exposure, c.develop.basic.contrast, c.develop.basic.temperature), (5.0, 1.0, -100.0));
    }
}
