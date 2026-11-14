//! *Retouch* with Lumen's own trained model (Round 12 spec §4): the photo's uncropped preview
//! base prepared exactly as the training scripts see it (`retouch_input::prepare`), its stored
//! CLIP embedding, the dev model's `v`, decoded into a whole recipe. Every way the model can be
//! unusable is a state: Retouch then falls back to classic Auto. It writes nothing.

use photos_core::edits::{Crop, Develop, Recipe};
use photos_core::retouch::{decode, onto_draft, Group, PARAMS};
use photos_media::retouch_input::prepare;
use serde::{Deserialize, Serialize};
use tauri::AppHandle;
use ts_rs::TS;

use crate::ai_task::{AiTask, Emit};
use crate::develop_auto::{cancelled, AutoResult};
use crate::develop_preview::{base_edge, load_preview_base, source, PREVIEW_EDGE};
use crate::error::AppResult;
use crate::model_catalogue::{CatalogueModel, Tier};
use photos_core::settings::{TaskChoice, TierChoice};
use crate::state::AppState;

/// The dev models folder's role for both Experimental features.
pub const ROLE: &str = "retouch";

/// What *Retouch* proposes: a whole recipe, applied by Develop as one history step.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RetouchResult {
    pub develop: Develop,
    pub crop: Option<Crop>,
    pub straighten: f32,
    /// False when the trained model was not used and this is classic Auto.
    pub used_model: bool,
    pub summary: Vec<String>,
    /// The model's name (its id when unnamed) when the model was used.
    pub model: Option<String>,
}

/// The model's `v` for a photo of `aspect` as a result, over the draft's develop (its denoise
/// and the person's own masks kept: `photos_core::retouch::onto_draft`).
pub fn retouch_from(v: &[f32], aspect: f32, id: &str, draft: &Develop) -> RetouchResult {
    let r = decode(v, aspect);
    let develop = onto_draft(r.develop, draft);
    RetouchResult { develop, crop: r.crop, straighten: r.straighten, used_model: true, summary: r.summary, model: Some(id.to_string()) }
}

/// Classic Auto's answer as a result: the draft's other panels and geometry kept.
pub fn fallback(auto: AutoResult, recipe: &Recipe) -> RetouchResult {
    // Auto that started from the model carries the tone panels its Basic was searched under.
    let draft = recipe.develop.clone().unwrap_or_default();
    let develop = match &auto.look {
        Some(look) => look.onto(auto.basic, draft),
        None => Develop { basic: auto.basic, ..draft },
    };
    RetouchResult {
        develop,
        crop: recipe.crop,
        straighten: recipe.straighten,
        used_model: false,
        summary: vec!["Auto (Retouch model not ready)".into()],
        model: None,
    }
}

/// The geometry Retouch reads the photo with: the draft's rotation and flips, no crop and no
/// straightening, since it proposes its own.
fn whole_photo(recipe: &Recipe) -> Recipe {
    Recipe { crop: None, straighten: 0.0, develop: None, ..recipe.clone() }
}

/// The stored CLIP embedding dequantised; `None` (zeros and flag 0) while indexing has not
/// reached the photo or the vector is not the model's width.
fn clip_input(stored: Option<photos_core::embedding::Features>) -> Option<Vec<f32>> {
    stored.filter(|f| f.vec.len() == photos_ml::retouch::CLIP).map(|f| f.vec.iter().map(|&x| x as f32 / 127.0).collect())
}

