//! The Retouch vector: every slider the model sets, each in `[0, 1]` between its `min` and
//! `max`. The order is fixed: entries are only ever appended, with `RETOUCH_LAYOUT` bumped.

use serde::{Deserialize, Serialize};

use crate::edits::{Crop, Develop, Grading, Grain, Hsl, LocalAdjust, Mask, MaskShape, Sharpen, Vignette, Wheel, HSL_BANDS};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Group {
    Geometry,
    Basic,
    Curve,
    Hsl,
    Grading,
    Detail,
    Effects,
    Linear,
    Radial,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Param {
    pub name: &'static str,
    pub min: f32,
    pub max: f32,
    pub neutral: f32,
    pub group: Group,
}

pub const RETOUCH_LAYOUT: u32 = 1;

const fn p(name: &'static str, min: f32, max: f32, neutral: f32, group: Group) -> Param {
    Param { name, min, max, neutral, group }
}

use Group::*;

pub static PARAMS: &[Param] = &[
    // Geometry: straighten in degrees; the crop as a share of the photo, `crop_ratio` picking
    // one of `RATIOS` (a free crop uses `crop_h`).
    p("straighten", -10.0, 10.0, 0.0, Geometry),
    p("crop_on", 0.0, 1.0, 0.0, Geometry),
    p("crop_x", 0.0, 0.3, 0.0, Geometry),
    p("crop_y", 0.0, 0.3, 0.0, Geometry),
    p("crop_w", 0.4, 1.0, 1.0, Geometry),
    p("crop_h", 0.4, 1.0, 1.0, Geometry),
    p("crop_ratio", 0.0, 1.0, RATIO_FREE, Geometry),
    // Basic, in Develop's own units (exposure narrowed to ±3 EV).
    p("temperature", -100.0, 100.0, 0.0, Basic),
    p("tint", -100.0, 100.0, 0.0, Basic),
    p("exposure", -3.0, 3.0, 0.0, Basic),
    p("contrast", -1.0, 1.0, 0.0, Basic),
    p("highlights", -1.0, 1.0, 0.0, Basic),
    p("shadows", -1.0, 1.0, 0.0, Basic),
    p("whites", -1.0, 1.0, 0.0, Basic),
    p("blacks", -1.0, 1.0, 0.0, Basic),
    p("texture", -1.0, 1.0, 0.0, Basic),
    p("clarity", -1.0, 1.0, 0.0, Basic),
    p("dehaze", -1.0, 1.0, 0.0, Basic),
    p("vibrance", -1.0, 1.0, 0.0, Basic),
    p("saturation", -1.0, 1.0, 0.0, Basic),
    // The master curve at x = 0, .25, .5, .75, 1 (the ends as levels, the middle as offsets
    // from the diagonal), and each channel's middle.
    p("curve_m0", 0.0, 0.25, 0.0, Curve),
    p("curve_m1", -0.25, 0.25, 0.0, Curve),
    p("curve_m2", -0.25, 0.25, 0.0, Curve),
    p("curve_m3", -0.25, 0.25, 0.0, Curve),
    p("curve_m4", 0.75, 1.0, 1.0, Curve),
    p("curve_r", -0.1, 0.1, 0.0, Curve),
    p("curve_g", -0.1, 0.1, 0.0, Curve),
    p("curve_b", -0.1, 0.1, 0.0, Curve),
    // HSL, band by band (`HSL_BANDS` order).
    p("hsl_red_h", -1.0, 1.0, 0.0, Hsl),
    p("hsl_red_s", -1.0, 1.0, 0.0, Hsl),
    p("hsl_red_l", -1.0, 1.0, 0.0, Hsl),
    p("hsl_orange_h", -1.0, 1.0, 0.0, Hsl),
    p("hsl_orange_s", -1.0, 1.0, 0.0, Hsl),
    p("hsl_orange_l", -1.0, 1.0, 0.0, Hsl),
    p("hsl_yellow_h", -1.0, 1.0, 0.0, Hsl),
    p("hsl_yellow_s", -1.0, 1.0, 0.0, Hsl),
    p("hsl_yellow_l", -1.0, 1.0, 0.0, Hsl),
    p("hsl_green_h", -1.0, 1.0, 0.0, Hsl),
    p("hsl_green_s", -1.0, 1.0, 0.0, Hsl),
    p("hsl_green_l", -1.0, 1.0, 0.0, Hsl),
    p("hsl_aqua_h", -1.0, 1.0, 0.0, Hsl),
    p("hsl_aqua_s", -1.0, 1.0, 0.0, Hsl),
    p("hsl_aqua_l", -1.0, 1.0, 0.0, Hsl),
    p("hsl_blue_h", -1.0, 1.0, 0.0, Hsl),
    p("hsl_blue_s", -1.0, 1.0, 0.0, Hsl),
    p("hsl_blue_l", -1.0, 1.0, 0.0, Hsl),
    p("hsl_purple_h", -1.0, 1.0, 0.0, Hsl),
    p("hsl_purple_s", -1.0, 1.0, 0.0, Hsl),
    p("hsl_purple_l", -1.0, 1.0, 0.0, Hsl),
    p("hsl_magenta_h", -1.0, 1.0, 0.0, Hsl),
    p("hsl_magenta_s", -1.0, 1.0, 0.0, Hsl),
    p("hsl_magenta_l", -1.0, 1.0, 0.0, Hsl),
    // Grading wheels as chroma (a, b), so hue has no seam at 0/360°.
    p("grade_shadows_a", -1.0, 1.0, 0.0, Grading),
    p("grade_shadows_b", -1.0, 1.0, 0.0, Grading),
    p("grade_shadows_lum", -1.0, 1.0, 0.0, Grading),
    p("grade_midtones_a", -1.0, 1.0, 0.0, Grading),
    p("grade_midtones_b", -1.0, 1.0, 0.0, Grading),
    p("grade_midtones_lum", -1.0, 1.0, 0.0, Grading),
    p("grade_highlights_a", -1.0, 1.0, 0.0, Grading),
    p("grade_highlights_b", -1.0, 1.0, 0.0, Grading),
    p("grade_highlights_lum", -1.0, 1.0, 0.0, Grading),
    p("grade_blending", 0.0, 1.0, 0.5, Grading),
    p("grade_balance", -1.0, 1.0, 0.0, Grading),
    // Detail: sharpening only (denoise is a choice of its own).
    p("sharpen_amount", 0.0, 1.5, 0.0, Detail),
    p("sharpen_radius", 0.5, 3.0, 1.0, Detail),
    p("sharpen_detail", 0.0, 1.0, 0.25, Detail),
    p("sharpen_masking", 0.0, 1.0, 0.0, Detail),
    // Effects.
    p("vignette_amount", -1.0, 1.0, 0.0, Effects),
    p("vignette_midpoint", 0.0, 1.0, 0.5, Effects),
    p("vignette_feather", 0.0, 1.0, 0.5, Effects),
    p("grain_amount", 0.0, 1.0, 0.0, Effects),
    p("grain_size", 0.0, 1.0, 0.25, Effects),
    // One linear gradient (full effect at `from`), in the unedited photo's 0–1 coordinates.
    p("lin_on", 0.0, 1.0, 0.0, Linear),
    p("lin_fx", 0.0, 1.0, 0.5, Linear),
    p("lin_fy", 0.0, 1.0, 0.0, Linear),
    p("lin_tx", 0.0, 1.0, 0.5, Linear),
    p("lin_ty", 0.0, 1.0, 0.5, Linear),
    p("lin_exposure", -3.0, 3.0, 0.0, Linear),
    p("lin_contrast", -1.0, 1.0, 0.0, Linear),
    p("lin_highlights", -1.0, 1.0, 0.0, Linear),
    p("lin_shadows", -1.0, 1.0, 0.0, Linear),
    p("lin_temperature", -100.0, 100.0, 0.0, Linear),
    p("lin_saturation", -1.0, 1.0, 0.0, Linear),
    // One radial gradient, acting inside.
    p("rad_on", 0.0, 1.0, 0.0, Radial),
    p("rad_cx", 0.0, 1.0, 0.5, Radial),
    p("rad_cy", 0.0, 1.0, 0.5, Radial),
    p("rad_rx", 0.05, 1.0, 0.35, Radial),
    p("rad_ry", 0.05, 1.0, 0.35, Radial),
    p("rad_feather", 0.0, 1.0, 0.5, Radial),
    p("rad_exposure", -3.0, 3.0, 0.0, Radial),
    p("rad_contrast", -1.0, 1.0, 0.0, Radial),
    p("rad_shadows", -1.0, 1.0, 0.0, Radial),
    p("rad_saturation", -1.0, 1.0, 0.0, Radial),
    p("rad_temperature", -100.0, 100.0, 0.0, Radial),
];

/// The crop shapes `crop_ratio` picks between, as long side / short side in pixels, the long
/// side along the photo's (4:5 is 1.25, portrait on a portrait photo): 0 is free, -1 the photo's own shape.
pub const RATIOS: [f32; 6] = [0.0, -1.0, 1.0, 1.25, 1.5, 16.0 / 9.0];
/// `crop_ratio`'s value at the middle of each shape's bin.
pub const RATIO_FREE: f32 = 0.5 / 6.0;
pub const RATIO_ORIGINAL: f32 = 1.5 / 6.0;
pub const RATIO_1_1: f32 = 2.5 / 6.0;
pub const RATIO_4_5: f32 = 3.5 / 6.0;

/// What a Retouch vector means.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct RetouchRecipe {
    pub develop: Develop,
    pub crop: Option<Crop>,
    pub straighten: f32,
    pub summary: Vec<String>,
}

