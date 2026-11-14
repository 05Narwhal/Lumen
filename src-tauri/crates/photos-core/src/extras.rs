//! What an import source knows about a photo beyond its file — Apple Photos' favourite
//! flag, albums, title, caption, keywords, and the date and place the user set there.
//! Applied after the asset row exists, to new and duplicate assets alike.

use rusqlite::{params, Connection, OptionalExtension};

use crate::{albums, assets, fts, Result};

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ItemExtras {
    pub favorite: bool,
    /// Manual albums by name; created when missing.
    pub albums: Vec<String>,
    /// A folder import's album (2026-11-08): the collections it sits in, outermost first, then
    /// the album's name. Each is found by name within its parent, else made.
    pub folder_album: Option<Vec<String>>,
    pub title: Option<String>,
    pub caption: Option<String>,
    pub keywords: Vec<String>,
    /// When the source knows better than the file: UTC seconds and the offset in minutes.
    pub captured: Option<(i64, Option<i32>)>,
    /// A wall-clock date with no offset (a plugin's EXIF-style date): `captured_at` takes it and
    /// `captured_at_utc` follows from the offset the photo already has (none: UTC). Ignored when
    /// `captured` is set.
    pub captured_wall: Option<i64>,
    pub location: Option<(f64, f64)>,
    /// A Lightroom catalog's marks (Round 11): 0–5 stars, −1 rejected / 1 picked, a colour
    /// label 1–5 in Lumen's order. Applied without lowering or replacing Lumen's.
    pub rating: Option<i64>,
    pub flag: Option<i64>,
    pub color_label: Option<i64>,
    /// A catalog's develop settings, converted: saved only when the photo has no edit yet.
    pub recipe: Option<crate::edits::Recipe>,
}

