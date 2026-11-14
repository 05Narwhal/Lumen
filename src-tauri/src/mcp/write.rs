//! The MCP tools that change photos (Round 16 P2): drafts and history steps only, never
//! `edits` or originals; `mark` (Round 19) also sets rating, label and flag. The photo open in Develop, with Develop on screen, changes through
//! the page (`bridge`); any other filmstrip photo changes in its draft here.

use std::sync::Arc;

use photos_core::ai_masks::{AiMaskInfo, AiMaskKind};
use photos_core::develop::{merge_panels, Panel};
use photos_core::edits::{Develop, LocalAdjust, Mask, MaskShape, Recipe};
use photos_core::Library;
use photos_media::develop::masks::FrameMap;
use serde_json::{json, Map, Value};

use super::bridge::{self, Op};
use super::render::Compare;
use super::tools::{bad, id_arg, recipe, Output};
use super::{Host, Shared};
use crate::error::{AppError, AppResult};

pub fn tools() -> Vec<(&'static str, String, Value)> {
    use photos_core::develop_mcp::{OTHER_FIELDS, RANGES};
    let id = json!({ "type": "integer", "description": "A photo id from list_photos." });
    let fields: Vec<String> = RANGES.iter().map(|r| format!("{} {}…{}: {}", r.path, r.min, r.max, r.meaning)).collect();
    let panels = json!({ "type": "array", "items": { "type": "string", "enum": ["basic", "curve", "hsl", "grading", "detail", "effects", "masks"] } });
    let point = json!({ "type": "array", "items": { "type": "number" }, "minItems": 2, "maxItems": 2 });
    let adjust = json!({ "type": "object", "description": "A merge-patch of the mask's own tools, shaped like get_recipe's develop. Local Basic: temperature, tint (±100), exposure (±5 EV), contrast, highlights, shadows, whites, blacks, texture, clarity, dehaze, vibrance, saturation, sharpness (±1). Exposure +1 is one stop brighter; clarity or texture ±0.3 is clearly visible, ±0.6 strong. Also as for the photo: curve {master|r|g|b: [[x,y],…]}, hsl {red…magenta: {h,s,l}}, grading {shadows|midtones|highlights: {hue,sat,lum}, blending, balance}, sharpen {radius 0.5–3, detail, masking} (the mask's own; its amount is sharpness), grain {amount,size}, glow {amount,size}, halation {amount,size}, motion_blur {amount,angle}. A mask earns its place when its change shows in a split view." });
    vec![
        (
            "edit",
            format!(
                "Change Develop settings with a JSON merge-patch shaped like get_recipe's output: fields you leave out keep their values, every value is clamped to its slider. One edit is one history step the user can undo. The curve and grading are where looks are made; basic sliders alone only correct. Returns the new summary, luma_mean and clip; render:true adds a 1024 px JPEG. Fields: {}; {OTHER_FIELDS}. Masks are the mask tool's.",
                fields.join("; ")
            ),
            json!({
                "type": "object",
                "properties": {
                    "id": id,
                    "patch": { "type": "object", "description": "e.g. {\"develop\":{\"basic\":{\"exposure\":0.3,\"shadows\":0.2}}}" },
                    "label": { "type": "string", "description": "A few words for the history step, e.g. \"warmer, lift shadows\"." },
                    "render": { "type": "boolean", "default": false }
                },
                "required": ["id", "patch", "label"]
            }),
        ),
        (
            "mask",
            "Add, update or remove a local adjustment. Shapes are in 0–1 of the edited frame (what view shows; analyze.faces boxes are in the same space): linear {kind:\"linear\", from:[x,y] (full effect), to:[x,y] (none)} for skies and ground; radial {kind:\"radial\", center:[x,y], radius:[rx,ry] (shares of the frame's width and height), angle (degrees, default 0), feather (0–1, default 0.5), inside (default true)} for faces and subjects; ai {kind:\"ai\", target:\"sky\"|\"subject\"|\"background\"|\"person\", instance (which person, from 0)} for what Lumen Facet found on the photo, edge-exact and following crops (no_model unless the user has AI masks switched on; the first one on a photo takes a few seconds). add:false subtracts a shape. Brushes are not offered. Returns every mask.".into(),
            json!({
                "type": "object",
                "properties": {
                    "id": id,
                    "op": { "type": "string", "enum": ["add", "update", "remove"], "default": "add" },
                    "mask_id": { "type": "integer", "description": "For update and remove." },
                    "name": { "type": "string" },
                    "shapes": { "type": "array", "items": { "type": "object", "properties": { "kind": { "type": "string", "enum": ["linear", "radial", "ai"] }, "add": { "type": "boolean" }, "from": point, "to": point, "center": point, "radius": point, "angle": { "type": "number" }, "feather": { "type": "number" }, "inside": { "type": "boolean" }, "target": { "type": "string", "enum": ["sky", "subject", "background", "person"] }, "instance": { "type": "integer", "minimum": 0 } }, "required": ["kind"] } },
                    "invert": { "type": "boolean" },
                    "opacity": { "type": "number", "minimum": 0, "maximum": 1 },
                    "adjust": adjust
                },
                "required": ["id"]
            }),
        ),
        (
            "auto",
            "A starting point. model \"standard\" is Develop's Auto button; \"ray\" (Lumen Ray, Experimental Auto) and \"prism\" (Lumen Prism, Experimental Retouch, which also crops and straightens) answer no_model unless the user has them switched on. One history step.".into(),
            json!({ "type": "object", "properties": { "id": id, "model": { "type": "string", "enum": ["standard", "ray", "prism"], "default": "standard" } }, "required": ["id"] }),
        ),
        (
            "undo",
            "Undo your newest step on the photo. Refuses with last_step_not_ai when the newest history step is the user's.".into(),
            json!({ "type": "object", "properties": { "id": id }, "required": ["id"] }),
        ),
        (
            "reset",
            "Put panels back to no edit (all of them when panels is left out). Crop, turns and straighten stay; remove a crop with edit {\"crop\":null}.".into(),
            json!({ "type": "object", "properties": { "id": id, "panels": panels }, "required": ["id"] }),
        ),
        (
            "apply_to",
            "Copy panels of one photo's settings onto other filmstrip photos (each keeps its own crop). Only when the user asked to match a set. Returns how many changed.".into(),
            json!({ "type": "object", "properties": { "from": id, "ids": { "type": "array", "items": { "type": "integer" } }, "panels": panels }, "required": ["from", "ids", "panels"] }),
        ),
        (
            "draft_variant",
            "Keep the photo's current edit as a look the user can flip to: a Develop snapshot named \"AI: <name> · <score>\". op \"save\" (default) stores what the photo looks like now; build the next look after a `reset`. op \"use\" puts the saved look called <name> back on the photo (one history step); do it last, with the top-scoring look. Returns the names of your snapshots on this photo.".into(),
            json!({
                "type": "object",
                "properties": {
                    "id": id,
                    "name": { "type": "string", "description": "The look's name, e.g. \"Golden film\"." },
                    "score": { "type": "integer", "minimum": 0, "maximum": 100, "description": "Its suggest_looks score, shown in the snapshot's name." },
                    "op": { "type": "string", "enum": ["save", "use"], "default": "save" }
                },
                "required": ["id", "name"]
            }),
        ),
        (
            "mark",
            "Set a photo's star rating, colour label or flag. Only when the user asked you to rate, label, flag or cull photos; never as part of editing. Fields you leave out keep their value. list_photos shows the current marks. Returns the photo's marks.".into(),
            json!({
                "type": "object",
                "properties": {
                    "id": id,
                    "rating": { "type": "integer", "minimum": 0, "maximum": 5, "description": "0 clears the stars." },
                    "label": { "type": "string", "enum": ["none", "red", "yellow", "green", "blue", "purple"] },
                    "flag": { "type": "string", "enum": ["none", "pick", "reject"] }
                },
                "required": ["id"]
            }),
        ),
        (
            "open_photo",
            "Show the photo in Develop so the user can watch you work on it. Needs Develop on screen.".into(),
            json!({ "type": "object", "properties": { "id": id }, "required": ["id"] }),
        ),
    ]
}