/// The `kind`'s model's `v` for `asset_id`, the photo's aspect and the model's name; `None`
/// while no model of the task's family runs (`resolve`). It ignores the Experimental switches:
/// callers check them. The base is the whole photo as displayed (the draft's rotation and flips
/// kept, its crop and straightening dropped), so the crop that comes back replaces the draft's.
/// A tier with `tta` 5 runs five views (`tta_views`) and merges them. Reports `predicting` per
/// pass, and returns `cancelled` if the task was, between passes too. Anything but a cancel
/// that goes wrong on the way is logged and reads as no model, so Auto and Retouch fall back
/// to classic Auto.
pub fn predict<E: Emit>(
    state: &AppState,
    asset_id: i64,
    recipe: &Recipe,
    task: &AiTask<E>,
    kind: AiTaskKind,
) -> AppResult<Option<(Vec<f32>, f32, String)>> {
    match try_predict(state, asset_id, recipe, task, kind) {
        Err(e) if e.code != "cancelled" => {
            tracing::warn!(asset_id, "retouch model: {} ({:?})", e.message, e.detail);
            Ok(None)
        }
        out => out,
    }
}

fn try_predict<E: Emit>(
    state: &AppState,
    asset_id: i64,
    recipe: &Recipe,
    task: &AiTask<E>,
    kind: AiTaskKind,
) -> AppResult<Option<(Vec<f32>, f32, String)>> {
    let Some((model, tier)) = resolve(state, kind) else { return Ok(None) };
    let Some((path, entry, tta)) = model.ready(tier) else { return Ok(None) };
    let name = entry.name.clone().unwrap_or_else(|| model.name.clone());
    let lib = state.library()?;
    let src = source(&lib, asset_id)?;
    // The preview's own base: the model reads 256 px, so the larger base a cropped photo's
    // preview holds does, and is never evicted for an uncropped one (which would decode the
    // RAW twice more per click).
    let edge = base_edge(if src.long > 0 { src.long } else { PREVIEW_EDGE }, None);
    let ffmpeg = state.ffmpeg();
    let base = state.develop.base_at_least(asset_id, edge, || load_preview_base(&src, ffmpeg.as_ref(), edge))?;
    let whole = photos_media::develop::geometry::apply(&base.linear, &whole_photo(recipe));
    let clip = clip_input(lib.db.with_conn(|c| photos_core::embedding::load(c, &[asset_id]))?.remove(&asset_id));
    let views = tta_views(whole.width, whole.height, tta);
    let mut outputs = Vec::with_capacity(views.len());
    let mut aspect = 1.0;
    for (i, view) in views.iter().enumerate() {
        if task.cancelled() {
            return Err(cancelled());
        }
        // One pass reports as before (no count); TTA reports pass i of n.
        let (done, total) = if views.len() > 1 { (i as u32, views.len() as u32) } else { (0, 0) };
        task.stage("predicting", done, total);
        // The first view is the photo itself: its aspect is the photo's.
        let prepared = if i == 0 { prepare(&whole) } else { prepare(&view_linear(&whole, view)) };
        if i == 0 {
            aspect = prepared.aspect;
        }
        match crate::models::with_retoucher(state, kind.family(), path, entry, |m| m.run(&prepared.input, clip.as_deref())) {
            Some(Ok(v)) => outputs.push(v),
            Some(Err(e)) => {
                tracing::warn!("retouch model {}: {e}", entry.id);
                return Ok(None);
            }
            None => return Ok(None),
        }
    }
    Ok(Some((merge(&outputs, &views), aspect, name)))
}

/// The `develop_retouch` command: the trained model's recipe for `asset_id`, or classic Auto
/// while the switch or smart features are off or the model is not usable. Stages: `reading`,
/// `predicting`, `applying` (classic Auto's own when it falls back); cancel between them.
pub fn run<E: Emit>(app: &AppHandle, state: &AppState, asset_id: i64, recipe: &Recipe, task: &AiTask<E>) -> AppResult<RetouchResult> {
    let on = state.settings.read().map(|s| s.smart.enabled && s.smart.experimental.retouch).unwrap_or(false);
    if on {
        task.stage("reading", 0, 0);
        if let Some((v, aspect, id)) = predict(state, asset_id, recipe, task, AiTaskKind::Retouch)? {
            if task.cancelled() {
                return Err(cancelled());
            }
            task.stage("applying", 0, 0);
            return Ok(retouch_from(&v, aspect, &id, &recipe.develop.clone().unwrap_or_default()));
        }
    }
    Ok(fallback(crate::develop_auto::run(app, state, asset_id, recipe, task)?, recipe))
}

