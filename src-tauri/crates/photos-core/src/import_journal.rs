//! Files placed in `originals/` whose catalogue row is not written yet (round 3). An import
//! records a file here before moving it into place and clears the record in the transaction
//! that inserts its row, so a crash in between leaves a record naming exactly the file Lumen
//! placed. `photos_import::recover` settles them when the library next opens.

use rusqlite::{params, Connection};

use crate::Result;

#[derive(Debug, Clone, PartialEq)]
pub struct Placement {
    pub id: i64,
    pub content_hash: String,
    pub relative_path: String,
}

pub fn record(conn: &Connection, batch_id: i64, content_hash: &str, relative_path: &str, now: i64) -> Result<i64> {
    conn.execute(
        "INSERT INTO import_journal (batch_id, content_hash, relative_path, placed_at) VALUES (?1, ?2, ?3, ?4)",
        params![batch_id, content_hash, relative_path, now],
    )?;
    Ok(conn.last_insert_rowid())
}

/// Forget placements whose rows now exist (or that were settled).
pub fn clear(conn: &Connection, ids: &[i64]) -> Result<()> {
    let mut stmt = conn.prepare_cached("DELETE FROM import_journal WHERE id = ?1")?;
    for id in ids {
        stmt.execute([id])?;
    }
    Ok(())
}

pub fn list(conn: &Connection) -> Result<Vec<Placement>> {
    let mut stmt = conn.prepare("SELECT id, content_hash, relative_path FROM import_journal ORDER BY id")?;
    let rows = stmt
        .query_map([], |r| Ok(Placement { id: r.get(0)?, content_hash: r.get(1)?, relative_path: r.get(2)? }))?
        .collect::<std::result::Result<_, _>>()?;
    Ok(rows)
}