/// A write tool on photo `id` (already checked to be in the filmstrip); `None` for a name
/// that is not one.
pub fn call(host: &dyn Host, shared: &Shared, lib: &Arc<Library>, name: &str, id: i64, args: &Map<String, Value>) -> Option<AppResult<Output>> {
    let summary = |r: AppResult<Recipe>| r.map(|r| Output::Json(photos_core::develop_mcp::summary(&r)));
    Some(match name {
        "edit" => edit(host, shared, lib, id, args),
        "mask" => {
            let name = args.get("name").and_then(Value::as_str).unwrap_or("mask");
            write(host, shared, lib, id, &format!("AI: mask {name}"), Op::Mask { args: Value::Object(args.clone()) }).map(|r| Output::Json(masks_summary(&r)))
        }
        "auto" => {
            let model = args.get("model").and_then(Value::as_str).unwrap_or("standard").to_string();
            summary(write(host, shared, lib, id, &format!("AI: Auto ({model})"), Op::Auto { model }))
        }
        "undo" => summary(write(host, shared, lib, id, "", Op::Undo)),
        "reset" => panels_arg(args, "panels").and_then(|panels| summary(write(host, shared, lib, id, "AI: Reset", Op::Reset { panels }))),
        "mark" => mark(host, lib, id, args),
        "open_photo" => open_photo(host, shared, id),
        "draft_variant" => draft_variant(host, shared, lib, id, args),
        _ => return None,
    })
}

