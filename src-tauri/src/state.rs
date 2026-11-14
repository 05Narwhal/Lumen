use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, RwLock};

use photos_core::layout::{GroupBy, Layout, LayoutIndex};
use photos_core::scope::LayoutScope;
use photos_core::{AppPaths, Library, LibraryInfo, LibraryRegistry, Settings};
use photos_media::{ExifTool, Ffmpeg};
use photos_jobs::{CancelToken, Scheduler};
use serde::Serialize;
use ts_rs::TS;

use crate::error::{AppError, AppResult};
use crate::events::ChangeEmitter;

pub use photos_core::now_unix;

pub const JOB_QUEUE_CAPACITY: usize = 4096;

/// An Apple Photos library opened this session, with the Mac disk volume it was read from.
pub type OpenLibrary = (Option<Arc<photos_import::macdisk::fs::MacFs>>, Arc<photos_import::apple::source::PhotosLibrarySource>);

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct LayoutKey {
    pub width: u32,
    pub row_height: u32,
    pub gap: u32,
    pub scope: LayoutScope,
    /// Ignored for a scope in manual order, which is never sectioned.
    pub group_by: GroupBy,
}

/// One index per scope and every layout computed from them, all dropped together when
/// the library version moves on.
#[derive(Default)]
pub struct LayoutCache {
    pub indexes: HashMap<LayoutScope, Arc<LayoutIndex>>,
    pub version: u64,
    pub computed: HashMap<LayoutKey, Arc<Layout>>,
}

impl LayoutCache {
    pub fn invalidate(&mut self) {
        self.indexes.clear();
        self.computed.clear();
    }
}

pub struct AppState {
    pub paths: AppPaths,
    /// FFmpeg + ffprobe when found (see `photos_media::Ffmpeg::locate`). Re-checked by
    /// `get_ffmpeg_status`, so installing FFmpeg while the app runs is picked up.
    pub ffmpeg: RwLock<Option<Ffmpeg>>,
    /// ExifTool, looked for on first use rather than at launch (it starts Perl): the outer
    /// `None` is "not looked yet". `get_exiftool_status` looks again.
    pub exiftool: RwLock<Option<Option<ExifTool>>>,
    pub settings: RwLock<Settings>,
    pub registry: RwLock<LibraryRegistry>,
    pub library: RwLock<Option<Arc<Library>>>,
    /// Why the current library did not open at launch (Round 17).
    pub library_problem: RwLock<Option<LibraryProblem>>,
    pub jobs: Scheduler,
    pub layout: Mutex<LayoutCache>,
    /// batch id → cancel token for running imports
    pub imports: Mutex<HashMap<i64, CancelToken>>,
    /// batch id → the card's or phone's `import_seen` key, for imports from a device: one at a
    /// time per device, and its page finds the running one again.
    pub import_keys: Mutex<HashMap<i64, String>>,
    /// The user chose to stop running imports and close (round 3): the next close goes through.
    pub close_confirmed: std::sync::atomic::AtomicBool,
    /// Develop has a save waiting (Round 5): a close first asks the page to write it.
    pub unsaved_edits: std::sync::atomic::AtomicBool,
    /// When the page was last asked to write before closing (`window::unsaved_close`).
    pub flush_asked_at: Mutex<Option<std::time::Instant>>,
    pub changes: ChangeEmitter,
    /// Debug-only: id → file path served at `photos://probe/{id}`.
    pub probes: Mutex<HashMap<u64, PathBuf>>,
    /// Tokens for `photos://device-thumb/{token}` and the previews rendered for them.
    pub device_previews: crate::connected::PreviewCache,
    /// Open device browsers (Round 4E).
    pub device_browsers: crate::device_browser::Browsers,
    /// The only HTTP client in Lumen; it exists whether or not music is configured.
    pub http: reqwest::Client,
    pub music_previews: crate::music::PreviewCache,
    /// Apple Photos libraries opened this session, keyed `"<disk id or 'local'>:<path>"`.
    /// Opening one copies its database (hundreds of MB), so it happens once. The Mac disk's
    /// volume is kept alongside to tell a library on a re-plugged disk from a stale one.
    pub photo_libraries: Mutex<HashMap<String, OpenLibrary>>,
    /// The editor's live previews and the unedited images they start from.
    pub edit_previews: crate::edit_preview::PreviewCache,
    /// Develop's preview bases and 100 % frame (Phase 8c).
    pub develop: crate::develop_preview::DevelopState,
    /// Export batch ids, for `export://progress` and `export://finished`.
    pub export_batches: std::sync::atomic::AtomicU64,
    /// On-device models, opened on first use (Phase 7).
    pub ml: crate::models::MlState,
    /// Search's vocabulary and recent parses (Round 4K).
    pub search: crate::search::SearchCache,
    /// Whether the Hidden folder is open (round 3). Starts locked.
    pub vault_lock: crate::vault::VaultLock,
    /// What the WebView said it can play (Round 8A), once per run; `None` until it says.
    pub video_support: RwLock<Option<photos_media::VideoSupport>>,
    /// The AI assistants' MCP server and what it may see (Round 16).
    pub mcp: crate::mcp::McpState,
}

