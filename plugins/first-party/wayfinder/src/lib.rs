//! Wayfinder: guesses where photos without a location were taken (spec §10).
//!
//! `find_locations` is one tool that takes several calls: a call has 30 s, so each one works for
//! about [`BUDGET_SECS`] and answers `{"status":"working",…}` until everything is done. All state
//! lives in the plugin's storage (an idle instance is dropped by the host), in this order:
//!
//! 1. `job.json`: the library listing (camera places, photos without a place);
//! 2. `personal.bin` + `personal-ids.json`: the person's own camera-GPS photos, embedded once;
//! 3. `dense.bin`: extra gallery points around the person's places, from the location encoder;
//! 4. `results-<n>.json`: each photo's candidates (the cache that makes a re-run instant);
//! 5. `session.json`: the review's state, which `on-review` loads, changes and saves.
//!
//! Only the pure helpers build natively (`cargo test`); the plugin itself is wasm32-only.

// `as_chunks` is newer than the toolchain this plugin builds with everywhere.
#![allow(unknown_lints, clippy::chunks_exact_to_as_chunks)]

pub mod logic;

use serde::{Deserialize, Serialize};

/// What a call may spend before it answers `working` (the host stops a call at 30 s).
pub const BUDGET_SECS: u64 = 18;
/// The image model's embedding width (GeoCLIP: 512).
pub const DIM: usize = 512;

/// The tool's arguments.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Args {
    pub scope: String,
    pub album: Option<i64>,
    pub refresh: bool,
}

pub fn parse_args(json: &str) -> Result<Args, String> {
    let v: serde_json::Value = if json.trim().is_empty() { serde_json::json!({}) } else { serde_json::from_str(json).map_err(|e| format!("the arguments aren't JSON: {e}"))? };
    let scope = v.get("scope").and_then(|s| s.as_str()).unwrap_or("library").to_string();
    let album = v.get("album").and_then(|a| a.as_i64());
    match (scope.as_str(), album) {
        ("library" | "selection", _) => {}
        ("album", Some(_)) => {}
        ("album", None) => return Err("scope album needs an album id".into()),
        (other, _) => return Err(format!("scope is selection, album or library, not {other}")),
    }
    Ok(Args { scope, album, refresh: v.get("refresh").and_then(|r| r.as_bool()).unwrap_or(false) })
}

/// The place a folder's name may point to: `Paris 2024` → `Paris`, `2024-07 Bled` → `Bled`;
/// `None` for names that are no place (`DCIM`, `100APPLE`, `Camera`, a bare date).
pub fn folder_query(path: &str) -> Option<String> {
    const NOT_PLACES: [&str; 22] = [
        "dcim", "camera", "photos", "pictures", "images", "image", "img", "screenshots", "download", "downloads", "whatsapp", "apple", "misc", "new folder", "backup", "export", "exports", "originals", "raw", "jpg", "sd", "card",
    ];
    let last = path.split(['/', '\\']).rfind(|p| !p.is_empty())?;
    let words: Vec<String> = last.split(|c: char| c.is_ascii_digit() || matches!(c, '_' | '-' | '.' | ',' | '(' | ')')).flat_map(|w| w.split_whitespace()).map(str::to_string).collect();
    let name = words.join(" ");
    let lower = name.to_lowercase();
    if name.chars().filter(|c| c.is_alphabetic()).count() < 3 || NOT_PLACES.iter().any(|n| lower == *n || lower.ends_with(&format!(" {n}")) || lower.starts_with(&format!("{n} "))) {
        return None;
    }
    Some(name)
}

#[cfg(target_arch = "wasm32")]
mod plugin {
    use std::collections::{BTreeMap, HashMap, HashSet};
    use std::time::{Duration, Instant};

    use lumen_plugin::{
        library, log, models, places, setting, storage, Asset, AssetFilter, Candidate, GpsSource, MediaKind, PlaceFlag, PlaceState, Review, ReviewEvent, Suggestion, Tensor, Tool, Tools, VectorSource,
    };
    use serde::de::DeserializeOwned;
    use serde::{Deserialize, Serialize};
    use serde_json::json;

    use crate::logic::{self, Anchor, Cand, Event, Hit, Item, Lwvg, Out, Point, Prior, Session};
    use crate::{folder_query, parse_args, Args, BUDGET_SECS, DIM};

