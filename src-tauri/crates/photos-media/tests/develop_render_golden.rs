//! Writes `develop-render-golden.json` (Round 11B): small linear images, recipes over every
//! global develop stage, and what `develop::process` makes of them, so the training scripts'
//! PyTorch copy of the pipeline (`scripts/ml/lumen_train/retouch/render.py`) can be held to
//! Rust. Regenerated on every run; never edited by hand.

use photos_core::edits::{Develop, Recipe};
use photos_media::develop::{self, Ctx, Linear};
use serde_json::{json, Value};

const W: u32 = 16;
const H: u32 = 12;

/// The global stages: per pixel, or (vignette, masks) by position only.
const GLOBAL: [&str; 6] = ["gains", "tones", "masks", "curve", "color", "vignette"];
/// The stages that need neighbours and are not in this file.
const SPATIAL: [&str; 5] = ["texture", "clarity", "dehaze", "sharpen", "grain"];

fn image(kind: usize) -> Linear {
    let mut img = Linear::new(W, H, [0.0; 3]);
    for y in 0..H {
        for x in 0..W {
            let (u, v) = (x as f32 / (W - 1) as f32, y as f32 / (H - 1) as f32);
            img.data[(y * W + x) as usize] = match kind {
                // A grey ramp from deep shadow to past clipping, brighter to the right.
                0 => [0.004 * 2f32.powf(9.0 * u + v); 3],
                // Sky over ground: blue above, green-brown below.
                1 if v < 0.5 => [0.25 + 0.2 * u, 0.4 + 0.2 * u, 0.9],
                1 => [0.12 + 0.05 * u, 0.15, 0.05],
                // A skin-tone patch in a mid-grey surround.
                2 if (0.3..0.7).contains(&u) && (0.25..0.75).contains(&v) => [0.45, 0.28, 0.2],
                2 => [0.18; 3],
                // Saturated primaries and secondaries, and their dark versions below.
                _ => {
                    let hue = (u * 6.0).floor() as usize % 6;
                    let c = [[1.0, 0.02, 0.02], [1.0, 0.6, 0.02], [0.9, 0.9, 0.02], [0.02, 0.8, 0.05], [0.02, 0.6, 0.9], [0.6, 0.05, 0.9]][hue];
                    let k = if v < 0.5 { 0.8 } else { 0.08 };
                    [c[0] * k, c[1] * k, c[2] * k]
                }
            };
        }
    }
    img
}

