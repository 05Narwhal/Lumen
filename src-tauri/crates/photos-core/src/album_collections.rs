//! Collections (Round 5A): folders of albums. An album is in at most one collection
//! (`albums.collection_id`), and collections nest (`album_collections.parent_id`). Not the
//! built-in kinds of [`crate::collections`] (Favorites, Videos, …), which the UI calls
//! *Media types*. Deleting a collection never deletes what it holds: migration 0026's
//! trigger moves its albums and collections up to its parent.

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::albums::CoverStyle;
use crate::{CoreError, Result};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AlbumCollection {
    #[ts(type = "number")]
    pub id: i64,
    pub name: String,
    #[ts(type = "number | null")]
    pub parent_id: Option<i64>,
    #[ts(type = "number")]
    pub created_at: i64,
    #[ts(type = "number")]
    pub updated_at: i64,
    /// A shape with an icon instead of its albums' covers (migration 0034).
    #[serde(default)]
    pub cover_style: Option<CoverStyle>,
}

const COLUMNS: &str = "id, name, parent_id, created_at, updated_at, cover_style";

fn row(r: &rusqlite::Row) -> rusqlite::Result<AlbumCollection> {
    Ok(AlbumCollection {
        id: r.get(0)?,
        name: r.get(1)?,
        parent_id: r.get(2)?,
        created_at: r.get(3)?,
        updated_at: r.get(4)?,
        cover_style: CoverStyle::from_json(r.get(5)?),
    })
}

/// What Undo needs after a collection is deleted: the collection and what it held.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct CollectionUndo {
    pub collection: AlbumCollection,
    #[ts(type = "Array<number>")]
    pub albums: Vec<i64>,
    #[ts(type = "Array<number>")]
    pub collections: Vec<i64>,
}

fn clean(name: &str) -> Result<&str> {
    let name = name.trim();
    if name.is_empty() {
        return Err(CoreError::Invalid("A collection needs a name.".into()));
    }
    Ok(name)
}

fn exists(conn: &Connection, id: i64) -> Result<bool> {
    Ok(conn.query_row("SELECT 1 FROM album_collections WHERE id = ?1", [id], |_| Ok(())).optional()?.is_some())
}

fn require(conn: &Connection, id: i64) -> Result<()> {
    if exists(conn, id)? {
        Ok(())
    } else {
        Err(CoreError::NotFound(format!("collection {id}")))
    }
}

pub fn create(conn: &Connection, name: &str, parent: Option<i64>, now: i64) -> Result<i64> {
    let name = clean(name)?;
    if let Some(p) = parent {
        require(conn, p)?;
    }
    conn.execute("INSERT INTO album_collections (name, parent_id, created_at, updated_at) VALUES (?1, ?2, ?3, ?3)", params![name, parent, now])?;
    Ok(conn.last_insert_rowid())
}

pub fn rename(conn: &Connection, id: i64, name: &str, now: i64) -> Result<()> {
    let name = clean(name)?;
    if conn.execute("UPDATE album_collections SET name = ?2, updated_at = ?3 WHERE id = ?1", params![id, name, now])? == 0 {
        return Err(CoreError::NotFound(format!("collection {id}")));
    }
    Ok(())
}

/// An icon cover for a collection, or `None` for its albums' covers again.
pub fn set_cover_style(conn: &Connection, id: i64, style: Option<&CoverStyle>) -> Result<()> {
    let json = CoverStyle::to_json(style)?;
    if conn.execute("UPDATE album_collections SET cover_style = ?2 WHERE id = ?1", params![id, json])? == 0 {
        return Err(CoreError::NotFound(format!("collection {id}")));
    }
    Ok(())
}

/// Every collection, by name.
pub fn list(conn: &Connection) -> Result<Vec<AlbumCollection>> {
    let mut stmt = conn.prepare(&format!("SELECT {COLUMNS} FROM album_collections ORDER BY name COLLATE NOCASE, id"))?;
    let rows = stmt.query_map([], row)?;
    Ok(rows.collect::<std::result::Result<_, _>>()?)
}

