use std::path::{Path, PathBuf};
use std::collections::HashMap;
use std::panic::Location;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Condvar, Mutex};
use std::time::{Duration, Instant};

use rusqlite::{Connection, TransactionBehavior};

use crate::{CoreError, Result};

/// Forward-only migrations, applied in order inside one transaction each.
pub const MIGRATIONS: &[(u32, &str)] = &[
    (1, include_str!("../migrations/0001_initial.sql")),
    (2, include_str!("../migrations/0002_import_failures.sql")),
    (3, include_str!("../migrations/0003_devices_seen.sql")),
    (4, include_str!("../migrations/0004_search_indexes.sql")),
    (5, include_str!("../migrations/0005_filter_index.sql")),
    (6, include_str!("../migrations/0006_hdr.sql")),
    (7, include_str!("../migrations/0007_places_and_geo.sql")),
    (8, include_str!("../migrations/0008_orientation_repair.sql")),
    (9, include_str!("../migrations/0009_merge_split_devices.sql")),
    (10, include_str!("../migrations/0010_asset_text.sql")),
    (11, include_str!("../migrations/0011_edit_aspect.sql")),
    (12, include_str!("../migrations/0012_memories.sql")),
    (13, include_str!("../migrations/0013_capture_source.sql")),
    (14, include_str!("../migrations/0014_ml.sql")),
    (15, include_str!("../migrations/0015_people.sql")),
    (16, include_str!("../migrations/0016_develop.sql")),
    (17, include_str!("../migrations/0017_derived.sql")),
    (18, include_str!("../migrations/0018_develop_history.sql")),
    (19, include_str!("../migrations/0019_round3.sql")),
    (20, include_str!("../migrations/0020_people_round4.sql")),
    (21, include_str!("../migrations/0021_people_recall.sql")),
    (22, include_str!("../migrations/0022_duplicates.sql")),
    (23, include_str!("../migrations/0023_person_groups.sql")),
    (24, include_str!("../migrations/0024_localities.sql")),
    (25, include_str!("../migrations/0025_pets.sql")),
    (26, include_str!("../migrations/0026_album_collections.sql")),
    (27, include_str!("../migrations/0027_develop_drafts.sql")),
    (28, include_str!("../migrations/0028_track_profiles_video_highlights.sql")),
    (29, include_str!("../migrations/0029_covering_indexes.sql")),
    (30, include_str!("../migrations/0030_file_created_at.sql")),
    (31, include_str!("../migrations/0031_ocr.sql")),
    (32, include_str!("../migrations/0032_ai_masks.sql")),
    (33, include_str!("../migrations/0033_unreadable.sql")),
    (34, include_str!("../migrations/0034_cover_style.sql")),
    (35, include_str!("../migrations/0035_place_lookups.sql")),
    (36, include_str!("../migrations/0036_album_cover_picks.sql")),
    (37, include_str!("../migrations/0037_sidecars_skips.sql")),
    (38, include_str!("../migrations/0038_external_edits.sql")),
    (39, include_str!("../migrations/0039_recovered.sql")),
    (40, include_str!("../migrations/0040_place_writes.sql")),
    (41, include_str!("../migrations/0041_plugin_proposals.sql")),
];

/// Refresh the query planner's statistics. Without it SQLite guesses index selectivity
/// and can pick a narrow index that forces a table fetch per row; with it, filter-heavy
/// searches stay inside the covering index. Cheap enough to run on the maintenance lane.
pub fn analyze(conn: &Connection) -> Result<()> {
    conn.execute_batch("ANALYZE")?;
    Ok(())
}

/// A small pool of SQLite connections to one WAL database. Readers run alongside a writer
/// and writers queue on SQLite's own lock, so one long maintenance query no longer stalls
/// every other caller — with a single connection behind a mutex, a startup job could hold
/// the grid's first layout (and the main thread waiting for it) for as long as it ran.
///
/// Transactions start `IMMEDIATE`: a deferred one that reads and then writes fails outright
/// (`SQLITE_BUSY_SNAPSHOT`) when another connection wrote in between, where an immediate one
/// takes the write lock up front and waits its turn under the busy timeout.
///
/// A caller finding every connection busy takes the first one handed back (Round 5B). Before,
/// it waited on one chosen in turn, which could be the one memories or the duplicate finder
/// held for seconds while the others came free.
pub struct Db {
    free: Mutex<Vec<Connection>>,
    returned: Condvar,
    /// id → (since, caller) for every connection out of the pool: the watchdog's view of a
    /// call that never returns, which the slow-call warning only logs once it ends.
    leases: Mutex<HashMap<u64, (Instant, &'static Location<'static>)>>,
    next_lease: AtomicU64,
}

/// Connections to a database file. An in-memory database is private to its connection, so
/// it gets exactly one. Background lanes can hold several at once; the user's calls should
/// still find one free.
const POOL: usize = 8;
/// How long a writer waits for another connection's write to finish before giving up.
const BUSY_TIMEOUT: Duration = Duration::from_secs(30);
/// How much of the database file reads may map (address space, not memory: pages are the
/// OS's file cache, shared by every connection). Julian's 22k photos are 54 MB.
const MMAP_BYTES: i64 = 1 << 30;
/// A connection held — or waited for — longer than this is logged with the caller, to find
/// the slow query.
const SLOW_HOLD: Duration = Duration::from_millis(250);

