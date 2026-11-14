//! What a search's text means (Round 4K) and what it shows (Phase 7b).
//!
//! The text is first read against the library's vocabulary (`photos_core::query_parse`):
//! people, places, dates, albums, cameras and scene tags become structured filters, and only
//! what is left is free text. That rest is then embedded by CLIP's text tower and compared with
//! every photo's embedding; the matches ride inside `AssetQuery.semantic`, and a *Best match*
//! scope gets its order in `AssetQuery.rank`. Everything stays in the one `AssetQuery`, so the
//! grid, the count and everything built on a scope see the same results. With smart features
//! off or the models missing, the rest is searched by words alone.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use photos_core::embedding::{semantic_hits, semantic_scan};
use photos_core::layout::count_scoped;
use photos_core::query::AssetQuery;
use photos_core::query_expr::{self, ParsedExpr, QueryExpr};
use photos_core::query_parse::{self, ChipKind, Parsed, Vocabulary};
use photos_core::scope::LayoutScope;
use photos_core::search_overview::SearchOverview;
use photos_core::tag_expr;
use photos_core::tag_search::{self, Facets, TagSuggestion, TagValue};
use photos_core::tag_sentence::{self, SentenceReading};
use tauri::{AppHandle, Manager};

use crate::error::AppResult;
use crate::state::AppState;

/// How many nearest photos a scan keeps before the band is applied.
const SCAN_LIMIT: usize = 2000;
/// Searches remembered, newest last.
const REMEMBERED: usize = 16;

type Scored = Arc<Vec<(i64, f32)>>;
/// A text read as a tree, or `None` when it has no operator.
type TreeRead = Option<Arc<ParsedExpr>>;

/// The vocabulary of the open library, and the texts read and scanned with it, each for one
/// library version.
#[derive(Default)]
pub struct SearchCache {
    vocabulary: Mutex<Option<(u64, Arc<Vocabulary>)>>,
    parsed: Mutex<Vec<(String, u64, Arc<Parsed>)>>,
    /// Texts read as `or` / `and` / `not` (Round 19 §6), `None` for a text without operators.
    expressions: Mutex<Vec<(String, u64, TreeRead)>>,
    /// Meaning scans: (lowercased text, library version, nearest photos best first).
    scored: Mutex<Vec<(String, u64, Scored)>>,
    /// What the tag search can suggest (Round 7): the library's folder, the version it was
    /// read at, the facets.
    facets: Mutex<Option<(std::path::PathBuf, u64, Arc<Facets>)>>,
    /// A newer read of the facets is under way.
    reading_facets: AtomicBool,
    /// Held by the one cold read of the facets (Round 15).
    loading: Mutex<()>,
}

/// Keeps the newest `REMEMBERED` entries, `key` replaced.
fn remember<T>(cache: &Mutex<Vec<(String, u64, T)>>, key: String, version: u64, value: T) {
    if let Ok(mut cache) = cache.lock() {
        cache.retain(|(k, _, _)| *k != key);
        cache.push((key, version, value));
        if cache.len() > REMEMBERED {
            cache.remove(0);
        }
    }
}

fn recall<T: Clone>(cache: &Mutex<Vec<(String, u64, T)>>, key: &str, version: u64) -> Option<T> {
    let cache = cache.lock().ok()?;
    cache.iter().find(|(k, v, _)| k == key && *v == version).map(|(_, _, t)| t.clone())
}

