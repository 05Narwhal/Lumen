//! Maintenance-lane housekeeping: purge the trash after 30 days.

use std::time::Duration;

use photos_core::assets;
use photos_core::now_unix;
use photos_media::DerivativeStore;
use tauri::{AppHandle, Manager};

use crate::state::AppState;

pub const TRASH_RETENTION_SECS: i64 = 30 * 24 * 3600;
const PERIOD: Duration = Duration::from_secs(6 * 3600);

/// Deletes originals and derivatives of assets trashed more than 30 days ago, then their rows.
pub fn purge_trash(app: &AppHandle) -> usize {
    let state = app.state::<AppState>();
    let Ok(lib) = state.library() else { return 0 };
    let store = DerivativeStore::new(&lib.root);
    let cutoff = now_unix() - TRASH_RETENTION_SECS;
    match assets::purge_expired(&lib, cutoff, &|hash| store.delete_all(hash)) {
        Ok(0) => 0,
        Ok(n) => {
            tracing::info!(n, "purged expired trash");
            state.changes.changed(app, true);
            n
        }
        Err(e) => {
            tracing::warn!("trash purge failed: {e}");
            0
        }
    }
}

/// How many photos are re-read per pass. Each is an EXIF read plus, at worst, a 1 MB scan
/// for the XMP packet: fast, but it runs beside imports, so it goes in batches.
const DATE_BATCH: usize = 200;

/// What a re-read should store: the date, its UTC instant, the offset and the source. A
/// file with no date of its own keeps the file's modified time, marked `FileTime`, so the
/// pass never looks at it again.
pub fn better_date(
    current: Option<i64>,
    found: &photos_media::Metadata,
    file_modified: Option<i64>,
) -> Option<(i64, Option<i64>, Option<i64>, photos_media::CaptureSource)> {
    match (found.captured_at, found.capture_source) {
        (Some(local), Some(source)) => {
            let utc = found.tz_offset_minutes.map(|off| local - off as i64 * 60);
            Some((local, utc, found.tz_offset_minutes.map(i64::from), source))
        }
        _ => {
            let local = file_modified.or(current)?;
            Some((local, None, None, photos_media::CaptureSource::FileTime))
        }
    }
}

/// Re-read the dates of photos catalogued before Lumen read every date tag, and correct the
/// ones that were wrong. Returns `(read, moved)`: call until `read` is 0, because a batch
/// that corrects nothing still has to be followed by the next batch.
///
/// Originals are only ever read. The grid orders by `captured_at`, so the layout cache is
/// dropped whenever anything moves and the timeline re-sorts into true chronological order.
pub fn recheck_capture_dates(app: &AppHandle) -> (usize, usize) {
    let state = app.state::<AppState>();
    let Ok(lib) = state.library() else { return (0, 0) };
    let (read, moved) = recheck_dates_in(&lib, state.ffmpeg().as_ref(), DATE_BATCH);
    if moved > 0 {
        tracing::info!(moved, of = read, "corrected capture dates");
        state.layout.lock().expect("layout lock").invalidate();
        state.changes.changed(app, true);
    }
    (read, moved)
}

/// One batch, against a library rather than the running app, so the pass can be replayed
/// over a copy of a real catalogue in a test.
pub fn recheck_dates_in(lib: &photos_core::Library, ffmpeg: Option<&photos_media::Ffmpeg>, batch: usize) -> (usize, usize) {
    let Ok(todo) = lib.db.with_conn(|c| assets::list_unsourced_dates(c, batch)) else { return (0, 0) };
    let originals = lib.originals_dir();
    let mut moved = 0;
    for (id, relative_path, file_modified) in &todo {
        let path = originals.join(relative_path);
        let meta = photos_media::read_metadata_with(&path, photos_media::kind_for_path(&path), ffmpeg);
        let current = lib.db.with_conn(|c| assets::captured_at_of(c, *id)).ok().flatten();
        let Some((local, utc, tz, source)) = better_date(current, &meta, *file_modified) else { continue };
        if lib.db.with_conn(|c| assets::set_capture_time(c, *id, local, utc, tz, source.as_str())).is_err() {
            continue;
        }
        if current != Some(local) {
            moved += 1;
        }
    }
    (todo.len(), moved)
}

