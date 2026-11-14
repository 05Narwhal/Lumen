//! A photo's develop history (spec §2): every committed change as a step, capped at 200, and
//! named snapshots, which are never dropped. Rows are recipes, like the edit itself.

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::edits::Recipe;
use crate::error::CoreError;
use crate::Result;

/// Ordinary steps kept per photo; the oldest go first. Snapshots do not count.
pub const HISTORY_CAP: i64 = 200;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct HistoryStep {
    #[ts(type = "number")]
    pub id: i64,
    /// What the step did, in words ("Exposure +0.30").
    pub label: String,
    pub recipe: Recipe,
    /// Set for a snapshot.
    pub snapshot_name: Option<String>,
    #[ts(type = "number")]
    pub created_at: i64,
}

pub fn push_step(conn: &Connection, asset_id: i64, recipe: &Recipe, label: &str, now_ms: i64) -> Result<i64> {
    conn.execute(
        "INSERT INTO edit_history (asset_id, recipe_json, label, snapshot_name, created_at) VALUES (?1, ?2, ?3, NULL, ?4)",
        params![asset_id, serde_json::to_string(&recipe.clone().normalized())?, label, now_ms],
    )?;
    let id = conn.last_insert_rowid();
    conn.execute(
        "DELETE FROM edit_history WHERE asset_id = ?1 AND snapshot_name IS NULL AND id NOT IN
           (SELECT id FROM edit_history WHERE asset_id = ?1 AND snapshot_name IS NULL ORDER BY id DESC LIMIT ?2)",
        params![asset_id, HISTORY_CAP],
    )?;
    Ok(id)
}

/// Steps and snapshots, oldest first.
pub fn list_history(conn: &Connection, asset_id: i64) -> Result<Vec<HistoryStep>> {
    let mut stmt = conn.prepare("SELECT id, label, recipe_json, snapshot_name, created_at FROM edit_history WHERE asset_id = ?1 ORDER BY id ASC")?;
    let rows = stmt.query_map([asset_id], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?, r.get::<_, Option<String>>(3)?, r.get::<_, i64>(4)?)))?;
    let mut out = Vec::new();
    for row in rows {
        let (id, label, json, snapshot_name, created_at) = row?;
        out.push(HistoryStep { id, label, recipe: serde_json::from_str(&json)?, snapshot_name, created_at });
    }
    Ok(out)
}

/// Going back to step `step_id` and then changing something discards the steps after it.
/// Snapshots stay.
pub fn truncate_after(conn: &Connection, asset_id: i64, step_id: i64) -> Result<usize> {
    Ok(conn.execute("DELETE FROM edit_history WHERE asset_id = ?1 AND snapshot_name IS NULL AND id > ?2", params![asset_id, step_id])?)
}

/// Forget every ordinary step (snapshots stay).
pub fn clear_history(conn: &Connection, asset_id: i64) -> Result<usize> {
    Ok(conn.execute("DELETE FROM edit_history WHERE asset_id = ?1 AND snapshot_name IS NULL", [asset_id])?)
}

pub fn save_snapshot(conn: &Connection, asset_id: i64, name: &str, recipe: &Recipe, now_ms: i64) -> Result<i64> {
    let name = name.trim();
    if name.is_empty() {
        return Err(CoreError::Invalid("a snapshot needs a name".into()));
    }
    conn.execute(
        "INSERT INTO edit_history (asset_id, recipe_json, label, snapshot_name, created_at) VALUES (?1, ?2, ?3, ?3, ?4)",
        params![asset_id, serde_json::to_string(&recipe.clone().normalized())?, name, now_ms],
    )?;
    Ok(conn.last_insert_rowid())
}

fn snapshot_exists(conn: &Connection, id: i64) -> Result<()> {
    let found: Option<i64> = conn.query_row("SELECT id FROM edit_history WHERE id = ?1 AND snapshot_name IS NOT NULL", [id], |r| r.get(0)).optional()?;
    found.map(|_| ()).ok_or_else(|| CoreError::NotFound(format!("snapshot {id}")))
}

pub fn rename_snapshot(conn: &Connection, id: i64, name: &str) -> Result<()> {
    snapshot_exists(conn, id)?;
    let name = name.trim();
    if name.is_empty() {
        return Err(CoreError::Invalid("a snapshot needs a name".into()));
    }
    conn.execute("UPDATE edit_history SET snapshot_name = ?2, label = ?2 WHERE id = ?1", params![id, name])?;
    Ok(())
}

pub fn delete_snapshot(conn: &Connection, id: i64) -> Result<()> {
    snapshot_exists(conn, id)?;
    conn.execute("DELETE FROM edit_history WHERE id = ?1", [id])?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::develop::tests::library;

    fn r(rotate: u16) -> Recipe {
        Recipe { rotate, ..Recipe::identity() }
    }

    #[test]
    fn history_keeps_the_newest_two_hundred_steps_and_every_snapshot() {
        let db = library();
        db.with_conn(|c| {
            save_snapshot(c, 1, "Start", &r(0), 0)?;
            for i in 0..210 {
                push_step(c, 1, &r(90 * (i % 4) as u16), &format!("step {i}"), i)?;
            }
            let h = list_history(c, 1)?;
            let steps: Vec<&HistoryStep> = h.iter().filter(|s| s.snapshot_name.is_none()).collect();
            assert_eq!(steps.len(), 200);
            assert_eq!(steps[0].label, "step 10", "the oldest went first");
            assert_eq!(h.iter().filter(|s| s.snapshot_name.is_some()).count(), 1);
            assert!(list_history(c, 2)?.is_empty(), "per photo");
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn going_back_then_changing_drops_the_newer_steps_but_not_snapshots() {
        let db = library();
        db.with_conn(|c| {
            let a = push_step(c, 1, &r(90), "Rotate", 1)?;
            push_step(c, 1, &r(180), "Rotate", 2)?;
            save_snapshot(c, 1, "Upside down", &r(180), 3)?;
            push_step(c, 1, &r(270), "Rotate", 4)?;
            assert_eq!(truncate_after(c, 1, a)?, 2);
            let h = list_history(c, 1)?;
            assert_eq!(h.len(), 2);
            assert_eq!(h[1].snapshot_name.as_deref(), Some("Upside down"));
            assert_eq!(h[0].recipe, r(90).normalized());
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn snapshots_are_named_renamed_and_deleted() {
        let db = library();
        db.with_conn(|c| {
            assert!(matches!(save_snapshot(c, 1, "  ", &r(0), 0), Err(CoreError::Invalid(_))));
            let id = save_snapshot(c, 1, "Warm", &r(0), 0)?;
            rename_snapshot(c, id, "Warmer")?;
            assert_eq!(list_history(c, 1)?[0].snapshot_name.as_deref(), Some("Warmer"));
            let step = push_step(c, 1, &r(90), "Rotate", 1)?;
            assert!(matches!(delete_snapshot(c, step), Err(CoreError::NotFound(_))), "a step is not a snapshot");
            delete_snapshot(c, id)?;
            clear_history(c, 1)?;
            assert!(list_history(c, 1)?.is_empty());
            Ok(())
        })
        .unwrap();
    }
}