/// `scope` with its query's text read into filters and the rest searched by meaning. A
/// *Best match* scope also gets its ranking.
pub fn understood(state: &AppState, scope: &LayoutScope) -> LayoutScope {
    // A tag sequence that is not a flat list (Round 20 §1) becomes a tree here, its text tags
    // read as sentences; `with_expression` then searches every term by meaning.
    if let Some(query) = scope.query.as_ref().and_then(|q| with_tag_tree(q, &mut |text| text_term(state, text))) {
        return understood(state, &LayoutScope { query: Some(query), ..scope.clone() });
    }
    // Search tags (Round 7) first, so a text tag is read and searched by meaning like typed text.
    // A plain tag sequence joins them in `expanded()`.
    if scope.query.as_ref().is_some_and(|q| !q.search_tags.is_empty() || q.tag_expr.is_some()) {
        let expanded = LayoutScope { query: scope.query.as_ref().map(AssetQuery::expanded), ..scope.clone() };
        return understood(state, &expanded);
    }
    // `or` / `and` / `not` (Round 19 §6); a text without them goes on exactly as before.
    if let Some(out) = with_expression(state, scope) {
        return out;
    }
    let Some(text) = scope.query.as_ref().and_then(|q| q.text.as_deref()).map(str::trim).filter(|t| !t.is_empty()) else {
        return scope.clone();
    };
    let mut out = scope.clone();
    let Some(query) = out.query.as_mut() else { return out };
    // The text read in photos is searched only while *Read text in photos* is on (Round 13).
    query.photo_text = crate::models::read_text_on(state);
    let parsed = parse(state, text);
    if let Some(parsed) = &parsed {
        *query = query_parse::merge(query, &parsed.query);
    }
    if let Some(rest) = query.text.as_deref().map(str::trim).filter(|t| t.chars().count() >= 3) {
        query.semantic = scored_for(state, rest).map(|s| semantic_hits(&s));
    }
    if scope.best_match {
        let ranked_by = parsed.as_deref().map(rank_text).unwrap_or_else(|| text.to_string());
        if !ranked_by.trim().is_empty() {
            query.rank = scored_for(state, &ranked_by).map(|s| s.iter().map(|(id, _)| *id).collect());
        }
    }
    out
}

/// `query` with its tag sequence turned into `expr` (ANDed with a tree it already has), each
/// text tag and the typed `text` read by `read` as one term; `tag_expr` and `text` are cleared.
/// `None` when there is no sequence, it is empty or it is plain: `expanded()` runs it as a flat
/// tag list, as before.
fn with_tag_tree(query: &AssetQuery, read: &mut impl FnMut(&str) -> QueryExpr) -> Option<AssetQuery> {
    let seq = query.tag_expr.as_ref()?.repaired();
    if seq.as_plain().is_some() {
        return None;
    }
    let tree = seq.to_expr(&mut |tag| match &tag.value {
        TagValue::Text { text } => read(text.trim()),
        _ => tag_expr::core_leaf(tag),
    })?;
    let mut terms: Vec<QueryExpr> = query.expr.as_deref().cloned().into_iter().collect();
    terms.push(tree);
    terms.extend(query.text.as_deref().map(str::trim).filter(|t| !t.is_empty()).map(&mut *read));
    let mut out = query.clone();
    out.tag_expr = None;
    out.text = None;
    out.expr = Some(Box::new(if terms.len() == 1 { terms.remove(0) } else { QueryExpr::And(terms) }));
    Some(out)
}

/// One text tag or typed text as a term: its `or` / `and` / `not` tree, else its words read
/// against the vocabulary like typed text (people, places, dates…), the rest left as words.
fn text_term(state: &AppState, text: &str) -> QueryExpr {
    if let Some(parsed) = parse_expression(state, text) {
        return parsed.expr.clone();
    }
    let words = AssetQuery { text: Some(text.to_string()).filter(|t| !t.is_empty()), ..Default::default() };
    let query = parse(state, text).map(|p| query_parse::merge(&words, &p.query)).unwrap_or(words);
    QueryExpr::Leaf(Box::new(query))
}

/// `scope` with its text read as a tree of terms, or its saved tree, each term's words searched
/// by meaning. `None` when there is no tree: today's reading applies.
fn with_expression(state: &AppState, scope: &LayoutScope) -> Option<LayoutScope> {
    let mut out = scope.clone();
    let query = out.query.as_mut()?;
    if query.expr.is_none() {
        let text = query.text.as_deref().map(str::trim).filter(|t| !t.is_empty())?;
        let parsed = parse_expression(state, text)?;
        query.expr = Some(Box::new(parsed.expr.clone()));
        // The words now live in the terms; left here they would narrow everything again.
        query.text = None;
    }
    query.photo_text = crate::models::read_text_on(state);
    let photo_text = query.photo_text;
    let expr = query.expr.as_deref_mut()?;
    let ranked = fill_terms(expr, photo_text, |rest| scored_for(state, rest));
    if scope.best_match && !ranked.is_empty() {
        let lists: Vec<&[(i64, f32)]> = ranked.iter().map(|s| s.as_slice()).collect();
        query.rank = Some(query_expr::best_first(&lists));
    }
    Some(out)
}