/// The photo's AI snapshots, oldest first.
fn ai_snapshots(lib: &Library, id: i64) -> AppResult<Vec<(String, Recipe)>> {
    let all = lib.db.with_conn(|c| photos_core::develop_history::list_history(c, id))?;
    Ok(all.into_iter().filter_map(|s| Some((s.snapshot_name.filter(|n| n.starts_with("AI: "))?, s.recipe))).collect())
}

/// `draft_variant`: keep the photo's current look as a snapshot, or put a kept one back.
fn draft_variant(host: &dyn Host, shared: &Shared, lib: &Arc<Library>, id: i64, args: &Map<String, Value>) -> AppResult<Output> {
    let name = args.get("name").and_then(Value::as_str).map(str::trim).filter(|n| !n.is_empty()).ok_or_else(|| bad("name must be the look's name."))?;
    match args.get("op").and_then(Value::as_str).unwrap_or("save") {
        "save" => {
            // The live recipe: through the page when the photo is open on screen, so what the
            // user sees is what is kept, not the draft the page has not written yet.
            let live = if develop_on_screen(shared, id) { write(host, shared, lib, id, &format!("AI: keep {name}"), Op::Patch { patch: json!({}) })? } else { recipe(lib, id)? };
            let label = match args.get("score").and_then(Value::as_u64) {
                Some(s) => format!("AI: {name} · {}", s.min(100)),
                None => format!("AI: {name}"),
            };
            lib.db.with_conn(|c| photos_core::develop_history::save_snapshot(c, id, &label, &live, now_ms()))?;
            host.changed();
        }
        "use" => {
            let kept = ai_snapshots(lib, id)?;
            let wanted = format!("AI: {name}");
            let hit = kept.iter().rev().find(|(n, _)| *n == wanted || n.starts_with(&format!("{wanted} · ")));
            let Some((_, saved)) = hit else {
                let names: Vec<&str> = kept.iter().map(|(n, _)| n.trim_start_matches("AI: ")).collect();
                return Err(bad(if names.is_empty() { "You have kept no look on this photo yet; save one first.".to_string() } else { format!("No kept look is called {name}. Kept: {}.", names.join(", ")) }));
            };
            let develop = Box::new(saved.develop.clone().unwrap_or_default());
            write(host, shared, lib, id, &format!("AI: use {name}"), Op::Paste { develop, panels: Panel::ALL.to_vec() })?;
        }
        other => return Err(bad(format!("op must be save or use, not {other}."))),
    }
    let names: Vec<String> = ai_snapshots(lib, id)?.into_iter().map(|(n, _)| n).collect();
    Ok(Output::Json(json!({ "snapshots": names })))
}

