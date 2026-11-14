//! Limits and instances (Round 23 A2 Task 5): 30 s per call with `progress` buying more time
//! and cancel stopping a loop, 512 MB per instance, 3 trips stop a plugin, the shared tools
//! instance runs one call at a time and is dropped when idle, and every call runs on the
//! `lumen-plugins` pool.
//!
//! The per-call limit is a process-wide setting: the default run uses 1 s; the 30 s case is
//! `#[ignore]` (run it alone with `--ignored`).

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use photos_plugins::host::{limits, ChosenRoot, Engine, HostCtx};
use photos_plugins::{Pick, PluginPermission};

const ID: &str = "com.test.x";

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("tests/wasm/{name}.wasm"))
}

type Stops = Arc<Mutex<Vec<(String, String)>>>;
type Lines = Arc<Mutex<Vec<String>>>;

fn engine(dir: &Path) -> (Engine, Stops) {
    let stops: Stops = Arc::default();
    let s = stops.clone();
    let e = Engine::new(&dir.join("cache"), Arc::new(move |id: &str, why: String| s.lock().unwrap().push((id.into(), why))))
        .unwrap();
    (e, stops)
}

fn ctx(dir: &Path, granted: Vec<PluginPermission>, settings: serde_json::Value) -> (HostCtx, Lines) {
    let lines: Lines = Arc::default();
    let sink = lines.clone();
    let ctx = HostCtx {
        plugin: ID.into(),
        granted,
        settings: settings.as_object().cloned().unwrap_or_default(),
        data_dir: dir.join("data"),
        log: Arc::new(move |l: &str| sink.lock().unwrap().push(format!("{l} @{}", std::thread::current().name().unwrap_or("?")))),
        cancel: None,
        chosen: None,
        photos: None,
        drafts: None,
    };
    (ctx, lines)
}

fn chosen_dir(dir: &Path) -> ChosenRoot {
    let root = dir.join("chosen");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("a.jpg"), b"jpegbytes").unwrap();
    ChosenRoot::new(&root, Pick::Folder).unwrap()
}

fn importer_perms() -> Vec<PluginPermission> {
    vec![PluginPermission::ImportReadChosen]
}

fn tools_perms() -> Vec<PluginPermission> {
    vec![PluginPermission::PhotosRead, PluginPermission::DevelopWriteDrafts, PluginPermission::Storage]
}

fn endless_within(secs: u64, lo: f64, hi: f64) {
    limits::set_call_secs_for_tests(secs);
    let dir = tempfile::tempdir().unwrap();
    let (e, _) = engine(dir.path());
    let (c, _) = ctx(dir.path(), importer_perms(), serde_json::json!({}));
    let mut inst = e.importer_at(&fixture("endless"), c).unwrap();
    let t = Instant::now();
    let err = inst.enumerate().unwrap_err();
    let took = t.elapsed().as_secs_f64();
    assert_eq!(err.code, "plugin_timeout", "{err}");
    assert!((lo..hi).contains(&took), "took {took:.2} s");
    assert_eq!(e.trips(ID), 1);
}

#[test]
fn endless_times_out() {
    endless_within(1, 0.9, 2.5);
}

#[test]
#[ignore = "takes 30 s; run alone with --ignored"]
fn endless_times_out_at_30s() {
    endless_within(30, 30.0, 32.0);
}

#[test]
fn memory_hog_traps() {
    limits::set_call_secs_for_tests(1);
    let dir = tempfile::tempdir().unwrap();
    let (e, stops) = engine(dir.path());
    let (c, _) = ctx(dir.path(), importer_perms(), serde_json::json!({}));
    let mut inst = e.importer_at(&fixture("memory_hog"), c).unwrap();
    let err = inst.enumerate().unwrap_err();
    assert_eq!(err.code, "plugin_trap", "{err}");
    assert_eq!(e.trips(ID), 1);
    assert_eq!(e.stopped(ID), None);
    assert!(stops.lock().unwrap().is_empty());
}

#[test]
fn trap_counted() {
    limits::set_call_secs_for_tests(1);
    let dir = tempfile::tempdir().unwrap();
    let (e, stops) = engine(dir.path());
    let (c, lines) = ctx(dir.path(), importer_perms(), serde_json::json!({}));
    for _ in 0..3 {
        let (c, _) = ctx(dir.path(), importer_perms(), serde_json::json!({}));
        let mut inst = e.importer_at(&fixture("trap"), c).unwrap();
        assert_eq!(inst.enumerate().unwrap_err().code, "plugin_trap");
    }
    assert_eq!(e.trips(ID), 3);
    assert_eq!(e.stopped(ID).as_deref(), Some("Stopped working"));
    assert_eq!(*stops.lock().unwrap(), vec![(ID.to_string(), "Stopped working".to_string())]);

    // A new instance is refused without running (no new trip).
    let err = e.importer_at(&fixture("trap"), c).err().expect("refused");
    assert!(err.message.contains("stopped working"), "{err}");
    assert_eq!(e.trips(ID), 3);
    assert_eq!(stops.lock().unwrap().len(), 1);
    drop(lines);
}

