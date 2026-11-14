//! Develop settings as panels (spec §1): taking some panels of one photo's develop onto
//! others — a preset, *Paste*, *Sync settings* — and undoing it.

use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::edits::{delete_draft, develop_recipe, get_draft, save_draft, save_recipe, Develop, Recipe};
use crate::Result;

/// A panel of the Develop screen, and the part of `Develop` it sets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export)]
pub enum Panel {
    Basic,
    Curve,
    Hsl,
    Grading,
    Detail,
    Effects,
    /// The photo's masks (recipe v3). Off by default when copying, never in a preset.
    Masks,
}

impl Panel {
    pub const ALL: [Panel; 7] = [Panel::Basic, Panel::Curve, Panel::Hsl, Panel::Grading, Panel::Detail, Panel::Effects, Panel::Masks];
}

/// `into` with the chosen panels taken from `from`. Taking Basic from elsewhere means the
/// Basic values are no longer *Auto*'s own.
pub fn merge_panels(into: &Develop, from: &Develop, panels: &[Panel]) -> Develop {
    let mut d = into.clone();
    for p in panels {
        match p {
            Panel::Basic => {
                d.basic = from.basic;
                d.auto = from.auto;
            }
            Panel::Curve => d.curve = from.curve.clone(),
            Panel::Hsl => d.hsl = from.hsl,
            Panel::Grading => d.grading = from.grading,
            Panel::Detail => d.detail = from.detail.clone(),
            Panel::Effects => d.effects = from.effects,
            Panel::Masks => d.masks = from.masks.clone(),
        }
    }
    d
}

/// What a photo's recipe was before a paste, so *Undo* can put it back.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PreviousRecipe {
    #[ts(type = "number")]
    pub asset_id: i64,
    pub recipe: Option<Recipe>,
}

/// Take `panels` of `from` onto each photo in `ids` (videos are skipped), in Develop's drafts
/// (Round 6: the library keeps its look until export). Each keeps its own geometry and trim,
/// gets a new version and a history step; the old drafts come back for *Undo*
/// (`restore_drafts`).
pub fn paste_develop(conn: &Connection, ids: &[i64], from: &Develop, panels: &[Panel], label: &str, now_ms: i64) -> Result<Vec<PreviousRecipe>> {
    let tx = conn.unchecked_transaction()?;
    let mut previous = Vec::new();
    for &id in ids {
        let media_type: Option<i64> = tx.query_row("SELECT media_type FROM assets WHERE id = ?1 AND state = 0", [id], |r| r.get(0)).ok();
        if media_type.is_none_or(|m| m == 1) {
            continue;
        }
        let old = get_draft(&tx, id)?.map(|(r, _)| r);
        let base = develop_recipe(&tx, id)?.unwrap_or_else(Recipe::identity);
        let develop = merge_panels(&base.develop.clone().unwrap_or_default(), from, panels);
        let new = Recipe { develop: Some(develop), ..base };
        save_draft(&tx, id, &new, now_ms)?;
        crate::develop_history::push_step(&tx, id, &new.clone().normalized(), label, now_ms)?;
        previous.push(PreviousRecipe { asset_id: id, recipe: old });
    }
    tx.commit()?;
    Ok(previous)
}

/// Put back the drafts a paste replaced: a photo that had none has none again.
pub fn restore_drafts(conn: &Connection, previous: &[PreviousRecipe], now_ms: i64) -> Result<()> {
    let tx = conn.unchecked_transaction()?;
    for p in previous {
        match &p.recipe {
            Some(r) => {
                save_draft(&tx, p.asset_id, r, now_ms)?;
            }
            None => {
                delete_draft(&tx, p.asset_id)?;
            }
        }
    }
    tx.commit()?;
    Ok(())
}

