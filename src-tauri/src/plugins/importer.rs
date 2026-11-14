//! Plugin importers: the Import page's cards and an import through a plugin (Round 23 lane A3).

use std::collections::{HashMap, HashSet};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};

use photos_core::extras::ItemExtras;
use photos_core::sidecars::SKIP_UNREADABLE;
use photos_import::{ImportError, ImportSource, SourceItem};
use photos_plugins::host::{ChosenRoot, Extras, HostCtx, ImporterInstance, Item, LogSink};
use photos_plugins::manifest::ImporterDecl;
use photos_plugins::{Pick, PluginError, PluginState};

use super::Plugins;

/// A path made comparable: canonical when it exists, without the Windows `\?\` prefix, and
/// lower-cased on Windows (whose file systems ignore case).
fn comparable(p: &Path) -> PathBuf {
    let canon = std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    #[cfg(windows)]
    {
        let s = canon.to_string_lossy().replace('/', "\\");
        let s = s.strip_prefix(r"\\?\").unwrap_or(&s);
        PathBuf::from(s.to_lowercase())
    }
    #[cfg(not(windows))]
    canon
}

/// The folder or file the person chose for a plugin import, checked: it exists, is the kind the
/// importer asked for (a file with one of its extensions, any file when it lists none), and has
/// nothing to do with a Lumen library, in either direction.
pub fn check_chosen(
    chosen: &Path,
    decl: &ImporterDecl,
    libraries: &[PathBuf],
) -> Result<ChosenRoot, PluginError> {
    let root = ChosenRoot::new(chosen, decl.pick)?;
    let me = comparable(root.path());
    for lib in libraries {
        let lib = comparable(lib);
        if me.starts_with(&lib) || lib.starts_with(&me) {
            return Err(PluginError::chosen_root(
                "That folder is part of a Lumen library. Choose the folder your export is in.",
            ));
        }
    }
    if decl.pick == Pick::File && !decl.extensions.is_empty() {
        let ext = chosen.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
        if !decl.extensions.iter().any(|x| x.trim_start_matches('.').eq_ignore_ascii_case(&ext)) {
            return Err(PluginError::chosen_root(format!(
                "{} isn't a file this importer reads.",
                chosen.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
            )));
        }
    }
    Ok(root)
}

/// The most one `read` asks a plugin for.
const READ_CHUNK: u32 = 4 << 20;

const STOPPED: &str = "The import was stopped.";

fn source_err(e: PluginError) -> ImportError {
    ImportError::Source(e.message)
}

type Job = Box<dyn FnOnce(&mut ImporterInstance) + Send>;

/// The importer instance on a thread of its own, called through a channel. The thread ends when
/// the import drops its source (the last sender).
///
/// Calls must not wait on a lock or on rayon: ingest reads items on its own rayon pool, and a
/// rayon worker that blocks in `ThreadPool::install` (which every guest call does) steals the
/// next item's job meanwhile, which would then wait for the instance the stolen-from job holds.
/// A plain channel wait never steals.
struct Worker {
    /// `None` only while dropping.
    tx: Option<mpsc::Sender<Job>>,
    thread: Option<std::thread::JoinHandle<()>>,
    /// The first trap's message. After it every call, and every item, fails with it without
    /// reaching the plugin: the import ends with that reason rather than as cancelled, and a
    /// trap is one trip however many items fail after it.
    tripped: Mutex<Option<String>>,
    label: String,
}

impl Worker {
    fn start(mut instance: ImporterInstance, label: String) -> std::io::Result<Worker> {
        let (tx, rx) = mpsc::channel::<Job>();
        let thread = std::thread::Builder::new().name("lumen-plugin-import".into()).spawn(move || {
            for job in rx {
                job(&mut instance);
            }
        })?;
        Ok(Worker { tx: Some(tx), thread: Some(thread), tripped: Mutex::default(), label })
    }

    fn tripped(&self) -> Option<PluginError> {
        let t = self.tripped.lock().unwrap_or_else(|e| e.into_inner());
        t.as_ref().map(|m| PluginError { code: "plugin_trap", message: m.clone() })
    }

    fn call<R: Send + 'static>(&self, f: impl FnOnce(&mut ImporterInstance) -> Result<R, PluginError> + Send + 'static) -> Result<R, PluginError> {
        if let Some(e) = self.tripped() {
            return Err(e);
        }
        let (reply, answer) = mpsc::sync_channel(1);
        let gone = || PluginError::invalid("The importer stopped working.");
        let tx = self.tx.as_ref().ok_or_else(gone)?;
        tx.send(Box::new(move |i| {
            let _ = reply.send(f(i));
        })).map_err(|_| gone())?;
        let r = answer.recv().map_err(|_| gone())?;
        r.map_err(|mut e| {
            if e.code == "plugin_trap" {
                let mut t = self.tripped.lock().unwrap_or_else(|e| e.into_inner());
                e.message = t.get_or_insert_with(|| format!("{} stopped working, so the import was stopped. {}", self.label, e.message)).clone();
            }
            e
        })
    }
}

