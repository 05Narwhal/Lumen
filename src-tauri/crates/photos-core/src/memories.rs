//! Memories (spec §13): trips, events and "on this day", found by rules over the catalogue
//! and curated to their best photos. A pure pass over one compact row per photo — grouping
//! happens here in Rust, not in SQL — followed by one transaction that replaces the rows.

use crate::embedding::{cosine, Features, Prototypes};
use std::collections::{BTreeMap, HashMap, HashSet};

use chrono::{Datelike, NaiveDate};
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use ts_rs::TS;

use crate::places::haversine_km;
use crate::{CoreError, Result};

pub const KIND_TRIP: i64 = 0;
pub const KIND_EVENT: i64 = 1;
pub const KIND_ON_THIS_DAY: i64 = 2;

const SESSION_GAP: i64 = 12 * 3600;
const TRIP_MIN_ITEMS: usize = 10;
const TRIP_MIN_GPS: usize = 3;
const TRIP_MIN_KM: f64 = 50.0;
/// Raised from the roadmap's 8 after a real 34 k-photo history: at 8 almost every outing
/// was an event (658 of them), which buried the ones worth remembering.
const EVENT_MIN_ITEMS: usize = 12;
/// The busiest days only: at most this many events in one calendar month.
const MAX_EVENTS_PER_MONTH: usize = 2;
const EVENT_MAX_KM: f64 = 2.0;
const ON_THIS_DAY_MIN_ITEMS: usize = 3;
const HOME_WINDOW: i64 = 365 * 86_400;
/// About 5 km at mid latitudes.
const HOME_CELL_LAT: f64 = 0.045;
const HOME_CELL_LON: f64 = 0.06;
/// How far apart two perceptual hashes must be to be different pictures.
pub(crate) const NEAR_DUPLICATE_BITS: u32 = 6;

/// What clustering and curation need to know about one photo. `local` is the capture time
/// as wall-clock seconds (`captured_at`), so calendar days are the photographer's days.
#[derive(Debug, Clone, PartialEq)]
pub struct Photo {
    pub id: i64,
    pub local: i64,
    pub lat: Option<f64>,
    pub lon: Option<f64>,
    pub favorite: bool,
    pub rating: i64,
    pub sharpness: Option<f32>,
    pub phash: Option<u64>,
    /// The stored name of where it was taken: its locality, else its town ("Idrija, Slovenia").
    pub place: Option<String>,
}

impl Photo {
    fn gps(&self) -> Option<(f64, f64)> {
        self.lat.zip(self.lon)
    }

    fn date(&self) -> NaiveDate {
        chrono::DateTime::from_timestamp(self.local, 0).unwrap_or_default().date_naive()
    }
}

/// A memory before it is stored. `items` is every photo with its score and whether it is
/// in the curated set, best first.
#[derive(Debug, Clone, PartialEq)]
pub struct Draft {
    pub kind: i64,
    pub title: String,
    pub subtitle: Option<String>,
    pub start_at: i64,
    pub end_at: i64,
    pub centroid: Option<(f64, f64)>,
    pub items: Vec<(i64, f64, bool)>,
}

/// Time-ordered runs split where the gap exceeds 12 hours. `photos` is sorted by `local`.
pub fn sessions(photos: &[Photo]) -> Vec<&[Photo]> {
    let mut out = Vec::new();
    let mut start = 0;
    for i in 1..photos.len() {
        if photos[i].local - photos[i - 1].local > SESSION_GAP {
            out.push(&photos[start..i]);
            start = i;
        }
    }
    if start < photos.len() {
        out.push(&photos[start..]);
    }
    out
}

fn centroid<'a>(photos: impl IntoIterator<Item = &'a Photo>) -> Option<(f64, f64)> {
    let (mut lat, mut lon, mut n) = (0.0, 0.0, 0.0);
    for (a, b) in photos.into_iter().filter_map(Photo::gps) {
        lat += a;
        lon += b;
        n += 1.0;
    }
    (n > 0.0).then(|| (lat / n, lon / n))
}

/// Home: the mean position of the fullest ~5 km cell over the year before the newest photo.
pub fn home(photos: &[Photo]) -> Option<(f64, f64)> {
    let newest = photos.iter().map(|p| p.local).max()?;
    let mut cells: HashMap<(i64, i64), (usize, f64, f64)> = HashMap::new();
    for p in photos.iter().filter(|p| p.local >= newest - HOME_WINDOW) {
        let Some((lat, lon)) = p.gps() else { continue };
        let cell = cells.entry(((lat / HOME_CELL_LAT).floor() as i64, (lon / HOME_CELL_LON).floor() as i64)).or_default();
        cell.0 += 1;
        cell.1 += lat;
        cell.2 += lon;
    }
    // Fullest first; ties go to the smaller key, so the answer never depends on hash order.
    let (_, (n, lat, lon)) = cells.into_iter().max_by(|a, b| a.1 .0.cmp(&b.1 .0).then(b.0.cmp(&a.0)))?;
    Some((lat / n as f64, lon / n as f64))
}

const MONTHS_SHORT: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

/// "12 March 2024", "12–19 March 2024", "28 March – 2 April 2024", "30 Dec 2023 – 3 Jan 2024".
pub fn date_range(a: NaiveDate, b: NaiveDate) -> String {
    if a == b {
        a.format("%-d %B %Y").to_string()
    } else if (a.year(), a.month()) == (b.year(), b.month()) {
        format!("{}–{}", a.day(), b.format("%-d %B %Y"))
    } else if a.year() == b.year() {
        format!("{} – {}", a.format("%-d %B"), b.format("%-d %B %Y"))
    } else {
        let short = |d: NaiveDate| format!("{} {} {}", d.day(), MONTHS_SHORT[d.month0() as usize], d.year());
        format!("{} – {}", short(a), short(b))
    }
}

/// A stored name for a title: "Idrija, Slovenia" → "Idrija"; a bare country stays as it is.
fn title_place(stored: &str) -> &str {
    stored.rsplit_once(", ").map_or(stored, |(head, _)| head)
}

/// Where most of the photos were taken. The average position of a trip through several
/// places can land between them — at sea, or in a town it only passed — so each photo's
/// stored name (its locality, else its town; exact once named online, never guessed here
/// from the atlas) counts and the commonest wins (ties: alphabetical, so the answer is stable).
fn common_place(photos: &[&Photo]) -> Option<String> {
    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    for name in photos.iter().filter_map(|p| p.place.as_deref()).map(title_place).filter(|n| !n.is_empty()) {
        *counts.entry(name).or_default() += 1;
    }
    counts.into_iter().max_by(|a, b| a.1.cmp(&b.1).then(b.0.cmp(a.0))).map(|(name, _)| name.to_string())
}

/// How much a photograph is penalised for repeating one already chosen. At 1.0 a perfect
/// repeat would be worth nothing at all; the terms below are each in 0..1.
const DIVERSITY_WEIGHT: f64 = 0.65;
/// Photographs closer together in time than this are the same part of the occasion.
const SAME_MOMENT_S: f64 = 20.0 * 60.0;
/// And closer together than this are the same place — roughly 1.5 km, in degrees.
const SAME_PLACE_DEG: f64 = 0.015;
/// Embeddings this close are the same picture — burst frames that fooled the 64-bit hash.
const SAME_CONTENT_COS: f32 = 0.96;
/// Below this cosine two photographs show different things; `SIMILAR_SPAN` above it they
/// show the same thing. In between, similarity rises linearly.
const SIMILAR_FROM: f32 = 0.70;
const SIMILAR_SPAN: f32 = 0.25;
/// A junk prototype must take more than this share of the softmax to count at all, so an
/// ordinary photograph is never penalised for faintly resembling a document.
const JUNK_FROM: f32 = 0.5;

