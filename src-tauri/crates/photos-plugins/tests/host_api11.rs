//! API 1.1 (Round 24 D1): 1.0.0 components still run against the 1.1.0 host, the new
//! interfaces reach their host traits only when granted, and the `review` export answers.
//! `api10_*.wasm` are the Round 23 fixtures built against WIT 1.0.0, kept as they were.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use photos_plugins::host::{
    Asset, AssetFilter, AssetPage, ChosenRoot, Engine, HostCtx, LibraryHost, MediaKind, ModelHost, Neighbour, PlaceHit,
    PlaceState, PlacesHost, ReviewEvent, Suggestion, Tensor, VectorSource,
};
use photos_plugins::{Pick, PluginPermission};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("tests/wasm/{name}.wasm"))
}

fn engine(dir: &Path) -> Engine {
    Engine::new(&dir.join("cache"), Arc::new(|_, _| {})).unwrap()
}

#[derive(Default)]
struct Fake {
    suggested: Mutex<Vec<Suggestion>>,
}

impl ModelHost for Fake {
    fn run(&self, plugin: &str, model: &str, inputs: Vec<Tensor>) -> Result<Vec<Tensor>, String> {
        assert_eq!((plugin, model), ("com.test.x", "m"));
        let data = inputs[0].data.iter().map(|v| v * 2.0).collect();
        Ok(vec![Tensor { name: "y".into(), shape: vec![2], data }])
    }
    fn run_on_asset(&self, _: &str, _: &str, _: u64, _: &str) -> Result<Vec<Tensor>, String> {
        Err("not in this test".into())
    }
    fn nearest(&self, _: &str, source: &VectorSource, query: &[f32], k: u32) -> Result<Vec<Neighbour>, String> {
        assert!(matches!(source, VectorSource::Storage(n) if n == "own"));
        assert_eq!((query, k), (&[1.0f32, 0.0][..], 2));
        Ok(vec![Neighbour { row: 4, score: 0.9, payload: vec![] }, Neighbour { row: 1, score: 0.5, payload: vec![] }])
    }
}

impl LibraryHost for Fake {
    fn assets(&self, filter: &AssetFilter, cursor: Option<&str>) -> Result<AssetPage, String> {
        assert_eq!((filter.has_gps, cursor), (Some(false), None));
        let asset = |id| Asset {
            id,
            captured_at: None,
            offset_minutes: None,
            file_name: format!("{id}.jpg"),
            folder: None,
            kind: MediaKind::Photo,
            gps: None,
            place: None,
            group: None,
            camera: None,
        };
        Ok(AssetPage { items: vec![asset(3), asset(5)], next_cursor: None })
    }
}

impl PlacesHost for Fake {
    fn suggest(&self, _: &str, batch: Vec<Suggestion>) -> Result<(), String> {
        self.suggested.lock().unwrap().extend(batch);
        Ok(())
    }
    fn lookup(&self, name: &str) -> Vec<PlaceHit> {
        vec![PlaceHit { name: format!("{name}, Slovenia"), lat: 46.36, lon: 14.09, country: Some("SI".into()) }]
    }
}

fn ctx(dir: &Path, granted: Vec<PluginPermission>, fake: &Arc<Fake>) -> HostCtx {
    HostCtx {
        plugin: "com.test.x".into(),
        granted,
        settings: Default::default(),
        data_dir: dir.join("data"),
        log: Arc::new(|_: &str| {}),
        cancel: None,
        chosen: None,
        photos: None,
        drafts: None,
        models: Some(fake.clone()),
        library: Some(fake.clone()),
        places: Some(fake.clone()),
        proposals: None,
    }
}

const ALL_11: [PluginPermission; 4] =
    [PluginPermission::Models, PluginPermission::LibraryRead, PluginPermission::PlacesWrite, PluginPermission::Storage];

