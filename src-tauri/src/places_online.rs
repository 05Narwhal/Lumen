//! Exact place names from OpenStreetMap Nominatim (plan `2026-11-08-exact-place-names`).
//!
//! Opt-in (`Settings::exact_places`). Sends only a position, at most one request per second
//! (Nominatim's absolute limit), and keeps every answer in the library's `place_lookups`.
//! Photos share a lookup only when their positions are equal to 5 decimals (the border rule).
//! The atlas keeps naming everything at once and offline; this worker names every row the
//! atlas named (`place_source = 0`) again.
//!
//! It runs on its own thread, not the Maintenance lane: a first run on a large library takes
//! over an hour at 1 request/s and would hold every maintenance job behind it.

use std::sync::{Condvar, Mutex, OnceLock};
use std::time::{Duration, Instant};

use photos_core::assets;
use photos_core::places_online::{self, Address, Names};
use tauri::{AppHandle, Manager};

use crate::state::AppState;

/// Why a lookup gave no answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GeoError {
    /// No network (DNS, connect or timeout): try again later.
    Offline,
    /// The service refused us (429 / 403): back off.
    Limited,
    Other(String),
}

/// A reverse geocoder: the address at a position, and its `address` JSON to keep.
pub trait Geocoder {
    fn reverse(&self, lat: f64, lon: f64) -> Result<(Address, String), GeoError>;
}

/// How a run ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// Nothing is left to name.
    Done,
    /// Asked to stop (the switch went off, or Lumen is closing).
    Stopped,
    /// Try again after this long.
    RetryIn(Duration),
}

pub const OFFLINE_RETRY: Duration = Duration::from_secs(10 * 60);
pub const LIMITED_RETRY: Duration = Duration::from_secs(60 * 60);
const BATCH: usize = 50;
/// Just over Nominatim's 1 request per second.
pub const PACE: Duration = Duration::from_millis(1100);

/// Name every row waiting for an online name, cache first. `keep_going` is asked before each
/// request; `on_batch(named)` after each stored batch. Rows a failed lookup could not name stay
/// at `place_source = 0` for the next run.
pub fn run(
    db: &photos_core::Db,
    geocoder: &dyn Geocoder,
    pace: Duration,
    keep_going: &dyn Fn() -> bool,
    on_batch: &mut dyn FnMut(usize),
) -> Outcome {
    let mut last_request: Option<Instant> = None;
    loop {
        let Ok(todo) = db.with_conn(|c| assets::list_needing_online_place(c, BATCH)) else { return Outcome::RetryIn(OFFLINE_RETRY) };
        if todo.is_empty() {
            return Outcome::Done;
        }
        let mut named: Vec<(i64, Names)> = Vec::with_capacity(todo.len());
        let mut ended = None;
        for (id, key, lat, lon) in todo {
            if let Ok(Some(n)) = db.with_conn(|c| assets::lookup_cached(c, &key)) {
                named.push((id, n));
                continue;
            }
            if !keep_going() {
                ended = Some(Outcome::Stopped);
                break;
            }
            if let Some(t) = last_request {
                std::thread::sleep(pace.saturating_sub(t.elapsed()));
            }
            last_request = Some(Instant::now());
            match geocoder.reverse(lat, lon) {
                Ok((address, raw)) => {
                    let n = places_online::names_from_address(&address);
                    if let Err(e) = db.with_conn(|c| assets::cache_lookup(c, &key, &n, &raw, photos_core::now_unix())) {
                        tracing::warn!("place lookup not kept: {e}");
                    }
                    named.push((id, n));
                }
                Err(GeoError::Offline) => {
                    ended = Some(Outcome::RetryIn(OFFLINE_RETRY));
                    break;
                }
                Err(GeoError::Limited) => {
                    ended = Some(Outcome::RetryIn(LIMITED_RETRY));
                    break;
                }
                Err(GeoError::Other(e)) => {
                    // A bad answer for one point must not stop the rest; the next run tries again.
                    tracing::warn!("place lookup failed: {e}");
                    ended = Some(Outcome::RetryIn(OFFLINE_RETRY));
                    break;
                }
            }
        }
        if !named.is_empty() {
            match db.with_conn(|c| assets::set_online_places(c, &named)) {
                Ok(n) => on_batch(n),
                Err(e) => {
                    tracing::warn!("online place names not stored: {e}");
                    return Outcome::RetryIn(OFFLINE_RETRY);
                }
            }
        }
        if let Some(o) = ended {
            return o;
        }
    }
}

/// Nominatim's `/reverse`, in English, with Lumen's User-Agent and nothing about the person.
pub struct Nominatim {
    client: reqwest::Client,
}

