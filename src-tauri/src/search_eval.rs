//! Search quality on a real library (Round 4K Task 3). Runs on a **copy** of `library.db`;
//! thumbnails are only read. Writes a contact sheet of each query's candidates to
//! `LUMEN_EVAL_OUT`, and with `LUMEN_EVAL_MARKS` (the sheet's saved marks) prints precision@20
//! and recall for each rule.
//!
//! `LUMEN_DB=<copy> LUMEN_LIBRARY=<library> LUMEN_MODELS_DIR=<models> LUMEN_QUERIES="cat,dog"
//! LUMEN_EVAL_OUT=<dir> [LUMEN_EVAL_MARKS=<marks.json>] cargo test --release -p lumen --lib
//! search_quality_on_a_real_library -- --ignored --nocapture`

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use photos_core::embedding::{mean, semantic_hits, semantic_scan, Vector, SEMANTIC_BAND, SEMANTIC_FLOOR};
use photos_core::layout::{count_scoped, ids_scoped};
use photos_core::query::AssetQuery;
use photos_core::query_parse::{self, Vocabulary};
use photos_core::scope::LayoutScope;
use photos_ml::manifest::manifest;
use photos_ml::text::TextEncoder;
use rusqlite::Connection;

/// One rule's answer: what it returns, best first as the user would see it.
struct Answer {
    name: &'static str,
    ranked: Vec<i64>,
}

fn ids(c: &Connection, q: &AssetQuery) -> Vec<i64> {
    ids_scoped(c, &LayoutScope { query: Some(q.clone()), ..Default::default() }, 1_000_000).unwrap()
}

fn fts_ranked(c: &Connection, text: &str) -> Vec<i64> {
    let expr = AssetQuery::match_expression(text);
    if expr.is_empty() {
        return Vec::new();
    }
    let mut stmt = c.prepare("SELECT rowid FROM assets_fts WHERE assets_fts MATCH ?1 ORDER BY rank").unwrap();
    let rows = stmt.query_map([expr], |r| r.get(0)).unwrap();
    rows.filter_map(|r| r.ok()).collect()
}

/// Hits within the band of the best, or the top 1 % of the library, whichever is larger.
fn adaptive(scored: &[(i64, f32)], library: usize) -> Vec<i64> {
    let Some(top) = scored.first().map(|s| s.1) else { return Vec::new() };
    let band: Vec<i64> = scored.iter().filter(|(_, c)| *c >= SEMANTIC_FLOOR && *c >= top - SEMANTIC_BAND).map(|(id, _)| *id).collect();
    let share = (library / 100).max(1);
    if band.len() >= share {
        band
    } else {
        scored.iter().take(share).filter(|(_, c)| *c >= SEMANTIC_FLOOR).map(|(id, _)| *id).collect()
    }
}

/// Reciprocal-rank fusion (k = 60) of several rankings, restricted to `keep`.
fn rrf(lists: &[&[i64]], keep: &HashSet<i64>) -> Vec<i64> {
    let mut score: HashMap<i64, f64> = HashMap::new();
    for list in lists {
        for (rank, id) in list.iter().filter(|id| keep.contains(id)).enumerate() {
            *score.entry(*id).or_default() += 1.0 / (60.0 + rank as f64 + 1.0);
        }
    }
    let mut out: Vec<(i64, f64)> = score.into_iter().collect();
    out.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
    let mut ranked: Vec<i64> = out.into_iter().map(|(id, _)| id).collect();
    // What neither list ranked keeps its order after them.
    let seen: HashSet<i64> = ranked.iter().copied().collect();
    let mut rest: Vec<i64> = keep.iter().copied().filter(|id| !seen.contains(id)).collect();
    rest.sort_unstable();
    ranked.extend(rest);
    ranked
}

fn by_score(set: &[i64], cos: &HashMap<i64, f32>) -> Vec<i64> {
    let mut v = set.to_vec();
    // A photo the scan did not reach (a tag or word match far down the cosines) goes after.
    v.sort_by(|a, b| cos.get(b).unwrap_or(&-1.0).total_cmp(cos.get(a).unwrap_or(&-1.0)));
    v
}