/// Puts `album` in `into` (or in no collection). Returns where it was, for Undo.
pub fn move_album(conn: &Connection, album: i64, into: Option<i64>, now: i64) -> Result<Option<i64>> {
    if let Some(c) = into {
        require(conn, c)?;
    }
    let was: Option<i64> = conn
        .query_row("SELECT collection_id FROM albums WHERE id = ?1", [album], |r| r.get(0))
        .optional()?
        .ok_or_else(|| CoreError::NotFound(format!("album {album}")))?;
    conn.execute("UPDATE albums SET collection_id = ?2, updated_at = ?3 WHERE id = ?1", params![album, into, now])?;
    Ok(was)
}

/// Puts collection `id` inside `into` (or at the top). A collection never goes inside itself
/// or one of its own. Returns where it was, for Undo.
pub fn move_collection(conn: &Connection, id: i64, into: Option<i64>, now: i64) -> Result<Option<i64>> {
    let was: Option<i64> = conn
        .query_row("SELECT parent_id FROM album_collections WHERE id = ?1", [id], |r| r.get(0))
        .optional()?
        .ok_or_else(|| CoreError::NotFound(format!("collection {id}")))?;
    if let Some(target) = into {
        require(conn, target)?;
        // Walk up from the target: meeting `id` means it would contain itself.
        let mut at = Some(target);
        while let Some(c) = at {
            if c == id {
                return Err(CoreError::Invalid("A collection cannot go inside itself.".into()));
            }
            at = conn.query_row("SELECT parent_id FROM album_collections WHERE id = ?1", [c], |r| r.get(0))?;
        }
    }
    conn.execute("UPDATE album_collections SET parent_id = ?2, updated_at = ?3 WHERE id = ?1", params![id, into, now])?;
    Ok(was)
}

/// Deletes collection `id`; what it held moves up to its parent.
pub fn delete(conn: &Connection, id: i64) -> Result<CollectionUndo> {
    let collection = conn
        .query_row(&format!("SELECT {COLUMNS} FROM album_collections WHERE id = ?1"), [id], row)
        .optional()?
        .ok_or_else(|| CoreError::NotFound(format!("collection {id}")))?;
    let ids = |sql: &str| -> Result<Vec<i64>> { Ok(conn.prepare(sql)?.query_map([id], |r| r.get(0))?.collect::<std::result::Result<_, _>>()?) };
    let albums = ids("SELECT id FROM albums WHERE collection_id = ?1 ORDER BY id")?;
    let collections = ids("SELECT id FROM album_collections WHERE parent_id = ?1 ORDER BY id")?;
    conn.execute("DELETE FROM album_collections WHERE id = ?1", [id])?;
    Ok(CollectionUndo { collection, albums, collections })
}