/// Quality in 0..1. Sharpness enters as a *rank* rather than a fraction of the sharpest
/// photograph, so one unusually crisp frame cannot flatten everyone else to nearly zero;
/// favourites and stars are added on top, because those are the user's own judgement and
/// are not relative to anything.
pub(crate) fn quality(photos: &[&Photo], features: &HashMap<i64, Features>, protos: &Prototypes) -> Vec<f64> {
    let sharpness = |p: &Photo| p.sharpness.unwrap_or(0.0);
    let mut sorted: Vec<f32> = photos.iter().map(|p| sharpness(p)).collect();
    sorted.sort_by(f32::total_cmp);
    let last = photos.len().saturating_sub(1).max(1) as f64;
    // The fraction of the memory that is *strictly* less sharp. Photographs of equal
    // sharpness share a rank, so a library with no sharpness measured at all — or one
    // where every frame is equally crisp — does not invent an order out of nothing.
    let sharp_rank: Vec<f64> = photos
        .iter()
        .map(|p| {
            let s = sharpness(p);
            sorted.partition_point(|v| *v < s) as f64 / last
        })
        .collect();
    photos
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let fav = if p.favorite { 3.0 } else { 0.0 };
            let stars = p.rating.clamp(0, 5) as f64;
            let base = fav + stars + sharp_rank[i];
            match features.get(&p.id) {
                // With an embedding: how good it looks counts two stars, and looking like a
                // screenshot or a receipt costs three. 9 + 2 = 11 at the very best.
                Some(f) => {
                    let aesthetic = f.aesthetic.unwrap_or(0.5) as f64;
                    ((base + 2.0 * aesthetic - 3.0 * junk(&f.vec, protos)) / 11.0).clamp(0.0, 1.0)
                }
                // 3 + 5 + 1 = 9 at the very best.
                None => base / 9.0,
            }
        })
        .collect()
}

/// How much `p` repeats `other` as *part of the occasion*, in 0..1.
///
/// Two photographs are redundant when they are of the same place **and** the same minutes:
/// a second frame of one spot ten minutes later adds nothing, where the same spot four
/// hours later is a different part of the day, and the same minute somewhere else is a
/// different part of the place. So the two terms multiply rather than compete — taking the
/// larger of them made a memory shot entirely in one spot look uniformly redundant, which
/// left the time term unable to spread the highlights across it at all.
///
/// Where one of the pair has no position, only time is known, and time alone decides.
pub(crate) fn repeats(p: &Photo, other: &Photo, similar: Option<f64>) -> f64 {
    let time = (1.0 - (p.local - other.local).abs() as f64 / SAME_MOMENT_S).clamp(0.0, 1.0);
    let place = match (p.gps(), other.gps()) {
        (Some((a_lat, a_lon)), Some((b_lat, b_lon))) => {
            let d = ((a_lat - b_lat).powi(2) + (a_lon - b_lon).powi(2)).sqrt();
            (1.0 - d / SAME_PLACE_DEG).clamp(0.0, 1.0)
        }
        _ => 1.0,
    };
    // Content scales the result rather than joining the product: two frames of the same
    // subject in the same minutes repeat fully, a different subject in them half as much.
    time * place * similar.map(|s| 0.5 + 0.5 * s).unwrap_or(1.0)
}

/// How far two embeddings show the same thing, 0..1 (`SIMILAR_FROM` .. + `SIMILAR_SPAN`).
pub(crate) fn similar(a: &[i8], b: &[i8]) -> f64 {
    ((cosine(a, b) - SIMILAR_FROM) / SIMILAR_SPAN).clamp(0.0, 1.0) as f64
}

/// How much a photograph looks like something nobody wants in a highlight reel — a
/// screenshot, a receipt, a whiteboard, a blur, the inside of a pocket — as the largest
/// share of the softmax over the `junk:*` prototypes and `good`. 0 below `JUNK_FROM`.
pub fn junk(v: &[i8], protos: &Prototypes) -> f64 {
    let scored: Vec<(bool, f32)> = protos
        .items
        .iter()
        .filter(|(n, _)| n.starts_with("junk:") || n == "good")
        .map(|(n, p)| (n.starts_with("junk:"), cosine(v, p) / 0.05))
        .collect();
    let Some(max) = scored.iter().map(|s| s.1).reduce(f32::max) else { return 0.0 };
    let total: f32 = scored.iter().map(|s| (s.1 - max).exp()).sum();
    let worst = scored.iter().filter(|s| s.0).map(|s| (s.1 - max).exp() / total).fold(0.0, f32::max);
    if worst > JUNK_FROM { worst as f64 } else { 0.0 }
}

pub(crate) fn looks_like(a: &Photo, b: &Photo, features: &HashMap<i64, Features>) -> bool {
    let hashes = match (a.phash, b.phash) {
        (Some(x), Some(y)) => (x ^ y).count_ones() <= NEAR_DUPLICATE_BITS,
        _ => false,
    };
    hashes
        || match (features.get(&a.id), features.get(&b.id)) {
            (Some(x), Some(y)) => cosine(&x.vec, &y.vec) >= SAME_CONTENT_COS,
            _ => false,
        }
}

/// The highlights of a memory: the best photographs, chosen so that between them they cover
/// the occasion rather than its busiest few minutes.
///
/// Greedy maximum marginal relevance. The first pick is simply the best photograph; each one
/// after it is the best of what is left *after* subtracting how much it repeats a moment or
/// a place already covered. Looking almost identical to something already chosen is a veto
/// rather than a penalty: two frames of one instant are never both highlights, however well
/// rated the pair is, and only a memory with nothing else left to show will take a second.
///
/// The score returned is the photograph's own quality, for every photograph, curated or not
/// — one meaning, comparable across the whole memory. `curated` carries the selection.
///
/// Cost is `want` rounds of two passes, so linear in the memory's size with `want` ≤ 40.
pub fn curate(photos: &[&Photo]) -> Vec<(i64, f64, bool)> {
    curate_with(photos, &HashMap::new(), &Prototypes::empty())
}

