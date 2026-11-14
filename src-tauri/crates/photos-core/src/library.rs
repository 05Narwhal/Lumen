use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::db::Db;
use crate::manifest::LibraryManifest;
use crate::{CoreError, Result};

pub const DB_FILE: &str = "library.db";
/// Files being copied in live here until their catalogue row exists.
pub const INCOMING_DIR: &str = "originals/.incoming";
const SUBDIRS: &[&str] = &[
    "originals",
    "derivatives/thumbs",
    "derivatives/previews",
    "derivatives/video",
    "trash",
    "exports",
    "logs",
];

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct LibraryInfo {
    pub id: String,
    pub name: String,
    pub path: String,
    pub schema_version: u32,
}

/// A managed library root: the folder tree from the architecture spec §5.1, its manifest and database.
pub struct Library {
    pub root: PathBuf,
    pub manifest: LibraryManifest,
    pub db: Db,
}

impl Library {
    pub fn exists_at(root: &Path) -> bool {
        root.join(DB_FILE).is_file()
    }

    pub fn create(root: &Path) -> Result<Self> {
        if Self::exists_at(root) {
            return Err(CoreError::AlreadyExists(root.display().to_string()));
        }
        for d in SUBDIRS {
            std::fs::create_dir_all(root.join(d))?;
        }
        let manifest = LibraryManifest::new(&dir_name(root));
        manifest.save(root)?;
        let db = Db::open(&root.join(DB_FILE))?;
        Ok(Self { root: root.to_path_buf(), manifest, db })
    }

    /// What Lumen itself opens (Round 17): a library newer than this build is refused and left
    /// exactly as it was; one with migrations pending is copied first to its own folder under
    /// `backups`, and if that fails it is not migrated. Other callers use [`Self::open`].
    pub fn open_protected(root: &Path, backups: &Path) -> Result<Self> {
        if !Self::exists_at(root) {
            return Err(CoreError::NotFound(root.display().to_string()));
        }
        let db = root.join(DB_FILE);
        crate::db::check_not_newer(&db)?;
        crate::db::safety_copy(&db, &backups.join(safety_copy_folder(root)), crate::now_unix())?;
        Self::open(root)
    }

    /// Opens an existing library. A library made before manifests existed gets one now. One newer
    /// than this build is refused before anything in it changes.
    pub fn open(root: &Path) -> Result<Self> {
        if !Self::exists_at(root) {
            return Err(CoreError::NotFound(root.display().to_string()));
        }
        crate::db::check_not_newer(&root.join(DB_FILE))?;
        for d in SUBDIRS {
            std::fs::create_dir_all(root.join(d))?;
        }
        let manifest = match LibraryManifest::load(root)? {
            Some(m) => m,
            None => {
                let m = LibraryManifest::new(&dir_name(root));
                m.save(root)?;
                m
            }
        };
        let db = Db::open(&root.join(DB_FILE))?;
        // Device names follow the naming rule until the user renames them; a handful of
        // rows, so this is cheap enough to do on every open rather than in a migration
        // (which cannot call the Rust that owns the rule).
        db.with_conn(crate::devices::refresh_default_names)?;
        Ok(Self { root: root.to_path_buf(), manifest, db })
    }

    pub fn name(&self) -> String {
        self.manifest.name.clone()
    }

    pub fn info(&self) -> Result<LibraryInfo> {
        Ok(LibraryInfo {
            id: self.manifest.id.clone(),
            name: self.name(),
            path: self.root.display().to_string(),
            schema_version: self.db.schema_version()?,
        })
    }

    /// Absolute path of an asset's original file.
    pub fn original_path(&self, id: i64) -> Result<PathBuf> {
        let rel: String = self.db.with_conn(|c| {
            c.query_row("SELECT relative_path FROM assets WHERE id = ?1", [id], |r| r.get(0))
                .map_err(|e| match e {
                    rusqlite::Error::QueryReturnedNoRows => CoreError::NotFound(format!("asset {id}")),
                    e => e.into(),
                })
        })?;
        Ok(self.originals_dir().join(rel))
    }

    pub fn originals_dir(&self) -> PathBuf {
        self.root.join("originals")
    }

    pub fn incoming_dir(&self) -> PathBuf {
        self.root.join(INCOMING_DIR)
    }

    /// Crash recovery on open: batches left `running` become `interrupted` (what they
    /// brought in is kept, and importing again picks up the rest), and staged copies that
    /// never got a catalogue row are removed. Files already placed in `originals/` without a
    /// row are `photos_import::recover`'s, which can hash them. Returns the batches touched.
    pub fn reconcile(&self) -> Result<u32> {
        let n = self.db.with_conn(|c| {
            Ok(c.execute(
                "UPDATE import_batches SET state = ?1, finished_at = unixepoch() WHERE state = 0",
                [crate::batches::BATCH_INTERRUPTED],
            )? as u32)
        })?;
        match std::fs::remove_dir_all(self.incoming_dir()) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
        Ok(n)
    }
}