/// How many photos have their metadata read again per pass (Round 7): an EXIF read and,
/// when it names no software, a 1 MB scan for XMP.
const METADATA_BATCH: usize = 200;

/// What a newer reading adds to an older row (Round 7): the software and the exposure
/// details, and the version, so a photo is read once. Nothing else in the row changes
/// (`merge_metadata` only sets the keys named here; the originals are opened read-only).
pub fn metadata_patch(m: &photos_media::Metadata, probe: Option<&photos_media::Probe>) -> serde_json::Value {
    let mut patch = serde_json::Map::new();
    // Round 9A.
    for (key, value) in [("lens_make", &m.lens_make), ("color_space", &m.color_space)] {
        if let Some(v) = value {
            patch.insert(key.into(), v.clone().into());
        }
    }
    if let Some(at) = m.digitized_at {
        patch.insert("digitized_at".into(), at.into());
    }
    // Round 19: the fraction of the second and the compass direction.
    if let Some(ms) = m.capture_subsec_ms {
        patch.insert("capture_subsec_ms".into(), ms.into());
    }
    if let Some(d) = m.gps_direction {
        patch.insert("gps_direction".into(), d.into());
    }
    // A video probed again: only what the stored probe lacks, merged into it.
    if probe.is_some_and(|p| p.dolby_vision) {
        patch.insert("probe".into(), serde_json::json!({ "dolby_vision": true }));
    }
    for (key, value) in [
        ("software", &m.software),
        ("white_balance", &m.white_balance),
        ("flash", &m.flash),
        ("metering", &m.metering),
        ("exposure_program", &m.exposure_program),
        ("exposure_bias", &m.exposure_bias),
    ] {
        if let Some(v) = value {
            patch.insert(key.into(), v.clone().into());
        }
    }
    patch.insert("exif".into(), photos_media::METADATA_VERSION.into());
    serde_json::Value::Object(patch)
}

/// What one batch of [`backfill_metadata_in`] did.
#[derive(Debug, Default, Clone, Copy)]
pub struct MetadataBatch {
    /// Photos read; call until 0.
    pub read: usize,
    /// Of those, how many got a position they lacked (version 4's video locations).
    pub located: usize,
}

/// One batch of photos read before the current reading: their originals are opened read-only
/// and what they add is merged into `metadata_json`.
/// Against a database and an originals folder, so it can be replayed over a copy of a real
/// catalogue while the originals are only read.
/// With `ffmpeg`, videos are probed again too (Round 9A: Dolby Vision).
pub fn backfill_metadata_in(db: &photos_core::Db, originals: &std::path::Path, ffmpeg: Option<&photos_media::Ffmpeg>, batch: usize) -> MetadataBatch {
    let Ok(todo) = db.with_conn(|c| assets::list_metadata_before(c, photos_media::METADATA_VERSION, batch)) else { return MetadataBatch::default() };
    let located = todo.iter().filter(|(id, relative_path)| reread_metadata(db, *id, &originals.join(relative_path), ffmpeg)).count();
    MetadataBatch { read: todo.len(), located }
}

/// Reads one original's metadata again and merges what it adds (never overwrites); `true` when
/// it gave the photo a position it lacked. Also used after Import → Repair swaps an original.
pub fn reread_metadata(db: &photos_core::Db, id: i64, path: &std::path::Path, ffmpeg: Option<&photos_media::Ffmpeg>) -> bool {
    // A missing or unreadable file still gets the version, so it is not read again.
    let kind = photos_media::kind_for_path(path);
    let meta = photos_media::read_metadata(path, kind);
    let probe = ffmpeg.filter(|_| kind == photos_media::MediaKind::Video).and_then(|f| f.probe(path).ok());
    // Version 4: a video's QuickTime location (iPhone), never read before, and any photo
    // whose position was missed. Only rows with no position get one.
    let gps = match (meta.gps_lat, meta.gps_lon) {
        (Some(lat), Some(lon)) => Some((lat, lon, meta.gps_altitude)),
        _ => probe.as_ref().and_then(|p| Some((p.gps_lat?, p.gps_lon?, p.gps_altitude))),
    };
    let mut located = false;
    if let Some((lat, lon, alt)) = gps {
        match db.with_conn(|c| assets::fill_missing_gps(c, id, lat, lon, alt)) {
            Ok(true) => located = true,
            Ok(false) => {}
            Err(e) => tracing::warn!(id, "gps backfill: {e}"),
        }
    }
    if let Err(e) = db.with_conn(|c| assets::merge_metadata(c, id, &metadata_patch(&meta, probe.as_ref()))) {
        tracing::warn!(id, "metadata backfill: {e}");
    }
    located
}

