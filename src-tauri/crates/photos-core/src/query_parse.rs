//! Reading a search's text (Round 4K): "Anna beach 2023" is the person Anna, the scene tag
//! *beach* and the year 2023, not three words for the full-text index. The parser matches the
//! text against a [`Vocabulary`] read from the catalogue and turns what it recognises into
//! structured [`AssetQuery`] fields; whatever it does not recognise stays free text for FTS
//! and search by meaning, exactly as before.
//!
//! The query lives in the URL as text, so a parse is derived, never stored. A reading the user
//! picks over the parser's choice is written back into the text as a token, `place:"Florence"`,
//! which the parser obeys.

use chrono::NaiveDate;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::query::{AssetKind, AssetQuery, PlaceRef};
use crate::settings::DateOrder;
use crate::Result;

/// The names a query can mean, read from the catalogue.
#[derive(Debug, Clone, Default)]
pub struct Vocabulary {
    /// Listed people with a name (`status = 'listed'`): someone removed from People is not a person.
    pub people: Vec<(i64, String)>,
    /// Pets with a name, once Phase C's table exists.
    pub pets: Vec<(i64, String)>,
    /// Places the catalogue's photos were taken at: towns, countries and every locality.
    pub places: Vec<PlaceRef>,
    /// Manual albums.
    pub albums: Vec<(i64, String)>,
    /// Collections of albums (Round 19 §7).
    pub collections: Vec<(i64, String)>,
    /// The collections an album is in, outermost first: `Trips`, `Europe` for an album in
    /// *Trips › Europe*. Absent for an album outside any collection.
    pub album_paths: std::collections::HashMap<i64, Vec<String>>,
    /// The collections a collection is in, outermost first.
    pub collection_paths: std::collections::HashMap<i64, Vec<String>>,
    /// Camera models as EXIF reports them.
    pub cameras: Vec<String>,
    /// Scene tags (`embedding::SCENE_TAGS`).
    pub tags: Vec<String>,
    /// The same names folded for matching, built on the first parse and kept with the vocabulary.
    index: std::sync::OnceLock<Index>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export)]
pub enum ChipKind {
    Person,
    Pet,
    Place,
    Album,
    /// A collection of albums: photos in any of its albums, nested ones included (Round 19 §7).
    Collection,
    Date,
    Camera,
    Tag,
    Kind,
    /// An operator or bracket of a search with `or` / `and` / `not` (Round 19): `or`, `and`,
    /// `not` (also for `-`), `(` or `)`. Never a reading of words, so never an alternative.
    Op,
}

/// One recognised part of the text, shown as a removable chip.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Chip {
    pub kind: ChipKind,
    pub label: String,
    /// Byte range of the text this chip read, so removing it rewrites the text.
    #[ts(type = "[number, number]")]
    pub span: (usize, usize),
    /// Text that forces this reading when written in place of `span`, e.g. `place:"Florence, Italy"`.
    pub token: String,
    /// For an album or collection inside collections: those, outermost first (Round 19 §4).
    #[serde(default)]
    pub path: Vec<String>,
    /// The other things the same words could mean, least likely last.
    pub alternatives: Vec<Chip>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Parsed {
    pub query: AssetQuery,
    pub chips: Vec<Chip>,
    /// What is left for full-text search and search by meaning.
    pub rest: String,
}

impl Vocabulary {
    /// Reads the vocabulary of a library: listed people, pets when Phase C's table exists, the
    /// places and cameras of visible photos, manual albums and the scene tags.
    pub fn load(conn: &Connection) -> Result<Vocabulary> {
        let pairs = |sql: &str| -> Result<Vec<(i64, String)>> {
            let mut stmt = conn.prepare(sql)?;
            let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
            Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
        };
        let strings = |sql: &str| -> Result<Vec<String>> {
            let mut stmt = conn.prepare(sql)?;
            let rows = stmt.query_map([], |r| r.get::<_, Option<String>>(0))?;
            let mut out: Vec<String> = rows.filter_map(|r| r.ok().flatten()).map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
            out.sort();
            out.dedup();
            Ok(out)
        };
        let people = pairs("SELECT id, name FROM people WHERE status = 'listed' AND name IS NOT NULL AND TRIM(name) != '' ORDER BY id")?;
        let has_pets: bool = conn.query_row("SELECT EXISTS (SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'pets')", [], |r| r.get(0))?;
        let pets = if has_pets { pairs("SELECT id, name FROM pets WHERE name IS NOT NULL AND TRIM(name) != '' AND status = 'listed' ORDER BY id")? } else { Vec::new() };
        let albums = pairs(&format!("SELECT id, name FROM albums WHERE kind = {} ORDER BY id", crate::albums::KIND_MANUAL))?;
        let collections = pairs("SELECT id, name FROM album_collections ORDER BY id")?;
        let (album_paths, collection_paths) = collection_paths(conn, &collections)?;
        let mut places: Vec<PlaceRef> = Vec::new();
        for name in strings("SELECT DISTINCT place_name FROM assets WHERE place_name IS NOT NULL AND state = 0")? {
            let found = match name.rsplit_once(", ") {
                Some((city, country)) => vec![
                    PlaceRef::City { name: city.to_string(), country: Some(country.to_string()) },
                    PlaceRef::Country { name: country.to_string() },
                ],
                // The atlas names only the country when no city is near enough.
                None => vec![PlaceRef::Country { name }],
            };
            for p in found {
                if !places.contains(&p) {
                    places.push(p);
                }
            }
        }
        // Every village and district the library's photos were taken in (Round 4K), unless
        // the town of the same name already stands for it.
        for label in strings("SELECT DISTINCT locality FROM assets WHERE locality IS NOT NULL AND locality != '' AND state = 0")? {
            let (name, country) = match label.rsplit_once(", ") {
                Some((n, c)) => (n.to_string(), Some(c.to_string())),
                None => (label.clone(), None),
            };
            let town = PlaceRef::City { name: name.clone(), country: country.clone() };
            let p = PlaceRef::Locality { name, country };
            if !places.contains(&town) && !places.contains(&p) {
                places.push(p);
            }
        }
        let cameras = strings("SELECT DISTINCT json_extract(metadata_json, '$.model') FROM assets WHERE metadata_json IS NOT NULL AND state = 0")?;
        let tags = crate::embedding::SCENE_TAGS.iter().map(|t| t.to_string()).collect();
        Ok(Vocabulary { people, pets, places, albums, collections, album_paths, collection_paths, cameras, tags, index: Default::default() })
    }

    fn index(&self) -> &Index {
        self.index.get_or_init(|| Index::new(self))
    }
}

/// Where each album and collection sits: `(album id → collections it is in, collection id →
/// collections it is in)`, outermost first. A cycle (never written) ends the walk.
#[allow(clippy::type_complexity)]
fn collection_paths(
    conn: &Connection,
    collections: &[(i64, String)],
) -> Result<(std::collections::HashMap<i64, Vec<String>>, std::collections::HashMap<i64, Vec<String>>)> {
    let names: std::collections::HashMap<i64, &str> = collections.iter().map(|(id, n)| (*id, n.as_str())).collect();
    let mut parents: std::collections::HashMap<i64, i64> = std::collections::HashMap::new();
    let mut stmt = conn.prepare("SELECT id, parent_id FROM album_collections WHERE parent_id IS NOT NULL")?;
    for row in stmt.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)))? {
        let (id, parent) = row?;
        parents.insert(id, parent);
    }
    // The chain from `start` up to the outermost collection, written outermost first.
    let chain = |start: i64| -> Vec<String> {
        let mut out = Vec::new();
        let mut at = Some(start);
        while let Some(id) = at.filter(|_| out.len() < 16) {
            out.push(names.get(&id).copied().unwrap_or_default().to_string());
            at = parents.get(&id).copied();
        }
        out.reverse();
        out
    };
    let mut albums = std::collections::HashMap::new();
    let mut stmt = conn.prepare(&format!("SELECT id, collection_id FROM albums WHERE kind = {} AND collection_id IS NOT NULL", crate::albums::KIND_MANUAL))?;
    for row in stmt.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)))? {
        let (album, collection) = row?;
        albums.insert(album, chain(collection));
    }
    let nested = collections.iter().filter_map(|(id, _)| parents.get(id).map(|p| (*id, chain(*p)))).collect();
    Ok((albums, nested))
}

