//! Colour labels and flags (Phase 8): the marks a shoot is culled with before it is
//! developed. Labels are Lightroom's five colours; flags are Pick, Reject or none.

use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use ts_rs::TS;

use crate::Result;

/// 0 none, then red, yellow, green, blue, purple — the order Lightroom numbers them.
pub const LABEL_NAMES: [&str; 6] = ["", "Red", "Yellow", "Green", "Blue", "Purple"];
pub const FLAG_REJECT: i64 = -1;
pub const FLAG_NONE: i64 = 0;
pub const FLAG_PICK: i64 = 1;

/// What a mark was before a change, so a change to many photos can be undone exactly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PreviousMark {
    #[ts(type = "number")]
    pub id: i64,
    #[ts(type = "number")]
    pub previous: i64,
}

fn set_mark(conn: &Connection, column: &str, ids: &[i64], value: i64) -> Result<Vec<PreviousMark>> {
    let mut read = conn.prepare_cached(&format!("SELECT {column} FROM assets WHERE id = ?1"))?;
    let mut write = conn.prepare_cached(&format!("UPDATE assets SET {column} = ?2 WHERE id = ?1"))?;
    let mut changed = Vec::new();
    for &id in ids {
        let Some(previous) = read.query_row([id], |r| r.get::<_, i64>(0)).optional()? else { continue };
        if previous != value {
            write.execute(params![id, value])?;
            changed.push(PreviousMark { id, previous });
        }
    }
    Ok(changed)
}

/// Sets `label` (clamped to 0–5) on `ids`; returns the ones that changed, with what they were.
pub fn set_color_label(conn: &Connection, ids: &[i64], label: i64) -> Result<Vec<PreviousMark>> {
    set_mark(conn, "color_label", ids, label.clamp(0, 5))
}

/// Sets `flag` (clamped to −1..1) on `ids`; returns the ones that changed, with what they were.
pub fn set_flag(conn: &Connection, ids: &[i64], flag: i64) -> Result<Vec<PreviousMark>> {
    set_mark(conn, "flag", ids, flag.clamp(FLAG_REJECT, FLAG_PICK))
}

/// The XMP name of a label (`xmp:Label`), as Lightroom and Bridge write it.
pub fn label_name(label: i64) -> Option<&'static str> {
    LABEL_NAMES.get(label.clamp(0, 5) as usize).copied().filter(|n| !n.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::query::AssetQuery;
    use crate::scope::LayoutScope;
    use crate::Db;

    fn library() -> Db {
        let db = Db::open_in_memory().unwrap();
        db.with_conn(|c| {
            for i in 1..=4 {
                c.execute(
                    "INSERT INTO assets (id, content_hash, relative_path, filename, extension, media_type, byte_size, orientation, imported_at)
                     VALUES (?1, ?2, 'p', 'f.jpg', 'jpg', 0, 1, 1, 10)",
                    params![i, format!("h{i}")],
                )?;
            }
            Ok(())
        })
        .unwrap();
        db
    }

    #[test]
    fn labels_and_flags_are_set_clamped_and_counted() {
        let db = library();
        db.with_conn(|c| {
            assert_eq!(set_color_label(c, &[1, 2], 3)?.len(), 2);
            assert!(set_color_label(c, &[1], 3)?.is_empty(), "already green");
            assert_eq!(set_color_label(c, &[3, 99], 9)?, vec![PreviousMark { id: 3, previous: 0 }], "a missing id is skipped");
            assert_eq!(set_color_label(c, &[1, 3], 0)?, vec![PreviousMark { id: 1, previous: 3 }, PreviousMark { id: 3, previous: 5 }]);
            set_color_label(c, &[3], 9)?;
            let l3: i64 = c.query_row("SELECT color_label FROM assets WHERE id = 3", [], |r| r.get(0))?;
            assert_eq!(l3, 5, "clamped to purple");
            set_flag(c, &[1, 4], FLAG_PICK)?;
            set_flag(c, &[2], -7)?;
            let f2: i64 = c.query_row("SELECT flag FROM assets WHERE id = 2", [], |r| r.get(0))?;
            assert_eq!(f2, FLAG_REJECT);
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn the_query_filters_by_label_and_flag() {
        let db = library();
        db.with_conn(|c| {
            set_color_label(c, &[1, 2], 3)?;
            set_color_label(c, &[3], 1)?;
            set_flag(c, &[1, 3], FLAG_PICK)?;
            let count = |q: AssetQuery| crate::layout::count_scoped(c, &LayoutScope { query: Some(q), ..Default::default() });
            assert_eq!(count(AssetQuery { color_labels: vec![3], ..Default::default() })?, 2);
            assert_eq!(count(AssetQuery { color_labels: vec![1, 3], ..Default::default() })?, 3, "any of the labels");
            assert_eq!(count(AssetQuery { flag: Some(FLAG_PICK as i8), ..Default::default() })?, 2);
            assert_eq!(count(AssetQuery { flag: Some(FLAG_PICK as i8), color_labels: vec![3], ..Default::default() })?, 1);
            assert_eq!(count(AssetQuery { flag: Some(FLAG_NONE as i8), ..Default::default() })?, 2);
            assert_eq!(label_name(3), Some("Green"));
            assert_eq!(label_name(0), None);
            Ok(())
        })
        .unwrap();
    }
}