#[test]
fn trapped_instance_is_spent_not_tripped_again() {
    // wasmtime refuses to enter an instance after a trap; the import's later calls must not
    // count that as more trips (one bad file would otherwise stop the plugin at once).
    limits::set_call_secs_for_tests(1);
    let dir = tempfile::tempdir().unwrap();
    let (e, _) = engine(dir.path());
    let (c, _) = ctx(dir.path(), importer_perms(), serde_json::json!({}));
    let mut inst = e.importer_at(&fixture("trap"), c).unwrap();
    let first = inst.enumerate().unwrap_err();
    assert_eq!(first.code, "plugin_trap");
    assert_eq!(inst.read("a", 0, 4).unwrap_err(), first);
    assert!(inst.extras("a").is_err());
    assert!(inst.sidecars("a").is_err());
    assert_eq!(e.trips(ID), 1);
    assert_eq!(e.stopped(ID), None);
}

#[test]
fn wasi_sleep_times_out() {
    // A guest blocked in a WASI sleep runs no guest code, so epochs can't stop it; the host
    // ends the sleep at the call's limit and the next tick times the call out.
    limits::set_call_secs_for_tests(1);
    let dir = tempfile::tempdir().unwrap();
    let (e, _) = engine(dir.path());
    let (c, _) = ctx(dir.path(), importer_perms(), serde_json::json!({ "mode": "sleep" }));
    let mut inst = e.importer_at(&fixture("importer_ok"), c).unwrap();
    let t = Instant::now();
    let err = inst.enumerate().unwrap_err();
    assert_eq!(err.code, "plugin_timeout", "{err}");
    assert!(t.elapsed() < Duration::from_millis(2500), "{:?}", t.elapsed());
    assert_eq!(e.trips(ID), 1);
}

#[test]
fn guest_error_is_not_a_trip() {
    limits::set_call_secs_for_tests(1);
    let dir = tempfile::tempdir().unwrap();
    let (e, _) = engine(dir.path());
    let (c, _) = ctx(dir.path(), importer_perms(), serde_json::json!({ "mode": "setting" }));
    let mut inst = e.importer_at(&fixture("importer_ok"), c).unwrap();
    for _ in 0..4 {
        assert_eq!(inst.enumerate().unwrap_err().code, "plugin_invalid");
    }
    assert_eq!(e.trips(ID), 0);
    assert_eq!(e.stopped(ID), None);
}

#[test]
fn progress_resets_deadline() {
    limits::set_call_secs_for_tests(1);
    let dir = tempfile::tempdir().unwrap();
    let (e, _) = engine(dir.path());
    let (mut c, _) = ctx(dir.path(), importer_perms(), serde_json::json!({ "mode": "progress", "secs": 3 }));
    c.chosen = Some(chosen_dir(dir.path()));
    let mut inst = e.importer_at(&fixture("importer_ok"), c).unwrap();
    let t = Instant::now();
    assert!(inst.enumerate().unwrap().is_empty());
    assert!(t.elapsed() >= Duration::from_millis(2900));
    assert_eq!(e.trips(ID), 0);
}

#[test]
fn cancel_stops_progress_loop() {
    limits::set_call_secs_for_tests(1);
    let dir = tempfile::tempdir().unwrap();
    let (e, _) = engine(dir.path());
    let (mut c, _) = ctx(dir.path(), importer_perms(), serde_json::json!({ "mode": "progress-forever" }));
    let cancel = Arc::new(AtomicBool::new(false));
    c.cancel = Some(cancel.clone());
    c.chosen = Some(chosen_dir(dir.path()));
    let mut inst = e.importer_at(&fixture("importer_ok"), c).unwrap();
    let flip = cancel.clone();
    let t = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(1500));
        flip.store(true, Ordering::Relaxed);
    });
    // Runs past the 1 s limit on progress alone, then stops on cancel with the host's text.
    let err = inst.enumerate().unwrap_err();
    t.join().unwrap();
    assert_eq!(err.message, "cancelled", "{err}");
    assert_eq!(e.trips(ID), 0);
}