/// The AI tasks Settings → Experimental lists, each run by one model family.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export)]
pub enum AiTaskKind {
    Auto,
    Retouch,
    Search,
    Text,
    Masks,
}

impl AiTaskKind {
    pub fn family(self) -> &'static str {
        match self {
            AiTaskKind::Auto => "ray",
            AiTaskKind::Retouch => "prism",
            AiTaskKind::Search => "spectrum",
            AiTaskKind::Text => "glyph",
            AiTaskKind::Masks => "facet",
        }
    }

    /// The model time one call may take before a tier counts as too slow here.
    pub fn budget_ms(self) -> Option<f32> {
        match self {
            AiTaskKind::Auto => Some(1000.0),
            AiTaskKind::Retouch => Some(1500.0),
            _ => None,
        }
    }
}

/// The model and tier `kind` runs given the person's `choice`; `recommend` gives the tier
/// for *Automatic*. A model or tier that is gone falls back (`model_catalogue::pick`); `None`
/// when no model of the family runs.
pub fn resolve_in(
    all: &[CatalogueModel],
    kind: AiTaskKind,
    choice: &TaskChoice,
    recommend: impl Fn(&CatalogueModel) -> Tier,
) -> Option<(CatalogueModel, Tier)> {
    let family = kind.family();
    let (model, _) = crate::model_catalogue::pick(all, family, choice.model.as_deref(), Tier::High)?;
    let want = match choice.tier {
        TierChoice::Auto => recommend(&model),
        TierChoice::Low => Tier::Low,
        TierChoice::High => Tier::High,
        TierChoice::Ultra => Tier::Ultra,
    };
    crate::model_catalogue::pick(all, family, Some(&model.id), want)
}

/// `resolve_in` over the dev folder's catalogue with the person's settings.
pub fn resolve(state: &AppState, kind: AiTaskKind) -> Option<(CatalogueModel, Tier)> {
    let choice = state
        .settings
        .read()
        .map(|s| match kind {
            AiTaskKind::Auto => s.smart.experimental.tasks.auto.clone(),
            AiTaskKind::Retouch => s.smart.experimental.tasks.retouch.clone(),
            _ => TaskChoice::default(),
        })
        .unwrap_or_default();
    resolve_in(&crate::model_catalogue::catalogue(), kind, &choice, |m| crate::model_bench::recommended_now(state, kind, m))
}

/// One test-time augmentation pass (Round 14 spec §3): a crop of the whole photo, maybe
/// mirrored. A point `(u, v)` of the view lies at `x = x0 + sx * (if mirror { 1 - u } else { u })`,
/// `y = y0 + sy * v` of the whole photo, all in 0–1.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct View {
    pub mirror: bool,
    pub x0: f32,
    pub y0: f32,
    pub sx: f32,
    pub sy: f32,
}

const IDENTITY: View = View { mirror: false, x0: 0.0, y0: 0.0, sx: 1.0, sy: 1.0 };
/// The share of the long side each TTA crop keeps, and where the three crops start.
const TTA_CROP: f32 = 0.9;
const TTA_SHIFTS: [f32; 3] = [0.0, 0.05, 0.1];

/// The views for `passes` passes over a `width`×`height` photo: one pass is the photo itself;
/// five add its mirror and three 90 % crops shifted along the long side (x when square).
pub fn tta_views(width: u32, height: u32, passes: u8) -> Vec<View> {
    if passes < 5 {
        return vec![IDENTITY];
    }
    let along_x = width >= height;
    let mut views = vec![IDENTITY, View { mirror: true, ..IDENTITY }];
    views.extend(TTA_SHIFTS.iter().map(|&s| {
        if along_x {
            View { x0: s, sx: TTA_CROP, ..IDENTITY }
        } else {
            View { y0: s, sy: TTA_CROP, ..IDENTITY }
        }
    }));
    views
}

