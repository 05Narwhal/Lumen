//! The MCP tools (Round 16 plan, *Tools*). Each takes an asset `id` from Develop's filmstrip
//! and answers terse JSON, or a JPEG from `view`. Kept free of the protocol: `mod.rs` turns
//! an `Output` into MCP content.

use std::sync::Arc;

use photos_core::edits::{Crop, Recipe};
use photos_core::Library;
use photos_media::develop::analyze::FaceBox;
use serde_json::{json, Map, Value};

use super::fit::RegionStats;
use super::render::{Compare, Jpeg};
use super::{Host, Shared};
use crate::error::{AppError, AppResult};

pub enum Output {
    Json(Value),
    /// A JPEG and a text part (a line about it, or `edit`'s JSON).
    Image(Jpeg, String),
    /// A plugin tool's text, as the plugin gave it (Round 23 A4).
    Text(String),
}

pub const PAGE: usize = 200;

pub fn bad(message: impl Into<String>) -> AppError {
    AppError { code: "invalid_arguments".into(), message: message.into(), detail: None }
}

/// The answer to a tool name nothing lists, built-in or plugin.
pub fn unknown(name: &str) -> AppError {
    AppError { code: "unknown_tool".into(), message: format!("There is no tool called {name}."), detail: None }
}

pub fn id_arg(args: &Map<String, Value>, key: &str) -> AppResult<i64> {
    args.get(key).and_then(Value::as_i64).ok_or_else(|| bad(format!("`{key}` must be a photo id from list_photos.")))
}

