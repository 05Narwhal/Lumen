//! Turning photos from the grid (round 3): a quarter turn is an edit, so the file is never
//! touched and *Undo* puts the old recipe back (`develop::restore_recipes`).

use rusqlite::Connection;

use crate::develop::PreviousRecipe;
use crate::edits::{get_recipe, save_recipe, Crop, Recipe};
use crate::Result;

/// `r` turned a quarter clockwise (`dir` 1) or anticlockwise (-1). The crop is relative to
/// the frame after the turn, so it turns with the picture — the same rule as the quick
/// editor's `rotateBy` in `edit-geometry.ts`.
pub fn turned(r: &Recipe, dir: i32) -> Recipe {
    let crop = r.crop.map(|k| {
        if dir >= 0 {
            Crop { x: 1.0 - k.y - k.h, y: k.x, w: k.h, h: k.w }
        } else {
            Crop { x: k.y, y: 1.0 - k.x - k.w, w: k.h, h: k.w }
        }
    });
    let rotate = (r.rotate as i32 + if dir >= 0 { 90 } else { 270 }).rem_euclid(360) as u16;
    Recipe { rotate, crop, ..r.clone() }
}

/// Turns each photo in `ids` a quarter (`dir` 1 clockwise, -1 anticlockwise). Videos and
/// trashed items are skipped. Returns what each recipe was, for *Undo*.
pub fn rotate_assets(conn: &Connection, ids: &[i64], dir: i32, now_ms: i64) -> Result<Vec<PreviousRecipe>> {
    let tx = conn.unchecked_transaction()?;
    let mut previous = Vec::new();
    for &id in ids {
        let media_type: Option<i64> = tx.query_row("SELECT media_type FROM assets WHERE id = ?1 AND state = 0", [id], |r| r.get(0)).ok();
        if media_type.is_none_or(|m| m == 1) {
            continue;
        }
        let old = get_recipe(&tx, id)?.map(|(r, _)| r);
        let new = turned(&old.clone().unwrap_or_else(Recipe::identity), dir);
        save_recipe(&tx, id, &new, now_ms)?;
        // Develop's history shows the turn, so opening the photo there does not jump.
        crate::develop_history::push_step(&tx, id, &new.clone().normalized(), if dir >= 0 { "Rotate right" } else { "Rotate left" }, now_ms)?;
        previous.push(PreviousRecipe { asset_id: id, recipe: old });
    }
    tx.commit()?;
    Ok(previous)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assets::{insert_assets, NewAsset};
    use crate::develop::restore_recipes;
    use crate::Db;

    fn asset(hash: &str, media_type: i64) -> NewAsset {
        NewAsset {
            content_hash: hash.into(),
            relative_path: format!("x/{hash}.jpg"),
            filename: format!("{hash}.jpg"),
            extension: "jpg".into(),
            media_type,
            byte_size: 1,
            width: Some(4000),
            height: Some(3000),
            orientation: 1,
            imported_at: 1,
            ..Default::default()
        }
    }

    #[test]
    fn quarter_turns_add_up_and_wrap() {
        let r = Recipe::identity();
        assert_eq!(turned(&r, 1).rotate, 90);
        assert_eq!(turned(&turned(&r, 1), 1).rotate, 180);
        assert_eq!(turned(&r, -1).rotate, 270);
        assert_eq!(turned(&turned(&r, -1), 1).rotate, 0);
    }

    #[test]
    fn the_crop_turns_with_the_picture() {
        // The top-left quarter of a landscape frame…
        let r = Recipe { crop: Some(Crop { x: 0.0, y: 0.0, w: 0.5, h: 0.5 }), ..Recipe::identity() };
        // …turned clockwise is the top-right quarter of the portrait frame.
        assert_eq!(turned(&r, 1).crop, Some(Crop { x: 0.5, y: 0.0, w: 0.5, h: 0.5 }));
        // A turn and its opposite give the crop back.
        let r = Recipe { crop: Some(Crop { x: 0.1, y: 0.2, w: 0.3, h: 0.6 }), ..Recipe::identity() };
        let back = turned(&turned(&r, 1), -1).crop.unwrap();
        let k = r.crop.unwrap();
        assert!((back.x - k.x).abs() < 1e-6 && (back.y - k.y).abs() < 1e-6 && (back.w - k.w).abs() < 1e-6 && (back.h - k.h).abs() < 1e-6);
        // The frame the core predicts swaps axes.
        assert_eq!(turned(&Recipe::identity(), 1).output_size(4000, 3000), (3000, 4000));
    }

    #[test]
    fn rotating_skips_videos_and_undo_restores_exactly() {
        let db = Db::open_in_memory().unwrap();
        db.with_conn(|c| {
            let ids = insert_assets(c, &[asset("a", 0), asset("b", 1)])?;
            let edited = Recipe { straighten: 2.0, ..Recipe::identity() };
            save_recipe(c, ids[0], &edited, 1)?;
            let previous = rotate_assets(c, &ids, 1, 2)?;
            assert_eq!(previous.len(), 1, "the video is skipped");
            assert_eq!(get_recipe(c, ids[0])?.unwrap().0.rotate, 90);
            assert_eq!(get_recipe(c, ids[0])?.unwrap().0.straighten, 2.0);
            assert!(get_recipe(c, ids[1])?.is_none());
            restore_recipes(c, &previous, 3)?;
            assert_eq!(get_recipe(c, ids[0])?.unwrap().0, edited);
            Ok(())
        })
        .unwrap();
    }
}