const LINEAR_NAME: &str = "Retouch gradient";
const RADIAL_NAME: &str = "Retouch radial";
const CURVE_X: [f32; 5] = [0.0, 0.25, 0.5, 0.75, 1.0];

fn index(name: &str) -> usize {
    PARAMS.iter().position(|p| p.name == name).unwrap_or_else(|| panic!("no Retouch param {name}"))
}

/// Reads values by name, each clamped to its range (a NaN reads as neutral).
struct Reader<'a>(&'a [f32]);

impl Reader<'_> {
    /// The raw `[0, 1]` value.
    fn unit(&self, name: &str) -> f32 {
        let i = index(name);
        let p = PARAMS[i];
        self.0.get(i).copied().filter(|u| u.is_finite()).map_or((p.neutral - p.min) / (p.max - p.min), |u| u.clamp(0.0, 1.0))
    }

    fn get(&self, name: &str) -> f32 {
        let p = PARAMS[index(name)];
        p.min + self.unit(name) * (p.max - p.min)
    }

    fn on(&self, name: &str) -> bool {
        self.unit(name) >= 0.5
    }
}

/// Writes values by name; everything not written stays neutral.
struct Writer(Vec<f32>);

impl Writer {
    fn new() -> Self {
        Self(PARAMS.iter().map(|p| (p.neutral - p.min) / (p.max - p.min)).collect())
    }

