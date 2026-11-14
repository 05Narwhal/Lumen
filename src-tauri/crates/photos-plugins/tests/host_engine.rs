//! The wasm engine and its compile cache (Round 23 A2 Task 1).

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use photos_plugins::host::{Engine, OnStopped};

fn wasm(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/wasm")
        .join(format!("{name}.wasm"))
}

fn noop() -> OnStopped {
    Arc::new(|_: &str, _: String| {})
}

fn cached(dir: &Path, ext: &str) -> Vec<PathBuf> {
    fs::read_dir(dir)
        .map(|r| {
            r.flatten()
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|x| x == ext))
                .collect()
        })
        .unwrap_or_default()
}

#[test]
fn wit_hash_matches_fixtures() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let wit = fs::read_to_string(root.join("wit/lumen-plugin.wit")).unwrap();
    let hash = blake3::hash(wit.replace("\r\n", "\n").as_bytes()).to_hex().to_string();
    let want = fs::read_to_string(root.join("tests/wasm/WIT_HASH")).unwrap();
    assert_eq!(
        hash,
        want.trim(),
        "the WIT changed since the fixtures were built: run build-fixtures (node scripts/plugins/build-fixtures.mjs)"
    );
}

#[test]
fn compiles_and_caches() {
    let tmp = tempfile::tempdir().unwrap();
    let first = Engine::new(tmp.path(), noop()).unwrap();
    first.component(&wasm("importer_ok")).unwrap();
    assert_eq!(first.compiles(), 1);
    assert_eq!(cached(tmp.path(), "cwasm").len(), 1);
    assert_eq!(cached(tmp.path(), "b3").len(), 1);

    let second = Engine::new(tmp.path(), noop()).unwrap();
    second.component(&wasm("importer_ok")).unwrap();
    assert_eq!(second.compiles(), 0, "the second engine reads the cache");
}

#[test]
fn corrupt_cache_recompiles() {
    let tmp = tempfile::tempdir().unwrap();
    Engine::new(tmp.path(), noop())
        .unwrap()
        .component(&wasm("importer_ok"))
        .unwrap();
    let cwasm = cached(tmp.path(), "cwasm").remove(0);
    let full = fs::metadata(&cwasm).unwrap().len();
    let bytes = fs::read(&cwasm).unwrap();
    fs::write(&cwasm, &bytes[..bytes.len() / 2]).unwrap();

    let engine = Engine::new(tmp.path(), noop()).unwrap();
    engine.component(&wasm("importer_ok")).unwrap();
    assert_eq!(engine.compiles(), 1, "a truncated .cwasm is compiled again");
    assert_eq!(fs::metadata(&cwasm).unwrap().len(), full, "and rewritten");
}