/// Put back the library recipes a grid action (a turn) replaced.
pub fn restore_recipes(conn: &Connection, previous: &[PreviousRecipe], now_ms: i64) -> Result<()> {
    let tx = conn.unchecked_transaction()?;
    for p in previous {
        match &p.recipe {
            Some(r) => {
                save_recipe(&tx, p.asset_id, r, now_ms)?;
            }
            None => {
                crate::edits::delete_recipe(&tx, p.asset_id)?;
            }
        }
    }
    tx.commit()?;
    Ok(())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::db::Db;
    use crate::edits::{get_recipe, Basic, Crop, Effects, Vignette};

    /// Photos 1–3 and video 4.
    pub(crate) fn library() -> Db {
        let db = Db::open_in_memory().unwrap();
        db.with_conn(|c| {
            c.execute_batch(
                "INSERT INTO assets (id, content_hash, relative_path, filename, extension, byte_size, media_type, imported_at, captured_at, rating, width, height) VALUES
                 (1, 'h1', 'a/1.jpg', 'c.jpg', 'jpg', 1, 0, 0, 300, 2, 400, 300),
                 (2, 'h2', 'a/2.jpg', 'a.jpg', 'jpg', 1, 0, 0, 100, 5, 400, 300),
                 (3, 'h3', 'a/3.jpg', 'b.jpg', 'jpg', 1, 0, 0, 200, 5, 400, 300),
                 (4, 'h4', 'a/4.mov', 'd.mov', 'mov', 1, 1, 0, 150, 0, 400, 300)",
            )?;
            Ok(())
        })
        .unwrap();
        db
    }

    fn warm() -> Develop {
        Develop {
            basic: Basic { temperature: 30.0, exposure: 0.5, ..Default::default() },
            effects: Effects { vignette: Vignette { amount: -0.4, ..Default::default() }, ..Default::default() },
            ..Default::default()
        }
    }

    #[test]
    fn merging_takes_exactly_the_chosen_panels() {
        let mine = Develop { basic: Basic { contrast: 0.3, ..Default::default() }, auto: true, ..Default::default() };
        let m = merge_panels(&mine, &warm(), &[Panel::Effects]);
        assert_eq!(m.basic.contrast, 0.3);
        assert!(m.auto);
        assert_eq!(m.effects.vignette.amount, -0.4);
        let m = merge_panels(&mine, &warm(), &[Panel::Basic]);
        assert_eq!((m.basic.exposure, m.basic.contrast, m.auto), (0.5, 0.0, false));
        assert_eq!(m.effects, Effects::default());
    }

    #[test]
    fn masks_are_a_panel_of_their_own() {
        let theirs = crate::edits::tests::masked(crate::edits::LocalAdjust { exposure: 1.0, ..Default::default() }).develop.unwrap();
        let theirs = Develop { basic: Basic { exposure: 0.7, ..Default::default() }, ..theirs };
        let mine = Develop { basic: Basic { contrast: 0.3, ..Default::default() }, ..Default::default() };
        let m = merge_panels(&mine, &theirs, &[Panel::Masks]);
        assert_eq!(m.masks, theirs.masks, "Masks copies the masks");
        assert_eq!(m.basic, mine.basic, "and nothing else");
        let m = merge_panels(&mine, &theirs, &Panel::ALL.iter().copied().filter(|p| *p != Panel::Masks).collect::<Vec<_>>());
        assert!(m.masks.is_empty(), "every other panel leaves the masks alone");
        assert!(Panel::ALL.contains(&Panel::Masks));
    }

    #[test]
    fn a_paste_keeps_geometry_skips_videos_and_undoes_exactly() {
        let db = library();
        db.with_conn(|c| {
            let cropped = Recipe { crop: Some(Crop { x: 0.1, y: 0.1, w: 0.5, h: 0.5 }), ..Recipe::identity() };
            save_recipe(c, 1, &cropped, 1_000)?;
            let prev = paste_develop(c, &[1, 2, 4], &warm(), &[Panel::Basic], "Pasted settings", 2_000)?;
            assert_eq!(prev.len(), 2, "the video is skipped");
            // Round 6: into Develop's drafts; the library keeps its look.
            let (r1, _) = get_draft(c, 1)?.unwrap();
            assert_eq!(r1.crop, cropped.crop, "geometry stays");
            assert_eq!(r1.develop.as_ref().unwrap().basic.exposure, 0.5);
            assert_eq!(r1.develop.as_ref().unwrap().effects, Effects::default(), "only Basic was pasted");
            assert_eq!(get_recipe(c, 1)?.unwrap().0, cropped.clone().normalized(), "the library is untouched");
            assert!(get_draft(c, 2)?.is_some() && get_recipe(c, 2)?.is_none());
            assert!(get_draft(c, 4)?.is_none());
            assert_eq!(crate::develop_history::list_history(c, 2)?.len(), 1);
            restore_drafts(c, &prev, 3_000)?;
            assert!(get_draft(c, 1)?.is_none() && get_draft(c, 2)?.is_none(), "neither had a draft before");
            assert_eq!(get_recipe(c, 1)?.unwrap().0, cropped.normalized());
            Ok(())
        })
        .unwrap();
    }
}