fn panels_arg(args: &Map<String, Value>, key: &str) -> AppResult<Vec<Panel>> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(v) => serde_json::from_value(v.clone()).map_err(|_| bad(format!("{key} must list panels: basic, curve, hsl, grading, detail, effects, masks."))),
    }
}

fn invalid_patch(message: impl Into<String>) -> AppError {
    AppError { code: "invalid_patch".into(), message: message.into(), detail: None }
}

pub fn no_model(model: &str) -> AppError {
    AppError {
        code: "no_model".into(),
        message: format!("The {model} model is not installed or not switched on (Settings → Experimental). Use model \"standard\"."),
        detail: None,
    }
}

pub(super) fn develop_on_screen(shared: &Shared, id: i64) -> bool {
    let s = shared.scope.get();
    s.develop_mounted && s.open == Some(id)
}

/// Change a photo: through the page when it is open in Develop on screen, else in its draft
/// with a history step. Returns the recipe it now has.
fn write(host: &dyn Host, shared: &Shared, lib: &Library, id: i64, label: &str, op: Op) -> AppResult<Recipe> {
    if develop_on_screen(shared, id) {
        return shared.bridge.ask(id, label, op, |r| host.emit(r), bridge::TIMEOUT)?.ok_or_else(bridge::busy);
    }
    if op == Op::Undo {
        return undo_draft(host, lib, id);
    }
    let next = transform(host, lib, id, &recipe(lib, id)?, &op)?;
    save(lib, id, &next, label)?;
    host.changed();
    Ok(next)
}

fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as i64)
}

fn save(lib: &Library, id: i64, r: &Recipe, label: &str) -> AppResult<()> {
    lib.db.with_conn(|c| -> photos_core::Result<()> {
        let tx = c.unchecked_transaction()?;
        photos_core::edits::save_draft(&tx, id, r, now_ms())?;
        photos_core::develop_history::push_step(&tx, id, r, label, now_ms())?;
        tx.commit()?;
        Ok(())
    })?;
    Ok(())
}

/// `undo` on the draft: back to the step before the newest, which must be an `AI:` one. With
/// no step before it the draft goes, so Develop opens the photo as the library has it.
fn undo_draft(host: &dyn Host, lib: &Library, id: i64) -> AppResult<Recipe> {
    let back = lib.db.with_conn(|c| -> photos_core::Result<Option<Option<Recipe>>> {
        let steps: Vec<_> = photos_core::develop_history::list_history(c, id)?.into_iter().filter(|s| s.snapshot_name.is_none()).collect();
        let Some(top) = steps.last().filter(|s| s.label.starts_with("AI:")) else { return Ok(None) };
        let prev = steps.len().checked_sub(2).map(|i| &steps[i]);
        let tx = c.unchecked_transaction()?;
        match prev {
            Some(p) => {
                photos_core::edits::save_draft(&tx, id, &p.recipe, now_ms())?;
                photos_core::develop_history::truncate_after(&tx, id, p.id)?;
            }
            None => {
                photos_core::edits::delete_draft(&tx, id)?;
                photos_core::develop_history::truncate_after(&tx, id, top.id - 1)?;
            }
        }
        tx.commit()?;
        Ok(Some(prev.map(|p| p.recipe.clone())))
    })?;
    let Some(back) = back else {
        return Err(AppError { code: "last_step_not_ai".into(), message: "The newest step on this photo is the user's; only your own steps can be undone.".into(), detail: None });
    };
    host.changed();
    match back {
        Some(r) => Ok(r),
        None => recipe(lib, id),
    }
}

