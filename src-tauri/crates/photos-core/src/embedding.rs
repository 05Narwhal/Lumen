//! Image embeddings as Lumen stores them: 512 signed bytes, L2-normalised then scaled by
//! 127. Half a kilobyte a photograph. This module knows nothing about the model that made
//! them — `photos-ml` does — only how to keep and compare them.

use std::collections::HashMap;

use rusqlite::{params, Connection, OptionalExtension};

use crate::Result;

pub const DIM: usize = 512;
pub type Vector = Vec<i8>;

/// L2-normalise, scale by 127 and round. Cosines survive to about a hundredth.
pub fn quantize(v: &[f32]) -> Vector {
    let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm <= f32::EPSILON {
        return vec![0; v.len()];
    }
    v.iter().map(|x| (x / norm * 127.0).round().clamp(-127.0, 127.0) as i8).collect()
}

/// The exact cosine of two stored vectors; 0 when either is empty, zero or mismatched.
pub fn cosine(a: &[i8], b: &[i8]) -> f32 {
    if a.is_empty() || a.len() != b.len() {
        return 0.0;
    }
    let (mut dot, mut na, mut nb) = (0i64, 0i64, 0i64);
    for (x, y) in a.iter().zip(b) {
        let (x, y) = (*x as i64, *y as i64);
        dot += x * y;
        na += x * x;
        nb += y * y;
    }
    if na == 0 || nb == 0 {
        return 0.0;
    }
    dot as f32 / ((na as f32).sqrt() * (nb as f32).sqrt())
}

/// The direction a set of photographs shares, e.g. what a memory is "about".
pub fn mean(vs: &[&[i8]]) -> Option<Vector> {
    let first = vs.first()?;
    let mut sum = vec![0f32; first.len()];
    for v in vs {
        for (s, x) in sum.iter_mut().zip(v.iter()) {
            *s += *x as f32;
        }
    }
    Some(quantize(&sum))
}

/// Named reference vectors, embedded offline from fixed phrases (ML design §3).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Prototypes {
    pub model: String,
    pub items: Vec<(String, Vector)>,
}

impl Prototypes {
    pub fn empty() -> Self {
        Self::default()
    }

    pub fn get(&self, name: &str) -> Option<&[i8]> {
        self.items.iter().find(|(n, _)| n == name).map(|(_, v)| v.as_slice())
    }
}

/// How `v` divides between the prototypes whose names start with `prefix`: a softmax of
/// their cosines at `temperature`. Names come back without the prefix, heaviest first.
pub fn softmax(v: &[i8], protos: &Prototypes, prefix: &str, temperature: f32) -> Vec<(String, f32)> {
    let scored: Vec<(String, f32)> = protos
        .items
        .iter()
        .filter_map(|(n, p)| n.strip_prefix(prefix).map(|short| (short.to_string(), cosine(v, p) / temperature)))
        .collect();
    let Some(max) = scored.iter().map(|s| s.1).reduce(f32::max) else { return Vec::new() };
    let total: f32 = scored.iter().map(|s| (s.1 - max).exp()).sum();
    let mut out: Vec<(String, f32)> = scored.into_iter().map(|(n, s)| (n, (s - max).exp() / total)).collect();
    out.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
    out
}

/// What the model said about one photograph.
#[derive(Debug, Clone, PartialEq)]
pub struct Features {
    pub vec: Vector,
    /// 0..1; `None` when the aesthetic head was unavailable.
    pub aesthetic: Option<f32>,
}

fn to_blob(v: &[i8]) -> Vec<u8> {
    v.iter().map(|x| *x as u8).collect()
}

pub(crate) fn from_blob(b: &[u8]) -> Vector {
    b.iter().map(|x| *x as i8).collect()
}

pub fn store(conn: &Connection, asset_id: i64, model: &str, v: &[i8], aesthetic: Option<f32>) -> Result<()> {
    conn.execute(
        "INSERT INTO asset_embeddings (asset_id, model, vec, aesthetic) VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(asset_id) DO UPDATE SET model = excluded.model, vec = excluded.vec, aesthetic = excluded.aesthetic",
        params![asset_id, model, to_blob(v), aesthetic.map(|a| a as f64)],
    )?;
    Ok(())
}

