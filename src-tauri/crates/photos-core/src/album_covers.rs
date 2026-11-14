//! Album covers: up to three prints that look as different from each other as possible,
//! picked with the Memories terms (time × place × content, a look-alike veto) and seeded so
//! the pick stays put until the album's photos change or the person shuffles it.

use std::collections::HashMap;

use rusqlite::{params, Connection, OptionalExtension};

use crate::albums::{first_matches, smart_scope, AlbumSummary, KIND_SMART, SHOWN};
use crate::embedding::{self, Features, Prototypes};
use crate::layout::count_scoped;
use crate::memories::{looks_like, quality, repeats, similar, Photo};
use crate::Result;

/// How many prints a cover shows at most.
pub const PRINTS: usize = 3;
/// The most photos one pick looks at, so a 20,000-photo album costs no more than a small one.
const POOL: usize = 400;
/// The columns of a [`Photo`], for an asset aliased `x`.
const PHOTO_COLUMNS: &str = "x.id, COALESCE(x.captured_at, 0), x.gps_lat, x.gps_lon, x.is_favorite, x.rating, x.sharpness, x.phash,
                             COALESCE(NULLIF(x.locality, ''), NULLIF(x.place_name, ''))";

fn photo_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Photo> {
    Ok(Photo {
        id: r.get(0)?,
        local: r.get(1)?,
        lat: r.get(2)?,
        lon: r.get(3)?,
        favorite: r.get::<_, i64>(4)? != 0,
        rating: r.get(5)?,
        sharpness: r.get::<_, Option<f64>>(6)?.map(|v| v as f32),
        phash: r.get::<_, Option<i64>>(7)?.map(|v| v as u64),
        place: r.get(8)?,
    })
}