/// Reads, once, what older imports never kept (*Exported from* and the exposure details) so
/// search can offer them (Round 7). Shown on the activity ring while it runs (Julian's
/// 2026-11-08 ask: version 4 re-reads every original for video locations), and each batch that
/// found positions is announced, so the map and info panel fill in as it goes.
pub fn backfill_metadata(app: &AppHandle, cancel: &photos_jobs::CancelToken) {
    let state = app.state::<AppState>();
    let Ok(lib) = state.library() else { return };
    let originals = lib.originals_dir();
    let left = lib.db.with_conn(|c| assets::count_metadata_before(c, photos_media::METADATA_VERSION)).unwrap_or(0);
    if left == 0 {
        return;
    }
    let ring = photos_jobs::activity::start("metadata", "Reading locations and photo details", left);
    let (mut total, mut located) = (0, 0);
    loop {
        let n = backfill_metadata_in(&lib.db, &originals, state.ffmpeg().as_ref(), METADATA_BATCH);
        total += n.read;
        located += n.located;
        ring.advance(n.read as u64);
        if n.located > 0 {
            state.changes.changed_background(app);
        }
        if n.read == 0 || cancel.is_cancelled() {
            break;
        }
    }
    drop(ring);
    // The startup place naming runs beside this job, so positions found after it finished
    // stayed unnamed until the next launch: unsearchable, and the info panel's live fallback
    // named the nearest big town (Julian's Idrija videos as Gorizia). Name them now.
    if located > 0 {
        while backfill_places(app) > 0 {}
        backfill_localities(app);
        crate::places_online::wake();
    }
    if total > 0 {
        tracing::info!(total, "read the software and exposure details of older photos");
        state.changes.changed(app, true);
    }
}

/// Assets imported as `.mts` / `.m2ts` video whose original is plain text (Round 8A):
/// TypeScript modules from a `node_modules` folder, imported before import looked inside such
/// files. Both tests must hold: not an MPEG transport stream, and UTF-8 text with no NUL byte,
/// so a real video with a damaged start is never taken for one. Originals are only read.
pub fn text_posing_as_video(db: &photos_core::Db, originals: &std::path::Path) -> Vec<i64> {
    let Ok(rows) = db.with_conn(|c| {
        let mut stmt = c.prepare("SELECT id, relative_path FROM assets WHERE state = 0 AND extension IN ('mts', 'm2ts')")?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))?;
        Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
    }) else {
        return Vec::new();
    };
    rows.into_iter()
        .filter(|(_, rel)| {
            let mut head = [0u8; 600];
            let Ok(n) = std::fs::File::open(originals.join(rel)).and_then(|mut f| std::io::Read::read(&mut f, &mut head)) else { return false };
            let head = &head[..n];
            n > 0 && !photos_media::is_transport_stream(head) && !head.contains(&0) && std::str::from_utf8(head).is_ok_or_cut()
        })
        .map(|(id, _)| id)
        .collect()
}

/// UTF-8 up to a character the 600-byte read may have cut in two.
trait Utf8Prefix {
    fn is_ok_or_cut(&self) -> bool;
}

impl Utf8Prefix for Result<&str, std::str::Utf8Error> {
    fn is_ok_or_cut(&self) -> bool {
        match self {
            Ok(_) => true,
            Err(e) => e.error_len().is_none(),
        }
    }
}

/// Moves [`text_posing_as_video`] to the Trash (30 days, restorable), once each: a trashed
/// asset is no longer looked at.
fn trash_text_posing_as_video(app: &AppHandle) {
    let state = app.state::<AppState>();
    let Ok(lib) = state.library() else { return };
    let ids = text_posing_as_video(&lib.db, &lib.originals_dir());
    if ids.is_empty() {
        return;
    }
    match lib.db.with_conn(|c| assets::trash_assets(c, &ids, photos_core::now_unix())) {
        Ok(n) => {
            tracing::info!(moved = n, "moved text files imported as video to the Trash");
            state.changes.changed(app, true);
        }
        Err(e) => tracing::warn!("text files imported as video: {e}"),
    }
}