fn answers(c: &Connection, enc: &mut TextEncoder, vocab: &Vocabulary, text: &str, library: usize) -> (Vec<String>, Vec<Answer>) {
    let today = chrono::Local::now().date_naive();
    let offset = 0; // captured_at is wall-clock time
    let parsed = query_parse::parse(text, vocab, today, offset, crate::locale::date_order(Default::default()), |q| {
        count_scoped(c, &LayoutScope { query: Some(q.clone()), ..Default::default() }).unwrap_or(0).max(0) as u64
    });
    let chips: Vec<String> = parsed.chips.iter().map(|ch| format!("{:?}: {}", ch.kind, ch.label)).collect();
    let filters = query_parse::merge(&AssetQuery::default(), &parsed.query);
    let rest = filters.text.clone().unwrap_or_default();
    let long_rest = rest.trim().chars().count() >= 3;
    // Meaning is asked of the rest, or of the whole text when all of it was recognised.
    let asked = if rest.trim().is_empty() { text.to_string() } else { rest.clone() };
    let single: Vector = enc.embed(&asked).unwrap();
    let prompts: Vec<Vector> =
        [format!("a photo of {asked}"), asked.clone(), format!("a picture showing {asked}")].iter().map(|p| enc.embed(p).unwrap()).collect();
    let ensemble = mean(&prompts.iter().map(|v| v.as_slice()).collect::<Vec<_>>()).unwrap();
    let scan1 = semantic_scan(c, &single, 3000).unwrap();
    let scan_e = semantic_scan(c, &ensemble, 3000).unwrap();
    let cos1: HashMap<i64, f32> = scan1.iter().copied().collect();
    let cos_e: HashMap<i64, f32> = scan_e.iter().copied().collect();
    let sem1: Vec<i64> = scan1.iter().map(|s| s.0).collect();
    let sem_e: Vec<i64> = scan_e.iter().map(|s| s.0).collect();
    let fts = fts_ranked(c, &rest);

    let with_hits = |hits: Option<Vec<i64>>| -> Vec<i64> {
        let mut q = filters.clone();
        q.semantic = hits;
        ids(c, &q)
    };
    // Today: the app's rule (meaning only over a rest of three letters or more), date order.
    let today_set = with_hits(long_rest.then(|| semantic_hits(&scan1)));
    let today_keep: HashSet<i64> = today_set.iter().copied().collect();
    let mut out = vec![
        Answer { name: "today (date order)", ranked: today_set.clone() },
        Answer { name: "today, best match (cosine)", ranked: by_score(&today_set, &cos1) },
        Answer { name: "today, RRF(fts, meaning)", ranked: rrf(&[&fts, &sem1], &today_keep) },
    ];
    if filters.tags.is_empty() {
        let set1 = with_hits(long_rest.then(|| adaptive(&scan1, library)));
        let set_e = with_hits(long_rest.then(|| adaptive(&scan_e, library)));
        out.push(Answer { name: "adaptive floor", ranked: by_score(&set1, &cos1) });
        out.push(Answer { name: "adaptive + ensemble", ranked: by_score(&set_e, &cos_e) });
        let keep: HashSet<i64> = set_e.iter().copied().collect();
        out.push(Answer { name: "adaptive + ensemble, RRF", ranked: rrf(&[&fts, &sem_e], &keep) });
    } else {
        // A recognised tag also finds what *shows* it: the other filters, AND (tag OR meaning).
        let tags_or = |hits: Vec<i64>| -> Vec<i64> {
            let mut base = filters.clone();
            base.tags.clear();
            base.text = None;
            let tagged: HashSet<i64> = today_set.iter().copied().collect();
            let hits: HashSet<i64> = hits.into_iter().collect();
            ids(c, &base).into_iter().filter(|id| tagged.contains(id) || hits.contains(id)).collect()
        };
        let band_set = tags_or(semantic_hits(&scan1));
        let adaptive_set = tags_or(adaptive(&scan_e, library));
        out.push(Answer { name: "tag or meaning (band)", ranked: by_score(&band_set, &cos1) });
        out.push(Answer { name: "tag or meaning (adaptive + ensemble)", ranked: by_score(&adaptive_set, &cos_e) });
        out.push(Answer { name: "tag, best match (ensemble)", ranked: by_score(&today_set, &cos_e) });
        // Added after the marks (Round 14): judged only on the photos the marks cover.
        let adaptive_single = tags_or(adaptive(&scan1, library));
        out.push(Answer { name: "+ tag or meaning (adaptive, single)", ranked: by_score(&adaptive_single, &cos1) });
        let tagged_first = |set: &[i64], cos: &HashMap<i64, f32>| -> Vec<i64> {
            let (mut a, b): (Vec<i64>, Vec<i64>) = by_score(set, cos).into_iter().partition(|id| today_keep.contains(id));
            a.extend(b);
            a
        };
        out.push(Answer { name: "+ tag first, then meaning (adaptive + ensemble)", ranked: tagged_first(&adaptive_set, &cos_e) });
        out.push(Answer { name: "+ tag first, then meaning (band, single)", ranked: tagged_first(&band_set, &cos1) });
    }
    // What the app does since Round 14: the batched ensemble, the band, best match first.
    let shipped = semantic_scan(c, &enc.embed_ensemble(&asked).unwrap(), 2000).unwrap();
    let cos_s: HashMap<i64, f32> = shipped.iter().copied().collect();
    let set = with_hits(long_rest.then(|| semantic_hits(&shipped)));
    out.push(Answer { name: "+ Round 14 (ensemble, band, best match)", ranked: by_score(&set, &cos_s) });
    // Without any rule: the raw nearest photos, which widen the labelled pool.
    out.push(Answer { name: "raw nearest (ensemble)", ranked: sem_e });
    (chips, out)
}