impl Db {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(p) = path.parent() {
            std::fs::create_dir_all(p)?;
        }
        let first = Connection::open(path)?;
        first.pragma_update(None, "journal_mode", "WAL")?;
        let mut conns = vec![first];
        for _ in 1..POOL {
            conns.push(Connection::open(path)?);
        }
        Self::init(conns)
    }

    pub fn open_in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        Self::init(vec![conn])
    }

    fn init(conns: Vec<Connection>) -> Result<Self> {
        let mut pool = Vec::with_capacity(conns.len());
        for mut conn in conns {
            conn.pragma_update(None, "synchronous", "NORMAL")?;
            conn.pragma_update(None, "foreign_keys", "ON")?;
            // Round 7: reads come straight from the OS's cache of the file through a mapping
            // all eight connections share, instead of each copying pages into its own 2 MB
            // cache (a screen's first call on a cold connection was 3–6× its warm time), and
            // the sorts and DISTINCTs of the People and search queries stay in memory.
            conn.pragma_update(None, "mmap_size", MMAP_BYTES)?;
            conn.pragma_update(None, "temp_store", "MEMORY")?;
            conn.busy_timeout(BUSY_TIMEOUT)?;
            conn.set_transaction_behavior(TransactionBehavior::Immediate);
            pool.push(conn);
        }
        let db = Self { free: Mutex::new(pool), returned: Condvar::new(), leases: Mutex::new(HashMap::new()), next_lease: AtomicU64::new(0) };
        db.migrate()?;
        Ok(db)
    }

    fn migrate(&self) -> Result<()> {
        let mut free = self.free.lock().unwrap_or_else(|p| p.into_inner());
        apply_migrations(free.last_mut().expect("a connection"), MIGRATIONS)
    }

    pub fn schema_version(&self) -> Result<u32> {
        self.with_conn(|c| {
            Ok(c.query_row(
                "SELECT COALESCE(MAX(version), 0) FROM schema_version",
                [],
                |r| r.get(0),
            )?)
        })
    }

    /// Runs `f` on a free connection, or on the first one handed back when all are busy.
    #[track_caller]
    pub fn with_conn<T>(&self, f: impl FnOnce(&Connection) -> Result<T>) -> Result<T> {
        let caller = Location::caller();
        let asked = Instant::now();
        let conn = {
            let mut free = self.free.lock().unwrap_or_else(|p| p.into_inner());
            loop {
                if let Some(c) = free.pop() {
                    break c;
                }
                free = self.returned.wait(free).unwrap_or_else(|p| p.into_inner());
            }
        };
        let waited = asked.elapsed();
        if waited > SLOW_HOLD {
            tracing::warn!(ms = waited.as_millis() as u64, at = %caller, "waited for a database connection");
        }
        // Handed back even if `f` panics; an open transaction rolls back as it unwinds.
        let id = self.next_lease.fetch_add(1, Ordering::Relaxed);
        let held = Instant::now();
        self.leases.lock().unwrap_or_else(|p| p.into_inner()).insert(id, (held, caller));
        let lease = Lease { db: self, id, conn: Some(conn) };
        let out = f(lease.conn.as_ref().expect("leased"));
        let took = held.elapsed();
        if took > SLOW_HOLD {
            tracing::warn!(ms = took.as_millis() as u64, at = %caller, "slow database call");
        }
        out
    }

    /// The connection held longest right now: how long, and by which call.
    pub fn longest_lease(&self) -> Option<(u64, String)> {
        let leases = self.leases.lock().unwrap_or_else(|p| p.into_inner());
        leases.values().min_by_key(|(at, _)| *at).map(|(at, caller)| (at.elapsed().as_millis() as u64, caller.to_string()))
    }
}

/// A connection out of the pool, returned on drop.
struct Lease<'a> {
    db: &'a Db,
    id: u64,
    conn: Option<Connection>,
}

impl Drop for Lease<'_> {
    fn drop(&mut self) {
        self.db.leases.lock().unwrap_or_else(|p| p.into_inner()).remove(&self.id);
        if let Some(c) = self.conn.take() {
            self.db.free.lock().unwrap_or_else(|p| p.into_inner()).push(c);
            self.db.returned.notify_one();
        }
    }
}

/// How many copies [`safety_copy`] keeps per library.
const KEEP_SAFETY_COPIES: usize = 3;

/// A read-only connection: looking at a library must not change it. (On a WAL database closed
/// cleanly, SQLite may still leave empty `-wal` and `-shm` files beside it, which hold no data.)
fn read_only(path: &Path) -> Result<Connection> {
    Ok(Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX)?)
}

/// The migrations a database has, or `None` when it has no `schema_version` table.
fn applied(conn: &Connection) -> Result<Option<std::collections::HashSet<u32>>> {
    let exists: i64 = conn.query_row("SELECT count(*) FROM sqlite_master WHERE type = 'table' AND name = 'schema_version'", [], |r| r.get(0))?;
    if exists == 0 {
        return Ok(None);
    }
    let done = conn.prepare("SELECT version FROM schema_version")?.query_map([], |r| r.get(0))?.collect::<std::result::Result<_, _>>()?;
    Ok(Some(done))
}

/// The newest migration a database has had, without changing anything. `None` when it has never
/// been migrated.
pub fn peek_schema(path: &Path) -> Result<Option<u32>> {
    Ok(applied(&read_only(path)?)?.map(|done| done.into_iter().max().unwrap_or(0)))
}

/// Refuses a database newer than this build, which could not know what its tables mean.
fn refuse_newer(done: &std::collections::HashSet<u32>, migrations: &[(u32, &str)]) -> Result<()> {
    let app = migrations.iter().map(|(v, _)| *v).max().unwrap_or(0);
    match done.iter().copied().max() {
        Some(library) if library > app => Err(CoreError::NewerSchema { library, app }),
        _ => Ok(()),
    }
}