/// Fills in each term of `expr` as [`understood`] fills a whole query: the text read in photos
/// while that is on, and the photos its words find by meaning. Returns the meaning scans of the
/// terms that count for photos, which order *Best match*; a `not` term's hits only exclude.
fn fill_terms(expr: &mut QueryExpr, photo_text: bool, scored: impl Fn(&str) -> Option<Scored>) -> Vec<Scored> {
    let mut ranked = Vec::new();
    expr.leaves_mut(&mut |leaf, positive| {
        leaf.photo_text = photo_text;
        let Some(rest) = leaf.text.as_deref().map(str::trim).filter(|t| t.chars().count() >= 3) else { return };
        if let Some(s) = scored(rest) {
            leaf.semantic = Some(semantic_hits(&s));
            if positive {
                ranked.push(s);
            }
        }
    });
    ranked
}

/// `text` read as `or` / `and` / `not` of terms (Round 19 §6), remembered like [`parse`].
/// `None` when it has no operator.
pub fn parse_expression(state: &AppState, text: &str) -> Option<Arc<ParsedExpr>> {
    let version = state.changes.current();
    let order = crate::locale::current(state);
    let key = format!("{order:?} {text}");
    if let Some(p) = recall(&state.search.expressions, &key, version) {
        return p;
    }
    let lib = state.library().ok()?;
    let vocab = vocabulary(state, version)?;
    let today = chrono::Local::now().date_naive();
    let parsed = lib
        .db
        .with_conn(|c| {
            Ok(query_expr::parse_expr(text, &vocab, today, 0, order, |q| {
                let scope = LayoutScope { query: Some(q.clone()), ..Default::default() };
                count_scoped(c, &scope).unwrap_or(0).max(0) as u64
            }))
        })
        .ok()?
        .map(Arc::new);
    remember(&state.search.expressions, key, version, parsed.clone());
    parsed
}

/// What the results page's overview names for a tree: the people, pets, places and albums of
/// the terms that count for photos, every chip with its operators, and their leftover words.
fn overview_reading(read: &ParsedExpr) -> Parsed {
    let mut query = AssetQuery::default();
    let mut rest: Vec<String> = Vec::new();
    let mut expr = read.expr.clone();
    expr.leaves_mut(&mut |leaf, positive| {
        if !positive {
            return;
        }
        query.person_ids.extend(&leaf.person_ids);
        query.pet_ids.extend(&leaf.pet_ids);
        query.places.extend(leaf.places.iter().cloned());
        query.album_ids.extend(&leaf.album_ids);
        query.collection_ids.extend(&leaf.collection_ids);
        rest.extend(leaf.text.as_deref().map(str::trim).filter(|t| !t.is_empty()).map(String::from));
    });
    Parsed { query, chips: read.chips.clone(), rest: rest.join(" ") }
}

/// What *Best match* ranks by: the words left over and the things recognised (tags, kinds),
/// which have a look; never a name or a date, which CLIP cannot see. Empty means newest first.
fn rank_text(parsed: &Parsed) -> String {
    let mut parts: Vec<String> = Vec::new();
    if !parsed.rest.trim().is_empty() {
        parts.push(parsed.rest.clone());
    }
    for chip in parsed.chips.iter().filter(|c| matches!(c.kind, ChipKind::Tag | ChipKind::Kind)) {
        parts.push(chip.label.to_lowercase());
    }
    parts.join(" ")
}