/// `op` on `current`: what both write paths do (the page calls it through `mcp_transform`).
pub fn transform(host: &dyn Host, lib: &Library, id: i64, current: &Recipe, op: &Op) -> AppResult<Recipe> {
    let develop = current.develop.clone().unwrap_or_default();
    let with = |d: Develop| Recipe { develop: Some(d), ..current.clone() }.normalized();
    match op {
        Op::Patch { patch } => photos_core::develop_mcp::patch(current, patch).map_err(|e| invalid_patch(e.0)),
        Op::Mask { args } => mask_op(host, lib, id, current, args.as_object().ok_or_else(|| bad("mask arguments must be an object."))?),
        Op::Auto { model } => host.auto(id, current, model),
        Op::Reset { panels } => {
            let panels: &[Panel] = if panels.is_empty() { &Panel::ALL } else { panels };
            Ok(with(merge_panels(&develop, &Develop::default(), panels)))
        }
        Op::Paste { develop: from, panels } => Ok(with(merge_panels(&develop, from, panels))),
        Op::Undo | Op::Open => Err(bad("undo and open are not changes to a recipe.")),
    }
}

/// The photo's frame map and frame size, from its display-oriented size.
fn frame_of(lib: &Library, id: i64, r: &Recipe) -> AppResult<(FrameMap, [f32; 2])> {
    let a = lib.db.with_conn(|c| photos_core::assets::get_asset(c, id, false))?;
    let (w, h) = (a.width.unwrap_or(0).max(1) as u32, a.height.unwrap_or(0).max(1) as u32);
    let (w, h) = if photos_media::orientation_swaps(a.orientation as u16) { (h, w) } else { (w, h) };
    Ok((FrameMap::new(w, h, r), FrameMap::frame_size(w, h, r)))
}

fn pair(v: Option<&Value>, what: &str) -> AppResult<[f32; 2]> {
    let a = v.and_then(Value::as_array).filter(|a| a.len() == 2).ok_or_else(|| bad(format!("{what} must be [x, y].")))?;
    match (a[0].as_f64(), a[1].as_f64()) {
        (Some(x), Some(y)) => Ok([x as f32, y as f32]),
        _ => Err(bad(format!("{what} must be two numbers."))),
    }
}

/// A shape given in the edited frame (0–1), as a mask stores it: in the unedited photo.
fn shape_in_photo(map: &FrameMap, [fw, fh]: [f32; 2], s: &Value) -> AppResult<MaskShape> {
    let add = s.get("add").and_then(Value::as_bool).unwrap_or(true);
    let back = |[x, y]: [f32; 2]| map.from_frame(x * fw, y * fh);
    match s.get("kind").and_then(Value::as_str) {
        Some("linear") => Ok(MaskShape::Linear { add, from: back(pair(s.get("from"), "from")?), to: back(pair(s.get("to"), "to")?) }),
        Some("radial") => {
            let [rx, ry] = pair(s.get("radius"), "radius")?;
            let angle = s.get("angle").and_then(Value::as_f64).unwrap_or(0.0) as f32;
            let (sin, cos) = angle.to_radians().sin_cos();
            // The ellipse's axes in frame pixels, taken back into the photo's pixels.
            let a = map.vector_back(rx * fw * cos, rx * fw * sin);
            let b = map.vector_back(-ry * fh * sin, ry * fh * cos);
            Ok(MaskShape::Radial {
                add,
                center: back(pair(s.get("center"), "center")?),
                radius: [a[0].hypot(a[1]) / map.base[0], b[0].hypot(b[1]) / map.base[1]],
                angle: a[1].atan2(a[0]).to_degrees(),
                feather: s.get("feather").and_then(Value::as_f64).unwrap_or(0.5) as f32,
                inside: s.get("inside").and_then(Value::as_bool).unwrap_or(true),
            })
        }
        _ => Err(bad("each shape needs kind \"linear\", \"radial\" or \"ai\" (brushes are not offered).")),
    }
}