/// [`refuse_newer`] on a file, before anything opens it for writing (Round 17).
pub fn check_not_newer(path: &Path) -> Result<()> {
    match applied(&read_only(path)?)? {
        Some(done) => refuse_newer(&done, MIGRATIONS),
        None => Ok(()),
    }
}

/// A copy of an existing database in `dir`, made before migrations run on it (Round 17): `None`
/// when none are pending, or when it has never been migrated and so holds nothing to lose. It is a
/// `VACUUM INTO`, so one consistent copy of a live WAL database, put in place only once it opens
/// and agrees with the original; the newest [`KEEP_SAFETY_COPIES`] stay. Any failure is a
/// [`CoreError::SafetyCopy`], and the caller then must not migrate.
pub fn safety_copy(path: &Path, dir: &Path, now: i64) -> Result<Option<PathBuf>> {
    let fail = |e: &dyn std::fmt::Display| CoreError::SafetyCopy(e.to_string());
    let conn = read_only(path).map_err(|e| fail(&e))?;
    let Some(done) = applied(&conn).map_err(|e| fail(&e))? else { return Ok(None) };
    let Some(from) = done.iter().copied().max() else { return Ok(None) };
    if MIGRATIONS.iter().all(|(v, _)| done.contains(v)) {
        return Ok(None);
    }
    std::fs::create_dir_all(dir).map_err(|e| fail(&e))?;
    clear_safety_parts(dir);
    let part = dir.join(format!("{now}-v{from}.db.part"));
    let done_copy = dir.join(format!("{now}-v{from}.db"));
    let made = (|| -> Result<()> {
        conn.execute("VACUUM INTO ?1", [part.to_string_lossy().as_ref()])?;
        let copy = read_only(&part)?;
        let count = |c: &Connection| -> Result<i64> {
            let has: i64 = c.query_row("SELECT count(*) FROM sqlite_master WHERE type = 'table' AND name = 'assets'", [], |r| r.get(0))?;
            Ok(if has == 0 { 0 } else { c.query_row("SELECT count(*) FROM assets", [], |r| r.get(0))? })
        };
        let copied_from = applied(&copy)?.and_then(|d| d.into_iter().max());
        if copied_from != Some(from) || count(&copy)? != count(&conn)? {
            return Err(CoreError::Invalid(format!("the copy of {} does not match it", path.display())));
        }
        drop(copy);
        std::fs::rename(&part, &done_copy)?;
        Ok(())
    })();
    if let Err(e) = made {
        let _ = std::fs::remove_file(&part);
        return Err(fail(&e));
    }
    prune_safety_copies(dir, KEEP_SAFETY_COPIES);
    Ok(Some(done_copy))
}

/// One consistent copy of the database at `path` written to `to`, through a read-only connection
/// (`VACUUM INTO`), so the original is never changed. `to` must not exist yet.
pub fn snapshot_into(path: &Path, to: &Path) -> Result<()> {
    read_only(path)?.execute("VACUUM INTO ?1", [to.to_string_lossy().as_ref()])?;
    Ok(())
}

/// `(when, name)` when `name` is one of [`safety_copy`]'s, `<seconds>-v<schema>.db<tail>`.
fn safety_copy_name(name: &str, tail: &str) -> Option<i64> {
    let (when, rest) = name.strip_suffix(tail)?.strip_suffix(".db")?.split_once("-v")?;
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    if !digits(when) || !digits(rest) {
        return None;
    }
    when.parse().ok()
}

/// Removes what a crash left mid-copy. Only names [`safety_copy`] makes.
fn clear_safety_parts(dir: &Path) {
    for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        if safety_copy_name(&e.file_name().to_string_lossy(), ".part").is_some() {
            let _ = std::fs::remove_file(e.path());
        }
    }
}

/// Keeps the newest `keep` copies. Only names [`safety_copy`] makes are ever considered.
fn prune_safety_copies(dir: &Path, keep: usize) {
    let mut copies: Vec<(i64, PathBuf)> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| Some((safety_copy_name(&e.file_name().to_string_lossy(), "")?, e.path())))
        .collect();
    copies.sort_by_key(|(when, _)| std::cmp::Reverse(*when));
    for (_, old) in copies.into_iter().skip(keep) {
        if let Err(e) = std::fs::remove_file(&old) {
            tracing::warn!(file = %old.display(), "could not remove an old safety copy: {e}");
        }
    }
}

/// Applies every migration not yet recorded in `schema_version`, in order. Not only those
/// above the newest: branches take numbers ahead of one another (Round 5A's 0026 landed before
/// Phase C's 0025), and a gap must be filled when its migration arrives. A database with a
/// migration this build doesn't know is refused (Round 17).
pub(crate) fn apply_migrations(conn: &mut Connection, migrations: &[(u32, &str)]) -> Result<()> {
    conn.execute_batch("CREATE TABLE IF NOT EXISTS schema_version (version INTEGER NOT NULL, applied_at INTEGER NOT NULL);")?;
    let done: std::collections::HashSet<u32> =
        conn.prepare("SELECT version FROM schema_version")?.query_map([], |r| r.get(0))?.collect::<std::result::Result<_, _>>()?;
    refuse_newer(&done, migrations)?;
    for (version, sql) in migrations.iter().filter(|(v, _)| !done.contains(v)) {
        let tx = conn.transaction()?;
        tx.execute_batch(sql).map_err(|e| CoreError::Migration(format!("v{version}: {e}")))?;
        tx.execute("INSERT INTO schema_version(version, applied_at) VALUES (?1, unixepoch())", [version])?;
        tx.commit()?;
    }
    Ok(())
}