#[test]
fn cancel_interrupts_a_loop() {
    // Cancel lands at 0.3 s, well before the 1 s limit: the text says which one stopped it.
    limits::set_call_secs_for_tests(1);
    let dir = tempfile::tempdir().unwrap();
    let (e, _) = engine(dir.path());
    let (mut c, _) = ctx(dir.path(), importer_perms(), serde_json::json!({}));
    let cancel = Arc::new(AtomicBool::new(false));
    c.cancel = Some(cancel.clone());
    let mut inst = e.importer_at(&fixture("endless"), c).unwrap();
    let flip = cancel.clone();
    let t = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(300));
        flip.store(true, Ordering::Relaxed);
    });
    let start = Instant::now();
    let err = inst.enumerate().unwrap_err();
    t.join().unwrap();
    assert!(start.elapsed() < Duration::from_millis(900), "{:?}", start.elapsed());
    assert_eq!(err.message, "cancelled", "{err}");
    assert_eq!(e.trips(ID), 0);
}

fn tools_engine(dir: &Path) -> (Engine, Arc<AtomicUsize>, Lines) {
    let (e, _) = engine(dir);
    let built = Arc::new(AtomicUsize::new(0));
    let (_, lines) = ctx(dir, vec![], serde_json::json!({}));
    (e, built, lines)
}

fn ctx_for(dir: &Path, built: Arc<AtomicUsize>, lines: Lines) -> impl Fn() -> HostCtx + Send + Sync + 'static {
    let dir = dir.to_path_buf();
    move || {
        built.fetch_add(1, Ordering::SeqCst);
        let (mut c, _) = ctx(&dir, tools_perms(), serde_json::json!({}));
        let sink = lines.clone();
        c.log = Arc::new(move |l: &str| sink.lock().unwrap().push(format!("{l} @{}", std::thread::current().name().unwrap_or("?"))));
        c
    }
}

#[test]
fn tools_list_and_call() {
    let dir = tempfile::tempdir().unwrap();
    let (e, built, lines) = tools_engine(dir.path());
    let tools = e.tools_at(ID, &fixture("tools_ok"), ctx_for(dir.path(), built, lines)).unwrap();
    let names: Vec<String> = tools.list().unwrap().into_iter().map(|t| t.name).collect();
    assert_eq!(names, ["echo", "record"]);
    assert_eq!(tools.call("echo", "{\"a\":1}").unwrap(), "{\"args\":{\"a\":1},\"scope\":0}");
    // The same plugin gets the same shared instance.
    let again = e.tools_at(ID, &fixture("tools_ok"), || unreachable!("cached")).unwrap();
    assert!(Arc::ptr_eq(&tools, &again));
}

#[test]
fn tools_calls_serialised() {
    let dir = tempfile::tempdir().unwrap();
    let (e, built, lines) = tools_engine(dir.path());
    let tools = e.tools_at(ID, &fixture("tools_ok"), ctx_for(dir.path(), built, lines)).unwrap();
    let a = tools.clone();
    let b = tools.clone();
    let ta = std::thread::spawn(move || a.call("record", "{}").unwrap());
    let tb = std::thread::spawn(move || b.call("record", "{}").unwrap());
    ta.join().unwrap();
    tb.join().unwrap();
    let calls = std::fs::read_to_string(dir.path().join("data/calls")).unwrap();
    assert_eq!(calls, "in;out;in;out;");
}

#[test]
fn idle_instance_dropped() {
    photos_plugins::host::pool::set_idle_secs_for_tests(1);
    let dir = tempfile::tempdir().unwrap();
    let (e, built, lines) = tools_engine(dir.path());
    let tools = e.tools_at(ID, &fixture("tools_ok"), ctx_for(dir.path(), built.clone(), lines)).unwrap();
    tools.call("echo", "{}").unwrap();
    tools.call("echo", "{}").unwrap();
    assert_eq!(built.load(Ordering::SeqCst), 1);
    assert!(tools.is_live());
    std::thread::sleep(Duration::from_millis(2500));
    assert!(!tools.is_live(), "dropped while idle, before any new call");
    tools.call("echo", "{}").unwrap();
    assert_eq!(built.load(Ordering::SeqCst), 2);
    photos_plugins::host::pool::set_idle_secs_for_tests(60);
}

#[test]
fn calls_run_on_plugin_pool() {
    limits::set_call_secs_for_tests(1);
    let dir = tempfile::tempdir().unwrap();
    let (e, built, lines) = tools_engine(dir.path());
    let tools = e.tools_at(ID, &fixture("tools_ok"), ctx_for(dir.path(), built, lines.clone())).unwrap();
    tools.call("echo", "{}").unwrap();
    let (mut c, ilines) = ctx(dir.path(), importer_perms(), serde_json::json!({}));
    c.chosen = Some(chosen_dir(dir.path()));
    e.importer_at(&fixture("importer_ok"), c).unwrap().enumerate().unwrap();
    for l in lines.lock().unwrap().iter().chain(ilines.lock().unwrap().iter()) {
        let thread = l.rsplit(" @").next().unwrap();
        assert!(thread.starts_with("lumen-plugins-"), "{l}");
    }
    assert!(!lines.lock().unwrap().is_empty() && !ilines.lock().unwrap().is_empty());
}