/// Photos whose decoder already turns them upright (HEIC and video, through FFmpeg) but
/// whose stored orientation still turns them: before round 3 a tag that does not swap axes
/// (3, upside down) was applied on top of FFmpeg's own turn. Resets them to upright and
/// queues them to be derived again. Returns how many were reset; idempotent.
pub fn refix_orientation_in(db: &photos_core::Db) -> usize {
    let Ok(rows) = db.with_conn(assets::oriented_rows) else { return 0 };
    let ids: Vec<i64> = rows
        .into_iter()
        .filter(|(_, ext)| photos_media::decoder_orients(photos_media::kind_for_extension(ext)))
        .map(|(id, _)| id)
        .collect();
    db.with_conn(|c| assets::reset_orientation(c, &ids)).unwrap_or(0)
}

fn refix_orientation(app: &AppHandle) -> usize {
    let state = app.state::<AppState>();
    let Ok(lib) = state.library() else { return 0 };
    let n = refix_orientation_in(&lib.db);
    if n > 0 {
        tracing::info!(n, "reset photos that had been turned twice");
        state.layout.lock().expect("layout lock").invalidate();
        state.changes.changed(app, true);
    }
    n
}

/// How many photos get a place name per pass. Each lookup is microseconds, but the pass
/// also writes and re-indexes, so it runs in batches and picks up where it left off.
const PLACE_BATCH: usize = 2000;

/// Name the places of geotagged photos that have not been looked up yet, using the
/// bundled atlas. Returns how many were filled in.
pub fn backfill_places(app: &AppHandle) -> usize {
    let state = app.state::<AppState>();
    let Ok(lib) = state.library() else { return 0 };
    let Ok(todo) = lib.db.with_conn(|c| assets::list_needing_place(c, PLACE_BATCH)) else { return 0 };
    if todo.is_empty() {
        return 0;
    }
    let places: Vec<(i64, String)> = todo
        .into_iter()
        .map(|(id, lat, lon)| {
            // An empty string means "looked up, nothing to say", so the next pass skips it.
            let label = photos_core::places::reverse_geocode(lat, lon).and_then(|p| p.label()).unwrap_or_default();
            (id, label)
        })
        .collect();
    match lib.db.with_conn(|c| assets::set_places(c, &places)) {
        Ok(n) => {
            tracing::info!(n, "named places");
            state.changes.changed(app, false);
            n
        }
        Err(e) => {
            tracing::warn!("place backfill failed: {e}");
            0
        }
    }
}

/// Name the village or district of every geotagged photo not looked up yet (Round 4K), so
/// every place in the library can be searched for. The gazetteer (~20 MB loaded) is read only
/// when there is work and dropped when it is done. Returns how many were filled in.
pub fn backfill_localities(app: &AppHandle) -> usize {
    let state = app.state::<AppState>();
    let Ok(lib) = state.library() else { return 0 };
    let mut gazetteer: Option<photos_core::places::Localities> = None;
    let mut done = 0;
    loop {
        let Ok(todo) = lib.db.with_conn(|c| assets::list_needing_locality(c, PLACE_BATCH)) else { break };
        if todo.is_empty() {
            break;
        }
        let g = gazetteer.get_or_insert_with(photos_core::places::Localities::load);
        // '' means "looked up, nothing near", so the next pass skips it.
        let named: Vec<(i64, String)> = todo.into_iter().map(|(id, lat, lon)| (id, g.nearest(lat, lon).unwrap_or_default())).collect();
        match lib.db.with_conn(|c| assets::set_localities(c, &named)) {
            Ok(n) => done += n,
            Err(e) => {
                tracing::warn!("locality backfill failed: {e}");
                break;
            }
        }
    }
    if done > 0 {
        tracing::info!(n = done, "named localities");
        state.changes.changed(app, false);
    }
    done
}