/// A database as a Lumen knowing only migrations up to `v` would leave it, for tests.
#[cfg(test)]
pub(crate) fn apply_up_to(conn: &mut Connection, v: u32) -> Result<()> {
    let before: Vec<(u32, &str)> = MIGRATIONS.iter().copied().filter(|(n, _)| *n <= v).collect();
    apply_migrations(conn, &before)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_longest_lease_names_its_caller() {
        let db = std::sync::Arc::new(Db::open_in_memory().unwrap());
        let (tx, rx) = std::sync::mpsc::channel();
        let (done_tx, done_rx) = std::sync::mpsc::channel::<()>();
        let d = db.clone();
        let t = std::thread::spawn(move || {
            d.with_conn(|_| {
                tx.send(()).unwrap();
                done_rx.recv().unwrap();
                Ok(())
            })
            .unwrap()
        });
        rx.recv().unwrap();
        std::thread::sleep(std::time::Duration::from_millis(60));
        let (ms, at) = db.longest_lease().expect("a lease");
        assert!(ms >= 50 && at.contains("db.rs"), "{ms} {at}");
        done_tx.send(()).unwrap();
        t.join().unwrap();
        assert!(db.longest_lease().is_none());
    }

    /// A library file stopped at migration `v`, as an older Lumen left it.
    fn database_at(path: &Path, v: u32) {
        let mut conn = Connection::open(path).unwrap();
        conn.pragma_update(None, "journal_mode", "WAL").unwrap();
        let before: Vec<(u32, &str)> = MIGRATIONS.iter().copied().filter(|(n, _)| *n <= v).collect();
        apply_migrations(&mut conn, &before).unwrap();
        conn.execute(
            "INSERT INTO assets (id, content_hash, relative_path, filename, extension, media_type, byte_size, orientation, imported_at) VALUES (1, 'a', 'a.jpg', 'a.jpg', 'jpg', 0, 1, 1, 1)",
            [],
        )
        .unwrap();
    }

    fn names_in(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        names.sort();
        names
    }

    #[test]
    fn apply_migrations_refuses_a_newer_database() {
        let mut conn = Connection::open_in_memory().unwrap();
        apply_migrations(&mut conn, &[(1, "CREATE TABLE a (x);"), (2, "CREATE TABLE b (x);")]).unwrap();
        let older: &[(u32, &str)] = &[(1, "CREATE TABLE a (x);")];
        let err = apply_migrations(&mut conn, older).unwrap_err();
        assert!(matches!(err, CoreError::NewerSchema { library: 2, app: 1 }), "{err:?}");
        assert_eq!(err.code(), "library_too_new");
    }

    #[test]
    fn db_open_refuses_a_newer_database() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("library.db");
        Db::open(&path).unwrap();
        Connection::open(&path).unwrap().execute("INSERT INTO schema_version(version, applied_at) VALUES (9999, 1)", []).unwrap();
        assert!(matches!(Db::open(&path), Err(CoreError::NewerSchema { library: 9999, .. })));
    }

    #[test]
    fn peek_schema_reads_without_a_table_or_a_write() {
        let tmp = tempfile::tempdir().unwrap();
        let empty = tmp.path().join("empty.db");
        Connection::open(&empty).unwrap().execute_batch("CREATE TABLE other (x);").unwrap();
        assert_eq!(peek_schema(&empty).unwrap(), None);
        let at30 = tmp.path().join("at30.db");
        database_at(&at30, 30);
        assert_eq!(peek_schema(&at30).unwrap(), Some(30));
    }

    #[test]
    fn a_pending_migration_makes_a_copy_first() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("library.db");
        database_at(&path, 30);
        let dir = tmp.path().join("backups").join("lib-1");
        let copy = safety_copy(&path, &dir, 1_800_000_000).unwrap().expect("a copy");
        assert_eq!(copy, dir.join("1800000000-v30.db"));
        assert_eq!(peek_schema(&copy).unwrap(), Some(30), "the copy is the library as it was");
        assert_eq!(peek_schema(&path).unwrap(), Some(30), "the library itself is not migrated by this");
        let assets: i64 = Connection::open(&copy).unwrap().query_row("SELECT count(*) FROM assets", [], |r| r.get(0)).unwrap();
        assert_eq!(assets, 1);
        assert_eq!(names_in(&dir), ["1800000000-v30.db"], "no part left behind");
    }

    #[test]
    fn no_copy_when_nothing_is_pending() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("library.db");
        Db::open(&path).unwrap();
        let dir = tmp.path().join("backups");
        assert_eq!(safety_copy(&path, &dir, 1).unwrap(), None);
        assert!(!dir.exists());
    }

    #[test]
    fn no_copy_for_a_database_with_nothing_in_it_yet() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("library.db");
        Connection::open(&path).unwrap().execute_batch("CREATE TABLE other (x);").unwrap();
        let dir = tmp.path().join("backups");
        assert_eq!(safety_copy(&path, &dir, 1).unwrap(), None);
        assert!(!dir.exists());
    }

    /// The copy comes from `VACUUM INTO`, so it holds what is still only in the WAL.
    #[test]
    fn the_copy_is_consistent_with_a_live_wal() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("library.db");
        database_at(&path, 30);
        let live = Connection::open(&path).unwrap();
        live.pragma_update(None, "wal_autocheckpoint", 0).unwrap();
        live.execute(
            "INSERT INTO assets (id, content_hash, relative_path, filename, extension, media_type, byte_size, orientation, imported_at) VALUES (2, 'b', 'b.jpg', 'b.jpg', 'jpg', 0, 1, 1, 1)",
            [],
        )
        .unwrap();
        let copy = safety_copy(&path, &tmp.path().join("backups"), 1).unwrap().unwrap();
        let assets: i64 = Connection::open(&copy).unwrap().query_row("SELECT count(*) FROM assets", [], |r| r.get(0)).unwrap();
        assert_eq!(assets, 2);
    }

    #[test]
    fn a_failed_safety_copy_says_so_and_leaves_no_part() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("library.db");
        database_at(&path, 30);
        // A file where the folder should go stands in for a full disk: the copy cannot be made.
        let blocked = tmp.path().join("backups");
        std::fs::write(&blocked, "in the way").unwrap();
        let err = safety_copy(&path, &blocked.join("lib-1"), 1).unwrap_err();
        assert!(matches!(err, CoreError::SafetyCopy(_)), "{err:?}");
        assert_eq!(err.code(), "safety_copy_failed");
        assert_eq!(peek_schema(&path).unwrap(), Some(30));
    }

    #[test]
    fn the_last_three_copies_are_kept() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("library.db");
        database_at(&path, 30);
        let dir = tmp.path().join("backups");
        for now in [100, 200, 300, 400, 500] {
            safety_copy(&path, &dir, now).unwrap();
        }
        assert_eq!(names_in(&dir), ["300-v30.db", "400-v30.db", "500-v30.db"]);
    }

    /// Ordered as numbers: `1000` is newer than `999`, though it sorts first as text.
    #[test]
    fn the_newest_copy_is_found_by_its_number() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("library.db");
        database_at(&path, 30);
        let dir = tmp.path().join("backups");
        for now in [997, 998, 999, 1000] {
            safety_copy(&path, &dir, now).unwrap();
        }
        assert_eq!(names_in(&dir), ["1000-v30.db", "998-v30.db", "999-v30.db"]);
    }

    #[test]
    fn pruning_touches_only_its_own_files() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("library.db");
        database_at(&path, 30);
        let dir = tmp.path().join("backups");
        std::fs::create_dir_all(&dir).unwrap();
        for other in ["tax.pdf", "library.db", "notes-v2.db", "1-v.db"] {
            std::fs::write(dir.join(other), "mine").unwrap();
        }
        for now in [100, 200, 300, 400] {
            safety_copy(&path, &dir, now).unwrap();
        }
        let names = names_in(&dir);
        for other in ["tax.pdf", "library.db", "notes-v2.db", "1-v.db"] {
            assert!(names.contains(&other.to_string()), "{other} was removed: {names:?}");
        }
        assert!(!names.contains(&"100-v30.db".to_string()));
    }

    #[test]
    fn a_part_left_by_a_crash_is_cleared_and_never_counted() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("library.db");
        database_at(&path, 30);
        let dir = tmp.path().join("backups");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("50-v30.db.part"), "half").unwrap();
        safety_copy(&path, &dir, 100).unwrap();
        assert_eq!(names_in(&dir), ["100-v30.db"]);
    }

    #[test]
    fn open_applies_all_migrations() {
        let db = Db::open_in_memory().unwrap();
        assert_eq!(db.schema_version().unwrap(), MIGRATIONS.last().unwrap().0);
        db.with_conn(|c| {
            let n: i64 = c.query_row(
                "SELECT count(*) FROM sqlite_master WHERE name IN ('assets','devices','albums','assets_fts','assets_geo')",
                [],
                |r| r.get(0),
            )?;
            assert_eq!(n, 5);
            Ok(())
        })
        .unwrap();
    }

    /// Round 5A took 0026 while 0025 (Phase C, pets) was still unwritten: a library migrated
    /// to 26 must still get 25 when it lands, not skip it for being below the newest.
    #[test]
    fn a_migration_numbered_below_the_newest_is_still_applied() {
        let mut conn = Connection::open_in_memory().unwrap();
        let early: &[(u32, &str)] = &[(1, "CREATE TABLE a (x);"), (3, "CREATE TABLE c (x);")];
        apply_migrations(&mut conn, early).unwrap();
        let late: &[(u32, &str)] = &[(1, "CREATE TABLE a (x);"), (2, "CREATE TABLE b (x);"), (3, "CREATE TABLE c (x);")];
        apply_migrations(&mut conn, late).unwrap();
        let tables: i64 = conn.query_row("SELECT count(*) FROM sqlite_master WHERE type = 'table' AND name IN ('a', 'b', 'c')", [], |r| r.get(0)).unwrap();
        assert_eq!(tables, 3);
        // And nothing twice.
        apply_migrations(&mut conn, late).unwrap();
        let rows: i64 = conn.query_row("SELECT count(*) FROM schema_version", [], |r| r.get(0)).unwrap();
        assert_eq!(rows, 3);
    }

    /// Round 6: a recipe with Develop settings moves, whole, to Develop's drafts, and the
    /// library shows the original; a viewer edit (crop, turn, trim) stays where it is.
    #[test]
    fn develop_edits_move_to_develop_drafts() {
        let mut conn = Connection::open_in_memory().unwrap();
        let before: Vec<(u32, &str)> = MIGRATIONS.iter().copied().filter(|(v, _)| *v <= 26).collect();
        apply_migrations(&mut conn, &before).unwrap();
        conn.execute_batch(
            r#"INSERT INTO assets (id, content_hash, relative_path, filename, extension, media_type, byte_size, orientation, imported_at)
                 VALUES (1, 'a', 'a.jpg', 'a.jpg', 'jpg', 0, 1, 1, 1), (2, 'b', 'b.jpg', 'b.jpg', 'jpg', 0, 1, 1, 1);
               INSERT INTO edits (asset_id, recipe_json, version, updated_at) VALUES
                 (1, '{"version":2,"rotate":90,"develop":{"basic":{"exposure":0.5}}}', 7, 100),
                 (2, '{"version":2,"rotate":90,"develop":null}', 8, 100);"#,
        )
        .unwrap();
        apply_migrations(&mut conn, MIGRATIONS).unwrap();
        let edits: Vec<i64> = conn.prepare("SELECT asset_id FROM edits").unwrap().query_map([], |r| r.get(0)).unwrap().map(|r| r.unwrap()).collect();
        assert_eq!(edits, vec![2], "the viewer's turn stays in the library");
        let (json, version): (String, i64) = conn.query_row("SELECT recipe_json, version FROM develop_drafts WHERE asset_id = 1", [], |r| Ok((r.get(0)?, r.get(1)?))).unwrap();
        assert!(json.contains("\"exposure\":0.5") && json.contains("\"rotate\":90"), "moved whole: {json}");
        assert_eq!(version, 7);
    }

    fn table_columns(conn: &Connection, table: &str) -> Vec<String> {
        conn.prepare(&format!("PRAGMA table_info({table})")).unwrap().query_map([], |r| r.get(1)).unwrap().map(|r| r.unwrap()).collect()
    }

    fn migration_31() -> &'static str {
        MIGRATIONS.iter().find(|(v, _)| *v == 31).expect("migration 31").1
    }

    /// Round 13: the OCR tables exist and the search index has its fifth column.
    #[test]
    fn migration_31_creates_ocr_tables() {
        let mut conn = Connection::open_in_memory().unwrap();
        apply_migrations(&mut conn, MIGRATIONS).unwrap();
        let n: i64 = conn
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE name IN ('ocr_scan', 'asset_text', 'idx_asset_text_asset')",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(n, 3);
        assert_eq!(table_columns(&conn, "assets_fts"), ["filename", "device_name", "place_name", "keywords", "photo_text"]);
    }

    /// Exact place names: the lookup cache exists and every existing row starts on the atlas.
    #[test]
    fn migration_35_creates_place_lookups() {
        let mut conn = Connection::open_in_memory().unwrap();
        apply_migrations(&mut conn, MIGRATIONS).unwrap();
        let n: i64 = conn
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE name IN ('place_lookups', 'idx_assets_place_source')",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(n, 2);
        assert!(table_columns(&conn, "assets").iter().any(|c| c == "place_source"));
        assert_eq!(
            table_columns(&conn, "place_lookups"),
            ["key", "place_name", "locality", "country_code", "address_json", "fetched_at"]
        );
    }

    /// Round 24 lane W: 0040 on a copy of a library file keeps its rows, adds the columns and
    /// tables, and applying again changes nothing.
    #[test]
    fn migration_0040_idempotent_on_copy() {
        let dir = tempfile::tempdir().unwrap();
        let original = dir.path().join("library.db");
        {
            let mut conn = Connection::open(&original).unwrap();
            let before: Vec<(u32, &str)> = MIGRATIONS.iter().copied().filter(|(v, _)| *v <= 39).collect();
            apply_migrations(&mut conn, &before).unwrap();
            conn.execute_batch(
                "INSERT INTO assets (id, content_hash, relative_path, filename, extension, media_type, byte_size, orientation, imported_at, place_source)
                   VALUES (1, 'a', 'a.jpg', 'a.jpg', 'jpg', 0, 1, 1, 1, 'atlas');",
            )
            .unwrap();
        }
        let copy = dir.path().join("copy.db");
        std::fs::copy(&original, &copy).unwrap();
        let mut conn = Connection::open(&copy).unwrap();
        apply_migrations(&mut conn, MIGRATIONS).unwrap();
        apply_migrations(&mut conn, MIGRATIONS).unwrap();
        let row: (String, Option<String>, Option<i64>) =
            conn.query_row("SELECT content_hash, file_hash, metadata_written_at FROM assets WHERE id = 1", [], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).unwrap();
        assert_eq!(row, ("a".into(), None, None));
        assert_eq!(table_columns(&conn, "place_writes"), ["id", "plugin_id", "created_at"]);
        assert_eq!(table_columns(&conn, "place_write_items"), ["write_id", "asset_id", "lat", "lon", "wrote_file", "prev_place_source"]);
        let version: u32 = conn.query_row("SELECT MAX(version) FROM schema_version", [], |r| r.get(0)).unwrap();
        assert_eq!(version, MIGRATIONS.last().unwrap().0);
        // The original file is untouched by migrating its copy.
        let orig = Connection::open(&original).unwrap();
        assert!(!table_columns(&orig, "assets").iter().any(|c| c == "file_hash"));
    }

    /// Round 25 P4b: 0041 on a copy of a library file keeps its rows, adds `plugin_proposals`,
    /// and applying again changes nothing.
    #[test]
    fn migration_0041_idempotent_on_copy() {
        let dir = tempfile::tempdir().unwrap();
        let original = dir.path().join("library.db");
        {
            let mut conn = Connection::open(&original).unwrap();
            let before: Vec<(u32, &str)> = MIGRATIONS.iter().copied().filter(|(v, _)| *v <= 40).collect();
            apply_migrations(&mut conn, &before).unwrap();
            conn.execute_batch(
                "INSERT INTO assets (id, content_hash, relative_path, filename, extension, media_type, byte_size, orientation, imported_at)
                   VALUES (1, 'a', 'a.jpg', 'a.jpg', 'jpg', 0, 1, 1, 1);",
            )
            .unwrap();
        }
        let copy = dir.path().join("copy.db");
        std::fs::copy(&original, &copy).unwrap();
        let mut conn = Connection::open(&copy).unwrap();
        apply_migrations(&mut conn, MIGRATIONS).unwrap();
        apply_migrations(&mut conn, MIGRATIONS).unwrap();
        let n: i64 = conn.query_row("SELECT count(*) FROM assets", [], |r| r.get(0)).unwrap();
        assert_eq!(n, 1);
        assert_eq!(
            table_columns(&conn, "plugin_proposals"),
            ["id", "plugin_id", "asset_id", "kind", "value", "confidence", "reason", "status", "created_at", "decided_at"]
        );
        let orig = Connection::open(&original).unwrap();
        assert!(table_columns(&orig, "plugin_proposals").is_empty());
    }

    /// Julian's 2026-11-09 answer: every cover set before distinct covers counts as automatic.
    #[test]
    fn migration_36_treats_existing_covers_as_automatic() {
        let mut conn = Connection::open_in_memory().unwrap();
        let before: Vec<(u32, &str)> = MIGRATIONS.iter().copied().filter(|(v, _)| *v <= 35).collect();
        apply_migrations(&mut conn, &before).unwrap();
        conn.execute_batch(
            "INSERT INTO assets (id, content_hash, relative_path, filename, extension, media_type, byte_size, orientation, imported_at)
               VALUES (1, 'a', 'a.jpg', 'a.jpg', 'jpg', 0, 1, 1, 1);
             INSERT INTO albums (id, name, kind, cover_asset_id, created_at, updated_at) VALUES (1, 'A', 0, 1, 1, 1);",
        )
        .unwrap();
        apply_migrations(&mut conn, MIGRATIONS).unwrap();
        let (picked, seed, key, cover): (i64, i64, Option<String>, Option<i64>) = conn
            .query_row("SELECT cover_picked, cover_seed, preview_key, cover_asset_id FROM albums WHERE id = 1", [], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
            })
            .unwrap();
        assert_eq!((picked, seed, key, cover), (0, 0, None, Some(1)));
    }

    /// Recreating `assets_fts` refills it from the catalogue: a person's name still finds
    /// their photo without a rebuild.
    #[test]
    fn migration_31_keeps_search_working() {
        let mut conn = Connection::open_in_memory().unwrap();
        let before: Vec<(u32, &str)> = MIGRATIONS.iter().copied().filter(|(v, _)| *v <= 30).collect();
        apply_migrations(&mut conn, &before).unwrap();
        conn.execute_batch(
            "INSERT INTO assets (id, content_hash, relative_path, filename, extension, media_type, byte_size, orientation, imported_at)
               VALUES (1, 'a', 'a.jpg', 'a.jpg', 'jpg', 0, 1, 1, 1), (2, 'b', 'b.jpg', 'b.jpg', 'jpg', 0, 1, 1, 1),
                      (3, 'c', 'c.jpg', 'c.jpg', 'jpg', 0, 1, 1, 1);
             INSERT INTO people (id, name, created_at, status) VALUES (1, 'Anna', 1, 'listed');
             INSERT INTO faces (asset_id, x, y, w, h, score, vec, model, person_id) VALUES (2, 0, 0, 1, 1, 1, x'00', 'm', 1);",
        )
        .unwrap();
        apply_migrations(&mut conn, MIGRATIONS).unwrap();
        let hits: Vec<i64> = conn
            .prepare("SELECT rowid FROM assets_fts WHERE assets_fts MATCH '\"anna\"*'")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .map(|r| r.unwrap())
            .collect();
        assert_eq!(hits, vec![2]);
        let rows: i64 = conn.query_row("SELECT count(*) FROM assets_fts", [], |r| r.get(0)).unwrap();
        assert_eq!(rows, 3);
    }

    #[test]
    fn migration_31_is_idempotent() {
        let mut conn = Connection::open_in_memory().unwrap();
        apply_migrations(&mut conn, MIGRATIONS).unwrap();
        conn.execute_batch(
            "INSERT INTO assets (id, content_hash, relative_path, filename, extension, media_type, byte_size, orientation, imported_at)
               VALUES (1, 'a', 'a.jpg', 'a.jpg', 'jpg', 0, 1, 1, 1);
             INSERT INTO ocr_scan (asset_id, model) VALUES (1, 'd+r');
             INSERT INTO asset_text (asset_id, ord, text, conf, quad, script) VALUES (1, 0, 'MENU', 0.9, x'00', 'Latn');",
        )
        .unwrap();
        crate::fts::index_ids(&conn, &[1]).unwrap();
        let counts = |c: &Connection| -> (i64, i64, i64) {
            c.query_row(
                "SELECT (SELECT count(*) FROM ocr_scan), (SELECT count(*) FROM asset_text), (SELECT count(*) FROM assets_fts)",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap()
        };
        let before = counts(&conn);
        conn.execute_batch(migration_31()).unwrap();
        assert_eq!(counts(&conn), before);
        assert_eq!(before, (1, 1, 1));
    }

    /// Times the 0031 refill at 250k assets (plan Task 5 Step 3). Run with
    /// `cargo test --release -p photos-core migration_31_refill_at_250k -- --ignored --nocapture`.
    #[test]
    #[ignore = "timing: 250k synthetic assets"]
    fn migration_31_refill_at_250k() {
        let dir = tempfile::tempdir().unwrap();
        let mut conn = Connection::open(dir.path().join("library.db")).unwrap();
        let before: Vec<(u32, &str)> = MIGRATIONS.iter().copied().filter(|(v, _)| *v <= 30).collect();
        apply_migrations(&mut conn, &before).unwrap();
        crate::synthetic::generate(&conn, 250_000, 7).unwrap();
        let t = Instant::now();
        apply_migrations(&mut conn, MIGRATIONS).unwrap();
        let took = t.elapsed();
        let rows: i64 = conn.query_row("SELECT count(*) FROM assets_fts", [], |r| r.get(0)).unwrap();
        println!("migration 31 at {rows} rows: {took:?}");
        assert_eq!(rows, 250_000);
    }

    /// Round 5B: with every connection busy, a caller takes the first one handed back, not one
    /// picked in advance that a long call may hold for seconds.
    #[test]
    fn a_caller_takes_the_first_connection_handed_back() {
        use std::sync::{Arc, Barrier};
        let tmp = tempfile::tempdir().unwrap();
        let db = Arc::new(Db::open(&tmp.path().join("library.db")).unwrap());
        let all_held = Arc::new(Barrier::new(POOL + 1));
        let mut holders = Vec::new();
        for i in 0..POOL {
            let (db, all_held) = (db.clone(), all_held.clone());
            holders.push(std::thread::spawn(move || {
                db.with_conn(|_| {
                    all_held.wait();
                    // One short call; the rest are long.
                    std::thread::sleep(if i == 0 { Duration::from_millis(50) } else { Duration::from_secs(3) });
                    Ok(())
                })
                .unwrap();
            }));
        }
        all_held.wait();
        let t = Instant::now();
        db.with_conn(|_| Ok(())).unwrap();
        assert!(t.elapsed() < Duration::from_secs(1), "waited {:?} for a connection", t.elapsed());
        for h in holders {
            h.join().unwrap();
        }
    }

    #[test]
    fn a_panicking_call_hands_its_connection_back() {
        let db = Db::open_in_memory().unwrap();
        let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| db.with_conn::<()>(|_| panic!("boom"))));
        assert!(caught.is_err());
        assert_eq!(db.schema_version().unwrap(), MIGRATIONS.last().unwrap().0);
    }

    #[test]
    fn reopening_is_idempotent_and_uses_wal() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("library.db");
        {
            Db::open(&path).unwrap();
        }
        let db = Db::open(&path).unwrap();
        assert_eq!(db.schema_version().unwrap(), MIGRATIONS.last().unwrap().0);
        let mode: String = db
            .with_conn(|c| Ok(c.query_row("PRAGMA journal_mode", [], |r| r.get(0))?))
            .unwrap();
        assert_eq!(mode, "wal");
    }

    /// The point of the pool: a reader is not held up by another connection's long hold.
    #[test]
    fn a_reader_is_not_blocked_by_a_long_hold() {
        let tmp = tempfile::tempdir().unwrap();
        let db = std::sync::Arc::new(Db::open(&tmp.path().join("library.db")).unwrap());
        let (tx, rx) = std::sync::mpsc::channel();
        let holder = {
            let db = db.clone();
            std::thread::spawn(move || {
                db.with_conn(|_| {
                    tx.send(()).unwrap();
                    std::thread::sleep(Duration::from_millis(600));
                    Ok(())
                })
                .unwrap()
            })
        };
        rx.recv().unwrap();
        let t = Instant::now();
        assert_eq!(db.schema_version().unwrap(), MIGRATIONS.last().unwrap().0);
        assert!(t.elapsed() < Duration::from_millis(200), "waited {:?} behind another connection", t.elapsed());
        holder.join().unwrap();
    }

    /// Writers on different connections queue rather than fail, even inside transactions that
    /// read first — the case a deferred transaction turns into SQLITE_BUSY_SNAPSHOT.
    #[test]
    fn concurrent_read_then_write_transactions_all_commit() {
        let tmp = tempfile::tempdir().unwrap();
        let db = std::sync::Arc::new(Db::open(&tmp.path().join("library.db")).unwrap());
        db.with_conn(|c| Ok(c.execute_batch("CREATE TABLE counter (n INTEGER); INSERT INTO counter VALUES (0);")?)).unwrap();
        let threads: Vec<_> = (0..8)
            .map(|_| {
                let db = db.clone();
                std::thread::spawn(move || {
                    for _ in 0..25 {
                        db.with_conn(|c| {
                            let tx = c.unchecked_transaction()?;
                            let n: i64 = tx.query_row("SELECT n FROM counter", [], |r| r.get(0))?;
                            tx.execute("UPDATE counter SET n = ?1", [n + 1])?;
                            tx.commit()?;
                            Ok(())
                        })
                        .unwrap();
                    }
                })
            })
            .collect();
        for t in threads {
            t.join().unwrap();
        }
        let n: i64 = db.with_conn(|c| Ok(c.query_row("SELECT n FROM counter", [], |r| r.get(0))?)).unwrap();
        assert_eq!(n, 200, "no lost update, no failed transaction");
    }
}
