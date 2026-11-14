//! Hidden photos (round 3): a photo in the Hidden folder or in an album that hides its
//! items is `assets.concealed = 1`, kept true by triggers (migration 0019), and every query
//! that shows or curates photos leaves it out. It appears only in its own album or the
//! Hidden folder. Maintenance (thumbnails, embeddings, faces) still sees it.

use rusqlite::{params, Connection};

use crate::Result;

/// SQL that is true for a photo the library may show, for alias `a`.
pub const SHOWN: &str = "a.concealed = 0";

/// Marks `album_id` as hiding its photos from the rest of the library, or not.
pub fn set_album_hides(conn: &Connection, album_id: i64, hides: bool, now: i64) -> Result<()> {
    let n = conn.execute("UPDATE albums SET hides_items = ?2, updated_at = ?3 WHERE id = ?1", params![album_id, hides as i64, now])?;
    if n == 0 {
        return Err(crate::CoreError::NotFound(format!("album {album_id}")));
    }
    Ok(())
}

/// Whether `album_id` hides its photos (false for an album that does not exist).
pub fn album_hides(conn: &Connection, album_id: i64) -> Result<bool> {
    use rusqlite::OptionalExtension;
    Ok(conn
        .query_row("SELECT hides_items FROM albums WHERE id = ?1", [album_id], |r| r.get::<_, i64>(0))
        .optional()?
        .is_some_and(|h| h != 0))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::albums::{add_to_album, create_album, delete_album, get_album, remove_from_album};
    use crate::assets::{insert_assets, neighbors, NewAsset};
    use crate::geo::{assets_in_bounds, Bounds};
    use crate::layout::count_scoped;
    use crate::scope::LayoutScope;
    use crate::Db;

    fn photo(hash: &str, at: i64, device: Option<i64>) -> NewAsset {
        NewAsset {
            content_hash: hash.into(),
            relative_path: format!("x/{hash}.jpg"),
            filename: format!("{hash}.jpg"),
            extension: "jpg".into(),
            media_type: 0,
            byte_size: 1,
            width: Some(400),
            height: Some(300),
            orientation: 1,
            captured_at: Some(at),
            imported_at: 1,
            gps_lat: Some(46.05),
            gps_lon: Some(14.5),
            device_id: device,
            ..Default::default()
        }
    }

    /// Review Focus 3: a photo in a hiding album that is also in a normal album, a favourite,
    /// on the map, of a person and between two others on the timeline.
    #[test]
    fn concealed_photos_are_nowhere_but_their_album() {
        let db = Db::open_in_memory().unwrap();
        db.with_conn(|c| {
            let device = crate::devices::upsert_device(c, Some("Apple"), Some("iPhone 13"), None, 1)?.unwrap();
            let ids = insert_assets(c, &[photo("q1", 100, Some(device)), photo("p", 200, Some(device)), photo("q2", 300, Some(device))])?;
            let (q1, p, q2) = (ids[0], ids[1], ids[2]);
            c.execute("UPDATE assets SET is_favorite = 1", [])?;
            c.execute("INSERT INTO people (id, name, created_at) VALUES (1, 'Anna', 1)", [])?;
            for id in [q1, p] {
                c.execute("INSERT INTO faces (asset_id, x, y, w, h, score, vec, model, person_id) VALUES (?1, 0, 0, 1, 1, 1, x'00', 'm', 1)", [id])?;
            }
            let school = create_album(c, "School", None, 1)?;
            let trip = create_album(c, "Trip", None, 1)?;
            add_to_album(c, trip, &[q1, p], 2)?;
            add_to_album(c, school, &[p], 2)?;
            set_album_hides(c, school, true, 3)?;

            let count = |s: LayoutScope| count_scoped(c, &s).unwrap();
            assert_eq!(count(LayoutScope::library()), 2, "the library");
            assert_eq!(count(LayoutScope { album_id: Some(trip), ..Default::default() }), 1, "another album");
            assert_eq!(count(LayoutScope { favorites: true, ..Default::default() }), 2, "favourites");
            assert_eq!(count(LayoutScope { person_id: Some(1), ..Default::default() }), 1, "a person");
            assert_eq!(count(LayoutScope { album_id: Some(school), ..Default::default() }), 1, "its own album");
            assert_eq!(get_album(c, trip)?.item_count, 1);
            assert_eq!(get_album(c, school)?.item_count, 1);
            assert!(get_album(c, school)?.hides_items);
            let people = crate::people::list_people(c)?;
            assert_eq!(people[0].photo_count, 1, "a person's count");
            assert_eq!(crate::devices::list_devices(c)?[0].item_count, 2, "a device's count");
            let world = Bounds { min_lat: -90.0, max_lat: 90.0, min_lon: -180.0, max_lon: 180.0 };
            assert!(!assets_in_bounds(c, world, &LayoutScope::library(), 10)?.contains(&p), "the map");
            assert!(!crate::memories::load_photos(c)?.iter().any(|ph| ph.id == p), "memories");
            assert_eq!(neighbors(c, q2)?.next, Some(q1), "the viewer steps over it");
            assert!(!crate::assets::favorite_ids(c, 10)?.contains(&p));

            // Out of the hiding album, it is everywhere again.
            remove_from_album(c, school, &[p])?;
            assert_eq!(count(LayoutScope::library()), 3);
            // Deleting a hiding album frees its photos too.
            add_to_album(c, school, &[p], 4)?;
            assert_eq!(count(LayoutScope::library()), 2);
            delete_album(c, school)?;
            assert_eq!(count(LayoutScope::library()), 3);
            // Turning hiding off frees them as well.
            let other = create_album(c, "Other", None, 5)?;
            set_album_hides(c, other, true, 5)?;
            add_to_album(c, other, &[q2], 5)?;
            assert_eq!(count(LayoutScope::library()), 2);
            set_album_hides(c, other, false, 6)?;
            assert_eq!(count(LayoutScope::library()), 3);
            Ok(())
        })
        .unwrap();
    }
}