/// The view's pixels: the crop of `base`, then its mirror.
pub fn view_linear(base: &photos_media::develop::Linear, v: &View) -> photos_media::develop::Linear {
    let (w, h) = (base.width, base.height);
    // Start and length in pixels of a view's span along a side of `n` pixels.
    let span = |start: f32, share: f32, n: u32| {
        let len = ((share * n as f32).round() as u32).clamp(1.min(n), n);
        let from = ((start * n as f32).round() as u32).min(n - len);
        from..from + len
    };
    let (xs, ys) = (span(v.x0, v.sx, w), span(v.y0, v.sy, h));
    let (cw, ch) = (xs.len() as u32, ys.len() as u32);
    let mut data = Vec::with_capacity(xs.len() * ys.len());
    for y in ys {
        let row = &base.data[(y * w) as usize..((y + 1) * w) as usize][xs.start as usize..xs.end as usize];
        if v.mirror {
            data.extend(row.iter().rev());
        } else {
            data.extend_from_slice(row);
        }
    }
    photos_media::develop::Linear { width: cw, height: ch, data }
}

/// `v`'s entry `i` as the parameter's value, read as `photos_core::retouch::decode` reads it:
/// non-finite is neutral, otherwise clamped to 0–1 first.
fn to_value(i: usize, u: f32) -> f32 {
    let p = &PARAMS[i];
    let unit = if u.is_finite() { u.clamp(0.0, 1.0) } else { (p.neutral - p.min) / (p.max - p.min) };
    p.min + unit * (p.max - p.min)
}

fn from_value(i: usize, value: f32) -> f32 {
    let p = &PARAMS[i];
    ((value - p.min) / (p.max - p.min)).clamp(0.0, 1.0)
}