    fn set(&mut self, name: &str, value: f32) {
        let i = index(name);
        let p = PARAMS[i];
        if value.is_finite() {
            self.0[i] = ((value - p.min) / (p.max - p.min)).clamp(0.0, 1.0);
        }
    }
}

/// The pixel ratio (width / height) a ratio bin asks for on a photo of `aspect`, or `None`
/// for a free crop.
fn ratio_of(bin: usize, aspect: f32) -> Option<f32> {
    let r = RATIOS[bin];
    if r == 0.0 {
        None
    } else if r < 0.0 {
        Some(aspect)
    } else if aspect < 1.0 {
        Some(1.0 / r)
    } else {
        Some(r)
    }
}

fn sane_aspect(aspect: f32) -> f32 {
    if aspect.is_finite() && aspect > 0.0 {
        aspect
    } else {
        1.0
    }
}

fn decode_crop(r: &Reader, aspect: f32) -> Option<Crop> {
    if !r.on("crop_on") {
        return None;
    }
    let aspect = sane_aspect(aspect);
    let (x, y) = (r.get("crop_x"), r.get("crop_y"));
    let mut w = r.get("crop_w").min(1.0 - x);
    let bin = ((r.unit("crop_ratio") * RATIOS.len() as f32) as usize).min(RATIOS.len() - 1);
    let h = match ratio_of(bin, aspect) {
        None => r.get("crop_h").min(1.0 - y),
        Some(ratio) => {
            // In pixels (w · W) / (h · H) = ratio, so h = w · aspect / ratio.
            let mut h = w * aspect / ratio;
            if h > 1.0 - y {
                h = 1.0 - y;
                w = h * ratio / aspect;
            }
            h
        }
    };
    let whole = x < 1e-4 && y < 1e-4 && w > 0.9999 && h > 0.9999;
    (!whole).then_some(Crop { x, y, w, h })
}

fn encode_crop(c: &Crop, aspect: f32, out: &mut Writer) {
    let aspect = sane_aspect(aspect);
    out.set("crop_on", 1.0);
    out.set("crop_x", c.x);
    out.set("crop_y", c.y);
    out.set("crop_w", c.w);
    out.set("crop_h", c.h);
    let pixels = c.w * aspect / c.h.max(1e-6);
    let bin = (1..RATIOS.len()).find(|&b| ratio_of(b, aspect).is_some_and(|r| (r - pixels).abs() < 1e-3 * r));
    out.set("crop_ratio", (bin.unwrap_or(0) as f32 + 0.5) / RATIOS.len() as f32);
}