/// What a reading does to the query.
#[derive(Debug, Clone, PartialEq)]
enum Effect {
    Person(i64),
    Pet(i64),
    Place(PlaceRef),
    Album(i64),
    Collection(i64),
    Tag(String),
    Kind(AssetKind),
    Videos,
    Favorites,
    Camera(String),
    Dates(i64, i64),
    Months(Vec<u8>),
}

impl Effect {
    fn apply(&self, q: &mut AssetQuery) {
        fn add<T: PartialEq + Clone>(v: &mut Vec<T>, x: &T) {
            if !v.contains(x) {
                v.push(x.clone());
            }
        }
        match self {
            Effect::Person(id) => add(&mut q.person_ids, id),
            Effect::Pet(id) => add(&mut q.pet_ids, id),
            Effect::Place(p) => add(&mut q.places, p),
            Effect::Album(id) => add(&mut q.album_ids, id),
            Effect::Collection(id) => add(&mut q.collection_ids, id),
            Effect::Tag(t) => add(&mut q.tags, t),
            Effect::Kind(k) => q.kind = Some(*k),
            Effect::Videos => q.media_type = Some(1),
            Effect::Favorites => q.favorites = true,
            Effect::Camera(c) => q.camera = Some(c.clone()),
            Effect::Dates(from, to) => {
                q.date_from = Some(*from);
                q.date_to = Some(*to);
                q.months.clear();
            }
            Effect::Months(m) => {
                q.months = m.clone();
                q.date_from = None;
                q.date_to = None;
            }
        }
    }
}

#[derive(Debug, Clone)]
struct Reading {
    kind: ChipKind,
    label: String,
    effect: Effect,
}

impl Reading {
    fn token(&self, raw: &str, path: &[String]) -> String {
        match self.kind {
            // A date reads the same written out, so its words are its token.
            ChipKind::Date => raw.to_string(),
            // Two albums named alike are told apart by where they are: `album:"Trips › Europe › Italy"`.
            k if !path.is_empty() => format!("{}:\"{} › {}\"", kind_name(k), path.join(" › "), self.label),
            k => format!("{}:\"{}\"", kind_name(k), self.label),
        }
    }
}

fn kind_name(k: ChipKind) -> &'static str {
    match k {
        ChipKind::Person => "person",
        ChipKind::Pet => "pet",
        ChipKind::Place => "place",
        ChipKind::Album => "album",
        ChipKind::Collection => "collection",
        ChipKind::Date => "date",
        ChipKind::Camera => "camera",
        ChipKind::Tag => "tag",
        ChipKind::Kind => "kind",
        // No reading has this kind; `kind_from_name` never returns it.
        ChipKind::Op => "op",
    }
}

fn kind_from_name(s: &str) -> Option<ChipKind> {
    Some(match s {
        "person" => ChipKind::Person,
        "pet" => ChipKind::Pet,
        "place" => ChipKind::Place,
        "album" => ChipKind::Album,
        "collection" => ChipKind::Collection,
        "date" => ChipKind::Date,
        "camera" => ChipKind::Camera,
        "tag" => ChipKind::Tag,
        "kind" => ChipKind::Kind,
        _ => return None,
    })
}

/// Which reading wins a tie of photo counts.
fn priority(k: ChipKind) -> u8 {
    match k {
        ChipKind::Person => 0,
        ChipKind::Pet => 1,
        ChipKind::Place => 2,
        ChipKind::Album => 3,
        ChipKind::Collection => 4,
        ChipKind::Date => 5,
        ChipKind::Tag => 6,
        ChipKind::Kind => 7,
        ChipKind::Camera => 8,
        ChipKind::Op => 9,
    }
}

/// Words that join a sentence rather than narrow it: dropped once anything was recognised, so
/// "photos of Anna at the beach" does not also ask full-text search for "photos of at the".
const FILLER: [&str; 30] = [
    "a", "an", "and", "at", "by", "for", "from", "in", "of", "on", "the", "to", "with", "photo", "photos", "picture", "pictures",
    "pic", "pics", "image", "images", "shot", "shots", "taken", "during", "near", "my", "our", "me", "show",
];

/// Whether a folded word only joins a sentence ("of", "photos").
pub fn is_filler(word: &str) -> bool {
    FILLER.contains(&word)
}

const MONTHS: [&str; 12] = ["january", "february", "march", "april", "may", "june", "july", "august", "september", "october", "november", "december"];

/// Accents folded and case lowered, so "Zürich" is "zurich" and "ANNA" is "anna".
pub fn fold(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars().flat_map(char::to_lowercase) {
        let r = match c {
            'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' | 'ā' | 'ą' | 'ă' => "a",
            'ç' | 'ć' | 'č' => "c",
            'ď' | 'đ' => "d",
            'è' | 'é' | 'ê' | 'ë' | 'ē' | 'ę' | 'ě' => "e",
            'ğ' => "g",
            'ì' | 'í' | 'î' | 'ï' | 'ī' | 'ı' => "i",
            'ł' => "l",
            'ñ' | 'ń' | 'ň' => "n",
            'ò' | 'ó' | 'ô' | 'õ' | 'ö' | 'ø' | 'ō' | 'ő' => "o",
            'ř' => "r",
            'ś' | 'š' | 'ş' | 'ș' => "s",
            'ť' | 'ț' => "t",
            'ù' | 'ú' | 'û' | 'ü' | 'ū' | 'ů' | 'ű' => "u",
            'ý' | 'ÿ' => "y",
            'ź' | 'ż' | 'ž' => "z",
            'ß' => "ss",
            'æ' => "ae",
            'œ' => "oe",
            'þ' => "th",
            _ => {
                out.push(c);
                continue;
            }
        };
        out.push_str(r);
    }
    out
}

/// A name's words, folded, apostrophes dropped: "Anna-Lena O'Neill" is `["anna", "lena", "oneill"]`.
pub(crate) fn name_words(s: &str) -> Vec<String> {
    fold(s)
        .split(|c: char| !(c.is_alphanumeric() || c == '\'' || c == '’'))
        .map(|w| w.replace(['\'', '’'], ""))
        .filter(|w| !w.is_empty())
        .collect()
}

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '\'' || c == '’'
}

#[derive(Debug, Clone)]
struct Word {
    start: usize,
    end: usize,
    norm: String,
}

#[derive(Debug, Clone)]
enum Item {
    Word(Word),
    /// `kind:"value"`, written by a chip to force a reading.
    Forced { kind: ChipKind, value: String, start: usize, end: usize },
}

fn tokenize(text: &str) -> Vec<Item> {
    let mut items = Vec::new();
    let mut chars = text.char_indices().peekable();
    while let Some(&(start, c)) = chars.peek() {
        if !is_word_char(c) {
            chars.next();
            continue;
        }
        let mut end = start;
        while let Some(&(i, c)) = chars.peek() {
            if !is_word_char(c) {
                break;
            }
            end = i + c.len_utf8();
            chars.next();
        }
        let raw = &text[start..end];
        let lower = fold(raw);
        // A forced reading: `place:"New York, United States"` or `tag:beach`.
        if let (Some(kind), true) = (kind_from_name(&lower), text[end..].starts_with(':')) {
            let after = end + 1;
            let (value, stop) = if text[after..].starts_with('"') {
                let body = after + 1;
                match text[body..].find('"') {
                    Some(q) => (&text[body..body + q], body + q + 1),
                    None => (&text[body..], text.len()),
                }
            } else {
                let len = text[after..].find(char::is_whitespace).unwrap_or(text.len() - after);
                (&text[after..after + len], after + len)
            };
            if !value.trim().is_empty() {
                items.push(Item::Forced { kind, value: value.trim().to_string(), start, end: stop });
                while chars.peek().is_some_and(|&(i, _)| i < stop) {
                    chars.next();
                }
                continue;
            }
        }
        let norm = lower.strip_suffix("'s").or_else(|| lower.strip_suffix("’s")).unwrap_or(&lower).replace(['\'', '’'], "");
        if !norm.is_empty() {
            items.push(Item::Word(Word { start, end, norm }));
        }
    }
    items
}