/// The results page's overview of `text`: its chips, and the people, places, albums and
/// memories it names, with how many photos the whole search finds.
pub fn overview(state: &AppState, text: &str) -> AppResult<SearchOverview> {
    let lib = state.library()?;
    let tree = parse_expression(state, text).map(|e| Arc::new(overview_reading(&e)));
    let parsed = tree.or_else(|| parse(state, text)).unwrap_or_else(|| {
        Arc::new(Parsed { query: AssetQuery { text: Some(text.to_string()), ..Default::default() }, chips: Vec::new(), rest: text.to_string() })
    });
    let scope = understood(state, &LayoutScope { query: Some(AssetQuery { text: Some(text.to_string()), ..Default::default() }), ..Default::default() });
    Ok(lib.db.with_conn(|c| {
        let mut o = photos_core::search_overview::overview(c, text, &parsed)?;
        o.total = count_scoped(c, &scope)?;
        Ok(o)
    })?)
}

/// `text` read against the open library's vocabulary, remembered per library version.
pub fn parse(state: &AppState, text: &str) -> Option<Arc<Parsed>> {
    let version = state.changes.current();
    // A change of date order reads the same text another way.
    let order = crate::locale::current(state);
    let key = format!("{order:?} {text}");
    if let Some(p) = recall(&state.search.parsed, &key, version) {
        return Some(p);
    }
    let lib = state.library().ok()?;
    let vocab = vocabulary(state, version)?;
    let today = chrono::Local::now().date_naive();
    // `captured_at` is wall-clock time stored as if it were UTC, so "2023" starts at
    // 2023-01-01 00:00 of that clock: no offset.
    let offset = 0;
    let parsed = lib
        .db
        .with_conn(|c| {
            Ok(query_parse::parse(text, &vocab, today, offset, order, |q| {
                let scope = LayoutScope { query: Some(q.clone()), ..Default::default() };
                count_scoped(c, &scope).unwrap_or(0).max(0) as u64
            }))
        })
        .ok()?;
    let parsed = Arc::new(parsed);
    remember(&state.search.parsed, key, version, parsed.clone());
    Some(parsed)
}

fn vocabulary(state: &AppState, version: u64) -> Option<Arc<Vocabulary>> {
    let mut slot = state.search.vocabulary.lock().ok()?;
    if let Some((v, vocab)) = slot.as_ref() {
        if *v == version {
            return Some(vocab.clone());
        }
    }
    let lib = state.library().ok()?;
    let started = std::time::Instant::now();
    let vocab = Arc::new(lib.db.with_conn(Vocabulary::load).ok()?);
    tracing::debug!(people = vocab.people.len(), places = vocab.places.len(), ms = started.elapsed().as_millis() as u64, "search vocabulary");
    *slot = Some((version, vocab.clone()));
    Some(vocab)
}

/// The tag facets: the cached ones at once (re-read in the background when the library has
/// changed), else one cold read. Another library's facets are never returned.
pub fn current_facets(app: &AppHandle, state: &AppState) -> AppResult<Arc<Facets>> {
    let version = state.changes.current();
    let root = state.library()?.root.clone();
    let cached = state.search.facets.lock().ok().and_then(|f| f.clone()).filter(|(r, _, _)| *r == root);
    Ok(match cached {
        Some((_, v, f)) => {
            if v != version && !state.search.reading_facets.swap(true, Ordering::AcqRel) {
                let app = app.clone();
                std::thread::spawn(move || {
                    let state = app.state::<AppState>();
                    let _ = read_facets(&state, version);
                    state.search.reading_facets.store(false, Ordering::Release);
                });
            }
            f
        }
        None => cold_facets(&state.search, &root, || read_facets(state, version))?,
    })
}

/// What the tag search offers for `input` (Round 7). Facets are read once per library version;
/// after a change the last ones answer at once while newer ones are read in the background,
/// so typing never waits on a read of the whole catalogue (~0.7 s at 22k photos).
pub fn suggest_tags(app: &AppHandle, state: &AppState, input: &str) -> AppResult<Vec<TagSuggestion>> {
    let facets = current_facets(app, state)?;
    let mut tags = tag_search::suggest(&facets, input, chrono::Local::now().date_naive(), crate::locale::current(state));
    // *Has text*, *No text* and `Text:` only while text is read (Round 13-3, 18 C2).
    if !crate::models::read_text_on(state) {
        tags.retain(|t| !matches!(t.tag.value, tag_search::TagValue::HasText | tag_search::TagValue::NoText | tag_search::TagValue::PhotoText { .. }));
        // `Text: menu` answers with that token alone, so the box still offers the words.
        if tags.is_empty() && !input.trim().is_empty() {
            let text = input.trim().to_string();
            tags.push(tag_search::TagSuggestion { tag: tag_search::SearchTag { label: text.clone(), value: tag_search::TagValue::Text { text } }, field: "Text".into(), count: None });
        }
    }
    Ok(tags)
}