/// The features of whichever of `ids` have them. Read per memory or per show, never for the
/// whole library: 250k of them is 128 MB.
pub fn load(conn: &Connection, ids: &[i64]) -> Result<HashMap<i64, Features>> {
    let mut stmt = conn.prepare_cached("SELECT vec, aesthetic FROM asset_embeddings WHERE asset_id = ?1")?;
    let mut out = HashMap::with_capacity(ids.len());
    for id in ids {
        let row = stmt.query_row([id], |r| Ok((r.get::<_, Vec<u8>>(0)?, r.get::<_, Option<f64>>(1)?))).optional()?;
        if let Some((blob, aesthetic)) = row {
            out.insert(*id, Features { vec: from_blob(&blob), aesthetic: aesthetic.map(|a| a as f32) });
        }
    }
    Ok(out)
}

/// The backfill's worklist: `(asset_id, content_hash)` of visible photos with no embedding
/// from `model` — none at all, or one from a model version since replaced.
pub fn missing(conn: &Connection, model: &str, limit: usize) -> Result<Vec<(i64, String)>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT a.id, a.content_hash FROM assets a LEFT JOIN asset_embeddings e ON e.asset_id = a.id
         WHERE a.state = 0 AND a.media_type != 1 AND NOT {} AND (e.asset_id IS NULL OR e.model != ?1)
         ORDER BY a.id LIMIT ?2",
        crate::assets::MOVIE_HALF
    ))?;
    let rows = stmt.query_map(params![model, limit as i64], |r| Ok((r.get(0)?, r.get(1)?)))?;
    Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
}

/// Embeddings whose photo has no scene tags yet (`scene_tags IS NULL`): the tag backfill's
/// worklist. A photo that shows nothing taggable is stored as `''`, so it leaves the list.
pub fn untagged(conn: &Connection, limit: usize) -> Result<Vec<(i64, Vector)>> {
    let mut stmt = conn.prepare(
        "SELECT e.asset_id, e.vec FROM asset_embeddings e JOIN assets a ON a.id = e.asset_id
         WHERE a.scene_tags IS NULL ORDER BY e.asset_id LIMIT ?1",
    )?;
    let rows = stmt.query_map([limit as i64], |r| Ok((r.get::<_, i64>(0)?, from_blob(&r.get::<_, Vec<u8>>(1)?))))?;
    Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
}

/// Stores a photo's scene tags and puts them in the search index.
pub fn set_scene_tags(conn: &Connection, asset_id: i64, tags: &[String]) -> Result<()> {
    conn.execute("UPDATE assets SET scene_tags = ?2 WHERE id = ?1", params![asset_id, tags.join(" ")])?;
    crate::fts::index_ids(conn, &[asset_id])
}

/// The scene tags a photograph shows: the `tag:*` prototypes whose softmax share (at a sharp
/// temperature) exceeds `TAG_SHARE` and whose cosine exceeds `TAG_COSINE`, best first, at most
/// `MAX_TAGS`. Both tests, because the share alone names *something* for every photograph —
/// the least unlikely tag of a receipt is still a tag — and the cosine alone cannot choose.
pub fn scene_tags(v: &[i8], protos: &Prototypes) -> Vec<String> {
    let shares = softmax(v, protos, "tag:", 0.01);
    shares
        .into_iter()
        .filter(|(name, share)| *share > TAG_SHARE && protos.get(&format!("tag:{name}")).is_some_and(|p| cosine(v, p) > TAG_COSINE))
        .take(MAX_TAGS)
        .map(|(name, _)| name)
        .collect()
}

/// Every scene tag a photograph can carry: the `tag:*` keys of `photos-ml`'s
/// `prototypes.json` (a test there keeps the two equal). Search reads tags out of a query
/// with this list (`query_parse`), so it must not wait for the models to load.
pub const SCENE_TAGS: [&str; 60] = [
    "airplane", "autumn leaves", "baby", "beach", "bicycle", "bird", "boat", "bridge", "building", "cake", "camping", "car",
    "castle", "cat", "child", "christmas", "church", "city", "coffee", "concert", "cycling", "desert", "dog", "drinks", "field",
    "fireworks", "flowers", "fog", "food", "football", "forest", "garden", "group of people", "hiking", "horse", "interior",
    "kitchen", "lake", "mountain", "museum", "night sky", "painting", "party", "portrait", "rain", "restaurant", "river", "sea",
    "selfie", "skiing", "snow", "sport", "stage", "street", "sunrise", "sunset", "swimming", "train", "waterfall", "wedding",
];

