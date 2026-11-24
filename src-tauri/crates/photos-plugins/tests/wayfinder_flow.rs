//! Round 24 W5: the real Wayfinder wasm (`plugins/first-party/wayfinder/plugin.wasm`, built by
//! `cargo build --release --target wasm32-wasip2` there) driven through the real host with a fake
//! model, library and places: the resumable tool, the cache, the review export. Skipped when the
//! wasm isn't built.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use photos_plugins::host::{
    Asset, AssetFilter, AssetPage, Engine, Gps, GpsSource, HostCtx, LibraryHost, MediaKind, ModelHost, Neighbour, PlaceHit, PlaceState, PlacesHost, ReviewEvent, Suggestion, Tensor,
    VectorSource,
};
use photos_plugins::PluginPermission;

const ID: &str = "jl.wayfinder";
const BLED: (f32, f32) = (46.3683, 14.1146);
const ROME: (f32, f32) = (41.9028, 12.4964);

fn wasm() -> Option<PathBuf> {
    let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../plugins/first-party/wayfinder/plugin.wasm");
    p.exists().then_some(p)
}

#[derive(Default)]
struct Fake {
    embedded: AtomicUsize,
    suggested: Mutex<Vec<Suggestion>>,
}

impl ModelHost for Fake {
    fn run(&self, _: &str, model: &str, inputs: Vec<Tensor>) -> Result<Vec<Tensor>, String> {
        assert_eq!((model, inputs[0].name.as_str()), ("location", "latlon"));
        let n = inputs[0].shape[0] as usize;
        Ok(vec![Tensor { name: "embedding".into(), shape: vec![n as u32, 512], data: vec![0.0; n * 512] }])
    }
    fn run_on_asset(&self, _: &str, model: &str, _: u64, input: &str) -> Result<Vec<Tensor>, String> {
        assert_eq!((model, input), ("image", "pixel_values"));
        self.embedded.fetch_add(1, Ordering::SeqCst);
        Ok(vec![Tensor { name: "embedding".into(), shape: vec![1, 512], data: vec![0.01; 512] }])
    }
    fn nearest(&self, _: &str, source: &VectorSource, _: &[f32], _: u32) -> Result<Vec<Neighbour>, String> {
        Ok(match source {
            VectorSource::Model(m) if m == "gallery" => vec![
                Neighbour { row: 0, score: 0.30, payload: vec![ROME.0, ROME.1] },
                Neighbour { row: 1, score: 0.29, payload: vec![BLED.0, BLED.1] },
            ],
            // the person's own photos: a very similar one in Bled
            VectorSource::Storage(n) if n == "personal.bin" => vec![Neighbour { row: 0, score: 0.8, payload: vec![BLED.0, BLED.1] }],
            VectorSource::Storage(_) => Vec::new(),
            VectorSource::Model(m) => return Err(format!("unexpected model {m}")),
        })
    }
}

impl LibraryHost for Fake {
    fn assets(&self, filter: &AssetFilter, _: Option<&str>) -> Result<AssetPage, String> {
        let asset = |id: u64, minutes: u32, gps: Option<(f32, f32)>| Asset {
            id,
            captured_at: Some(format!("2024-03-12T14:{minutes:02}:00")),
            offset_minutes: None,
            file_name: format!("{id}.jpg"),
            folder: Some("D:/Photos/Bled 2024".into()),
            kind: MediaKind::Photo,
            gps: gps.map(|g| Gps { lat: f64::from(g.0), lon: f64::from(g.1), source: GpsSource::Camera }),
            place: None,
            group: None,
            camera: None,
        };
        let items = match filter.has_gps {
            Some(true) => (1..=4).map(|i| asset(i, i as u32, Some(BLED))).collect(),
            _ => vec![asset(10, 10, None), asset(11, 12, None)],
        };
        Ok(AssetPage { items, next_cursor: None })
    }
}

impl PlacesHost for Fake {
    fn suggest(&self, _: &str, batch: Vec<Suggestion>) -> Result<(), String> {
        self.suggested.lock().unwrap().extend(batch);
        Ok(())
    }
    fn lookup(&self, name: &str) -> Vec<PlaceHit> {
        assert_eq!(name, "Bled", "the folder's name is cleaned before the lookup");
        vec![PlaceHit { name: "Bled, Slovenia".into(), lat: 46.37, lon: 14.11, country: Some("SI".into()) }]
    }
}

#[test]
fn find_locations_review_and_cache() {
    let Some(wasm) = wasm() else {
        eprintln!("skipped: build plugins/first-party/wayfinder first");
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let fake = Arc::new(Fake::default());
    let granted = vec![PluginPermission::Models, PluginPermission::LibraryRead, PluginPermission::PlacesWrite, PluginPermission::Storage];
    let (d, f) = (dir.path().to_path_buf(), fake.clone());
    let make = move || HostCtx {
        plugin: ID.into(),
        granted: granted.clone(),
        settings: Default::default(),
        data_dir: d.join("data"),
        log: Arc::new(|l: &str| eprintln!("[wayfinder] {l}")),
        cancel: None,
        chosen: None,
        photos: None,
        drafts: None,
        models: Some(f.clone()),
        library: Some(f.clone()),
        places: Some(f.clone()),
        proposals: None,
    };
    let engine = Engine::new(&dir.path().join("cache"), Arc::new(|_, _| {})).unwrap();
    let tools = engine.tools_at(ID, &wasm, make).unwrap();
    assert_eq!(tools.list().unwrap()[0].name, "find_locations");
    assert!(tools.has_review());

    let done = tools.call("find_locations", "{}").unwrap();
    assert!(done.contains("\"status\":\"done\""), "{done}");
    // four camera photos learnt, two photos looked at
    assert_eq!(fake.embedded.load(Ordering::SeqCst), 6);
    {
        let sent = fake.suggested.lock().unwrap();
        let mut ids: Vec<u64> = sent.iter().map(|s| s.asset).collect();
        ids.sort_unstable();
        assert_eq!(ids, [10, 11]);
        let a = sent.iter().find(|s| s.asset == 10).unwrap();
        // the own-photo match (0.8 ≥ 0.65) beats the gallery's Rome
        assert!((a.candidates[0].lat - f64::from(BLED.0)).abs() < 1e-3, "{:?}", a.candidates[0]);
        let b = sent.iter().find(|s| s.asset == 11).unwrap();
        assert_eq!(a.group, b.group, "two minutes apart: one moment");
        assert!(a.group_label.contains("2 photos") && a.group_label.ends_with("Bled, Slovenia"), "{}", a.group_label);
        // 14:10 is six minutes after a camera photo in Bled, so the time signal makes it inferred
        assert_eq!(a.state, PlaceState::Inferred);
    }

    // confirming one photo makes it an anchor
    let changed = tools.on_review(&ReviewEvent::Confirm(vec![10])).unwrap();
    assert!(changed.iter().any(|s| s.asset == 10 && s.state == PlaceState::Anchor), "{changed:?}");
    // rejecting it again leaves it without a suggestion
    tools.on_review(&ReviewEvent::Reject(vec![10])).unwrap();

    // a second run uses the cache: nothing is embedded again
    let before = fake.embedded.load(Ordering::SeqCst);
    let again = tools.call("find_locations", "{}").unwrap();
    assert!(again.contains("\"status\":\"done\""), "{again}");
    assert_eq!(fake.embedded.load(Ordering::SeqCst), before, "a re-run is instant");

    assert!(tools.call("find_locations", r#"{"scope":"moon"}"#).is_err());
    assert!(tools.call("nope", "{}").is_err());
}
