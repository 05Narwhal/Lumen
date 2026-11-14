//! Develop's filmstrip (spec §1): the photos of a scope — narrowed by the top bar's flag,
//! label and rating filters, which ride in the scope's query — in the order the top bar
//! chooses, a page at a time by keyset.

use rusqlite::types::Value;
use rusqlite::{params_from_iter, Connection};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::query::AssetQuery;
use crate::scope::LayoutScope;
use crate::Result;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export)]
pub enum StripSort {
    /// Capture time, oldest first: a shoot reads left to right.
    #[default]
    Captured,
    /// Highest rated first, then by capture time.
    Rating,
    Filename,
}

/// Where the next page starts: the last item's sort values.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct StripCursor {
    #[ts(type = "number")]
    pub time: i64,
    #[ts(type = "number")]
    pub rating: i64,
    pub filename: String,
    #[ts(type = "number")]
    pub id: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct StripItem {
    #[ts(type = "number")]
    pub id: i64,
    pub filename: String,
    /// −1 rejected, 0 none, 1 picked.
    #[ts(type = "number")]
    pub flag: i64,
    #[ts(type = "number")]
    pub color_label: i64,
    #[ts(type = "number")]
    pub rating: i64,
    /// Has a saved recipe.
    pub edited: bool,
    /// Display-oriented.
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct StripPage {
    pub items: Vec<StripItem>,
    pub next_cursor: Option<StripCursor>,
}

const TIME: &str = "COALESCE(a.captured_at, a.imported_at)";

fn order(sort: StripSort) -> String {
    match sort {
        StripSort::Captured => format!("{TIME} ASC, a.id ASC"),
        StripSort::Rating => format!("a.rating DESC, {TIME} ASC, a.id ASC"),
        StripSort::Filename => "a.filename COLLATE NOCASE ASC, a.id ASC".into(),
    }
}

/// The keyset condition: strictly after `c` in `sort`'s order.
fn after(sort: StripSort, c: &StripCursor, params: &mut Vec<Value>) -> String {
    match sort {
        StripSort::Captured => {
            params.extend([Value::Integer(c.time), Value::Integer(c.time), Value::Integer(c.id)]);
            format!("({TIME} > ? OR ({TIME} = ? AND a.id > ?))")
        }
        StripSort::Rating => {
            params.extend([c.rating, c.rating, c.time, c.time, c.id].map(Value::Integer));
            format!("(a.rating < ? OR (a.rating = ? AND ({TIME} > ? OR ({TIME} = ? AND a.id > ?))))")
        }
        StripSort::Filename => {
            params.extend([Value::Text(c.filename.clone()), Value::Text(c.filename.clone()), Value::Integer(c.id)]);
            "(a.filename > ? COLLATE NOCASE OR (a.filename = ? COLLATE NOCASE AND a.id > ?))".into()
        }
    }
}

/// The scope the strip reads: photos only, and when the user picked the photos, every one of
/// them — a RAW shot beside a JPEG included, though the library may hide such pairs, or
/// the RAWs they chose to develop would silently vanish from the strip.
fn strip_scope(scope: &LayoutScope, only: Option<&[i64]>) -> LayoutScope {
    let mut scope = LayoutScope { media_type: Some(0), ..scope.clone() };
    if only.is_some() {
        scope.query = Some(AssetQuery { include_raw_pairs: true, ..scope.query.unwrap_or_default() });
    }
    scope
}

/// `FROM … WHERE …` and its parameters: the scope's photos (never videos), and, when the
/// strip was opened on a selection, only those.
fn base_sql(scope: &LayoutScope, only: Option<&[i64]>) -> (String, Vec<Value>) {
    let (mut from_where, _, params) = strip_scope(scope, only).sql();
    if let Some(ids) = only {
        let list = if ids.is_empty() { "NULL".to_string() } else { ids.iter().map(|i| i.to_string()).collect::<Vec<_>>().join(",") };
        from_where.push_str(&format!(" AND a.id IN ({list})"));
    }
    (from_where, params)
}

pub fn strip_page(conn: &Connection, scope: &LayoutScope, only: Option<&[i64]>, sort: StripSort, cursor: Option<&StripCursor>, limit: u32) -> Result<StripPage> {
    let (mut from_where, mut params) = base_sql(scope, only);
    if let Some(c) = cursor {
        from_where.push_str(&format!(" AND {}", after(sort, c, &mut params)));
    }
    let limit = limit.clamp(1, 500);
    let sql = format!(
        "SELECT a.id, a.filename, a.flag, a.color_label, a.rating, (EXISTS (SELECT 1 FROM edits e WHERE e.asset_id = a.id) OR EXISTS (SELECT 1 FROM develop_drafts dd WHERE dd.asset_id = a.id)),
                a.width, a.height, a.orientation, {TIME}
         {from_where} ORDER BY {} LIMIT {}",
        order(sort),
        limit + 1
    );
    let mut stmt = conn.prepare(&sql)?;
    let mut rows: Vec<(StripItem, i64)> = stmt
        .query_map(params_from_iter(params.iter()), |r| {
            let (w, h): (Option<i64>, Option<i64>) = (r.get(6)?, r.get(7)?);
            let (w, h) = (w.unwrap_or(0) as u32, h.unwrap_or(0) as u32);
            let orientation: i64 = r.get(8)?;
            let (width, height) = if (5..=8).contains(&orientation) { (h, w) } else { (w, h) };
            Ok((
                StripItem { id: r.get(0)?, filename: r.get(1)?, flag: r.get(2)?, color_label: r.get(3)?, rating: r.get(4)?, edited: r.get(5)?, width, height },
                r.get::<_, i64>(9)?,
            ))
        })?
        .collect::<std::result::Result<_, _>>()?;
    let more = rows.len() > limit as usize;
    rows.truncate(limit as usize);
    let next_cursor = rows.last().filter(|_| more).map(|(it, time)| StripCursor { time: *time, rating: it.rating, filename: it.filename.clone(), id: it.id });
    Ok(StripPage { items: rows.into_iter().map(|(it, _)| it).collect(), next_cursor })
}

/// Every id in the strip, in order: for *Select all*, *Invert* and *Select picked*.
pub fn strip_ids(conn: &Connection, scope: &LayoutScope, only: Option<&[i64]>, sort: StripSort) -> Result<Vec<i64>> {
    let (from_where, params) = base_sql(scope, only);
    let mut stmt = conn.prepare(&format!("SELECT a.id {from_where} ORDER BY {}", order(sort)))?;
    let ids = stmt.query_map(params_from_iter(params.iter()), |r| r.get(0))?.collect::<std::result::Result<_, _>>()?;
    Ok(ids)
}

/// How many photos the strip holds.
pub fn strip_count(conn: &Connection, scope: &LayoutScope, only: Option<&[i64]>) -> Result<i64> {
    let (from_where, params) = base_sql(scope, only);
    Ok(conn.query_row(&format!("SELECT COUNT(*) {from_where}"), params_from_iter(params.iter()), |r| r.get(0))?)
}

/// Whether the strip holds photo `id`.
pub fn strip_has(conn: &Connection, scope: &LayoutScope, only: Option<&[i64]>, id: i64) -> Result<bool> {
    let (from_where, mut params) = base_sql(scope, only);
    params.push(Value::Integer(id));
    Ok(conn.query_row(&format!("SELECT EXISTS (SELECT 1 {from_where} AND a.id = ?)"), params_from_iter(params.iter()), |r| r.get(0))?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::develop::tests::library;
    use crate::query::AssetQuery;

    fn all_pages(c: &Connection, scope: &LayoutScope, only: Option<&[i64]>, sort: StripSort) -> Vec<i64> {
        let mut out = Vec::new();
        let mut cursor = None;
        loop {
            let page = strip_page(c, scope, only, sort, cursor.as_ref(), 1).unwrap();
            out.extend(page.items.iter().map(|i| i.id));
            match page.next_cursor {
                Some(n) => cursor = Some(n),
                None => break,
            }
        }
        out
    }

    #[test]
    fn a_picked_selection_shows_raws_the_library_hides_behind_their_jpeg() {
        let scope = LayoutScope { query: Some(AssetQuery { min_rating: Some(2), ..Default::default() }), ..LayoutScope::library() };
        let picked = strip_scope(&scope, Some(&[1, 2]));
        let q = picked.query.unwrap();
        assert!(q.include_raw_pairs);
        assert_eq!(q.min_rating, Some(2), "the top bar's filters still apply");
        assert!(!strip_scope(&scope, None).query.unwrap().include_raw_pairs, "a scope shows what its grid shows");
    }

    #[test]
    fn pages_cover_every_photo_once_in_each_order_and_skip_videos() {
        let db = library();
        db.with_conn(|c| {
            c.execute_batch("INSERT INTO assets (id, content_hash, relative_path, filename, extension, byte_size, media_type, imported_at, captured_at, rating, state) VALUES (5, 'h5', 'a/5.jpg', 'e.jpg', 'jpg', 1, 0, 0, 50, 5, 1)")?;
            let scope = LayoutScope::library();
            // Captured 300, 100, 200 → 2, 3, 1; ratings 2, 5, 5; names c, a, b.
            assert_eq!(all_pages(c, &scope, None, StripSort::Captured), [2, 3, 1]);
            assert_eq!(all_pages(c, &scope, None, StripSort::Rating), [2, 3, 1]);
            assert_eq!(all_pages(c, &scope, None, StripSort::Filename), [2, 3, 1]);
            c.execute("UPDATE assets SET rating = 1 WHERE id = 2", [])?;
            assert_eq!(all_pages(c, &scope, None, StripSort::Rating), [3, 1, 2]);
            for sort in [StripSort::Captured, StripSort::Rating, StripSort::Filename] {
                assert_eq!(strip_ids(c, &scope, None, sort)?, all_pages(c, &scope, None, sort), "{sort:?}");
            }
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn the_top_bar_filters_and_a_selection_narrow_the_strip() {
        let db = library();
        db.with_conn(|c| {
            c.execute_batch("UPDATE assets SET flag = 1, color_label = 3 WHERE id = 3; UPDATE assets SET flag = -1 WHERE id = 1;")?;
            let picks = LayoutScope { query: Some(AssetQuery { flag: Some(1), ..Default::default() }), ..Default::default() };
            assert_eq!(strip_ids(c, &picks, None, StripSort::Captured)?, [3]);
            let green = LayoutScope { query: Some(AssetQuery { color_labels: vec![3], ..Default::default() }), ..Default::default() };
            assert_eq!(strip_ids(c, &green, None, StripSort::Captured)?, [3]);
            assert_eq!(strip_ids(c, &LayoutScope::library(), Some(&[1, 3, 4]), StripSort::Captured)?, [3, 1], "the video in a selection is not developed");
            assert!(strip_ids(c, &LayoutScope::library(), Some(&[]), StripSort::Captured)?.is_empty());
            let page = strip_page(c, &LayoutScope::library(), None, StripSort::Captured, None, 10)?;
            assert!(page.next_cursor.is_none());
            let three = page.items.iter().find(|i| i.id == 3).unwrap();
            assert_eq!((three.flag, three.color_label, three.edited), (1, 3, false));
            Ok(())
        })
        .unwrap();
    }
}