impl Nominatim {
    pub fn new() -> Self {
        let client = reqwest::Client::builder()
            .user_agent(format!("Lumen/{} (JL Studios photo library)", env!("CARGO_PKG_VERSION")))
            .timeout(Duration::from_secs(10))
            .build()
            .expect("http client");
        Self { client }
    }
}

impl Default for Nominatim {
    fn default() -> Self {
        Self::new()
    }
}

impl Geocoder for Nominatim {
    fn reverse(&self, lat: f64, lon: f64) -> Result<(Address, String), GeoError> {
        let url = format!(
            "https://nominatim.openstreetmap.org/reverse?format=jsonv2&zoom=18&addressdetails=1&accept-language=en&lat={lat:.5}&lon={lon:.5}"
        );
        tauri::async_runtime::block_on(async {
            let res = self.client.get(&url).send().await.map_err(|e| {
                if e.is_connect() || e.is_timeout() || e.is_request() { GeoError::Offline } else { GeoError::Other(e.to_string()) }
            })?;
            let status = res.status().as_u16();
            if status == 429 || status == 403 {
                return Err(GeoError::Limited);
            }
            if !res.status().is_success() {
                return Err(GeoError::Other(format!("HTTP {status}")));
            }
            let body = res.text().await.map_err(|_| GeoError::Offline)?;
            places_online::parse_reverse(&body).map_err(|e| GeoError::Other(e.to_string()))
        })
    }
}

fn wake_signal() -> &'static (Mutex<bool>, Condvar) {
    static W: OnceLock<(Mutex<bool>, Condvar)> = OnceLock::new();
    W.get_or_init(|| (Mutex::new(false), Condvar::new()))
}

pub fn enabled(app: &AppHandle) -> bool {
    app.state::<AppState>().settings.read().expect("settings lock").exact_places
}

/// Ask the worker to look for rows to name: at startup, after an import, after positions were
/// found, when the switch turns on. Cheap; does nothing while the switch is off.
pub fn wake() {
    let (lock, cv) = wake_signal();
    *lock.lock().expect("wake lock") = true;
    cv.notify_all();
}

/// Starts the worker thread once. It sleeps until woken or until its retry tick (Lumen has no
/// OS "online" signal, so the tick is how it notices the person got online).
pub fn start(app: AppHandle) {
    static STARTED: OnceLock<()> = OnceLock::new();
    if STARTED.set(()).is_err() {
        return;
    }
    std::thread::Builder::new()
        .name("lumen-places-online".into())
        .spawn(move || {
            let geocoder = Nominatim::new();
            let mut retry: Option<Duration> = None;
            loop {
                {
                    let (lock, cv) = wake_signal();
                    let mut woken = lock.lock().expect("wake lock");
                    while !*woken {
                        match retry {
                            Some(d) => {
                                let (g, t) = cv.wait_timeout(woken, d).expect("wake lock");
                                woken = g;
                                if t.timed_out() {
                                    break;
                                }
                            }
                            None => woken = cv.wait(woken).expect("wake lock"),
                        }
                    }
                    *woken = false;
                }
                retry = if enabled(&app) { run_in_app(&app, &geocoder) } else { None };
            }
        })
        .expect("spawn places worker");
}