/// What [`check_upgrade`] found: the schema the library is at, and what its copy reached.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct UpgradeCheck {
    pub from: Option<u32>,
    pub to: u32,
}

/// The release check (Round 17): copies the library at `root`'s database into `scratch`, opens the
/// copy as Lumen would and migrates it, then removes `scratch`. The library is only read.
pub fn check_upgrade(root: &Path, scratch: &Path) -> Result<UpgradeCheck> {
    if !Library::exists_at(root) {
        return Err(CoreError::NotFound(root.display().to_string()));
    }
    let comparable = |p: &Path| -> Result<String> {
        let mut s = std::path::absolute(p)?.to_string_lossy().replace('\\', "/");
        if cfg!(windows) {
            s = s.to_lowercase();
        }
        if !s.ends_with('/') {
            s.push('/');
        }
        Ok(s)
    };
    if comparable(scratch)?.starts_with(&comparable(root)?) {
        return Err(CoreError::Invalid("the check's scratch folder cannot be inside the library".into()));
    }
    crate::db::check_not_newer(&root.join(DB_FILE))?;
    let result = (|| -> Result<UpgradeCheck> {
        let copy = scratch.join("library");
        std::fs::create_dir_all(&copy)?;
        if let Ok(manifest) = std::fs::read(root.join(crate::manifest::MANIFEST_FILE)) {
            std::fs::write(copy.join(crate::manifest::MANIFEST_FILE), manifest)?;
        }
        crate::db::snapshot_into(&root.join(DB_FILE), &copy.join(DB_FILE))?;
        let from = crate::db::peek_schema(&copy.join(DB_FILE))?;
        let lib = Library::open_protected(&copy, &scratch.join("backups"))?;
        let to = lib.db.schema_version()?;
        Ok(UpgradeCheck { from, to })
    })();
    let _ = std::fs::remove_dir_all(scratch);
    result
}

/// One library's folder of safety copies: its id, or its folder's name before it has a manifest.
fn safety_copy_folder(root: &Path) -> String {
    match LibraryManifest::load(root) {
        Ok(Some(m)) => m.id,
        _ => dir_name(root).chars().map(|c| if c.is_alphanumeric() || " ._-".contains(c) { c } else { '_' }).collect(),
    }
}

