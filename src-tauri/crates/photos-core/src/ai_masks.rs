//! AI masks' rasters (Phase 8f): what Lumen Facet found on a photo, stored so a render never
//! needs the model. A recipe's `ai` shape names its raster by hash, so a new detection marks the
//! old rows as no longer current instead of deleting them: a recipe that uses one still renders.

use std::collections::HashMap;
use std::io::{Read, Write};

use flate2::{read::DeflateDecoder, write::DeflateEncoder, Compression};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::edits::Recipe;
use crate::error::{CoreError, Result};

/// What an AI mask selects.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export)]
pub enum AiMaskKind {
    Sky,
    Subject,
    Background,
    Person,
}

impl AiMaskKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Sky => "sky",
            Self::Subject => "subject",
            Self::Background => "background",
            Self::Person => "person",
        }
    }

    fn parse(s: &str) -> Option<Self> {
        match s {
            "sky" => Some(Self::Sky),
            "subject" => Some(Self::Subject),
            "background" => Some(Self::Background),
            "person" => Some(Self::Person),
            _ => None,
        }
    }
}

/// A raster as the model gave it: `width` × `height` 8-bit weights.
#[derive(Debug, Clone, PartialEq)]
pub struct NewAiMask {
    pub kind: AiMaskKind,
    /// Which person, from 0; 0 for the other kinds.
    pub instance: u32,
    pub score: f32,
    pub width: u32,
    pub height: u32,
    pub weights: Vec<u8>,
}

/// A stored raster without its bytes: what the panel lists.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[ts(export)]
pub struct AiMaskInfo {
    pub hash: String,
    pub kind: AiMaskKind,
    pub instance: u32,
    pub model: String,
    pub score: f32,
    pub width: u32,
    pub height: u32,
}

/// A stored raster with its bytes, inflated.
#[derive(Debug, Clone, PartialEq)]
pub struct AiMaskRaster {
    pub kind: AiMaskKind,
    pub instance: u32,
    pub width: u32,
    pub height: u32,
    pub weights: Vec<u8>,
}

/// Names a raster within its photo. Kind and instance are part of it: one person alone in a
/// photo can have the same weights as Subject, and each needs its own row.
pub fn hash_of(mask: &NewAiMask) -> String {
    let mut h = blake3::Hasher::new();
    h.update(mask.kind.as_str().as_bytes());
    h.update(&mask.instance.to_le_bytes());
    h.update(&mask.width.to_le_bytes());
    h.update(&mask.height.to_le_bytes());
    h.update(&mask.weights);
    h.finalize().to_hex()[..32].to_string()
}

fn deflate(bytes: &[u8]) -> Result<Vec<u8>> {
    let mut e = DeflateEncoder::new(Vec::with_capacity(bytes.len() / 8), Compression::default());
    e.write_all(bytes).map_err(CoreError::Io)?;
    e.finish().map_err(CoreError::Io)
}

/// Stores what one run of `model` found on a photo, in one transaction, and returns the hashes
/// in the order given. This detection becomes the photo's current one, so a mask it no longer
/// finds stops being offered; its row stays for the recipes that name it. Running the same model
/// again on the same pixels changes nothing.
pub fn put_detection(conn: &Connection, asset: i64, model: &str, masks: &[NewAiMask]) -> Result<Vec<String>> {
    for m in masks {
        if m.weights.len() != m.width as usize * m.height as usize {
            return Err(CoreError::Invalid(format!("a {}×{} mask has {} weights", m.width, m.height, m.weights.len())));
        }
    }
    // Compressed before the write lock is taken: a photo's masks take milliseconds.
    let packed: Vec<(String, Vec<u8>)> = masks.iter().map(|m| Ok((hash_of(m), deflate(&m.weights)?))).collect::<Result<_>>()?;
    let tx = conn.unchecked_transaction()?;
    tx.execute("UPDATE ai_masks SET current = 0 WHERE asset_id = ?1 AND current = 1", [asset])?;
    {
        let mut put = tx.prepare_cached(
            "INSERT INTO ai_masks (asset_id, hash, kind, instance, model, score, width, height, weights, current)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 1)
             ON CONFLICT (asset_id, hash) DO UPDATE SET model = excluded.model, score = excluded.score, current = 1",
        )?;
        for (m, (hash, weights)) in masks.iter().zip(&packed) {
            put.execute(params![asset, hash, m.kind.as_str(), m.instance, model, m.score as f64, m.width, m.height, weights])?;
        }
    }
    tx.execute(
        "INSERT INTO ai_mask_scan (asset_id, model) VALUES (?1, ?2)
         ON CONFLICT (asset_id) DO UPDATE SET model = excluded.model",
        params![asset, model],
    )?;
    tx.commit()?;
    Ok(packed.into_iter().map(|(hash, _)| hash).collect())
}

