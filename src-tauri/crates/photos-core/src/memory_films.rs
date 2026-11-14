//! Memory films (Round 4F, Task 5; Round 3 Task 17): a memory played as a slideshow with its
//! music, transitions and cuts on the beat, built once and kept until the memory's photos
//! change. The table is migration 0019's `memory_films`.

use rusqlite::{params, Connection, OptionalExtension};

use crate::slideshow::SlideshowDoc;
use crate::Result;

/// A film as it was stored.
#[derive(Debug, Clone, PartialEq)]
pub struct StoredFilm {
    pub doc: SlideshowDoc,
    pub bpm: Option<u32>,
    pub built_at: i64,
}

/// What a film was made from: the memory's curated photos, in their order. A memory
/// regenerated with other photos (a new import, a photo hidden) has another key.
pub fn source_key(curated: &[i64]) -> String {
    curated.iter().map(i64::to_string).collect::<Vec<_>>().join(",")
}

pub fn put(conn: &Connection, memory_id: i64, key: &str, doc: &SlideshowDoc, bpm: Option<u32>, now: i64) -> Result<()> {
    conn.execute(
        "INSERT INTO memory_films (memory_id, source_key, doc_json, bpm, built_at) VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(memory_id) DO UPDATE SET source_key = excluded.source_key, doc_json = excluded.doc_json, bpm = excluded.bpm, built_at = excluded.built_at",
        params![memory_id, key, serde_json::to_string(doc).unwrap_or_default(), bpm, now],
    )?;
    Ok(())
}

/// The stored film, when it was made from exactly these photos (`source_key`).
pub fn get_fresh(conn: &Connection, memory_id: i64, key: &str) -> Result<Option<StoredFilm>> {
    let row: Option<(String, String, Option<u32>, i64)> = conn
        .query_row("SELECT source_key, doc_json, bpm, built_at FROM memory_films WHERE memory_id = ?1", [memory_id], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
        })
        .optional()?;
    Ok(row.and_then(|(stored, json, bpm, built_at)| {
        if stored != key {
            return None;
        }
        let doc: SlideshowDoc = serde_json::from_str(&json).ok()?;
        Some(StoredFilm { doc, bpm, built_at })
    }))
}

/// How many memories the background step prepares (Round 21 §14): the ones the Memories page
/// shows first.
pub const PREPARE_NEWEST: usize = 12;

/// The ones among the first `limit` visible memories (the page's order) that have no film made
/// from their present photos, with the key a film made now would carry. A memory with no
/// curated photos has nothing to play and is left out. Running a step on this list and then
/// asking again gives an empty list, which is what makes the step safe to run at any time.
pub fn missing(conn: &Connection, limit: usize) -> Result<Vec<(i64, String)>> {
    let mut out = Vec::new();
    for memory in crate::memories::list_memories(conn, false)?.into_iter().take(limit) {
        let curated = crate::memories::curated_ids(conn, memory.id)?;
        if curated.is_empty() {
            continue;
        }
        let key = source_key(&curated);
        if get_fresh(conn, memory.id, &key)?.is_none() {
            out.push((memory.id, key));
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Db;

    #[test]
    fn only_memories_without_a_fresh_film_are_missing_and_a_second_pass_finds_none() {
        let db = Db::open_in_memory().unwrap();
        db.with_conn(|c| {
            // Three memories; the second has no curated photos; the third already has a film.
            c.execute_batch(
                "INSERT INTO memories (id, kind, title, start_at, end_at, generated_at) VALUES
                   (1, 1, 'A', 300, 310, 0), (2, 1, 'B', 200, 210, 0), (3, 1, 'C', 100, 110, 0);",
            )?;
            for asset in [11, 12, 31] {
                c.execute(
                    "INSERT INTO assets (id, content_hash, relative_path, filename, extension, media_type, byte_size, orientation, imported_at)
                     VALUES (?1, ?2, 'p', 'f.jpg', 'jpg', 0, 1, 1, 1)",
                    params![asset, format!("h{asset}")],
                )?;
            }
            for (memory, asset) in [(1, 11), (1, 12), (3, 31)] {
                c.execute("INSERT INTO memory_items (memory_id, asset_id, curated, score) VALUES (?1, ?2, 1, 1.0)", params![memory, asset])?;
            }
            put(c, 3, &source_key(&[31]), &SlideshowDoc::new_for(&[31]), None, 1)?;

            let first = missing(c, PREPARE_NEWEST)?;
            assert_eq!(first.iter().map(|m| m.0).collect::<Vec<_>>(), vec![1], "2 has nothing to play, 3 has its film");
            for (id, key) in &first {
                put(c, *id, key, &SlideshowDoc::new_for(&[11, 12]), None, 2)?;
            }
            assert!(missing(c, PREPARE_NEWEST)?.is_empty(), "the second run builds nothing");
            // New photos in a memory make its film stale again.
            c.execute("INSERT INTO memory_items (memory_id, asset_id, curated, score) VALUES (1, 31, 1, 0.5)", [])?;
            assert_eq!(missing(c, PREPARE_NEWEST)?.len(), 1);
            // Only the newest `limit` are looked at.
            assert!(missing(c, 0)?.is_empty());
            Ok(())
        })
        .unwrap();
    }

    fn library() -> Db {
        let db = Db::open_in_memory().unwrap();
        db.with_conn(|c| {
            c.execute_batch(
                "INSERT INTO memories (id, kind, title, start_at, end_at, generated_at) VALUES (7, 1, 'Lisbon', 0, 10, 0);",
            )?;
            Ok(())
        })
        .unwrap();
        db
    }

    #[test]
    fn a_film_comes_back_as_it_was_stored() {
        let db = library();
        db.with_conn(|c| {
            let doc = SlideshowDoc::new_for(&[1, 2, 3]);
            put(c, 7, &source_key(&[1, 2, 3]), &doc, Some(92), 100)?;
            let got = get_fresh(c, 7, &source_key(&[1, 2, 3]))?.unwrap();
            assert_eq!(got, StoredFilm { doc: doc.clone(), bpm: Some(92), built_at: 100 });
            // Built again: replaced, not added.
            put(c, 7, &source_key(&[1, 2, 3]), &doc, None, 200)?;
            assert_eq!(get_fresh(c, 7, &source_key(&[1, 2, 3]))?.unwrap().built_at, 200);
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn a_film_of_other_photos_is_stale() {
        let db = library();
        db.with_conn(|c| {
            put(c, 7, &source_key(&[1, 2, 3]), &SlideshowDoc::new_for(&[1, 2, 3]), None, 1)?;
            assert!(get_fresh(c, 7, &source_key(&[1, 2, 4]))?.is_none());
            assert!(get_fresh(c, 7, &source_key(&[2, 1, 3]))?.is_none(), "the order is part of it");
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn a_film_goes_with_its_memory() {
        let db = library();
        db.with_conn(|c| {
            put(c, 7, "1", &SlideshowDoc::new_for(&[1]), None, 1)?;
            c.execute("DELETE FROM memories WHERE id = 7", [])?;
            let left: i64 = c.query_row("SELECT COUNT(*) FROM memory_films", [], |r| r.get(0))?;
            assert_eq!(left, 0);
            Ok(())
        })
        .unwrap();
    }
}
