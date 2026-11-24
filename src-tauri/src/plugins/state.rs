//! The loaded plugin store in app state (Round 23; A1 minimal, C1 adds events, listeners, the
//! log and the pending package). Managed as `Arc<Plugins>`.

use std::collections::{HashMap, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock, RwLock, RwLockReadGuard, RwLockWriteGuard};

use tauri::{AppHandle, Emitter};

use photos_plugins::host::Engine;
use photos_plugins::signature::Keys;
use photos_plugins::store::{Ctx, Store};
use photos_plugins::{PluginError, PluginState};

/// The JL Studios plugin public key, baked in by `build.rs` from the tracked
/// `src-tauri/plugin-pubkey.txt` (`LUMEN_PLUGIN_PUBKEY` overrides it). `None`: no package is *From JL Studios*.
pub const JL_PUBKEY: Option<&str> = option_env!("LUMEN_PLUGIN_PUBKEY");

/// A plugin file the protocol may serve, and whether it is a Developer-mode folder read in place.
pub struct Served {
    pub path: PathBuf,
    pub dev: bool,
}

/// `%XX` decoded once; `None` for a bad escape, a NUL or bytes that aren't UTF-8. Decoding once
/// is the point: `%252e` stays `%2e`, which names no file.
pub(crate) fn percent_decode(s: &str) -> Option<String> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' {
            let hex = s.get(i + 1..i + 3)?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    let s = String::from_utf8(out).ok()?;
    (!s.contains('\0')).then_some(s)
}

/// The event a running Lumen sends when another `.lumenplugin` is opened: `{ path }`.
pub const OPEN_EVENT: &str = "plugins://open-package";

#[derive(Clone, serde::Serialize)]
struct OpenPackage {
    path: String,
}

/// A package Lumen opens from the system: a plugin or (Round 26) a theme, whatever the case.
fn is_package(p: &std::path::Path) -> bool {
    p.extension().is_some_and(|e| e.eq_ignore_ascii_case("lumenplugin") || e.eq_ignore_ascii_case("lumentheme"))
}

/// The first argument (after the program) that is an existing `.lumenplugin` or `.lumentheme` file, whatever the
/// case of its extension: what Windows and Linux pass when one is double-clicked.
pub fn package_arg(args: &[String]) -> Option<PathBuf> {
    args.iter().skip(1).map(PathBuf::from).find(|p| is_package(p) && p.is_file())
}

/// A second launch's command line, in the running Lumen: the page opens the install sheet. The
/// path is kept as well as sent, since this Lumen's page may not be listening yet (two files
/// opened at once start one process and forward the other); the page drains it after opening.
pub fn forward_package(app: &AppHandle, args: &[String]) {
    // The state is built in `setup`, after this callback can first run.
    let path = match tauri::Manager::try_state::<Arc<Plugins>>(app) {
        Some(plugins) => plugins.keep_package_arg(args),
        None => package_arg(args),
    };
    if let Some(path) = path {
        let _ = app.emit(OPEN_EVENT, OpenPackage { path: path.display().to_string() });
    }
}

/// macOS hands a double-clicked file over as an event, before or after the page is up: keep it
/// for the page's first ask and tell a page that is already listening.
#[cfg(target_os = "macos")]
pub fn on_opened(app: &AppHandle, plugins: &Plugins, urls: &[tauri::Url]) {
    for path in urls.iter().filter_map(|u| u.to_file_path().ok()) {
        if is_package(&path) && path.is_file() {
            plugins.set_pending(path.clone());
            let _ = app.emit(OPEN_EVENT, OpenPackage { path: path.display().to_string() });
        }
    }
}

/// The event the page refetches its plugin list on (no payload).
pub const CHANGED_EVENT: &str = "plugins://changed";