/// Every tool and its input schema, as `tools/list` shows them.
pub fn list() -> Vec<(&'static str, String, Value)> {
    let mut all: Vec<(&'static str, String, Value)> = read_tools().into_iter().map(|(n, d, v)| (n, d.to_string(), v)).collect();
    all.extend(super::write::tools());
    all
}

fn read_tools() -> Vec<(&'static str, &'static str, Value)> {
    let id = json!({ "type": "integer", "description": "A photo id from list_photos." });
    vec![
        (
            "list_photos",
            "The photos in Lumen Develop's filmstrip — the only photos you can see or edit. `open` marks the one on screen. 200 per page; pass `next` back as `cursor`.",
            json!({ "type": "object", "properties": { "cursor": { "type": "integer", "description": "`next` from the previous page." } } }),
        ),
        (
            "view",
            "Look at a photo with its current edit, as a JPEG. compare: \"after\" (default), \"before\" (unedited, same crop) or \"split\" (before left, after right). region {x,y,w,h} in 0–1 of the edited frame renders that part at 100 % to judge detail, noise and skin. overlay: a mask id paints where that mask lands in red at 50 % (stronger where it weighs more), to check a mask before trusting its adjustment.",
            json!({
                "type": "object",
                "properties": {
                    "id": id,
                    "size": { "type": "integer", "minimum": 256, "maximum": 1568, "default": 1024, "description": "Long edge in pixels." },
                    "compare": { "type": "string", "enum": ["after", "before", "split"], "default": "after" },
                    "region": {
                        "type": "object",
                        "properties": { "x": { "type": "number" }, "y": { "type": "number" }, "w": { "type": "number" }, "h": { "type": "number" } },
                        "required": ["x", "y", "w", "h"]
                    },
                    "overlay": { "type": "integer", "description": "A mask id from the mask tool's list: shows where it lands, in red." }
                },
                "required": ["id"]
            }),
        ),
        (
            "analyze",
            "Measurements of the photo as edited, for what pixels alone can't tell you: luma mean/p1/p50/p99 (0–1), zones (% of pixels: blacks, shadows, mids, lights, whites), clip (% clipped: shadow, highlight, r, g, b), cast (temp/tint slider corrections that would neutralise near-grey midtones; null when there are too few; unreliable when grass, foliage or sky fill the frame — trust your eyes there), sat (mean, p95), faces (box 0–1 of the edited frame, skin OKLCh hue in degrees — natural skin is ~20–50 — and luma), regions (only when Lumen Facet is on: what it found — sky, subject, background, person n — each with share % of the edited frame, mean luma and sat, and OKLCh hue in degrees or null when grey; each name is an AI mask target), plus the camera and whether it is RAW.",
            json!({ "type": "object", "properties": { "id": id }, "required": ["id"] }),
        ),
        (
            "suggest_looks",
            "Which looks suit this photo. Lumen scores every look in its catalogue 0–100 for this photo by colour theory (hue harmony, tonal key, skin, sky), from the unedited render, so the numbers do not move when you edit. `offer` is what to build: Natural first, then every look scoring 60 or more, at most 4. Each look has its `patch` (a merge-patch for `edit`, a starting point to adapt to this photo), its `intent` and the `reasons` for its score. Works with Lumen Facet off.",
            json!({ "type": "object", "properties": { "id": id }, "required": ["id"] }),
        ),
        (
            "get_recipe",
            "The photo's current Develop settings: only the fields that differ from the defaults (an empty object is an unedited photo), rounded to 2 decimals.",
            json!({ "type": "object", "properties": { "id": id }, "required": ["id"] }),
        ),
    ]
}

pub fn call(host: &dyn Host, shared: &Shared, name: &str, args: &Map<String, Value>) -> AppResult<Output> {
    if name == "list_photos" {
        return list_photos(host, shared, args.get("cursor").and_then(Value::as_i64));
    }
    if name == "apply_to" {
        return super::write::apply_to(host, shared, args);
    }
    let id = id_arg(args, "id")?;
    shared.scope.check(id)?;
    let lib = host.library()?;
    match name {
        "view" => view(host, shared, &lib, id, args),
        "analyze" => analyze(host, shared, &lib, id),
        "suggest_looks" => suggest_looks(host, shared, &lib, id),
        "get_recipe" => Ok(Output::Json(photos_core::develop_mcp::summary(&live_recipe(host, shared, &lib, id)?))),
        _ => super::write::call(host, shared, &lib, name, id, args).unwrap_or_else(|| Err(unknown(name))),
    }
}

/// What Develop opens the photo with: its draft, else the library's edit, else nothing.
pub fn recipe(lib: &Library, id: i64) -> AppResult<Recipe> {
    Ok(lib.db.with_conn(|c| photos_core::edits::develop_recipe(c, id))?.unwrap_or_default())
}

/// What the photo looks like right now: with Develop on screen the page is ahead of the
/// draft (its save is debounced), so ask it through an empty patch, which adds no history
/// step; else the draft.
pub fn live_recipe(host: &dyn Host, shared: &Shared, lib: &Library, id: i64) -> AppResult<Recipe> {
    if super::write::develop_on_screen(shared, id) {
        return shared.bridge.ask(id, "AI: read", super::bridge::Op::Patch { patch: json!({}) }, |r| host.emit(r), super::bridge::TIMEOUT)?.ok_or_else(super::bridge::busy);
    }
    recipe(lib, id)
}

fn list_photos(host: &dyn Host, shared: &Shared, cursor: Option<i64>) -> AppResult<Output> {
    let scope = shared.scope.get();
    let start = match cursor {
        Some(c) => scope.order.iter().position(|id| *id == c).map(|i| i + 1).unwrap_or(scope.order.len()),
        None => 0,
    };
    let page: Vec<i64> = scope.order.iter().skip(start).take(PAGE).copied().collect();
    if page.is_empty() {
        return Ok(Output::Json(json!({ "photos": [], "next": null })));
    }
    let lib = host.library()?;
    let ffmpeg = host.ffmpeg().is_some();
    let mut photos = Vec::with_capacity(page.len());
    lib.db.with_conn(|c| -> photos_core::Result<()> {
        for &id in &page {
            let a = photos_core::assets::get_asset(c, id, ffmpeg)?;
            let (w, h) = (a.width.unwrap_or(0), a.height.unwrap_or(0));
            let (w, h) = if photos_media::orientation_swaps(a.orientation as u16) { (h, w) } else { (w, h) };
            let edited = photos_core::edits::develop_recipe(c, id)?.is_some_and(|r| r.normalized() != Recipe::default().normalized());
            photos.push(json!({ "id": id, "name": a.filename, "w": w, "h": h, "raw": a.raw, "edited": edited, "open": scope.open == Some(id), "rating": a.rating, "label": super::write::label_name(a.color_label), "flag": super::write::flag_name(a.flag) }));
        }
        Ok(())
    })?;
    let next = (start + page.len() < scope.order.len()).then(|| page.last().copied()).flatten();
    Ok(Output::Json(json!({ "photos": photos, "next": next })))
}

fn view(host: &dyn Host, shared: &Shared, lib: &Arc<Library>, id: i64, args: &Map<String, Value>) -> AppResult<Output> {
    let size = args.get("size").and_then(Value::as_u64).unwrap_or(1024).clamp(256, 1568) as u32;
    let compare = match args.get("compare").and_then(Value::as_str).unwrap_or("after") {
        "after" => Compare::After,
        "before" => Compare::Before,
        "split" => Compare::Split,
        other => return Err(bad(format!("compare must be after, before or split, not {other}."))),
    };
    let region = match args.get("region") {
        None | Some(Value::Null) => None,
        Some(v) => Some(serde_json::from_value::<Crop>(v.clone()).map_err(|_| bad("region needs x, y, w, h in 0–1."))?),
    };
    let r = live_recipe(host, shared, lib, id)?;
    let overlay = match args.get("overlay") {
        None | Some(Value::Null) => None,
        Some(v) => {
            let masks = r.develop.as_ref().map(|d| d.masks.as_slice()).unwrap_or_default();
            let found = v.as_u64().and_then(|m| masks.iter().find(|k| u64::from(k.id) == m));
            let Some(mask) = found else {
                let ids: Vec<String> = masks.iter().map(|k| format!("{} ({})", k.id, k.name)).collect();
                return Err(bad(if ids.is_empty() { "overlay must be a mask id; this photo has no masks.".to_string() } else { format!("overlay must be a mask id: {}.", ids.join(", ")) }));
            };
            if region.is_some() {
                return Err(bad("overlay shows the whole frame; leave out region."));
            }
            Some((mask.id, mask.name.clone()))
        }
    };
    let jpeg = shared.renders.view(lib, host.ffmpeg().as_ref(), id, &r, size, compare, region, overlay.as_ref().map(|(m, _)| *m))?;
    let mode = match compare {
        Compare::After => "after",
        Compare::Before => "before",
        Compare::Split => "split: before left, after right",
    };
    let mut line = format!("{}×{}, {mode}{}", jpeg.width, jpeg.height, if region.is_some() { ", region at 100 %" } else { "" });
    if let Some((_, name)) = overlay {
        line.push_str(&format!(", mask {name} in red"));
    }
    Ok(Output::Image(jpeg, line))
}

/// "Z 6II · 50mm f/4 · 1/250 · ISO 400", from what the catalogue read.
fn exif_line(meta: Option<&Value>) -> String {
    let Some(m) = meta else { return String::new() };
    let mut parts = Vec::new();
    if let Some(model) = m.get("model").and_then(Value::as_str) {
        parts.push(model.to_string());
    }
    let lens: Vec<String> = [
        m.get("focal_length_mm").and_then(Value::as_f64).map(|f| format!("{}mm", (f * 10.0).round() / 10.0)),
        m.get("aperture").and_then(Value::as_f64).map(|a| format!("f/{}", (a * 10.0).round() / 10.0)),
    ]
    .into_iter()
    .flatten()
    .collect();
    if !lens.is_empty() {
        parts.push(lens.join(" "));
    }
    if let Some(e) = m.get("exposure").and_then(Value::as_str) {
        parts.push(e.to_string());
    }
    if let Some(iso) = m.get("iso").and_then(Value::as_u64) {
        parts.push(format!("ISO {iso}"));
    }
    parts.join(" · ")
}

/// What Facet found, or nothing: off, missing or failing is a state, not an error.
fn facet(host: &dyn Host, id: i64, tool: &str) -> Option<Vec<photos_core::ai_masks::AiMaskInfo>> {
    host.ai_masks(id).unwrap_or_else(|e| {
        tracing::warn!(id, error = %e.message, "MCP {tool}: no Facet regions");
        None
    })
}

fn face_boxes(lib: &Library, id: i64) -> AppResult<Vec<FaceBox>> {
    let faces = lib.db.with_conn(|c| photos_core::people::faces_for_asset(c, id))?;
    Ok(faces.iter().map(|f| FaceBox { x: f.x, y: f.y, w: f.w, h: f.h }).collect())
}

/// `suggest_looks`: every catalogue look scored for this photo, and the ones to build.
fn suggest_looks(host: &dyn Host, shared: &Shared, lib: &Arc<Library>, id: i64) -> AppResult<Output> {
    let ai = facet(host, id, "suggest_looks");
    let colour = shared.renders.colour(lib, host.ffmpeg().as_ref(), id, &live_recipe(host, shared, lib, id)?, &face_boxes(lib, id)?, ai.as_deref().unwrap_or_default())?;
    // The built-ins and every enabled plugin's looks (Round 23 B2), scored alike.
    let all = super::looks::with_plugins(host.plugin_looks());
    let mut fits: Vec<super::fit::Fit> = all.iter().map(|l| super::fit::score(l, &colour)).collect();
    let offer = super::fit::offer(&fits);
    fits.sort_by_key(|f| std::cmp::Reverse(f.score));
    let looks: Vec<Value> = fits
        .iter()
        .filter_map(|f| {
            let l = all.iter().find(|l| l.id == f.id)?;
            Some(json!({ "id": l.id, "name": l.name, "score": f.score, "offered": offer.contains(&l.id), "reasons": f.reasons, "intent": l.intent, "patch": l.patch }))
        })
        .collect();
    Ok(Output::Json(json!({ "threshold": super::fit::THRESHOLD, "offer": offer, "looks": looks })))
}

fn analyze(host: &dyn Host, shared: &Shared, lib: &Arc<Library>, id: i64) -> AppResult<Output> {
    let detail = lib.db.with_conn(|c| photos_core::assets::get_asset(c, id, false))?;
    let boxes = face_boxes(lib, id)?;
    let ai = facet(host, id, "analyze");
    let (a, w, h, regions) = shared.renders.analyze(lib, host.ffmpeg().as_ref(), id, &live_recipe(host, shared, lib, id)?, &boxes, ai.as_deref().unwrap_or_default())?;
    let mut out = json!({ "size": format!("{w}×{h}"), "exif": exif_line(detail.metadata.as_ref()), "raw": detail.raw });
    if let (Value::Object(o), Value::Object(m)) = (&mut out, serde_json::to_value(&a).unwrap_or_default()) {
        o.extend(m);
    }
    if ai.is_some() {
        out["regions"] = regions_json(&regions);
    }
    Ok(Output::Json(out))
}

fn round(v: f32, places: i32) -> f64 {
    let k = 10f64.powi(places);
    (f64::from(v) * k).round() / k
}

/// `analyze`'s regions: share in % of the edited frame, mean luma and saturation 0–1, hue in degrees.
fn regions_json(regions: &[(String, RegionStats)]) -> Value {
    regions
        .iter()
        .map(|(name, r)| json!({ "name": name, "share": round(r.share * 100.0, 1), "luma": round(r.luma, 2), "sat": round(r.sat, 2), "hue": r.hue.map(|h| h.round()) }))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_exif_line_reads_like_a_camera_back() {
        let m = json!({ "model": "Z 6II", "focal_length_mm": 50.0, "aperture": 4.0, "exposure": "1/250", "iso": 400 });
        assert_eq!(exif_line(Some(&m)), "Z 6II · 50mm f/4 · 1/250 · ISO 400");
        assert_eq!(exif_line(None), "");
    }

    #[test]
    fn every_tool_has_an_object_schema() {
        for (name, description, schema) in list() {
            assert_eq!(schema["type"], "object", "{name}");
            assert!(!description.is_empty());
        }
    }
}