/// Writes what `x` says about `asset_id`. Adds, never removes: a photo that is a favourite
/// in Lumen stays one, album membership only grows, a rating is only raised, a flag or label
/// only set where Lumen has none, and an edit only saved where Lumen has none.
pub fn apply(conn: &Connection, asset_id: i64, x: &ItemExtras, now: i64) -> Result<()> {
    if x.favorite {
        assets::set_favorite(conn, &[asset_id], true)?;
    }
    for name in &x.albums {
        let existing = conn
            .query_row(
                "SELECT id FROM albums WHERE name = ?1 AND kind = ?2 ORDER BY id LIMIT 1",
                params![name, albums::KIND_MANUAL],
                |r| r.get::<_, i64>(0),
            )
            .optional()?;
        let id = match existing {
            Some(id) => id,
            None => albums::create_album(conn, name, None, now)?,
        };
        albums::add_to_album(conn, id, &[asset_id], now)?;
    }
    if let Some((album, collections)) = x.folder_album.as_ref().and_then(|p| p.split_last()) {
        let mut parent: Option<i64> = None;
        for name in collections {
            let found = conn
                .query_row("SELECT id FROM album_collections WHERE name = ?1 AND parent_id IS ?2 ORDER BY id LIMIT 1", params![name.trim(), parent], |r| r.get(0))
                .optional()?;
            parent = Some(match found {
                Some(id) => id,
                None => crate::album_collections::create(conn, name, parent, now)?,
            });
        }
        let found = conn
            .query_row(
                "SELECT id FROM albums WHERE name = ?1 AND kind = ?2 AND collection_id IS ?3 ORDER BY id LIMIT 1",
                params![album.trim(), albums::KIND_MANUAL, parent],
                |r| r.get::<_, i64>(0),
            )
            .optional()?;
        let id = match found {
            Some(id) => id,
            None => {
                let id = albums::create_album(conn, album, None, now)?;
                if parent.is_some() {
                    crate::album_collections::move_album(conn, id, parent, now)?;
                }
                id
            }
        };
        albums::add_to_album(conn, id, &[asset_id], now)?;
    }
    // One per line, so a keyword of two words is still one keyword on export.
    let keywords = (!x.keywords.is_empty()).then(|| x.keywords.join("\n"));
    if x.title.is_some() || x.caption.is_some() || keywords.is_some() {
        conn.execute(
            "UPDATE assets SET title = COALESCE(?2, title), caption = COALESCE(?3, caption), keywords = COALESCE(?4, keywords) WHERE id = ?1",
            params![asset_id, x.title, x.caption, keywords],
        )?;
    }
    if let Some((utc, offset)) = x.captured {
        let local = utc + offset.unwrap_or(0) as i64 * 60;
        conn.execute(
            "UPDATE assets SET captured_at = ?2, captured_at_utc = ?3, tz_offset_minutes = COALESCE(?4, tz_offset_minutes) WHERE id = ?1",
            params![asset_id, local, utc, offset],
        )?;
    } else if let Some(local) = x.captured_wall {
        conn.execute(
            "UPDATE assets SET captured_at = ?2, captured_at_utc = ?2 - COALESCE(tz_offset_minutes, 0) * 60 WHERE id = ?1",
            params![asset_id, local],
        )?;
    }
    if let Some((lat, lon)) = x.location {
        // Clearing the place name hands the new position to the maintenance pass's geocoder.
        conn.execute("UPDATE assets SET gps_lat = ?2, gps_lon = ?3, place_name = NULL, locality = NULL, place_source = 0 WHERE id = ?1", params![asset_id, lat, lon])?;
    }
    if let Some(rating) = x.rating.filter(|r| *r > 0) {
        conn.execute("UPDATE assets SET rating = ?2 WHERE id = ?1 AND rating < ?2", params![asset_id, rating.clamp(0, 5)])?;
    }
    if let Some(flag) = x.flag.filter(|f| *f != 0) {
        conn.execute("UPDATE assets SET flag = ?2 WHERE id = ?1 AND flag = 0", params![asset_id, flag.clamp(-1, 1)])?;
    }
    if let Some(label) = x.color_label.filter(|l| *l != 0) {
        conn.execute("UPDATE assets SET color_label = ?2 WHERE id = ?1 AND color_label = 0", params![asset_id, label.clamp(0, 5)])?;
    }
    if let Some(recipe) = &x.recipe {
        // The same save the editor makes: a new version, so the grid draws the photo again.
        if crate::edits::get_recipe(conn, asset_id)?.is_none() {
            crate::edits::save_recipe(conn, asset_id, recipe, now * 1000)?;
        }
    }
    fts::index_ids(conn, &[asset_id])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Db;
    use crate::synthetic;

    #[test]
    fn a_folder_album_lands_in_its_collections_made_once() {
        let db = Db::open_in_memory().unwrap();
        db.with_conn(|c| {
            synthetic::generate(c, 4, 7)?;
            let path = |p: &[&str]| ItemExtras { folder_album: Some(p.iter().map(|s| s.to_string()).collect()), ..Default::default() };
            apply(c, 1, &path(&["Trips", "2019", "Summer"]), 100)?;
            apply(c, 2, &path(&["Trips", "2019", "Summer"]), 100)?;
            apply(c, 3, &path(&["Trips", "2020", "Summer"]), 100)?;
            apply(c, 4, &path(&["Loose"]), 100)?;
            // An album of the same name elsewhere is not the folder's.
            let elsewhere = albums::create_album(c, "Summer", None, 50)?;
            apply(c, 1, &path(&["Trips", "2019", "Summer"]), 200)?;
            let rows: Vec<(String, Option<String>, Option<String>, i64)> = c
                .prepare(
                    "SELECT a.name, k.name, p.name, (SELECT COUNT(*) FROM album_items i WHERE i.album_id = a.id) FROM albums a
                     LEFT JOIN album_collections k ON k.id = a.collection_id LEFT JOIN album_collections p ON p.id = k.parent_id
                     WHERE a.id != ?1 ORDER BY a.id",
                )?
                .query_map([elsewhere], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?
                .collect::<std::result::Result<_, _>>()?;
            let s = |v: &str| Some(v.to_string());
            assert_eq!(
                rows,
                [("Summer".into(), s("2019"), s("Trips"), 2), ("Summer".into(), s("2020"), s("Trips"), 1), ("Loose".into(), None, None, 1)],
                "one album per folder, each in its own collections, made once"
            );
            let collections: i64 = c.query_row("SELECT COUNT(*) FROM album_collections", [], |r| r.get(0))?;
            assert_eq!(collections, 3, "Trips, 2019 and 2020, once each");
            let stray: i64 = c.query_row("SELECT COUNT(*) FROM album_items WHERE album_id = ?1", [elsewhere], |r| r.get(0))?;
            assert_eq!(stray, 0);
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn extras_mark_favourites_fill_albums_and_become_searchable() {
        let db = Db::open_in_memory().unwrap();
        db.with_conn(|c| {
            synthetic::generate(c, 3, 7)?;
            let x = ItemExtras {
                favorite: true,
                albums: vec!["Iceland".into(), "Best of".into()],
                title: Some("Skógafoss".into()),
                caption: Some("Rainbow over the falls".into()),
                keywords: vec!["waterfall".into()],
                ..Default::default()
            };
            apply(c, 1, &x, 100)?;
            apply(c, 2, &ItemExtras { albums: vec!["Iceland".into()], ..Default::default() }, 100)?;
            let fav: i64 = c.query_row("SELECT is_favorite FROM assets WHERE id = 1", [], |r| r.get(0))?;
            assert_eq!(fav, 1);
            let albums: Vec<(String, i64)> = c
                .prepare("SELECT a.name, COUNT(i.asset_id) FROM albums a JOIN album_items i ON i.album_id = a.id GROUP BY a.id ORDER BY a.name")?
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
                .collect::<std::result::Result<_, _>>()?;
            assert_eq!(albums, [("Best of".to_string(), 1), ("Iceland".to_string(), 2)], "one Iceland album, not two");
            for word in ["rainbow", "skógafoss", "waterfall"] {
                let hits: i64 = c.query_row("SELECT COUNT(*) FROM assets_fts WHERE assets_fts MATCH ?1", [word], |r| r.get(0))?;
                assert_eq!(hits, 1, "{word}");
            }
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn a_wall_clock_date_keeps_the_photos_offset_and_utc_agrees() {
        let db = Db::open_in_memory().unwrap();
        db.with_conn(|c| {
            synthetic::generate(c, 2, 7)?;
            c.execute("UPDATE assets SET tz_offset_minutes = 120 WHERE id = 1", [])?;
            c.execute("UPDATE assets SET tz_offset_minutes = NULL WHERE id = 2", [])?;
            for id in [1, 2] {
                apply(c, id, &ItemExtras { captured_wall: Some(1_700_007_200), ..Default::default() }, 100)?;
            }
            let row = |id: i64| -> rusqlite::Result<(i64, i64, Option<i64>)> {
                c.query_row("SELECT captured_at, captured_at_utc, tz_offset_minutes FROM assets WHERE id = ?1", [id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            };
            assert_eq!(row(1)?, (1_700_007_200, 1_700_000_000, Some(120)), "UTC is the wall clock less the photo's +02:00");
            assert_eq!(row(2)?, (1_700_007_200, 1_700_007_200, None), "no offset known: the wall clock is taken as UTC");
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn a_date_and_place_from_the_source_replace_what_the_file_said() {
        let db = Db::open_in_memory().unwrap();
        db.with_conn(|c| {
            synthetic::generate(c, 1, 7)?;
            c.execute("UPDATE assets SET place_name = 'Somewhere else' WHERE id = 1", [])?;
            let x = ItemExtras { captured: Some((1_700_000_000, Some(120))), location: Some((64.04, -16.18)), ..Default::default() };
            apply(c, 1, &x, 100)?;
            let row: (i64, i64, i64, f64, f64, Option<String>) = c.query_row(
                "SELECT captured_at, captured_at_utc, tz_offset_minutes, gps_lat, gps_lon, place_name FROM assets WHERE id = 1",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)),
            )?;
            // The place name is cleared so the maintenance pass geocodes the new position.
            assert_eq!(row, (1_700_007_200, 1_700_000_000, 120, 64.04, -16.18, None));
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn a_catalogs_marks_add_to_lumens_and_never_lower_them() {
        let db = Db::open_in_memory().unwrap();
        db.with_conn(|c| {
            synthetic::generate(c, 2, 7)?;
            c.execute_batch("UPDATE assets SET rating = 2, flag = 0, color_label = 0 WHERE id = 1; UPDATE assets SET rating = 5, flag = -1, color_label = 3 WHERE id = 2;")?;
            let x = ItemExtras { rating: Some(4), flag: Some(1), color_label: Some(1), ..Default::default() };
            apply(c, 1, &x, 100)?;
            apply(c, 2, &x, 100)?;
            let marks = |id: i64| c.query_row("SELECT rating, flag, color_label FROM assets WHERE id = ?1", [id], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?, r.get::<_, i64>(2)?)));
            assert_eq!(marks(1)?, (4, 1, 1), "raised, and set where Lumen had none");
            assert_eq!(marks(2)?, (5, -1, 3), "Lumen's higher rating, its flag and its label stay");
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn a_catalogs_edit_is_saved_only_where_lumen_has_none() {
        let db = Db::open_in_memory().unwrap();
        db.with_conn(|c| {
            synthetic::generate(c, 2, 7)?;
            let lumen = crate::edits::Recipe { rotate: 90, ..crate::edits::Recipe::identity() };
            crate::edits::save_recipe(c, 2, &lumen, 1_000)?;
            let develop = crate::edits::Develop { basic: crate::edits::Basic { exposure: 0.5, ..Default::default() }, ..Default::default() };
            let lightroom = crate::edits::Recipe { develop: Some(develop), ..crate::edits::Recipe::identity() };
            let x = ItemExtras { recipe: Some(lightroom.clone()), ..Default::default() };
            apply(c, 1, &x, 100)?;
            apply(c, 2, &x, 100)?;
            let (r1, v1) = crate::edits::get_recipe(c, 1)?.expect("saved");
            assert_eq!(r1.develop.map(|d| d.basic.exposure), Some(0.5));
            assert!(v1 > 0, "a version, so the grid draws it again");
            assert_eq!(crate::edits::get_recipe(c, 2)?.map(|r| r.0.rotate), Some(90), "Lumen's own edit stays");
            // Importing again changes nothing: the first import's edit is now Lumen's.
            apply(c, 1, &ItemExtras { recipe: Some(crate::edits::Recipe::identity()), ..Default::default() }, 100)?;
            assert!(crate::edits::get_recipe(c, 1)?.is_some());
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn nothing_to_say_changes_nothing() {
        let db = Db::open_in_memory().unwrap();
        db.with_conn(|c| {
            synthetic::generate(c, 1, 7)?;
            let before: (Option<i64>, Option<f64>) = c.query_row("SELECT captured_at, gps_lat FROM assets WHERE id = 1", [], |r| Ok((r.get(0)?, r.get(1)?)))?;
            apply(c, 1, &ItemExtras::default(), 100)?;
            let after: (Option<i64>, Option<f64>) = c.query_row("SELECT captured_at, gps_lat FROM assets WHERE id = 1", [], |r| Ok((r.get(0)?, r.get(1)?)))?;
            assert_eq!(before, after);
            Ok(())
        })
        .unwrap();
    }
}
