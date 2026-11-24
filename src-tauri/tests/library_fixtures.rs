//! Release simplification, phase 2: a synthetic library for every schema a published release
//! shipped, made in a temp folder from the migrations themselves. The release's `library-check`
//! gate opens each with the built Lumen (`--check-library`), so a migration that cannot upgrade a
//! library an earlier release made is caught on a tag, without anybody's real library.
//!
//! Nothing here reads or writes a real library or a device: every path is under a temp dir (or
//! the empty folder named by `LUMEN_FIXTURES_OUT`, which CI creates).

use std::path::{Path, PathBuf};

use photos_core::db::MIGRATIONS;
use photos_core::library::{check_upgrade, DB_FILE};
use photos_core::manifest::LibraryManifest;
use rusqlite::Connection;

/// Each published release and the newest migration its library can have. A new entry is added
/// when a release is published (docs/RELEASING.md); a library from any of them must still open.
const PUBLISHED_SCHEMAS: &[(&str, u32)] = &[("26.1.0-alpha", 37), ("26.1.1-alpha", 39), ("26.2.0-alpha", 39)];

const ORIGINAL: &[u8] = b"not a real photo: a fixture original that must never change";

/// A library as release `tag` left it: its migrations, a few rows, one original and a manifest.
fn library_at(root: &Path, schema: u32) {
    std::fs::create_dir_all(root.join("originals/2026/01")).unwrap();
    std::fs::write(root.join("originals/2026/01/a.jpg"), ORIGINAL).unwrap();
    LibraryManifest::new("Fixture").save(root).unwrap();
    let conn = Connection::open(root.join(DB_FILE)).unwrap();
    conn.pragma_update(None, "journal_mode", "WAL").unwrap();
    conn.execute_batch("CREATE TABLE schema_version (version INTEGER NOT NULL, applied_at INTEGER NOT NULL);").unwrap();
    for (version, sql) in MIGRATIONS.iter().filter(|(v, _)| *v <= schema) {
        conn.execute_batch(sql).unwrap_or_else(|e| panic!("migration {version}: {e}"));
        conn.execute("INSERT INTO schema_version(version, applied_at) VALUES (?1, 1)", [version]).unwrap();
    }
    for id in 1..=3 {
        conn.execute(
            "INSERT INTO assets (id, content_hash, relative_path, filename, extension, media_type, byte_size, orientation, imported_at) VALUES (?1, ?2, ?3, ?4, 'jpg', 0, 1, 1, 1)",
            rusqlite::params![id, format!("hash{id}"), format!("2026/01/{id}.jpg"), format!("{id}.jpg")],
        )
        .unwrap();
    }
}

/// The files that must come through a check untouched, with their bytes.
fn snapshot(root: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_none_or(|e| e != "db-wal" && e != "db-shm") {
                out.push((path.strip_prefix(root).unwrap().to_path_buf(), std::fs::read(&path).unwrap()));
            }
        }
    }
    out.sort();
    out
}

#[test]
fn every_published_schema_opens_and_migrates() {
    let head = MIGRATIONS.last().unwrap().0;
    for (tag, schema) in PUBLISHED_SCHEMAS {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join(format!("Library {tag}"));
        library_at(&root, *schema);
        let before = snapshot(&root);
        let scratch = tmp.path().join("scratch");
        let found = check_upgrade(&root, &scratch).unwrap_or_else(|e| panic!("{tag}: {e}"));
        assert_eq!(found.from, Some(*schema), "{tag}: the fixture's own schema");
        assert_eq!(found.to, head, "{tag}: migrated to this build's head");
        assert_eq!(snapshot(&root), before, "{tag}: the checked library and its originals are untouched");
        assert!(!scratch.exists(), "{tag}: the scratch copy is removed");
    }
}

#[test]
fn published_schemas_never_exceed_this_build() {
    let head = MIGRATIONS.last().unwrap().0;
    assert!(PUBLISHED_SCHEMAS.iter().all(|(_, s)| *s <= head), "a published schema newer than the newest migration");
    assert!(PUBLISHED_SCHEMAS.windows(2).all(|w| w[0].1 <= w[1].1), "list them oldest first");
}

/// CI sets `LUMEN_FIXTURES_OUT` to an empty folder it made; this writes one library per published
/// schema into it, named `<tag>`, and `preflight.mjs` runs the built exe on each. Without the
/// variable it does nothing.
#[test]
fn write_fixtures_when_asked() {
    let Some(out) = std::env::var_os("LUMEN_FIXTURES_OUT") else { return };
    let out = PathBuf::from(out);
    assert!(!out.join("library.json").exists() && std::fs::read_dir(&out).map(|d| d.count()).unwrap_or(0) == 0, "LUMEN_FIXTURES_OUT must be an empty folder");
    for (tag, schema) in PUBLISHED_SCHEMAS {
        library_at(&out.join(format!("{tag}-schema-{schema}")), *schema);
    }
}