/// `points` read piecewise-linearly at `x` (the diagonal when empty).
fn curve_at(points: &[[f32; 2]], x: f32) -> f32 {
    let (Some(first), Some(last)) = (points.first(), points.last()) else { return x };
    if x <= first[0] {
        return first[1];
    }
    for pair in points.windows(2) {
        let (a, b) = (pair[0], pair[1]);
        if x <= b[0] {
            let t = if b[0] > a[0] { (x - a[0]) / (b[0] - a[0]) } else { 1.0 };
            return a[1] + t * (b[1] - a[1]);
        }
    }
    last[1]
}

fn wheel(a: f32, b: f32, lum: f32) -> Wheel {
    let sat = a.hypot(b).min(1.0);
    if sat < 1e-4 {
        return Wheel { hue: 0.0, sat: 0.0, lum };
    }
    Wheel { hue: b.atan2(a).to_degrees().rem_euclid(360.0), sat, lum }
}

fn push_mask(d: &mut Develop, id: u32, name: &str, shape: MaskShape, adjust: LocalAdjust) {
    if !adjust.is_zero() {
        d.masks.push(Mask { id, name: name.into(), shapes: vec![shape], invert: false, opacity: 1.0, adjust, visible: true });
    }
}

/// The model's develop applied over the draft's: every panel the model sets replaces the
/// draft's, but the draft's denoise and the person's own masks stay (the model sets neither).
/// Masks from an earlier Retouch give way to this one's, renumbered after the kept ones.
pub fn onto_draft(model: Develop, draft: &Develop) -> Develop {
    let mut masks: Vec<Mask> = draft.masks.iter().filter(|m| m.name != LINEAR_NAME && m.name != RADIAL_NAME).cloned().collect();
    let next = masks.iter().map(|m| m.id).max().unwrap_or(0);
    masks.extend(model.masks.into_iter().enumerate().map(|(i, m)| Mask { id: next + 1 + i as u32, ..m }));
    let mut d = Develop { masks, ..model };
    d.detail.denoise = draft.detail.denoise.clone();
    d
}

/// `v` (one value in `[0, 1]` per `PARAMS` entry) as a recipe for a photo of `aspect`
/// (width / height). Values outside `[0, 1]` are clamped; a NaN reads as neutral.
pub fn decode(v: &[f32], aspect: f32) -> RetouchRecipe {
    let r = Reader(v);
    let mut d = Develop::default();
    let b = &mut d.basic;
    b.temperature = r.get("temperature");
    b.tint = r.get("tint");
    b.exposure = r.get("exposure");
    b.contrast = r.get("contrast");
    b.highlights = r.get("highlights");
    b.shadows = r.get("shadows");
    b.whites = r.get("whites");
    b.blacks = r.get("blacks");
    b.texture = r.get("texture");
    b.clarity = r.get("clarity");
    b.dehaze = r.get("dehaze");
    b.vibrance = r.get("vibrance");
    b.saturation = r.get("saturation");

    let ys = [r.get("curve_m0"), 0.25 + r.get("curve_m1"), 0.5 + r.get("curve_m2"), 0.75 + r.get("curve_m3"), r.get("curve_m4")];
    if CURVE_X.iter().zip(ys).any(|(x, y)| (x - y).abs() > 1e-4) {
        d.curve.master = CURVE_X.iter().zip(ys).map(|(&x, y)| [x, y.clamp(0.0, 1.0)]).collect();
    }
    let channel = |o: f32| if o.abs() > 1e-4 { vec![[0.0, 0.0], [0.5, 0.5 + o], [1.0, 1.0]] } else { vec![] };
    d.curve.r = channel(r.get("curve_r"));
    d.curve.g = channel(r.get("curve_g"));
    d.curve.b = channel(r.get("curve_b"));

    for (i, band) in HSL_BANDS.iter().enumerate() {
        d.hsl[i] = Hsl { h: r.get(&format!("hsl_{band}_h")), s: r.get(&format!("hsl_{band}_s")), l: r.get(&format!("hsl_{band}_l")) };
    }
    let w = |name: &str| wheel(r.get(&format!("grade_{name}_a")), r.get(&format!("grade_{name}_b")), r.get(&format!("grade_{name}_lum")));
    d.grading = Grading { shadows: w("shadows"), midtones: w("midtones"), highlights: w("highlights"), blending: r.get("grade_blending"), balance: r.get("grade_balance") };
    d.detail.sharpen = Sharpen { amount: r.get("sharpen_amount"), radius: r.get("sharpen_radius"), detail: r.get("sharpen_detail"), masking: r.get("sharpen_masking") };
    d.effects.vignette = Vignette { amount: r.get("vignette_amount"), midpoint: r.get("vignette_midpoint"), feather: r.get("vignette_feather") };
    d.effects.grain = Grain { amount: r.get("grain_amount"), size: r.get("grain_size") };

    if r.on("lin_on") {
        let adjust = LocalAdjust {
            exposure: r.get("lin_exposure"),
            contrast: r.get("lin_contrast"),
            highlights: r.get("lin_highlights"),
            shadows: r.get("lin_shadows"),
            temperature: r.get("lin_temperature"),
            saturation: r.get("lin_saturation"),
            ..LocalAdjust::default()
        };
        let shape = MaskShape::Linear { add: true, from: [r.get("lin_fx"), r.get("lin_fy")], to: [r.get("lin_tx"), r.get("lin_ty")] };
        push_mask(&mut d, 1, LINEAR_NAME, shape, adjust);
    }
    if r.on("rad_on") {
        let adjust = LocalAdjust {
            exposure: r.get("rad_exposure"),
            contrast: r.get("rad_contrast"),
            shadows: r.get("rad_shadows"),
            saturation: r.get("rad_saturation"),
            temperature: r.get("rad_temperature"),
            ..LocalAdjust::default()
        };
        let shape = MaskShape::Radial {
            add: true,
            center: [r.get("rad_cx"), r.get("rad_cy")],
            radius: [r.get("rad_rx"), r.get("rad_ry")],
            angle: 0.0,
            feather: r.get("rad_feather"),
            inside: true,
        };
        push_mask(&mut d, 2, RADIAL_NAME, shape, adjust);
    }

    let develop = d.normalized();
    let crop = decode_crop(&r, aspect);
    let straighten = r.get("straighten");
    let summary = summarise(&develop, crop.is_some(), straighten);
    RetouchRecipe { develop, crop, straighten, summary }
}