/// Brings `album.preview_ids` and `album.cover_asset_id` up to date, picking again only when
/// what the prints were picked from has changed (`cached` = stored `(preview_json, preview_key)`).
/// `album.cover_asset_id` comes in as the stored cover; `picked` says the person chose it.
pub fn refresh(conn: &Connection, album: &mut AlbumSummary, picked: bool, seed: i64, cached: Option<(String, String)>) -> Result<()> {
    let mixed = (seed as u64) ^ (album.id as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
    let hit = |key: &str| -> Option<Vec<i64>> {
        cached.as_ref().filter(|(_, k)| k == key).and_then(|(json, _)| serde_json::from_str(json).ok())
    };
    if album.kind == KIND_SMART {
        // A smart album has no membership rows: its count and prints come from its query, and
        // its newest match stays in front.
        let Some(q) = album.query.clone() else { return Ok(()) };
        let scope = smart_scope(q);
        album.item_count = count_scoped(conn, &scope).unwrap_or(0);
        let ids = first_matches(conn, &scope, POOL).unwrap_or_default();
        let key = format!("{}|{}|{seed}", album.item_count, ids.first().copied().unwrap_or(0));
        let preview = match hit(&key) {
            Some(p) => p,
            None => {
                let mut stmt = conn.prepare_cached(&format!("SELECT {PHOTO_COLUMNS} FROM assets x WHERE x.id = ?1"))?;
                let mut photos = Vec::with_capacity(ids.len());
                for id in &ids {
                    photos.extend(stmt.query_row([id], photo_row).optional()?);
                }
                let preview = pick_from(conn, &photos, ids.first().copied(), mixed)?;
                store(conn, album.id, &preview, &key, None)?;
                preview
            }
        };
        album.cover_asset_id = preview.first().copied();
        album.preview_ids = preview;
        return Ok(());
    }

    let (count, newest, sum): (i64, Option<i64>, Option<i64>) = conn.query_row(
        &format!(
            "SELECT COUNT(*), MAX(ai.added_at), SUM(ai.asset_id) FROM album_items ai
               JOIN assets x ON x.id = ai.asset_id JOIN albums a ON a.id = ai.album_id
              WHERE ai.album_id = ?1 AND {SHOWN}"
        ),
        [album.id],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    )?;
    album.item_count = count;
    let mut all: Vec<Photo> = conn
        .prepare_cached(&format!(
            "SELECT {PHOTO_COLUMNS} FROM album_items ai JOIN assets x ON x.id = ai.asset_id JOIN albums a ON a.id = ai.album_id
              WHERE ai.album_id = ?1 AND {SHOWN}"
        ))?
        .query_map([album.id], photo_row)?
        .collect::<std::result::Result<_, _>>()?;
    // A hand-picked cover that left the album, or is trashed or in the vault, is forgotten.
    let front = album.cover_asset_id.filter(|c| picked && all.iter().any(|p| p.id == *c));
    if picked && front.is_none() {
        conn.execute("UPDATE albums SET cover_picked = 0 WHERE id = ?1", [album.id])?;
    }
    let key = format!("{count}|{}|{}|{}|{seed}", newest.unwrap_or(0), sum.unwrap_or(0), front.map(|c| c.to_string()).unwrap_or_default());
    let preview = match hit(&key) {
        Some(p) => p,
        None => {
            all.sort_by(|a, b| jitter(mixed, a.id).total_cmp(&jitter(mixed, b.id)).then(a.id.cmp(&b.id)));
            if let Some(f) = front {
                if let Some(i) = all.iter().position(|p| p.id == f).filter(|i| *i >= POOL) {
                    all.swap(0, i);
                }
            }
            all.truncate(POOL);
            let preview = pick_from(conn, &all, front, mixed)?;
            store(conn, album.id, &preview, &key, Some(front.or(preview.first().copied())))?;
            preview
        }
    };
    album.cover_asset_id = front.or(preview.first().copied());
    album.preview_ids = preview;
    Ok(())
}

fn pick_from(conn: &Connection, photos: &[Photo], front: Option<i64>, seed: u64) -> Result<Vec<i64>> {
    let ids: Vec<i64> = photos.iter().map(|p| p.id).collect();
    let features = embedding::load(conn, &ids)?;
    let refs: Vec<&Photo> = photos.iter().collect();
    Ok(pick(&refs, &features, &Prototypes::empty(), front, seed))
}

/// `cover`: `Some(id)` also stores the cover (a manual album), `None` leaves it (a smart one).
fn store(conn: &Connection, album_id: i64, preview: &[i64], key: &str, cover: Option<Option<i64>>) -> Result<()> {
    let json = serde_json::to_string(preview)?;
    match cover {
        Some(c) => conn.execute(
            "UPDATE albums SET preview_json = ?2, preview_key = ?3, cover_asset_id = ?4 WHERE id = ?1",
            params![album_id, json, key, c],
        )?,
        None => conn.execute("UPDATE albums SET preview_json = ?2, preview_key = ?3 WHERE id = ?1", params![album_id, json, key])?,
    };
    Ok(())
}
/// A photo this much like one already chosen is the same picture for a cover.
const TOO_SAME: f64 = 0.85;
/// How far chance moves the front print: enough to choose among the good ones, not enough
/// to put a poor one first.
const FRONT_JITTER: f64 = 0.6;
const SIDE_QUALITY: f64 = 0.35;
const SIDE_JITTER: f64 = 0.25;

/// A number in 0..1 that depends only on `seed` and `id` (splitmix64).
pub fn jitter(seed: u64, id: i64) -> f64 {
    let mut z = (seed ^ id as u64).wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^= z >> 31;
    (z >> 11) as f64 / (1u64 << 53) as f64
}

/// How much `p` is the same picture as `c`, 0..1. Time and place only count when both
/// photos have a capture time (`local` 0 means none), so screenshots and EXIF-less imports
/// are told apart by their content alone.
fn sameness(p: &Photo, c: &Photo, features: &HashMap<i64, Features>) -> f64 {
    let content = match (features.get(&p.id), features.get(&c.id)) {
        (Some(a), Some(b)) => Some(similar(&a.vec, &b.vec)),
        _ => None,
    };
    let moment = if p.local != 0 && c.local != 0 { repeats(p, c, content) } else { 0.0 };
    moment.max(content.unwrap_or(0.0))
}

/// Up to `PRINTS` ids, front first: `front` when it is among `photos`, else a good photo
/// chosen by chance; then the photos least like everything already chosen. Look-alikes
/// are never both chosen, so an album of one burst gives one print.
pub fn pick(photos: &[&Photo], features: &HashMap<i64, Features>, protos: &Prototypes, front: Option<i64>, seed: u64) -> Vec<i64> {
    if photos.is_empty() {
        return Vec::new();
    }
    let q = quality(photos, features, protos);
    let first = front.and_then(|f| photos.iter().position(|p| p.id == f)).unwrap_or_else(|| {
        argmax(photos.iter().enumerate().map(|(i, p)| (i, q[i] + FRONT_JITTER * jitter(seed, p.id)))).unwrap_or(0)
    });
    let mut chosen = vec![first];
    while chosen.len() < PRINTS {
        let next = argmax(photos.iter().enumerate().filter_map(|(i, p)| {
            if chosen.contains(&i) || chosen.iter().any(|&c| looks_like(p, photos[c], features)) {
                return None;
            }
            let same = chosen.iter().map(|&c| sameness(p, photos[c], features)).fold(0.0, f64::max);
            (same < TOO_SAME).then(|| (i, (1.0 - same) + SIDE_QUALITY * q[i] + SIDE_JITTER * jitter(seed, p.id)))
        }));
        match next {
            Some(i) => chosen.push(i),
            None => break,
        }
    }
    chosen.into_iter().map(|i| photos[i].id).collect()
}

fn argmax(scored: impl Iterator<Item = (usize, f64)>) -> Option<usize> {
    scored.fold(None, |best: Option<(usize, f64)>, (i, s)| match best {
        Some((_, b)) if b >= s => best,
        _ => Some((i, s)),
    })
    .map(|(i, _)| i)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::embedding::quantize;

    const DAY: i64 = 86_400;

    fn mix(i: u64) -> u64 {
        let mut z = i.wrapping_add(0x9E37_79B9_7F4A_7C15);
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn photo(id: i64, local: i64, gps: Option<(f64, f64)>, phash: u64) -> Photo {
        Photo {
            id,
            local,
            lat: gps.map(|g| g.0),
            lon: gps.map(|g| g.1),
            favorite: false,
            rating: 0,
            sharpness: None,
            phash: Some(phash),
            place: None,
        }
    }

    fn run(photos: &[Photo], features: &HashMap<i64, Features>, front: Option<i64>, seed: u64) -> Vec<i64> {
        let refs: Vec<&Photo> = photos.iter().collect();
        pick(&refs, features, &Prototypes::empty(), front, seed)
    }

    /// Five burst frames 1 s apart in one spot with one hash, and two photos days apart
    /// elsewhere with hashes far from it.
    fn burst_and_two() -> Vec<Photo> {
        let t = 1_700_000_000;
        let mut v: Vec<Photo> = (1..=5).map(|i| photo(i, t + i, Some((46.0, 14.0)), 0xABCD)).collect();
        v.push(photo(10, t + 3 * DAY, Some((48.0, 2.0)), mix(10)));
        v.push(photo(11, t + 9 * DAY, Some((40.0, -3.0)), mix(11)));
        v
    }

    #[test]
    fn burst_counts_as_one() {
        for seed in 0..10 {
            let out = run(&burst_and_two(), &HashMap::new(), None, seed);
            assert_eq!(out.len(), 3, "seed {seed}: {out:?}");
            assert_eq!(out.iter().filter(|id| **id <= 5).count(), 1, "seed {seed}: {out:?}");
        }
    }

    #[test]
    fn only_lookalikes_give_one_print() {
        let t = 1_700_000_000;
        let photos: Vec<Photo> = (1..=4).map(|i| photo(i, t + i * DAY, None, 0x1234)).collect();
        assert_eq!(run(&photos, &HashMap::new(), None, 3).len(), 1);
    }

    #[test]
    fn front_stays_first_and_sides_differ_from_it() {
        let out = run(&burst_and_two(), &HashMap::new(), Some(1), 5);
        assert_eq!(out[0], 1);
        assert!(out[1..].iter().all(|id| *id > 5), "{out:?}");
    }

    #[test]
    fn missing_front_acts_as_automatic() {
        let out = run(&burst_and_two(), &HashMap::new(), Some(999), 5);
        assert_eq!(out.len(), 3);
        assert!(!out.contains(&999));
    }

    #[test]
    fn stable_for_a_seed_and_changes_with_another() {
        let t = 1_700_000_000;
        let photos: Vec<Photo> =
            (1..=12).map(|i| photo(i, t + i * 7 * DAY, Some((30.0 + i as f64, i as f64 * 3.0)), mix(i as u64))).collect();
        let first = run(&photos, &HashMap::new(), None, 7);
        assert_eq!(first, run(&photos, &HashMap::new(), None, 7));
        assert!((1..20).any(|s| run(&photos, &HashMap::new(), None, s) != first));
    }

    #[test]
    fn no_metadata_still_distinct() {
        let photos: Vec<Photo> = (1..=5).map(|i| photo(i, 0, None, mix(i as u64))).collect();
        assert_eq!(run(&photos, &HashMap::new(), None, 1).len(), 3);
    }

    #[test]
    fn embedding_lookalike_is_vetoed() {
        let t = 1_700_000_000;
        let photos = vec![photo(1, t, None, mix(1)), photo(2, t + DAY, None, mix(2)), photo(3, t + 5 * DAY, None, mix(3))];
        let mut a = vec![0f32; 512];
        a[0] = 1.0;
        let mut b = a.clone();
        b[1] = 0.1;
        let mut c = vec![0f32; 512];
        c[2] = 1.0;
        let features: HashMap<i64, Features> = [(1, a), (2, b), (3, c)]
            .into_iter()
            .map(|(id, v)| (id, Features { vec: quantize(&v), aesthetic: None }))
            .collect();
        for seed in 0..10 {
            let out = run(&photos, &features, None, seed);
            assert!(!(out.contains(&1) && out.contains(&2)), "seed {seed}: {out:?}");
            assert_eq!(out.len(), 2, "seed {seed}: {out:?}");
        }
    }

    /// `LUMEN_DB=<copy of library.db> cargo test --release -p photos-core album_covers_on_a_copy -- --ignored --nocapture`
    /// — never the real file: the first list writes the cached picks.
    #[test]
    #[ignore]
    fn album_covers_on_a_copy() {
        let db = crate::Db::open(std::path::Path::new(&std::env::var("LUMEN_DB").unwrap())).unwrap();
        db.with_conn(|c| {
            for round in ["first (picks)", "second (cache)"] {
                let t = std::time::Instant::now();
                let albums = crate::albums::list_albums(c)?;
                println!("{round}: {} albums in {:?}", albums.len(), t.elapsed());
                for a in albums.iter().filter(|a| a.item_count > 500) {
                    println!("  {} ({}): {:?}", a.name, a.item_count, a.preview_ids);
                }
            }
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn jitter_is_in_unit_range_and_deterministic() {
        for id in 0..100 {
            let j = jitter(42, id);
            assert!((0.0..1.0).contains(&j));
            assert_eq!(j, jitter(42, id));
        }
    }
}