/// A typed sentence read into tags (Round 20 §2), with the facets [`suggest_tags`] reads;
/// `None` when it is one plain term.
pub fn read_search_sentence(app: &AppHandle, state: &AppState, input: &str) -> AppResult<Option<SentenceReading>> {
    let facets = current_facets(app, state)?;
    Ok(tag_sentence::read(&facets, input, chrono::Local::now().date_naive(), crate::locale::current(state)))
}

/// The facets of the library at `root` when none are cached: one read at a time (Round 15). On a
/// cold start every grid page's search asks at once, and each read the whole catalogue (4–7
/// "slow database call" lines together); now the first reads and the rest take its result.
fn cold_facets(cache: &SearchCache, root: &std::path::Path, read: impl FnOnce() -> AppResult<Arc<Facets>>) -> AppResult<Arc<Facets>> {
    let _one = cache.loading.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((_, _, f)) = cache.facets.lock().ok().and_then(|f| f.clone()).filter(|(r, _, _)| r == root) {
        return Ok(f);
    }
    read()
}

fn read_facets(state: &AppState, version: u64) -> AppResult<Arc<Facets>> {
    let lib = state.library()?;
    let started = std::time::Instant::now();
    let facets = Arc::new(Facets::load_from(&lib.db)?);
    tracing::debug!(facets = facets.len(), ms = started.elapsed().as_millis() as u64, "tag search facets");
    if let Ok(mut slot) = state.search.facets.lock() {
        *slot = Some((lib.root.clone(), version, facets.clone()));
    }
    Ok(facets)
}

/// The photos nearest to `text` by meaning (the mean of its `search_prompts`), best first,
/// remembered per library version, so the extra text-tower runs happen once per text.
fn scored_for(state: &AppState, text: &str) -> Option<Scored> {
    let version = state.changes.current();
    let key = text.to_lowercase();
    if let Some(s) = recall(&state.search.scored, &key, version) {
        return Some(s);
    }
    let query = crate::models::with_text(state, |enc| enc.embed_ensemble(text)).and_then(|r| r.ok())?;
    let lib = state.library().ok()?;
    let started = std::time::Instant::now();
    let scored = Arc::new(lib.db.with_conn(|c| semantic_scan(c, &query, SCAN_LIMIT)).ok()?);
    tracing::debug!(query = text, scanned = scored.len(), ms = started.elapsed().as_millis() as u64, "search by meaning");
    remember(&state.search.scored, key, version, scored.clone());
    Some(scored)
}