/// Why the current library did not open (Round 17): the app says so instead of failing to start.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct LibraryProblem {
    pub path: String,
    pub code: String,
    pub message: String,
}

/// The library at `path`, or why it can't be opened. Never a panic: a library this build refuses,
/// or can't copy before upgrading, is a state the person can act on.
pub fn open_current(path: &Path, backups: &Path) -> (Option<Library>, Option<LibraryProblem>) {
    match Library::open_protected(path, backups) {
        Ok(lib) => (Some(lib), None),
        Err(e) => {
            tracing::error!(path = %path.display(), "the current library did not open: {e}");
            let e = AppError::from(e);
            (None, Some(LibraryProblem { path: path.display().to_string(), code: e.code, message: e.message }))
        }
    }
}

impl AppState {
    pub fn bootstrap() -> AppResult<Self> {
        let paths = AppPaths::resolve()?;
        paths.ensure_dirs()?;
        let settings = Settings::load(&paths.settings_file())?;
        photos_core::scope::set_hide_raw_with_jpeg(settings.appearance.hide_raw_with_jpeg);
        let registry = LibraryRegistry::load(&paths.registry_file())?;
        let (library, library_problem) = match registry.current_entry().map(|e| PathBuf::from(&e.path)).filter(|p| Library::exists_at(p)) {
            Some(p) => open_current(&p, &paths.pre_migration_dir()),
            None => (None, None),
        };
        let library = library.map(Arc::new);
        if let Some(lib) = &library {
            match lib.reconcile() {
                Ok(0) => {}
                Ok(n) => tracing::warn!(batches = n, "reconciled interrupted imports"),
                Err(e) => tracing::warn!("reconcile failed: {e}"),
            }
            // Files an interrupted import placed but never catalogued (round 3).
            match photos_import::recover(lib) {
                Ok(r) if r.settled > 0 => tracing::warn!(removed = r.removed, settled = r.settled, "settled an interrupted import"),
                Ok(_) => {}
                Err(e) => tracing::warn!("import recovery failed: {e}"),
            }
        }
        let workers = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(2).saturating_sub(1).max(1);
        let ffmpeg = Ffmpeg::locate();
        match &ffmpeg {
            Some(f) => tracing::info!(path = %f.ffmpeg.display(), version = %f.version, "ffmpeg found"),
            None => tracing::info!("ffmpeg not found: video and HEIC are catalogued without previews"),
        }
        // A RAW rawler refuses develops from a DNG cached under app data, never in a library.
        photos_media::develop::stand_in::configure(paths.app_data.join("cache").join("dng"));
        tracing::info!(app_data = %paths.app_data.display(), library = library.is_some(), workers, "bootstrapped");
        Ok(Self {
            paths,
            ffmpeg: RwLock::new(ffmpeg),
            exiftool: RwLock::new(None),
            settings: RwLock::new(settings),
            registry: RwLock::new(registry),
            library: RwLock::new(library),
            library_problem: RwLock::new(library_problem),
            jobs: Scheduler::new(workers, JOB_QUEUE_CAPACITY),
            layout: Mutex::new(LayoutCache::default()),
            imports: Mutex::new(HashMap::new()),
            import_keys: Mutex::new(HashMap::new()),
            close_confirmed: std::sync::atomic::AtomicBool::new(false),
            unsaved_edits: std::sync::atomic::AtomicBool::new(false),
            flush_asked_at: Mutex::new(None),
            changes: ChangeEmitter::default(),
            probes: Mutex::new(HashMap::new()),
            device_previews: Default::default(),
            device_browsers: Default::default(),
            http: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(10))
                .user_agent(concat!("Lumen/", env!("CARGO_PKG_VERSION")))
                .build()
                .unwrap_or_default(),
            music_previews: Default::default(),
            photo_libraries: Mutex::new(HashMap::new()),
            edit_previews: Default::default(),
            develop: Default::default(),
            export_batches: Default::default(),
            ml: Default::default(),
            search: Default::default(),
            vault_lock: Default::default(),
            video_support: RwLock::new(None),
            mcp: Default::default(),
        })
    }

    pub fn ffmpeg(&self) -> Option<Ffmpeg> {
        self.ffmpeg.read().expect("ffmpeg lock").clone()
    }

    /// ExifTool when installed, found once and remembered.
    pub fn exiftool(&self) -> Option<ExifTool> {
        if let Some(found) = self.exiftool.read().expect("exiftool lock").clone() {
            return found;
        }
        self.relocate_exiftool()
    }

    /// Look for ExifTool again (a fresh install is picked up without a restart).
    pub fn relocate_exiftool(&self) -> Option<ExifTool> {
        let found = ExifTool::locate();
        match &found {
            Some(e) => tracing::info!(path = %e.program.display(), version = %e.version, "exiftool found"),
            None => tracing::info!("exiftool not found: exports carry pixels only"),
        }
        *self.exiftool.write().expect("exiftool lock") = Some(found.clone());
        found
    }

    pub fn ffmpeg_available(&self) -> bool {
        self.ffmpeg.read().expect("ffmpeg lock").is_some()
    }

    /// Look for FFmpeg again. When availability changes, cached layouts (which carry
    /// `renderable`) are dropped and the grid is told to refresh.
    pub fn relocate_ffmpeg(&self, app: &tauri::AppHandle) -> Option<Ffmpeg> {
        let found = Ffmpeg::locate();
        let changed = {
            let mut cur = self.ffmpeg.write().expect("ffmpeg lock");
            let changed = cur.is_some() != found.is_some();
            *cur = found.clone();
            changed
        };
        if changed {
            self.layout.lock().expect("layout lock").invalidate();
            self.changes.changed(app, true);
        }
        found
    }

    pub fn save_settings(&self) -> AppResult<()> {
        let s = self.settings.read().expect("settings lock");
        s.save(&self.paths.settings_file())?;
        Ok(())
    }

    /// The open library, or `no_library`.
    pub fn library(&self) -> AppResult<Arc<Library>> {
        self.library
            .read()
            .expect("library lock")
            .clone()
            .ok_or_else(|| AppError::from(photos_core::CoreError::NoLibrary))
    }

    /// Install `lib` as current, record it in the registry and persist.
    pub fn adopt_library(&self, lib: Library) -> AppResult<LibraryInfo> {
        let info = lib.info()?;
        {
            let mut reg = self.registry.write().expect("registry lock");
            reg.upsert(&info.id, &info.name, Path::new(&info.path), now_unix());
            reg.save(&self.paths.registry_file())?;
        }
        *self.library.write().expect("library lock") = Some(Arc::new(lib));
        *self.library_problem.write().expect("library problem lock") = None;
        self.layout.lock().expect("layout lock").invalidate();
        // Asset ids are per library: another library's photo 7 is not this one's.
        self.develop.clear();
        Ok(info)
    }

    pub fn current_library_info(&self) -> AppResult<Option<LibraryInfo>> {
        let guard = self.library.read().expect("library lock");
        guard.as_ref().map(|l| l.info().map_err(AppError::from)).transpose()
    }

    /// The Hidden folder's grid only while it is open; asking keeps it open.
    pub fn check_vault(&self, scope: &photos_core::scope::LayoutScope) -> AppResult<()> {
        if !scope.vault {
            return Ok(());
        }
        if !self.vault_lock.is_unlocked() {
            return Err(AppError { code: "vault_locked".into(), message: "The Hidden folder is locked.".into(), detail: None });
        }
        self.vault_lock.touch();
        Ok(())
    }

    /// Layout for `key`, reusing the scope's index while the library version is unchanged.
    pub fn layout_for(&self, key: LayoutKey) -> AppResult<Arc<Layout>> {
        let lib = self.library()?;
        self.check_vault(&key.scope)?;
        let version = self.changes.current();
        let mut cache = self.layout.lock().expect("layout lock");
        if cache.version != version {
            cache.invalidate();
            cache.version = version;
        }
        if let Some(l) = cache.computed.get(&key) {
            return Ok(l.clone());
        }
        let index = match cache.indexes.get(&key.scope) {
            Some(i) => i.clone(),
            None => {
                let ffmpeg = self.ffmpeg_available();
                // A query's text also finds photos by what they show (Phase 7b).
                let scope = crate::search::understood(self, &key.scope);
                let i = Arc::new(lib.db.with_conn(|c| LayoutIndex::load_scoped(c, &scope, ffmpeg))?);
                if cache.indexes.len() > 6 {
                    cache.indexes.clear();
                }
                cache.indexes.insert(key.scope.clone(), i.clone());
                i
            }
        };
        let layout = Arc::new(index.compute(key.width, key.row_height, key.gap, key.scope.sectioned().then_some(key.group_by)));
        if cache.computed.len() > 12 {
            cache.computed.clear();
        }
        cache.computed.insert(key, layout.clone());
        Ok(layout)
    }
}

#[cfg(test)]
mod problem_tests {
    use super::*;

    #[test]
    fn a_refused_library_is_a_state_not_a_panic() {
        let t = tempfile::tempdir().unwrap();
        let root = t.path().join("lib");
        std::fs::create_dir_all(&root).unwrap();
        let conn = rusqlite::Connection::open(root.join("library.db")).unwrap();
        conn.execute_batch("CREATE TABLE schema_version (version INTEGER NOT NULL, applied_at INTEGER NOT NULL); INSERT INTO schema_version VALUES (9999, 1);").unwrap();
        drop(conn);
        let (lib, problem) = open_current(&root, &t.path().join("backups"));
        assert!(lib.is_none());
        let p = problem.expect("a problem to show");
        assert_eq!(p.code, "library_too_new");
        assert!(p.message.contains("newer Lumen"), "{}", p.message);
        assert_eq!(p.path, root.display().to_string());
    }

    #[test]
    fn a_library_that_opens_has_no_problem() {
        let t = tempfile::tempdir().unwrap();
        let root = t.path().join("lib");
        drop(Library::create(&root).unwrap());
        let (lib, problem) = open_current(&root, &t.path().join("backups"));
        assert!(lib.is_some());
        assert_eq!(problem, None);
    }
}