/// The model that last looked at a photo; `None` if none has, however many masks it found.
pub fn scanned_by(conn: &Connection, asset: i64) -> Result<Option<String>> {
    Ok(conn.query_row("SELECT model FROM ai_mask_scan WHERE asset_id = ?1", [asset], |r| r.get(0)).optional()?)
}

/// The photo's current masks, in the order the panel offers them: Sky, Subject, Background,
/// then each person. Reads no weights. A row of a kind this Lumen doesn't know is left out.
pub fn list_for_asset(conn: &Connection, asset: i64) -> Result<Vec<AiMaskInfo>> {
    let mut stmt = conn.prepare_cached(
        "SELECT hash, kind, instance, model, score, width, height FROM ai_masks WHERE asset_id = ?1 AND current = 1",
    )?;
    let rows = stmt.query_map([asset], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, u32>(2)?, r.get::<_, String>(3)?, r.get::<_, f64>(4)?, r.get::<_, u32>(5)?, r.get::<_, u32>(6)?))
    })?;
    let mut out = Vec::new();
    for row in rows {
        let (hash, kind, instance, model, score, width, height) = row?;
        if let Some(kind) = AiMaskKind::parse(&kind) {
            out.push(AiMaskInfo { hash, kind, instance, model, score: score as f32, width, height });
        }
    }
    out.sort_by_key(|m| (m.kind as u8, m.instance));
    Ok(out)
}

/// One raster by the hash a recipe names, current or not. `None` when this photo doesn't have
/// it (a mask pasted from another photo) or its row can't be read: a shape with no raster weighs
/// nothing, it is never an error.
pub fn get(conn: &Connection, asset: i64, hash: &str) -> Result<Option<AiMaskRaster>> {
    let row = conn
        .prepare_cached("SELECT kind, instance, width, height, weights FROM ai_masks WHERE asset_id = ?1 AND hash = ?2")?
        .query_row(params![asset, hash], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, u32>(1)?, r.get::<_, u32>(2)?, r.get::<_, u32>(3)?, r.get::<_, Vec<u8>>(4)?))
        })
        .optional()?;
    let Some((kind, instance, width, height, packed)) = row else { return Ok(None) };
    let Some(kind) = AiMaskKind::parse(&kind) else { return Ok(None) };
    let size = width as usize * height as usize;
    let mut weights = Vec::with_capacity(size);
    // One byte more than it should be is enough to tell a damaged row, and bounds the inflate.
    if DeflateDecoder::new(packed.as_slice()).take(size as u64 + 1).read_to_end(&mut weights).is_err() || weights.len() != size {
        tracing::warn!(asset, hash, "an AI mask's stored weights don't inflate to its size");
        return Ok(None);
    }
    Ok(Some(AiMaskRaster { kind, instance, width, height, weights }))
}

/// A render's rasters, by the hash a recipe names.
pub type AiRasters = HashMap<String, AiMaskRaster>;

