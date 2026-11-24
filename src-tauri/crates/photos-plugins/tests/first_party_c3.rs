//! Round 26 C3: the real Clockwork and Captioner wasm
//! (`plugins/first-party/{clockwork,captioner}/plugin.wasm`, each built by
//! `cargo build --release --target wasm32-wasip2` in its folder) driven through the real host
//! with a fake library and a fake proposals host, on synthetic photos only. Each test is skipped
//! when its wasm isn't built.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use photos_core::proposals::{NewProposal, ProposalValue};
use photos_plugins::host::{Asset, AssetDetails, AssetFilter, AssetPage, Engine, Gps, GpsSource, HostCtx, LibraryHost, MediaKind, ProposalsHost};
use photos_plugins::PluginPermission;

fn wasm(name: &str) -> Option<PathBuf> {
    let p = Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("../../../plugins/first-party/{name}/plugin.wasm"));
    p.exists().then_some(p)
}

struct Fake {
    assets: Vec<Asset>,
    details: Vec<AssetDetails>,
    proposed: Mutex<Vec<NewProposal>>,
}

impl LibraryHost for Fake {
    fn assets(&self, filter: &AssetFilter, cursor: Option<&str>) -> Result<AssetPage, String> {
        let after: u64 = cursor.map(|c| c.parse().unwrap()).unwrap_or(0);
        let mut all: Vec<&Asset> = self.assets.iter().filter(|a| a.id > after).filter(|a| !filter.selection || a.id % 2 == 0).collect();
        all.sort_by_key(|a| a.id);
        let more = all.len() > 200;
        all.truncate(200);
        let next_cursor = more.then(|| all.last().unwrap().id.to_string());
        Ok(AssetPage { items: all.into_iter().cloned().collect(), next_cursor })
    }
    fn details(&self, ids: &[u64]) -> Result<Vec<AssetDetails>, String> {
        assert!(ids.len() <= 200);
        Ok(self.details.iter().filter(|d| ids.contains(&d.id)).cloned().collect())
    }
}

impl ProposalsHost for Fake {
    fn submit(&self, _: &str, batch: Vec<NewProposal>) -> Result<(), String> {
        self.proposed.lock().unwrap().extend(batch);
        Ok(())
    }
}

fn wall(minutes_since_epoch: i64) -> String {
    let secs = minutes_since_epoch * 60;
    let (days, rest) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    // 2024-01-01 is day 19723.
    let d = days - 19_723;
    let (m, day) = if d < 31 { (1, d + 1) } else { (2, d - 30) };
    format!("2024-{m:02}-{day:02}T{:02}:{:02}:{:02}", rest / 3600, rest % 3600 / 60, rest % 60)
}

fn asset(id: u64, captured_at: Option<String>, camera: &str, offset: Option<i32>) -> Asset {
    Asset {
        id,
        captured_at,
        offset_minutes: offset,
        file_name: format!("{id}.jpg"),
        folder: None,
        kind: MediaKind::Photo,
        gps: offset.map(|_| Gps { lat: 46.0, lon: 14.0, source: GpsSource::Camera }),
        place: None,
        group: None,
        camera: Some(camera.into()),
    }
}

fn details(id: u64) -> AssetDetails {
    AssetDetails { id, capture_source: Some("original".into()), title: None, caption: None, keywords: vec![], scene: vec![], people: vec![], albums: vec![], text: vec![] }
}

fn run(name: &str, id: &str, granted: Vec<PluginPermission>, fake: Arc<Fake>, settings: serde_json::Map<String, serde_json::Value>, tool: &str, args: &str) -> Option<String> {
    let wasm = wasm(name)?;
    let dir = tempfile::tempdir().unwrap();
    let (d, f, plugin) = (dir.path().to_path_buf(), fake, id.to_string());
    let make = move || HostCtx {
        plugin: plugin.clone(),
        granted: granted.clone(),
        settings: settings.clone(),
        data_dir: d.join("data"),
        log: Arc::new(|l: &str| eprintln!("[{l}]")),
        cancel: None,
        chosen: None,
        photos: None,
        drafts: None,
        models: None,
        library: Some(f.clone() as Arc<dyn LibraryHost>),
        places: None,
        proposals: Some(f.clone() as Arc<dyn ProposalsHost>),
    };
    let engine = Engine::new(&dir.path().join("cache"), Arc::new(|_, _| {})).unwrap();
    let tools = engine.tools_at(id, &wasm, make).unwrap();
    assert_eq!(tools.list().unwrap().len(), 1);
    Some(tools.call(tool, args).unwrap())
}

/// A tiny deterministic generator.
struct Rng(u64);
impl Rng {
    fn below(&mut self, n: u64) -> i64 {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        ((self.0 >> 33) % n) as i64
    }
}

