//! The top of a search's results page (Round 4K): what the text was read as, and the people,
//! places, albums and memories it names, above the photos.

use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::albums::AlbumSummary;
use crate::memories::MemorySummary;
use crate::people::PersonSummary;
use crate::pets::PetSummary;
use crate::query::{AssetQuery, PlaceRef};
use crate::query_parse::{fold, is_filler, Chip, ChipKind, Parsed};
use crate::scope::LayoutScope;
use crate::Result;

/// How many of each kind the page is given; it shows one row and links to the rest.
const MAX_EACH: usize = 12;

/// A place the search names, with where its photos were taken.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PlaceSummary {
    pub place: PlaceRef,
    pub label: String,
    #[ts(type = "number")]
    pub photo_count: i64,
    /// The middle of its photos that carry GPS, for the map.
    pub lat: Option<f64>,
    pub lon: Option<f64>,
    #[ts(type = "number | null")]
    pub cover_asset_id: Option<i64>,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SearchOverview {
    pub chips: Vec<Chip>,
    /// The words left for full text and meaning.
    pub rest: String,
    pub people: Vec<PersonSummary>,
    /// The pets it names (Round 4C).
    pub pets: Vec<PetSummary>,
    pub places: Vec<PlaceSummary>,
    pub albums: Vec<AlbumSummary>,
    pub memories: Vec<MemorySummary>,
    /// Photos the whole search finds; filled in by the app, which knows the meaning hits.
    #[ts(type = "number")]
    pub total: i64,
}

/// Words worth matching against album and memory titles: three letters or more, not filler.
fn words(text: &str) -> Vec<String> {
    fold(text).split(|c: char| !c.is_alphanumeric()).filter(|w| w.chars().count() >= 3 && !is_filler(w)).map(str::to_string).collect()
}

fn contains_word(title: &str, words: &[String]) -> bool {
    let t = fold(title);
    words.iter().any(|w| t.split(|c: char| !c.is_alphanumeric()).any(|tw| tw.starts_with(w.as_str())))
}

/// `place` with its photo count, the middle of its GPS and its newest photo as the cover
/// (the search page's place card, and Home's).
pub fn place_summary(conn: &Connection, place: &PlaceRef) -> Result<PlaceSummary> {
    let scope = LayoutScope { query: Some(AssetQuery { places: vec![place.clone()], ..Default::default() }), ..Default::default() };
    let (from_where, order, params) = scope.sql();
    let (count, lat, lon): (i64, Option<f64>, Option<f64>) = conn.query_row(
        &format!("SELECT COUNT(*), AVG(a.gps_lat), AVG(a.gps_lon) {from_where}"),
        rusqlite::params_from_iter(params.iter()),
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    )?;
    let cover: Option<i64> = conn
        .query_row(&format!("SELECT a.id {from_where} ORDER BY {order} LIMIT 1"), rusqlite::params_from_iter(params.iter()), |r| r.get(0))
        .optional()?;
    Ok(PlaceSummary { place: place.clone(), label: place.label(), photo_count: count, lat, lon, cover_asset_id: cover })
}