#[test]
#[ignore = "needs a copy of a real library and the models"]
fn search_quality_on_a_real_library() {
    let env = |k: &str| std::env::var(k).unwrap_or_else(|_| panic!("{k}"));
    let db = photos_core::db::Db::open(Path::new(&env("LUMEN_DB"))).unwrap();
    let library = PathBuf::from(env("LUMEN_LIBRARY"));
    let models = PathBuf::from(env("LUMEN_MODELS_DIR"));
    let out = PathBuf::from(env("LUMEN_EVAL_OUT"));
    std::fs::create_dir_all(&out).unwrap();
    let queries: Vec<String> = env("LUMEN_QUERIES").split(',').map(|q| q.trim().to_string()).filter(|q| !q.is_empty()).collect();
    let (m, t) = (manifest().by_role("text_embedding").unwrap(), manifest().by_role("tokenizer").unwrap());
    let mut enc = TextEncoder::open(&models.join(&m.file), &models.join(&t.file), m).unwrap();
    let store = photos_media::store::DerivativeStore::new(&library);
    let marks: Option<HashMap<String, serde_json::Value>> =
        std::env::var("LUMEN_EVAL_MARKS").ok().map(|p| serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap());

    let mut sheet = Vec::new();
    let mut report = String::from("| query | rule | returned | p@20 | recall |\n|---|---|---:|---:|---:|\n");
    let mut totals: HashMap<&'static str, (f64, f64, usize)> = HashMap::new();
    db.with_conn(|c| {
        let vocab = Vocabulary::load(c)?;
        let library_size = c.query_row("SELECT COUNT(*) FROM assets WHERE state = 0", [], |r| r.get::<_, i64>(0))? as usize;
        for q in &queries {
            let started = std::time::Instant::now();
            let (chips, answers) = answers(c, &mut enc, &vocab, q, library_size);
            println!("{q}: chips {chips:?} ({:?})", started.elapsed());
            for a in &answers {
                println!("  {:<40} {:>6}", a.name, a.ranked.len());
            }
            // The pool to label: the date-ordered top 20, and every other rule's top 40.
            let mut pool: Vec<i64> = Vec::new();
            // Rules named `+ …` came after the marks, so they never widen the pool.
            for a in answers.iter().filter(|a| !a.name.starts_with('+')) {
                let take = if a.name.starts_with("today (date") { 20 } else { 40 };
                for id in a.ranked.iter().take(take) {
                    if !pool.contains(id) {
                        pool.push(*id);
                    }
                }
            }
            let mut tiles = Vec::new();
            for id in &pool {
                let hash: String = c.query_row("SELECT content_hash FROM assets WHERE id = ?1", [id], |r| r.get(0))?;
                let path = store.thumb_path(&hash, 256).to_string_lossy().replace('\\', "/").replace(' ', "%20");
                tiles.push(serde_json::json!({ "id": id, "src": format!("file:///{path}") }));
            }
            sheet.push(serde_json::json!({ "q": q, "chips": chips, "tiles": tiles }));

            let Some(m) = marks.as_ref().and_then(|m| m.get(q)) else { continue };
            if m.get("skip").and_then(|v| v.as_bool()) == Some(true) {
                continue;
            }
            let wrong: HashSet<i64> = m["wrong"].as_array().map(|a| a.iter().filter_map(|v| v.as_i64()).collect()).unwrap_or_default();
            let relevant: HashSet<i64> = pool.iter().copied().filter(|id| !wrong.contains(id)).collect();
            let tagged_wrong = answers[0].ranked.iter().filter(|id| wrong.contains(id)).count();
            println!("  marked wrong: {} ({tagged_wrong} of them in today's set)", wrong.len());
            for a in answers.iter().filter(|a| !a.name.starts_with("raw")) {
                // Only photos Julian saw count; an unseen one in a later rule's top 20 is left out.
                let top: Vec<&i64> = a.ranked.iter().filter(|id| pool.contains(id)).take(20).collect();
                let unseen = a.ranked.iter().take(20).filter(|id| !pool.contains(id)).count();
                if unseen > 0 {
                    println!("  {}: {unseen} unseen in the top 20", a.name);
                }
                let p = if top.is_empty() { 0.0 } else { top.iter().filter(|id| relevant.contains(id)).count() as f64 / top.len() as f64 };
                let set: HashSet<&i64> = a.ranked.iter().collect();
                let r = if relevant.is_empty() { 0.0 } else { relevant.iter().filter(|id| set.contains(id)).count() as f64 / relevant.len() as f64 };
                report.push_str(&format!("| {q} | {} | {} | {p:.2} | {r:.2} |\n", a.name, a.ranked.len()));
                let e = totals.entry(a.name).or_default();
                e.0 += p;
                e.1 += r;
                e.2 += 1;
            }
        }
        Ok(())
    })
    .unwrap();

    let data = serde_json::to_string(&sheet).unwrap();
    std::fs::write(out.join("contact-sheet.html"), include_str!("search_eval.html").replace("/*DATA*/[]", &data)).unwrap();
    println!("contact sheet: {}", out.join("contact-sheet.html").display());
    if marks.is_some() {
        let mut rows: Vec<_> = totals.into_iter().collect();
        rows.sort_by(|a, b| a.0.cmp(b.0));
        report.push_str("\n| rule (mean over the queries it applies to) | queries | p@20 | recall |\n|---|---:|---:|---:|\n");
        for (name, (p, r, n)) in rows {
            report.push_str(&format!("| {name} | {n} | {:.3} | {:.3} |\n", p / n as f64, r / n as f64));
        }
        std::fs::write(out.join("report.md"), &report).unwrap();
        println!("{report}");
    }
}

