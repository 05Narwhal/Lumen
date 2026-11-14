//! Develop's *Auto* (Phase 8d, develop spec §4) in the app: the photo's preview base, cut to
//! its crop at 256 px, measured; CLIP reads the scene; then `photos_core::develop_auto`'s
//! search, whose candidates are rendered here and judged by the aesthetic head.

use image::RgbImage;
use photos_core::develop_auto::{measure, prior, scene_from, search, Eval, Scene, ROUNDS};
use photos_core::edits::{Basic, Curve, Develop, Grading, Hsl, Recipe};
use photos_core::settings::SmartSettings;
use photos_media::develop::{self, geometry, Ctx, Linear};
use serde::Serialize;
use tauri::AppHandle;
use ts_rs::TS;

use crate::ai_task::{AiTask, Emit};
use crate::develop_preview::{base_edge, load_preview_base, source, PREVIEW_EDGE};
use crate::error::{AppError, AppResult};
use crate::state::AppState;

/// The edge Auto measures and judges at: CLIP sees 224 px, so more would be wasted.
const AUTO_EDGE: u32 = 256;

/// What *Auto* chose.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AutoResult {
    pub basic: Basic,
    /// False when smart features are off or the image model is missing: the Basic values are
    /// the measured prior alone.
    pub used_model: bool,
    /// The scene it read the photo as, when one was clear ("sunset").
    pub scene: Option<String>,
    /// `"model"` when the search started from the trained model (Experimental), else
    /// `"classic"`.
    pub source: String,
    /// The trained model's name (its id when unnamed) when `source` is `"model"`.
    pub model: Option<String>,
    /// With the model: its curve, HSL and grading, which Develop applies together with `basic`
    /// as one step. `None` for classic Auto.
    pub look: Option<AutoLook>,
}

/// The panels Auto takes from the trained model besides Basic: tone and colour, never detail,
/// effects or masks (so the answer carries only these).
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AutoLook {
    pub curve: Curve,
    pub hsl: [Hsl; 8],
    pub grading: Grading,
}

impl AutoLook {
    pub fn of(d: &Develop) -> Self {
        Self { curve: d.curve.clone(), hsl: d.hsl, grading: d.grading }
    }

    /// `d` with this look and `basic`: what Develop applies.
    pub fn onto(&self, basic: Basic, d: Develop) -> Develop {
        Develop { basic, curve: self.curve.clone(), hsl: self.hsl, grading: self.grading, ..d }
    }
}

/// Whether Auto starts from the trained model: smart features and the Experimental switch on.
/// Read per call, so a switch flipped while Develop is open counts on the next click.
pub fn use_model(s: &SmartSettings) -> bool {
    s.enabled && s.experimental.auto_model
}

/// The model's answer as Auto's start: its Basic is the search prior, and the draft keeps
/// everything but the model's curve, HSL and grading — never its detail, effects or masks
/// (Auto is tone and colour; crop and straightening are not in a `Develop`).
pub fn prior_from_model(model: &Develop, current: &Develop) -> (Basic, Develop) {
    let develop = Develop { curve: model.curve.clone(), hsl: model.hsl, grading: model.grading, ..current.clone() };
    (model.basic, develop)
}

/// What a cancelled Auto or Retouch returns; Develop applies nothing.
pub fn cancelled() -> AppError {
    AppError { code: "cancelled".into(), message: "Cancelled.".into(), detail: None }
}

/// The judge's error when the task was cancelled between rounds.
const CANCELLED: &str = "cancelled";

pub(crate) fn classic(basic: Basic, used_model: bool, scene: Option<String>) -> AutoResult {
    AutoResult { basic, used_model, scene, source: "classic".into(), model: None, look: None }
}

/// The median display luminance (sRGB-encoded, 0–1) of a candidate as shown.
fn display_median(img: &RgbImage) -> f32 {
    let mut l: Vec<f32> = img.pixels().map(|p| (0.2126 * p.0[0] as f32 + 0.7152 * p.0[1] as f32 + 0.0722 * p.0[2] as f32) / 255.0).collect();
    if l.is_empty() {
        return 0.0;
    }
    let mid = l.len() / 2;
    *l.select_nth_unstable_by(mid, f32::total_cmp).1
}

/// The share of pixels clipped in linear light: any channel ≥ 1, or all ≤ 0.002.
fn clipping(img: &Linear) -> f32 {
    if img.data.is_empty() {
        return 0.0;
    }
    let n = img.data.iter().filter(|p| p.iter().any(|&c| c >= 1.0) || p.iter().all(|&c| c <= 0.002)).count();
    n as f32 / img.data.len() as f32
}