/// Undoes [`delete`]: the collection comes back with its id, holding what it held.
pub fn restore(conn: &Connection, undo: &CollectionUndo) -> Result<()> {
    let c = &undo.collection;
    let parent = match c.parent_id {
        Some(p) if exists(conn, p)? => Some(p),
        _ => None,
    };
    conn.execute(
        "INSERT INTO album_collections (id, name, parent_id, created_at, updated_at, cover_style) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![c.id, c.name, parent, c.created_at, c.updated_at, CoverStyle::to_json(c.cover_style.as_ref())?],
    )?;
    for a in &undo.albums {
        conn.execute("UPDATE albums SET collection_id = ?2 WHERE id = ?1", params![a, c.id])?;
    }
    for k in &undo.collections {
        conn.execute("UPDATE album_collections SET parent_id = ?2 WHERE id = ?1", params![k, c.id])?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::albums::{create_album, list_albums};
    use crate::db::Db;

    fn collection_of(c: &Connection, album: i64) -> Option<i64> {
        c.query_row("SELECT collection_id FROM albums WHERE id = ?1", [album], |r| r.get(0)).unwrap()
    }
    fn parent_of(c: &Connection, id: i64) -> Option<i64> {
        c.query_row("SELECT parent_id FROM album_collections WHERE id = ?1", [id], |r| r.get(0)).unwrap()
    }

    #[test]
    fn albums_go_into_collections_and_collections_nest() {
        let db = Db::open_in_memory().unwrap();
        db.with_conn(|c| {
            let trips = create(c, " Trips ", None, 1)?;
            let italy = create(c, "Italy", Some(trips), 1)?;
            let rome = create_album(c, "Rome", None, 1)?;
            assert_eq!(move_album(c, rome, Some(italy), 2)?, None);
            assert_eq!(collection_of(c, rome), Some(italy));
            let names: Vec<String> = list(c)?.into_iter().map(|k| k.name).collect();
            assert_eq!(names, ["Italy", "Trips"], "by name, trimmed");
            assert_eq!(list_albums(c)?[0].collection_id, Some(italy));
            // Out again, and Undo knows where it was.
            assert_eq!(move_album(c, rome, None, 3)?, Some(italy));
            assert!(matches!(move_album(c, rome, Some(999), 3), Err(CoreError::NotFound(_))));
            assert!(matches!(create(c, "  ", None, 1), Err(CoreError::Invalid(_))));
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn a_collection_never_goes_inside_itself() {
        let db = Db::open_in_memory().unwrap();
        db.with_conn(|c| {
            let a = create(c, "A", None, 1)?;
            let b = create(c, "B", Some(a), 1)?;
            let d = create(c, "D", Some(b), 1)?;
            assert!(matches!(move_collection(c, a, Some(a), 2), Err(CoreError::Invalid(_))));
            assert!(matches!(move_collection(c, a, Some(d), 2), Err(CoreError::Invalid(_))));
            assert_eq!(parent_of(c, a), None, "a refused move changes nothing");
            assert_eq!(move_collection(c, d, None, 2)?, Some(b));
            assert_eq!(move_collection(c, a, Some(d), 2)?, None, "no longer its own");
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn deleting_a_collection_keeps_its_albums_and_undo_puts_them_back() {
        let db = Db::open_in_memory().unwrap();
        db.with_conn(|c| {
            let top = create(c, "Top", None, 1)?;
            let mid = create(c, "Mid", Some(top), 1)?;
            let inner = create(c, "Inner", Some(mid), 1)?;
            let album = create_album(c, "Rome", None, 1)?;
            move_album(c, album, Some(mid), 1)?;

            let undo = delete(c, mid)?;
            assert_eq!(list_albums(c)?.len(), 1, "the album is kept");
            assert_eq!(collection_of(c, album), Some(top), "and moves up");
            assert_eq!(parent_of(c, inner), Some(top));
            assert_eq!((undo.albums.clone(), undo.collections.clone()), (vec![album], vec![inner]));

            restore(c, &undo)?;
            assert_eq!(collection_of(c, album), Some(mid));
            assert_eq!(parent_of(c, inner), Some(mid));
            assert_eq!(parent_of(c, mid), Some(top));
            assert!(list(c)?.iter().any(|k| k.id == mid && k.name == "Mid"));

            // A collection with no parent left comes back at the top.
            let undo = delete(c, mid)?;
            delete(c, top)?;
            restore(c, &undo)?;
            assert_eq!(parent_of(c, mid), None);
            Ok(())
        })
        .unwrap();
    }

    /// Item 3: a collection can show an icon cover; deleting and undoing keeps it.
    #[test]
    fn a_collection_keeps_its_icon_cover_through_undo() {
        use crate::albums::{CoverShape, CoverStyle};
        let db = Db::open_in_memory().unwrap();
        db.with_conn(|c| {
            let id = create(c, "Trips", None, 1)?;
            let style = CoverStyle::Icon { shape: CoverShape::Binder, icon: "map".into(), color: "#ff8800".into() };
            set_cover_style(c, id, Some(&style))?;
            assert_eq!(list(c)?[0].cover_style, Some(style.clone()));
            assert!(set_cover_style(c, id, Some(&CoverStyle::Icon { shape: CoverShape::Box, icon: "map".into(), color: "blue".into() })).is_err());
            let undo = delete(c, id)?;
            restore(c, &undo)?;
            assert_eq!(list(c)?[0].cover_style, Some(style));
            set_cover_style(c, id, None)?;
            assert_eq!(list(c)?[0].cover_style, None);
            assert!(set_cover_style(c, 999, None).is_err());
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn deleting_an_album_leaves_its_collection() {
        let db = Db::open_in_memory().unwrap();
        db.with_conn(|c| {
            let k = create(c, "K", None, 1)?;
            let album = create_album(c, "Rome", None, 1)?;
            move_album(c, album, Some(k), 1)?;
            let copy = crate::albums::duplicate_album(c, album, 2)?;
            assert_eq!(collection_of(c, copy), Some(k), "a copy stays beside its album");
            crate::albums::delete_album(c, album)?;
            assert_eq!(list(c)?.len(), 1);
            Ok(())
        })
        .unwrap();
    }
}