/// Refresh the query planner's statistics so filter-heavy searches keep using the
/// covering index. Runs with the trash purge; a no-op on a library that has not changed.
pub fn analyze(app: &AppHandle) {
    let state = app.state::<AppState>();
    let Ok(lib) = state.library() else { return };
    if let Err(e) = lib.db.with_conn(photos_core::db::analyze) {
        tracing::warn!("analyze failed: {e}");
    }
}

/// The calendar day at `now` for a UTC offset of `offset_secs`: what "on this day" means.
pub fn local_today(now: i64, offset_secs: i32) -> chrono::NaiveDate {
    chrono::DateTime::from_timestamp(now + offset_secs as i64, 0).unwrap_or_default().date_naive()
}

/// Fill in curation facts, regenerate memories and tell the frontend. Coalesced on the
/// Maintenance lane, so a burst of imports queues one run.
pub fn schedule_memories(app: &AppHandle) {
    let a = app.clone();
    app.state::<AppState>().jobs.submit("maintenance:memories", photos_jobs::Lane::Maintenance, move |cancel| {
        while crate::derivatives::backfill_quality(&a) > 0 {
            if cancel.is_cancelled() {
                return;
            }
        }
        let state = a.state::<AppState>();
        let Ok(lib) = state.library() else { return };
        let now = photos_core::now_unix();
        let today = local_today(now, photos_core::layout::local_offset_secs());
        let t = std::time::Instant::now();
        // The reference phrases curation compares photographs against (junk, good).
        let protos = photos_ml::prototypes();
        match photos_core::memories::regenerate_on(&lib.db, today, now, &protos) {
            Ok(n) => {
                tracing::info!(memories = n, ms = t.elapsed().as_millis() as u64, "memories regenerated");
                state.changes.changed(&a, false);
                // A new memory gets its film made ahead of Play.
                crate::memory_prep::schedule(&a, Duration::from_secs(5));
            }
            Err(e) => tracing::warn!("memories: {e}"),
        }
    });
}

/// How long after launch background work waits. None of it is urgent, and all of it
/// competes with the first screen for the database, the disk and the CPU: at launch it once
/// held the grid's first queries back for minutes.
const STARTUP_GRACE: Duration = Duration::from_secs(20);

/// Starts the background work once the first screen has had the machine to itself.
pub fn start(app: AppHandle) {
    crate::memory_prep::start();
    std::thread::Builder::new()
        .name("lumen-maintenance-start".into())
        .spawn(move || {
            std::thread::sleep(STARTUP_GRACE);
            start_now(app);
        })
        .expect("spawn maintenance start");
}