/// An `ai` shape: the raster Facet found for `target` / `instance` on this photo. It lives in
/// the unedited photo like every shape, so no frame map is needed and crops never move it.
fn ai_shape(found: &[AiMaskInfo], s: &Value) -> AppResult<MaskShape> {
    let add = s.get("add").and_then(Value::as_bool).unwrap_or(true);
    let target: AiMaskKind = s
        .get("target")
        .and_then(|t| serde_json::from_value(t.clone()).ok())
        .ok_or_else(|| bad("an ai shape needs target \"sky\", \"subject\", \"background\" or \"person\"."))?;
    let instance = s.get("instance").and_then(Value::as_u64).unwrap_or(0) as u32;
    let hit = found.iter().find(|m| m.kind == target && m.instance == instance);
    let Some(m) = hit else {
        let names: Vec<String> = found.iter().map(ai_name).collect();
        let what = if names.is_empty() { "nothing".to_string() } else { names.join(", ") };
        return Err(bad(format!("Lumen Facet found no {} on this photo. It found: {what}.", ai_name(&AiMaskInfo { instance, ..blank(target) }))));
    };
    Ok(MaskShape::Ai { add, target, instance, raster: m.hash.clone() })
}

fn blank(kind: AiMaskKind) -> AiMaskInfo {
    AiMaskInfo { hash: String::new(), kind, instance: 0, model: String::new(), score: 0.0, width: 0, height: 0 }
}

/// "sky", "person 2": how the tool names a found mask.
fn ai_name(m: &AiMaskInfo) -> String {
    match m.kind {
        AiMaskKind::Person => format!("person {}", m.instance),
        k => k.as_str().to_string(),
    }
}

fn mask_op(host: &dyn Host, lib: &Library, id: i64, current: &Recipe, args: &Map<String, Value>) -> AppResult<Recipe> {
    let mut develop = current.develop.clone().unwrap_or_default();
    let op = args.get("op").and_then(Value::as_str).unwrap_or("add");
    let mask_id = args.get("mask_id").and_then(Value::as_u64).map(|m| m as u32);
    let find = |masks: &[Mask]| mask_id.and_then(|m| masks.iter().position(|k| k.id == m)).ok_or_else(|| bad("mask_id must be one of the photo's masks (see the mask tool's result)."));
    let i = match op {
        "remove" => {
            let i = find(&develop.masks)?;
            develop.masks.remove(i);
            return Ok(Recipe { develop: Some(develop), ..current.clone() }.normalized());
        }
        "add" => {
            let next = develop.masks.iter().map(|m| m.id).max().unwrap_or(0) + 1;
            develop.masks.push(Mask { id: next, name: format!("AI mask {next}"), shapes: Vec::new(), invert: false, opacity: 1.0, adjust: LocalAdjust::default(), visible: true });
            develop.masks.len() - 1
        }
        "update" => find(&develop.masks)?,
        other => return Err(bad(format!("op must be add, update or remove, not {other}."))),
    };
    let m = &mut develop.masks[i];
    if let Some(name) = args.get("name").and_then(Value::as_str) {
        m.name = name.to_string();
    }
    if let Some(shapes) = args.get("shapes") {
        let (map, size) = frame_of(lib, id, current)?;
        let list = shapes.as_array().ok_or_else(|| bad("shapes must be a list."))?;
        let is_ai = |s: &Value| s.get("kind").and_then(Value::as_str) == Some("ai");
        // Facet runs (once per photo) only when a shape asks for it.
        let found = if list.iter().any(is_ai) { Some(host.ai_masks(id)?.ok_or_else(|| no_model("Lumen Facet (AI masks)"))?) } else { None };
        m.shapes = list.iter().map(|s| if is_ai(s) { ai_shape(found.as_deref().unwrap_or_default(), s) } else { shape_in_photo(&map, size, s) }).collect::<AppResult<_>>()?;
    }
    if let Some(v) = args.get("invert").and_then(Value::as_bool) {
        m.invert = v;
    }
    if let Some(v) = args.get("opacity").and_then(Value::as_f64) {
        m.opacity = v as f32;
    }
    if let Some(adjust) = args.get("adjust") {
        m.adjust = photos_core::develop_mcp::patch_local(&m.adjust, adjust).map_err(|e| invalid_patch(e.0))?;
    }
    if m.shapes.is_empty() {
        return Err(bad("a mask needs at least one shape."));
    }
    Ok(Recipe { develop: Some(develop), ..current.clone() }.normalized())
}