#[cfg(test)]
#[path = "search_eval.rs"]
mod quality;

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    #[test]
    fn a_cold_start_reads_the_facets_once() {
        let cache = Arc::new(SearchCache::default());
        let loads = Arc::new(AtomicUsize::new(0));
        let root = std::path::PathBuf::from("lib");
        let threads: Vec<_> = (0..4)
            .map(|_| {
                let (cache, loads, root) = (cache.clone(), loads.clone(), root.clone());
                std::thread::spawn(move || {
                    cold_facets(&cache, &root, || {
                        loads.fetch_add(1, Ordering::SeqCst);
                        std::thread::sleep(std::time::Duration::from_millis(50));
                        let f = Arc::new(Facets::default());
                        *cache.facets.lock().unwrap() = Some((root.clone(), 1, f.clone()));
                        Ok(f)
                    })
                    .unwrap()
                })
            })
            .collect();
        for t in threads {
            t.join().unwrap();
        }
        assert_eq!(loads.load(Ordering::SeqCst), 1, "every grid page's search asked at once; one read");
        // Another library's facets are never taken for this one's.
        let other = cold_facets(&cache, std::path::Path::new("other"), || {
            loads.fetch_add(1, Ordering::SeqCst);
            Ok(Arc::new(Facets::default()))
        });
        assert!(other.is_ok());
        assert_eq!(loads.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn every_term_is_searched_by_meaning_and_only_positive_ones_rank() {
        let term = |text: &str| QueryExpr::Leaf(Box::new(AssetQuery { text: Some(text.into()), ..Default::default() }));
        // "seaside or not forest", plus a two-letter term too short to search by meaning.
        let mut expr = QueryExpr::Or(vec![term("seaside"), QueryExpr::Not(Box::new(term("forest"))), term("ab")]);
        let scans = |text: &str| -> Option<Scored> {
            match text {
                "seaside" => Some(Arc::new(vec![(7, 0.9)])),
                "forest" => Some(Arc::new(vec![(8, 0.9)])),
                other => panic!("{other:?} is too short to scan"),
            }
        };
        let ranked = fill_terms(&mut expr, true, scans);
        assert_eq!(ranked.len(), 1, "a `not` term's hits only exclude");
        assert_eq!(ranked[0].as_slice(), &[(7, 0.9)]);
        let mut seen = Vec::new();
        expr.leaves_mut(&mut |q, _| seen.push((q.semantic.clone(), q.photo_text)));
        assert_eq!(seen, vec![(Some(vec![7]), true), (Some(vec![8]), true), (None, true)]);
    }

    #[test]
    fn a_tag_sequence_reads_its_text_tags_as_sentences() {
        use photos_core::tag_expr::{Join, TagItem, TagSeq};
        use photos_core::tag_search::{SearchTag, TagValue};
        let tag = |value: TagValue| TagItem::Tag { not: false, tag: SearchTag { label: String::new(), value } };
        let words = |text: &str| QueryExpr::Leaf(Box::new(AssetQuery { text: Some(text.into()), ..Default::default() }));
        // A typed sentence is read with its operators; anything else stays words.
        let mut read = Vec::new();
        let mut reader = |text: &str| {
            read.push(text.to_string());
            match text {
                "cats or dogs" => QueryExpr::Or(vec![words("cats"), words("dogs")]),
                other => words(other),
            }
        };
        // `Favorites and not "cats or dogs"`, with "beach" typed beside it.
        let seq = TagSeq {
            items: vec![tag(TagValue::Favorites), TagItem::Tag { not: true, tag: SearchTag { label: "cats or dogs".into(), value: TagValue::Text { text: "cats or dogs".into() } } }],
            joins: vec![Join::And],
        };
        let query = AssetQuery { tag_expr: Some(seq), text: Some("beach".into()), ..Default::default() };
        let out = with_tag_tree(&query, &mut reader).expect("not plain: read here");
        assert_eq!(read, vec!["cats or dogs", "beach"]);
        assert_eq!(out.tag_expr, None);
        assert_eq!(out.text, None, "the typed words are a term now");
        let favorites = tag_expr::core_leaf(&SearchTag { label: String::new(), value: TagValue::Favorites });
        let tree = QueryExpr::And(vec![favorites, QueryExpr::Not(Box::new(QueryExpr::Or(vec![words("cats"), words("dogs")])))]);
        assert_eq!(out.expr.as_deref(), Some(&QueryExpr::And(vec![tree, words("beach")])));

        // A plain sequence (`(photos or videos) and favorites`) goes the old way, through `expanded()`.
        let media = TagSeq { items: vec![tag(TagValue::Media { media: 0 }), tag(TagValue::Media { media: 1 })], joins: vec![Join::Or] };
        let plain = TagSeq { items: vec![TagItem::Group { not: false, seq: media }, tag(TagValue::Favorites)], joins: vec![Join::And] };
        let query = AssetQuery { tag_expr: Some(plain), ..Default::default() };
        assert!(with_tag_tree(&query, &mut |_: &str| panic!("plain: nothing to read")).is_none());
        assert!(with_tag_tree(&AssetQuery { tag_expr: Some(TagSeq::default()), ..Default::default() }, &mut |_: &str| unreachable!()).is_none());
    }
}