fn start_now(app: AppHandle) {
    // Music and cuts ready before Play (Round 21 §14).
    crate::memory_prep::schedule(&app, Duration::ZERO);
    // Exact place names (plan 2026-11-08): its thread sleeps until woken or its retry tick.
    crate::places_online::start(app.clone());
    // Round 22 lane C: what the person's editors saved since Lumen last ran.
    crate::external_edits::schedule_sync(&app);
    {
        // Assets imported before FFmpeg appeared can get their derivatives now.
        let a = app.clone();
        app.state::<AppState>().jobs.submit("maintenance:requeue-underived", photos_jobs::Lane::Maintenance, move |_| {
            // Photos turned twice are reset first, so the re-derive below redraws them.
            let refixed = refix_orientation(&a);
            // Thumbnails drawn sideways before an orientation fix are redrawn too.
            let reshaped = crate::derivatives::refix_shapes(&a);
            // Files no reader can open leave the grids (Round 18 C1), before anything is queued.
            if let Ok(lib) = a.state::<AppState>().library() {
                if crate::derivatives::flag_unreadable_from_marks(&lib) > 0 {
                    a.state::<AppState>().changes.changed_background(&a);
                }
            }
            crate::derivatives::requeue_underived(&a);
            if refixed + reshaped > 0 {
                // Their embeddings and face scans were dropped with the wrong pixels.
                crate::embeddings::schedule(&a);
            }
        });
    }
    {
        // Dates first: memories cluster by capture time, so they are regenerated from the
        // corrected dates rather than the ones this pass is about to replace.
        let a = app.clone();
        app.state::<AppState>().jobs.submit("maintenance:capture-dates", photos_jobs::Lane::Maintenance, move |cancel| {
            let mut moved = 0;
            loop {
                let (read, corrected) = recheck_capture_dates(&a);
                moved += corrected;
                if read == 0 || cancel.is_cancelled() {
                    break;
                }
            }
            if moved > 0 {
                schedule_memories(&a);
            }
        });
    }
    // How long each model × tier takes here (Round 14), once start-up has settled.
    crate::model_bench::schedule(&app, false, std::time::Duration::from_secs(20));
    std::thread::Builder::new()
        .name("lumen-maintenance".into())
        .spawn(move || loop {
            {
                let a = app.clone();
                app.state::<AppState>().jobs.submit("maintenance:analyze", photos_jobs::Lane::Maintenance, move |_| {
                    analyze(&a);
                });
            }
            {
                let a = app.clone();
                app.state::<AppState>().jobs.submit("maintenance:purge-trash", photos_jobs::Lane::Maintenance, move |_| {
                    purge_trash(&a);
                });
            }
            {
                let a = app.clone();
                app.state::<AppState>().jobs.submit("maintenance:text-as-video", photos_jobs::Lane::Maintenance, move |_| {
                    trash_text_posing_as_video(&a);
                });
            }
            // The newest videos' proxies, once the WebView has said what it plays (Round 8A);
            // until it has, `set_video_support` does this.
            if app.state::<AppState>().video_support.read().expect("video support lock").is_some() {
                crate::derivatives::prefetch_newest_proxies(&app);
                // And every other video that needs one (Round 9A): what an import, a full disk
                // or a library switch left.
                crate::video_prep::schedule(&app);
            }
            {
                let a = app.clone();
                app.state::<AppState>().jobs.submit("maintenance:metadata", photos_jobs::Lane::Maintenance, move |cancel| {
                    backfill_metadata(&a, cancel);
                });
            }
            {
                let a = app.clone();
                app.state::<AppState>().jobs.submit("maintenance:places", photos_jobs::Lane::Maintenance, move |_| {
                    // Keep going while there is work, so a freshly imported library is
                    // named in minutes rather than over days of six-hour ticks.
                    while backfill_places(&a) > 0 {}
                    backfill_localities(&a);
                    // Then the exact names, on their own thread (off unless switched on).
                    crate::places_online::wake();
                });
            }
            // Models that failed to arrive are tried again; photos imported since are embedded
            // (which regenerates memories when it stores any).
            crate::models::fetch_all(&app);
            crate::embeddings::schedule(&app);
            crate::video_highlights::schedule(&app);
            // After the places, so a trip is named where it was.
            schedule_memories(&app);
            std::thread::sleep(PERIOD);
        })
        .expect("spawn maintenance thread");
}

#[cfg(test)]
mod tests {
    /// Round 7's metadata pass over a copy of a real catalogue: the originals are only read.
    /// `LUMEN_DB=<copy of library.db> LUMEN_ORIGINALS=<library>/originals cargo test --release
    /// -p lumen --lib metadata_backfill_on_a_copy -- --ignored --nocapture`
    #[test]
    #[ignore = "needs LUMEN_DB (a copy) and LUMEN_ORIGINALS"]
    fn metadata_backfill_on_a_copy() {
        let (Ok(db), Ok(originals)) = (std::env::var("LUMEN_DB"), std::env::var("LUMEN_ORIGINALS")) else { return };
        let db = photos_core::Db::open(std::path::Path::new(&db)).unwrap();
        let before: i64 = db.with_conn(|c| Ok(c.query_row("SELECT COUNT(*) FROM assets", [], |r| r.get(0))?)).unwrap();
        let started = std::time::Instant::now();
        let mut read = 0;
        loop {
            let n = super::backfill_metadata_in(&db, std::path::Path::new(&originals), photos_media::Ffmpeg::locate().as_ref(), 500).read;
            read += n;
            if n == 0 {
                break;
            }
        }
        let secs = started.elapsed().as_secs_f64();
        println!("read {read} photos in {secs:.1} s ({:.0}/s)", read as f64 / secs.max(0.001));
        db.with_conn(|c| {
            let after: i64 = c.query_row("SELECT COUNT(*) FROM assets", [], |r| r.get(0))?;
            assert_eq!(before, after);
            let left: i64 = c.query_row("SELECT COUNT(*) FROM assets WHERE COALESCE(json_extract(metadata_json, '$.exif'), 0) < 5", [], |r| r.get(0))?;
            assert_eq!(left, 0);
            // Round 19: what version 5 added.
            let (subsec, direction): (i64, i64) = c.query_row(
                "SELECT COUNT(json_extract(metadata_json, '$.capture_subsec_ms')), COUNT(json_extract(metadata_json, '$.gps_direction')) FROM assets",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )?;
            println!("sub-seconds on {subsec} photos, a direction on {direction}");
            let videos: (i64, i64) = c.query_row("SELECT COUNT(*), COUNT(gps_lat) FROM assets WHERE media_type = 1 AND state = 0", [], |r| Ok((r.get(0)?, r.get(1)?)))?;
            println!("videos: {} of {} have a position", videos.1, videos.0);
            let mut stmt = c.prepare("SELECT json_extract(metadata_json, '$.software') s, COUNT(*) n FROM assets GROUP BY s ORDER BY n DESC LIMIT 12")?;
            for row in stmt.query_map([], |r| Ok((r.get::<_, Option<String>>(0)?, r.get::<_, i64>(1)?)))? {
                println!("{:?}", row?);
            }
            let keeps: i64 = c.query_row("SELECT COUNT(*) FROM assets WHERE json_extract(metadata_json, '$.model') IS NOT NULL", [], |r| r.get(0))?;
            println!("rows still with a camera model: {keeps}");
            Ok(())
        })
        .unwrap();
    }