/// Query to first page on a real library (Round 4K Task 5): reading the text, the meaning scan,
/// loading the ranked index and laying it out, each timed, as the app does them for `/search`.
///
/// `LUMEN_DB=<copy> LUMEN_MODELS_DIR=<models> LUMEN_QUERIES="cat,anna 2023" cargo test --release
/// -p lumen --lib search_timings_on_a_real_library -- --ignored --nocapture`
#[test]
#[ignore = "needs a copy of a real library and the models"]
fn search_timings_on_a_real_library() {
    use photos_core::layout::{GroupBy, LayoutIndex};
    use std::time::Instant;
    let env = |k: &str| std::env::var(k).unwrap_or_else(|_| panic!("{k}"));
    let db = photos_core::db::Db::open(Path::new(&env("LUMEN_DB"))).unwrap();
    let models = PathBuf::from(env("LUMEN_MODELS_DIR"));
    let (m, t) = (manifest().by_role("text_embedding").unwrap(), manifest().by_role("tokenizer").unwrap());
    let mut enc = TextEncoder::open(&models.join(&m.file), &models.join(&t.file), m).unwrap();
    let queries: Vec<String> = env("LUMEN_QUERIES").split(',').map(|q| q.trim().to_string()).filter(|q| !q.is_empty()).collect();
    db.with_conn(|c| {
        let started = Instant::now();
        let vocab = Vocabulary::load(c)?;
        println!("vocabulary: {} people, {} places, {} albums in {:?} (once per library version)", vocab.people.len(), vocab.places.len(), vocab.albums.len(), started.elapsed());
        let today = chrono::Local::now().date_naive();
        for q in &queries {
            let t0 = Instant::now();
            let parsed = query_parse::parse(q, &vocab, today, 0, crate::locale::date_order(Default::default()), |x| {
                count_scoped(c, &LayoutScope { query: Some(x.clone()), ..Default::default() }).unwrap_or(0).max(0) as u64
            });
            let t_parse = t0.elapsed();
            let mut query = query_parse::merge(&AssetQuery { text: Some(q.clone()), ..Default::default() }, &parsed.query);
            let t1 = Instant::now();
            let asked = if parsed.rest.trim().is_empty() { q.clone() } else { parsed.rest.clone() };
            let v = enc.embed_ensemble(&asked).unwrap();
            let t_embed = t1.elapsed();
            let t2 = Instant::now();
            let scored = semantic_scan(c, &v, 2000)?;
            let t_scan = t2.elapsed();
            if parsed.rest.trim().chars().count() >= 3 {
                query.semantic = Some(semantic_hits(&scored));
            }
            query.rank = Some(scored.iter().map(|s| s.0).collect());
            let scope = LayoutScope { best_match: true, query: Some(query), ..Default::default() };
            let t3 = Instant::now();
            let index = LayoutIndex::load_scoped(c, &scope, true)?;
            let t_load = t3.elapsed();
            let t4 = Instant::now();
            let layout = index.compute(1200, 200, 4, scope.sectioned().then_some(GroupBy::Month));
            let t_layout = t4.elapsed();
            let total = t0.elapsed();
            // The batch must agree with the prompts asked one by one, as the marks measured them.
            let t5 = Instant::now();
            let one = enc.embed(&asked).unwrap();
            let t_one = t5.elapsed();
            let one_by_one: Vec<_> = photos_core::embedding::search_prompts(&asked).iter().map(|p| enc.embed(p).unwrap()).collect();
            let agree = photos_core::embedding::cosine(&v, &mean(&one_by_one.iter().map(|v| v.as_slice()).collect::<Vec<_>>()).unwrap());
            assert!(agree > 0.999, "{q}: batch vs one by one {agree}");
            let _ = one;
            println!(
                "{q:>24}: {:>5} photos | parse {t_parse:>9.2?} embed {t_embed:>9.2?} (one prompt {t_one:>9.2?}) scan {t_scan:>9.2?} index {t_load:>9.2?} layout {t_layout:>9.2?} | total {total:>9.2?}",
                index.items.len()
            );
            let _ = layout;
        }
        Ok(())
    })
    .unwrap();
}