    const JOB: &str = "job.json";
    const SESSION: &str = "session.json";
    const PERSONAL: &str = "personal.bin";
    const PERSONAL_IDS: &str = "personal-ids.json";
    const DENSE: &str = "dense.bin";
    const DENSE_KEY: &str = "dense.key";
    const RESULTS: &str = "results-";
    /// Rows asked of the host's top-k; GeoCLIP's gallery and the dense points each give this many.
    const TOP_K: u32 = 50;
    /// Points per call of the location encoder.
    const LOCATION_CHUNK: usize = 2048;
    const SUGGEST_CHUNK: usize = 200;

    #[derive(Clone, Serialize, Deserialize)]
    struct Camera {
        id: u64,
        time: Option<i64>,
        at: Point,
        photo: bool,
    }

    #[derive(Clone, Serialize, Deserialize)]
    struct Target {
        id: u64,
        time: Option<i64>,
        folder: Option<String>,
        burst: Option<String>,
        photo: bool,
    }

    #[derive(Serialize, Deserialize)]
    struct Job {
        args: Args,
        include_estimated: bool,
        cameras: Vec<Camera>,
        targets: Vec<Target>,
        /// Photos the model couldn't read: never asked again.
        failed: Vec<u64>,
    }

    fn load<T: DeserializeOwned>(name: &str) -> Option<T> {
        storage::read(name).ok().flatten().and_then(|b| serde_json::from_slice(&b).ok())
    }

    fn save<T: Serialize>(name: &str, v: &T) -> Result<(), String> {
        storage::write(name, &serde_json::to_vec(v).map_err(|e| e.to_string())?)
    }

    fn time_of(a: &Asset) -> Option<i64> {
        a.captured_at.as_deref().and_then(logic::parse_time)
    }

    fn filter(has_gps: bool, args: &Args) -> AssetFilter {
        AssetFilter { has_gps: Some(has_gps), since: None, until: None, album: if args.scope == "album" { args.album } else { None }, selection: args.scope == "selection" }
    }

    /// Lists the library: every camera place (an anchor, and the person's own photos), and the
    /// photos to find places for in the chosen scope.
    fn list(args: &Args) -> Result<Job, String> {
        let include_estimated = setting::<bool>("include_estimated").unwrap_or(false);
        let mut cameras = Vec::new();
        let everything = AssetFilter { has_gps: Some(true), since: None, until: None, album: None, selection: false };
        library::each_page(&everything, |page| {
            for a in page {
                if let Some(g) = a.gps.as_ref().filter(|g| g.source == GpsSource::Camera) {
                    cameras.push(Camera { id: a.id, time: time_of(&a), at: Point::new(g.lat, g.lon), photo: a.kind == MediaKind::Photo });
                }
            }
            true
        })?;
        let mut targets = Vec::new();
        let mut add = |page: Vec<Asset>, only_estimated: bool| {
            for a in page {
                if only_estimated && a.gps.as_ref().is_none_or(|g| g.source != GpsSource::Estimated) {
                    continue;
                }
                targets.push(Target { id: a.id, time: time_of(&a), folder: a.folder.clone(), burst: a.group.clone(), photo: a.kind == MediaKind::Photo });
            }
        };
        library::each_page(&filter(false, args), |page| {
            add(page, false);
            true
        })?;
        if include_estimated {
            library::each_page(&filter(true, args), |page| {
                add(page, true);
                true
            })?;
        }
        Ok(Job { args: args.clone(), include_estimated, cameras, targets, failed: Vec::new() })
    }

    fn embed(asset: u64) -> Result<Vec<f32>, String> {
        let out = models::run_on_asset("image", asset, "pixel_values")?;
        out.into_iter().next().map(|t| t.data).filter(|d| d.len() == DIM).ok_or_else(|| "the model gave no embedding".to_string())
    }

    fn hits(source: &VectorSource, q: &[f32]) -> Vec<Hit> {
        models::nearest(source, q, TOP_K).map(|rows| rows.into_iter().filter(|r| r.payload.len() >= 2).map(|r| Hit { at: Point::new(f64::from(r.payload[0]), f64::from(r.payload[1])), score: r.score }).collect()).unwrap_or_default()
    }