/// Lines kept in memory per plugin.
pub const LOG_RING: usize = 200;
/// A line is cut here (bytes), so 200 lines can never hold more than about 800 KB.
const LOG_LINE_MAX: usize = 4096;
/// How many of a plugin's lines Lumen's own log gets in a session; the plugin's ring keeps
/// every line. A flooding plugin can't push Lumen's diagnostics out of the rotated log files.
const LOG_TRACED: usize = 500;

/// One plugin's last [`LOG_RING`] lines.
#[derive(Default)]
struct LogRing {
    lines: VecDeque<String>,
    total: usize,
}

/// `line` as one bounded line: no line breaks (a plugin must not forge Lumen's log lines), cut at
/// [`LOG_LINE_MAX`] bytes on a character boundary and marked.
fn bounded(line: &str) -> String {
    let one: String = line.chars().map(|c| if c == '\n' || c == '\r' { ' ' } else { c }).collect();
    if one.len() <= LOG_LINE_MAX {
        return one;
    }
    let mut end = LOG_LINE_MAX;
    while !one.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &one[..end])
}

impl LogRing {
    /// Keeps `line` and returns what Lumen's own log should record for it, if anything.
    fn push(&mut self, line: &str) -> Option<String> {
        let line = bounded(line);
        if self.lines.len() == LOG_RING {
            self.lines.pop_front();
        }
        self.lines.push_back(line.clone());
        self.total += 1;
        match self.total {
            n if n <= LOG_TRACED => Some(line),
            n if n == LOG_TRACED + 1 => Some("more lines stay in the plugin's own log".to_string()),
            _ => None,
        }
    }
}

type Listener = Box<dyn Fn() + Send + Sync>;

/// Who hears that the plugin list changed: the page (an event) and in-process listeners
/// (the MCP server, the menus). Shared with the engine's `on_stopped`, which has no `AppHandle`
/// argument, so `attach` leaves a closure that emits. The emit sits behind that closure so unit
/// tests that reach `engine()` don't link the whole Tauri runtime (a Windows test exe has no
/// common-controls manifest and fails to load, STATUS_ENTRYPOINT_NOT_FOUND).
#[derive(Default)]
struct Notifier {
    emit: OnceLock<Box<dyn Fn() + Send + Sync>>,
    listeners: Mutex<Vec<Listener>>,
    /// Listeners that last a while (one per plugin import): each says whether to keep it.
    passing: Mutex<Vec<Box<dyn Fn() -> bool + Send + Sync>>>,
}

impl Notifier {
    fn run_listeners(&self) {
        for f in self.listeners.lock().unwrap_or_else(|e| e.into_inner()).iter() {
            f();
        }
        self.passing.lock().unwrap_or_else(|e| e.into_inner()).retain(|f| f());
    }

    /// From a thread that has no `AppHandle` of its own (a plugin that stopped).
    fn fire(&self) {
        if let Some(emit) = self.emit.get() {
            emit();
        }
        self.run_listeners();
    }
}

pub struct Plugins {
    /// `<app>/cache/plugins/`, for the engine's compiled modules.
    cache: PathBuf,
    /// `<app>/plugins/store-cache/`: the store's icons and screenshots, which `store-image/` serves.
    store_images: PathBuf,
    store: Arc<RwLock<Store>>,
    /// Built on first use, never at boot.
    engine: OnceLock<Arc<Engine>>,
    notifier: Arc<Notifier>,
    /// Last [`LOG_RING`] lines per plugin id.
    logs: Arc<Mutex<HashMap<String, LogRing>>>,
    /// The `.lumenplugin` a cold start was opened with, until the page asks for it.
    pending: Mutex<Option<PathBuf>>,
    /// Round 24 lane W: what reaches the open library, set by [`Plugins::attach`]. Held as trait
    /// objects so code that tests reach never names the app handle (Tauri's window code needs a
    /// manifest a test executable doesn't have).
    live: OnceLock<super::hosts::Live>,
    /// Round 24 lane W: every plugin's Place review suggestions.
    reviews: Arc<super::places::ReviewStore>,
}