/// Every mask, briefly: what `mask` answers.
fn masks_summary(r: &Recipe) -> Value {
    let masks = r.develop.as_ref().map(|d| d.masks.as_slice()).unwrap_or_default();
    let list: Vec<Value> = masks
        .iter()
        .map(|m| {
            let adjust = photos_core::develop_mcp::local_summary(&m.adjust);
            let mut out = json!({ "mask_id": m.id, "name": m.name, "shapes": m.shapes.len(), "invert": m.invert, "opacity": m.opacity, "adjust": adjust });
            let ai: Vec<String> = m
                .shapes
                .iter()
                .filter_map(|s| match s {
                    MaskShape::Ai { add, target, instance, .. } => Some(format!("{}{}", if *add { "" } else { "minus " }, ai_name(&AiMaskInfo { instance: *instance, ..blank(*target) }))),
                    _ => None,
                })
                .collect();
            if !ai.is_empty() {
                out["ai"] = json!(ai);
            }
            out
        })
        .collect();
    json!({ "masks": list })
}

/// `edit`'s change: `patch` (an object shaped like get_recipe's output) onto photo `id` (already
/// checked to be in the filmstrip) as one history step `label`. Plugin drafts (Round 23 A4) come
/// through here too, so they meet the same refusals.
pub(crate) fn patch_draft(host: &dyn Host, shared: &Shared, lib: &Library, id: i64, patch: Option<&Value>, label: &str) -> AppResult<Recipe> {
    let patch = patch.filter(|p| p.is_object()).ok_or_else(|| invalid_patch("patch must be an object shaped like get_recipe's output."))?;
    write(host, shared, lib, id, label, Op::Patch { patch: patch.clone() })
}

fn edit(host: &dyn Host, shared: &Shared, lib: &Arc<Library>, id: i64, args: &Map<String, Value>) -> AppResult<Output> {
    let label = args.get("label").and_then(Value::as_str).map(str::trim).filter(|l| !l.is_empty()).unwrap_or("edit");
    let r = patch_draft(host, shared, lib, id, args.get("patch"), &format!("AI: {label}"))?;
    let (a, _, _, _) = shared.renders.analyze(lib, host.ffmpeg().as_ref(), id, &r, &[], &[])?;
    let a = serde_json::to_value(&a).unwrap_or_default();
    let out = json!({ "recipe": photos_core::develop_mcp::summary(&r), "luma_mean": a["luma"]["mean"], "clip": a["clip"] });
    if args.get("render").and_then(Value::as_bool).unwrap_or(false) {
        let jpeg = shared.renders.view(lib, host.ffmpeg().as_ref(), id, &r, 1024, Compare::After, None, None)?;
        return Ok(Output::Image(jpeg, out.to_string()));
    }
    Ok(Output::Json(out))
}

/// `apply_to`: its photos are checked here, as it has no single `id`.
pub fn apply_to(host: &dyn Host, shared: &Shared, args: &Map<String, Value>) -> AppResult<Output> {
    let from = id_arg(args, "from")?;
    shared.scope.check(from)?;
    let ids: Vec<i64> = args.get("ids").and_then(Value::as_array).ok_or_else(|| bad("ids must be a list of photo ids."))?.iter().filter_map(Value::as_i64).filter(|i| *i != from).collect();
    for &i in &ids {
        shared.scope.check(i)?;
    }
    let panels = panels_arg(args, "panels")?;
    if panels.is_empty() {
        return Err(bad("panels must name at least one panel."));
    }
    let lib = host.library()?;
    let develop = recipe(&lib, from)?.develop.unwrap_or_default();
    let label = format!("AI: Match {}", lib.db.with_conn(|c| photos_core::assets::get_asset(c, from, false))?.filename);
    let (live, stored): (Vec<i64>, Vec<i64>) = ids.into_iter().partition(|i| develop_on_screen(shared, *i));
    let mut count = lib.db.with_conn(|c| photos_core::develop::paste_develop(c, &stored, &develop, &panels, &label, now_ms()))?.len();
    if count > 0 {
        host.changed();
    }
    for i in live {
        write(host, shared, &lib, i, &label, Op::Paste { develop: Box::new(develop.clone()), panels: panels.clone() })?;
        count += 1;
    }
    Ok(Output::Json(json!({ "changed": count })))
}