/// The rasters `recipe` names that this photo has. Nothing is read for a recipe with no AI shape.
pub fn rasters_for(conn: &Connection, asset: i64, recipe: &Recipe) -> Result<AiRasters> {
    let mut out = AiRasters::new();
    for hash in recipe.ai_rasters() {
        if let Some(raster) = get(conn, asset, hash)? {
            out.insert(hash.to_string(), raster);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{apply_migrations, MIGRATIONS};
    use crate::edits::{Develop, LocalAdjust, Mask, MaskShape};

    #[test]
    fn a_render_gets_the_rasters_its_recipe_names_and_a_pasted_one_is_simply_absent() {
        let c = conn();
        let a = asset(&c, 1);
        let m = mask(AiMaskKind::Sky, 0, 100);
        let hash = put_detection(&c, a, "facet@1", std::slice::from_ref(&m)).unwrap().remove(0);
        let shape = |raster: &str| MaskShape::Ai { add: true, target: AiMaskKind::Sky, instance: 0, raster: raster.into() };
        let recipe = Recipe {
            develop: Some(Develop {
                masks: vec![Mask { id: 1, name: "Sky".into(), shapes: vec![shape(&hash), shape(&hash), shape("abcdef")], invert: false, opacity: 1.0, adjust: LocalAdjust { exposure: 1.0, ..Default::default() }, visible: true }],
                ..Default::default()
            }),
            ..Recipe::identity()
        };
        let got = rasters_for(&c, a, &recipe).unwrap();
        assert_eq!(got.keys().collect::<Vec<_>>(), [&hash]);
        assert_eq!(got[&hash].weights, m.weights);
        assert!(rasters_for(&c, a, &Recipe::identity()).unwrap().is_empty());
    }

    #[test]
    fn render_info_brings_an_edits_rasters_and_doesnt_count_a_missing_one_as_an_edit() {
        let c = conn();
        let a = asset(&c, 1);
        let hash = put_detection(&c, a, "facet@1", &[mask(AiMaskKind::Sky, 0, 100)]).unwrap().remove(0);
        let recipe = |raster: &str| Recipe {
            develop: Some(Develop {
                masks: vec![Mask { id: 1, name: "Sky".into(), shapes: vec![MaskShape::Ai { add: true, target: AiMaskKind::Sky, instance: 0, raster: raster.into() }], invert: false, opacity: 1.0, adjust: LocalAdjust { exposure: 1.0, ..Default::default() }, visible: true }],
                ..Default::default()
            }),
            ..Recipe::identity()
        };
        crate::edits::save_recipe(&c, a, &recipe(&hash), 1000).unwrap();
        let info = crate::assets::render_info(&c, a).unwrap();
        assert!(info.edit.is_some() && info.ai_rasters.contains_key(&hash));

        crate::edits::save_recipe(&c, a, &recipe("abcdef"), 2000).unwrap();
        let info = crate::assets::render_info(&c, a).unwrap();
        assert!(info.edit.is_none() && info.ai_rasters.is_empty(), "pasted from another photo: nothing to render");

        // An export goes by the draft, and so do its rasters.
        crate::edits::save_draft(&c, a, &recipe(&hash), 3000).unwrap();
        let info = crate::assets::export_render_info(&c, a).unwrap();
        assert!(info.edit.is_some() && info.ai_rasters.contains_key(&hash));
    }

    fn conn() -> Connection {
        let mut c = Connection::open_in_memory().unwrap();
        c.pragma_update(None, "foreign_keys", "ON").unwrap();
        apply_migrations(&mut c, MIGRATIONS).unwrap();
        c
    }

    fn asset(c: &Connection, id: i64) -> i64 {
        c.execute(
            "INSERT INTO assets (id, content_hash, relative_path, filename, extension, media_type, byte_size, orientation, imported_at)
             VALUES (?1, ?2, ?2, ?2, 'jpg', 0, 1, 1, 1)",
            params![id, format!("h{id}")],
        )
        .unwrap();
        id
    }

    fn mask(kind: AiMaskKind, instance: u32, fill: u8) -> NewAiMask {
        NewAiMask { kind, instance, score: 0.97, width: 4, height: 3, weights: (0..12u8).map(|i| fill.wrapping_add(i)).collect() }
    }

    fn count(c: &Connection, asset: i64) -> i64 {
        c.query_row("SELECT COUNT(*) FROM ai_masks WHERE asset_id = ?1", [asset], |r| r.get(0)).unwrap()
    }

    #[test]
    fn rasters_come_back_as_they_went_in() {
        let c = conn();
        let a = asset(&c, 1);
        let m = mask(AiMaskKind::Sky, 0, 100);
        let hashes = put_detection(&c, a, "facet@1", std::slice::from_ref(&m)).unwrap();
        assert_eq!(hashes, [hash_of(&m)]);
        let got = get(&c, a, &hashes[0]).unwrap().unwrap();
        assert_eq!((got.kind, got.instance, got.width, got.height), (AiMaskKind::Sky, 0, 4, 3));
        assert_eq!(got.weights, m.weights);
    }

    #[test]
    fn a_list_is_in_panel_order_and_carries_no_bytes() {
        let c = conn();
        let a = asset(&c, 1);
        let masks = [
            mask(AiMaskKind::Person, 1, 1),
            mask(AiMaskKind::Background, 0, 2),
            mask(AiMaskKind::Person, 0, 3),
            mask(AiMaskKind::Subject, 0, 4),
            mask(AiMaskKind::Sky, 0, 5),
        ];
        put_detection(&c, a, "facet@1", &masks).unwrap();
        let order: Vec<_> = list_for_asset(&c, a).unwrap().into_iter().map(|m| (m.kind, m.instance)).collect();
        assert_eq!(
            order,
            [(AiMaskKind::Sky, 0), (AiMaskKind::Subject, 0), (AiMaskKind::Background, 0), (AiMaskKind::Person, 0), (AiMaskKind::Person, 1)]
        );
    }

    #[test]
    fn detecting_again_with_the_same_result_changes_nothing() {
        let c = conn();
        let a = asset(&c, 1);
        let masks = [mask(AiMaskKind::Sky, 0, 1), mask(AiMaskKind::Person, 0, 2)];
        let first = put_detection(&c, a, "facet@1", &masks).unwrap();
        let second = put_detection(&c, a, "facet@1", &masks).unwrap();
        assert_eq!(first, second);
        assert_eq!(count(&c, a), 2);
        assert_eq!(list_for_asset(&c, a).unwrap().len(), 2);
    }

    #[test]
    fn a_new_detection_replaces_the_list_but_keeps_the_rasters_recipes_name() {
        let c = conn();
        let a = asset(&c, 1);
        let old = put_detection(&c, a, "facet@1", &[mask(AiMaskKind::Sky, 0, 1)]).unwrap();
        let new = put_detection(&c, a, "facet@2", &[mask(AiMaskKind::Sky, 0, 9)]).unwrap();
        assert_ne!(old, new);
        let listed = list_for_asset(&c, a).unwrap();
        assert_eq!(listed.iter().map(|m| (m.hash.as_str(), m.model.as_str())).collect::<Vec<_>>(), [(new[0].as_str(), "facet@2")]);
        assert!(get(&c, a, &old[0]).unwrap().is_some(), "a recipe that names the old raster still renders");
        assert_eq!(scanned_by(&c, a).unwrap().as_deref(), Some("facet@2"));
    }

    #[test]
    fn a_mask_found_again_after_being_lost_is_current_again() {
        let c = conn();
        let a = asset(&c, 1);
        let sky = mask(AiMaskKind::Sky, 0, 1);
        put_detection(&c, a, "facet@1", std::slice::from_ref(&sky)).unwrap();
        put_detection(&c, a, "facet@1", &[]).unwrap();
        assert!(list_for_asset(&c, a).unwrap().is_empty());
        put_detection(&c, a, "facet@1", &[sky]).unwrap();
        assert_eq!(list_for_asset(&c, a).unwrap().len(), 1);
        assert_eq!(count(&c, a), 1);
    }

    #[test]
    fn a_photo_with_nothing_found_is_still_scanned() {
        let c = conn();
        let a = asset(&c, 1);
        assert_eq!(scanned_by(&c, a).unwrap(), None);
        put_detection(&c, a, "facet@1", &[]).unwrap();
        assert_eq!(scanned_by(&c, a).unwrap().as_deref(), Some("facet@1"));
        assert!(list_for_asset(&c, a).unwrap().is_empty());
    }

    #[test]
    fn subject_and_a_lone_person_with_the_same_weights_keep_a_row_each() {
        let c = conn();
        let a = asset(&c, 1);
        let hashes = put_detection(&c, a, "facet@1", &[mask(AiMaskKind::Subject, 0, 7), mask(AiMaskKind::Person, 0, 7)]).unwrap();
        assert_ne!(hashes[0], hashes[1]);
        assert_eq!(list_for_asset(&c, a).unwrap().len(), 2);
    }

    #[test]
    fn another_photos_hash_or_a_damaged_row_finds_nothing() {
        let c = conn();
        let (a, b) = (asset(&c, 1), asset(&c, 2));
        let hashes = put_detection(&c, a, "facet@1", &[mask(AiMaskKind::Sky, 0, 1)]).unwrap();
        assert_eq!(get(&c, b, &hashes[0]).unwrap(), None, "pasted onto another photo");
        assert_eq!(get(&c, a, "nope").unwrap(), None);
        c.execute("UPDATE ai_masks SET width = 5 WHERE asset_id = ?1", [a]).unwrap();
        assert_eq!(get(&c, a, &hashes[0]).unwrap(), None, "weights that no longer fit the size");
        c.execute("UPDATE ai_masks SET width = 4, weights = x'00ff' WHERE asset_id = ?1", [a]).unwrap();
        assert_eq!(get(&c, a, &hashes[0]).unwrap(), None, "weights that don't inflate");
    }

    #[test]
    fn a_mask_of_the_wrong_size_is_refused_and_nothing_is_written() {
        let c = conn();
        let a = asset(&c, 1);
        let mut bad = mask(AiMaskKind::Sky, 0, 1);
        bad.weights.pop();
        assert!(put_detection(&c, a, "facet@1", &[mask(AiMaskKind::Person, 0, 2), bad]).is_err());
        assert_eq!(count(&c, a), 0);
        assert_eq!(scanned_by(&c, a).unwrap(), None);
    }

    #[test]
    fn deleting_the_photo_takes_its_masks_and_leaves_the_others() {
        let c = conn();
        let (a, b) = (asset(&c, 1), asset(&c, 2));
        put_detection(&c, a, "facet@1", &[mask(AiMaskKind::Sky, 0, 1)]).unwrap();
        put_detection(&c, b, "facet@1", &[mask(AiMaskKind::Sky, 0, 1)]).unwrap();
        c.execute("DELETE FROM assets WHERE id = ?1", [a]).unwrap();
        assert_eq!(count(&c, a), 0);
        assert_eq!(scanned_by(&c, a).unwrap(), None);
        assert_eq!(count(&c, b), 1);
    }
}