/// Auto on `small` (the photo cut to its crop, before any develop). Without a judge it is the
/// measured prior; with one, the search, each candidate rendered with `develop`'s other
/// panels kept and judged one round per call. Returns the Basic values and whether the judge
/// was used.
pub fn auto_with<J>(small: &Linear, develop: &Develop, scene: Scene, ctx: &Ctx, seed: u64, judge: Option<J>) -> Result<(Basic, bool), String>
where
    J: FnMut(&[RgbImage]) -> Result<Vec<f32>, String>,
{
    auto_from(prior(&measure(&small.data), &scene), small, develop, ctx, seed, judge)
}

/// [`auto_with`] from a given prior: the measured one, or the trained model's Basic.
pub fn auto_from<J>(mut p: Basic, small: &Linear, develop: &Develop, ctx: &Ctx, seed: u64, judge: Option<J>) -> Result<(Basic, bool), String>
where
    J: FnMut(&[RgbImage]) -> Result<Vec<f32>, String>,
{
    // Texture, clarity and dehaze are the user's: Auto searches tone and colour. The search
    // copies them from the prior into every candidate. Only a user's zero dehaze gives way to
    // the prior's (a foggy scene).
    p.texture = develop.basic.texture;
    p.clarity = develop.basic.clarity;
    if develop.basic.dehaze != 0.0 {
        p.dehaze = develop.basic.dehaze;
    }
    let Some(mut judge) = judge else { return Ok((p, false)) };
    let best = search(p, seed, ROUNDS, |bs: &[Basic]| -> Result<Vec<Eval>, String> {
        let mut clips = Vec::with_capacity(bs.len());
        let mut medians = Vec::with_capacity(bs.len());
        let mut images = Vec::with_capacity(bs.len());
        for b in bs {
            let d = Develop { basic: *b, auto: true, ..develop.clone() }.normalized();
            let mut img = small.clone();
            develop::develop_stages(&mut img, &d, ctx);
            clips.push(clipping(&img));
            let shown = img.to_rgb8();
            medians.push(display_median(&shown));
            images.push(shown);
        }
        let scores = judge(&images)?;
        Ok(scores.into_iter().zip(clips).zip(medians).map(|((aesthetic, clipping), median)| Eval { aesthetic, clipping, median }).collect())
    })?;
    Ok((best, true))
}

/// What Auto measures and judges: the base cut to the recipe's geometry first, then brought
/// down to `AUTO_EDGE` — so a tight crop is measured on its own pixels, not on a crop of an
/// already small copy. Also returns the cut frame's long edge, before the reduction.
pub fn auto_frame(base: &Linear, recipe: &Recipe) -> (Linear, u32) {
    let frame = geometry::apply(base, &Recipe { develop: None, ..recipe.clone() });
    let long = frame.long_edge();
    (frame.resize_long_edge(AUTO_EDGE), long)
}

fn failed(e: impl std::fmt::Display) -> AppError {
    AppError { code: "auto_failed".into(), message: "Auto could not read this photo.".into(), detail: Some(e.to_string()) }
}