/// A phone (records its offset: trusted) and a camera whose clock runs 2 h 14 min ahead, over two
/// days: 40 phone shots a day, 30 camera shots a day of which 20 are the same moments.
fn clock_library() -> (Fake, Vec<(u64, String)>) {
    let mut rng = Rng(42);
    let start = 19_723 * 1440;
    let (mut assets, mut cam_walls) = (Vec::new(), Vec::new());
    let mut id = 1;
    for day in 0..2 {
        let base = start + day * 1440 + 540;
        let phone: Vec<i64> = (0..40).map(|_| base + rng.below(660)).collect();
        for &m in &phone {
            assets.push(asset(id, Some(wall(m)), "Pixel 8", Some(60)));
            id += 1;
        }
        for k in 0..30usize {
            let m = if k < 20 { phone[k] + rng.below(3) - 1 } else { base + rng.below(660) } + 134;
            assets.push(asset(id, Some(wall(m)), "Nikon Z6", None));
            cam_walls.push((id, wall(m)));
            id += 1;
        }
    }
    let details = assets.iter().map(|a| details(a.id)).collect();
    (Fake { assets, details, proposed: Mutex::new(Vec::new()) }, cam_walls)
}

#[test]
fn clockwork_proposes_the_corrected_dates_for_the_camera_that_is_off() {
    let (fake, cam) = clock_library();
    let fake = Arc::new(fake);
    let granted = vec![PluginPermission::LibraryRead, PluginPermission::DatesWrite];
    let Some(out) = run("clockwork", "jl.clockwork", granted, fake.clone(), Default::default(), "check_clocks", "{}") else {
        eprintln!("skipped: build plugins/first-party/clockwork first");
        return;
    };
    assert!(out.contains("\"status\":\"done\"") && out.contains("jl.clockwork"), "{out}");
    let got = fake.proposed.lock().unwrap();
    // Every camera photo (the phone never gets one), each 2 h 14 min earlier than it was taken.
    let mut ids: Vec<i64> = got.iter().map(|p| p.asset_id).collect();
    ids.sort_unstable();
    assert_eq!(ids, cam.iter().map(|(id, _)| *id as i64).collect::<Vec<_>>());
    for p in got.iter() {
        let (_, was) = cam.iter().find(|(id, _)| *id as i64 == p.asset_id).unwrap();
        let ProposalValue::Date(new) = &p.value else { panic!("not a date: {:?}", p.value) };
        // Compare in minutes of the month: the camera's wall clock minus 134 (±1 for the jitter's median).
        let m = |s: &str| i64::from(s[8..10].parse::<i32>().unwrap()) * 1440 + i64::from(s[11..13].parse::<i32>().unwrap()) * 60 + i64::from(s[14..16].parse::<i32>().unwrap());
        assert!((m(was) - m(new) - 134).abs() <= 1, "{was} -> {new}");
        assert!(p.confidence >= 0.7 && p.confidence <= 0.97, "{}", p.confidence);
        let why = p.reason.as_deref().unwrap();
        // The shared moments are jittered by a minute either way, so 2 h 14 or 2 h 15 is right.
        assert!(why.starts_with("Nikon Z6's clock looks 2 h 1") && why.contains(" min ahead of Pixel 8"), "{why}");
    }
}

#[test]
fn clockwork_leaves_alone_a_library_it_cannot_judge() {
    // No camera records its zone or GPS: nothing to trust, so nothing to compare with.
    let assets: Vec<Asset> = (1..=30).map(|i| asset(i, Some(wall(19_723 * 1440 + 600 + i as i64 * 7)), "Nikon Z6", None)).collect();
    let details = assets.iter().map(|a| details(a.id)).collect();
    let fake = Arc::new(Fake { assets, details, proposed: Mutex::new(Vec::new()) });
    let granted = vec![PluginPermission::LibraryRead, PluginPermission::DatesWrite];
    let Some(out) = run("clockwork", "jl.clockwork", granted, fake.clone(), Default::default(), "check_clocks", "{}") else {
        return;
    };
    assert!(out.contains("\"suggested\":0") && out.contains("whose clock it can trust"), "{out}");
    assert!(fake.proposed.lock().unwrap().is_empty());
}

#[test]
fn clockwork_ignores_dates_that_came_from_the_file_and_not_a_shutter() {
    let (mut fake, _) = clock_library();
    for d in &mut fake.details {
        d.capture_source = Some("file_time".into());
    }
    let fake = Arc::new(fake);
    let granted = vec![PluginPermission::LibraryRead, PluginPermission::DatesWrite];
    let Some(out) = run("clockwork", "jl.clockwork", granted, fake.clone(), Default::default(), "check_clocks", "{}") else {
        return;
    };
    assert!(out.contains("\"suggested\":0"), "{out}");
    assert!(fake.proposed.lock().unwrap().is_empty(), "a file's date says nothing about a clock");
}