/// `curate`, seeing the pictures (ML design §8): an aesthetic score and a junk penalty in
/// the quality, content similarity in the repetition, and near-identical embeddings as a
/// veto. A photograph with no embedding is judged exactly as `curate` judges it.
pub fn curate_with(photos: &[&Photo], features: &HashMap<i64, Features>, protos: &Prototypes) -> Vec<(i64, f64, bool)> {
    let quality = quality(photos, features, protos);
    let n = photos.len();
    let want = n.min((n / 4).clamp(8, 40));

    let mut taken = vec![false; n];
    // How much each photograph repeats the moments and places already chosen, kept up to
    // date so a round is O(n) rather than O(n·chosen): a memory can hold thousands.
    let mut penalty = vec![0.0f64; n];
    let mut chosen: Vec<usize> = Vec::with_capacity(want);
    let mut out: Vec<(i64, f64, bool)> = Vec::with_capacity(n);

    for _ in 0..want {
        let mut best: Option<(usize, f64)> = None;
        // Two rounds: anything that does not look like a photograph already chosen, and
        // then — only if that left nothing — the rest.
        for allow_lookalikes in [false, true] {
            for i in 0..n {
                if taken[i] {
                    continue;
                }
                if !allow_lookalikes && chosen.iter().any(|c| looks_like(photos[i], photos[*c], features)) {
                    continue;
                }
                let value = quality[i] - DIVERSITY_WEIGHT * penalty[i];
                let better = match best {
                    None => true,
                    Some((bi, bv)) => value > bv || (value == bv && photos[i].id < photos[bi].id),
                };
                if better {
                    best = Some((i, value));
                }
            }
            if best.is_some() {
                break;
            }
        }
        let Some((pick, _)) = best else { break };
        taken[pick] = true;
        chosen.push(pick);
        out.push((photos[pick].id, quality[pick], true));
        for i in 0..n {
            if !taken[i] {
                let content = match (features.get(&photos[i].id), features.get(&photos[pick].id)) {
                    (Some(a), Some(b)) => Some(similar(&a.vec, &b.vec)),
                    _ => None,
                };
                penalty[i] = penalty[i].max(repeats(photos[i], photos[pick], content));
            }
        }
    }

    // The rest, by quality, so every photograph carries a score on the same scale.
    let mut rest: Vec<usize> = (0..n).filter(|i| !taken[*i]).collect();
    rest.sort_by(|a, b| quality[*b].total_cmp(&quality[*a]).then(photos[*a].id.cmp(&photos[*b].id)));
    out.extend(rest.into_iter().map(|i| (photos[i].id, quality[i], false)));
    out
}

fn span(photos: &[&Photo]) -> (i64, i64) {
    let start = photos.iter().map(|p| p.local).min().unwrap_or(0);
    let end = photos.iter().map(|p| p.local).max().unwrap_or(0);
    (start, end)
}

fn draft(kind: i64, title: String, subtitle: Option<String>, photos: &[&Photo], centroid: Option<(f64, f64)>) -> Draft {
    let (start_at, end_at) = span(photos);
    Draft { kind, title, subtitle, start_at, end_at, centroid, items: curate(photos) }
}

/// Every memory in display order: on this day first (the most recent year first), then trips
/// and events, newest first. `today` is the calendar day on-this-day looks back from.
/// `photos` is sorted by `local`.
pub fn cluster(photos: &[Photo], today: NaiveDate) -> Vec<Draft> {
    let home = home(photos);
    let mut in_trip: HashSet<i64> = HashSet::new();
    let mut found: Vec<Draft> = Vec::new();

    for session in sessions(photos) {
        let gps = session.iter().filter(|p| p.gps().is_some()).count();
        if session.len() < TRIP_MIN_ITEMS || gps < TRIP_MIN_GPS {
            continue;
        }
        let Some(home) = home else { continue };
        // Most located photos far from home: a session that starts and ends at home with a
        // few photos away is a day out, not a trip. The trip is named and placed by its away
        // part, or the photos taken before leaving would name it after home.
        let away: Vec<&Photo> = session
            .iter()
            .filter(|p| p.gps().is_some_and(|(lat, lon)| haversine_km(home.0, home.1, lat, lon) > TRIP_MIN_KM))
            .collect();
        if away.len() < TRIP_MIN_GPS || away.len() * 2 <= gps {
            continue;
        }
        let c = centroid(away.iter().copied()).unwrap_or(home);
        let refs: Vec<&Photo> = session.iter().collect();
        let title = common_place(&away).map_or_else(|| "A trip".to_string(), |p| format!("Trip to {p}"));
        let subtitle = date_range(session[0].date(), session[session.len() - 1].date());
        in_trip.extend(session.iter().map(|p| p.id));
        found.push(draft(KIND_TRIP, title, Some(subtitle), &refs, Some(c)));
    }

    let mut days: BTreeMap<NaiveDate, Vec<&Photo>> = BTreeMap::new();
    for p in photos.iter().filter(|p| !in_trip.contains(&p.id)) {
        days.entry(p.date()).or_default().push(p);
    }
    let mut events: Vec<Draft> = Vec::new();
    for (day, items) in &days {
        if items.len() < EVENT_MIN_ITEMS {
            continue;
        }
        let c = centroid(items.iter().copied());
        if let Some(c) = c {
            let spread = items.iter().filter_map(|p| p.gps()).any(|(a, b)| haversine_km(a, b, c.0, c.1) > EVENT_MAX_KM);
            if spread {
                continue;
            }
        }
        let date = day.format("%A, %-d %B %Y").to_string();
        let (title, subtitle) = match c.and(common_place(items)) {
            Some(place) => (place, date),
            None => (date, format!("{} photos", items.len())),
        };
        events.push(draft(KIND_EVENT, title, Some(subtitle), items, c));
    }
    // Only the busiest days of each month stay events.
    let mut by_month: BTreeMap<(i32, u32), Vec<Draft>> = BTreeMap::new();
    for e in events {
        let d = chrono::DateTime::from_timestamp(e.start_at, 0).unwrap_or_default().date_naive();
        by_month.entry((d.year(), d.month())).or_default().push(e);
    }
    for mut month in by_month.into_values() {
        month.sort_by_key(|e| std::cmp::Reverse(e.items.len()));
        month.truncate(MAX_EVENTS_PER_MONTH);
        found.extend(month);
    }
    found.sort_by_key(|d| std::cmp::Reverse(d.start_at));

    let mut years: BTreeMap<i32, Vec<&Photo>> = BTreeMap::new();
    for p in photos {
        let d = p.date();
        if (d.month(), d.day()) == (today.month(), today.day()) && d.year() < today.year() {
            years.entry(d.year()).or_default().push(p);
        }
    }
    let mut on_this_day: Vec<Draft> = years
        .into_iter()
        .rev()
        .filter(|(_, items)| items.len() >= ON_THIS_DAY_MIN_ITEMS)
        .map(|(year, items)| {
            let ago = today.year() - year;
            let subtitle = format!("{year} · {ago} {}", if ago == 1 { "year ago" } else { "years ago" });
            draft(KIND_ON_THIS_DAY, "On this day".into(), Some(subtitle), &items, centroid(items.iter().copied()))
        })
        .collect();
    on_this_day.extend(found);
    on_this_day
}