    /// Candidates for one embedding: GeoCLIP's gallery, the dense points, the person's nearest photo.
    fn candidates(q: &[f32], have_personal: bool, have_dense: bool, centres: &[Point]) -> Vec<Cand> {
        let gallery = hits(&VectorSource::Model("gallery".into()), q);
        let dense = if have_dense { hits(&VectorSource::Storage(DENSE.into()), q) } else { Vec::new() };
        let personal = if have_personal { models::nearest(&VectorSource::Storage(PERSONAL.into()), q, 1).ok().and_then(|r| r.into_iter().next()).filter(|r| r.payload.len() >= 2).map(|r| Hit { at: Point::new(f64::from(r.payload[0]), f64::from(r.payload[1])), score: r.score }) } else { None };
        logic::rank(&gallery, &dense, personal, centres)
    }

    fn working(phase: &str, done: usize, total: usize) -> Result<String, String> {
        Ok(json!({ "status": "working", "phase": phase, "done": done, "total": total }).to_string())
    }

    fn forget() {
        for name in storage::names() {
            if name.starts_with(RESULTS) || [JOB, SESSION, PERSONAL, PERSONAL_IDS, DENSE, DENSE_KEY].contains(&name.as_str()) {
                let _ = storage::remove(&name);
            }
        }
    }

    fn missing(e: &str) -> bool {
        e.starts_with("model_missing")
    }

    /// Embeds the person's own camera-GPS photos that aren't in `personal.bin` yet. `true` when none are left.
    fn learn_places(job: &mut Job, deadline: Instant) -> Result<bool, String> {
        let mut ids: Vec<u64> = load(PERSONAL_IDS).unwrap_or_default();
        let mut file = storage::read(PERSONAL).ok().flatten().and_then(|b| Lwvg::parse(&b)).filter(|f| f.dim == DIM && f.rows() == ids.len()).unwrap_or_else(|| {
            ids.clear();
            Lwvg::new(DIM)
        });
        let have: HashSet<u64> = ids.iter().copied().chain(job.failed.iter().copied()).collect();
        let todo: Vec<&Camera> = job.cameras.iter().filter(|c| c.photo && !have.contains(&c.id)).collect();
        let mut done = true;
        let before = ids.len();
        let mut failed = Vec::new();
        for c in todo {
            if Instant::now() >= deadline {
                done = false;
                break;
            }
            match embed(c.id) {
                Ok(e) => {
                    file.push(&e, c.at);
                    ids.push(c.id);
                }
                Err(e) if missing(&e) => return Err(e),
                Err(_) => failed.push(c.id),
            }
        }
        job.failed.extend(failed);
        if ids.len() != before {
            storage::write(PERSONAL, &file.to_bytes())?;
            save(PERSONAL_IDS, &ids)?;
        }
        Ok(done)
    }

    /// Encodes the extra gallery points around the person's places once (again when the places change).
    fn ensure_dense(centres: &[Point]) -> Result<bool, String> {
        if centres.is_empty() {
            return Ok(false);
        }
        let step = logic::dense_step_km(centres);
        let key = format!("{step}:{}:{:.3},{:.3}", centres.len(), centres[0].lat, centres[0].lon);
        let current = storage::read(DENSE_KEY).ok().flatten().map(|b| String::from_utf8_lossy(&b).to_string());
        if current.as_deref() == Some(&key) && storage::read(DENSE).ok().flatten().is_some() {
            return Ok(true);
        }
        let points = logic::dense_points(centres, step);
        let mut file = Lwvg::new(DIM);
        for chunk in points.chunks(LOCATION_CHUNK) {
            let data: Vec<f32> = chunk.iter().flat_map(|p| [p.lat as f32, p.lon as f32]).collect();
            let out = models::run("location", &[Tensor { name: "latlon".into(), shape: vec![chunk.len() as u32, 2], data }])?;
            let flat = out.into_iter().next().map(|t| t.data).ok_or("the location model gave nothing")?;
            if flat.len() != chunk.len() * DIM {
                return Err("the location model's output has the wrong size".into());
            }
            for (p, row) in chunk.iter().zip(flat.chunks_exact(DIM)) {
                file.push(row, *p);
            }
        }
        storage::write(DENSE, &file.to_bytes())?;
        storage::write(DENSE_KEY, key.as_bytes())?;
        log(&format!("{} extra gallery points around {} places", points.len(), centres.len()));
        Ok(true)
    }