/// The vocabulary folded once per parse: exact phrases to readings, and names for prefixes and typos.
#[derive(Debug, Clone, Default)]
struct Index {
    exact: std::collections::HashMap<String, Vec<Reading>>,
    /// (entity, the name's folded words, the whole folded name) for people and pets.
    names: Vec<(Effect, String, Vec<String>, String)>,
    cameras: Vec<(String, String)>,
    longest: usize,
    /// Collections an album or collection is in, for chips and tokens (Round 19 §4).
    paths: std::collections::HashMap<(ChipKind, i64), Vec<String>>,
}

impl Index {
    fn new(v: &Vocabulary) -> Index {
        let mut exact: std::collections::HashMap<String, Vec<Reading>> = std::collections::HashMap::new();
        let mut longest = 1;
        let mut put = |phrase: String, r: Reading| {
            if phrase.is_empty() {
                return;
            }
            longest = longest.max(phrase.split(' ').count());
            let list = exact.entry(phrase).or_default();
            if !list.iter().any(|x| x.effect == r.effect) {
                list.push(r);
            }
        };
        let mut names = Vec::new();
        for (list, kind, make) in [(&v.people, ChipKind::Person, Effect::Person as fn(i64) -> Effect), (&v.pets, ChipKind::Pet, Effect::Pet)] {
            for (id, name) in list {
                let words = name_words(name);
                let whole = words.join(" ");
                let r = Reading { kind, label: name.clone(), effect: make(*id) };
                put(whole.clone(), r.clone());
                // "Anna" finds Anna Berg.
                if words.len() > 1 {
                    put(words[0].clone(), r.clone());
                }
                names.push((make(*id), name.clone(), words, whole));
            }
        }
        for p in &v.places {
            let (name, label) = match p {
                PlaceRef::City { name, .. } | PlaceRef::Locality { name, .. } => (name, p.label()),
                PlaceRef::Country { name } => (name, p.label()),
            };
            put(name_words(name).join(" "), Reading { kind: ChipKind::Place, label: label.clone(), effect: Effect::Place(p.clone()) });
            put(name_words(&label).join(" "), Reading { kind: ChipKind::Place, label, effect: Effect::Place(p.clone()) });
        }
        // An album or collection is also read by its path ("trips europe italy") and followed by
        // its kind ("italy album", "in the trips collection"), so the kind word is not left over.
        let mut paths = std::collections::HashMap::new();
        for (list, kind, noun, in_paths, make) in [
            (&v.albums, ChipKind::Album, "album", &v.album_paths, Effect::Album as fn(i64) -> Effect),
            (&v.collections, ChipKind::Collection, "collection", &v.collection_paths, Effect::Collection),
        ] {
            for (id, name) in list {
                let words = name_words(name).join(" ");
                let r = Reading { kind, label: name.clone(), effect: make(*id) };
                put(words.clone(), r.clone());
                put(format!("{words} {noun}"), r.clone());
                if let Some(path) = in_paths.get(id).filter(|p| !p.is_empty()) {
                    let with_path = format!("{} {words}", name_words(&path.join(" ")).join(" "));
                    put(with_path.clone(), r.clone());
                    put(format!("{with_path} {noun}"), r);
                    paths.insert((kind, *id), path.clone());
                }
            }
        }
        for t in &v.tags {
            let label = capitalise(t);
            put(t.clone(), Reading { kind: ChipKind::Tag, label, effect: Effect::Tag(t.clone()) });
        }
        let kinds: [(&[&str], &str, Effect); 8] = [
            (&["video", "videos", "movie", "movies"], "Videos", Effect::Videos),
            (&["live", "live photo", "live photos"], "Live Photos", Effect::Kind(AssetKind::Live)),
            (&["raw", "raws"], "RAW", Effect::Kind(AssetKind::Raw)),
            (&["hdr"], "HDR", Effect::Kind(AssetKind::Hdr)),
            (&["screenshot", "screenshots"], "Screenshots", Effect::Kind(AssetKind::Screenshot)),
            (&["panorama", "panoramas", "pano", "panos"], "Panoramas", Effect::Kind(AssetKind::Panorama)),
            (&["selfie", "selfies"], "Selfies", Effect::Kind(AssetKind::Selfie)),
            (&["favorite", "favorites", "favourite", "favourites"], "Favorites", Effect::Favorites),
        ];
        for (phrases, label, effect) in kinds {
            for p in phrases {
                put(p.to_string(), Reading { kind: ChipKind::Kind, label: label.into(), effect: effect.clone() });
            }
            put(fold(label), Reading { kind: ChipKind::Kind, label: label.into(), effect: effect.clone() });
        }
        let cameras = v.cameras.iter().map(|c| (format!(" {} ", name_words(c).join(" ")), c.clone())).collect();
        Index { exact, names, cameras, longest, paths }
    }

    /// The collections an album or collection reading sits in; empty for any other.
    fn path_of(&self, r: &Reading) -> Vec<String> {
        let id = match r.effect {
            Effect::Album(id) | Effect::Collection(id) => id,
            _ => return Vec::new(),
        };
        self.paths.get(&(r.kind, id)).cloned().unwrap_or_default()
    }

    /// Every reading of a whole phrase: exact names, a plural of one, or part of a camera's model.
    fn readings(&self, phrase: &str) -> Vec<Reading> {
        let mut out = self.exact.get(phrase).cloned().unwrap_or_default();
        if out.is_empty() {
            if let Some(single) = phrase.strip_suffix('s').filter(|s| s.len() >= 3) {
                out = self.exact.get(single).cloned().unwrap_or_default();
            }
        }
        if phrase.chars().count() >= 4 && phrase.chars().any(char::is_alphabetic) {
            let needle = format!(" {phrase} ");
            let models: Vec<&str> = self.cameras.iter().filter(|(m, _)| m.contains(&needle)).map(|(_, c)| c.as_str()).collect();
            if !models.is_empty() {
                let label = if models.len() == 1 && name_words(models[0]).join(" ") == phrase { models[0].to_string() } else { phrase.to_string() };
                out.push(Reading { kind: ChipKind::Camera, label, effect: Effect::Camera(phrase.to_string()) });
            }
        }
        out
    }

    /// A person or pet a single word only nearly names: a unique prefix of three letters or
    /// more, or one edit from a name of five letters or more. Never both ways at once.
    fn nearly(&self, word: &str) -> Option<Reading> {
        let n = word.chars().count();
        if n < 3 || FILLER.contains(&word) || word.chars().all(|c| c.is_ascii_digit()) {
            return None;
        }
        let mut found: Vec<&(Effect, String, Vec<String>, String)> = Vec::new();
        for entry in &self.names {
            let (_, _, words, whole) = entry;
            let hit = std::iter::once(whole).chain(words.iter()).any(|t| {
                t.starts_with(word) || (n >= 5 && t.len().abs_diff(word.len()) <= 2 && t.chars().count() >= 5 && within_one_edit(word, t))
            });
            if hit && !found.iter().any(|f| f.0 == entry.0) {
                found.push(entry);
            }
        }
        match found.as_slice() {
            [(effect, name, _, _)] => Some(Reading {
                kind: if matches!(effect, Effect::Person(_)) { ChipKind::Person } else { ChipKind::Pet },
                label: name.to_string(),
                effect: effect.clone(),
            }),
            _ => None,
        }
    }
}

fn capitalise(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().chain(c).collect(),
        None => String::new(),
    }
}

/// Levenshtein distance ≤ 1.
fn within_one_edit(a: &str, b: &str) -> bool {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let (short, long) = if a.len() <= b.len() { (&a, &b) } else { (&b, &a) };
    if long.len() - short.len() > 1 {
        return false;
    }
    let mut i = 0;
    while i < short.len() && short[i] == long[i] {
        i += 1;
    }
    if short.len() == long.len() {
        short[i.min(short.len())..].iter().skip(1).eq(long[i.min(long.len())..].iter().skip(1))
    } else {
        short[i..] == long[i + 1..]
    }
}

/// Dates in local time, as unix seconds.
struct Clock {
    today: NaiveDate,
    offset: i64,
    order: DateOrder,
}

impl Clock {
    fn start(&self, d: NaiveDate) -> i64 {
        d.and_hms_opt(0, 0, 0).map(|t| t.and_utc().timestamp()).unwrap_or(0) - self.offset
    }