/// Merges the passes' `v`s (one per view, in `tta_views` order) into one: geometry from the
/// first, unmirrored full-frame pass; gradient coordinates mapped back to the whole photo
/// (mirror undone, crop offset added, radii scaled), then averaged; everything else averaged.
pub fn merge(outputs: &[Vec<f32>], views: &[View]) -> Vec<f32> {
    let Some(first) = outputs.first() else { return Vec::new() };
    if outputs.len() == 1 {
        return first.clone();
    }
    let passes = outputs.iter().zip(views);
    let n = outputs.len().min(views.len()) as f32;
    (0..first.len())
        .map(|i| {
            let Some(p) = PARAMS.get(i) else { return first[i] };
            if p.group == Group::Geometry {
                return first[i];
            }
            let to_photo: fn(f32, &View) -> f32 = match p.name {
                "lin_fx" | "lin_tx" | "rad_cx" => |x, v| v.x0 + v.sx * if v.mirror { 1.0 - x } else { x },
                "lin_fy" | "lin_ty" | "rad_cy" => |y, v| v.y0 + v.sy * y,
                "rad_rx" => |r, v| r * v.sx,
                "rad_ry" => |r, v| r * v.sy,
                _ => |x, _| x,
            };
            let sum: f32 = passes.clone().map(|(o, v)| to_photo(to_value(i, o.get(i).copied().unwrap_or(f32::NAN)), v)).sum();
            from_value(i, sum / n)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use photos_core::edits::Basic;
    use std::path::PathBuf;

    fn fixture() -> serde_json::Value {
        let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("crates/photos-ml/tests/fixtures/retouch-tiny.json");
        serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap()
    }

    fn vectors() -> Vec<Vec<f32>> {
        let doc = fixture();
        let mut out: Vec<Vec<f32>> = doc
            .as_object()
            .unwrap()
            .values()
            .filter_map(|v| v.as_array())
            .map(|a| a.iter().map(|x| x.as_f64().unwrap() as f32).collect())
            .collect();
        assert!(!out.is_empty() && out.iter().all(|v| v.len() == PARAMS.len()));
        // The extremes with the crop switched on: whatever the model says, the crop stays in.
        let on = PARAMS.iter().position(|p| p.name == "crop_on").unwrap();
        for fill in [0.0, 1.0, 0.5] {
            let mut v = vec![fill; PARAMS.len()];
            v[on] = 1.0;
            out.push(v);
        }
        out
    }

    #[test]
    fn crops_lie_inside_the_photo_for_portrait_and_panorama() {
        let mut cropped = 0;
        for aspect in [0.5, 3.0, 1.0] {
            for v in vectors() {
                let r = retouch_from(&v, aspect, "retouch-dev-x", &Develop::default());
                assert!(r.used_model);
                assert_eq!(r.model.as_deref(), Some("retouch-dev-x"));
                if let Some(c) = r.crop {
                    cropped += 1;
                    let e = 1e-5;
                    assert!(c.x >= -e && c.y >= -e && c.w > 0.0 && c.h > 0.0, "{c:?} at {aspect}");
                    assert!(c.x + c.w <= 1.0 + e && c.y + c.h <= 1.0 + e, "{c:?} at {aspect}");
                }
            }
        }
        assert!(cropped > 0, "no vector produced a crop to check");
    }

    #[test]
    fn fallback_keeps_the_drafts_geometry_and_other_panels_and_says_why() {
        let crop = Crop { x: 0.1, y: 0.2, w: 0.5, h: 0.6 };
        let mut develop = Develop::default();
        develop.basic.clarity = 30.0;
        develop.hsl[0].s = 12.0;
        let recipe = Recipe { crop: Some(crop), straighten: 2.5, develop: Some(develop), ..Recipe::default() };
        let basic = Basic { exposure: 0.7, clarity: 30.0, ..Basic::default() };
        let r = fallback(crate::develop_auto::classic(basic, true, None), &recipe);
        assert!(!r.used_model);
        assert_eq!(r.model, None);
        assert_eq!(r.crop, Some(crop));
        assert_eq!(r.straighten, 2.5);
        assert_eq!(r.develop.basic, basic);
        assert_eq!(r.develop.hsl[0].s, 12.0);
        assert_eq!(r.summary, vec!["Auto (Retouch model not ready)".to_string()]);
    }

    #[test]
    fn fallback_from_auto_with_the_model_keeps_the_tone_panels_its_basic_was_searched_under() {
        let mut model_tone = Develop::default();
        model_tone.basic.exposure = -0.4;
        model_tone.hsl[2].h = 8.0;
        model_tone.curve.master = vec![[0.0, 0.0], [0.5, 0.6], [1.0, 1.0]];
        let look = crate::develop_auto::AutoLook::of(&model_tone);
        let auto = AutoResult { look: Some(look), source: "model".into(), ..crate::develop_auto::classic(model_tone.basic, true, None) };
        // The draft's masks and detail stay; only Basic and the look come from Auto.
        let mut draft = Develop::default();
        draft.detail.sharpen.amount = 0.8;
        draft.masks = vec![photos_core::edits::Mask { id: 3, name: "Mine".into(), shapes: vec![], invert: false, opacity: 1.0, adjust: Default::default(), visible: true }];
        let r = fallback(auto, &Recipe { develop: Some(draft.clone()), ..Recipe::default() });
        assert_eq!(r.develop, Develop { basic: model_tone.basic, curve: model_tone.curve.clone(), hsl: model_tone.hsl, grading: model_tone.grading, ..draft });
    }

    #[test]
    fn a_cropped_straightened_draft_is_read_whole_with_its_rotation_kept() {
        let recipe = Recipe {
            crop: Some(Crop { x: 0.2, y: 0.2, w: 0.3, h: 0.3 }),
            straighten: -4.0,
            rotate: 90,
            flip_h: true,
            develop: Some(Develop::default()),
            ..Recipe::default()
        };
        let w = whole_photo(&recipe);
        assert_eq!((w.crop, w.straighten, w.develop), (None, 0.0, None));
        assert_eq!((w.rotate, w.flip_h, w.flip_v), (90, true, false));
    }

    #[test]
    fn clip_is_dequantised_or_absent() {
        use photos_core::embedding::Features;
        assert_eq!(clip_input(None), None);
        assert_eq!(clip_input(Some(Features { vec: vec![1; 10], aesthetic: None })), None);
        let v = clip_input(Some(Features { vec: vec![127; photos_ml::retouch::CLIP], aesthetic: Some(0.5) })).unwrap();
        assert_eq!(v.len(), 512);
        assert!(v.iter().all(|&x| (x - 1.0).abs() < 1e-6));
    }

    fn legacy_only() -> (tempfile::TempDir, Vec<crate::model_catalogue::CatalogueModel>) {
        let dir = tempfile::tempdir().unwrap();
        crate::model_catalogue::tests::write_legacy(dir.path(), b"ray weights");
        let all = crate::model_catalogue::catalogue_in(dir.path());
        (dir, all)
    }

    #[test]
    fn auto_uses_the_legacy_model() {
        let (_dir, all) = legacy_only();
        let untimed = |_: &crate::model_catalogue::CatalogueModel| Tier::High;
        let (m, tier) = resolve_in(&all, AiTaskKind::Auto, &TaskChoice::default(), untimed).expect("Ray");
        assert!(m.legacy && m.family == "ray");
        assert_eq!(tier, Tier::High);
        let ultra = TaskChoice { model: None, tier: TierChoice::Ultra };
        assert_eq!(resolve_in(&all, AiTaskKind::Auto, &ultra, untimed).unwrap().1, Tier::High, "legacy has only High");
    }

    #[test]
    fn retouch_needs_a_prism_model() {
        let (_dir, all) = legacy_only();
        assert!(resolve_in(&all, AiTaskKind::Retouch, &TaskChoice::default(), |_| Tier::High).is_none());
        assert!(resolve_in(&all, AiTaskKind::Search, &TaskChoice::default(), |_| Tier::High).is_none());
    }

    #[test]
    fn tasks_map_to_families_and_budgets() {
        let f: Vec<_> = [AiTaskKind::Auto, AiTaskKind::Retouch, AiTaskKind::Search, AiTaskKind::Text, AiTaskKind::Masks]
            .iter()
            .map(|k| (k.family(), k.budget_ms()))
            .collect();
        assert_eq!(
            f,
            vec![("ray", Some(1000.0)), ("prism", Some(1500.0)), ("spectrum", None), ("glyph", None), ("facet", None)]
        );
    }

    fn at(name: &str) -> usize {
        PARAMS.iter().position(|p| p.name == name).unwrap()
    }

    fn neutral() -> Vec<f32> {
        (0..PARAMS.len()).map(|i| from_value(i, PARAMS[i].neutral)).collect()
    }

    #[test]
    fn value_mapping_round_trips_every_param() {
        for (i, p) in PARAMS.iter().enumerate() {
            for u in [0.0, 0.25, 0.5, 1.0] {
                assert!((from_value(i, to_value(i, u)) - u).abs() < 1e-5, "{}", p.name);
            }
            assert!((to_value(i, f32::NAN) - p.neutral).abs() < 1e-5, "{}", p.name);
        }
    }

    #[test]
    fn one_pass_is_identity() {
        let v: Vec<f32> = (0..PARAMS.len()).map(|i| (i % 7) as f32 / 7.0).collect();
        assert_eq!(tta_views(400, 300, 1), vec![IDENTITY]);
        assert_eq!(merge(std::slice::from_ref(&v), &tta_views(400, 300, 1)), v);
    }

    #[test]
    fn views_shift_along_the_long_side() {
        let crops = |w, h| tta_views(w, h, 5).into_iter().skip(2).collect::<Vec<_>>();
        let land = tta_views(400, 300, 5);
        assert_eq!(land.len(), 5);
        assert_eq!(land[0], IDENTITY);
        assert_eq!(land[1], View { mirror: true, ..IDENTITY });
        for (c, x0) in crops(400, 300).iter().zip([0.0, 0.05, 0.1]) {
            assert_eq!(*c, View { mirror: false, x0, y0: 0.0, sx: 0.9, sy: 1.0 });
        }
        for (c, y0) in crops(300, 400).iter().zip([0.0, 0.05, 0.1]) {
            assert_eq!(*c, View { mirror: false, x0: 0.0, y0, sx: 1.0, sy: 0.9 });
        }
        assert!(crops(300, 300).iter().all(|c| c.sx == 0.9 && c.sy == 1.0));
    }

    #[test]
    fn geometry_comes_from_the_first_pass() {
        let views = tta_views(400, 300, 5);
        let outs: Vec<Vec<f32>> = (0..5).map(|k| vec![k as f32 / 4.0; PARAMS.len()]).collect();
        let m = merge(&outs, &views);
        for name in ["straighten", "crop_on", "crop_x", "crop_y", "crop_w", "crop_h", "crop_ratio"] {
            assert_eq!(m[at(name)], 0.0, "{name}");
        }
    }

    #[test]
    fn tone_is_averaged() {
        let i = at("exposure");
        let views = tta_views(400, 300, 5);
        let outs: Vec<Vec<f32>> = (0..5)
            .map(|k| {
                let mut v = neutral();
                v[i] = from_value(i, k as f32 * 0.5);
                v
            })
            .collect();
        assert!((to_value(i, merge(&outs, &views)[i]) - 1.0).abs() < 1e-5);
    }

    #[test]
    fn mirror_is_undone_for_mask_x() {
        let i = at("rad_cx");
        let views = &tta_views(400, 300, 5)[..2];
        let (mut a, mut b) = (neutral(), neutral());
        a[i] = 0.3;
        b[i] = 0.7;
        assert!((merge(&[a, b], views)[i] - 0.3).abs() < 1e-5);
    }

    #[test]
    fn merged_mask_coordinates_stay_in_the_photo() {
        for (w, h) in [(400, 300), (300, 400), (300, 300)] {
            let views = tta_views(w, h, 5);
            for fill in [0.0, 1.0, f32::NAN, 5.0] {
                let outs = vec![vec![fill; PARAMS.len()]; 5];
                let m = merge(&outs, &views);
                for name in ["lin_fx", "lin_fy", "lin_tx", "lin_ty", "rad_cx", "rad_cy", "rad_rx", "rad_ry"] {
                    let x = to_value(at(name), m[at(name)]);
                    assert!((0.0..=1.0).contains(&x), "{name} = {x} for {fill} at {w}×{h}");
                }
            }
        }
    }

    #[test]
    fn view_linear_crops_and_mirrors_pixels() {
        use photos_media::develop::Linear;
        let base = Linear { width: 4, height: 2, data: (0..8).map(|i| [i as f32; 3]).collect() };
        let m = view_linear(&base, &View { mirror: true, ..IDENTITY });
        assert_eq!((m.width, m.height), (4, 2));
        assert_eq!(m.data.iter().map(|p| p[0]).collect::<Vec<_>>(), vec![3.0, 2.0, 1.0, 0.0, 7.0, 6.0, 5.0, 4.0]);
        assert_eq!(view_linear(&base, &IDENTITY).data, base.data);
        let wide = Linear { width: 20, height: 2, data: (0..40).map(|i| [i as f32; 3]).collect() };
        let c = view_linear(&wide, &View { x0: 0.1, sx: 0.9, ..IDENTITY });
        assert_eq!((c.width, c.height), (18, 2));
        assert_eq!(c.data[0][0], 2.0);
    }
}