/// What a recipe did, in a few words for the toast, in a fixed order.
fn summarise(d: &Develop, cropped: bool, straighten: f32) -> Vec<String> {
    let mut s = Vec::new();
    if straighten.abs() >= 0.3 {
        s.push(format!("straightened {:.1}°", straighten.abs()));
    }
    if cropped {
        s.push("cropped".into());
    }
    match d.basic.exposure {
        e if e >= 0.15 => s.push("brighter".into()),
        e if e <= -0.15 => s.push("darker".into()),
        _ => {}
    }
    match d.basic.temperature {
        t if t >= 8.0 => s.push("warmer".into()),
        t if t <= -8.0 => s.push("cooler".into()),
        _ => {}
    }
    for m in &d.masks {
        match m.shapes.first() {
            Some(MaskShape::Linear { from, to, .. }) if m.adjust.exposure <= -0.15 && from[1] < to[1] && from[1] < 0.5 => s.push("sky darkened".into()),
            Some(MaskShape::Linear { .. }) => s.push("graduated".into()),
            Some(MaskShape::Radial { .. }) if m.adjust.exposure >= 0.1 => s.push("subject lifted".into()),
            Some(MaskShape::Radial { .. }) => s.push("spot adjusted".into()),
            _ => {}
        }
    }
    s
}