    /// From the start of `from` to the second before `until`.
    fn span(&self, from: NaiveDate, until: NaiveDate) -> Effect {
        Effect::Dates(self.start(from), self.start(until) - 1)
    }

    fn ymd(y: i32, m: u32) -> NaiveDate {
        let (y, m) = if m > 12 { (y + 1, m - 12) } else { (y, m) };
        NaiveDate::from_ymd_opt(y, m, 1).unwrap_or_default()
    }

    /// One day, or a month of a year written in numbers (Round 10): "12.8.2023", "2023-08-12",
    /// "12/08/2023" (in the date order, unless that cannot be a day: "8/25/2023" read day first),
    /// "12 August 2023", "Aug 12, 2023", "2023-08", "08/2023". A day needs its year.
    fn day_at(&self, words: &[&Word], text: &str, year: &dyn Fn(Option<&str>) -> Option<i32>, month: &dyn Fn(Option<&str>) -> Option<u32>) -> Option<(usize, Reading)> {
        let w = |i: usize| words.get(i).map(|w| w.norm.as_str());
        // A day or month number of one or two digits; a day may say "12th".
        let small = |s: Option<&str>| -> Option<u32> {
            let s = s?;
            let digits = ["st", "nd", "rd", "th"].iter().find_map(|x| s.strip_suffix(x)).unwrap_or(s);
            (digits.len() <= 2 && !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit())).then(|| digits.parse().ok()).flatten()
        };
        let plain = |s: Option<&str>| s.filter(|s| s.bytes().all(|b| b.is_ascii_digit())).and_then(|s| small(Some(s)));
        // What stands between words `i` and `i + 1`: a date's separator, or nothing but space.
        let sep = |i: usize| -> Option<char> {
            let (a, b) = (words.get(i)?, words.get(i + 1)?);
            let between = text[a.end..b.start].trim();
            match between {
                "" => Some(' '),
                "." | "-" | "/" | "," => between.chars().next(),
                _ => None,
            }
        };
        let numeric = |c: Option<char>| matches!(c, Some('.' | '-' | '/'));
        let day = |y: i32, m: u32, d: u32| -> Option<Reading> {
            let from = NaiveDate::from_ymd_opt(y, m, d)?;
            let label = format!("{d} {} {y}", capitalise(MONTHS[m as usize - 1]));
            Some(Reading { kind: ChipKind::Date, label, effect: self.span(from, from + chrono::Days::new(1)) })
        };
        let month_of = |y: i32, m: u32| -> Option<Reading> {
            (1..=12).contains(&m).then(|| Reading {
                kind: ChipKind::Date,
                label: format!("{} {y}", capitalise(MONTHS[m as usize - 1])),
                effect: self.span(Self::ymd(y, m), Self::ymd(y, m + 1)),
            })
        };
        // 2023-08-12, 2023-08.
        if let (Some(y), Some(m)) = (year(w(0)), plain(w(1))) {
            if numeric(sep(0)) {
                if let (Some(d), true) = (plain(w(2)), numeric(sep(1))) {
                    return day(y, m, d).map(|r| (3, r));
                }
                return month_of(y, m).map(|r| (2, r));
            }
        }
        // 12.8.2023, 8/25/2023, 08/2023.
        if let Some(a) = plain(w(0)) {
            if numeric(sep(0)) {
                if let (Some(b), Some(y), true) = (plain(w(1)), year(w(2)), numeric(sep(1))) {
                    // In the date order, else the other way round when that cannot be a day.
                    let found = match self.order {
                        DateOrder::Mdy => day(y, a, b).or_else(|| day(y, b, a)),
                        DateOrder::Dmy | DateOrder::Ymd => day(y, b, a).or_else(|| day(y, a, b)),
                    };
                    return found.map(|r| (3, r));
                }
                if let Some(y) = year(w(1)) {
                    return month_of(y, a).map(|r| (2, r));
                }
            }
        }
        // 12 August 2023, 12. aug 2023.
        if let (Some(d), Some(m), Some(y)) = (small(w(0)), month(w(1)), year(w(2))) {
            if matches!(sep(0), Some(' ' | '.')) && matches!(sep(1), Some(' ' | '.' | ',')) {
                return day(y, m, d).map(|r| (3, r));
            }
        }
        // August 12 2023, Aug 12, 2023.
        if let (Some(m), Some(d), Some(y)) = (month(w(0)), small(w(1)), year(w(2))) {
            if sep(0) == Some(' ') && matches!(sep(1), Some(' ' | ',')) {
                return day(y, m, d).map(|r| (3, r));
            }
        }
        // 4 June, June 4 (Round 11): the last one up to today.
        let recent = |m: u32, d: u32| -> Option<Reading> {
            use chrono::Datelike;
            let y = self.today.year();
            let this = NaiveDate::from_ymd_opt(y, m, d)?;
            day(if this > self.today { y - 1 } else { y }, m, d)
        };
        if let (Some(d), Some(m)) = (small(w(0)), month(w(1))) {
            if matches!(sep(0), Some(' ' | '.')) {
                return recent(m, d).map(|r| (2, r));
            }
        }
        if let (Some(m), Some(d)) = (month(w(0)), small(w(1))) {
            if sep(0) == Some(' ') {
                return recent(m, d).map(|r| (2, r));
            }
        }
        None
    }

    /// A date at the start of `words`: how many words it took and what it means.
    fn date_at(&self, words: &[&Word], text: &str) -> Option<(usize, Reading)> {
        use chrono::Datelike;
        let w = |i: usize| words.get(i).map(|w| w.norm.as_str());
        let year = |s: Option<&str>| s.filter(|s| s.len() == 4).and_then(|s| s.parse::<i32>().ok()).filter(|y| (1900..=2100).contains(y));
        let month = |s: Option<&str>| {
            let s = s?;
            MONTHS.iter().position(|m| *m == s || (s.len() >= 3 && m.starts_with(s) && (s.len() == 3 || s == "sept"))).map(|i| i as u32 + 1)
        };
        let season = |s: Option<&str>| match s? {
            "spring" => Some(("Spring", 3)),
            "summer" => Some(("Summer", 6)),
            "autumn" | "fall" => Some(("Autumn", 9)),
            "winter" => Some(("Winter", 12)),
            _ => None,
        };
        let date = |label: String, effect: Effect| Reading { kind: ChipKind::Date, label, effect };
        let t = self.today;
        let first_of_month = Self::ymd(t.year(), t.month());
        let monday = t - chrono::Days::new(t.weekday().num_days_from_monday() as u64);

        match w(0)? {
            "today" => return Some((1, date("Today".into(), self.span(t, t + chrono::Days::new(1))))),
            "yesterday" => return Some((1, date("Yesterday".into(), self.span(t - chrono::Days::new(1), t)))),
            which @ ("last" | "this") => {
                let last = which == "last";
                let (label, effect) = match w(1)? {
                    "week" if last => ("Last week", self.span(monday - chrono::Days::new(7), monday)),
                    "week" => ("This week", self.span(monday, monday + chrono::Days::new(7))),
                    "month" if last => {
                        let prev = if t.month() == 1 { Self::ymd(t.year() - 1, 12) } else { Self::ymd(t.year(), t.month() - 1) };
                        ("Last month", self.span(prev, first_of_month))
                    }
                    "month" => ("This month", self.span(first_of_month, Self::ymd(t.year(), t.month() + 1))),
                    "year" if last => ("Last year", self.span(Self::ymd(t.year() - 1, 1), Self::ymd(t.year(), 1))),
                    "year" => ("This year", self.span(Self::ymd(t.year(), 1), Self::ymd(t.year() + 1, 1))),
                    _ => return None,
                };
                return Some((2, date(label.into(), effect)));
            }
            _ => {}
        }
        if let Some(found) = self.day_at(words, text, &year, &month) {
            return Some(found);
        }
        if let Some(y1) = year(w(0)) {
            // "2019-2021", "2019–2021", "2019 to 2021".
            let dashed = words.get(1).is_some_and(|b| text[words[0].end..b.start].contains(['-', '–', '—']));
            let (n, y2) = if dashed { (2, year(w(1))) } else if w(1) == Some("to") { (3, year(w(2))) } else { (1, None) };
            if let Some(y2) = y2.filter(|y2| *y2 >= y1) {
                return Some((n, date(format!("{y1}–{y2}"), self.span(Self::ymd(y1, 1), Self::ymd(y2 + 1, 1)))));
            }
            return Some((1, date(y1.to_string(), self.span(Self::ymd(y1, 1), Self::ymd(y1 + 1, 1)))));
        }
        if let Some(m) = month(w(0)) {
            let name = capitalise(MONTHS[m as usize - 1]);
            return Some(match year(w(1)) {
                Some(y) => (2, date(format!("{name} {y}"), self.span(Self::ymd(y, m), Self::ymd(y, m + 1)))),
                None => (1, date(name, Effect::Months(vec![m as u8]))),
            });
        }
        if let Some((name, m)) = season(w(0)) {
            // Northern-hemisphere seasons, whole months; winter 2023 runs into 2024.
            return Some(match year(w(1)) {
                Some(y) => (2, date(format!("{name} {y}"), self.span(Self::ymd(y, m), Self::ymd(y, m + 3)))),
                None => (1, date(name.into(), Effect::Months((0..3).map(|i| ((m - 1 + i) % 12 + 1) as u8).collect()))),
            });
        }
        None
    }
}