/// The `develop_auto` command: Auto for `recipe` (the draft on screen) on `asset_id`. With
/// the Experimental switch on and a dev model ready, the search starts from the model's Basic
/// and renders its candidates with the model's curve, HSL and grading. Stages: `reading`,
/// `trying` (one per search round, cancel checked between them), `applying`.
pub fn run<E: Emit>(app: &AppHandle, state: &AppState, asset_id: i64, recipe: &Recipe, task: &AiTask<E>) -> AppResult<AutoResult> {
    task.stage("reading", 0, 0);
    let lib = state.library()?;
    let src = source(&lib, asset_id)?;
    let recipe = recipe.clone().normalized();
    // The preview's own base: Auto and the preview decode the photo once between them.
    let edge = base_edge(if src.long > 0 { src.long } else { PREVIEW_EDGE }, recipe.crop);
    let ffmpeg = state.ffmpeg();
    let base = state.develop.base(asset_id, edge, || load_preview_base(&src, ffmpeg.as_ref(), edge))?;
    // Measured on the cropped frame, before any develop.
    let (small, frame_long) = auto_frame(&base.linear, &recipe);
    // The frame at full resolution is the base's frame scaled by the base's own reduction.
    let full_frame_long = frame_long as f32 * base.full_long.max(1) as f32 / base.linear.long_edge().max(1) as f32;
    let ctx = Ctx::new(small.long_edge() as f32 / full_frame_long.max(1.0), develop::seed_for(&src.hash));
    let current = recipe.develop.clone().unwrap_or_default();
    let seed = develop::seed_for(&src.hash);

    // Experimental: the trained model's tone and colour as the start. Settings are read per
    // call; no model, or the switch off, is classic Auto.
    let with_model = state.settings.read().map(|s| use_model(&s.smart)).unwrap_or(false);
    let model = if with_model { crate::develop_retouch::predict(state, asset_id, &recipe, task, crate::develop_retouch::AiTaskKind::Auto)? } else { None };
    if task.cancelled() {
        return Err(cancelled());
    }
    let (start, develop_for, id) = match model {
        Some((v, aspect, id)) => {
            let (b, d) = prior_from_model(&photos_core::retouch::decode(&v, aspect).develop, &current);
            (Some(b), d, Some(id))
        }
        None => (None, current.clone(), None),
    };
    let result = |basic: Basic, used_model: bool, scene: Option<String>| match &id {
        Some(id) => AutoResult {
            basic,
            used_model,
            scene,
            source: "model".into(),
            model: Some(id.clone()),
            look: Some(AutoLook::of(&develop_for)),
        },
        None => classic(basic, used_model, scene),
    };
    let prior_for = |scene: &Scene| start.unwrap_or_else(|| prior(&measure(&small.data), scene));
    let no_judge = None::<fn(&[RgbImage]) -> Result<Vec<f32>, String>>;

    // The encoder's lock is taken per call, never across renders.
    let embed = |imgs: &[RgbImage]| -> Result<Vec<(photos_core::embedding::Vector, Option<f32>)>, String> {
        let batch: Vec<photos_ml::clip::Rgb<'_>> = imgs.iter().map(|i| photos_ml::clip::Rgb { data: i.as_raw(), width: i.width(), height: i.height() }).collect();
        crate::models::with_encoder(app, |enc| enc.embed_batch(&batch).map_err(|e| e.to_string())).ok_or_else(|| "the image model is not available".to_string())?
    };
    let scene = embed(std::slice::from_ref(&small.to_rgb8())).ok().and_then(|mut v| v.pop()).map(|(v, _)| scene_from(&v, &photos_ml::prototypes()));
    let Some(scene) = scene else {
        let (basic, _) = auto_from(prior_for(&Scene::default()), &small, &develop_for, &ctx, seed, no_judge).map_err(failed)?;
        task.stage("applying", 0, 0);
        return Ok(result(basic, false, None));
    };
    // The judge scores the prior, then each round's candidates: ROUNDS + 1 batches, each
    // reported before it runs, so the ring fills only when the last batch is done.
    let batches = ROUNDS + 1;
    let mut round = 0u32;
    let judge = |imgs: &[RgbImage]| -> Result<Vec<f32>, String> {
        if task.cancelled() {
            return Err(CANCELLED.into());
        }
        task.stage("trying", round.min(batches), batches);
        round += 1;
        Ok(embed(imgs)?.into_iter().map(|(_, a)| a.unwrap_or(0.5)).collect())
    };
    let out = match auto_from(prior_for(&scene), &small, &develop_for, &ctx, seed, Some(judge)) {
        Ok((basic, used)) => result(basic, used, scene.strongest().map(str::to_string)),
        Err(e) if e == CANCELLED => return Err(cancelled()),
        // The model went away mid-search: the prior still stands.
        Err(e) => {
            tracing::warn!("auto: {e}");
            let (basic, _) = auto_from(prior_for(&scene), &small, &develop_for, &ctx, seed, no_judge).map_err(failed)?;
            result(basic, false, None)
        }
    };
    task.stage("applying", 0, 0);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use photos_core::edits::{Curve, Develop};

    #[test]
    fn without_a_model_auto_is_the_measured_prior_and_says_so() {
        let dark = Linear::new(64, 48, [0.01, 0.01, 0.01]);
        let (b, used) = auto_with(&dark, &Develop::default(), Scene::default(), &Ctx::new(1.0, 1), 1, None::<fn(&[RgbImage]) -> Result<Vec<f32>, String>>).unwrap();
        assert!(!used && b.exposure > 1.0, "{b:?}");
    }

    #[test]
    fn auto_keeps_the_users_texture_clarity_and_dehaze() {
        let img = Linear::new(64, 48, [0.05, 0.05, 0.05]);
        let mine = Develop { basic: Basic { texture: 0.2, clarity: 0.3, dehaze: -0.1, ..Default::default() }, ..Default::default() };
        let judge = |imgs: &[RgbImage]| Ok(vec![0.5; imgs.len()]);
        for (b, _) in [
            auto_with(&img, &mine, Scene::default(), &Ctx::new(1.0, 1), 1, None::<fn(&[RgbImage]) -> Result<Vec<f32>, String>>).unwrap(),
            auto_with(&img, &mine, Scene::default(), &Ctx::new(1.0, 1), 1, Some(judge)).unwrap(),
        ] {
            assert_eq!((b.texture, b.clarity, b.dehaze), (0.2, 0.3, -0.1), "{b:?}");
        }
    }

    #[test]
    fn a_tight_crop_is_measured_at_the_bases_resolution() {
        use photos_core::edits::Crop;
        let base = Linear::new(2000, 1000, [0.1, 0.1, 0.1]);
        let recipe = Recipe { crop: Some(Crop { x: 0.4, y: 0.4, w: 0.1, h: 0.2 }), ..Recipe::identity() };
        let (frame, long) = auto_frame(&base, &recipe);
        assert_eq!((frame.long_edge(), long), (200, 200), "the crop's own 200 px, not a crop of a 256 px copy");
        let (wide, long) = auto_frame(&Linear::new(4000, 3000, [0.1; 3]), &Recipe::identity());
        assert_eq!((wide.long_edge(), long), (256, 4000));
    }

    #[test]
    fn the_model_lends_auto_its_tone_and_colour_only() {
        let mut model = Develop { basic: Basic { exposure: 0.7, contrast: 0.2, texture: 0.9, ..Default::default() }, ..Default::default() };
        model.curve = Curve { master: vec![[0.0, 0.1], [1.0, 0.9]], ..Default::default() };
        model.hsl[2].s = 0.4;
        model.grading.shadows.hue = 200.0;
        model.detail.sharpen.amount = 0.8;
        model.effects.vignette.amount = -0.5;
        let mut mine = Develop::default();
        mine.detail.sharpen.amount = 0.1;
        mine.effects.grain.amount = 0.3;
        let (prior, d) = prior_from_model(&model, &mine);
        assert_eq!(prior, model.basic);
        assert_eq!((&d.curve, &d.hsl, &d.grading), (&model.curve, &model.hsl, &model.grading));
        assert_eq!((&d.detail, &d.effects, &d.masks, d.basic), (&mine.detail, &mine.effects, &mine.masks, mine.basic), "detail, effects, masks are the draft's");
    }

    #[test]
    fn without_a_judge_auto_from_the_model_is_the_models_basic_with_the_users_texture() {
        let img = Linear::new(64, 48, [0.05, 0.05, 0.05]);
        let mine = Develop { basic: Basic { texture: 0.2, clarity: 0.3, dehaze: -0.1, ..Default::default() }, ..Default::default() };
        let start = Basic { exposure: 0.4, contrast: 0.15, texture: 0.9, clarity: 0.9, dehaze: 0.0, ..Default::default() };
        let (b, used) = auto_from(start, &img, &mine, &Ctx::new(1.0, 1), 1, None::<fn(&[RgbImage]) -> Result<Vec<f32>, String>>).unwrap();
        assert!(!used);
        assert_eq!(b, Basic { texture: 0.2, clarity: 0.3, dehaze: -0.1, ..start });
    }

    #[test]
    fn auto_uses_the_model_only_with_smart_features_and_the_switch_on() {
        let mut s = SmartSettings::default();
        assert!(!use_model(&s), "off by default: classic");
        s.experimental.auto_model = true;
        assert!(use_model(&s));
        s.enabled = false;
        assert!(!use_model(&s), "smart features off: classic");
        let r = classic(Basic::default(), false, None);
        assert_eq!((r.source.as_str(), r.model, r.look), ("classic", None, None));
        // The model's answer carries its tone and colour panels only, not its masks.
        let mut d = Develop::default();
        d.hsl[1].s = 20.0;
        d.masks = vec![photos_core::edits::Mask { id: 1, name: "m".into(), shapes: vec![], invert: false, opacity: 1.0, adjust: Default::default(), visible: true }];
        let look = AutoLook::of(&d);
        let json = serde_json::to_value(&look).unwrap();
        let mut keys: Vec<_> = json.as_object().unwrap().keys().cloned().collect();
        keys.sort();
        assert_eq!(keys, ["curve", "grading", "hsl"]);
        let on = look.onto(Basic { exposure: 0.3, ..Basic::default() }, Develop::default());
        assert_eq!((on.hsl[1].s, on.basic.exposure, on.masks.len()), (20.0, 0.3, 0));
    }

    #[test]
    fn candidates_are_rendered_with_the_other_panels_kept() {
        let img = Linear::new(64, 48, [0.1, 0.1, 0.1]);
        // A curve that crushes everything under a tenth: unmistakable in every candidate.
        let dark = Develop { curve: Curve { master: vec![[0.0, 0.0], [1.0, 0.1]], ..Default::default() }, ..Default::default() };
        let mut brightest = 0u8;
        let mut batches = 0;
        let judge = |imgs: &[RgbImage]| {
            batches += 1;
            brightest = brightest.max(imgs.iter().flat_map(|i| i.pixels().flat_map(|p| p.0)).max().unwrap_or(0));
            Ok(vec![0.5; imgs.len()])
        };
        let (_, used) = auto_with(&img, &dark, Scene::default(), &Ctx::new(1.0, 1), 1, Some(judge)).unwrap();
        assert!(used);
        assert_eq!(batches, 1 + photos_core::develop_auto::ROUNDS as usize, "one batch per round");
        assert!(brightest <= 30, "the curve stayed in every candidate: {brightest}");
    }

    // ---- Round 5: what Auto puts on screen, measured on synthetic scenes (never Julian's
    // photos). Each scene is a seeded texture with a known exposure, spread and cast.

    /// A deterministic stream of normals (Box–Muller over SplitMix-ish integers).
    struct Noise(u64);
    impl Noise {
        fn uniform(&mut self) -> f32 {
            self.0 = self.0.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
            ((self.0 >> 40) as f32 + 0.5) / (1u64 << 24) as f32
        }
        fn normal(&mut self) -> f32 {
            let (u, v) = (self.uniform(), self.uniform());
            (-2.0 * u.ln()).sqrt() * (std::f32::consts::TAU * v).cos()
        }
    }

    /// `share` of the frame drawn around `median` (linear) with `stops` of spread (one sigma),
    /// tinted by `cast`; the rest around `median2`.
    fn texture(seed: u64, parts: &[(f32, f32, f32)], cast: [f32; 3]) -> Linear {
        let (w, h) = (256u32, 170u32);
        let mut n = Noise(seed);
        let total: f32 = parts.iter().map(|p| p.0).sum();
        let data = (0..w * h)
            .map(|i| {
                // Parts are bands down the frame, as sky over ground is.
                let y = (i / w) as f32 / h as f32 * total;
                let mut acc = 0.0;
                let part = parts.iter().find(|p| {
                    acc += p.0;
                    y < acc
                });
                let (_, median, stops) = *part.unwrap_or(&parts[parts.len() - 1]);
                let l = median * 2f32.powf(stops * n.normal());
                let c = 1.0 + 0.08 * n.normal();
                [l * cast[0] * c, l * cast[1], l * cast[2] / c]
            })
            .collect();
        Linear { width: w, height: h, data }
    }

    /// The scenes Auto is judged on, by name.
    fn scenes() -> Vec<(&'static str, Linear)> {
        let neutral = [1.0, 1.0, 1.0];
        vec![
            ("daylight", texture(1, &[(1.0, 0.16, 1.2)], neutral)),
            ("dim interior", texture(2, &[(1.0, 0.025, 1.3)], [1.05, 1.0, 0.9])),
            ("flat fog", texture(3, &[(1.0, 0.3, 0.35)], neutral)),
            ("backlit", texture(4, &[(0.35, 0.85, 0.3), (0.65, 0.03, 1.0)], neutral)),
            ("hard light", texture(5, &[(1.0, 0.09, 2.6)], neutral)),
            ("tungsten room", texture(6, &[(1.0, 0.06, 1.2)], [1.45, 1.0, 0.55])),
        ]
    }

    /// What the eye sees of a render: percentiles of display luma (sRGB-encoded, 0–1) and the
    /// shares crushed to black and blown to white.
    #[derive(Debug)]
    struct Shown {
        p02: f32,
        median: f32,
        p98: f32,
        crushed: f32,
        blown: f32,
    }

    fn shown(img: &Linear) -> Shown {
        let rgb = img.to_rgb8();
        let mut l: Vec<f32> = rgb.pixels().map(|p| (0.2126 * p.0[0] as f32 + 0.7152 * p.0[1] as f32 + 0.0722 * p.0[2] as f32) / 255.0).collect();
        l.sort_by(f32::total_cmp);
        let n = l.len();
        let at = |q: f32| l[((q * (n - 1) as f32).round() as usize).min(n - 1)];
        let max = |p: &image::Rgb<u8>| p.0.iter().copied().max().unwrap_or(0);
        Shown {
            p02: at(0.02),
            median: at(0.5),
            p98: at(0.98),
            crushed: rgb.pixels().filter(|p| max(p) <= 3).count() as f32 / n as f32,
            blown: rgb.pixels().filter(|p| p.0.iter().any(|&c| c >= 254)).count() as f32 / n as f32,
        }
    }

    /// Auto's prior on a scene, rendered as it would be shown.
    fn auto_shown(img: &Linear) -> (Basic, Shown) {
        let none = None::<fn(&[RgbImage]) -> Result<Vec<f32>, String>>;
        let (b, _) = auto_with(img, &Develop::default(), Scene::default(), &Ctx::new(1.0, 1), 1, none).unwrap();
        let mut out = img.clone();
        develop::develop_stages(&mut out, &Develop { basic: b, auto: true, ..Default::default() }.normalized(), &Ctx::new(1.0, 1));
        (b, shown(&out))
    }

    /// The before/after table in the Round 5 plan: `cargo test -p lumen --lib auto_report -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn auto_report() {
        println!("{:<14} {:>22} | {:>6} {:>6} {:>6} {:>7} {:>7} | exp   con   hi    sh    wh    bl", "scene", "input p02/med/p98", "p02", "median", "p98", "crushed", "blown");
        for (name, img) in scenes() {
            let before = shown(&img);
            let (b, s) = auto_shown(&img);
            println!(
                "{name:<14} {:>6.3} {:>6.3} {:>6.3}   | {:>6.3} {:>6.3} {:>6.3} {:>7.3} {:>7.3} | {:+.2} {:+.2} {:+.2} {:+.2} {:+.2} {:+.2}",
                before.p02, before.median, before.p98, s.p02, s.median, s.p98, s.crushed, s.blown, b.exposure, b.contrast, b.highlights, b.shadows, b.whites, b.blacks
            );
        }
    }

    /// Round 5: Auto was too dark, too contrasty and generally timid. On every scene the
    /// result sits near a mid-grey median, keeps its shadows open and its whites bright, and
    /// blows no more than the photo already did.
    #[test]
    fn auto_opens_up_every_scene() {
        for (name, img) in scenes() {
            let input = shown(&img);
            let (b, s) = auto_shown(&img);
            // A photo that already blows (a backlit sky, hard sun) is brightened only as far
            // as it can be without blowing more: Shadows does that lifting, not exposure.
            let floor = if input.blown > 0.02 { 0.38 } else { 0.45 };
            assert!((floor..=0.58).contains(&s.median), "{name}: median {s:?} {b:?}");
            assert!(s.crushed <= 0.01 && s.p02 >= input.p02.min(0.06), "{name}: shadows {s:?} {b:?}");
            assert!(s.blown <= input.blown.max(0.015) + 0.01, "{name}: blown {s:?} from {input:?} {b:?}");
            // Whites are never left duller than the photo's own (Auto used to pull them a
            // stop down nearly every time).
            // A blown sky brought back sits just under white, which is the point.
            let whites = if input.blown > 0.02 { 0.85 } else { input.p98.min(0.92) - 0.01 };
            assert!(s.p98 >= whites, "{name}: dull whites {s:?} from {input:?} {b:?}");
        }
    }

    #[test]
    fn auto_adds_contrast_only_where_a_photo_is_flat() {
        let all = scenes();
        let get = |n: &str| auto_shown(&all.iter().find(|s| s.0 == n).unwrap().1).0;
        assert!(get("daylight").contrast <= 0.15, "{:?}", get("daylight"));
        assert!(get("hard light").contrast <= 0.0, "{:?}", get("hard light"));
        assert!(get("flat fog").contrast >= 0.2, "{:?}", get("flat fog"));
    }

    #[test]
    fn a_bright_misty_scene_is_not_darkened_much() {
        let fog = scenes().into_iter().find(|s| s.0 == "flat fog").unwrap().1;
        let (b, s) = auto_shown(&fog);
        assert!(b.exposure >= -0.4 && s.median >= 0.5, "{b:?} {s:?}");
    }
}