    use super::*;

    use photos_media::{CaptureSource, Metadata};

    /// Round 9A: the re-read carries the date digitized, the lens maker and the colour space,
    /// and a video's Dolby Vision, merged into what the row already holds.
    #[test]
    fn the_metadata_patch_carries_the_new_fields_and_keeps_the_rest() {
        let m = Metadata {
            digitized_at: Some(5),
            lens_make: Some("Sigma".into()),
            color_space: Some("Display P3".into()),
            capture_subsec_ms: Some(420),
            gps_direction: Some(270.5),
            ..Default::default()
        };
        let probe = photos_media::Probe { dolby_vision: true, ..Default::default() };
        let patch = metadata_patch(&m, Some(&probe));
        assert_eq!(patch["digitized_at"], 5);
        assert_eq!(patch["lens_make"], "Sigma");
        assert_eq!(patch["color_space"], "Display P3");
        // Round 19: sub-seconds and the direction the camera pointed.
        assert_eq!(patch["capture_subsec_ms"], 420);
        assert_eq!(patch["gps_direction"], 270.5);
        assert_eq!(patch["exif"], photos_media::METADATA_VERSION);
        let db = photos_core::Db::open_in_memory().unwrap();
        db.with_conn(|c| {
            c.execute(
                "INSERT INTO assets (id, content_hash, relative_path, filename, extension, media_type, byte_size, orientation, imported_at, metadata_json)
                 VALUES (1, 'h', 'p', 'a.mov', 'mov', 1, 1, 1, 1, '{\"model\":\"iPhone\",\"probe\":{\"video_codec\":\"hevc\"}}')",
                [],
            )?;
            assets::merge_metadata(c, 1, &patch)?;
            let json: String = c.query_row("SELECT metadata_json FROM assets WHERE id = 1", [], |r| r.get(0))?;
            let v: serde_json::Value = serde_json::from_str(&json).unwrap();
            assert_eq!((v["model"].as_str(), v["probe"]["video_codec"].as_str(), v["probe"]["dolby_vision"].as_bool()), (Some("iPhone"), Some("hevc"), Some(true)));
            Ok(())
        })
        .unwrap();
        assert!(metadata_patch(&m, Some(&photos_media::Probe::default())).get("probe").is_none(), "nothing to say: the probe is left alone");
    }