impl Drop for Worker {
    /// Closes the channel and waits for the thread, so the instance (and the cancel flag its
    /// context holds) is gone when the import's source is. No call is in flight here: calls
    /// borrow the worker, and this is its last owner.
    fn drop(&mut self) {
        self.tx = None;
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// What an item is, by the `rel_path` the ingest knows it by.
#[derive(Clone)]
struct Entry {
    key: String,
    /// Relative to the chosen root: the host opens the file itself.
    path: Option<String>,
}

/// A plugin importer as an [`ImportSource`] (Round 23 A3, spec §10): ingest, verify, sidecars,
/// skips and history see one more kind of source (`SOURCE_PLUGIN`).
///
/// An item's `rel_path` is its `path` when it has one, else its key. Items with a `path` are
/// opened by the host below the chosen root; the rest are read through the plugin in 4 MB
/// chunks. One instance serves the whole import, its calls one after another.
pub struct PluginSource {
    plugin: String,
    label: String,
    root: ChosenRoot,
    cancel: Arc<AtomicBool>,
    log: LogSink,
    worker: Arc<Worker>,
    entries: Mutex<HashMap<String, Entry>>,
    /// Items the host refused (a `path` outside the chosen root), listed by `leftovers`.
    skipped: Mutex<Vec<(String, &'static str)>>,
}

impl PluginSource {
    /// An importer for one import of `plugin` over `root`. `cancel` is set when the person
    /// cancels, and by the plugin leaving the on-and-ready state; it also stops a running call.
    pub fn new(plugins: &Plugins, plugin: &str, root: ChosenRoot, cancel: Arc<AtomicBool>) -> Result<PluginSource, PluginError> {
        // Everything is copied out of the store before the engine is used: a trap calls
        // `mark_stopped`, which writes the store.
        let (wasm, granted, settings, data_dir, label) = {
            let store = plugins.read();
            let inst = store.get(plugin).ok_or_else(|| PluginError::invalid(format!("No plugin {plugin}")))?;
            match &inst.info.state {
                PluginState::Ready => {}
                PluginState::Stopped { reason } => return Err(PluginError::invalid(reason.clone())),
                _ => return Err(PluginError::invalid(format!("{} isn't turned on", inst.info.name))),
            }
            let decl = inst.manifest.importer.as_ref().ok_or_else(|| PluginError::invalid(format!("{} isn't an importer", inst.info.name)))?;
            let settings = inst.info.settings.iter().map(|s| (s.key.clone(), s.value.clone())).collect();
            (inst.dir.join("plugin.wasm"), inst.granted.clone(), settings, store.data_dir(plugin), decl.label.clone())
        };
        let log = plugins.log_sink(plugin);
        let ctx = HostCtx {
            plugin: plugin.to_string(),
            granted,
            settings,
            data_dir,
            log: log.clone(),
            cancel: Some(cancel.clone()),
            chosen: Some(root.clone()),
            photos: None,
            drafts: None,
        };
        let instance = plugins.engine()?.importer_at(&wasm, ctx)?;
        plugins.cancel_when_off(plugin, &cancel);
        Ok(PluginSource {
            plugin: plugin.to_string(),
            label: label.clone(),
            root,
            cancel: cancel.clone(),
            log,
            worker: Arc::new(Worker::start(instance, label.clone()).map_err(|e| PluginError::invalid(format!("The importer couldn't start: {e}")))?),
            entries: Mutex::default(),
            skipped: Mutex::default(),
        })
    }

    fn log(&self, line: &str) {
        (self.log)(&format!("plugin={} {line}", self.plugin));
    }

    fn call<R: Send + 'static>(&self, f: impl FnOnce(&mut ImporterInstance) -> Result<R, PluginError> + Send + 'static) -> Result<R, ImportError> {
        self.worker.call(f).map_err(source_err)
    }

    fn entry(&self, item: &SourceItem) -> Option<Entry> {
        self.entries.lock().unwrap_or_else(|e| e.into_inner()).get(&item.rel_path).cloned()
    }

    /// `it` as an item ingest can open, or `None` (logged) when its `path` leaves the chosen root
    /// or names nothing readable (also listed as a skip), or when another item already goes by
    /// its `rel_path`. The same item again (a sidecar two photos share) is accepted again.
    fn accept(&self, it: Item) -> Option<SourceItem> {
        let rel_path = it.path.clone().unwrap_or_else(|| it.key.clone());
        if let Some(p) = &it.path {
            if let Err(e) = self.root.resolve(p) {
                self.log(&format!("skipped {p}: {}", e.message));
                self.skipped.lock().unwrap_or_else(|e| e.into_inner()).push((p.clone(), SKIP_UNREADABLE));
                return None;
            }
        }
        let mut entries = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        match entries.get(&rel_path) {
            Some(e) if e.key == it.key && e.path == it.path => {}
            Some(e) => {
                let other = e.key.clone();
                drop(entries);
                self.log(&format!("{} and {other} both go by {rel_path}; only {other} is imported", it.key));
                return None;
            }
            None => {
                entries.insert(rel_path.clone(), Entry { key: it.key, path: it.path });
            }
        }
        Some(SourceItem { rel_path, file_name: it.name, size: it.size, modified_at: it.modified, created_at: None })
    }
}

impl ImportSource for PluginSource {
    fn label(&self) -> String {
        self.label.clone()
    }

    fn kind(&self) -> i64 {
        photos_core::batches::SOURCE_PLUGIN
    }

    fn enumerate(&self) -> Result<Vec<SourceItem>, ImportError> {
        let items = self.call(|i| i.enumerate())?;
        self.entries.lock().unwrap_or_else(|e| e.into_inner()).clear();
        self.skipped.lock().unwrap_or_else(|e| e.into_inner()).clear();
        // Each key once: a plugin that lists a file twice must not import it twice.
        let mut keys = HashSet::new();
        let mut out = Vec::with_capacity(items.len());
        for it in items {
            if !keys.insert(it.key.clone()) {
                self.log(&format!("listed {} twice; imported once", it.key));
                continue;
            }
            out.extend(self.accept(it));
        }
        Ok(out)
    }

    fn open(&self, item: &SourceItem) -> Result<Box<dyn Read + Send>, ImportError> {
        if self.cancel.load(Ordering::SeqCst) {
            return Err(ImportError::Source(STOPPED.into()));
        }
        if let Some(e) = self.worker.tripped() {
            return Err(source_err(e));
        }
        let entry = self.entry(item).ok_or_else(|| ImportError::Source(format!("{} isn't one of the plugin's items", item.rel_path)))?;
        if let Some(path) = &entry.path {
            let real = self.root.resolve(path).map_err(source_err)?;
            return Ok(Box::new(std::fs::File::open(real)?));
        }
        let (worker, key) = (self.worker.clone(), entry.key);
        let fetch = move |offset: u64| {
            let key = key.clone();
            worker.call(move |i| i.read(&key, offset, READ_CHUNK)).map_err(|e| std::io::Error::other(e.message))
        };
        Ok(Box::new(PluginReader::new(Box::new(fetch), self.cancel.clone(), item.size)))
    }

    fn extras(&self, item: &SourceItem) -> Option<ItemExtras> {
        let entry = self.entry(item)?;
        let key = entry.key.clone();
        match self.call(move |i| i.extras(&key)) {
            Ok(x) => x.map(|x| item_extras(x, &|line| self.log(line))),
            Err(e) => {
                self.log(&format!("no extras for {}: {e}", entry.key));
                None
            }
        }
    }

    /// The chosen folder or file is still there.
    fn is_connected(&self) -> bool {
        self.root.path().exists()
    }

    fn sidecars(&self, item: &SourceItem) -> Vec<SourceItem> {
        let Some(entry) = self.entry(item) else { return Vec::new() };
        let key = entry.key.clone();
        match self.call(move |i| i.sidecars(&key)) {
            Ok(list) => list.into_iter().filter_map(|it| self.accept(it)).collect(),
            Err(e) => {
                self.log(&format!("no sidecars for {}: {e}", entry.key));
                Vec::new()
            }
        }
    }

    fn leftovers(&self) -> Vec<(String, &'static str)> {
        self.skipped.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }
}

type Fetch = Box<dyn FnMut(u64) -> std::io::Result<Vec<u8>> + Send>;

/// An item read through the plugin, one `read` call per [`READ_CHUNK`], until the plugin answers
/// with nothing. The bytes must come to the size the plugin listed (when it listed one): a
/// short or long item fails rather than landing cut short or padded, since verify hashes the
/// same stream and could not tell.
struct PluginReader {
    fetch: Fetch,
    cancel: Arc<AtomicBool>,
    size: u64,
    offset: u64,
    buf: Vec<u8>,
    pos: usize,
    done: bool,
}

impl PluginReader {
    fn new(fetch: Fetch, cancel: Arc<AtomicBool>, size: u64) -> PluginReader {
        PluginReader { fetch, cancel, size, offset: 0, buf: Vec::new(), pos: 0, done: false }
    }

    fn wrong_size(&self) -> std::io::Error {
        std::io::Error::other(format!("the plugin listed {} bytes but gave {}", self.size, self.offset))
    }
}

impl Read for PluginReader {
    fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
        if self.pos >= self.buf.len() {
            if self.done || out.is_empty() {
                return Ok(0);
            }
            if self.cancel.load(Ordering::SeqCst) {
                return Err(std::io::Error::other(STOPPED));
            }
            let chunk = (self.fetch)(self.offset)?;
            self.offset += chunk.len() as u64;
            if self.size > 0 && self.offset > self.size {
                return Err(self.wrong_size());
            }
            if chunk.is_empty() {
                self.done = true;
                if self.size > 0 && self.offset != self.size {
                    return Err(self.wrong_size());
                }
                return Ok(0);
            }
            self.buf = chunk;
            self.pos = 0;
        }
        let n = out.len().min(self.buf.len() - self.pos);
        out[..n].copy_from_slice(&self.buf[self.pos..self.pos + n]);
        self.pos += n;
        Ok(n)
    }
}

/// A plugin's `captured-at`: RFC 3339 is a moment (UTC seconds and its offset in minutes); the
/// wall-clock forms `YYYY:MM:DD HH:MM:SS` (EXIF), `YYYY-MM-DDTHH:MM:SS` and
/// `YYYY-MM-DD HH:MM:SS` carry no offset and are wall-clock seconds.
#[derive(Debug, PartialEq)]
enum Captured {
    At(i64, i32),
    Wall(i64),
}

fn parse_captured(s: &str) -> Option<Captured> {
    let s = s.trim();
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(s) {
        return Some(Captured::At(dt.timestamp(), dt.offset().local_minus_utc() / 60));
    }
    ["%Y:%m:%d %H:%M:%S", "%Y-%m-%dT%H:%M:%S", "%Y-%m-%d %H:%M:%S"]
        .iter()
        .find_map(|f| chrono::NaiveDateTime::parse_from_str(s, f).ok())
        .map(|n| Captured::Wall(n.and_utc().timestamp()))
}

/// A plugin's extras as the catalogue's; a date that can't be read is dropped and logged.
fn item_extras(x: Extras, log: &dyn Fn(&str)) -> ItemExtras {
    let date = x.captured_at.as_deref().and_then(|s| {
        let at = parse_captured(s);
        if at.is_none() {
            log(&format!("ignored a date it couldn't read: {s}"));
        }
        at
    });
    let albums = x.albums.into_iter().map(|a| a.trim().to_string()).filter(|a| !a.is_empty()).collect();
    let location = match (x.lat, x.lon) {
        (Some(lat), Some(lon)) if (-90.0..=90.0).contains(&lat) && (-180.0..=180.0).contains(&lon) => Some((lat, lon)),
        _ => None,
    };
    let (captured, captured_wall) = match date {
        Some(Captured::At(utc, offset)) => (Some((utc, Some(offset))), None),
        // The file's own offset (kept) turns it into UTC.
        Some(Captured::Wall(local)) => (None, Some(local)),
        None => (None, None),
    };
    ItemExtras { favorite: x.favourite, albums, title: x.title, caption: x.caption, keywords: x.keywords, captured, captured_wall, location, ..Default::default() }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn decl(pick: Pick, ext: &[&str]) -> ImporterDecl {
        ImporterDecl {
            label: "X".into(),
            pick,
            extensions: ext.iter().map(|s| s.to_string()).collect(),
            hint: String::new(),
        }
    }

    #[test]
    fn inside_library_refused() {
        let d = tempfile::tempdir().unwrap();
        let lib = d.path().join("lib");
        fs::create_dir_all(lib.join("sub")).unwrap();
        let e = check_chosen(&lib.join("sub"), &decl(Pick::Folder, &[]), &[lib]).unwrap_err();
        assert_eq!(e.code, "plugin_chosen_root");
        assert_eq!(e.message, "That folder is part of a Lumen library. Choose the folder your export is in.");
    }

    #[test]
    fn parent_of_library_refused() {
        let d = tempfile::tempdir().unwrap();
        let lib = d.path().join("pics").join("lib");
        fs::create_dir_all(&lib).unwrap();
        let e = check_chosen(&d.path().join("pics"), &decl(Pick::Folder, &[]), &[lib]).unwrap_err();
        assert_eq!(e.code, "plugin_chosen_root");
    }

    #[test]
    fn unrelated_folder_ok() {
        let d = tempfile::tempdir().unwrap();
        let (lib, other) = (d.path().join("lib"), d.path().join("export"));
        fs::create_dir_all(&lib).unwrap();
        fs::create_dir_all(&other).unwrap();
        assert!(check_chosen(&other, &decl(Pick::Folder, &[]), &[lib]).is_ok());
    }

    #[test]
    fn file_pick_extension_checked() {
        let d = tempfile::tempdir().unwrap();
        fs::write(d.path().join("a.ZIP"), b"z").unwrap();
        fs::write(d.path().join("a.tar"), b"t").unwrap();
        let dc = decl(Pick::File, &["zip"]);
        assert!(check_chosen(&d.path().join("a.ZIP"), &dc, &[]).is_ok());
        assert!(check_chosen(&d.path().join("a.tar"), &dc, &[]).is_err());
    }

    #[test]
    fn file_pick_without_extensions_takes_any_file() {
        let d = tempfile::tempdir().unwrap();
        fs::write(d.path().join("export.anything"), b"z").unwrap();
        assert!(check_chosen(&d.path().join("export.anything"), &decl(Pick::File, &[]), &[]).is_ok());
    }

    #[test]
    fn file_inside_library_refused() {
        let d = tempfile::tempdir().unwrap();
        let lib = d.path().join("lib");
        fs::create_dir_all(&lib).unwrap();
        fs::write(lib.join("library.db"), b"x").unwrap();
        let e = check_chosen(&lib.join("library.db"), &decl(Pick::File, &[]), &[lib]).unwrap_err();
        assert_eq!(e.code, "plugin_chosen_root");
    }

    #[test]
    fn missing_root_refused() {
        let d = tempfile::tempdir().unwrap();
        let e = check_chosen(&d.path().join("nope"), &decl(Pick::Folder, &[]), &[]).unwrap_err();
        assert_eq!(e.code, "plugin_chosen_root");
    }

    #[cfg(windows)]
    #[test]
    fn case_insensitive_on_windows() {
        let d = tempfile::tempdir().unwrap();
        let lib = d.path().join("Lib");
        fs::create_dir_all(lib.join("sub")).unwrap();
        let upper = PathBuf::from(lib.to_string_lossy().to_uppercase());
        assert!(check_chosen(&lib.join("sub"), &decl(Pick::Folder, &[]), &[upper]).is_err());
    }

    // ---- PluginSource (Task 3): A2's `importer_ok.wasm` as a Developer-mode folder, a temp
    // library, the real ingest. Nothing here touches a real library or device.

    use photos_core::batches::{list_batches, BATCH_CANCELLED, SOURCE_PLUGIN};
    use photos_core::Library;
    use photos_import::{run_import, ImportOptions, ImportSummary, Ingested};
    use photos_jobs::CancelToken;

    const ID: &str = "com.test.fixture";
    const WASM: &[u8] = include_bytes!("../../crates/photos-plugins/tests/wasm/importer_ok.wasm");

    const MANIFEST: &str = r#"id = "com.test.fixture"
name = "Fixture importer plugin"
version = "1.0.0"
author = "Tester"
description = "A test importer."
license = "MIT"
lumen = ">=26.0.0"
api = 1
permissions = ["import.read-chosen"]

[importer]
label = "Fixture importer"
pick = "folder"

[[settings]]
key = "mode"
label = "Mode"
type = "text"
default = ""

[[settings]]
key = "spec"
label = "Spec"
type = "text"
default = ""

[[settings]]
key = "trap_reads"
label = "Trap"
type = "text"
default = ""

[[settings]]
key = "captured"
label = "Captured"
type = "text"
default = ""
"#;

    struct Rig {
        dir: tempfile::TempDir,
        plugins: Arc<Plugins>,
        /// The folder the person "chose".
        root: PathBuf,
    }

    fn jpeg(path: &Path, seed: u8) {
        let img = image::ImageBuffer::from_fn(64, 48, |x, y| image::Rgb([(x as u8).wrapping_mul(seed), y as u8, seed]));
        img.save(path).unwrap();
    }

    /// The fixture installed in a temp store (Developer mode), `settings` applied, an empty
    /// chosen folder.
    fn rig(settings: &[(&str, &str)]) -> Rig {
        let dir = tempfile::tempdir().unwrap();
        let plugins = Arc::new(Plugins::load(&dir.path().join("app")));
        let folder = dir.path().join("plugin");
        fs::create_dir_all(&folder).unwrap();
        fs::write(folder.join("plugin.toml"), MANIFEST).unwrap();
        fs::write(folder.join("plugin.wasm"), WASM).unwrap();
        {
            let mut store = plugins.write();
            store.set_developer_mode(true).unwrap();
            store.load_dev(&folder, &plugins.ctx()).unwrap();
            for (k, v) in settings {
                store.set_setting(ID, k, serde_json::json!(v)).unwrap();
            }
        }
        let root = dir.path().join("export");
        fs::create_dir_all(&root).unwrap();
        Rig { dir, plugins, root }
    }

    impl Rig {
        fn source(&self) -> PluginSource {
            let root = ChosenRoot::new(&self.root, Pick::Folder).unwrap();
            PluginSource::new(&self.plugins, ID, root, Arc::new(AtomicBool::new(false))).unwrap()
        }

        fn library(&self) -> Library {
            Library::create(&self.dir.path().join("lib")).unwrap()
        }

        fn import(&self, lib: &Library, source: &PluginSource) -> (ImportSummary, Vec<Ingested>) {
            let mut got = Vec::new();
            let summary = run_import(lib, source, &ImportOptions::default(), &CancelToken::new(), &mut |_| {}, &mut |i| got.push(i.clone())).unwrap();
            (summary, got)
        }
    }

    fn count(lib: &Library, sql: &str) -> i64 {
        lib.db.with_conn(|c| Ok(c.query_row(sql, [], |r| r.get(0))?)).unwrap()
    }

    #[test]
    fn imports_items_with_extras() {
        let r = rig(&[("mode", "spec"), ("spec", "a.jpg,a.jpg,a.jpg;b.jpg,b.jpg,"), ("captured", "2020-05-06T07:08:09+02:00")]);
        jpeg(&r.root.join("a.jpg"), 1);
        jpeg(&r.root.join("b.jpg"), 2);
        let (lib, source) = (r.library(), r.source());
        let (summary, got) = r.import(&lib, &source);
        assert_eq!((summary.imported, summary.skipped, summary.failed), (2, 0, 0), "{summary:?}");
        assert_eq!(got.len(), 2);
        assert_eq!(summary.label, "Fixture importer");
        let batches = lib.db.with_conn(|c| list_batches(c, 10)).unwrap();
        assert_eq!((batches[0].source_kind, batches[0].source_label.as_str()), (SOURCE_PLUGIN, "Fixture importer"));
        // The album from the extras exists and holds both; the date and title came through.
        assert_eq!(count(&lib, "SELECT COUNT(*) FROM albums WHERE name = 'Trip'"), 1);
        assert_eq!(count(&lib, "SELECT COUNT(*) FROM album_items"), 2);
        assert_eq!(count(&lib, "SELECT COUNT(*) FROM assets WHERE title IN ('a.jpg', 'b.jpg')"), 2);
        assert_eq!(count(&lib, "SELECT COUNT(*) FROM assets WHERE is_favorite = 1"), 2);
        let wall = chrono::NaiveDate::from_ymd_opt(2020, 5, 6).unwrap().and_hms_opt(7, 8, 9).unwrap().and_utc().timestamp();
        assert_eq!(count(&lib, &format!("SELECT COUNT(*) FROM assets WHERE captured_at = {wall} AND tz_offset_minutes = 120")), 2);
        assert_eq!(count(&lib, &format!("SELECT COUNT(*) FROM assets WHERE captured_at_utc = {wall} - 7200")), 2);
    }

    #[test]
    fn wall_clock_date_utc_agrees_with_offset() {
        let r = rig(&[("mode", "spec"), ("spec", "a.jpg,a.jpg,a.jpg"), ("captured", "2020:05:06 07:08:09")]);
        jpeg(&r.root.join("a.jpg"), 1);
        let (lib, source) = (r.library(), r.source());
        let (summary, _) = r.import(&lib, &source);
        assert_eq!(summary.imported, 1, "{summary:?}");
        let wall = chrono::NaiveDate::from_ymd_opt(2020, 5, 6).unwrap().and_hms_opt(7, 8, 9).unwrap().and_utc().timestamp();
        // The test JPEG has no offset of its own: UTC is the wall clock, and the two agree.
        let (at, utc, tz): (i64, i64, Option<i64>) = lib
            .db
            .with_conn(|c| Ok(c.query_row("SELECT captured_at, captured_at_utc, tz_offset_minutes FROM assets", [], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?))
            .unwrap();
        assert_eq!(at, wall);
        assert_eq!(utc, wall - tz.unwrap_or(0) * 60);
    }

    #[test]
    fn path_items_opened_by_host() {
        let r = rig(&[("mode", "spec"), ("spec", "a.jpg,a.jpg,a.jpg;b.jpg,b.jpg,")]);
        jpeg(&r.root.join("a.jpg"), 1);
        jpeg(&r.root.join("b.jpg"), 2);
        let (lib, source) = (r.library(), r.source());
        let items = source.enumerate().unwrap();
        let mut bytes = Vec::new();
        source.open(&items[0]).unwrap().read_to_end(&mut bytes).unwrap();
        assert_eq!(bytes, fs::read(r.root.join("a.jpg")).unwrap());
        // Through the plugin the bytes are the same, one `read` call per 4 MB.
        let mut through = Vec::new();
        source.open(&items[1]).unwrap().read_to_end(&mut through).unwrap();
        assert_eq!(through, fs::read(r.root.join("b.jpg")).unwrap());

        let (summary, _) = r.import(&lib, &source);
        assert_eq!(summary.imported, 2);
        let log = r.plugins.log_lines(ID);
        assert!(log.iter().any(|l| l.ends_with("read b.jpg")), "{log:?}");
        assert!(!log.iter().any(|l| l.ends_with("read a.jpg")), "the plugin's read ran for a path item: {log:?}");
    }

    #[test]
    fn escaping_item_skipped() {
        let r = rig(&[("mode", "spec"), ("spec", "a.jpg,a.jpg,a.jpg;x.jpg,x.jpg,../x.jpg;b.jpg,b.jpg,b.jpg")]);
        jpeg(&r.root.join("a.jpg"), 1);
        jpeg(&r.root.join("b.jpg"), 2);
        jpeg(&r.dir.path().join("x.jpg"), 3); // outside the chosen folder
        let (lib, source) = (r.library(), r.source());
        let (summary, _) = r.import(&lib, &source);
        assert_eq!((summary.imported, summary.failed), (2, 0), "{summary:?}");
        let skips = lib.db.with_conn(|c| photos_core::sidecars::skips_of(c, summary.batch_id)).unwrap();
        assert_eq!(skips.len(), 1, "{skips:?}");
        assert_eq!((skips[0].path.as_str(), skips[0].reason.as_str()), ("../x.jpg", SKIP_UNREADABLE));
        assert_eq!(count(&lib, "SELECT COUNT(*) FROM assets"), 2, "the outside file was not imported");
    }

    #[test]
    fn duplicate_keys_once() {
        let r = rig(&[("mode", "spec"), ("spec", "a.jpg,a.jpg,a.jpg;a.jpg,a.jpg,a.jpg;b.jpg,b.jpg,b.jpg")]);
        jpeg(&r.root.join("a.jpg"), 1);
        jpeg(&r.root.join("b.jpg"), 2);
        let (lib, source) = (r.library(), r.source());
        assert_eq!(source.enumerate().unwrap().len(), 2);
        let (summary, _) = r.import(&lib, &source);
        assert_eq!((summary.imported, summary.skipped, summary.failed), (2, 0, 0), "{summary:?}");
    }

    #[test]
    fn trap_mid_import() {
        // Items 1-2 are opened by the host; 3-5 go through the plugin, which traps on its first read.
        let spec = "a.jpg,a.jpg,a.jpg;b.jpg,b.jpg,b.jpg;c.jpg,c.jpg,;d.jpg,d.jpg,;e.jpg,e.jpg,";
        let r = rig(&[("mode", "spec"), ("spec", spec), ("trap_reads", "true")]);
        for (i, n) in ["a", "b", "c", "d", "e"].iter().enumerate() {
            jpeg(&r.root.join(format!("{n}.jpg")), i as u8 + 1);
        }
        let lib = r.library();
        // As in the app: the plugin's cancel flag is the import's token.
        let cancel = Arc::new(AtomicBool::new(false));
        let source = PluginSource::new(&r.plugins, ID, ChosenRoot::new(&r.root, Pick::Folder).unwrap(), cancel.clone()).unwrap();
        let mut got = Vec::new();
        let summary = run_import(&lib, &source, &ImportOptions::default(), &CancelToken::from_flag(cancel), &mut |_| {}, &mut |i| got.push(i.clone())).unwrap();
        assert_eq!((summary.imported, summary.failed), (2, 3), "{summary:?}");
        assert!(!summary.cancelled, "a trap ends the import with its reason, not as cancelled: {summary:?}");
        assert_eq!(got.len(), 2, "items 1-2 are kept");
        let failures = lib.db.with_conn(|c| photos_core::batches::list_failures(c, summary.batch_id)).unwrap();
        assert!(failures.iter().all(|f| f.reason.contains("Fixture importer stopped working")), "{failures:?}");
        let engine = r.plugins.engine().unwrap();
        assert_eq!(engine.trips(ID), 1, "one trap is one trip, however many items fail after it");

        // Two more trapping imports make three trips: the plugin stops, and says why.
        for _ in 0..2 {
            let (s, _) = r.import(&lib, &r.source());
            assert_eq!(s.failed, 3);
        }
        assert_eq!(engine.trips(ID), 3);
        let info = r.plugins.read().get(ID).unwrap().info.clone();
        assert!(matches!(info.state, PluginState::Stopped { .. }), "{:?}", info.state);
        let root = ChosenRoot::new(&r.root, Pick::Folder).unwrap();
        let e = PluginSource::new(&r.plugins, ID, root, Arc::new(AtomicBool::new(false))).err().expect("a stopped plugin imports nothing");
        assert_eq!(e.code, "plugin_invalid");
    }

    #[test]
    fn off_mid_import_cancels() {
        let r = rig(&[("mode", "dir")]);
        for i in 0..40u8 {
            jpeg(&r.root.join(format!("p{i:02}.jpg")), i + 1);
        }
        let lib = r.library();
        let cancel = Arc::new(AtomicBool::new(false));
        let root = ChosenRoot::new(&r.root, Pick::Folder).unwrap();
        let source = PluginSource::new(&r.plugins, ID, root, cancel.clone()).unwrap();
        let token = CancelToken::new();
        let (mut got, mut turned_off) = (0usize, false);
        // The person turns the plugin off after the first files; the job's token follows `cancel`.
        let summary = run_import(
            &lib,
            &source,
            &ImportOptions::default(),
            &token,
            &mut |_| {
                if !turned_off {
                    turned_off = true;
                    r.plugins.write().set_enabled(ID, false).unwrap();
                    r.plugins.run_listeners();
                    assert!(cancel.load(Ordering::SeqCst), "turning the plugin off sets the cancel flag");
                    token.cancel();
                }
            },
            &mut |_| got += 1,
        )
        .unwrap();
        assert!(summary.cancelled, "{summary:?}");
        assert!(summary.imported > 0 && summary.imported < 40, "what came in stays, the rest is not read: {summary:?}");
        assert_eq!(got as u64, summary.imported);
        let batches = lib.db.with_conn(|c| list_batches(c, 10)).unwrap();
        assert_eq!(batches[0].state, BATCH_CANCELLED);
        assert_eq!(count(&lib, "SELECT COUNT(*) FROM assets") as u64, summary.imported);
        // Nothing more is opened through a plugin that is off.
        let items = source.enumerate().unwrap_or_default();
        assert!(items.is_empty() || source.open(&items[0]).is_err());
        assert_eq!(r.plugins.passing_listeners(), 0, "the import's listener went when it cancelled");
    }

    #[test]
    fn listener_dropped_with_the_import() {
        let r = rig(&[("mode", "dir")]);
        let source = r.source();
        assert_eq!(r.plugins.passing_listeners(), 1);
        r.plugins.run_listeners();
        assert_eq!(r.plugins.passing_listeners(), 1, "kept while the import runs and the plugin is on");
        drop(source);
        r.plugins.run_listeners();
        assert_eq!(r.plugins.passing_listeners(), 0, "an import that is over leaves nothing behind");
    }

    #[test]
    fn cancel_interrupts_a_running_call() {
        // `progress-forever` never returns on its own: only the cancel flag ends the call.
        let r = rig(&[("mode", "progress-forever")]);
        let cancel = Arc::new(AtomicBool::new(false));
        let source = PluginSource::new(&r.plugins, ID, ChosenRoot::new(&r.root, Pick::Folder).unwrap(), cancel.clone()).unwrap();
        // Cancel, as `cancel_import` does it: on the import's token.
        let token = CancelToken::from_flag(cancel);
        let t = std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(300));
            token.cancel();
        });
        let start = std::time::Instant::now();
        assert!(source.enumerate().is_err());
        assert!(start.elapsed() < std::time::Duration::from_secs(10), "{:?}", start.elapsed());
        t.join().unwrap();
    }

    #[test]
    fn shared_sidecar_accepted_for_each_photo() {
        let r = rig(&[("mode", "spec"), ("spec", "a.jpg,a.jpg,a.jpg")]);
        jpeg(&r.root.join("a.jpg"), 1);
        let source = r.source();
        source.enumerate().unwrap();
        let aae = || Item { key: "IMG_1.aae".into(), name: "IMG_1.aae".into(), size: 3, modified: None, path: None };
        assert!(source.accept(aae()).is_some(), "the HEIC's sidecar");
        assert!(source.accept(aae()).is_some(), "the same sidecar for the MOV");
        // A different item under a name already taken is refused, and says so.
        let clash = Item { key: "other".into(), name: "a.jpg".into(), size: 1, modified: None, path: Some("a.jpg".into()) };
        assert!(source.accept(clash).is_none());
        assert!(r.plugins.log_lines(ID).iter().any(|l| l.contains("both go by a.jpg")), "{:?}", r.plugins.log_lines(ID));
    }

    fn reader(chunks: &[&[u8]], size: u64) -> PluginReader {
        let mut it = chunks.iter().map(|c| c.to_vec()).collect::<Vec<_>>().into_iter();
        PluginReader::new(Box::new(move |_| Ok(it.next().unwrap_or_default())), Arc::new(AtomicBool::new(false)), size)
    }

    #[test]
    fn read_until_the_plugin_has_nothing_more() {
        let mut out = Vec::new();
        reader(&[b"abc", b"de"], 5).read_to_end(&mut out).unwrap();
        assert_eq!(out, b"abcde");
        let mut out = Vec::new();
        reader(&[b"abc", b"de"], 0).read_to_end(&mut out).unwrap();
        assert_eq!(out, b"abcde", "no size listed: read until empty");
    }

    #[test]
    fn wrong_listed_size_fails_the_item() {
        // Listed smaller than it is: never cut short.
        let e = reader(&[b"abcde"], 2).read_to_end(&mut Vec::new()).unwrap_err();
        assert!(e.to_string().contains("listed 2 bytes"), "{e}");
        // Listed larger than it gives.
        let e = reader(&[b"abc"], 9).read_to_end(&mut Vec::new()).unwrap_err();
        assert!(e.to_string().contains("listed 9 bytes but gave 3"), "{e}");
    }

    #[test]
    fn sidecars_kept() {
        let r = rig(&[("mode", "spec"), ("spec", "a.jpg,a.jpg,a.jpg")]);
        jpeg(&r.root.join("a.jpg"), 1);
        let xmp = br#"<x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#"><rdf:Description xmlns:xmp="http://ns.adobe.com/xap/1.0/" xmp:Rating="4"/></rdf:RDF></x:xmpmeta>"#;
        fs::write(r.root.join("a.jpg.xmp"), xmp).unwrap();
        let (lib, source) = (r.library(), r.source());
        let (summary, got) = r.import(&lib, &source);
        assert_eq!(summary.imported, 1, "{summary:?}");
        let sidecars = lib.db.with_conn(|c| photos_core::sidecars::sidecars_of(c, got[0].asset_id)).unwrap();
        assert_eq!(sidecars.len(), 1, "{sidecars:?}");
        assert_eq!((sidecars[0].kind.as_str(), sidecars[0].source_name.as_str()), ("xmp", "a.jpg.xmp"));
        assert!(lib.originals_dir().join(&sidecars[0].relative_path).is_file());
        assert_eq!(count(&lib, "SELECT rating FROM assets"), 4, "the XMP's marks reach the photo, as with a folder");
    }

    #[test]
    fn captured_dates() {
        let wall = chrono::NaiveDate::from_ymd_opt(2021, 3, 4).unwrap().and_hms_opt(5, 6, 7).unwrap().and_utc().timestamp();
        assert_eq!(parse_captured("2021:03:04 05:06:07"), Some(Captured::Wall(wall)));
        assert_eq!(parse_captured("2021-03-04T05:06:07"), Some(Captured::Wall(wall)));
        assert_eq!(parse_captured("2021-03-04 05:06:07"), Some(Captured::Wall(wall)));
        assert_eq!(parse_captured("2021-03-04T05:06:07Z"), Some(Captured::At(wall, 0)));
        assert_eq!(parse_captured("2021-03-04T05:06:07+01:00"), Some(Captured::At(wall - 3600, 60)));
        assert_eq!(parse_captured("last Tuesday"), None);
        // An unreadable date is dropped with a log line, the rest of the extras stay.
        let lines = Mutex::new(Vec::new());
        let x = Extras { favourite: true, albums: vec![" Trip ".into(), "".into()], title: None, caption: None, keywords: vec![], captured_at: Some("soon".into()), lat: Some(95.0), lon: Some(1.0) };
        let got = item_extras(x, &|l| lines.lock().unwrap().push(l.to_string()));
        assert_eq!((got.favorite, got.albums, got.captured, got.captured_wall, got.location), (true, vec!["Trip".to_string()], None, None, None));
        assert_eq!(lines.lock().unwrap().len(), 1);
        // A wall clock keeps the photo's own offset (applied by `extras::apply`), a moment brings its own.
        let with = |at: &str| item_extras(Extras { favourite: false, albums: vec![], title: None, caption: None, keywords: vec![], captured_at: Some(at.into()), lat: None, lon: None }, &|_| {});
        let w = with("2021:03:04 05:06:07");
        assert_eq!((w.captured, w.captured_wall), (None, Some(wall)));
        let m = with("2021-03-04T05:06:07+01:00");
        assert_eq!((m.captured, m.captured_wall), (Some((wall - 3600, Some(60))), None));
    }
}
