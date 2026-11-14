//! Built-in collections: Favorites, Videos, Live Photos, RAW, HDR, Screenshots, Panoramas
//! and Selfies. Each is a fixed `AssetQuery` over the library, so a collection's grid, its
//! count and the filter chip that names it all come from the one query path.

use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::query::{AssetKind, AssetQuery};
use crate::scope::LayoutScope;
use crate::Result;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export)]
pub enum CollectionId {
    Favorites,
    Videos,
    Live,
    Raw,
    Hdr,
    Screenshots,
    Panoramas,
    Selfies,
}

impl CollectionId {
    pub const ALL: [CollectionId; 8] = [
        CollectionId::Favorites,
        CollectionId::Videos,
        CollectionId::Live,
        CollectionId::Raw,
        CollectionId::Hdr,
        CollectionId::Screenshots,
        CollectionId::Panoramas,
        CollectionId::Selfies,
    ];

    /// The query the collection is. The frontend opens the Library with this same query.
    pub fn query(self) -> AssetQuery {
        let kind = |k| AssetQuery { kind: Some(k), ..Default::default() };
        match self {
            CollectionId::Favorites => AssetQuery { favorites: true, ..Default::default() },
            CollectionId::Videos => AssetQuery { media_type: Some(1), ..Default::default() },
            CollectionId::Live => kind(AssetKind::Live),
            CollectionId::Raw => kind(AssetKind::Raw),
            CollectionId::Hdr => kind(AssetKind::Hdr),
            CollectionId::Screenshots => kind(AssetKind::Screenshot),
            CollectionId::Panoramas => kind(AssetKind::Panorama),
            CollectionId::Selfies => kind(AssetKind::Selfie),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Collection {
    pub id: CollectionId,
    pub query: AssetQuery,
    #[ts(type = "number")]
    pub count: i64,
    /// The newest item, for the card.
    #[ts(type = "number | null")]
    pub cover_asset_id: Option<i64>,
}

/// Every collection with its count and cover, empty ones included; the caller decides
/// whether an empty collection is worth a card (Favorites always is).
pub fn list_collections(conn: &Connection) -> Result<Vec<Collection>> {
    CollectionId::ALL
        .iter()
        .map(|&id| {
            let scope = LayoutScope { query: Some(id.query()), ..LayoutScope::library() };
            // Count and cover in one pass: most kinds scan the catalogue, and two scans each
            // made the Albums index a slow call (Round 7).
            let (from_where, order, params) = scope.sql();
            let found: Option<(i64, i64)> = conn
                .query_row(
                    &format!("SELECT a.id, COUNT(*) OVER () {from_where} ORDER BY {order} LIMIT 1"),
                    rusqlite::params_from_iter(params.iter()),
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()?;
            let (cover, count) = match found {
                Some((cover, count)) => (Some(cover), count),
                None => (None, 0),
            };
            Ok(Collection { id, query: id.query(), count, cover_asset_id: cover })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Db;
    use rusqlite::params;

    struct Row<'a> {
        name: &'a str,
        media_type: i64,
        size: (i64, i64),
        hdr: i64,
        device: Option<i64>,
        lens: Option<&'a str>,
        trashed: bool,
        favorite: bool,
    }

    impl Default for Row<'_> {
        fn default() -> Self {
            Self { name: "", media_type: 0, size: (4032, 3024), hdr: 0, device: Some(1), lens: None, trashed: false, favorite: false }
        }
    }

    /// Inserted in order, each newer than the last, so "cover" means "the latest added".
    fn add(c: &Connection, r: Row) -> i64 {
        let ext = r.name.rsplit('.').next().unwrap().to_ascii_lowercase();
        let meta = r.lens.map(|l| format!(r#"{{"lens":"{l}"}}"#));
        let n: i64 = c.query_row("SELECT COUNT(*) FROM assets", [], |x| x.get(0)).unwrap();
        c.execute(
            "INSERT INTO assets (content_hash, relative_path, filename, extension, media_type, byte_size, width, height, orientation,
                                 imported_at, hdr, device_id, metadata_json, state, is_favorite)
             VALUES (?1, ?1, ?1, ?2, ?3, 1, ?4, ?5, 1, ?6, ?7, ?8, ?9, ?10, ?11)",
            params![r.name, ext, r.media_type, r.size.0, r.size.1, 1000 + n, r.hdr, r.device, meta, r.trashed as i64, r.favorite],
        )
        .unwrap();
        c.last_insert_rowid()
    }

    #[test]
    fn each_collection_holds_what_its_name_says() {
        let db = Db::open_in_memory().unwrap();
        db.with_conn(|c| {
            c.execute(
                "INSERT INTO devices (id, make, model, display_name, kind, first_seen_at) VALUES (1, 'Apple', 'iPhone 12', 'iPhone 12', 1, 1)",
                [],
            )?;
            let fav = add(c, Row { name: "IMG_1.jpg", lens: Some("iPhone 12 back camera 4.2mm f/1.6"), favorite: true, ..Default::default() });
            let live = add(c, Row { name: "IMG_2.heic", media_type: crate::assets::MEDIA_TYPE_LIVE, hdr: 1, ..Default::default() });
            let raw = add(c, Row { name: "DSC_1.NEF", size: (6000, 4000), device: None, ..Default::default() });
            add(c, Row { name: "IMG_3.png", size: (1170, 2532), device: None, ..Default::default() }); // phone screenshot
            let shot = add(c, Row { name: "Screenshot 2026-09-19 101500.png", size: (1920, 1080), ..Default::default() });
            let pano = add(c, Row { name: "IMG_4.jpg", size: (12000, 3000), ..Default::default() });
            let selfie = add(c, Row { name: "IMG_5.heic", size: (3024, 4032), lens: Some("iPhone 12 front camera 2.71mm f/2.2"), ..Default::default() });
            let video = add(c, Row { name: "IMG_6.mov", media_type: 1, size: (1920, 1080), ..Default::default() });
            add(c, Row { name: "DSC_2.NEF", device: None, trashed: true, ..Default::default() }); // never counted
            // An iPhone PNG export of a photo keeps its camera, so it is not a screenshot.
            add(c, Row { name: "IMG_8.png", ..Default::default() });

            let all = list_collections(c)?;
            let get = |id: CollectionId| {
                let x = all.iter().find(|x| x.id == id).unwrap();
                (x.count, x.cover_asset_id)
            };
            assert_eq!(get(CollectionId::Favorites), (1, Some(fav)));
            assert_eq!(get(CollectionId::Videos), (1, Some(video)));
            assert_eq!(get(CollectionId::Live), (1, Some(live)));
            assert_eq!(get(CollectionId::Raw), (1, Some(raw)));
            assert_eq!(get(CollectionId::Hdr), (1, Some(live)));
            assert_eq!(get(CollectionId::Screenshots), (2, Some(shot)));
            assert_eq!(get(CollectionId::Panoramas), (1, Some(pano)));
            assert_eq!(get(CollectionId::Selfies), (1, Some(selfie)));
            Ok(())
        })
        .unwrap();
    }
}