fn open_photo(host: &dyn Host, shared: &Shared, id: i64) -> AppResult<Output> {
    if !shared.scope.get().develop_mounted {
        return Err(AppError { code: "develop_busy".into(), message: "Develop is not on screen; ask the user to open it.".into(), detail: None });
    }
    shared.bridge.ask(id, "", Op::Open, |r| host.emit(r), bridge::TIMEOUT)?;
    Ok(Output::Json(json!({ "ok": true })))
}

/// The recipe after *Auto*'s answer, as Develop's own button applies it.
pub fn with_auto(r: &Recipe, a: &crate::develop_auto::AutoResult) -> Recipe {
    let d = r.develop.clone().unwrap_or_default();
    let d = match a.look.as_ref().filter(|_| a.source == "model") {
        Some(m) => Develop { auto: true, ..m.onto(a.basic, d) },
        None => Develop { basic: a.basic, auto: true, ..d },
    };
    Recipe { develop: Some(d), ..r.clone() }.normalized()
}

/// The recipe after *Retouch*'s answer: its develop, crop and straighten.
pub fn with_retouch(r: &Recipe, a: &crate::develop_retouch::RetouchResult) -> Recipe {
    Recipe { develop: Some(a.develop.clone()), crop: a.crop, straighten: a.straighten, ..r.clone() }.normalized()
}

const LABEL_KEYS: [&str; 6] = ["none", "red", "yellow", "green", "blue", "purple"];

pub fn label_name(label: i64) -> &'static str {
    LABEL_KEYS.get(label.max(0) as usize).copied().unwrap_or("none")
}

pub fn flag_name(flag: i64) -> &'static str {
    match flag {
        photos_core::labels::FLAG_PICK => "pick",
        photos_core::labels::FLAG_REJECT => "reject",
        _ => "none",
    }
}

/// `mark`: rating, colour label and flag of a filmstrip photo, through the same core setters
/// as the UI's keys, then the same change notice. Everything is checked before anything is written.
fn mark(host: &dyn Host, lib: &Arc<Library>, id: i64, args: &Map<String, Value>) -> AppResult<Output> {
    let bad = |m: String| AppError { code: "bad_arguments".into(), message: m, detail: None };
    let rating = match args.get("rating") {
        None | Some(Value::Null) => None,
        Some(v) => match v.as_i64() {
            Some(n @ 0..=5) => Some(n),
            _ => return Err(bad("rating must be a whole number from 0 to 5.".into())),
        },
    };
    let label = match args.get("label").and_then(Value::as_str) {
        None => None,
        Some(l) => Some(LABEL_KEYS.iter().position(|k| *k == l).ok_or_else(|| bad("label must be none, red, yellow, green, blue or purple.".into()))? as i64),
    };
    let flag = match args.get("flag").and_then(Value::as_str) {
        None => None,
        Some("none") => Some(photos_core::labels::FLAG_NONE),
        Some("pick") => Some(photos_core::labels::FLAG_PICK),
        Some("reject") => Some(photos_core::labels::FLAG_REJECT),
        Some(_) => return Err(bad("flag must be none, pick or reject.".into())),
    };
    let a = lib.db.with_conn(|c| -> photos_core::Result<_> {
        if let Some(r) = rating {
            photos_core::assets::set_rating(c, &[id], r)?;
        }
        if let Some(l) = label {
            photos_core::labels::set_color_label(c, &[id], l)?;
        }
        if let Some(f) = flag {
            photos_core::labels::set_flag(c, &[id], f)?;
        }
        photos_core::assets::get_asset(c, id, false)
    })?;
    host.changed();
    Ok(Output::Json(json!({ "id": id, "rating": a.rating, "label": label_name(a.color_label), "flag": flag_name(a.flag) })))
}