#[test]
fn api10_tools_still_run() {
    let dir = tempfile::tempdir().unwrap();
    let fake = Arc::new(Fake::default());
    let granted = vec![PluginPermission::PhotosRead, PluginPermission::Storage];
    let (d, f) = (dir.path().to_path_buf(), fake.clone());
    let tools = engine(dir.path()).tools_at("com.test.x", &fixture("api10_tools_ok"), move || ctx(&d, granted.clone(), &f)).unwrap();
    let names: Vec<String> = tools.list().unwrap().into_iter().map(|t| t.name).collect();
    assert_eq!(names, ["echo", "record"]);
    assert_eq!(tools.call("echo", "{}").unwrap(), "{\"args\":{},\"scope\":0}");
    assert!(!tools.has_review());
}

#[test]
fn api10_importer_still_runs() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("src");
    std::fs::create_dir_all(&src).unwrap();
    std::fs::write(src.join("a.jpg"), b"jpeg").unwrap();
    let fake = Arc::new(Fake::default());
    let mut c = ctx(dir.path(), vec![PluginPermission::ImportReadChosen], &fake);
    c.chosen = Some(ChosenRoot::new(&src, Pick::Folder).unwrap());
    let mut inst = engine(dir.path()).importer_at(&fixture("api10_importer_ok"), c).unwrap();
    assert_eq!(inst.enumerate().unwrap().len(), 2);
}

#[test]
fn new_interfaces_reach_their_hosts() {
    let dir = tempfile::tempdir().unwrap();
    let fake = Arc::new(Fake::default());
    let (d, f) = (dir.path().to_path_buf(), fake.clone());
    let tools = engine(dir.path()).tools_at("com.test.x", &fixture("review_ok"), move || ctx(&d, ALL_11.to_vec(), &f)).unwrap();
    assert_eq!(tools.call("library", "{}").unwrap(), "3,5");
    assert_eq!(tools.call("lookup", "{}").unwrap(), "Bled, Slovenia");
    assert_eq!(tools.call("run", "{}").unwrap(), "[2.0, 4.0]");
    assert_eq!(tools.call("nearest", "{}").unwrap(), "4,1");
    assert_eq!(tools.call("suggest", "{}").unwrap(), "sent");
    let sent = fake.suggested.lock().unwrap();
    assert_eq!((sent.len(), sent[0].asset, sent[0].state), (1, 7, PlaceState::Guess));
}

#[test]
fn review_export_answers() {
    let dir = tempfile::tempdir().unwrap();
    let fake = Arc::new(Fake::default());
    let (d, f) = (dir.path().to_path_buf(), fake.clone());
    let tools = engine(dir.path()).tools_at("com.test.x", &fixture("review_ok"), move || ctx(&d, ALL_11.to_vec(), &f)).unwrap();
    assert!(tools.has_review());
    let out = tools.on_review(&ReviewEvent::Confirm(vec![2, 9])).unwrap();
    assert_eq!(out.iter().map(|s| (s.asset, s.state)).collect::<Vec<_>>(), [(2, PlaceState::Anchor), (9, PlaceState::Anchor)]);
    assert!(tools.on_review(&ReviewEvent::Reject(vec![2])).unwrap().is_empty());
}

#[test]
fn ungranted_interface_refused() {
    for (missing, iface) in [
        (PluginPermission::PlacesWrite, "lumen:plugin/places"),
        (PluginPermission::Models, "lumen:plugin/models"),
        (PluginPermission::LibraryRead, "lumen:plugin/library"),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let fake = Arc::new(Fake::default());
        let granted: Vec<_> = ALL_11.into_iter().filter(|p| *p != missing).collect();
        let (d, f) = (dir.path().to_path_buf(), fake.clone());
        let err = engine(dir.path())
            .tools_at("com.test.x", &fixture("review_ok"), move || ctx(&d, granted.clone(), &f))
            .err()
            .expect("refused");
        assert_eq!(err.code, "plugin_permission", "{err}");
        assert!(err.message.contains(iface), "{err}");
    }
}