/// Reads `text` against `vocab`. `today` and `offset_secs` turn "last week" and "2023" into
/// timestamps: `offset_secs` is how far the stored clock is ahead of the one the dates are read
/// in, so 0 for `captured_at`, which is wall-clock time; `count` says how many photos a reading matches, to choose between
/// readings of the same words.
pub fn parse(text: &str, vocab: &Vocabulary, today: NaiveDate, offset_secs: i32, order: DateOrder, count: impl Fn(&AssetQuery) -> u64) -> Parsed {
    let index = vocab.index();
    let clock = Clock { today, offset: offset_secs as i64, order };
    let items = tokenize(text);
    let mut query = AssetQuery::default();
    let mut chips: Vec<Chip> = Vec::new();
    let mut rest: Vec<(&str, bool)> = Vec::new(); // (text, is filler)

    let mut i = 0;
    while i < items.len() {
        let words: Vec<&Word> = items[i..].iter().map_while(|it| if let Item::Word(w) = it { Some(w) } else { None }).collect();
        if let Item::Forced { kind, value, start, end } = &items[i] {
            match forced(index, &clock, *kind, value) {
                Some(r) => {
                    r.effect.apply(&mut query);
                    chips.push(Chip { kind: r.kind, label: r.label.clone(), span: (*start, *end), token: text[*start..*end].to_string(), path: index.path_of(&r), alternatives: Vec::new() });
                }
                None => rest.push((value.as_str(), false)),
            }
            i += 1;
            continue;
        }

        // The longest phrase the vocabulary knows, and a date, both from here.
        let mut best: (usize, Vec<Reading>) = (0, Vec::new());
        for n in (1..=index.longest.min(words.len())).rev() {
            let phrase = words[..n].iter().map(|w| w.norm.as_str()).collect::<Vec<_>>().join(" ");
            let found = index.readings(&phrase);
            if !found.is_empty() {
                best = (n, found);
                break;
            }
        }
        if let Some((n, r)) = clock.date_at(&words, text) {
            if n > best.0 {
                best = (n, vec![r]);
            } else if n == best.0 {
                best.1.push(r);
            }
        }
        if best.1.is_empty() {
            if let Some(r) = index.nearly(&words[0].norm) {
                best = (1, vec![r]);
            }
        }
        let (n, mut readings) = best;
        if readings.is_empty() {
            let w = words[0];
            rest.push((&text[w.start..w.end], FILLER.contains(&w.norm.as_str())));
            i += 1;
            continue;
        }

        if readings.len() > 1 {
            let counts: Vec<u64> = readings
                .iter()
                .map(|r| {
                    let mut q = AssetQuery::default();
                    r.effect.apply(&mut q);
                    count(&q)
                })
                .collect();
            let mut order: Vec<usize> = (0..readings.len()).collect();
            order.sort_by_key(|&k| (std::cmp::Reverse(counts[k]), priority(readings[k].kind)));
            readings = order.into_iter().map(|k| readings[k].clone()).collect();
        }
        let span = (words[0].start, words[n - 1].end);
        let raw = &text[span.0..span.1];
        let chip = |r: &Reading, alternatives: Vec<Chip>| {
            let path = index.path_of(r);
            Chip { kind: r.kind, label: r.label.clone(), span, token: r.token(raw, &path), path, alternatives }
        };
        let alternatives = readings[1..].iter().map(|r| chip(r, Vec::new())).collect();
        readings[0].effect.apply(&mut query);
        chips.push(chip(&readings[0], alternatives));
        i += n;
    }

    let rest: Vec<&str> = rest.into_iter().filter(|(_, filler)| chips.is_empty() || !filler).map(|(t, _)| t).collect();
    let rest = rest.join(" ");
    query.text = Some(rest.clone()).filter(|r| !r.is_empty());
    Parsed { query, chips, rest }
}

/// `filters` (the chips and toolbar the user set) narrowed by what the text was read as; the
/// text becomes what was left of it. A filter the user set wins over one read from the text.
pub fn merge(filters: &AssetQuery, parsed: &AssetQuery) -> AssetQuery {
    fn union<T: PartialEq + Clone>(a: &[T], b: &[T]) -> Vec<T> {
        let mut out = a.to_vec();
        out.extend(b.iter().filter(|x| !a.contains(x)).cloned());
        out
    }
    let mut q = filters.clone();
    q.text = parsed.text.clone();
    q.person_ids = union(&filters.person_ids, &parsed.person_ids);
    q.pet_ids = union(&filters.pet_ids, &parsed.pet_ids);
    q.places = union(&filters.places, &parsed.places);
    q.tags = union(&filters.tags, &parsed.tags);
    q.album_ids = union(&filters.album_ids, &parsed.album_ids);
    let dated = filters.date_from.is_some() || filters.date_to.is_some() || !filters.months.is_empty();
    if !dated {
        q.date_from = parsed.date_from;
        q.date_to = parsed.date_to;
        q.months = parsed.months.clone();
    }
    q.media_type = filters.media_type.or(parsed.media_type);
    q.kind = filters.kind.or(parsed.kind);
    q.camera = filters.camera.clone().or_else(|| parsed.camera.clone());
    q.favorites = filters.favorites || parsed.favorites;
    q
}

/// What a date tag narrows to (Round 7): a span of time, or months of any year.
#[derive(Debug, Clone, PartialEq)]
pub enum DateSpan {
    /// Inclusive, in the stored clock's seconds.
    Range(i64, i64),
    Months(Vec<u8>),
}

/// `text` read as one date, every word of it ("2023", "march 2024", "last week"), with its
/// label; `None` when any word is something else.
pub fn read_date(text: &str, today: NaiveDate, offset_secs: i32, order: DateOrder) -> Option<(String, DateSpan)> {
    let clock = Clock { today, offset: offset_secs as i64, order };
    match forced(&Index::default(), &clock, ChipKind::Date, text)? {
        Reading { label, effect: Effect::Dates(from, to), .. } => Some((label, DateSpan::Range(from, to))),
        Reading { label, effect: Effect::Months(m), .. } => Some((label, DateSpan::Months(m))),
        _ => None,
    }
}