fn dir_name(root: &Path) -> String {
    root.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "Library".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_then_open() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("Lumen Library");
        assert!(!Library::exists_at(&root));
        let lib = Library::create(&root).unwrap();
        for d in ["originals", "derivatives/thumbs", "derivatives/previews", "derivatives/video", "trash", "exports", "logs"] {
            assert!(root.join(d).is_dir(), "{d}");
        }
        assert!(Library::exists_at(&root));
        assert!(matches!(Library::create(&root), Err(CoreError::AlreadyExists(_))));
        let info = lib.info().unwrap();
        assert_eq!(info.name, "Lumen Library");
        assert_eq!(info.id, lib.manifest.id);
        assert_eq!(info.schema_version, crate::db::MIGRATIONS.last().unwrap().0);
        let reopened = Library::open(&root).unwrap();
        assert_eq!(reopened.manifest.id, lib.manifest.id);
        assert!(matches!(Library::open(&tmp.path().join("nope")), Err(CoreError::NotFound(_))));
    }

    /// A library left at migration 30 by an older Lumen, with its manifest and one original.
    fn library_at_30(root: &Path) -> LibraryManifest {
        std::fs::create_dir_all(root.join("originals")).unwrap();
        std::fs::write(root.join("originals/a.jpg"), "photo").unwrap();
        let manifest = LibraryManifest::new("Old");
        manifest.save(root).unwrap();
        let mut conn = rusqlite::Connection::open(root.join(DB_FILE)).unwrap();
        crate::db::apply_up_to(&mut conn, 30).unwrap();
        manifest
    }

    /// Every path under `root` with its bytes, so a test can say nothing changed.
    fn tree(root: &Path) -> Vec<(String, Vec<u8>)> {
        let mut out = Vec::new();
        let mut stack = vec![root.to_path_buf()];
        while let Some(dir) = stack.pop() {
            for e in std::fs::read_dir(&dir).unwrap() {
                let p = e.unwrap().path();
                if p.is_dir() {
                    stack.push(p.clone());
                    out.push((p.display().to_string(), Vec::new()));
                } else {
                    out.push((p.display().to_string(), std::fs::read(&p).unwrap()));
                }
            }
        }
        out.sort();
        out
    }

    #[test]
    fn a_newer_library_is_refused_and_left_untouched() {
        let t = tempfile::tempdir().unwrap();
        let root = t.path().join("lib");
        std::fs::create_dir_all(&root).unwrap();
        let conn = rusqlite::Connection::open(root.join(DB_FILE)).unwrap();
        conn.execute_batch("CREATE TABLE schema_version (version INTEGER NOT NULL, applied_at INTEGER NOT NULL); INSERT INTO schema_version VALUES (9999, 1);").unwrap();
        drop(conn);
        let before = tree(&root);
        let backups = t.path().join("backups");
        for result in [Library::open_protected(&root, &backups), Library::open(&root)] {
            assert!(matches!(result, Err(CoreError::NewerSchema { library: 9999, .. })));
        }
        assert_eq!(tree(&root), before, "no manifest, no folders, no database change");
        assert!(!backups.exists(), "and no copy");
    }

    /// Real libraries are WAL databases. A refusal leaves the database's own bytes alone.
    #[test]
    fn a_newer_wal_library_keeps_its_bytes() {
        let t = tempfile::tempdir().unwrap();
        let root = t.path().join("lib");
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join(DB_FILE);
        let conn = rusqlite::Connection::open(&path).unwrap();
        conn.pragma_update(None, "journal_mode", "WAL").unwrap();
        conn.execute_batch("CREATE TABLE schema_version (version INTEGER NOT NULL, applied_at INTEGER NOT NULL); INSERT INTO schema_version VALUES (9999, 1);").unwrap();
        drop(conn);
        let before = std::fs::read(&path).unwrap();
        assert!(matches!(Library::open_protected(&root, &t.path().join("b")), Err(CoreError::NewerSchema { .. })));
        assert_eq!(std::fs::read(&path).unwrap(), before);
    }

    #[test]
    fn opening_an_older_library_copies_it_first_outside_the_library() {
        let t = tempfile::tempdir().unwrap();
        let root = t.path().join("lib");
        std::fs::create_dir_all(&root).unwrap();
        let manifest = library_at_30(&root);
        let backups = t.path().join("backups");
        let lib = Library::open_protected(&root, &backups).unwrap();
        assert_eq!(lib.db.schema_version().unwrap(), crate::db::MIGRATIONS.last().unwrap().0);
        drop(lib);
        let folder = backups.join(&manifest.id);
        let copies: Vec<_> = std::fs::read_dir(&folder).unwrap().map(|e| e.unwrap().path()).collect();
        assert_eq!(copies.len(), 1);
        assert_eq!(crate::db::peek_schema(&copies[0]).unwrap(), Some(30));
        assert!(root.join("originals/a.jpg").is_file());
        let added: Vec<String> = std::fs::read_dir(&root).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).filter(|n| n.contains("v30")).collect();
        assert!(added.is_empty(), "nothing beside the library's own files: {added:?}");
        // Open again: nothing pending, no second copy.
        drop(Library::open_protected(&root, &backups).unwrap());
        assert_eq!(std::fs::read_dir(&folder).unwrap().count(), 1);
    }

    #[test]
    fn a_failed_safety_copy_blocks_the_migration() {
        let t = tempfile::tempdir().unwrap();
        let root = t.path().join("lib");
        std::fs::create_dir_all(&root).unwrap();
        library_at_30(&root);
        let blocked = t.path().join("backups");
        std::fs::write(&blocked, "in the way").unwrap();
        assert!(matches!(Library::open_protected(&root, &blocked), Err(CoreError::SafetyCopy(_))));
        assert_eq!(crate::db::peek_schema(&root.join(DB_FILE)).unwrap(), Some(30), "the migration did not run");
    }

    #[test]
    fn two_libraries_keep_separate_copies() {
        let t = tempfile::tempdir().unwrap();
        let backups = t.path().join("backups");
        let mut ids = Vec::new();
        for name in ["one", "two"] {
            let root = t.path().join(name);
            std::fs::create_dir_all(&root).unwrap();
            ids.push(library_at_30(&root).id);
            drop(Library::open_protected(&root, &backups).unwrap());
        }
        for id in ids {
            assert_eq!(std::fs::read_dir(backups.join(id)).unwrap().count(), 1);
        }
    }

    #[test]
    fn a_library_without_a_manifest_is_copied_under_its_folder_name() {
        let t = tempfile::tempdir().unwrap();
        let root = t.path().join("Old Photos");
        std::fs::create_dir_all(&root).unwrap();
        let mut conn = rusqlite::Connection::open(root.join(DB_FILE)).unwrap();
        crate::db::apply_up_to(&mut conn, 30).unwrap();
        drop(conn);
        let backups = t.path().join("backups");
        drop(Library::open_protected(&root, &backups).unwrap());
        assert_eq!(std::fs::read_dir(backups.join("Old Photos")).unwrap().count(), 1);
    }

    #[test]
    fn open_protected_says_not_found_for_nothing_there() {
        let t = tempfile::tempdir().unwrap();
        assert!(matches!(Library::open_protected(&t.path().join("nope"), &t.path().join("b")), Err(CoreError::NotFound(_))));
    }

    /// The release check (Round 17): a copy of a real library, in a scratch folder, opens and
    /// migrates. The library itself is only read.
    #[test]
    fn a_release_check_migrates_a_copy_and_reads_the_library_only() {
        let t = tempfile::tempdir().unwrap();
        let root = t.path().join("lib");
        std::fs::create_dir_all(&root).unwrap();
        library_at_30(&root);
        let before = tree(&root);
        let scratch = t.path().join("scratch");
        let r = check_upgrade(&root, &scratch).unwrap();
        assert_eq!(r, UpgradeCheck { from: Some(30), to: crate::db::MIGRATIONS.last().unwrap().0 });
        assert_eq!(tree(&root), before, "the library is untouched");
        assert!(!scratch.exists(), "the scratch copy is removed");
    }

    #[test]
    fn a_release_check_says_why_it_failed_and_cleans_up() {
        let t = tempfile::tempdir().unwrap();
        let root = t.path().join("lib");
        std::fs::create_dir_all(&root).unwrap();
        let conn = rusqlite::Connection::open(root.join(DB_FILE)).unwrap();
        conn.execute_batch("CREATE TABLE schema_version (version INTEGER NOT NULL, applied_at INTEGER NOT NULL); INSERT INTO schema_version VALUES (9999, 1);").unwrap();
        drop(conn);
        let scratch = t.path().join("scratch");
        assert!(matches!(check_upgrade(&root, &scratch), Err(CoreError::NewerSchema { .. })));
        assert!(!scratch.exists());
        assert!(matches!(check_upgrade(&t.path().join("nope"), &scratch), Err(CoreError::NotFound(_))));
    }

    #[test]
    fn a_release_check_refuses_a_scratch_folder_inside_the_library() {
        let t = tempfile::tempdir().unwrap();
        let root = t.path().join("lib");
        std::fs::create_dir_all(&root).unwrap();
        library_at_30(&root);
        let before = tree(&root);
        assert!(matches!(check_upgrade(&root, &root.join("scratch")), Err(CoreError::Invalid(_))));
        assert_eq!(tree(&root), before);
    }

    #[test]
    fn open_adopts_a_legacy_library_without_manifest() {
        let t = tempfile::tempdir().unwrap();
        let legacy = t.path().join("Old");
        std::fs::create_dir_all(&legacy).unwrap();
        Db::open(&legacy.join("library.db")).unwrap();
        let l = Library::open(&legacy).unwrap();
        assert_eq!(l.manifest.name, "Old");
        assert!(legacy.join("library.json").is_file());
    }

    #[test]
    fn reconcile_interrupts_running_batches_and_clears_incoming() {
        let t = tempfile::tempdir().unwrap();
        let lib = Library::create(t.path()).unwrap();
        lib.db
            .with_conn(|c| {
                c.execute("INSERT INTO import_batches(source_kind, source_label, started_at, state) VALUES (0,'x',0,0)", [])?;
                Ok(())
            })
            .unwrap();
        std::fs::create_dir_all(lib.incoming_dir()).unwrap();
        std::fs::write(lib.incoming_dir().join("junk"), b"x").unwrap();
        assert_eq!(lib.reconcile().unwrap(), 1);
        let st: i64 = lib.db.with_conn(|c| Ok(c.query_row("SELECT state FROM import_batches", [], |r| r.get(0))?)).unwrap();
        // Interrupted, not failed: what it brought in stays, and importing again finishes it.
        assert_eq!(st, crate::batches::BATCH_INTERRUPTED);
        assert!(!lib.incoming_dir().exists());
        assert_eq!(lib.reconcile().unwrap(), 0);
    }

    #[test]
    fn original_path_resolves_from_assets_row() {
        let tmp = tempfile::tempdir().unwrap();
        let lib = Library::create(tmp.path()).unwrap();
        lib.db
            .with_conn(|c| {
                c.execute(
                    "INSERT INTO assets (id, content_hash, relative_path, filename, extension, media_type, byte_size, imported_at) VALUES (7,'abc','2026/09/17/abc_a.jpg','a.jpg','jpg',0,3,0)",
                    [],
                )?;
                Ok(())
            })
            .unwrap();
        assert_eq!(
            lib.original_path(7).unwrap(),
            tmp.path().join("originals").join("2026/09/17/abc_a.jpg")
        );
        assert!(matches!(lib.original_path(8), Err(CoreError::NotFound(_))));
    }
}