#[test]
fn captioner_proposes_titles_and_captions_and_leaves_what_exists() {
    let mut a1 = asset(1, Some("2026-03-12T18:40:00".into()), "Pixel 8", Some(60));
    a1.place = Some("Idrija, Slovenia".into());
    let a2 = asset(2, Some("2026-03-13T09:00:00".into()), "Pixel 8", None);
    let a3 = asset(3, Some("2026-03-13T10:00:00".into()), "Pixel 8", None);
    let d1 = AssetDetails { people: vec!["Anna".into(), "Ben".into()], scene: vec!["sunset".into()], ..details(1) };
    let d2 = AssetDetails { title: Some("Mine".into()), scene: vec!["beach".into()], text: vec!["OPEN 24h".into()], ..details(2) };
    let d3 = details(3);
    let fake = Arc::new(Fake { assets: vec![a1, a2, a3], details: vec![d1, d2, d3], proposed: Mutex::new(Vec::new()) });
    let granted = vec![PluginPermission::LibraryRead, PluginPermission::CaptionsWrite, PluginPermission::Storage];
    let Some(out) = run("captioner", "jl.captioner", granted, fake.clone(), Default::default(), "suggest_captions", "{}") else {
        eprintln!("skipped: build plugins/first-party/captioner first");
        return;
    };
    assert!(out.contains("\"status\":\"done\"") && out.contains("\"suggested\":3"), "{out}");
    let got = fake.proposed.lock().unwrap();
    let find = |asset: i64, title: bool| got.iter().find(|p| p.asset_id == asset && matches!((&p.value, title), (ProposalValue::Title(_), true) | (ProposalValue::Caption(_), false)));
    let Some(NewProposal { value: ProposalValue::Title(t), confidence, .. }) = find(1, true) else { panic!("{got:?}") };
    assert_eq!((t.as_str(), *confidence), ("Anna and Ben in Idrija", 0.8));
    let Some(NewProposal { value: ProposalValue::Caption(c), .. }) = find(1, false) else { panic!() };
    assert_eq!(c, "Anna and Ben in Idrija, Slovenia. Thursday 12 March 2026, in the evening. Looks like sunset.");
    assert!(find(2, true).is_none(), "photo 2 has its own title");
    assert!(find(3, true).is_none() && find(3, false).is_none(), "nothing is known about photo 3");
    // photo 2: only a scene word and a sign: a caption (the title it has is kept), quoting the sign
    let Some(NewProposal { value: ProposalValue::Caption(c2), .. }) = find(2, false) else { panic!("{got:?}") };
    assert!(c2.starts_with("Beach. ") && c2.ends_with("Reads \u{201c}OPEN 24h\u{201d}."), "{c2}");
}

#[test]
fn captioner_settings_turn_parts_off() {
    let mut a1 = asset(1, Some("2026-03-12T18:40:00".into()), "Pixel 8", None);
    a1.place = Some("Idrija, Slovenia".into());
    let fake = Arc::new(Fake { assets: vec![a1], details: vec![details(1)], proposed: Mutex::new(Vec::new()) });
    let granted = vec![PluginPermission::LibraryRead, PluginPermission::CaptionsWrite, PluginPermission::Storage];
    let mut settings = serde_json::Map::new();
    settings.insert("captions".into(), serde_json::Value::Bool(false));
    let Some(_) = run("captioner", "jl.captioner", granted, fake.clone(), settings, "suggest_captions", r#"{"scope":"library"}"#) else {
        return;
    };
    let got = fake.proposed.lock().unwrap();
    assert_eq!(got.len(), 1);
    assert!(matches!(got[0].value, ProposalValue::Title(_)));
}

/// What the install sheet would check: the manifest parses (API 1.4, controls, contributions,
/// icons) and the wasm imports only what the permissions allow.
#[test]
fn both_packages_pass_the_install_checks() {
    use photos_plugins::host::ImportsCheck;
    use photos_plugins::package::{open_folder, KindCheck, Report};
    for (name, perms) in [("clockwork", vec!["library.read", "dates.write"]), ("captioner", vec!["library.read", "captions.write", "storage"])] {
        let Some(wasm) = wasm(name) else {
            eprintln!("skipped: build plugins/first-party/{name} first");
            continue;
        };
        // The folder as pack.mjs sees it: the manifest, the wasm and the listing files, not target/.
        let (src, dir) = (wasm.parent().unwrap(), tempfile::tempdir().unwrap());
        for file in ["plugin.toml", "plugin.wasm", "icon.png", "README.md"] {
            std::fs::copy(src.join(file), dir.path().join(file)).unwrap();
        }
        std::fs::create_dir(dir.path().join("screenshots")).unwrap();
        for e in std::fs::read_dir(src.join("screenshots")).unwrap() {
            let e = e.unwrap();
            std::fs::copy(e.path(), dir.path().join("screenshots").join(e.file_name())).unwrap();
        }
        let pkg = open_folder(dir.path()).unwrap();
        let granted: Vec<String> = pkg.manifest.permissions.iter().map(|p| p.id().to_string()).collect();
        assert_eq!(granted, perms, "{name}");
        let mut report = Report::default();
        ImportsCheck.check(&pkg, true, &mut report);
        assert!(report.problems.is_empty(), "{name}: {:?}", report.problems);
    }
}