impl Plugins {
    /// Sweeps `removed/`, then opens `<app_dir>/plugins/`. Never fails: a `plugins.json` that
    /// can't be read is set aside as `plugins.json.broken` and Lumen starts with no plugins.
    ///
    /// `removed/` is swept on a background thread, never on the boot path.
    pub fn load(app_dir: &Path) -> Plugins {
        Self::load_with_sweep(app_dir, |dir| Store::sweep_removed(&dir))
    }

    /// [`Plugins::load`] with the sweep chosen by the caller (a test's slow one).
    pub fn load_with_sweep(app_dir: &Path, sweep: impl FnOnce(PathBuf) + Send + 'static) -> Plugins {
        let dir = app_dir.join("plugins");
        let sweep_dir = dir.clone();
        let spawned = std::thread::Builder::new().name("lumen-plugin-sweep".into()).spawn(move || sweep(sweep_dir));
        if let Err(e) = spawned {
            tracing::warn!(error = %e, "the plugin sweep couldn't start; removed plugins stay until next start");
        }
        let store = Store::open(&dir).unwrap_or_else(|e| {
            tracing::error!(error = %e, "plugins.json couldn't be read; starting with no plugins");
            let state = dir.join("plugins.json");
            if state.exists() {
                let _ = fs::rename(&state, dir.join("plugins.json.broken"));
            }
            Store::open(&dir).unwrap_or_else(|e| {
                tracing::error!(error = %e, "the plugins folder couldn't be opened");
                Store::empty(&dir)
            })
        });
        Plugins {
            cache: app_dir.join("cache").join("plugins"),
            store_images: super::store::store_cache_dir(&dir),
            store: Arc::new(RwLock::new(store)),
            engine: OnceLock::new(),
            notifier: Arc::new(Notifier::default()),
            logs: Arc::new(Mutex::new(HashMap::new())),
            pending: Mutex::new(None),
            live: OnceLock::new(),
            reviews: Arc::new(super::places::ReviewStore::default()),
        }
    }

    /// The hosts that reach the library, once [`Plugins::attach`] has run (never in a unit test).
    pub fn live(&self) -> Option<&super::hosts::Live> {
        self.live.get()
    }

    /// The Place review suggestions of every plugin.
    pub fn reviews(&self) -> &Arc<super::places::ReviewStore> {
        &self.reviews
    }

    /// Gives the plugins the app handle, so a plugin that stops on its own (no command running)
    /// still reaches the page.
    pub fn attach(&self, app: AppHandle) {
        let _ = self.live.set(super::hosts::live(app.clone()));
        let _ = self.notifier.emit.set(Box::new(move || {
            let _ = app.emit(CHANGED_EVENT, ());
        }));
    }

    /// Tells the page and every listener that the list, a state or a setting changed.
    pub fn changed(&self, app: &AppHandle) {
        let _ = app.emit(CHANGED_EVENT, ());
        self.notifier.run_listeners();
    }

    /// Marks a plugin *Problem* (A4: its tools don't match its manifest) and tells the page and
    /// every listener. Call it with no lock on the store held.
    pub fn mark_problem(&self, id: &str, reason: &str) {
        self.write().mark_problem(id, reason.to_string());
        self.notifier.fire();
    }

    pub fn on_change(&self, f: Listener) {
        self.notifier.listeners.lock().unwrap_or_else(|e| e.into_inner()).push(f);
    }

    /// Sets `cancel` as soon as plugin `id` is turned off or removed, so an import through it
    /// ends. A plugin that stopped working is left alone: its import already fails each item
    /// with the reason. The listener holds `cancel` weakly and is dropped once the import is
    /// over or cancelled.
    pub fn cancel_when_off(&self, id: &str, cancel: &Arc<AtomicBool>) {
        let (store, id, weak) = (Arc::clone(&self.store), id.to_string(), Arc::downgrade(cancel));
        self.notifier.passing.lock().unwrap_or_else(|e| e.into_inner()).push(Box::new(move || {
            let Some(cancel) = weak.upgrade() else { return false };
            let store = store.read().unwrap_or_else(|e| e.into_inner());
            let stopped = matches!(store.get(&id).map(|i| &i.info.state), Some(PluginState::Stopped { .. }));
            let on = store.enabled().iter().any(|i| i.info.id == id);
            if on || stopped {
                return true;
            }
            cancel.store(true, Ordering::SeqCst);
            false
        }));
    }