fn run_in_app(app: &AppHandle, geocoder: &dyn Geocoder) -> Option<Duration> {
    let state = app.state::<AppState>();
    let Ok(lib) = state.library() else { return None };
    let left = lib.db.with_conn(assets::count_needing_online_place).unwrap_or(0);
    if left == 0 {
        return None;
    }
    let ring = photos_jobs::activity::start("places-online", "Naming places exactly", left as u64);
    let (mut done, total) = (0usize, left);
    // Closing ends the process; the switch going off stops the run before its next request.
    let keep_going = || enabled(app);
    let outcome = run(&lib.db, geocoder, PACE, &keep_going, &mut |n| {
        done += n;
        ring.advance(n as u64);
        ring.set_detail(Some(&format!("{} of {total}", done.min(total as usize))));
        state.changes.changed_background(app);
    });
    drop(ring);
    if done > 0 {
        tracing::info!(n = done, ?outcome, "named places exactly");
        state.changes.changed(app, false);
    }
    match outcome {
        Outcome::RetryIn(d) => Some(d),
        Outcome::Done | Outcome::Stopped => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    struct Fake {
        answers: Vec<(f64, f64, Result<Address, GeoError>)>,
        asked: RefCell<Vec<(f64, f64)>>,
    }

    impl Geocoder for Fake {
        fn reverse(&self, lat: f64, lon: f64) -> Result<(Address, String), GeoError> {
            self.asked.borrow_mut().push((lat, lon));
            let (_, _, a) = self.answers.iter().find(|(la, lo, _)| (*la - lat).abs() < 1e-9 && (*lo - lon).abs() < 1e-9).expect("unexpected point");
            a.clone().map(|a| (a, "{}".into()))
        }
    }

    fn idrija() -> Address {
        Address { town: Some("Idrija".into()), municipality: Some("Idrija".into()), country: Some("Slovenia".into()), country_code: Some("si".into()), ..Default::default() }
    }

    fn library(points: &[(f64, f64)]) -> (photos_core::Db, Vec<i64>) {
        let db = photos_core::Db::open_in_memory().unwrap();
        let ids = db
            .with_conn(|c| {
                let rows: Vec<assets::NewAsset> = (0..points.len())
                    .map(|i| assets::NewAsset {
                        content_hash: format!("h{i}"),
                        relative_path: format!("h{i}.jpg"),
                        filename: format!("IMG_{i}.JPG"),
                        extension: "jpg".into(),
                        byte_size: 1,
                        imported_at: 1,
                        ..Default::default()
                    })
                    .collect();
                let ids = assets::insert_assets(c, &rows)?;
                for (id, (lat, lon)) in ids.iter().zip(points) {
                    c.execute(
                        "UPDATE assets SET gps_lat = ?2, gps_lon = ?3, place_name = 'Gorizia, Italy', locality = 'Gorizia, Italy' WHERE id = ?1",
                        rusqlite::params![id, lat, lon],
                    )?;
                }
                photos_core::fts::index_ids(c, &ids)?;
                Ok(ids)
            })
            .unwrap();
        (db, ids)
    }

    fn sources(db: &photos_core::Db) -> Vec<(i64, String)> {
        db.with_conn(|c| {
            let mut s = c.prepare("SELECT place_source, place_name FROM assets ORDER BY id")?;
            let rows = s.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        })
        .unwrap()
    }

    #[test]
    fn a_shared_position_is_asked_once_and_renamed_rows_are_searchable() {
        let (db, _) = library(&[(46.0006, 14.0264), (46.000_601, 14.026_401), (46.0006, 14.0264)]);
        let fake = Fake { answers: vec![(46.0006, 14.0264, Ok(idrija())), (46.000_601, 14.026_401, Ok(idrija()))], asked: RefCell::new(vec![]) };
        let mut batches = 0;
        let out = run(&db, &fake, Duration::ZERO, &|| true, &mut |_| batches += 1);
        assert_eq!(out, Outcome::Done);
        // The second point rounds to the same key (46.00060,14.02640): one request for all three.
        assert_eq!(fake.asked.borrow().len(), 1);
        assert_eq!(batches, 1);
        assert!(sources(&db).iter().all(|(s, p)| *s == 1 && p == "Idrija, Slovenia"));
        let hits = |q: &str| db.with_conn(|c| Ok(c.query_row("SELECT COUNT(*) FROM assets_fts WHERE assets_fts MATCH ?1", [q], |r| r.get::<_, i64>(0))?)).unwrap();
        assert_eq!((hits("Idrija"), hits("Gorizia")), (3, 0));

        // A later photo at the same position is named from the cache, without a request.
        let fresh = Fake { answers: vec![], asked: RefCell::new(vec![]) };
        db.with_conn(|c| Ok(c.execute("UPDATE assets SET place_source = 0 WHERE id = 1", [])?)).unwrap();
        assert_eq!(run(&db, &fresh, Duration::ZERO, &|| true, &mut |_| {}), Outcome::Done);
        assert!(fresh.asked.borrow().is_empty());
    }

    #[test]
    fn offline_stops_and_keeps_the_rows_for_later() {
        let (db, _) = library(&[(46.0006, 14.0264), (45.9557, 13.6379)]);
        let fake = Fake { answers: vec![(46.0006, 14.0264, Ok(idrija())), (45.9557, 13.6379, Err(GeoError::Offline))], asked: RefCell::new(vec![]) };
        assert_eq!(run(&db, &fake, Duration::ZERO, &|| true, &mut |_| {}), Outcome::RetryIn(OFFLINE_RETRY));
        assert_eq!(sources(&db), vec![(1, "Idrija, Slovenia".to_string()), (0, "Gorizia, Italy".to_string())]);
    }

    #[test]
    fn refusals_back_off_an_hour_and_the_switch_stops_the_run() {
        let (db, _) = library(&[(45.9557, 13.6379)]);
        let limited = Fake { answers: vec![(45.9557, 13.6379, Err(GeoError::Limited))], asked: RefCell::new(vec![]) };
        assert_eq!(run(&db, &limited, Duration::ZERO, &|| true, &mut |_| {}), Outcome::RetryIn(LIMITED_RETRY));
        let never = Fake { answers: vec![], asked: RefCell::new(vec![]) };
        assert_eq!(run(&db, &never, Duration::ZERO, &|| false, &mut |_| {}), Outcome::Stopped);
        assert!(never.asked.borrow().is_empty());
    }

    #[test]
    fn requests_are_paced() {
        let (db, _) = library(&[(46.0006, 14.0264), (45.9557, 13.6379)]);
        let fake = Fake { answers: vec![(46.0006, 14.0264, Ok(idrija())), (45.9557, 13.6379, Ok(idrija()))], asked: RefCell::new(vec![]) };
        let t = Instant::now();
        run(&db, &fake, Duration::from_millis(150), &|| true, &mut |_| {});
        assert!(t.elapsed() >= Duration::from_millis(150));
    }

    /// Replay against the real Nominatim on a **copy** of a library's database, sending only
    /// the coordinates of photos around Idrija and Nova Gorica (every other row is parked on
    /// the copy so it is never sent), at most `LUMEN_PLACES_LIMIT` requests (default 20):
    ///
    /// ```text
    /// LUMEN_DB=<copy of library.db> LUMEN_PLACES_LIMIT=20 cargo test -p lumen --lib places_online_replay -- --ignored --nocapture
    /// ```
    #[test]
    #[ignore = "network; needs LUMEN_DB=<a copy of library.db>"]
    fn places_online_replay_on_a_copy() {
        let path = std::path::PathBuf::from(std::env::var("LUMEN_DB").expect("LUMEN_DB=<copy of library.db>"));
        let lower = path.to_string_lossy().to_lowercase().replace('\\', "/");
        assert!(!lower.contains("lumen library") && !lower.contains("/pictures/"), "LUMEN_DB must be a copy outside the library: {}", path.display());
        let limit: usize = std::env::var("LUMEN_PLACES_LIMIT").ok().and_then(|v| v.parse().ok()).unwrap_or(20);
        let db = photos_core::Db::open(&path).unwrap();
        // Idrija's municipality and Nova Gorica (with the border at Trg Evrope).
        const AREAS: &str = "((gps_lat BETWEEN 45.93 AND 46.10 AND gps_lon BETWEEN 13.90 AND 14.15)
                            OR (gps_lat BETWEEN 45.93 AND 45.98 AND gps_lon BETWEEN 13.62 AND 13.68))";
        db.with_conn(|c| {
            c.execute(&format!("UPDATE assets SET place_source = 2 WHERE place_source = 0 AND gps_lat IS NOT NULL AND NOT {AREAS}"), [])?;
            Ok(())
        })
        .unwrap();
        type Row = (i64, f64, f64, Option<String>, Option<String>);
        let before: Vec<Row> = db
            .with_conn(|c| {
                let mut s = c.prepare(&format!("SELECT id, gps_lat, gps_lon, place_name, locality FROM assets WHERE place_source = 0 AND state = 0 AND {AREAS} ORDER BY id"))?;
                let rows = s.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)))?.collect::<Result<Vec<_>, _>>()?;
                Ok(rows)
            })
            .unwrap();
        println!("{} photos around Idrija / Nova Gorica to name", before.len());
        let asked = std::cell::Cell::new(0usize);
        struct Counted<'a>(Nominatim, &'a std::cell::Cell<usize>);
        impl Geocoder for Counted<'_> {
            fn reverse(&self, lat: f64, lon: f64) -> Result<(Address, String), GeoError> {
                self.1.set(self.1.get() + 1);
                self.0.reverse(lat, lon)
            }
        }
        let geocoder = Counted(Nominatim::new(), &asked);
        let outcome = run(&db, &geocoder, PACE, &|| asked.get() < limit, &mut |n| println!("named {n}"));
        println!("{outcome:?} after {} requests", asked.get());
        for (id, lat, lon, place, locality) in &before {
            let now: (Option<String>, Option<String>, i64) = db
                .with_conn(|c| Ok(c.query_row("SELECT place_name, locality, place_source FROM assets WHERE id = ?1", [id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?))
                .unwrap();
            if now.2 == 1 {
                println!("{id} {lat:.5},{lon:.5}: {place:?} / {locality:?} → {:?} / {:?}", now.0, now.1);
                let names = format!("{:?} {:?}", now.0, now.1);
                if *lon > 13.9 {
                    assert!(names.contains("Slovenia") && !names.contains("Gorizia"), "Idrija's photo {id} is in Slovenia: {names}");
                }
            }
        }
    }
}