    fn results() -> (HashMap<u64, Vec<Cand>>, usize) {
        let mut map = HashMap::new();
        let mut shards = 0;
        for name in storage::names().into_iter().filter(|n| n.starts_with(RESULTS)) {
            shards += 1;
            if let Some(rows) = load::<Vec<(u64, Vec<Cand>)>>(&name) {
                map.extend(rows);
            }
        }
        (map, shards)
    }

    fn to_wit(o: &Out) -> Suggestion {
        Suggestion {
            asset: o.asset,
            candidates: o.cands.iter().take(5).map(|c| Candidate { lat: c.at.lat, lon: c.at.lon, confidence: c.conf, label: c.label.clone() }).collect(),
            state: match o.state {
                logic::State::Anchor => PlaceState::Anchor,
                logic::State::Inferred => PlaceState::Inferred,
                logic::State::Guess => PlaceState::Guess,
            },
            group: o.group.clone(),
            group_label: o.label.clone(),
            flag: o.flag.map(|f| match f {
                logic::Flag::FarFromGroup => PlaceFlag::FarFromGroup,
                logic::Flag::ImpossibleSpeed => PlaceFlag::ImpossibleSpeed,
                logic::Flag::LowConfidence => PlaceFlag::LowConfidence,
            }),
        }
    }

    fn suggest(outs: &[Out]) -> Result<(), String> {
        for chunk in outs.chunks(SUGGEST_CHUNK) {
            places::suggest(&chunk.iter().map(to_wit).collect::<Vec<_>>())?;
        }
        Ok(())
    }

    /// Looks up the places folder names point to (the offline atlas), once per distinct name.
    fn folder_priors(targets: &[Target]) -> BTreeMap<String, Prior> {
        let mut by_query: HashMap<String, Option<Prior>> = HashMap::new();
        let mut priors = BTreeMap::new();
        for folder in targets.iter().filter_map(|t| t.folder.as_ref()).collect::<HashSet<_>>() {
            let Some(q) = folder_query(folder) else { continue };
            if by_query.len() >= 300 && !by_query.contains_key(&q) {
                continue;
            }
            let prior = by_query.entry(q.clone()).or_insert_with(|| places::lookup(&q).into_iter().next().map(|h| Prior { at: Point::new(h.lat, h.lon), name: h.name })).clone();
            if let Some(p) = prior {
                priors.insert(folder.clone(), p);
            }
        }
        priors
    }

    fn find(args: Args) -> Result<String, String> {
        let deadline = Instant::now() + Duration::from_secs(BUDGET_SECS);
        if args.refresh {
            forget();
        }
        let mut job = match load::<Job>(JOB).filter(|j| j.args == Args { refresh: false, ..args.clone() }) {
            Some(j) => j,
            None => {
                let j = list(&Args { refresh: false, ..args.clone() })?;
                save(JOB, &j)?;
                log(&format!("{} camera places, {} photos to place", j.cameras.len(), j.targets.len()));
                j
            }
        };
        if job.targets.is_empty() {
            return Ok(json!({ "status": "done", "suggested": 0, "message": "Every photo in this scope already has a place." }).to_string());
        }

        // The person's own places first: both the extra gallery and the nearest-own-photo need them.
        let learned = learn_places(&mut job, deadline);
        save(JOB, &job)?;
        if !learned? {
            let ids: Vec<u64> = load(PERSONAL_IDS).unwrap_or_default();
            return working("Learning your own places", ids.len(), job.cameras.iter().filter(|c| c.photo).count());
        }
        let points: Vec<Point> = job.cameras.iter().map(|c| c.at).collect();
        let centres = logic::clusters(&points);
        let have_dense = ensure_dense(&centres)?;
        let have_personal = storage::read(PERSONAL).ok().flatten().is_some();

        // Then the photos without a place.
        let (mut found, shards) = results();
        let failed: HashSet<u64> = job.failed.iter().copied().collect();
        let todo: Vec<&Target> = job.targets.iter().filter(|t| t.photo && !found.contains_key(&t.id) && !failed.contains(&t.id)).collect();
        let total = job.targets.iter().filter(|t| t.photo).count();
        let mut fresh: Vec<(u64, Vec<Cand>)> = Vec::new();
        let mut broke_early = false;
        for t in &todo {
            if Instant::now() >= deadline {
                broke_early = true;
                break;
            }
            match embed(t.id) {
                Ok(q) => fresh.push((t.id, candidates(&q, have_personal, have_dense, &centres))),
                Err(e) if missing(&e) => return Err(e),
                Err(_) => fresh.push((t.id, Vec::new())),
            }
        }
        if !fresh.is_empty() {
            save(&format!("{RESULTS}{shards}.json"), &fresh)?;
            found.extend(fresh);
        }
        if broke_early {
            return working("Looking at your photos", found.len(), total);
        }

        // Everything has candidates: build the review and hand it to Lumen.
        let items: Vec<Item> = job.targets.iter().map(|t| Item::new(t.id, t.time, t.burst.clone(), t.folder.clone(), found.get(&t.id).cloned().unwrap_or_default())).collect();
        let anchors: Vec<Anchor> = job.cameras.iter().filter_map(|c| c.time.map(|time| Anchor { time, at: c.at })).collect();
        let session = Session::new(anchors, items, folder_priors(&job.targets));
        let outs = session.outputs();
        save(SESSION, &session)?;
        suggest(&outs)?;
        let _ = storage::remove(JOB);
        log(&format!("{} suggestions in {} moments", outs.len(), session.moment_count()));
        Ok(json!({ "status": "done", "suggested": outs.len(), "moments": session.moment_count(), "review": "/review/places?plugin=jl.wayfinder" }).to_string())
    }