/// A date or a span of dates (Round 11): anything [`read_date`] reads, or `before X`,
/// `after X`, `since X`, `until X`, `between X and Y`, `X – Y`, `X - Y`, `X to Y`. Returns its
/// label and the inclusive span, `None` at an open end. A month of every year is no span.
pub fn read_date_expr(text: &str, today: NaiveDate, offset_secs: i32, order: DateOrder) -> Option<(String, Option<i64>, Option<i64>)> {
    let span = |t: &str| match read_date(t.trim(), today, offset_secs, order)? {
        (label, DateSpan::Range(from, to)) => Some((label, from, to)),
        _ => None,
    };
    let text = text.trim();
    if let Some((label, from, to)) = span(text) {
        return Some((label, Some(from), Some(to)));
    }
    let lower = text.to_lowercase();
    // Lowercasing keeps every byte offset of these ASCII words.
    let after_word = |word: &str| lower.strip_prefix(word).filter(|r| r.starts_with(char::is_whitespace)).map(|r| &text[text.len() - r.len()..]);
    for (word, open_start) in [("before", true), ("until", true), ("till", true), ("after", false), ("since", false), ("from", false)] {
        if let Some((label, from, to)) = after_word(word).and_then(span) {
            let label = format!("{word} {label}");
            return Some(match (word, open_start) {
                ("before", _) => (label, None, Some(from - 1)),
                (_, true) => (label, None, Some(to)),
                ("after", _) => (label, Some(to + 1), None),
                _ => (label, Some(from), None),
            });
        }
    }
    let pair = |a: &str, b: &str| -> Option<(String, Option<i64>, Option<i64>)> {
        let (la, from, _) = span(a)?;
        let (lb, _, to) = span(b)?;
        (from <= to).then(|| (span_label(&la, &lb), Some(from), Some(to)))
    };
    if let Some(rest) = after_word("between") {
        let lower_rest = rest.to_lowercase();
        if let Some(i) = lower_rest.find(" and ") {
            return pair(&rest[..i], &rest[i + 5..]);
        }
    }
    for sep in [" – ", "–", " — ", " - ", " to "] {
        if let Some(i) = lower.find(sep) {
            if let Some(found) = pair(&text[..i], &text[i + sep.len()..]) {
                return Some(found);
            }
        }
    }
    None
}

/// "1 June 2026" and "4 June 2026" → "1 – 4 June 2026": the words both end with, said once.
fn span_label(a: &str, b: &str) -> String {
    let (wa, wb): (Vec<&str>, Vec<&str>) = (a.split(' ').collect(), b.split(' ').collect());
    let mut same = 0;
    while same < wa.len() - 1 && same < wb.len() - 1 && wa[wa.len() - 1 - same] == wb[wb.len() - 1 - same] {
        same += 1;
    }
    if same == 0 {
        return format!("{a} – {b}");
    }
    let tail = wa[wa.len() - same..].join(" ");
    format!("{} – {} {tail}", wa[..wa.len() - same].join(" "), wb[..wb.len() - same].join(" "))
}