pub const TAG_SHARE: f32 = 0.15;
pub const TAG_COSINE: f32 = 0.22;
pub const MAX_TAGS: usize = 5;
/// Semantic hits lie within this of the best match, and above `SEMANTIC_FLOOR`: CLIP's
/// text–image cosines sit between about 0.15 and 0.35, so a band relative to the best holds
/// across queries where one fixed cut would be too strict for one and too loose for another.
pub const SEMANTIC_BAND: f32 = 0.06;
pub const SEMANTIC_FLOOR: f32 = 0.20;

/// The sentences a search asks the text tower, whose embeddings are averaged (prompt
/// ensembling). Measured on Julian's marked queries (Round 14): precision@20 of *Best match*
/// 0.945 → 0.959 at the same recall, and tighter meaning sets ("night city" 3,000 → 1,635).
pub fn search_prompts(query: &str) -> [String; 3] {
    let q = query.trim();
    [format!("a photo of {q}"), q.to_string(), format!("a picture showing {q}")]
}

/// The `limit` photographs whose embeddings are nearest `query`, best first. One streaming
/// pass over the table — SQLite hands rows over one page at a time, so nothing like the
/// whole table is held — with the few hidden photos (trash, Live Photo movie halves) fetched
/// once as a set. Joining `assets` per row instead cost 1.8 s at 250k; the pass is 71 ms.
pub fn semantic_scan(conn: &Connection, query: &[i8], limit: usize) -> Result<Vec<(i64, f32)>> {
    let hidden: std::collections::HashSet<i64> = {
        let mut stmt = conn.prepare(&format!("SELECT a.id FROM assets a WHERE a.state != 0 OR a.concealed = 1 OR {}", crate::assets::MOVIE_HALF))?;
        let ids = stmt.query_map([], |r| r.get(0))?;
        ids.collect::<std::result::Result<_, _>>()?
    };
    let q_norm = (query.iter().map(|x| (*x as i32) * (*x as i32)).sum::<i32>() as f32).sqrt();
    let mut best: Vec<(i64, f32)> = Vec::with_capacity(limit + 1);
    let mut stmt = conn.prepare("SELECT asset_id, vec FROM asset_embeddings")?;
    let mut rows = stmt.query([])?;
    while let Some(r) = rows.next()? {
        let id: i64 = r.get(0)?;
        if hidden.contains(&id) {
            continue;
        }
        // Borrowed from SQLite's page, not copied.
        let c = cosine_bytes(query, q_norm, r.get_ref(1)?.as_blob().unwrap_or(&[]));
        if best.len() < limit || c > best[best.len() - 1].1 {
            let at = best.partition_point(|b| b.1 > c || (b.1 == c && b.0 < id));
            best.insert(at, (id, c));
            best.truncate(limit);
        }
    }
    Ok(best)
}

/// `cosine` against a stored blob, in i32 so the compiler can vectorise it (512 × 127² fits),
/// with the query's norm computed once.
fn cosine_bytes(query: &[i8], q_norm: f32, blob: &[u8]) -> f32 {
    if blob.len() != query.len() || q_norm == 0.0 {
        return 0.0;
    }
    let (mut dot, mut nb) = (0i32, 0i32);
    for (x, y) in query.iter().zip(blob) {
        let (x, y) = (*x as i32, *y as i8 as i32);
        dot += x * y;
        nb += y * y;
    }
    if nb == 0 { 0.0 } else { dot as f32 / (q_norm * (nb as f32).sqrt()) }
}