/// The vector `decode` would read as `r`. What the layout cannot hold (brush masks, a second
/// gradient, denoise, curve detail between its five points, a radial's turn) is left out.
pub fn encode(r: &RetouchRecipe, aspect: f32) -> Vec<f32> {
    let mut out = Writer::new();
    out.set("straighten", r.straighten);
    if let Some(c) = &r.crop {
        encode_crop(c, aspect, &mut out);
    }
    let d = &r.develop;
    let b = &d.basic;
    for (name, v) in [
        ("temperature", b.temperature),
        ("tint", b.tint),
        ("exposure", b.exposure),
        ("contrast", b.contrast),
        ("highlights", b.highlights),
        ("shadows", b.shadows),
        ("whites", b.whites),
        ("blacks", b.blacks),
        ("texture", b.texture),
        ("clarity", b.clarity),
        ("dehaze", b.dehaze),
        ("vibrance", b.vibrance),
        ("saturation", b.saturation),
    ] {
        out.set(name, v);
    }
    let m = &d.curve.master;
    out.set("curve_m0", curve_at(m, 0.0));
    out.set("curve_m1", curve_at(m, 0.25) - 0.25);
    out.set("curve_m2", curve_at(m, 0.5) - 0.5);
    out.set("curve_m3", curve_at(m, 0.75) - 0.75);
    out.set("curve_m4", curve_at(m, 1.0));
    out.set("curve_r", curve_at(&d.curve.r, 0.5) - 0.5);
    out.set("curve_g", curve_at(&d.curve.g, 0.5) - 0.5);
    out.set("curve_b", curve_at(&d.curve.b, 0.5) - 0.5);
    for (i, band) in HSL_BANDS.iter().enumerate() {
        out.set(&format!("hsl_{band}_h"), d.hsl[i].h);
        out.set(&format!("hsl_{band}_s"), d.hsl[i].s);
        out.set(&format!("hsl_{band}_l"), d.hsl[i].l);
    }
    let g = &d.grading;
    for (name, w) in [("shadows", g.shadows), ("midtones", g.midtones), ("highlights", g.highlights)] {
        let (sin, cos) = w.hue.to_radians().sin_cos();
        out.set(&format!("grade_{name}_a"), w.sat * cos);
        out.set(&format!("grade_{name}_b"), w.sat * sin);
        out.set(&format!("grade_{name}_lum"), w.lum);
    }
    out.set("grade_blending", g.blending);
    out.set("grade_balance", g.balance);
    let s = &d.detail.sharpen;
    out.set("sharpen_amount", s.amount);
    out.set("sharpen_radius", s.radius);
    out.set("sharpen_detail", s.detail);
    out.set("sharpen_masking", s.masking);
    let e = &d.effects;
    out.set("vignette_amount", e.vignette.amount);
    out.set("vignette_midpoint", e.vignette.midpoint);
    out.set("vignette_feather", e.vignette.feather);
    out.set("grain_amount", e.grain.amount);
    out.set("grain_size", e.grain.size);

    let single = |m: &&Mask| m.visible && !m.invert && m.shapes.len() == 1 && !m.adjust.is_zero();
    for m in d.masks.iter().filter(single) {
        let a = &m.adjust;
        let k = m.opacity;
        match m.shapes[0] {
            MaskShape::Linear { add: true, from, to } if out.0[index("lin_on")] < 0.5 => {
                out.set("lin_on", 1.0);
                out.set("lin_fx", from[0]);
                out.set("lin_fy", from[1]);
                out.set("lin_tx", to[0]);
                out.set("lin_ty", to[1]);
                out.set("lin_exposure", a.exposure * k);
                out.set("lin_contrast", a.contrast * k);
                out.set("lin_highlights", a.highlights * k);
                out.set("lin_shadows", a.shadows * k);
                out.set("lin_temperature", a.temperature * k);
                out.set("lin_saturation", a.saturation * k);
            }
            MaskShape::Radial { add: true, inside: true, center, radius, angle, feather } if out.0[index("rad_on")] < 0.5 => {
                // The layout's ellipse is upright: a turned one is read at the nearer quarter
                // turn, its semi-axes kept in pixels (`radius` is a share of each side).
                let ratio = sane_aspect(aspect);
                let turned = (angle.rem_euclid(180.0) - 90.0).abs() < 45.0;
                let (rx, ry) = if turned { (radius[1] / ratio, radius[0] * ratio) } else { (radius[0], radius[1]) };
                out.set("rad_on", 1.0);
                out.set("rad_cx", center[0]);
                out.set("rad_cy", center[1]);
                out.set("rad_rx", rx);
                out.set("rad_ry", ry);
                out.set("rad_feather", feather);
                out.set("rad_exposure", a.exposure * k);
                out.set("rad_contrast", a.contrast * k);
                out.set("rad_shadows", a.shadows * k);
                out.set("rad_saturation", a.saturation * k);
                out.set("rad_temperature", a.temperature * k);
            }
            _ => {}
        }
    }
    out.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retouch_keeps_the_drafts_denoise_and_own_masks_and_replaces_its_earlier_masks() {
        use crate::edits::Denoise;
        let mask = |id: u32, name: &str| Mask {
            id,
            name: name.into(),
            shapes: vec![MaskShape::Linear { add: true, from: [0.0, 0.0], to: [0.0, 1.0] }],
            invert: false,
            opacity: 1.0,
            adjust: LocalAdjust { exposure: 0.5, ..LocalAdjust::default() },
            visible: true,
        };
        let mut draft = Develop::default();
        draft.basic.exposure = 1.0;
        draft.detail.denoise = Some(Denoise { strength: 0.5, model: "m@1".into() });
        draft.masks = vec![mask(3, "Sky"), mask(4, LINEAR_NAME)];
        let mut model = Develop::default();
        model.basic.exposure = -0.4;
        model.effects.grain.amount = 0.2;
        model.masks = vec![mask(1, LINEAR_NAME), mask(2, RADIAL_NAME)];

        let d = onto_draft(model, &draft);
        assert_eq!((d.basic.exposure, d.effects.grain.amount), (-0.4, 0.2), "the model's panels");
        assert_eq!(d.detail.denoise, draft.detail.denoise, "denoise kept");
        let masks: Vec<(u32, &str)> = d.masks.iter().map(|m| (m.id, m.name.as_str())).collect();
        assert_eq!(masks, [(3, "Sky"), (4, LINEAR_NAME), (5, RADIAL_NAME)], "own mask kept, earlier Retouch mask replaced");
    }

    fn neutral() -> Vec<f32> {
        PARAMS.iter().map(|p| (p.neutral - p.min) / (p.max - p.min)).collect()
    }

    fn at(v: &mut [f32], name: &str, value: f32) {
        let i = PARAMS.iter().position(|p| p.name == name).unwrap_or_else(|| panic!("no param {name}"));
        let p = PARAMS[i];
        v[i] = (value - p.min) / (p.max - p.min);
    }

    /// A deterministic spread of vectors over the whole cube.
    fn samples() -> Vec<Vec<f32>> {
        let mut seed = 0x2545_f491_4f6c_dd1du64;
        (0..200)
            .map(|_| {
                PARAMS
                    .iter()
                    .map(|_| {
                        seed ^= seed << 13;
                        seed ^= seed >> 7;
                        seed ^= seed << 17;
                        (seed >> 40) as f32 / (1u64 << 24) as f32
                    })
                    .collect()
            })
            .collect()
    }

    #[test]
    fn the_layout_is_sane() {
        assert!(PARAMS.len() > 60, "every panel is there");
        let mut names: Vec<_> = PARAMS.iter().map(|p| p.name).collect();
        names.sort();
        names.dedup();
        assert_eq!(names.len(), PARAMS.len(), "names are unique");
        for p in PARAMS {
            assert!(p.min < p.max, "{}", p.name);
            assert!((p.min..=p.max).contains(&p.neutral), "{}", p.name);
        }
    }

    #[test]
    fn the_neutral_vector_changes_nothing() {
        let r = decode(&neutral(), 1.5);
        assert_eq!(r.develop, Develop::default());
        assert_eq!(r.crop, None);
        assert_eq!(r.straighten, 0.0);
        assert!(r.summary.is_empty(), "{:?}", r.summary);
    }

    #[test]
    fn decode_then_encode_is_the_same_vector() {
        for aspect in [0.66, 1.0, 1.5] {
            for v in samples() {
                let once = encode(&decode(&v, aspect), aspect);
                let twice = encode(&decode(&once, aspect), aspect);
                for (i, (a, b)) in once.iter().zip(&twice).enumerate() {
                    assert!((a - b).abs() < 1e-4, "{aspect}: {} {a} vs {b}", PARAMS[i].name);
                }
            }
        }
    }

    #[test]
    fn a_mask_that_is_off_is_absent() {
        let mut v = neutral();
        at(&mut v, "lin_exposure", -1.0);
        at(&mut v, "lin_on", 0.3);
        assert!(decode(&v, 1.5).develop.masks.is_empty());
        at(&mut v, "lin_on", 0.9);
        let d = decode(&v, 1.5).develop;
        assert_eq!(d.masks.len(), 1);
        assert!(matches!(d.masks[0].shapes[0], MaskShape::Linear { .. }));
        assert_eq!(d.masks[0].adjust.exposure, -1.0);
    }

    #[test]
    fn a_flat_curve_is_no_curve() {
        let mut v = neutral();
        at(&mut v, "curve_m2", 0.1);
        let c = decode(&v, 1.0).develop.curve;
        assert_eq!(c.master.len(), 5);
        assert!((c.master[2][1] - 0.6).abs() < 1e-5);
        assert!(c.r.is_empty() && c.g.is_empty() && c.b.is_empty());
    }

    #[test]
    fn crops_stay_inside_the_photo_whatever_its_shape() {
        for aspect in [0.5, 0.66, 1.0, 1.5, 3.0] {
            for v in samples() {
                if let Some(c) = decode(&v, aspect).crop {
                    assert!(c.x >= 0.0 && c.y >= 0.0 && c.w > 0.0 && c.h > 0.0, "{c:?}");
                    assert!(c.x + c.w <= 1.0 + 1e-5 && c.y + c.h <= 1.0 + 1e-5, "{aspect}: {c:?}");
                }
            }
        }
    }

    #[test]
    fn a_ratio_crop_has_that_ratio_in_pixels() {
        let mut v = neutral();
        at(&mut v, "crop_on", 1.0);
        at(&mut v, "crop_w", 0.8);
        at(&mut v, "crop_x", 0.1);
        at(&mut v, "crop_y", 0.05);
        at(&mut v, "crop_ratio", RATIO_1_1);
        let c = decode(&v, 1.5).crop.unwrap();
        assert!(((c.w * 1.5) / c.h - 1.0).abs() < 1e-4, "square in pixels: {c:?}");
        // 4:5 on a portrait photo is portrait.
        at(&mut v, "crop_ratio", RATIO_4_5);
        let c = decode(&v, 0.66).crop.unwrap();
        assert!(((c.w * 0.66) / c.h - 0.8).abs() < 1e-3, "{c:?}");
    }

    #[test]
    fn grading_wheels_have_no_hue_seam() {
        let mut v = neutral();
        at(&mut v, "grade_shadows_a", 0.0);
        at(&mut v, "grade_shadows_b", -0.5);
        let w = decode(&v, 1.0).develop.grading.shadows;
        assert!((w.hue - 270.0).abs() < 1e-3, "{w:?}");
        assert!((w.sat - 0.5).abs() < 1e-5);
    }

    #[test]
    fn the_summary_says_what_changed() {
        let mut v = neutral();
        at(&mut v, "straighten", 1.4);
        at(&mut v, "crop_on", 1.0);
        at(&mut v, "crop_w", 0.9);
        at(&mut v, "temperature", 20.0);
        at(&mut v, "exposure", 0.5);
        at(&mut v, "lin_on", 1.0);
        at(&mut v, "lin_exposure", -0.6);
        let s = decode(&v, 1.5).summary;
        assert_eq!(s, vec!["straightened 1.4°", "cropped", "brighter", "warmer", "sky darkened"]);
    }

    #[test]
    fn a_turned_radial_keeps_its_shape_in_pixels() {
        let adjust = LocalAdjust { exposure: 0.5, ..LocalAdjust::default() };
        let radial = |angle: f32| {
            let mut d = Develop::default();
            push_mask(&mut d, 2, "r", MaskShape::Radial { add: true, center: [0.4, 0.5], radius: [0.1, 0.3], angle, feather: 0.5, inside: true }, adjust.clone());
            RetouchRecipe { develop: d, ..RetouchRecipe::default() }
        };
        let radius = |angle: f32| match decode(&encode(&radial(angle), 1.5), 1.5).develop.masks[0].shapes[0] {
            MaskShape::Radial { radius, angle, .. } => {
                assert_eq!(angle, 0.0);
                radius
            }
            _ => unreachable!(),
        };
        let upright = radius(10.0);
        assert!((upright[0] - 0.1).abs() < 1e-4 && (upright[1] - 0.3).abs() < 1e-4, "{upright:?}");
        // Turned a quarter: 0.1 of the width (0.15 of the height) now lies along y.
        for angle in [90.0, 100.0, 270.0, -90.0] {
            let r = radius(angle);
            assert!((r[0] - 0.2).abs() < 1e-4 && (r[1] - 0.15).abs() < 1e-4, "{angle}: {r:?}");
        }
    }

    #[test]
    fn values_outside_the_cube_are_clamped() {
        let v: Vec<f32> = PARAMS.iter().map(|_| 7.0).collect();
        let r = decode(&v, 1.0);
        assert_eq!(r.develop.clone().normalized(), r.develop);
        assert!(r.straighten <= 10.0);
    }

    /// Writes the layout for the training scripts, with sample vectors and what Rust reads
    /// them as, so `params.py` can be checked against this file.
    #[test]
    fn writes_the_layout_for_the_training_scripts() {
        let samples: Vec<_> = samples()
            .into_iter()
            .take(6)
            .enumerate()
            .map(|(i, v)| {
                let aspect = [0.66, 1.0, 1.5][i % 3];
                let r = decode(&v, aspect);
                serde_json::json!({ "v": v, "aspect": aspect, "develop": r.develop, "crop": r.crop, "straighten": r.straighten })
            })
            .collect();
        let doc = serde_json::json!({ "layout": RETOUCH_LAYOUT, "ratios": RATIOS, "params": PARAMS, "samples": samples });
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/retouch-params.json");
        std::fs::write(path, serde_json::to_string_pretty(&doc).unwrap() + "\n").unwrap();
    }
}