/// Every photo with a capture time, in time order: one indexed scan.
pub fn load_photos(conn: &Connection) -> Result<Vec<Photo>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT a.id, a.captured_at, a.gps_lat, a.gps_lon, a.is_favorite, a.rating, a.sharpness, a.phash,
                COALESCE(NULLIF(a.locality, ''), NULLIF(a.place_name, '')) FROM assets a
         WHERE a.state = 0 AND a.concealed = 0 AND a.captured_at IS NOT NULL AND NOT {} ORDER BY a.captured_at, a.id",
        crate::assets::MOVIE_HALF
    ))?;
    let rows = stmt
        .query_map([], |r| {
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
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// Replaces every memory with a fresh clustering. A memory the user hid stays hidden as long
/// as its `(kind, start_at, end_at)` comes out the same. Returns how many were stored.
pub fn regenerate(conn: &Connection, today: NaiveDate, now: i64) -> Result<usize> {
    regenerate_with(conn, today, now, &Prototypes::empty())
}

/// `regenerate`, re-curating each memory with its photographs' embeddings. They are read one
/// memory at a time — a trip's thousand photos are half a megabyte, the library's are not.
pub fn regenerate_with(conn: &Connection, today: NaiveDate, now: i64, protos: &Prototypes) -> Result<usize> {
    let photos = load_photos(conn)?;
    let mut drafts = cluster(&photos, today);
    let features = drafts.iter().map(|d| load_features(conn, d)).collect::<Result<Vec<_>>>()?;
    curate_drafts(&photos, &mut drafts, features, protos);
    store(conn, &drafts, now)
}

/// `regenerate_with` for the app: each read takes a connection only for itself, curation runs
/// with none, and the write is one transaction, so no hold reaches the slow-call warning while
/// the user's screens wait for a connection (Round 7: 7 s in one hold before).
pub fn regenerate_on(db: &crate::Db, today: NaiveDate, now: i64, protos: &Prototypes) -> Result<usize> {
    let photos = db.with_conn(load_photos)?;
    let mut drafts = cluster(&photos, today);
    let features = drafts.iter().map(|d| db.with_conn(|c| load_features(c, d))).collect::<Result<Vec<_>>>()?;
    curate_drafts(&photos, &mut drafts, features, protos);
    db.with_conn(|c| store(c, &drafts, now))
}

/// The embeddings of one draft's photographs. The app reads each memory's on a connection of
/// its own (Round 7), so no hold lasts longer than one memory's reads.
pub fn load_features(conn: &Connection, draft: &Draft) -> Result<HashMap<i64, crate::embedding::Features>> {
    let ids: Vec<i64> = draft.items.iter().map(|(id, _, _)| *id).collect();
    crate::embedding::load(conn, &ids)
}

/// Re-curates every draft with its features (one map per draft, in order), in parallel and
/// with no connection held: curating inside one held it for 5–7 s on Julian's library (Round
/// 7). A draft with no features keeps its Phase 6 curation.
pub fn curate_drafts(photos: &[Photo], drafts: &mut [Draft], features: Vec<HashMap<i64, crate::embedding::Features>>, protos: &Prototypes) {
    use rayon::prelude::*;
    let by_id: HashMap<i64, &Photo> = photos.iter().map(|p| (p.id, p)).collect();
    drafts.par_iter_mut().zip(features.into_par_iter()).for_each(|(d, features)| {
        if features.is_empty() {
            return;
        }
        let mut members: Vec<&Photo> = d.items.iter().filter_map(|(id, _, _)| by_id.get(id).copied()).collect();
        members.sort_by_key(|p| (p.local, p.id));
        d.items = curate_with(&members, &features, protos);
    });
}

/// Replaces every memory with `drafts` in one transaction. A memory the user hid stays hidden
/// as long as its `(kind, start_at, end_at)` comes out the same. Returns how many were stored.
pub fn store(conn: &Connection, drafts: &[Draft], now: i64) -> Result<usize> {
    let tx = conn.unchecked_transaction()?;
    let hidden: HashSet<(i64, i64, i64)> = tx
        .prepare("SELECT kind, start_at, end_at FROM memories WHERE is_hidden = 1")?
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
        .collect::<std::result::Result<_, _>>()?;
    // Films are kept by the same identity: deleting the memories cascades to `memory_films`, and
    // without this every launch rebuilt every film. `get_fresh` still compares the photos.
    type Film = (String, String, Option<i64>, i64);
    let mut films: HashMap<(i64, i64, i64), Film> = tx
        .prepare(
            "SELECT m.kind, m.start_at, m.end_at, f.source_key, f.doc_json, f.bpm, f.built_at
             FROM memory_films f JOIN memories m ON m.id = f.memory_id",
        )?
        .query_map([], |r| Ok(((r.get(0)?, r.get(1)?, r.get(2)?), (r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?))))?
        .collect::<std::result::Result<_, _>>()?;
    tx.execute("DELETE FROM memory_items", [])?;
    tx.execute("DELETE FROM memories", [])?;
    let mut stored = 0;
    {
        let mut memory = tx.prepare(
            "INSERT OR IGNORE INTO memories (kind, title, subtitle, start_at, end_at, centroid_lat, centroid_lon, cover_asset_id, generated_at, is_hidden)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        )?;
        let mut item = tx.prepare("INSERT INTO memory_items (memory_id, asset_id, score, curated) VALUES (?1, ?2, ?3, ?4)")?;
        let mut film = tx.prepare(
            "INSERT INTO memory_films (memory_id, source_key, doc_json, bpm, built_at) VALUES (?1, ?2, ?3, ?4, ?5)",
        )?;
        for d in drafts {
            let cover = d.items.first().map(|(id, _, _)| *id);
            let is_hidden = hidden.contains(&(d.kind, d.start_at, d.end_at));
            let (lat, lon) = (d.centroid.map(|c| c.0), d.centroid.map(|c| c.1));
            if memory.execute(params![d.kind, d.title, d.subtitle, d.start_at, d.end_at, lat, lon, cover, now, is_hidden])? == 0 {
                continue; // the same identity twice: keep the first
            }
            let id = tx.last_insert_rowid();
            for (asset, score, curated) in &d.items {
                item.execute(params![id, asset, score, curated])?;
            }
            if let Some((key, doc, bpm, built_at)) = films.remove(&(d.kind, d.start_at, d.end_at)) {
                film.execute(params![id, key, doc, bpm, built_at])?;
            }
            stored += 1;
        }
    }
    tx.commit()?;
    Ok(stored)
}

#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct MemorySummary {
    #[ts(type = "number")]
    pub id: i64,
    /// `KIND_TRIP`, `KIND_EVENT`, `KIND_ON_THIS_DAY`
    #[ts(type = "number")]
    pub kind: i64,
    pub title: String,
    pub subtitle: Option<String>,
    /// Local wall-clock seconds, like `captured_at`.
    #[ts(type = "number")]
    pub start_at: i64,
    #[ts(type = "number")]
    pub end_at: i64,
    #[ts(type = "number | null")]
    pub cover_asset_id: Option<i64>,
    pub item_count: u32,
    pub curated_count: u32,
    pub hidden: bool,
}

const SUMMARY: &str = "SELECT m.id, m.kind, m.title, m.subtitle, m.start_at, m.end_at, m.cover_asset_id,
        (SELECT COUNT(*) FROM memory_items i WHERE i.memory_id = m.id),
        (SELECT COUNT(*) FROM memory_items i WHERE i.memory_id = m.id AND i.curated = 1),
        m.is_hidden
     FROM memories m";

fn summary(r: &rusqlite::Row<'_>) -> rusqlite::Result<MemorySummary> {
    Ok(MemorySummary {
        id: r.get(0)?,
        kind: r.get(1)?,
        title: r.get(2)?,
        subtitle: r.get(3)?,
        start_at: r.get(4)?,
        end_at: r.get(5)?,
        cover_asset_id: r.get(6)?,
        item_count: r.get(7)?,
        curated_count: r.get(8)?,
        hidden: r.get::<_, i64>(9)? != 0,
    })
}

/// On this day first, then everything newest first — the order `cluster` produces.
pub fn list_memories(conn: &Connection, include_hidden: bool) -> Result<Vec<MemorySummary>> {
    let filter = if include_hidden { "" } else { "WHERE m.is_hidden = 0" };
    let sql = format!("{SUMMARY} {filter} ORDER BY m.kind = {KIND_ON_THIS_DAY} DESC, m.start_at DESC, m.id");
    let rows = conn.prepare(&sql)?.query_map([], summary)?.collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(rows)
}

pub fn get_memory(conn: &Connection, id: i64) -> Result<MemorySummary> {
    conn.query_row(&format!("{SUMMARY} WHERE m.id = ?1"), [id], summary)
        .optional()?
        .ok_or_else(|| CoreError::NotFound(format!("memory {id}")))
}

/// The curated set, best first: what *Play* shows.
pub fn curated_ids(conn: &Connection, id: i64) -> Result<Vec<i64>> {
    let mut stmt = conn.prepare("SELECT asset_id FROM memory_items WHERE memory_id = ?1 AND curated = 1 ORDER BY score DESC, asset_id")?;
    let rows = stmt.query_map([id], |r| r.get(0))?.collect::<std::result::Result<Vec<i64>, _>>()?;
    Ok(rows)
}

pub fn set_hidden(conn: &Connection, id: i64, hidden: bool) -> Result<()> {
    conn.execute("UPDATE memories SET is_hidden = ?2 WHERE id = ?1", params![id, hidden])?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDateTime;

    const LJUBLJANA: (f64, f64) = (46.0569, 14.5058);
    const LISBON: (f64, f64) = (38.7223, -9.1393);

    fn at(s: &str) -> i64 {
        NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M").unwrap().and_utc().timestamp()
    }

    /// The town the atlas stores for a position ("Lisbon, Portugal"), as maintenance would.
    fn stored(gps: Option<(f64, f64)>) -> Option<String> {
        let p = crate::places::reverse_geocode(gps?.0, gps?.1)?;
        Some([p.city, p.country].into_iter().flatten().collect::<Vec<_>>().join(", "))
    }

    fn photo(id: i64, when: &str, gps: Option<(f64, f64)>) -> Photo {
        Photo { id, local: at(when), lat: gps.map(|g| g.0), lon: gps.map(|g| g.1), favorite: false, rating: 0, sharpness: None, phash: None, place: stored(gps) }
    }

    /// Thirty days at home, one photo a day at noon.
    fn home_month(first_id: i64) -> Vec<Photo> {
        (0..30).map(|d| photo(first_id + d, &format!("2024-01-{:02} 12:00", d + 1), Some(LJUBLJANA))).collect()
    }

    fn sorted(mut v: Vec<Photo>) -> Vec<Photo> {
        v.sort_by_key(|p| p.local);
        v
    }

    #[test]
    fn a_gap_of_more_than_twelve_hours_starts_a_new_session() {
        let p = sorted(vec![photo(1, "2024-03-12 10:00", None), photo(2, "2024-03-12 21:00", None), photo(3, "2024-03-13 08:00", None)]);
        assert_eq!(sessions(&p).len(), 1);
        let p = sorted(vec![photo(1, "2024-03-12 10:00", None), photo(2, "2024-03-13 00:01", None)]);
        assert_eq!(sessions(&p).len(), 2);
    }

    #[test]
    fn home_is_the_densest_cell_of_the_last_year() {
        let mut p = home_month(1);
        p.extend((0..12).map(|i| photo(100 + i, &format!("2024-03-12 {:02}:00", 8 + i), Some(LISBON))));
        let (lat, lon) = home(&sorted(p)).unwrap();
        assert!(crate::places::haversine_km(lat, lon, LJUBLJANA.0, LJUBLJANA.1) < 5.0);
        assert_eq!(home(&[photo(1, "2024-01-01 12:00", None)]), None);
    }

    #[test]
    fn a_far_away_session_is_a_trip_named_after_where_it_was() {
        let mut p = home_month(1);
        // Every five hours for three days in Lisbon: one session.
        p.extend((0..13).map(|i| {
            let t = at("2024-03-12 08:00") + i * 5 * 3600;
            Photo { local: t, ..photo(100 + i, "2024-03-12 08:00", Some(LISBON)) }
        }));
        let drafts = cluster(&sorted(p), chrono::NaiveDate::from_ymd_opt(2026, 9, 19).unwrap());
        let trips: Vec<&Draft> = drafts.iter().filter(|d| d.kind == KIND_TRIP).collect();
        assert_eq!(trips.len(), 1, "{drafts:#?}");
        assert_eq!(trips[0].title, "Trip to Lisbon");
        assert_eq!(trips[0].subtitle.as_deref(), Some("12–14 March 2024"));
        assert_eq!(trips[0].items.len(), 13);
    }

    #[test]
    fn a_busy_day_near_home_is_an_event_and_a_scattered_one_is_not() {
        let near = |i: i64| (LJUBLJANA.0 + 0.001 * (i % 3) as f64, LJUBLJANA.1);
        let mut p: Vec<Photo> = (0..12).map(|i| photo(i, &format!("2024-05-04 {:02}:{:02}", 10 + i / 3, (i % 3) * 20), Some(near(i)))).collect();
        // Twelve photos spread over 30 km on another day.
        p.extend((0..12).map(|i| photo(100 + i, &format!("2024-06-08 {:02}:00", 8 + i), Some((LJUBLJANA.0 + 0.025 * i as f64, LJUBLJANA.1)))));
        // Eleven photos on a third day: too few.
        p.extend((0..11).map(|i| photo(200 + i, &format!("2024-07-13 {:02}:00", 8 + i), Some(LJUBLJANA))));
        let drafts = cluster(&sorted(p), chrono::NaiveDate::from_ymd_opt(2026, 9, 19).unwrap());
        let events: Vec<&Draft> = drafts.iter().filter(|d| d.kind == KIND_EVENT).collect();
        assert_eq!(events.len(), 1, "{drafts:#?}");
        assert_eq!(events[0].title, "Ljubljana");
        assert_eq!(events[0].subtitle.as_deref(), Some("Saturday, 4 May 2024"));
    }

    #[test]
    fn an_event_without_gps_is_named_by_its_date() {
        let p: Vec<Photo> = (0..12).map(|i| photo(i, &format!("2024-05-04 {:02}:00", 8 + i), None)).collect();
        let drafts = cluster(&p, chrono::NaiveDate::from_ymd_opt(2026, 9, 19).unwrap());
        assert_eq!(drafts.len(), 1);
        assert_eq!(drafts[0].title, "Saturday, 4 May 2024");
        assert_eq!(drafts[0].subtitle.as_deref(), Some("12 photos"));
    }

    #[test]
    fn a_trip_is_named_after_the_away_part_and_needs_most_of_it_away() {
        let session = |first_id: i64, home_n: i64, away_n: i64| -> Vec<Photo> {
            (0..home_n + away_n)
                .map(|i| {
                    let t = at("2024-03-12 06:00") + i * 3 * 3600;
                    Photo { local: t, ..photo(first_id + i, "2024-03-12 06:00", Some(if i < home_n { LJUBLJANA } else { LISBON })) }
                })
                .collect()
        };
        // Leaving from home: 4 photos there, then 9 in Lisbon — a trip to Lisbon, not home.
        let mut p = home_month(1);
        p.extend(session(100, 4, 9));
        let drafts = cluster(&sorted(p), chrono::NaiveDate::from_ymd_opt(2026, 9, 19).unwrap());
        assert_eq!(drafts.iter().find(|d| d.kind == KIND_TRIP).map(|d| d.title.as_str()), Some("Trip to Lisbon"));
        // Mostly at home with a few photos far away: not a trip.
        let mut p = home_month(1);
        p.extend(session(100, 9, 4));
        let drafts = cluster(&sorted(p), chrono::NaiveDate::from_ymd_opt(2026, 9, 19).unwrap());
        assert!(drafts.iter().all(|d| d.kind != KIND_TRIP), "{drafts:#?}");
    }

    #[test]
    fn a_month_keeps_only_its_two_biggest_events() {
        // Four busy days in May at home: 20, 14, 30 and 12 photos.
        let mut p = Vec::new();
        for (day, n) in [(4, 20), (11, 14), (18, 30), (25, 12)] {
            p.extend((0..n).map(|i| photo(day * 100 + i, &format!("2024-05-{day:02} {:02}:{:02}", 8 + i / 6, (i % 6) * 10), None)));
        }
        let drafts = cluster(&sorted(p), chrono::NaiveDate::from_ymd_opt(2026, 9, 19).unwrap());
        let sizes: Vec<usize> = drafts.iter().map(|d| d.items.len()).collect();
        assert_eq!(sizes, [30, 20], "newest first, the smaller two dropped");
    }

    #[test]
    fn a_trip_is_named_after_where_most_of_it_happened() {
        // Home Ljubljana; a trip with 10 photos in Lisbon and 3 in Porto: the average position
        // is between them, but the trip is to Lisbon.
        const PORTO: (f64, f64) = (41.1579, -8.6291);
        let mut p = home_month(1);
        p.extend((0..13).map(|i| {
            let t = at("2024-03-12 08:00") + i * 5 * 3600;
            Photo { local: t, ..photo(100 + i, "2024-03-12 08:00", Some(if i < 10 { LISBON } else { PORTO })) }
        }));
        let drafts = cluster(&sorted(p), chrono::NaiveDate::from_ymd_opt(2026, 9, 19).unwrap());
        assert_eq!(drafts.iter().find(|d| d.kind == KIND_TRIP).unwrap().title, "Trip to Lisbon");
    }

    /// Exact place names: a memory is titled from the names stored on its photos, never from
    /// the atlas, which would call Idrija "Gorizia".
    #[test]
    fn places_come_from_stored_names_not_the_atlas() {
        const IDRIJA: (f64, f64) = (46.0006, 14.0264);
        let named = |id: i64, place: Option<&str>| Photo { place: place.map(str::to_string), ..photo(id, "2024-03-12 08:00", Some(IDRIJA)) };
        let mut photos = [named(1, Some("Idrija, Slovenia")), named(2, Some("Idrija, Slovenia")), named(3, Some("Jelični Vrh, Slovenia"))];
        let refs: Vec<&Photo> = photos.iter().collect();
        assert_eq!(common_place(&refs).as_deref(), Some("Idrija"));
        photos.iter_mut().for_each(|p| p.place = None);
        let refs: Vec<&Photo> = photos.iter().collect();
        assert_eq!(common_place(&refs), None, "no stored name: no guess");
        let country = [named(4, Some("Iceland"))];
        assert_eq!(common_place(&country.iter().collect::<Vec<_>>()).as_deref(), Some("Iceland"));
    }

    #[test]
    fn on_this_day_looks_back_one_memory_per_year() {
        let mut p: Vec<Photo> = (0..4).map(|i| photo(i, &format!("2024-03-12 {:02}:00", 9 + i), None)).collect();
        p.extend((0..2).map(|i| photo(10 + i, &format!("2025-03-12 {:02}:00", 9 + i), None)));
        let drafts = cluster(&sorted(p), chrono::NaiveDate::from_ymd_opt(2026, 3, 12).unwrap());
        let days: Vec<&Draft> = drafts.iter().filter(|d| d.kind == KIND_ON_THIS_DAY).collect();
        assert_eq!(days.len(), 1);
        assert_eq!(days[0].title, "On this day");
        assert_eq!(days[0].subtitle.as_deref(), Some("2024 · 2 years ago"));
        assert_eq!(drafts[0].kind, KIND_ON_THIS_DAY, "on this day comes first");
    }

    fn photo_at(id: i64, secs: i64, lat: f64, sharp: f32, phash: u64) -> Photo {
        Photo { id, local: secs, lat: Some(lat), lon: Some(14.5), favorite: false, rating: 0, sharpness: Some(sharp), phash: Some(phash), place: None }
    }

    #[test]
    fn highlights_cover_the_whole_occasion_not_just_its_busiest_hour() {
        // Forty photographs in one hour, six spread over the five hours after it.
        let mut photos: Vec<Photo> = (0..40).map(|i| photo_at(i + 1, i * 60, 46.0, 100.0, (i as u64) << 20)).collect();
        photos.extend((0..6).map(|i| photo_at(100 + i, 3600 + i * 3000, 46.0, 100.0, 0xDEAD_0000_0000 | ((i as u64) << 33))));
        let refs: Vec<&Photo> = photos.iter().collect();
        let curated: Vec<i64> = curate(&refs).into_iter().filter(|(_, _, c)| *c).map(|(id, _, _)| id).collect();
        let late = curated.iter().filter(|id| **id >= 100).count();
        assert!(late >= 3, "the quiet hours must be represented: {curated:?}");
    }

    #[test]
    fn highlights_visit_more_than_one_place() {
        // Two clusters 30 km apart; one has three times the photographs.
        let mut photos: Vec<Photo> = (0..30).map(|i| photo_at(i + 1, i * 120, 46.0, 100.0, (i as u64) << 20)).collect();
        photos.extend((0..10).map(|i| photo_at(100 + i, i * 120 + 60, 46.3, 100.0, 0xBEEF_0000_0000 | ((i as u64) << 33))));
        let refs: Vec<&Photo> = photos.iter().collect();
        let curated: Vec<i64> = curate(&refs).into_iter().filter(|(_, _, c)| *c).map(|(id, _, _)| id).collect();
        assert!(curated.iter().any(|id| *id >= 100), "the smaller place is still shown: {curated:?}");
    }

    #[test]
    fn the_best_photograph_still_leads() {
        let mut photos: Vec<Photo> = (0..20).map(|i| photo_at(i + 1, i * 300, 46.0, 50.0, (i as u64) << 20)).collect();
        photos[7].favorite = true;
        photos[7].rating = 5;
        let refs: Vec<&Photo> = photos.iter().collect();
        assert_eq!(curate(&refs)[0].0, 8, "the favourite five-star photo is the first highlight");
    }

    #[test]
    fn one_very_sharp_photo_does_not_flatten_the_rest() {
        // Nineteen ordinary photographs and one absurdly sharp: the ordinary ones must still
        // be told apart by their own sharpness.
        let mut photos: Vec<Photo> = (0..20).map(|i| photo_at(i + 1, i * 3000, 46.0, 10.0 + i as f32, (i as u64) << 20)).collect();
        photos[0].sharpness = Some(100_000.0);
        let refs: Vec<&Photo> = photos.iter().collect();
        let ranked = curate(&refs);
        let of = |id: i64| ranked.iter().find(|(i, _, _)| *i == id).unwrap().1;
        assert!(of(20) > of(2), "the sharper of two ordinary photos still wins: {} vs {}", of(20), of(2));
    }

    #[test]
    fn near_duplicates_do_not_both_become_highlights() {
        let h = 0x0F0F_0F0F_0F0F_0F0Fu64;
        let mut photos: Vec<Photo> = (0..30).map(|i| photo_at(i + 1, i * 3000, 46.0, 50.0, (i as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15))).collect();
        photos[0].phash = Some(h);
        photos[1].phash = Some(h ^ 0b111);
        photos[0].rating = 5;
        photos[1].rating = 5;
        let refs: Vec<&Photo> = photos.iter().collect();
        let curated: Vec<i64> = curate(&refs).into_iter().filter(|(_, _, c)| *c).map(|(id, _, _)| id).collect();
        assert!(!(curated.contains(&1) && curated.contains(&2)), "only one of the pair: {curated:?}");
    }

    #[test]
    fn curation_is_stable_and_complete() {
        let photos: Vec<Photo> = (0..50)
            .map(|i| photo_at(i + 1, i * 400, 46.0, (i % 7) as f32 * 10.0, (i as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15)))
            .collect();
        let refs: Vec<&Photo> = photos.iter().collect();
        let a = curate(&refs);
        assert_eq!(a, curate(&refs), "the same photos give the same highlights");
        let mut ids: Vec<i64> = a.iter().map(|(id, _, _)| *id).collect();
        ids.sort();
        assert_eq!(ids, (1..=50).collect::<Vec<_>>(), "every photo is ranked, curated or not");
    }

    #[test]
    fn curation_prefers_favourites_and_sharp_photos_and_demotes_near_duplicates() {
        let photos: Vec<Photo> = (1..=12)
            .map(|i| Photo {
                favorite: i == 5,
                sharpness: Some(match i {
                    7 => 90.0,
                    8 => 80.0,
                    _ => 10.0 + i as f32,
                }),
                phash: Some(if i == 8 { 7u64.wrapping_mul(0x9E37_79B9_7F4A_7C15) } else { (i as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) }),
                ..photo(i, "2024-05-04 10:00", None)
            })
            .collect();
        let refs: Vec<&Photo> = photos.iter().collect();
        let ranked = curate(&refs);
        assert_eq!(ranked[0].0, 5, "the favourite leads");
        assert_eq!(ranked[1].0, 7, "then the sharpest");
        assert_eq!(ranked.iter().filter(|(_, _, c)| *c).count(), 8);
        let eight = ranked.iter().find(|(id, _, _)| *id == 8).unwrap();
        assert!(!eight.2, "a near-duplicate of 7 is not curated");
        // It is still a sharp photograph, though, and the score says so: being a repeat
        // keeps it out of the highlights without pretending it is a bad picture.
        let uncurated: Vec<i64> = ranked.iter().filter(|(_, _, c)| !*c).map(|(id, _, _)| *id).collect();
        assert_eq!(uncurated[0], 8, "the best of the rest: {uncurated:?}");
    }

    #[test]
    fn date_ranges_read_naturally() {
        let d = |s: &str| chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap();
        assert_eq!(date_range(d("2024-03-12"), d("2024-03-12")), "12 March 2024");
        assert_eq!(date_range(d("2024-03-12"), d("2024-03-19")), "12–19 March 2024");
        assert_eq!(date_range(d("2024-03-28"), d("2024-04-02")), "28 March – 2 April 2024");
        assert_eq!(date_range(d("2023-12-30"), d("2024-01-03")), "30 Dec 2023 – 3 Jan 2024");
    }

    #[test]
    #[ignore = "timing; run with --release"]
    fn clusters_a_quarter_million_photos_quickly() {
        let mut x: u64 = 7;
        let mut next = || {
            x = x.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            x >> 11
        };
        let start = at("2018-01-01 00:00");
        let mut photos: Vec<Photo> = (0..250_000)
            .map(|id| {
                let local = start + (next() % (8 * 365 * 86_400)) as i64;
                let gps = (next() % 3 == 0).then(|| {
                    let (lat, lon) = if next() % 5 == 0 { LISBON } else { LJUBLJANA };
                    (lat + (next() % 1000) as f64 / 20_000.0, lon + (next() % 1000) as f64 / 20_000.0)
                });
                Photo { id, local, lat: gps.map(|g| g.0), lon: gps.map(|g| g.1), favorite: next() % 20 == 0, rating: (next() % 6) as i64, sharpness: Some((next() % 500) as f32), phash: Some(next()), place: None }
            })
            .collect();
        photos.sort_by_key(|p| p.local);
        let t = std::time::Instant::now();
        let drafts = cluster(&photos, chrono::NaiveDate::from_ymd_opt(2026, 3, 12).unwrap());
        eprintln!("cluster 250k: {:?}, {} memories", t.elapsed(), drafts.len());
        assert!(t.elapsed() < std::time::Duration::from_secs(1));
    }

    /// A catalogue with a Lisbon trip (12 photos) and a busy day at home (12 photos).
    fn library() -> crate::Db {
        let db = crate::Db::open_in_memory().unwrap();
        db.with_conn(|c| {
            let mut id = 0;
            let mut add = |when: i64, gps: (f64, f64)| -> crate::Result<()> {
                id += 1;
                c.execute(
                    "INSERT INTO assets (id, content_hash, relative_path, filename, extension, media_type, byte_size, orientation, imported_at, captured_at, gps_lat, gps_lon, place_name)
                     VALUES (?1, ?2, 'p', 'f.jpg', 'jpg', 0, 1, 1, 1, ?3, ?4, ?5, ?6)",
                    rusqlite::params![id, format!("h{id}"), when, gps.0, gps.1, stored(Some(gps))],
                )?;
                Ok(())
            };
            for d in 0..30 {
                add(at("2024-01-01 12:00") + d * 86_400, LJUBLJANA)?;
            }
            for i in 0..12 {
                add(at("2024-03-12 08:00") + i * 4 * 3600, LISBON)?;
            }
            for i in 0..12 {
                add(at("2024-05-04 10:00") + i * 600, LJUBLJANA)?;
            }
            Ok(())
        })
        .unwrap();
        db
    }

    #[test]
    fn regenerating_stores_memories_and_keeps_what_was_hidden() {
        let db = library();
        let today = NaiveDate::from_ymd_opt(2026, 9, 19).unwrap();
        db.with_conn(|c| {
            assert_eq!(regenerate(c, today, 1)?, 2);
            let all = list_memories(c, false)?;
            let titles: Vec<&str> = all.iter().map(|m| m.title.as_str()).collect();
            assert_eq!(titles, ["Ljubljana", "Trip to Lisbon"], "newest first");
            let trip = &all[1];
            assert_eq!((trip.item_count, trip.curated_count), (12, 8));
            assert!(trip.cover_asset_id.is_some());
            assert_eq!(curated_ids(c, trip.id)?.len(), 8);

            set_hidden(c, trip.id, true)?;
            assert_eq!(regenerate(c, today, 2)?, 2);
            assert_eq!(list_memories(c, false)?.len(), 1);
            let hidden = list_memories(c, true)?.into_iter().find(|m| m.title == "Trip to Lisbon").unwrap();
            assert!(hidden.hidden, "hiding survives regeneration");

            // A memory whose photos are gone is gone after the next regeneration.
            c.execute("UPDATE assets SET state = 1 WHERE captured_at >= ?1", [at("2024-05-04 00:00")])?;
            regenerate(c, today, 3)?;
            assert!(list_memories(c, true)?.iter().all(|m| m.title != "Ljubljana"));
            Ok(())
        })
        .unwrap();
    }

    /// Round 7: the app's path (reads, curation with no connection, one write) stores what the
    /// one-connection path stores, embeddings included.
    #[test]
    fn regenerating_in_steps_stores_the_same_memories() {
        let db = library();
        let today = NaiveDate::from_ymd_opt(2026, 9, 19).unwrap();
        let ids: Vec<i64> = db.with_conn(|c| Ok(c.prepare("SELECT id FROM assets ORDER BY id")?.query_map([], |r| r.get(0))?.collect::<std::result::Result<_, _>>()?)).unwrap();
        for (n, id) in ids.iter().enumerate() {
            let mut v = vec![0f32; crate::embedding::DIM];
            v[n % 7] = 1.0;
            v[7 + n % 5] = 0.5;
            db.with_conn(|c| crate::embedding::store(c, *id, "clip", &crate::embedding::quantize(&v), Some(n as f32 / 10.0))).unwrap();
        }
        let items = |db: &crate::Db| -> Vec<(String, i64, i64, bool)> {
            db.with_conn(|c| {
                let mut stmt = c.prepare("SELECT m.title, i.asset_id, CAST(i.score * 1000 AS INTEGER), i.curated FROM memory_items i JOIN memories m ON m.id = i.memory_id ORDER BY m.title, i.asset_id")?;
                let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?.collect::<std::result::Result<_, _>>()?;
                Ok(rows)
            })
            .unwrap()
        };
        let protos = Prototypes::empty();
        assert_eq!(db.with_conn(|c| regenerate_with(c, today, 1, &protos)).unwrap(), 2);
        let held = items(&db);
        assert!(!held.is_empty());
        assert_eq!(regenerate_on(&db, today, 1, &protos).unwrap(), 2);
        assert_eq!(items(&db), held);
    }

    #[test]
    fn a_memory_scope_is_its_curated_set() {
        let db = library();
        db.with_conn(|c| {
            regenerate(c, NaiveDate::from_ymd_opt(2026, 9, 19).unwrap(), 1)?;
            let trip = list_memories(c, false)?.into_iter().find(|m| m.kind == KIND_TRIP).unwrap();
            let scope = crate::LayoutScope { memory_id: Some(trip.id), ..Default::default() };
            assert!(!scope.sectioned());
            let idx = crate::layout::LayoutIndex::load_scoped(c, &scope, false)?;
            let mut got: Vec<i64> = idx.items.iter().map(|i| i.id).collect();
            let mut want = curated_ids(c, trip.id)?;
            got.sort();
            want.sort();
            assert_eq!(got, want);
            Ok(())
        })
        .unwrap();
    }


    // ---- Phase 7a: curation that sees the pictures ----

    use crate::embedding::{quantize, Features, Prototypes, Vector, DIM};

    fn tilt(axis: usize, other: usize, t: f32) -> Vector {
        let mut v = vec![0f32; DIM];
        v[axis] = (1.0 - t * t).sqrt();
        v[other] = t;
        quantize(&v)
    }

    fn pic(id: i64, local: i64) -> Photo {
        Photo { id, local, lat: Some(46.0), lon: Some(14.5), favorite: false, rating: 0, sharpness: Some(1.0), phash: None, place: None }
    }

    fn feats(pairs: &[(i64, Vector, Option<f32>)]) -> HashMap<i64, Features> {
        pairs.iter().map(|(id, v, a)| (*id, Features { vec: v.clone(), aesthetic: *a })).collect()
    }

    #[test]
    fn a_better_looking_photograph_wins_a_tie() {
        let photos: Vec<Photo> = (0..12).map(|i| pic(i, 1_700_000_000 + i * 3600)).collect();
        let refs: Vec<&Photo> = photos.iter().collect();
        let f = feats(&[(7, tilt(0, 1, 0.0), Some(0.95))]);
        let out = curate_with(&refs, &f, &Prototypes::empty());
        assert_eq!(out[0].0, 7, "the aesthetic lead should be the cover");
    }

    #[test]
    fn a_screenshot_is_not_a_highlight() {
        let protos = Prototypes { model: "m".into(), items: vec![("junk:screenshot".into(), tilt(5, 6, 0.0)), ("good".into(), tilt(6, 5, 0.0))] };
        // Everyone has a star; the screenshot has two and would otherwise lead.
        let mut photos: Vec<Photo> = (0..10).map(|i| Photo { rating: 1, ..pic(i, 1_700_000_000 + i * 3600) }).collect();
        photos[3].rating = 2;
        let refs: Vec<&Photo> = photos.iter().collect();
        let f = feats(&[(3, tilt(5, 6, 0.05), None)]);
        let out = curate_with(&refs, &f, &protos);
        assert_ne!(out[0].0, 3);
        assert!(out.iter().find(|x| x.0 == 3).unwrap().1 < out[0].1);
        assert!(junk(&tilt(5, 6, 0.05), &protos) > 0.5);
        assert_eq!(junk(&tilt(6, 5, 0.0), &protos), 0.0, "a good photograph is not junk");
    }

    /// Burst frames that moved enough to fool the 64-bit hash are still one moment.
    #[test]
    fn near_identical_content_is_a_veto_even_when_the_hashes_differ() {
        let photos: Vec<Photo> = (0..16).map(|i| pic(i, 1_700_000_000 + i * 7200)).collect();
        let refs: Vec<&Photo> = photos.iter().collect();
        let twin = tilt(0, 1, 0.0);
        let f = feats(&[(0, twin.clone(), Some(0.9)), (1, twin, Some(0.9))]);
        let chosen: Vec<i64> = curate_with(&refs, &f, &Prototypes::empty()).into_iter().filter(|x| x.2).map(|x| x.0).collect();
        assert!(!(chosen.contains(&0) && chosen.contains(&1)), "{chosen:?}");
    }

    /// Two subjects shot in the same minutes and place: the second subject repeats the first
    /// only half as much as another frame of the first would, so it is the one taken.
    #[test]
    fn a_different_subject_in_the_same_minutes_is_less_redundant() {
        let mut photos: Vec<Photo> = (0..3).map(|i| pic(i, 1_700_000_000 + i * 60)).collect();
        // Eight photographs in all, so every one is curated and the pick order is the ranking.
        photos.extend((3..8).map(|i| pic(i, 1_700_100_000 + i * 3600)));
        photos[0].favorite = true;
        let refs: Vec<&Photo> = photos.iter().collect();
        let f = feats(&[(0, tilt(0, 1, 0.0), None), (1, tilt(0, 1, 0.3), None), (2, tilt(2, 3, 0.0), None)]);
        let out = curate_with(&refs, &f, &Prototypes::empty());
        let rank = |id: i64| out.iter().position(|x| x.0 == id).unwrap();
        assert!(rank(2) < rank(1), "{out:?}");
    }

    #[test]
    fn without_embeddings_nothing_changes() {
        let photos: Vec<Photo> = (0..20).map(|i| Photo { rating: i % 5, ..pic(i, 1_700_000_000 + i * 900) }).collect();
        let refs: Vec<&Photo> = photos.iter().collect();
        assert_eq!(curate_with(&refs, &HashMap::new(), &Prototypes::empty()), curate(&refs));
    }
}