/// The scan's matches worth showing: within `SEMANTIC_BAND` of the best, above the floor.
pub fn semantic_hits(scored: &[(i64, f32)]) -> Vec<i64> {
    let Some(top) = scored.first().map(|s| s.1) else { return Vec::new() };
    scored.iter().filter(|(_, c)| *c >= SEMANTIC_FLOOR && *c >= top - SEMANTIC_BAND).map(|(id, _)| *id).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Db;

    /// `cargo test --release -p photos-core --lib scan_at_250k -- --ignored --nocapture`
    #[test]
    #[ignore = "benchmark"]
    fn scan_at_250k() {
        let db = Db::open_in_memory().unwrap();
        db.with_conn(|c| {
            let tx = c.unchecked_transaction()?;
            let mut state = 0x2545_F491_4F6C_DD1Du64;
            for i in 1..=250_000i64 {
                tx.execute(
                    "INSERT INTO assets (id, content_hash, relative_path, filename, extension, media_type, byte_size, orientation, imported_at)
                     VALUES (?1, ?1, 'p', 'f.jpg', 'jpg', 0, 1, 1, 10)",
                    [i],
                )?;
                let v: Vec<f32> = (0..DIM)
                    .map(|_| {
                        state ^= state << 13;
                        state ^= state >> 7;
                        state ^= state << 17;
                        (state % 2000) as f32 / 1000.0 - 1.0
                    })
                    .collect();
                store(&tx, i, "m", &quantize(&v), None)?;
            }
            tx.commit()?;
            let q = quantize(&vec![0.5f32; DIM]);
            let t = std::time::Instant::now();
            let top = semantic_scan(c, &q, 2000)?;
            println!("scan of 250k embeddings: {:?}, {} kept", t.elapsed(), top.len());
            let t = std::time::Instant::now();
            let mut stmt = c.prepare("SELECT asset_id, vec FROM asset_embeddings")?;
            let mut rows = stmt.query([])?;
            let mut n = 0usize;
            while let Some(r) = rows.next()? {
                n += r.get_ref(1)?.as_blob().map(|b| b.len()).unwrap_or(0);
            }
            println!("raw read of the table: {:?} ({n} bytes)", t.elapsed());
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn tags_are_stored_searchable_and_leave_the_worklist() {
        let db = Db::open_in_memory().unwrap();
        db.with_conn(|c| {
            c.execute_batch(
                "INSERT INTO assets (id, content_hash, relative_path, filename, extension, media_type, byte_size, orientation, imported_at)
                 VALUES (1,'h1','p','IMG_1.jpg','jpg',0,1,1,10), (2,'h2','p','IMG_2.jpg','jpg',0,1,1,10);",
            )?;
            crate::fts::rebuild(c)?;
            store(c, 1, "m", &tilt(0, 1, 0.0), None)?;
            store(c, 2, "m", &tilt(1, 0, 0.0), None)?;
            assert_eq!(untagged(c, 10)?.len(), 2);
            set_scene_tags(c, 1, &["beach".into(), "sunset".into()])?;
            set_scene_tags(c, 2, &[])?;
            assert!(untagged(c, 10)?.is_empty(), "an untaggable photo is done too");
            let q = crate::query::AssetQuery { text: Some("sunset".into()), ..Default::default() };
            assert_eq!(crate::layout::count_scoped(c, &crate::scope::LayoutScope { query: Some(q), ..Default::default() })?, 1);
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn a_photograph_is_tagged_with_what_it_clearly_shows() {
        let protos = Prototypes {
            model: "m".into(),
            items: vec![("tag:beach".into(), tilt(0, 1, 0.0)), ("tag:snow".into(), tilt(1, 0, 0.0)), ("tag:dog".into(), tilt(2, 0, 0.0))],
        };
        assert_eq!(scene_tags(&tilt(0, 1, 0.2), &protos), vec!["beach"]);
        assert!(scene_tags(&tilt(9, 1, 0.0), &protos).is_empty(), "nothing it resembles");
    }

    #[test]
    fn the_scan_keeps_the_nearest_in_order_and_the_hits_are_a_band() {
        let db = Db::open_in_memory().unwrap();
        db.with_conn(|c| {
            for i in 1..=6i64 {
                c.execute(
                    "INSERT INTO assets (id, content_hash, relative_path, filename, extension, media_type, byte_size, orientation, imported_at)
                     VALUES (?1, ?2, 'p', 'f.jpg', 'jpg', 0, 1, 1, 10)",
                    params![i, format!("h{i}")],
                )?;
                store(c, i, "m", &tilt(0, 1, 0.15 * i as f32), None)?;
            }
            let q = tilt(0, 1, 0.0);
            let top = semantic_scan(c, &q, 3)?;
            assert_eq!(top.iter().map(|t| t.0).collect::<Vec<_>>(), vec![1, 2, 3]);
            assert!(top[0].1 >= top[1].1 && top[1].1 >= top[2].1);
            let all = semantic_scan(c, &q, 10)?;
            let hits = semantic_hits(&all);
            assert!(hits.contains(&1) && !hits.contains(&6), "{all:?}");
            assert!(semantic_hits(&[]).is_empty());
            Ok(())
        })
        .unwrap();
    }

    /// A unit vector along `axis`, tilted towards `other` by `t` — cosines chosen by construction.
    fn tilt(axis: usize, other: usize, t: f32) -> Vector {
        let mut v = vec![0f32; DIM];
        v[axis] = (1.0 - t * t).sqrt();
        v[other] = t;
        quantize(&v)
    }

    #[test]
    fn quantised_vectors_keep_their_cosines() {
        let a = tilt(0, 1, 0.0);
        let b = tilt(0, 1, 0.6);
        assert!((cosine(&a, &b) - 0.8).abs() < 0.01, "{}", cosine(&a, &b));
        assert!((cosine(&a, &a) - 1.0).abs() < 1e-4);
        assert_eq!(cosine(&a, &[]), 0.0);
        assert_eq!(cosine(&vec![0; DIM], &a), 0.0);
    }

    #[test]
    fn the_mean_points_between_its_members() {
        let a = tilt(0, 1, 0.0);
        let b = tilt(1, 0, 0.0);
        let m = mean(&[&a, &b]).unwrap();
        assert!((cosine(&m, &a) - cosine(&m, &b)).abs() < 0.01);
        assert!(mean(&[]).is_none());
    }

    #[test]
    fn a_search_asks_the_text_tower_three_ways() {
        assert_eq!(search_prompts("dog"), ["a photo of dog", "dog", "a picture showing dog"]);
        assert_eq!(search_prompts("  night city "), ["a photo of night city", "night city", "a picture showing night city"]);
    }

    #[test]
    fn the_ensemble_ranks_first_what_the_prompts_agree_on() {
        let db = Db::open_in_memory().unwrap();
        db.with_conn(|c| {
            // Photo 1 matches only the first prompt, 2 only the second, 3 both a little less.
            let photos = [(1i64, tilt(0, 2, 0.1)), (2, tilt(1, 2, 0.1)), (3, mean(&[&tilt(0, 1, 0.0), &tilt(1, 0, 0.0)]).unwrap())];
            for (i, v) in &photos {
                c.execute(
                    "INSERT INTO assets (id, content_hash, relative_path, filename, extension, media_type, byte_size, orientation, imported_at)
                     VALUES (?1, ?2, 'p', 'f.jpg', 'jpg', 0, 1, 1, 10)",
                    params![i, format!("h{i}")],
                )?;
                store(c, *i, "m", v, None)?;
            }
            let (a, b) = (tilt(0, 1, 0.0), tilt(1, 0, 0.0));
            assert_eq!(semantic_scan(c, &a, 3)?[0].0, 1, "one prompt alone prefers its own photo");
            let both = mean(&[&a, &b]).unwrap();
            assert_eq!(semantic_scan(c, &both, 3)?[0].0, 3);
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn softmax_ranks_the_nearest_prototype_first() {
        let protos = Prototypes {
            model: "m".into(),
            items: vec![("mood:calm".into(), tilt(0, 1, 0.0)), ("mood:epic".into(), tilt(1, 0, 0.0)), ("good".into(), tilt(2, 0, 0.0))],
        };
        let w = softmax(&tilt(0, 1, 0.3), &protos, "mood:", 0.05);
        assert_eq!(w.len(), 2, "only the mood prototypes");
        assert_eq!(w[0].0, "calm");
        assert!((w.iter().map(|x| x.1).sum::<f32>() - 1.0).abs() < 1e-4);
        assert!(softmax(&tilt(0, 1, 0.3), &Prototypes::empty(), "mood:", 0.05).is_empty());
    }

    #[test]
    fn stores_loads_and_lists_what_is_missing() {
        let db = Db::open_in_memory().unwrap();
        db.with_conn(|c| {
            c.execute_batch(
                "INSERT INTO assets (id, content_hash, relative_path, filename, extension, media_type, byte_size, orientation, imported_at)
                 VALUES (1,'h1','p','a.jpg','jpg',0,1,1,10), (2,'h2','p','b.jpg','jpg',0,1,1,10), (3,'h3','p','c.mp4','mp4',1,1,1,10);",
            )?;
            store(c, 1, "clip@1", &tilt(0, 1, 0.0), Some(0.7))?;
            let loaded = load(c, &[1, 2])?;
            assert_eq!(loaded.len(), 1);
            assert_eq!(loaded[&1].aesthetic, Some(0.7));
            assert_eq!(missing(c, "clip@1", 10)?, vec![(2, "h2".to_string())], "videos are never embedded");
            assert_eq!(missing(c, "clip@2", 10)?.len(), 2, "a new model version re-queues");
            Ok(())
        })
        .unwrap();
    }
}