/// Every global slider alone at both ends, then a few together.
fn recipes() -> Vec<(String, Value)> {
    let mut out: Vec<(String, Value)> = Vec::new();
    for (name, lo, hi) in [
        ("temperature", -60.0, 60.0),
        ("tint", -60.0, 60.0),
        ("exposure", -1.5, 1.5),
        ("contrast", -0.7, 0.7),
        ("highlights", -0.8, 0.8),
        ("shadows", -0.8, 0.8),
        ("whites", -0.8, 0.8),
        ("blacks", -0.8, 0.8),
        ("vibrance", -0.7, 0.7),
        ("saturation", -0.7, 0.7),
    ] {
        out.push((format!("{name}-"), json!({ "basic": { name: lo } })));
        out.push((format!("{name}+"), json!({ "basic": { name: hi } })));
    }
    out.push(("curve-master".into(), json!({ "curve": { "master": [[0.0, 0.05], [0.25, 0.2], [0.5, 0.55], [0.75, 0.82], [1.0, 0.97]] } })));
    out.push(("curve-rgb".into(), json!({ "curve": { "r": [[0.0, 0.0], [0.5, 0.56], [1.0, 1.0]], "g": [[0.0, 0.0], [0.5, 0.47], [1.0, 1.0]], "b": [[0.0, 0.0], [0.5, 0.44], [1.0, 1.0]] } })));
    for (i, band) in ["red", "orange", "yellow", "green", "aqua", "blue", "purple", "magenta"].iter().enumerate() {
        let mut hsl = vec![json!({}); 8];
        hsl[i] = json!({ "h": 0.6, "s": -0.5, "l": 0.4 });
        out.push((format!("hsl-{band}"), json!({ "hsl": hsl })));
    }
    out.push(("grade-shadows".into(), json!({ "grading": { "shadows": { "hue": 220.0, "sat": 0.5, "lum": -0.2 } } })));
    out.push(("grade-midtones".into(), json!({ "grading": { "midtones": { "hue": 40.0, "sat": 0.3, "lum": 0.1 } } })));
    out.push(("grade-highlights".into(), json!({ "grading": { "highlights": { "hue": 60.0, "sat": 0.6, "lum": 0.3 }, "blending": 0.8, "balance": -0.4 } })));
    out.push(("vignette-".into(), json!({ "effects": { "vignette": { "amount": -0.6, "midpoint": 0.4, "feather": 0.7 } } })));
    out.push(("vignette+".into(), json!({ "effects": { "vignette": { "amount": 0.5, "midpoint": 0.6, "feather": 0.3 } } })));
    out.push((
        "mask-linear".into(),
        json!({ "masks": [{ "id": 1, "name": "g", "shapes": [{ "kind": "linear", "add": true, "from": [0.5, 0.0], "to": [0.5, 0.6] }], "adjust": { "exposure": -0.8, "contrast": 0.3, "highlights": -0.4, "shadows": 0.2, "temperature": -20.0, "saturation": 0.3 } }] }),
    ));
    out.push((
        "mask-radial".into(),
        json!({ "masks": [{ "id": 2, "name": "r", "shapes": [{ "kind": "radial", "add": true, "center": [0.5, 0.5], "radius": [0.3, 0.4], "angle": 0.0, "feather": 0.5, "inside": true }], "adjust": { "exposure": 0.6, "contrast": -0.2, "shadows": 0.4, "saturation": -0.3, "temperature": 25.0 } }] }),
    ));
    out.push((
        "together".into(),
        json!({
            "basic": { "temperature": 15.0, "tint": -5.0, "exposure": 0.4, "contrast": 0.25, "highlights": -0.5, "shadows": 0.4, "whites": 0.2, "blacks": -0.2, "vibrance": 0.3, "saturation": -0.1 },
            "curve": { "master": [[0.0, 0.03], [0.25, 0.22], [0.5, 0.52], [0.75, 0.8], [1.0, 1.0]] },
            "hsl": [{ "s": 0.2 }, { "l": 0.2, "h": -0.2 }, {}, {}, {}, { "s": -0.3, "l": -0.2 }, {}, {}],
            "grading": { "shadows": { "hue": 200.0, "sat": 0.2 }, "highlights": { "hue": 45.0, "sat": 0.25 } },
            "effects": { "vignette": { "amount": -0.3 } },
            "masks": [{ "id": 1, "name": "g", "shapes": [{ "kind": "linear", "add": true, "from": [0.5, 0.0], "to": [0.5, 0.5] }], "adjust": { "exposure": -0.5 } }]
        }),
    ));
    out
}

fn round(v: f32) -> f32 {
    (v * 1e5).round() / 1e5
}

fn pixels(img: &Linear) -> Vec<f32> {
    img.data.iter().flat_map(|p| p.iter().map(|&c| round(c))).collect()
}

#[test]
fn writes_the_render_golden_file() {
    let mut cases = Vec::new();
    for kind in 0..4 {
        let base = image(kind);
        for (name, d) in recipes() {
            let develop: Develop = serde_json::from_value(d.clone()).expect(&name);
            let recipe = Recipe { develop: Some(develop.clone()), ..Recipe::default() };
            let out = develop::process(&base, &recipe, &Ctx::new(1.0, 7));
            assert_eq!((out.width, out.height), (W, H), "{name}: no geometry");
            assert!(out.data.iter().all(|p| p.iter().all(|c| c.is_finite())), "{name}");
            cases.push(json!({ "image": kind, "name": name, "develop": develop.normalized(), "output": pixels(&out) }));
        }
    }
    let images: Vec<Value> = (0..4).map(|k| json!(pixels(&image(k)))).collect();
    let doc = json!({ "width": W, "height": H, "global": GLOBAL, "spatial": SPATIAL, "images": images, "cases": cases });
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/develop-render-golden.json");
    std::fs::write(path, serde_json::to_string(&doc).unwrap() + "\n").unwrap();
}

/// A recipe that changes nothing gives the image back, so the file's inputs are what
/// `process` sees.
#[test]
fn a_neutral_recipe_is_the_identity() {
    let base = image(1);
    let out = develop::process(&base, &Recipe { develop: Some(Develop::default()), ..Recipe::default() }, &Ctx::new(1.0, 7));
    for (a, b) in out.data.iter().zip(&base.data) {
        for c in 0..3 {
            assert!((a[c] - b[c]).abs() < 1e-5);
        }
    }
}