    /// Which assets of a real catalogue the cleanup would trash: a copy of the database, the
    /// originals only read. `LUMEN_DB=<copy> LUMEN_ORIGINALS=<library>/originals cargo test -p lumen --lib text_posing_as_video_on_a_copy -- --ignored --nocapture`
    #[test]
    #[ignore = "needs LUMEN_DB (a copy) and LUMEN_ORIGINALS"]
    fn text_posing_as_video_on_a_copy() {
        let (Ok(db), Ok(originals)) = (std::env::var("LUMEN_DB"), std::env::var("LUMEN_ORIGINALS")) else { return };
        let db = photos_core::Db::open(std::path::Path::new(&db)).unwrap();
        let ids = text_posing_as_video(&db, std::path::Path::new(&originals));
        let names: Vec<String> = db
            .with_conn(|c| {
                let mut out = Vec::new();
                for id in &ids {
                    out.push(c.query_row("SELECT filename FROM assets WHERE id = ?1", [id], |r| r.get(0))?);
                }
                Ok(out)
            })
            .unwrap();
        println!("{} would go to the Trash: {names:?}", ids.len());
    }

    #[test]
    fn only_text_files_imported_as_video_are_picked() {
        let dir = tempfile::tempdir().unwrap();
        let db = photos_core::Db::open_in_memory().unwrap();
        let mut avchd = vec![0u8; 800];
        for i in (4..800).step_by(192) {
            avchd[i] = 0x47;
        }
        let mut damaged = vec![0u8; 800];
        damaged[..4].copy_from_slice(b"\x00\x01zz");
        let files: [(i64, &str, Vec<u8>, &str, i64); 5] = [
            (1, "floating-ui.core.d.mts", b"export declare const x: number;\n".repeat(30), "mts", 0),
            (2, "00001.MTS", avchd, "mts", 0),
            (3, "broken.m2ts", damaged, "m2ts", 0),
            (4, "index-browser.mts", "export const café = 1;\n".repeat(30).into_bytes(), "mts", 1),
            (5, "notes.txt.mov", b"plain text".to_vec(), "mov", 0),
        ];
        db.with_conn(|c| {
            for (id, name, bytes, ext, state) in &files {
                std::fs::write(dir.path().join(name), bytes).unwrap();
                c.execute(
                    "INSERT INTO assets (id, content_hash, relative_path, filename, extension, media_type, byte_size, orientation, imported_at, state) VALUES (?1, ?2, ?3, ?3, ?4, 1, 1, 1, 1, ?5)",
                    rusqlite::params![id, format!("h{id}"), name, ext, state],
                )?;
            }
            Ok(())
        })
        .unwrap();
        assert_eq!(text_posing_as_video(&db, dir.path()), vec![1], "not a real or damaged video, nothing trashed already, no other kind");
    }

    fn found(at: i64, source: CaptureSource) -> Metadata {
        Metadata { captured_at: Some(at), tz_offset_minutes: Some(120), capture_source: Some(source), ..Default::default() }
    }

    #[test]
    fn a_real_capture_time_replaces_the_import_day_stamp() {
        let d = better_date(Some(1_789_000_000), &found(1_757_000_000, CaptureSource::Digitized), Some(1_789_000_000)).unwrap();
        assert_eq!(d.0, 1_757_000_000);
        assert_eq!(d.3, CaptureSource::Digitized);
        // Local wall time minus the offset is the UTC instant.
        assert_eq!(d.1, Some(1_757_000_000 - 120 * 60));
        assert_eq!(d.2, Some(120));
    }

    #[test]
    fn a_file_with_no_date_keeps_its_file_time_and_is_marked_so() {
        let d = better_date(Some(1_789_000_000), &Metadata::default(), Some(1_788_000_000)).unwrap();
        assert_eq!((d.0, d.3), (1_788_000_000, CaptureSource::FileTime));
    }

    #[test]
    fn a_file_with_neither_is_left_where_it_is() {
        let d = better_date(Some(1_789_000_000), &Metadata::default(), None).unwrap();
        assert_eq!((d.0, d.3), (1_789_000_000, CaptureSource::FileTime));
    }

    #[test]
    fn today_is_the_local_calendar_day() {
        // 2026-03-11 23:30 UTC is already the 12th in Ljubljana (UTC+1).
        let t = chrono::NaiveDate::from_ymd_opt(2026, 3, 11).unwrap().and_hms_opt(23, 30, 0).unwrap().and_utc().timestamp();
        assert_eq!(local_today(t, 3600), chrono::NaiveDate::from_ymd_opt(2026, 3, 12).unwrap());
        assert_eq!(local_today(t, -3600), chrono::NaiveDate::from_ymd_opt(2026, 3, 11).unwrap());
    }
}