    struct Wayfinder;

    impl Tools for Wayfinder {
        fn tools() -> Vec<Tool> {
            let schema = json!({
                "type": "object",
                "properties": {
                    "scope": { "type": "string", "enum": ["selection", "album", "library"], "description": "Which photos (default library). Only photos without a place are looked at." },
                    "album": { "type": "integer", "description": "The album's id, for scope album." },
                    "refresh": { "type": "boolean", "description": "Forget earlier guesses and start again." }
                }
            });
            vec![Tool { name: "find_locations".into(), input_schema: schema.to_string() }]
        }

        fn call(name: String, args_json: String) -> Result<String, String> {
            if name != "find_locations" {
                return Err(format!("no tool {name}"));
            }
            find(parse_args(&args_json)?)
        }
    }

    impl Review for Wayfinder {
        fn on_review(event: ReviewEvent) -> Vec<Suggestion> {
            let Some(mut session) = load::<Session>(SESSION) else { return Vec::new() };
            session.resolve();
            let event = match event {
                ReviewEvent::Confirm(ids) => Event::Confirm(ids),
                ReviewEvent::Reject(ids) => Event::Reject(ids),
                ReviewEvent::Move(m) => Event::Move { assets: m.assets, at: Point::new(m.lat, m.lon) },
                ReviewEvent::Split(s) => Event::Split { group: s.group, at_asset: s.at_asset },
            };
            let changed = session.apply(&event);
            if let Err(e) = save(SESSION, &session) {
                log(&format!("the review couldn't be saved: {e}"));
            }
            changed.iter().map(to_wit).collect()
        }
    }

    lumen_plugin::export_places!(Wayfinder, Wayfinder);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folder_names_become_place_queries() {
        assert_eq!(folder_query("D:/Photos/Paris 2024").as_deref(), Some("Paris"));
        assert_eq!(folder_query("C:\\Users\\a\\2024-07 Bled").as_deref(), Some("Bled"));
        assert_eq!(folder_query("/x/Lake Garda (summer)/").as_deref(), Some("Lake Garda summer"));
        for no in ["E:/DCIM/100APPLE", "E:/DCIM", "Camera", "2024-07-12", "IMG", "/x/Screenshots", "Misc", ""] {
            assert_eq!(folder_query(no), None, "{no}");
        }
    }

    #[test]
    fn arguments_are_checked() {
        assert_eq!(parse_args("").unwrap(), Args { scope: "library".into(), album: None, refresh: false });
        assert_eq!(parse_args(r#"{"scope":"album","album":4,"refresh":true}"#).unwrap(), Args { scope: "album".into(), album: Some(4), refresh: true });
        assert!(parse_args(r#"{"scope":"album"}"#).is_err());
        assert!(parse_args(r#"{"scope":"moon"}"#).is_err());
        assert!(parse_args("{nope").is_err());
    }
}