/// Names every geotagged photo of a **copy** of a library by its locality and searches the
/// places that gives (Round 4K): `LUMEN_DB=<copy> [LUMEN_QUERIES="Kropa"] cargo test --release
/// -p lumen --lib localities_on_a_real_library -- --ignored --nocapture`
#[test]
#[ignore = "needs a copy of a real library"]
fn localities_on_a_real_library() {
    use photos_core::places::Localities;
    use std::time::Instant;
    let db = photos_core::db::Db::open(Path::new(&std::env::var("LUMEN_DB").expect("LUMEN_DB"))).unwrap();
    let queries: Vec<String> = std::env::var("LUMEN_QUERIES").unwrap_or_else(|_| "Kropa".into()).split(',').map(|q| q.trim().to_string()).collect();
    let started = Instant::now();
    let g = Localities::load();
    println!("gazetteer: {} places in {:?}", g.len(), started.elapsed());
    let started = Instant::now();
    let mut named = 0;
    loop {
        let todo = db.with_conn(|c| photos_core::assets::list_needing_locality(c, 2000)).unwrap();
        if todo.is_empty() {
            break;
        }
        let rows: Vec<(i64, String)> = todo.into_iter().map(|(id, lat, lon)| (id, g.nearest(lat, lon).unwrap_or_default())).collect();
        named += db.with_conn(|c| photos_core::assets::set_localities(c, &rows)).unwrap();
    }
    println!("named {named} photos in {:?}", started.elapsed());
    db.with_conn(|c| {
        let mut stmt = c.prepare("SELECT locality, COUNT(*) FROM assets WHERE locality != '' GROUP BY 1 ORDER BY 2 DESC")?;
        let rows: Vec<(String, i64)> = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<std::result::Result<_, _>>()?;
        let empty: i64 = c.query_row("SELECT COUNT(*) FROM assets WHERE locality = ''", [], |r| r.get(0))?;
        println!("{} localities; {empty} geotagged photos have none within reach; the largest:", rows.len());
        for (l, n) in rows.iter().take(12) {
            println!("  {n:>5}  {l}");
        }
        let started = Instant::now();
        let vocab = Vocabulary::load(c)?;
        println!("vocabulary: {} places in {:?}", vocab.places.len(), started.elapsed());
        let today = chrono::Local::now().date_naive();
        for q in &queries {
            let p = query_parse::parse(q, &vocab, today, 0, crate::locale::date_order(Default::default()), |x| {
                count_scoped(c, &LayoutScope { query: Some(x.clone()), ..Default::default() }).unwrap_or(0).max(0) as u64
            });
            let n = count_scoped(c, &LayoutScope { query: Some(p.query.clone()), ..Default::default() })?;
            let chips: Vec<String> = p.chips.iter().map(|ch| format!("{:?}: {}", ch.kind, ch.label)).collect();
            println!("{q:>16}: {chips:?} rest {:?} -> {n} photos", p.rest);
        }
        Ok(())
    })
    .unwrap();
}