/// The reading a `kind:"value"` token asks for, if the vocabulary has it.
fn forced(index: &Index, clock: &Clock, kind: ChipKind, value: &str) -> Option<Reading> {
    let phrase = name_words(value).join(" ");
    if kind == ChipKind::Date {
        let items = tokenize(value);
        let words: Vec<&Word> = items.iter().filter_map(|it| if let Item::Word(w) = it { Some(w) } else { None }).collect();
        return clock.date_at(&words, value).filter(|(n, _)| *n == words.len()).map(|(_, r)| r);
    }
    let mut found = index.readings(&phrase);
    if kind == ChipKind::Person || kind == ChipKind::Pet {
        found.extend(index.nearly(&phrase));
    }
    found.into_iter().find(|r| r.kind == kind)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vocab() -> Vocabulary {
        Vocabulary {
            people: vec![(1, "Anna Berg".into()), (2, "Ben".into()), (3, "Florence".into()), (4, "Marta".into())],
            pets: vec![],
            places: vec![
                PlaceRef::City { name: "Florence".into(), country: Some("Italy".into()) },
                PlaceRef::City { name: "New York".into(), country: Some("United States".into()) },
                PlaceRef::City { name: "York".into(), country: Some("United Kingdom".into()) },
                PlaceRef::City { name: "Lisbon".into(), country: Some("Portugal".into()) },
                PlaceRef::City { name: "Zürich".into(), country: Some("Switzerland".into()) },
                PlaceRef::Country { name: "Portugal".into() },
            ],
            albums: vec![(10, "Summer in Lisbon".into())],
            cameras: vec!["iPhone 15 Pro".into(), "ILCE-7M4".into()],
            tags: crate::embedding::SCENE_TAGS.iter().map(|t| t.to_string()).collect(),
            ..Default::default()
        }
    }

    fn today() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 10, 3).unwrap() // a Saturday
    }

    fn ts(y: i32, m: u32, d: u32) -> i64 {
        NaiveDate::from_ymd_opt(y, m, d).unwrap().and_hms_opt(0, 0, 0).unwrap().and_utc().timestamp()
    }

    fn run(text: &str) -> Parsed {
        parse(text, &vocab(), today(), 0, DateOrder::Dmy, |_| 1)
    }

    /// Two albums called Italy, one in *Trips › Europe* and one in *Family*, and the collections.
    fn collections_vocab() -> Vocabulary {
        let paths = |l: &[(i64, &[&str])]| l.iter().map(|(id, p)| (*id, p.iter().map(|s| s.to_string()).collect())).collect();
        Vocabulary {
            albums: vec![(11, "Italy".into()), (12, "Italy".into()), (13, "Loose".into())],
            collections: vec![(1, "Trips".into()), (2, "Europe".into()), (3, "Family".into())],
            album_paths: paths(&[(11, &["Trips", "Europe"]), (12, &["Family"])]),
            collection_paths: paths(&[(2, &["Trips"])]),
            ..Default::default()
        }
    }

    fn run_collections(text: &str) -> Parsed {
        parse(text, &collections_vocab(), today(), 0, DateOrder::Dmy, |_| 1)
    }

    #[test]
    fn collections_and_albums_are_read_by_name_path_and_sentence() {
        let p = run_collections("in the trips collection");
        assert_eq!(p.query.collection_ids, vec![1]);
        assert_eq!(p.rest, "", "the kind word belongs to the chip");
        assert_eq!(p.chips.len(), 1);
        assert_eq!((p.chips[0].kind, p.chips[0].label.as_str()), (ChipKind::Collection, "Trips"));
        let p = run_collections("collection:europe");
        assert_eq!(p.query.collection_ids, vec![2]);
        assert_eq!(p.chips[0].path, vec!["Trips".to_string()], "a nested collection says where it is");
        // A bare name read both ways: the album Loose has no collection, so no path.
        let p = run_collections("album:loose");
        assert_eq!((p.query.album_ids.clone(), p.chips[0].path.clone()), (vec![13], Vec::<String>::new()));
        assert_eq!(run_collections("loose album").query.album_ids, vec![13]);
        assert_eq!(run_collections("in the loose album").rest, "");
    }

    #[test]
    fn two_albums_of_one_name_are_told_apart_by_their_collections() {
        let p = run_collections("italy album");
        assert_eq!(p.chips.len(), 1);
        let first = &p.chips[0];
        let all: Vec<&Chip> = std::iter::once(first).chain(first.alternatives.iter()).collect();
        assert_eq!(all.len(), 2, "both Italy albums are offered");
        let mut paths: Vec<Vec<String>> = all.iter().map(|c| c.path.clone()).collect();
        paths.sort();
        assert_eq!(paths, vec![vec!["Family".to_string()], vec!["Trips".to_string(), "Europe".to_string()]]);
        let europe = all.iter().find(|c| c.path.len() == 2).unwrap();
        assert_eq!(europe.token, "album:\"Trips › Europe › Italy\"");
        // The path reads the same typed or written by a chip, with or without the kind word.
        assert_eq!(run_collections(&europe.token).query.album_ids, vec![11]);
        assert_eq!(run_collections("album:\"Family › Italy\"").query.album_ids, vec![12]);
        assert_eq!(run_collections("trips europe italy").query.album_ids, vec![11]);
    }

    fn kinds(p: &Parsed) -> Vec<(ChipKind, String)> {
        p.chips.iter().map(|c| (c.kind, c.label.clone())).collect()
    }

    #[test]
    fn anna_beach_2023() {
        let p = run("Anna beach 2023");
        assert_eq!(
            kinds(&p),
            vec![(ChipKind::Person, "Anna Berg".into()), (ChipKind::Tag, "Beach".into()), (ChipKind::Date, "2023".into())]
        );
        assert_eq!(p.query.person_ids, vec![1]);
        assert_eq!(p.query.tags, vec!["beach".to_string()]);
        assert_eq!(p.query.date_from, Some(ts(2023, 1, 1)));
        assert_eq!(p.query.date_to, Some(ts(2024, 1, 1) - 1));
        assert_eq!(p.rest, "");
        assert_eq!(p.query.text, None);
        assert_eq!(p.chips[1].span, (5, 10));
    }

    #[test]
    fn red_umbrella() {
        let p = run("red umbrella");
        assert!(p.chips.is_empty());
        assert_eq!(p.rest, "red umbrella");
        assert_eq!(p.query, AssetQuery { text: Some("red umbrella".into()), ..Default::default() });
    }

    #[test]
    fn new_york_before_york() {
        let p = run("new york");
        assert_eq!(kinds(&p), vec![(ChipKind::Place, "New York, United States".into())]);
        assert_eq!(p.query.places, vec![PlaceRef::City { name: "New York".into(), country: Some("United States".into()) }]);
        let p = run("york");
        assert_eq!(p.query.places[0].label(), "York, United Kingdom");
    }

    #[test]
    fn an_ambiguous_word_prefers_the_reading_with_more_photos() {
        let city = |q: &AssetQuery| if q.places.is_empty() { 3 } else { 40 };
        let p = parse("florence", &vocab(), today(), 0, DateOrder::Dmy, city);
        assert_eq!(kinds(&p), vec![(ChipKind::Place, "Florence, Italy".into())]);
        assert_eq!(p.chips[0].alternatives.len(), 1);
        assert_eq!(p.chips[0].alternatives[0].kind, ChipKind::Person);
        assert_eq!(p.chips[0].alternatives[0].token, "person:\"Florence\"");
        let person = |q: &AssetQuery| if q.person_ids.is_empty() { 3 } else { 40 };
        let p = parse("florence", &vocab(), today(), 0, DateOrder::Dmy, person);
        assert_eq!(p.query.person_ids, vec![3]);
        assert!(p.query.places.is_empty());
        // The token the chip offers forces that reading, whatever the counts say.
        let forced = parse("person:\"Florence\" 2024", &vocab(), today(), 0, DateOrder::Dmy, city);
        assert_eq!(forced.query.person_ids, vec![3]);
        assert!(forced.chips[0].alternatives.is_empty());
        assert_eq!(forced.chips[0].span, (0, 17));
        assert_eq!(forced.chips[1].kind, ChipKind::Date);
    }

    #[test]
    fn possessive_and_prefix() {
        assert_eq!(run("anna's birthday").query.person_ids, vec![1]);
        assert_eq!(run("anna’s").query.person_ids, vec![1]);
        assert_eq!(run("ann").query.person_ids, vec![1], "a unique prefix of three letters");
        assert_eq!(run("Anna Berg").chips.len(), 1, "the full name is one chip");
        assert_eq!(run("mart").query.person_ids, vec![4]);
        assert_eq!(run("marte").query.person_ids, vec![4], "one edit away from a five-letter name");
        assert_eq!(run("zurich").query.places[0].label(), "Zürich, Switzerland", "accents do not matter");
        assert_eq!(run("dogs").query.tags, vec!["dog".to_string()], "plural");
    }

    #[test]
    fn a_short_name_never_fuzzy_matches() {
        let p = run("ken");
        assert!(p.query.person_ids.is_empty(), "Ken is not Ben");
        assert_eq!(p.rest, "ken");
        assert!(run("an").query.person_ids.is_empty(), "two letters are not a prefix");
    }

    #[test]
    fn a_removed_person_is_not_a_person() {
        let db = crate::Db::open_in_memory().unwrap();
        db.with_conn(|c| {
            c.execute_batch(
                "INSERT INTO people (id, name, created_at, status) VALUES (1, 'Anna', 0, 'listed'), (2, 'Ola', 0, 'removed'), (3, NULL, 0, 'listed');
                 INSERT INTO albums (id, name, kind, created_at, updated_at) VALUES (5, 'Lisbon trip', 0, 0, 0), (6, 'Smart', 1, 0, 0);
                 INSERT INTO assets (id, content_hash, relative_path, filename, extension, media_type, byte_size, orientation, imported_at, place_name, metadata_json)
                   VALUES (1, 'h1', 'p', 'a.jpg', 'jpg', 0, 1, 1, 10, 'Lisbon, Portugal', '{\"model\":\"iPhone 15 Pro\"}'),
                          (2, 'h2', 'p', 'b.jpg', 'jpg', 0, 1, 1, 10, 'Portugal', NULL);",
            )?;
            let v = Vocabulary::load(c)?;
            assert_eq!(v.people, vec![(1, "Anna".to_string())]);
            assert_eq!(v.albums, vec![(5, "Lisbon trip".to_string())]);
            assert_eq!(v.cameras, vec!["iPhone 15 Pro".to_string()]);
            assert!(v.places.contains(&PlaceRef::City { name: "Lisbon".into(), country: Some("Portugal".into()) }));
            assert!(v.places.contains(&PlaceRef::Country { name: "Portugal".into() }));
            assert_eq!(v.tags.len(), crate::embedding::SCENE_TAGS.len());
            let p = parse("ola", &v, today(), 0, DateOrder::Dmy, |_| 1);
            assert!(p.query.person_ids.is_empty());
            assert_eq!(p.rest, "ola", "still plain text, for a caption or keyword");
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn every_village_in_the_library_is_a_place() {
        let db = crate::Db::open_in_memory().unwrap();
        db.with_conn(|c| {
            c.execute_batch(
                "INSERT INTO assets (id, content_hash, relative_path, filename, extension, media_type, byte_size, orientation, imported_at, place_name, locality)
                   VALUES (1, 'h1', 'p', 'a.jpg', 'jpg', 0, 1, 1, 10, 'Kranj, Slovenia', 'Kropa, Slovenia'),
                          (2, 'h2', 'p', 'b.jpg', 'jpg', 0, 1, 1, 10, 'Kranj, Slovenia', 'Kranj, Slovenia'),
                          (3, 'h3', 'p', 'c.jpg', 'jpg', 0, 1, 1, 10, 'Kranj, Slovenia', '');",
            )?;
            let v = Vocabulary::load(c)?;
            let kropa = PlaceRef::Locality { name: "Kropa".into(), country: Some("Slovenia".into()) };
            assert!(v.places.contains(&kropa));
            assert!(
                !v.places.contains(&PlaceRef::Locality { name: "Kranj".into(), country: Some("Slovenia".into()) }),
                "a town already named by place_name is its city reading, not a second one"
            );
            let p = parse("kropa", &v, today(), 0, DateOrder::Dmy, |_| 1);
            assert_eq!(kinds(&p), vec![(ChipKind::Place, "Kropa, Slovenia".to_string())]);
            assert_eq!(p.query.places, vec![kropa.clone()]);
            let scope = crate::scope::LayoutScope { query: Some(p.query), ..Default::default() };
            assert_eq!(crate::layout::ids_scoped(c, &scope, 10)?, vec![1]);
            assert_eq!(kropa.label(), "Kropa, Slovenia");
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn seasons_and_relative_dates() {
        let p = run("march 2024");
        assert_eq!((p.query.date_from, p.query.date_to), (Some(ts(2024, 3, 1)), Some(ts(2024, 4, 1) - 1)));
        assert_eq!(p.chips[0].label, "March 2024");
        assert_eq!(run("march").query.months, vec![3]);
        assert_eq!(run("summer").query.months, vec![6, 7, 8]);
        let w = run("winter 2023").query;
        assert_eq!((w.date_from, w.date_to), (Some(ts(2023, 12, 1)), Some(ts(2024, 3, 1) - 1)));
        let t = run("today").query;
        assert_eq!((t.date_from, t.date_to), (Some(ts(2026, 10, 3)), Some(ts(2026, 10, 4) - 1)));
        let y = run("yesterday").query;
        assert_eq!(y.date_from, Some(ts(2026, 10, 2)));
        let lw = run("last week").query;
        assert_eq!((lw.date_from, lw.date_to), (Some(ts(2026, 9, 21)), Some(ts(2026, 9, 28) - 1)), "Monday to Sunday before this one");
        let lm = run("last month").query;
        assert_eq!((lm.date_from, lm.date_to), (Some(ts(2026, 9, 1)), Some(ts(2026, 10, 1) - 1)));
        let ly = run("last year").query;
        assert_eq!(ly.date_from, Some(ts(2025, 1, 1)));
        for range in ["2019-2021", "2019–2021", "2019 to 2021"] {
            let r = run(range);
            assert_eq!((r.query.date_from, r.query.date_to), (Some(ts(2019, 1, 1)), Some(ts(2022, 1, 1) - 1)), "{range}");
            assert_eq!(r.chips.len(), 1);
        }
        // The local offset moves midnight: UTC+2 starts the year two hours earlier in UTC.
        let east = parse("2023", &vocab(), today(), 7200, DateOrder::Dmy, |_| 1).query;
        assert_eq!(east.date_from, Some(ts(2023, 1, 1) - 7200));
    }

    #[test]
    fn a_single_day_in_every_common_form() {
        let day = (Some(ts(2023, 8, 12)), Some(ts(2023, 8, 13) - 1));
        for text in ["12.8.2023", "12. 8. 2023", "12/08/2023", "12-8-2023", "2023-08-12", "2023/8/12", "12 august 2023", "12. aug 2023", "12th aug 2023", "august 12 2023", "Aug 12, 2023"] {
            let p = run(text);
            assert_eq!((p.query.date_from, p.query.date_to), day, "{text}");
            assert_eq!(p.chips.len(), 1, "{text}");
            assert_eq!(p.chips[0].label, "12 August 2023", "{text}");
            assert_eq!(p.rest, "", "{text}");
        }
        // Month first when the day cannot be a month.
        assert_eq!(run("8/25/2023").query.date_from, Some(ts(2023, 8, 25)));
        // A month of a year, by number.
        for text in ["2023-08", "08/2023", "8.2023"] {
            let p = run(text);
            assert_eq!((p.query.date_from, p.query.date_to), (Some(ts(2023, 8, 1)), Some(ts(2023, 9, 1) - 1)), "{text}");
            assert_eq!(p.chips[0].label, "August 2023", "{text}");
        }
        // Not a date: no such day, and a range of years stays one.
        assert_eq!(run("31.2.2023").query.date_from, Some(ts(2023, 2, 1)), "31 February is no day: \"2.2023\" is still read as the month");
        assert_eq!(run("2019-2021").query.date_to, Some(ts(2022, 1, 1) - 1));
        assert_eq!(read_date("12.8.2023", today(), 0, DateOrder::Dmy).map(|d| d.0), Some("12 August 2023".into()));
    }

    #[test]
    fn a_day_in_numbers_reads_in_the_date_order() {
        use crate::settings::DateOrder::{Dmy, Mdy, Ymd};
        let day = |y, m, d| Some((ts(y, m, d), ts(y, m, d) + 86_399));
        let read = |text: &str, order| match read_date(text, today(), 0, order) {
            Some((_, DateSpan::Range(a, b))) => Some((a, b)),
            _ => None,
        };
        let label = |text: &str, order| read_date(text, today(), 0, order).map(|d| d.0);
        // A year last cannot be year-month-day, so YMD reads it day first.
        for (order, want) in [(Dmy, day(2026, 6, 4)), (Mdy, day(2026, 4, 6)), (Ymd, day(2026, 6, 4))] {
            assert_eq!(read("4.6.2026", order), want, "{order:?}");
            assert_eq!(read("4/6/2026", order), want, "{order:?}");
            // ISO is ISO whatever the order.
            assert_eq!(read("2026-06-04", order), day(2026, 6, 4), "{order:?}");
            // A month of a year is one in every order.
            assert_eq!(read("08/2023", order), Some((ts(2023, 8, 1), ts(2023, 9, 1) - 1)), "{order:?}");
        }
        assert_eq!(label("4.6.2026", Mdy).as_deref(), Some("6 April 2026"));
        // A reading the order cannot have falls back to the other one.
        assert_eq!(read("25.8.2023", Mdy), day(2023, 8, 25));
        assert_eq!(read("8/25/2023", Dmy), day(2023, 8, 25));
        // The whole parser carries it too.
        let p = parse("4.6.2026", &vocab(), today(), 0, Mdy, |_| 1);
        assert_eq!((p.query.date_from, p.query.date_to), day(2026, 4, 6).map(|(a, b)| (Some(a), Some(b))).unwrap());
    }

    #[test]
    fn dates_before_after_and_between() {
        let expr = |t: &str| read_date_expr(t, today(), 0, DateOrder::Dmy);
        assert_eq!(expr("before March 2024"), Some(("before March 2024".into(), None, Some(ts(2024, 3, 1) - 1))));
        assert_eq!(expr("after 4.6.2026"), Some(("after 4 June 2026".into(), Some(ts(2026, 6, 5)), None)));
        assert_eq!(expr("since 2024"), Some(("since 2024".into(), Some(ts(2024, 1, 1)), None)));
        assert_eq!(expr("until 2023"), Some(("until 2023".into(), None, Some(ts(2024, 1, 1) - 1))));
        let first_to_fourth = Some(("1 – 4 June 2026".to_string(), Some(ts(2026, 6, 1)), Some(ts(2026, 6, 5) - 1)));
        for t in ["between 1.6.2026 and 4.6.2026", "1.6.2026 – 4.6.2026", "1.6.2026 - 4.6.2026", "1.6.2026 to 4.6.2026", "Between 1.6.2026 and 4.6.2026"] {
            assert_eq!(expr(t), first_to_fourth, "{t}");
        }
        assert_eq!(expr("June 2026 to August 2026"), Some(("June – August 2026".into(), Some(ts(2026, 6, 1)), Some(ts(2026, 9, 1) - 1))));
        assert_eq!(expr("30.12.2025 – 2.1.2026").map(|e| e.0).as_deref(), Some("30 December 2025 – 2 January 2026"));
        // A plain date is one too; what is no date is none.
        assert_eq!(expr("4.6.2026"), Some(("4 June 2026".into(), Some(ts(2026, 6, 4)), Some(ts(2026, 6, 5) - 1))));
        assert_eq!(expr("before lunch"), None);
        assert_eq!(expr("4.6.2026 – 1.6.2026"), None, "a range that ends before it starts");
        assert_eq!(expr("june"), None, "a month of every year is no span");
        // A day without its year is its last one up to today (2026-10-05 in these tests).
        assert_eq!(expr("4 june").map(|e| e.0).as_deref(), Some("4 June 2026"));
        assert_eq!(expr("december 24").map(|e| e.0).as_deref(), Some("24 December 2025"));
    }

    #[test]
    fn kinds_cameras_albums_and_filler_words() {
        assert_eq!(run("videos").query.media_type, Some(1));
        assert_eq!(run("raw").query.kind, Some(AssetKind::Raw));
        assert!(run("favourites").query.favorites);
        assert_eq!(run("iphone").query.camera.as_deref(), Some("iphone"));
        assert_eq!(run("summer in lisbon").query.album_ids, vec![10], "an album's whole name beats its words");
        let p = run("photos of Anna and Ben at the beach");
        assert_eq!(p.query.person_ids, vec![1, 2]);
        assert_eq!(p.query.tags, vec!["beach".to_string()]);
        assert_eq!(p.rest, "", "filler words go once something was recognised");
        let p = run("Anna in a red dress");
        assert_eq!(p.rest, "red dress");
    }

    #[test]
    fn the_users_filters_win_and_the_text_narrows_them() {
        let filters = AssetQuery { text: Some("anna 2023 red".into()), date_from: Some(5), favorites: true, person_ids: vec![2], ..Default::default() };
        let parsed = run("anna 2023 red").query;
        let q = merge(&filters, &parsed);
        assert_eq!(q.text.as_deref(), Some("red"));
        assert_eq!(q.person_ids, vec![2, 1]);
        assert_eq!((q.date_from, q.date_to), (Some(5), None), "the toolbar's dates stand");
        assert!(q.favorites);
    }

    #[test]
    #[ignore]
    fn parsing_takes_under_2ms() {
        let mut v = vocab();
        v.people = (0..500).map(|i| (i, format!("Person{i} Surname{i}"))).collect();
        v.places = (0..2000).map(|i| PlaceRef::City { name: format!("Town{i}"), country: Some("Land".into()) }).collect();
        let built = std::time::Instant::now();
        v.index();
        println!("index: {:?}", built.elapsed());
        let start = std::time::Instant::now();
        for _ in 0..100 {
            let p = parse("Person42 at the beach in Town1999 summer 2023 red umbrella", &v, today(), 0, DateOrder::Dmy, |_| 1);
            assert_eq!(p.query.person_ids, vec![42]);
        }
        let each = start.elapsed() / 100;
        println!("parse: {each:?}");
        assert!(each < std::time::Duration::from_millis(2), "{each:?}");
    }
}