    #[cfg(test)]
    pub(crate) fn passing_listeners(&self) -> usize {
        self.notifier.passing.lock().unwrap_or_else(|e| e.into_inner()).len()
    }

    /// What `changed` does besides the page's event: tests in other modules call it.
    #[cfg(test)]
    pub(crate) fn run_listeners(&self) {
        self.notifier.run_listeners();
    }

    /// Where a plugin's `host.log` and WASI output go: Lumen's log (`plugin=<id>`) and the
    /// plugin's own ring of the last [`LOG_RING`] lines.
    pub fn log_sink(&self, id: &str) -> Arc<dyn Fn(&str) + Send + Sync> {
        let id = id.to_string();
        let logs = Arc::clone(&self.logs);
        Arc::new(move |line: &str| {
            let traced = {
                let mut logs = logs.lock().unwrap_or_else(|e| e.into_inner());
                logs.entry(id.clone()).or_default().push(line)
            };
            if let Some(line) = traced {
                tracing::info!(plugin = %id, "{line}");
            }
        })
    }

    pub fn log_lines(&self, id: &str) -> Vec<String> {
        let logs = self.logs.lock().unwrap_or_else(|e| e.into_inner());
        logs.get(id).map(|r| r.lines.iter().cloned().collect()).unwrap_or_default()
    }

    /// The `.lumenplugin` in a launch's command line, if any, kept for the page's first ask.
    pub fn keep_package_arg(&self, args: &[String]) -> Option<PathBuf> {
        let path = package_arg(args)?;
        self.set_pending(path.clone());
        Some(path)
    }

    /// A plugin's `icon.png`, whether the plugin is on or off (Settings lists both).
    pub fn serve_icon(&self, id: &str) -> Option<Served> {
        let store = self.read();
        let path = store.icon_path(id)?;
        Some(Served { path, dev: store.get(id)?.info.dev })
    }

    /// The icon of a package the install sheet is showing, by the hash `inspect` gave it.
    pub fn serve_preview(&self, hash: &str) -> Option<Served> {
        Some(Served { path: self.read().preview_icon_path(hash)?, dev: false })
    }

    /// An image the plugin store's index lists, downloaded by Rust and named by the hash of its URL.
    pub fn serve_store_image(&self, hash: &str) -> Option<Served> {
        Some(Served { path: super::store::find_cached(&self.store_images, hash)?, dev: false })
    }

    /// The `n`th `[listing]` screenshot, on or off like the icon: only a listed file is named.
    pub fn serve_screenshot(&self, id: &str, n: usize) -> Option<Served> {
        let store = self.read();
        let path = store.screenshot_path(id, n)?;
        Some(Served { path, dev: store.get(id)?.info.dev })
    }

    /// A file the manifest declares under `[[assets]]`, only while the plugin is on and ready.
    /// `encoded` is the route's path as the page wrote it; the store compares the decoded text
    /// to its declared list, so no spelling of `..` can name another file.
    pub fn serve_asset(&self, id: &str, encoded: &str) -> Option<Served> {
        let rel = percent_decode(encoded)?;
        let store = self.read();
        if !store.enabled().iter().any(|i| i.info.id == id) {
            return None;
        }
        let path = store.asset_path(id, &rel)?;
        Some(Served { path, dev: store.get(id)?.info.dev })
    }

    /// The `.lumenplugin` this launch was opened with, kept until the page asks (an event sent
    /// during boot would fire before the page listens).
    pub fn set_pending(&self, path: PathBuf) {
        *self.pending.lock().unwrap_or_else(|e| e.into_inner()) = Some(path);
    }