/// The overview of `parsed` (the reading of `text`), without `total`.
pub fn overview(conn: &Connection, text: &str, parsed: &Parsed) -> Result<SearchOverview> {
    let chips_of = |kind: ChipKind| parsed.chips.iter().filter(move |c| c.kind == kind);

    let people: Vec<PersonSummary> = {
        let ids = &parsed.query.person_ids;
        let mut all = crate::people::list_people(conn)?;
        all.retain(|p| ids.contains(&p.id));
        all.sort_by_key(|p| ids.iter().position(|i| *i == p.id));
        all
    };

    let pets: Vec<PetSummary> = {
        let ids = &parsed.query.pet_ids;
        let mut all = if ids.is_empty() { Vec::new() } else { crate::pets::list_pets(conn)? };
        all.retain(|p| ids.contains(&p.id));
        all.sort_by_key(|p| ids.iter().position(|i| *i == p.id));
        all
    };

    let mut places = Vec::new();
    for place in parsed.query.places.iter().take(MAX_EACH) {
        places.push(place_summary(conn, place)?);
    }

    // Albums and memories are named by their titles: the text's words, and each place's own
    // name ("Lisbon" for Lisbon, Portugal).
    let mut title_words = words(text);
    for chip in chips_of(ChipKind::Place) {
        title_words.extend(words(chip.label.split(", ").next().unwrap_or_default()));
    }
    let albums: Vec<AlbumSummary> = crate::albums::list_albums(conn)?
        .into_iter()
        .filter(|a| parsed.query.album_ids.contains(&a.id) || contains_word(&a.name, &title_words))
        .take(MAX_EACH)
        .collect();

    // A memory may also fall in the searched dates.
    let (from, to) = (parsed.query.date_from, parsed.query.date_to);
    let memories: Vec<MemorySummary> = crate::memories::list_memories(conn, false)?
        .into_iter()
        .filter(|m| {
            let dated = (from.is_some() || to.is_some()) && from.is_none_or(|f| m.end_at >= f) && to.is_none_or(|t| m.start_at <= t);
            dated || contains_word(&m.title, &title_words)
        })
        .take(MAX_EACH)
        .collect();

    Ok(SearchOverview { chips: parsed.chips.clone(), rest: parsed.rest.clone(), people, pets, places, albums, memories, total: 0 })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::query_parse::{parse, Vocabulary};

    #[test]
    fn names_the_people_places_albums_and_memories_a_search_means() {
        let db = crate::Db::open_in_memory().unwrap();
        db.with_conn(|c| {
            c.execute_batch(
                "INSERT INTO assets (id, content_hash, relative_path, filename, extension, media_type, byte_size, orientation, captured_at, imported_at, place_name, gps_lat, gps_lon)
                   VALUES (1, 'h1', 'p', 'a.jpg', 'jpg', 0, 1, 1, 1688000000, 1, 'Lisbon, Portugal', 38.7, -9.1),
                          (2, 'h2', 'p', 'b.jpg', 'jpg', 0, 1, 1, 1689000000, 1, 'Lisbon, Portugal', 38.8, -9.2),
                          (3, 'h3', 'p', 'c.jpg', 'jpg', 0, 1, 1, 1600000000, 1, 'Porto, Portugal', NULL, NULL);
                 INSERT INTO people (id, name, created_at) VALUES (1, 'Anna', 0), (2, 'Ben', 0);
                 INSERT INTO faces (asset_id, x, y, w, h, score, vec, model, person_id) VALUES (1, 0, 0, .1, .1, .9, x'00', 'm', 1);
                 INSERT INTO albums (id, name, kind, created_at, updated_at) VALUES (5, 'Lisbon weekend', 0, 0, 0), (6, 'Garden', 0, 0, 0);
                 INSERT INTO memories (id, kind, title, start_at, end_at, generated_at) VALUES (8, 0, 'Trip to Lisbon', 1688000000, 1689000000, 0), (9, 1, 'Birthday', 1500000000, 1500003600, 0);",
            )?;
            let vocab = Vocabulary::load(c)?;
            let today = chrono::NaiveDate::from_ymd_opt(2026, 10, 3).unwrap();
            let text = "anna lisbon";
            let parsed = parse(text, &vocab, today, 0, crate::settings::DateOrder::Dmy, |_| 1);
            let o = overview(c, text, &parsed)?;
            assert_eq!(o.people.iter().map(|p| p.id).collect::<Vec<_>>(), vec![1]);
            assert_eq!(o.places.len(), 1);
            assert_eq!(o.places[0].label, "Lisbon, Portugal");
            assert_eq!(o.places[0].photo_count, 2);
            assert!((o.places[0].lat.unwrap() - 38.75).abs() < 1e-9);
            assert_eq!(o.places[0].cover_asset_id, Some(2), "the newest photo");
            assert_eq!(o.albums.iter().map(|a| a.id).collect::<Vec<_>>(), vec![5], "a title holding the place's name");
            assert_eq!(o.memories.iter().map(|m| m.id).collect::<Vec<_>>(), vec![8]);
            let o = overview(c, "red umbrella", &parse("red umbrella", &vocab, today, 0, crate::settings::DateOrder::Dmy, |_| 1))?;
            assert!(o.people.is_empty() && o.places.is_empty() && o.albums.is_empty() && o.memories.is_empty());
            assert_eq!(o.rest, "red umbrella");
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn names_the_pets_a_search_means() {
        let db = crate::Db::open_in_memory().unwrap();
        db.with_conn(|c| {
            c.execute_batch(
                "INSERT INTO assets (id, content_hash, relative_path, filename, extension, media_type, byte_size, orientation, captured_at, imported_at)
                   VALUES (1, 'h1', 'p', 'a.jpg', 'jpg', 0, 1, 1, 1688000000, 1);
                 INSERT INTO pets (id, name, species, created_at) VALUES (3, 'Milo', 'dog', 0), (4, 'Luna', 'cat', 0);
                 INSERT INTO pet_detections (asset_id, species, x, y, w, h, score, vec, model, pet_id) VALUES (1, 'dog', 0, 0, .1, .1, .9, x'00', 'm', 3);",
            )?;
            let vocab = Vocabulary::load(c)?;
            let today = chrono::NaiveDate::from_ymd_opt(2026, 10, 4).unwrap();
            let o = overview(c, "milo beach", &parse("milo beach", &vocab, today, 0, crate::settings::DateOrder::Dmy, |_| 1))?;
            assert_eq!(o.pets.iter().map(|p| (p.id, p.species.as_str())).collect::<Vec<_>>(), vec![(3, "dog")]);
            assert_eq!(o.pets[0].photo_count, 1);
            Ok(())
        })
        .unwrap();
    }
}