    pub fn take_pending(&self) -> Option<PathBuf> {
        self.pending.lock().unwrap_or_else(|e| e.into_inner()).take()
    }

    pub fn read(&self) -> RwLockReadGuard<'_, Store> {
        self.store.read().unwrap_or_else(|e| e.into_inner())
    }

    pub fn write(&self) -> RwLockWriteGuard<'_, Store> {
        self.store.write().unwrap_or_else(|e| e.into_inner())
    }

    /// What the store's checks need: this Lumen's version, the built-in JL key and the
    /// binary's kind checkers.
    pub fn ctx(&self) -> Ctx {
        let keys = Keys::from_base64(JL_PUBKEY).unwrap_or_else(|e| {
            tracing::error!(error = %e, "the built-in plugin key isn't valid; no package is From JL Studios");
            Keys { jl: None }
        });
        Ctx {
            lumen: env!("CARGO_PKG_VERSION").parse().expect("Lumen's version is semver"),
            keys,
            checks: super::data::checks(),
        }
    }

    /// The code-plugin engine, built the first time a code plugin is used.
    pub fn engine(&self) -> Result<Arc<Engine>, PluginError> {
        if let Some(e) = self.engine.get() {
            return Ok(e.clone());
        }
        let store = Arc::clone(&self.store);
        let notifier = Arc::clone(&self.notifier);
        let on_stopped: photos_plugins::host::OnStopped = Arc::new(move |id: &str, reason: String| {
            store.write().unwrap_or_else(|e| e.into_inner()).mark_stopped(id, reason);
            notifier.fire();
        });
        let built = Arc::new(Engine::new(&self.cache, on_stopped)?);
        Ok(self.engine.get_or_init(|| built).clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    #[test]
    fn jl_pubkey_is_baked_into_a_clean_build() {
        // The key is a tracked file (src-tauri/plugin-pubkey.txt), so a build with no environment
        // still carries it; it must also parse as a key.
        let keys = Keys::from_base64(JL_PUBKEY).expect("the baked-in plugin key parses");
        assert!(keys.jl.is_some(), "no plugin public key was baked in");
    }

    #[test]
    fn load_without_plugins_json() {
        let t = tempfile::tempdir().unwrap();
        let p = Plugins::load(t.path());
        assert!(p.read().list().is_empty());
        assert!(!p.read().developer_mode());
    }

    #[test]
    fn load_broken_json_never_fails_boot() {
        let t = tempfile::tempdir().unwrap();
        let dir = t.path().join("plugins");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("plugins.json"), b"{ not json").unwrap();
        let p = Plugins::load(t.path());
        assert!(p.read().list().is_empty());
        assert_eq!(fs::read(dir.join("plugins.json.broken")).unwrap(), b"{ not json");
        assert!(!dir.join("plugins.json").exists());
    }

    #[test]
    fn boot_budget() {
        let t = tempfile::tempdir().unwrap();
        let dir = t.path().join("plugins");
        let mut plugins = serde_json::Map::new();
        for n in 0..50 {
            let id = format!("com.test.p{n}");
            let folder = dir.join("installed").join(&id);
            fs::create_dir_all(&folder).unwrap();
            fs::write(
                folder.join("plugin.toml"),
                format!(
                    "id = \"{id}\"\nname = \"P{n}\"\nversion = \"1.0.0\"\nauthor = \"T\"\ndescription = \"d\"\nlicense = \"MIT\"\nlumen = \">=26.0.0\"\napi = 1\n"
                ),
            )
            .unwrap();
            plugins.insert(
                id,
                serde_json::json!({
                    "version": "1.0.0", "enabled": true, "granted": [], "key": "A1B2C3D4E5F60708",
                    "trust": {"type": "author", "name": "T", "fingerprint": "A1B2-C3D4"},
                    "installedAt": "2026-11-14T00:00:00Z", "dev": false, "settings": {}
                }),
            );
        }
        fs::write(dir.join("plugins.json"), serde_json::to_vec(&serde_json::json!({ "plugins": plugins })).unwrap())
            .unwrap();
        let mut times: Vec<_> = (0..5)
            .map(|_| {
                let start = std::time::Instant::now();
                let p = Plugins::load(t.path());
                let took = start.elapsed();
                assert_eq!(p.read().list().len(), 50);
                took
            })
            .collect();
        times.sort();
        assert!(times[2] < std::time::Duration::from_millis(20), "median boot {:?}", times[2]);
    }

    #[test]
    fn engine_not_built_at_load() {
        let t = tempfile::tempdir().unwrap();
        let p = Plugins::load(t.path());
        assert!(p.engine.get().is_none());
        // Built on first use (A2 replaced A1's stub), then the same engine every time.
        let e = p.engine().unwrap();
        assert!(Arc::ptr_eq(&e, &p.engine().unwrap()));
    }

    #[test]
    fn ctx_has_the_binarys_checks_and_version() {
        let t = tempfile::tempdir().unwrap();
        let c = Plugins::load(t.path()).ctx();
        assert_eq!(c.lumen.to_string(), env!("CARGO_PKG_VERSION"));
        assert_eq!(c.keys.jl.is_some(), JL_PUBKEY.is_some());
    }

    #[test]
    fn listeners_run_on_change() {
        let t = tempfile::tempdir().unwrap();
        let p = Plugins::load(t.path());
        let hits = Arc::new(AtomicUsize::new(0));
        for _ in 0..2 {
            let h = hits.clone();
            p.on_change(Box::new(move || {
                h.fetch_add(1, Ordering::SeqCst);
            }));
        }
        p.run_listeners();
        assert_eq!(hits.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn log_ring_capped() {
        let t = tempfile::tempdir().unwrap();
        let p = Plugins::load(t.path());
        let sink = p.log_sink("com.test.a");
        for n in 0..10_000 {
            sink(&format!("line {n}"));
        }
        let lines = p.log_lines("com.test.a");
        assert_eq!(lines.len(), LOG_RING);
        assert_eq!(lines.first().unwrap(), "line 9800");
        assert_eq!(lines.last().unwrap(), "line 9999");
        assert!(p.log_lines("com.test.other").is_empty());
    }

    #[test]
    fn log_sink_tags_lines() {
        let t = tempfile::tempdir().unwrap();
        let p = Plugins::load(t.path());
        p.log_sink("com.test.a")("hello");
        p.log_sink("com.test.b")("other");
        assert_eq!(p.log_lines("com.test.a"), vec!["hello".to_string()]);
        assert_eq!(p.log_lines("com.test.b"), vec!["other".to_string()]);
    }

    #[test]
    fn sweep_runs_off_boot() {
        let t = tempfile::tempdir().unwrap();
        let done = Arc::new(AtomicBool::new(false));
        let d = done.clone();
        let start = std::time::Instant::now();
        let p = Plugins::load_with_sweep(t.path(), move |_| {
            std::thread::sleep(std::time::Duration::from_millis(200));
            d.store(true, Ordering::SeqCst);
        });
        assert!(start.elapsed() < std::time::Duration::from_millis(150), "load waited for the sweep");
        assert!(!done.load(Ordering::SeqCst));
        assert!(p.read().list().is_empty());
        for _ in 0..100 {
            if done.load(Ordering::SeqCst) {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        panic!("the sweep never ran");
    }

    #[test]
    fn real_sweep_deletes_removed_folders() {
        let t = tempfile::tempdir().unwrap();
        let gone = t.path().join("plugins/removed/com.test.old");
        fs::create_dir_all(&gone).unwrap();
        fs::write(gone.join("plugin.toml"), b"x").unwrap();
        let _p = Plugins::load(t.path());
        for _ in 0..100 {
            if !gone.exists() {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        panic!("removed/ was never swept");
    }

    #[test]
    fn cold_start_path_kept() {
        let t = tempfile::tempdir().unwrap();
        let p = Plugins::load(t.path());
        assert_eq!(p.take_pending(), None);
        p.set_pending(PathBuf::from("C:/x/A.lumenplugin"));
        assert_eq!(p.take_pending(), Some(PathBuf::from("C:/x/A.lumenplugin")));
        assert_eq!(p.take_pending(), None);
    }

    #[test]
    fn package_arg_picks_plugin_file() {
        let t = tempfile::tempdir().unwrap();
        let upper = t.path().join("A.LUMENPLUGIN");
        let lower = t.path().join("b.lumenplugin");
        let zip = t.path().join("c.zip");
        for f in [&upper, &lower, &zip] {
            fs::write(f, b"x").unwrap();
        }
        let s = |p: &Path| p.display().to_string();
        let exe = "Lumen.exe".to_string();
        // Any case of the extension, once the file exists.
        assert_eq!(package_arg(&[exe.clone(), s(&upper)]), Some(upper.clone()));
        // Flags are skipped; the first plugin file wins.
        assert_eq!(package_arg(&[exe.clone(), "--minimized".into(), s(&lower), s(&upper)]), Some(lower.clone()));
        // Not a plugin file, or not there.
        assert_eq!(package_arg(&[exe.clone(), s(&zip)]), None);
        assert_eq!(package_arg(&[exe.clone(), s(&t.path().join("gone.lumenplugin"))]), None);
        // A folder named like one is not a package.
        let dir = t.path().join("d.lumenplugin");
        fs::create_dir(&dir).unwrap();
        assert_eq!(package_arg(&[exe.clone(), s(&dir)]), None);
        // The program itself is never an argument.
        assert_eq!(package_arg(&[s(&lower)]), None);
        assert_eq!(package_arg(&[]), None);
    }

    #[test]
    fn second_launch_path_is_kept() {
        // A second launch can arrive before the first instance's page listens (two files opened
        // at once): the path is kept as well as sent, so the page finds it when it asks.
        let t = tempfile::tempdir().unwrap();
        let p = Plugins::load(t.path());
        let file = t.path().join("B.lumenplugin");
        fs::write(&file, b"x").unwrap();
        let args = vec!["Lumen.exe".to_string(), file.display().to_string()];
        assert_eq!(p.keep_package_arg(&args), Some(file.clone()));
        assert_eq!(p.take_pending(), Some(file));
        assert_eq!(p.keep_package_arg(&["Lumen.exe".to_string()]), None);
        assert_eq!(p.take_pending(), None);
    }

    #[test]
    fn log_line_bounded() {
        let mut ring = LogRing::default();
        let big = format!("a\nb\r\n{}", "é".repeat(100_000));
        ring.push(&big);
        let line = &ring.lines[0];
        assert!(line.len() <= LOG_LINE_MAX + '…'.len_utf8(), "{} bytes", line.len());
        assert!(!line.contains('\n') && !line.contains('\r'), "a plugin must not forge log lines");
        assert!(line.starts_with("a b"));
        assert!(line.ends_with('…'));
        ring.push("short");
        assert_eq!(ring.lines[1], "short");
    }

    #[test]
    fn log_flood_reaches_lumens_log_only_a_little() {
        let mut ring = LogRing::default();
        let forwarded = (0..10_000).filter_map(|n| ring.push(&format!("line {n}"))).count();
        assert_eq!(forwarded, LOG_TRACED + 1, "the first lines and one notice");
        assert_eq!(ring.lines.len(), LOG_RING, "the plugin's own ring keeps the newest");
    }

    #[test]
    fn source_has_no_nul_byte() {
        // A stray NUL made git treat this file as binary (no diff, no merge, no line-ending fix).
        assert!(!include_bytes!("state.rs").contains(&0u8));
    }
}
