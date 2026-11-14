//! Updates (Round 17). `tauri-plugin-updater` checks the channel's manifest, verifies the
//! signature and runs the installer; this file decides when, and keeps the file between.
//!
//! Lumen streams the installer to `updates/update-<version>.bin` instead of using the plugin's
//! in-memory download: a large file is not held in RAM on a slow line, and a dropped connection
//! resumes. `Update::verify` checks the complete file once. At Close, `RunEvent::Exit` restores
//! the `Update` from the saved manifest with no network and `Update::install` verifies again and
//! starts the installer, after Develop's flush and the import hold, with the window gone.
//!
//! Failures are states, never errors, and the check sends no identifier.

use std::collections::HashSet;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use photos_core::settings::{UpdateChannel, UpdateSettings};
use semver::Version;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_updater::UpdaterExt;
use ts_rs::TS;

use crate::error::{AppError, AppResult};
use crate::state::AppState;

/// Where the channels' manifests live when nothing overrides it.
const DEFAULT_CHANNELS_BASE: &str = "https://github.com/05Narwhal/lumen/releases/download/channels";

/// Where the channels' manifests live. A test build is compiled with its own local address.
const CHANNELS_BASE: &str = match option_env!("LUMEN_UPDATE_TEST_BASE") {
    Some(base) => base,
    None => DEFAULT_CHANNELS_BASE,
};

/// What this Lumen says it is to the update server: no version, no id.
pub const USER_AGENT: &str = "Lumen-updater";

/// `updates://state`: the new [`UpdateStatus`].
pub const STATE_EVENT: &str = "updates://state";

/// The first check waits for startup to settle; then one every six hours while Lumen is open.
const SETTLE: Duration = Duration::from_secs(90);
const EVERY: Duration = Duration::from_secs(6 * 3600);
/// Breaks a download survives before it is a failure (each resumes where it stopped).
const ATTEMPTS: usize = 5;
/// No installer is anywhere near this; a manifest pointing at more is not followed.
const MAX_BYTES: u64 = 2 << 30;

fn rank(c: UpdateChannel) -> u8 {
    match c {
        UpdateChannel::Alpha => 0,
        UpdateChannel::Beta => 1,
        UpdateChannel::Stable => 2,
    }
}

fn channel_name(c: UpdateChannel) -> &'static str {
    match c {
        UpdateChannel::Alpha => "alpha",
        UpdateChannel::Beta => "beta",
        UpdateChannel::Stable => "stable",
    }
}

/// The channel a build belongs to: its pre-release tag, and nothing else. A plain `26.1.0` is
/// never published (semver ranks it above `-stable`), so it belongs to none.
pub fn channel_of(version: &Version) -> Option<UpdateChannel> {
    match version.pre.as_str() {
        "alpha" => Some(UpdateChannel::Alpha),
        "beta" => Some(UpdateChannel::Beta),
        "stable" => Some(UpdateChannel::Stable),
        _ => None,
    }
}

/// Whether a person on `chosen` takes a build of `build`'s channel.
pub fn accepts(chosen: UpdateChannel, build: UpdateChannel) -> bool {
    rank(build) >= rank(chosen)
}

/// Whether `candidate` is an update for a person on `chosen` running `current`. Never a downgrade:
/// someone moving from Alpha to Stable waits until Stable passes them.
pub fn is_update(current: &Version, candidate: &Version, chosen: UpdateChannel) -> bool {
    channel_of(candidate).is_some_and(|c| accepts(chosen, c)) && candidate > current
}

pub fn manifest_url(channel: UpdateChannel) -> String {
    format!("{CHANNELS_BASE}/{}.json", channel_name(channel))
}

/// Why something failed, in the words Settings uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum FailWhy {
    Offline,
    /// The network works but the update server turned the connection away.
    NoServer,
    CouldNotCheck,
    BadSignature,
    NoSpace,
    Other,
}

/// Where the updater is. Settings shows one line per state.
#[derive(Debug, Clone, Default, PartialEq, Serialize, TS)]
#[serde(tag = "state", rename_all = "camelCase", rename_all_fields = "camelCase")]
#[ts(export)]
pub enum UpdateState {
    /// A development build: never checks.
    Off,
    /// This build has no public key, so it could never verify anything.
    NotSetUp,
    #[default]
    Idle,
    Checking,
    UpToDate {
        #[ts(type = "number")]
        checked_at: i64,
    },
    /// Found with automatic updates off: *Download* offers it.
    Available { version: String, notes: String },
    Downloading {
        version: String,
        #[ts(type = "number")]
        done: u64,
        #[ts(type = "number")]
        total: u64,
    },
    /// Staged: installs on Close. `attempts` counts installs that started and did not finish.
    Ready { version: String, notes: String, attempts: u32 },
    Failed {
        why: FailWhy,
        #[ts(type = "number")]
        at: i64,
    },
}

/// The updated version's notes, shown once after updating.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct UpdateReceipt {
    pub version: String,
    pub notes: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct UpdateStatus {
    pub version: String,
    pub state: UpdateState,
    /// Set for the session after an update, for Settings to keep showing the notes.
    pub whats_new: Option<UpdateReceipt>,
}

/// None when updates may run; otherwise the state that says why not.
pub fn gate(debug: bool, pubkey: &str) -> Option<UpdateState> {
    if debug {
        Some(UpdateState::Off)
    } else if pubkey.trim().is_empty() {
        Some(UpdateState::NotSetUp)
    } else {
        None
    }
}

/// A downloaded update waiting for Close, in `updates/pending.json`. The file name is derived from
/// the version, never stored, so nothing read from disk can point elsewhere.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Staged {
    pub version: String,
    pub notes: String,
    /// The channel's manifest as it arrived: `Updater::restore_update` rebuilds the `Update` from
    /// it at Close, with no network.
    pub raw_json: serde_json::Value,
    /// Downloaded by itself, not because the person asked: turning *Automatic* off discards it.
    pub automatic: bool,
    pub attempts: u32,
}

const PENDING: &str = "pending.json";
const PENDING_TMP: &str = "pending.json.tmp";
const LAST_VERSION: &str = "last-version";

pub fn stage_path(dir: &Path, version: &str) -> PathBuf {
    dir.join(format!("update-{version}.bin"))
}

fn part_path(dir: &Path, version: &str) -> PathBuf {
    dir.join(format!("update-{version}.bin.part"))
}

impl Staged {
    /// What is staged, or `None`: no file, a broken one, or a version with no installer beside it.
    pub fn load(dir: &Path) -> Option<Staged> {
        let s: Staged = serde_json::from_slice(&std::fs::read(dir.join(PENDING)).ok()?).ok()?;
        channel_of(&Version::parse(&s.version).ok()?)?;
        stage_path(dir, &s.version).is_file().then_some(s)
    }

    /// Written through a temporary file, so a crash never leaves half of it.
    pub fn save(&self, dir: &Path) -> std::io::Result<()> {
        std::fs::create_dir_all(dir)?;
        let tmp = dir.join(PENDING_TMP);
        std::fs::write(&tmp, serde_json::to_vec_pretty(self)?)?;
        std::fs::rename(tmp, dir.join(PENDING))
    }
}

/// Removes what the updater made, except the installer of `keep` (and then the record of it too).
/// Only its own names, ever: `pending.json` and `update-<version>.bin[.part]`.
pub fn clear_stage(dir: &Path, keep: Option<&str>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        let is_version = |v: &str| Version::parse(v).is_ok();
        let remove = if name == PENDING || name == PENDING_TMP {
            keep.is_none()
        } else if let Some(rest) = name.strip_prefix("update-") {
            match (rest.strip_suffix(".bin"), rest.strip_suffix(".bin.part")) {
                (Some(v), _) => is_version(v) && keep != Some(v),
                (_, Some(v)) => is_version(v),
                _ => false,
            }
        } else {
            false
        };
        if remove {
            if let Err(err) = std::fs::remove_file(e.path()) {
                tracing::warn!(file = %name, "updates: could not remove: {err}");
            }
        }
    }
}

/// The first launch after an update: which version, with the notes staged for it. `None` on the
/// very first run, on the same version, and after a downgrade.
pub fn receipt_for(last_seen: Option<&str>, current: &str, staged: Option<&Staged>) -> Option<UpdateReceipt> {
    let (last, now) = (Version::parse(last_seen?.trim()).ok()?, Version::parse(current).ok()?);
    (now > last).then(|| UpdateReceipt {
        version: current.into(),
        notes: staged.filter(|s| s.version == current).map(|s| s.notes.clone()).unwrap_or_default(),
    })
}

/// One running process, for [`other_lumen_open`].
#[derive(Debug, Clone, PartialEq)]
pub struct Proc {
    pub pid: u32,
    pub exe: String,
    pub args: Vec<String>,
}

/// Whether another window of this same program is open. The installer force-closes everything
/// running from the file it replaces, which is harmless for `--mcp` bridges (their client starts
/// them again) and not for a window with edits in it.
pub fn other_lumen_open(me: u32, exe: &str, procs: &[Proc]) -> bool {
    let norm = |s: &str| {
        let s = s.replace('/', "\\");
        if cfg!(windows) {
            s.to_lowercase()
        } else {
            s
        }
    };
    let exe = norm(exe);
    procs.iter().any(|p| p.pid != me && norm(&p.exe) == exe && !p.args.iter().any(|a| a == "--mcp"))
}

/// How long after Close the install may still start: until the exit guard would have ended the
/// process. Once it starts, [`InstallRunning`] holds the guard back, so it is never cut off.
/// A staged, allowed update lets the exit guard wait `INSTALL_GRACE` (see [`install_expected`]), so
/// the install may start well after the short grace, with room left for the installer to launch.
pub const LATE_CLOSE: Duration = Duration::from_secs(crate::window::INSTALL_GRACE.as_secs() - 5);

/// Why this Close does not install, if so. Each case retries at the next Close.
pub fn install_allowed(unsaved: bool, imports: usize, other_lumen: bool, since_close: Duration) -> Result<(), &'static str> {
    if unsaved {
        Err("an edit was still being saved")
    } else if imports > 0 {
        Err("an import was still running")
    } else if other_lumen {
        Err("another Lumen window is open")
    } else if since_close > LATE_CLOSE {
        Err("closing took too long to install safely")
    } else {
        Ok(())
    }
}

/// Whether what is staged still stands after the settings changed: a build the new channel doesn't
/// take goes, and so does one downloaded by itself when *Automatic* is turned off.
pub fn keeps_staged(staged: &Staged, new: &UpdateSettings) -> bool {
    let Some(c) = Version::parse(&staged.version).ok().as_ref().and_then(channel_of) else { return false };
    accepts(new.channel, c) && (new.automatic || !staged.automatic)
}

/// What a failed check means.
pub enum CheckFail {
    /// The manifest names no build for this platform: for this person, nothing to update to.
    NoBuildHere,
    Why(FailWhy),
}

/// The plugin reports any non-2xx answer as `ReleaseNotFound`, so a rate limit looks like a
/// missing file. Every channel's manifest exists from the first publish on, so it is read as
/// *could not check*, quietly, and retried at the next check.
pub fn check_fail(e: &tauri_plugin_updater::Error) -> CheckFail {
    use tauri_plugin_updater::Error as E;
    match e {
        E::TargetNotFound(_) | E::TargetsNotFound(_) => CheckFail::NoBuildHere,
        E::Reqwest(r) if r.is_decode() => CheckFail::Why(FailWhy::CouldNotCheck),
        E::Reqwest(r) if turned_away(r) => CheckFail::Why(FailWhy::NoServer),
        E::Reqwest(_) => CheckFail::Why(FailWhy::Offline),
        E::ReleaseNotFound | E::Network(_) | E::Serialization(_) | E::Semver(_) => CheckFail::Why(FailWhy::CouldNotCheck),
        _ => CheckFail::Why(FailWhy::Other),
    }
}

/// Whether the other end answered "no" at the connection (refused or reset): the network is up and
/// the server is not. DNS failures, timeouts and unreachable networks are not that — they are what
/// being offline looks like.
fn turned_away(e: &(dyn std::error::Error + 'static)) -> bool {
    let mut cur: Option<&(dyn std::error::Error + 'static)> = Some(e);
    while let Some(err) = cur {
        if let Some(io) = err.downcast_ref::<std::io::Error>() {
            if matches!(io.kind(), std::io::ErrorKind::ConnectionRefused | std::io::ErrorKind::ConnectionReset | std::io::ErrorKind::ConnectionAborted) {
                return true;
            }
        }
        cur = err.source();
    }
    false
}

/// Whether a failed install means the file can't be trusted, so it is deleted.
pub fn is_bad_signature(e: &tauri_plugin_updater::Error) -> bool {
    use tauri_plugin_updater::Error as E;
    matches!(e, E::Minisign(_) | E::Base64(_) | E::SignatureUtf8(_) | E::SignedVersionMismatch { .. } | E::MissingSignedVersion)
}

/// What Settings shows for something staged.
pub fn ready_state(staged: &Staged) -> UpdateState {
    UpdateState::Ready { version: staged.version.clone(), notes: staged.notes.clone(), attempts: staged.attempts }
}

/// The client for downloads: no total timeout, since a large file on a slow line is not a failure.
pub fn update_client() -> reqwest::Client {
    reqwest::Client::builder()
        .user_agent(USER_AGENT)
        .connect_timeout(Duration::from_secs(15))
        .read_timeout(Duration::from_secs(60))
        .build()
        .unwrap_or_default()
}

/// Why a download stopped.
#[derive(Debug)]
pub enum FetchError {
    Http(u16),
    Network(String),
    Io(std::io::Error),
    NoSpace,
    TooBig,
    Cancelled,
}

impl std::fmt::Display for FetchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FetchError::Http(code) => write!(f, "the server answered {code}"),
            FetchError::Network(why) => write!(f, "network: {why}"),
            FetchError::Io(e) => write!(f, "disk: {e}"),
            FetchError::NoSpace => write!(f, "not enough free space"),
            FetchError::TooBig => write!(f, "larger than any installer"),
            FetchError::Cancelled => write!(f, "stopped"),
        }
    }
}

impl FetchError {
    pub fn why(&self) -> FailWhy {
        match self {
            FetchError::Http(_) => FailWhy::CouldNotCheck,
            FetchError::Network(_) => FailWhy::Offline,
            FetchError::NoSpace => FailWhy::NoSpace,
            FetchError::Io(e) if e.kind() == std::io::ErrorKind::StorageFull => FailWhy::NoSpace,
            _ => FailWhy::Other,
        }
    }
}

/// Downloads `url` to `part`, resuming what is already there, and asks for what remains with
/// `Range` after a break. `on_progress(done, total)` returning false stops it. `free` is the disk's
/// free space when known: a file that can't fit stops before anything is written.
pub async fn fetch_to_part(
    client: &reqwest::Client,
    url: &str,
    part: &Path,
    free: Option<u64>,
    on_progress: &mut (dyn FnMut(u64, u64) -> bool + Send),
) -> Result<(), FetchError> {
    let mut last = FetchError::Network("no attempt made".into());
    for _ in 0..ATTEMPTS {
        let have = std::fs::metadata(part).map(|m| m.len()).unwrap_or(0);
        let mut req = client.get(url);
        if have > 0 {
            req = req.header(reqwest::header::RANGE, format!("bytes={have}-"));
        }
        let mut resp = match req.send().await {
            Ok(r) => r,
            Err(e) => {
                last = FetchError::Network(e.to_string());
                continue;
            }
        };
        let status = resp.status();
        if status == reqwest::StatusCode::RANGE_NOT_SATISFIABLE {
            // What was kept no longer matches the file: start over.
            let _ = std::fs::remove_file(part);
            last = FetchError::Http(status.as_u16());
            continue;
        }
        if !status.is_success() {
            return Err(FetchError::Http(status.as_u16()));
        }
        // 206 continues the file; a 200 is the whole file again (a server that ignores ranges).
        let append = status == reqwest::StatusCode::PARTIAL_CONTENT;
        let start = if append { have } else { 0 };
        let rest = resp.content_length();
        let total = rest.map(|n| start + n).unwrap_or(0);
        if total > MAX_BYTES {
            return Err(FetchError::TooBig);
        }
        if let (Some(rest), Some(free)) = (rest, free) {
            if rest > free {
                return Err(FetchError::NoSpace);
            }
        }
        if let Some(dir) = part.parent() {
            std::fs::create_dir_all(dir).map_err(FetchError::Io)?;
        }
        let mut file = std::fs::OpenOptions::new().create(true).write(true).append(append).truncate(!append).open(part).map_err(FetchError::Io)?;
        let mut done = start;
        if !on_progress(done, total.max(done)) {
            return Err(FetchError::Cancelled);
        }
        loop {
            match resp.chunk().await {
                Ok(Some(chunk)) => {
                    file.write_all(&chunk).map_err(FetchError::Io)?;
                    done += chunk.len() as u64;
                    if done > MAX_BYTES {
                        drop(file);
                        let _ = std::fs::remove_file(part);
                        return Err(FetchError::TooBig);
                    }
                    if !on_progress(done, total.max(done)) {
                        return Err(FetchError::Cancelled);
                    }
                }
                Ok(None) => {
                    file.sync_all().map_err(FetchError::Io)?;
                    if total == 0 || done >= total {
                        return Ok(());
                    }
                    last = FetchError::Network(format!("ended at {done} of {total} bytes"));
                    break;
                }
                Err(e) => {
                    last = FetchError::Network(e.to_string());
                    break;
                }
            }
        }
    }
    Err(last)
}

/// What the updater holds while Lumen runs.
#[derive(Default)]
pub struct Updates {
    state: Mutex<UpdateState>,
    busy: AtomicBool,
    install_then_restart: AtomicBool,
    offered: Mutex<Option<tauri_plugin_updater::Update>>,
    /// Versions that failed their signature this session: not fetched again.
    rejected: Mutex<HashSet<String>>,
    receipt: Mutex<Option<UpdateReceipt>>,
    whats_new: Mutex<Option<UpdateReceipt>>,
}

/// When Close went through (`window::begin_shutdown`).
static CLOSED_AT: OnceLock<Instant> = OnceLock::new();

pub fn note_closed() {
    let _ = CLOSED_AT.set(Instant::now());
}

fn shutting_down() -> bool {
    CLOSED_AT.get().is_some()
}

static INSTALL_RUNNING: AtomicBool = AtomicBool::new(false);

/// Whether an update is being handed to its installer, for the exit guard.
pub fn install_running() -> bool {
    INSTALL_RUNNING.load(Ordering::SeqCst)
}

static INSTALL_EXPECTED: AtomicBool = AtomicBool::new(false);

/// Whether Close found an update staged and allowed to install, so a slow Close must not be ended
/// by the short exit grace before `RunEvent::Exit` gets to install it.
pub fn install_expected() -> bool {
    INSTALL_EXPECTED.load(Ordering::SeqCst)
}

pub fn expect_install(on: bool) {
    INSTALL_EXPECTED.store(on, Ordering::SeqCst);
}

/// What the exit guard asks: an install is running, or is about to.
pub fn install_pending() -> bool {
    install_running() || install_expected()
}

/// Whether an update will install at this Close: the gate is open and what is staged still stands.
pub fn install_expected_at_close(gate_open: bool, staged: Option<&Staged>, settings: &UpdateSettings) -> bool {
    gate_open && staged.is_some_and(|s| keeps_staged(s, settings))
}

/// Whether this exit installs what is staged. Never when Lumen Setup's `--quit` closed Lumen
/// (Round 18 A2): Setup is installing, and two installers at once would fight over the folder.
pub fn installs_at_this_exit(for_setup: bool, gate_open: bool, staged: Option<&Staged>, settings: &UpdateSettings) -> bool {
    !for_setup && install_expected_at_close(gate_open, staged, settings)
}

/// Why `RunEvent::Exit` installs nothing even with an update staged; the stage is kept either way.
pub fn exit_install_refused(for_setup: bool) -> Option<&'static str> {
    for_setup.then_some("Lumen Setup is installing")
}

/// Called as Close begins, on the main thread: reads what is staged and tells the exit guard.
/// Busy settings count as not expected (the short grace), never as a wait.
pub fn note_close_install(app: &AppHandle) {
    let state = app.state::<AppState>();
    let settings = state.settings.try_read().map(|s| s.updates.clone()).ok();
    let for_setup = crate::window::quitting_for_setup();
    let expected = match settings {
        Some(s) => installs_at_this_exit(for_setup, gate_for(app).is_none(), Staged::load(&dir_of(app)).as_ref(), &s),
        None => false,
    };
    expect_install(expected);
}

/// Holds the exit guard back while it lives.
struct InstallRunning;

impl InstallRunning {
    fn begin() -> Self {
        INSTALL_RUNNING.store(true, Ordering::SeqCst);
        InstallRunning
    }
}

impl Drop for InstallRunning {
    fn drop(&mut self) {
        INSTALL_RUNNING.store(false, Ordering::SeqCst);
    }
}

/// Clears `busy` however the work ends.
struct Busy<'a>(&'a AtomicBool);

impl Drop for Busy<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

fn pubkey(app: &AppHandle) -> String {
    app.config().plugins.0.get("updater").and_then(|u| u.get("pubkey")).and_then(|k| k.as_str()).unwrap_or("").to_string()
}

fn gate_for(app: &AppHandle) -> Option<UpdateState> {
    gate(cfg!(debug_assertions), &pubkey(app))
}

fn dir_of(app: &AppHandle) -> PathBuf {
    app.state::<AppState>().paths.updates_dir()
}

fn settings_now(app: &AppHandle) -> UpdateSettings {
    app.state::<AppState>().settings.read().unwrap_or_else(|p| p.into_inner()).updates.clone()
}

pub fn status(app: &AppHandle) -> UpdateStatus {
    let u = app.state::<Updates>();
    let (state, whats_new) = (lock(&u.state).clone(), lock(&u.whats_new).clone());
    UpdateStatus { version: env!("CARGO_PKG_VERSION").into(), state, whats_new }
}

fn set_state(app: &AppHandle, s: UpdateState) {
    *lock(&app.state::<Updates>().state) = s;
    let _ = app.emit(STATE_EVENT, status(app));
}

fn failed(app: &AppHandle, why: FailWhy) {
    set_state(app, UpdateState::Failed { why, at: photos_core::now_unix() });
}

/// What Lumen adds to the installer's command line. `/P`: with `installMode: quiet` (`/S`) it skips
/// the reinstall page, which otherwise waits for a click nobody can see. `/LUMENPID=`: the hook in
/// `windows/installer-hooks.nsh` waits for this process to end before the template's running-app
/// check, whose forced shutdown aborts the install silently when it meets a Lumen still exiting.
pub fn installer_args(pid: u32) -> [String; 2] {
    ["/P".into(), format!("/LUMENPID={pid}")]
}

fn build(app: &AppHandle, channel: UpdateChannel, restart: bool) -> tauri_plugin_updater::Result<tauri_plugin_updater::Updater> {
    let url = tauri::Url::parse(&manifest_url(channel))?;
    let b = app
        .updater_builder()
        .endpoints(vec![url])?
        .version_comparator(move |current, release| is_update(&current, &release.version, channel))
        .timeout(Duration::from_secs(30))
        .installer_args(installer_args(std::process::id()));
    #[cfg(windows)]
    let b = b.restart_after_install(restart);
    #[cfg(not(windows))]
    let _ = restart;
    b.build()
}

/// At launch: the receipt after an update, what is still staged, and the checks every six hours.
pub fn start(app: AppHandle) {
    let dir = dir_of(&app);
    let current = env!("CARGO_PKG_VERSION");
    let mut staged = Staged::load(&dir);
    let last = std::fs::read_to_string(dir.join(LAST_VERSION)).ok();
    let receipt = receipt_for(last.as_deref(), current, staged.as_ref());
    if let Some(r) = &receipt {
        tracing::info!(from = last.as_deref().unwrap_or("?").trim(), to = %r.version, "updated");
    }
    // Staged and no newer than this build: installed (or overtaken). Or no longer wanted.
    if let Some(s) = &staged {
        let installed = Version::parse(&s.version).ok().zip(Version::parse(current).ok()).is_none_or(|(s, c)| s <= c);
        if installed || !keeps_staged(s, &settings_now(&app)) {
            clear_stage(&dir, None);
            staged = None;
        }
    }
    if receipt.is_some() {
        clear_stage(&dir, None);
    }
    if std::fs::create_dir_all(&dir).and_then(|_| std::fs::write(dir.join(LAST_VERSION), current)).is_err() {
        tracing::warn!("updates: could not record this version");
    }
    let u = app.state::<Updates>();
    *lock(&u.receipt) = receipt.clone();
    *lock(&u.whats_new) = receipt;
    let gated = gate_for(&app);
    *lock(&u.state) = gated.clone().or_else(|| staged.as_ref().map(ready_state)).unwrap_or_default();
    if gated.is_some() {
        return;
    }
    let _ = std::thread::Builder::new().name("lumen-updates".into()).spawn(move || {
        photos_jobs::background_priority();
        std::thread::sleep(SETTLE);
        loop {
            if shutting_down() {
                return;
            }
            if settings_now(&app).automatic {
                tauri::async_runtime::block_on(check(&app, false));
            }
            std::thread::sleep(EVERY);
        }
    });
}

/// Asks the channel's manifest for a newer build. With automatic updates on it is downloaded at
/// once; off, it is offered. `by_person`: *Check for updates* rather than the schedule.
pub async fn check(app: &AppHandle, by_person: bool) {
    if let Some(s) = gate_for(app) {
        set_state(app, s);
        return;
    }
    let u = app.state::<Updates>();
    if u.busy.swap(true, Ordering::SeqCst) {
        return;
    }
    let _busy = Busy(&u.busy);
    let settings = settings_now(app);
    let dir = dir_of(app);
    let staged = Staged::load(&dir).filter(|s| keeps_staged(s, &settings));
    set_state(app, UpdateState::Checking);
    let found = match build(app, settings.channel, false) {
        Ok(up) => up.check().await,
        Err(e) => Err(e),
    };
    match found {
        Ok(Some(update)) => {
            let version = update.version.clone();
            if let Some(s) = staged.as_ref().filter(|s| s.version == version) {
                set_state(app, ready_state(s));
            } else if lock(&u.rejected).contains(&version) {
                failed(app, FailWhy::BadSignature);
            } else if settings.automatic {
                *lock(&u.offered) = Some(update.clone());
                download(app, update, !by_person).await;
            } else {
                let notes = update.body.clone().unwrap_or_default();
                *lock(&u.offered) = Some(update);
                set_state(app, UpdateState::Available { version, notes });
            }
        }
        Ok(None) => set_state(app, staged.as_ref().map(ready_state).unwrap_or(UpdateState::UpToDate { checked_at: photos_core::now_unix() })),
        Err(e) => match check_fail(&e) {
            CheckFail::NoBuildHere => set_state(app, staged.as_ref().map(ready_state).unwrap_or(UpdateState::UpToDate { checked_at: photos_core::now_unix() })),
            CheckFail::Why(why) => {
                tracing::info!("update check did not finish: {e}");
                match &staged {
                    Some(s) => set_state(app, ready_state(s)),
                    None => failed(app, why),
                }
            }
        },
    }
}

/// *Download*: the build the last check offered, in the background.
pub fn download_offered(app: &AppHandle) {
    let u = app.state::<Updates>();
    let Some(update) = lock(&u.offered).clone() else { return };
    if u.busy.swap(true, Ordering::SeqCst) {
        return;
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let u = app.state::<Updates>();
        let _busy = Busy(&u.busy);
        download(&app, update, false).await;
    });
}

/// Streams the installer to a staged file, verifies it, and keeps it for Close. Its line shows in
/// the activity ring while it runs.
async fn download(app: &AppHandle, update: tauri_plugin_updater::Update, automatic: bool) {
    let dir = dir_of(app);
    let version = update.version.clone();
    if std::fs::create_dir_all(&dir).is_err() {
        failed(app, FailWhy::Other);
        return;
    }
    let part = part_path(&dir, &version);
    let free = fs4::available_space(&dir).ok();
    let activity = photos_jobs::activity::start("update", &format!("Downloading Lumen {version}"), 0);
    let (mut total_seen, mut done_seen) = (0u64, 0u64);
    let mut last_told = Instant::now() - Duration::from_secs(1);
    let ui = app.clone();
    let v = version.clone();
    let got = fetch_to_part(&update_client(), update.download_url.as_str(), &part, free, &mut |done, total| {
        if total > total_seen {
            activity.add_total(total - total_seen);
            total_seen = total;
        }
        if done > done_seen {
            activity.advance(done - done_seen);
            done_seen = done;
        }
        if last_told.elapsed() > Duration::from_millis(500) {
            last_told = Instant::now();
            set_state(&ui, UpdateState::Downloading { version: v.clone(), done, total });
        }
        !shutting_down()
    })
    .await;
    drop(activity);
    match got {
        Ok(()) => {}
        Err(FetchError::Cancelled) => return,
        Err(e) => {
            tracing::warn!(version = %version, "update download did not finish: {e}");
            if matches!(e, FetchError::Http(_) | FetchError::TooBig) {
                let _ = std::fs::remove_file(&part);
            }
            failed(app, e.why());
            return;
        }
    }
    let checked = match std::fs::read(&part) {
        Ok(bytes) => update.verify(&bytes),
        Err(e) => {
            tracing::warn!("update: could not read the download: {e}");
            failed(app, FailWhy::Other);
            return;
        }
    };
    if let Err(e) = checked {
        let _ = std::fs::remove_file(&part);
        if is_bad_signature(&e) {
            tracing::error!(version = %version, "update refused, its signature does not verify; the file was deleted: {e}");
            lock(&app.state::<Updates>().rejected).insert(version);
            failed(app, FailWhy::BadSignature);
        } else {
            tracing::warn!(version = %version, "update could not be verified: {e}");
            failed(app, FailWhy::Other);
        }
        return;
    }
    let staged_file = stage_path(&dir, &version);
    let s = Staged { version: version.clone(), notes: update.body.clone().unwrap_or_default(), raw_json: update.raw_json.clone(), automatic, attempts: 0 };
    if std::fs::rename(&part, &staged_file).and_then(|_| s.save(&dir)).is_err() {
        let _ = std::fs::remove_file(&staged_file);
        failed(app, FailWhy::Other);
        return;
    }
    clear_stage(&dir, Some(&version));
    tracing::info!(version = %version, "update downloaded and verified; it installs at the next close");
    set_state(app, ready_state(&s));
}

/// Settings changed: what is staged and offered must still be wanted, and a new channel is asked.
pub fn settings_changed(app: &AppHandle, old: &UpdateSettings, new: &UpdateSettings) {
    if old.channel == new.channel && old.automatic == new.automatic {
        return;
    }
    let dir = dir_of(app);
    let u = app.state::<Updates>();
    if let Some(s) = Staged::load(&dir) {
        if !keeps_staged(&s, new) {
            tracing::info!(version = %s.version, "the staged update is no longer wanted; removed");
            clear_stage(&dir, None);
            set_state(app, UpdateState::Idle);
        }
    }
    if old.channel != new.channel {
        *lock(&u.offered) = None;
        let stale = matches!(*lock(&u.state), UpdateState::Available { .. } | UpdateState::UpToDate { .. } | UpdateState::Failed { .. });
        if stale {
            set_state(app, UpdateState::Idle);
        }
    }
    if gate_for(app).is_none() && (old.channel != new.channel || (!old.automatic && new.automatic)) {
        let app = app.clone();
        tauri::async_runtime::spawn(async move { check(&app, false).await });
    }
}

/// *Install now*: the normal close (Develop's flush and the import question still run), then the
/// install, which opens the new version.
pub fn install_now(app: &AppHandle) -> AppResult<()> {
    if Staged::load(&dir_of(app)).is_none() {
        return Err(AppError { code: "no_update".into(), message: "No update is ready to install.".into(), detail: None });
    }
    app.state::<Updates>().install_then_restart.store(true, Ordering::SeqCst);
    if let Some(w) = app.get_webview_window("main") {
        w.close()?;
    }
    Ok(())
}

pub fn take_receipt(app: &AppHandle) -> Option<UpdateReceipt> {
    lock(&app.state::<Updates>().receipt).take()
}

/// Every process, for [`other_lumen_open`].
pub(crate) fn processes() -> Vec<Proc> {
    use sysinfo::{ProcessRefreshKind, RefreshKind, System, UpdateKind};
    let sys = System::new_with_specifics(RefreshKind::nothing().with_processes(ProcessRefreshKind::nothing().with_exe(UpdateKind::Always).with_cmd(UpdateKind::Always)));
    sys.processes()
        .values()
        .map(|p| Proc {
            pid: p.pid().as_u32(),
            exe: p.exe().map(|e| e.to_string_lossy().into_owned()).unwrap_or_default(),
            args: p.cmd().iter().map(|a| a.to_string_lossy().into_owned()).collect(),
        })
        .collect()
}

/// `RunEvent::Exit`: the window is gone, Develop has written and no import runs. Installs what is
/// staged, verified again, unless something says not this time; it then tries at the next Close.
/// Nothing is waited on, and the exit guard gives this `INSTALL_GRACE` rather than `EXIT_GRACE`.
pub fn install_on_exit(app: &AppHandle) {
    install_staged_on_exit(app);
    // Whatever happened, the wait promised at Close is over: an install that did not start must
    // not keep a stuck exit alive for the long grace.
    expect_install(false);
}

fn install_staged_on_exit(app: &AppHandle) {
    // A `--quit` launch that found no Lumen to close ends in `setup` before the state exists.
    if app.try_state::<AppState>().is_none() {
        return;
    }
    if gate_for(app).is_some() {
        return;
    }
    if let Some(why) = exit_install_refused(crate::window::quitting_for_setup()) {
        tracing::info!("the staged update is not installed at this exit: {why}");
        return;
    }
    let dir = dir_of(app);
    let Some(mut staged) = Staged::load(&dir) else { return };
    let _running = InstallRunning::begin();
    let restart = app.state::<Updates>().install_then_restart.load(Ordering::SeqCst);
    let state = app.state::<AppState>();
    let since_close = CLOSED_AT.get().map(|t| t.elapsed()).unwrap_or_default();
    let imports = state.imports.try_lock().map(|m| m.len()).unwrap_or(1);
    let unsaved = state.unsaved_edits.load(Ordering::SeqCst);
    let other = std::env::current_exe().map(|exe| other_lumen_open(std::process::id(), &exe.to_string_lossy(), &processes())).unwrap_or(true);
    if let Err(why) = install_allowed(unsaved, imports, other, since_close) {
        tracing::warn!(version = %staged.version, "the update waits for the next close: {why}");
        return;
    }
    let Ok(settings) = state.settings.try_read().map(|s| s.updates.clone()) else {
        tracing::warn!("the update waits for the next close: settings were busy");
        return;
    };
    if !keeps_staged(&staged, &settings) {
        clear_stage(&dir, None);
        return;
    }
    let update = match build(app, settings.channel, restart).and_then(|up| up.restore_update(staged.raw_json.clone())) {
        Ok(Some(u)) => u,
        Ok(None) => {
            clear_stage(&dir, None);
            return;
        }
        Err(e) => {
            tracing::warn!(version = %staged.version, "the staged update could not be read back: {e}");
            if is_bad_signature(&e) {
                clear_stage(&dir, None);
            }
            return;
        }
    };
    let bytes = match std::fs::read(stage_path(&dir, &staged.version)) {
        Ok(b) => b,
        Err(e) => {
            tracing::warn!("the staged update could not be read: {e}");
            return;
        }
    };
    staged.attempts += 1;
    let _ = staged.save(&dir);
    let after_close_ms = CLOSED_AT.get().map(|t| t.elapsed().as_millis() as u64).unwrap_or_default();
    tracing::info!(version = %staged.version, restart, attempt = staged.attempts, after_close_ms, "installing the update");
    match update.install(bytes) {
        Ok(()) => {
            #[cfg(not(windows))]
            if restart {
                tauri::process::restart(&app.env());
            }
        }
        Err(e) if is_bad_signature(&e) => {
            tracing::error!(version = %staged.version, "the staged update failed its signature; deleted: {e}");
            clear_stage(&dir, None);
        }
        Err(e) => tracing::warn!(version = %staged.version, "the installer did not start: {e}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(s: &str) -> Version {
        Version::parse(s).unwrap()
    }

    fn staged(version: &str, automatic: bool) -> Staged {
        Staged { version: version.into(), notes: "Notes".into(), raw_json: serde_json::json!({"version": version}), automatic, attempts: 0 }
    }

    fn settings(channel: UpdateChannel, automatic: bool) -> UpdateSettings {
        UpdateSettings { automatic, channel, ..Default::default() }
    }

    #[test]
    fn the_channel_ladder() {
        use UpdateChannel::*;
        for (chosen, build, yes) in [
            (Stable, Stable, true),
            (Stable, Beta, false),
            (Stable, Alpha, false),
            (Beta, Stable, true),
            (Beta, Beta, true),
            (Beta, Alpha, false),
            (Alpha, Stable, true),
            (Alpha, Beta, true),
            (Alpha, Alpha, true),
        ] {
            assert_eq!(accepts(chosen, build), yes, "{chosen:?} takes {build:?}");
        }
    }

    #[test]
    fn a_version_belongs_to_the_channel_it_names() {
        assert_eq!(channel_of(&v("26.1.0-alpha")), Some(UpdateChannel::Alpha));
        assert_eq!(channel_of(&v("26.1.0-beta")), Some(UpdateChannel::Beta));
        assert_eq!(channel_of(&v("26.1.0-stable")), Some(UpdateChannel::Stable));
    }

    #[test]
    fn a_plain_version_has_no_channel() {
        assert_eq!(channel_of(&v("26.1.0")), None);
        assert_eq!(channel_of(&v("26.1.0-rc.1")), None);
        assert!(!is_update(&v("26.1.0-alpha"), &v("26.2.0"), UpdateChannel::Alpha));
    }

    #[test]
    fn newer_is_an_update_and_the_same_or_older_is_not() {
        let a = UpdateChannel::Alpha;
        assert!(is_update(&v("26.1.0-alpha"), &v("26.1.1-alpha"), a));
        assert!(!is_update(&v("26.1.1-alpha"), &v("26.1.1-alpha"), a));
        assert!(!is_update(&v("26.1.2-alpha"), &v("26.1.1-alpha"), a));
    }

    #[test]
    fn moving_to_stable_never_downgrades() {
        let s = UpdateChannel::Stable;
        assert!(!is_update(&v("26.1.4-alpha"), &v("26.1.2-stable"), s));
        assert!(is_update(&v("26.1.4-alpha"), &v("26.1.5-stable"), s));
    }

    #[test]
    fn an_alpha_in_the_stable_list_is_refused() {
        assert!(!is_update(&v("26.1.0-alpha"), &v("26.2.0-alpha"), UpdateChannel::Stable));
        assert!(!is_update(&v("26.1.0-alpha"), &v("26.2.0-beta"), UpdateChannel::Stable));
        assert!(is_update(&v("26.1.0-alpha"), &v("26.2.0-beta"), UpdateChannel::Beta));
    }

    #[test]
    fn default_endpoint_is_public_repo() {
        assert_eq!(DEFAULT_CHANNELS_BASE, "https://github.com/05Narwhal/lumen/releases/download/channels");
    }

    #[test]
    fn the_endpoint_follows_the_channel() {
        let want = |name: &str| format!("{CHANNELS_BASE}/{name}.json");
        assert_eq!(manifest_url(UpdateChannel::Stable), want("stable"));
        assert_eq!(manifest_url(UpdateChannel::Beta), want("beta"));
        assert_eq!(manifest_url(UpdateChannel::Alpha), want("alpha"));
    }

    #[test]
    fn debug_builds_never_check() {
        assert_eq!(gate(true, "a key"), Some(UpdateState::Off));
        assert_eq!(gate(true, ""), Some(UpdateState::Off));
    }

    #[test]
    fn a_build_without_a_key_makes_no_call() {
        assert_eq!(gate(false, ""), Some(UpdateState::NotSetUp));
        assert_eq!(gate(false, "  "), Some(UpdateState::NotSetUp));
        assert_eq!(gate(false, "a key"), None);
    }

    #[test]
    fn a_staged_update_survives_a_round_trip() {
        let t = tempfile::tempdir().unwrap();
        let s = staged("26.1.1-alpha", true);
        std::fs::write(stage_path(t.path(), &s.version), "installer").unwrap();
        s.save(t.path()).unwrap();
        assert_eq!(Staged::load(t.path()), Some(s));
    }

    #[test]
    fn a_broken_pending_file_is_nothing_staged() {
        let t = tempfile::tempdir().unwrap();
        assert_eq!(Staged::load(t.path()), None, "no file");
        std::fs::write(t.path().join(PENDING), "{ truncated").unwrap();
        assert_eq!(Staged::load(t.path()), None, "half a file");
        // A good record whose installer has gone.
        staged("26.1.1-alpha", true).save(t.path()).unwrap();
        assert_eq!(Staged::load(t.path()), None, "no installer beside it");
        // A version that isn't one can't name a file.
        let mut odd = staged("26.1.1-alpha", true);
        odd.version = "../../evil".into();
        std::fs::write(t.path().join(PENDING), serde_json::to_vec(&odd).unwrap()).unwrap();
        assert_eq!(Staged::load(t.path()), None, "not a version");
    }

    #[test]
    fn clearing_touches_only_its_own_files() {
        let t = tempfile::tempdir().unwrap();
        for f in ["update-26.1.1-alpha.bin", "update-26.1.1-alpha.bin.part", "update-26.1.2-alpha.bin", "pending.json", "last-version", "photo.jpg", "update.txt"] {
            std::fs::write(t.path().join(f), "x").unwrap();
        }
        clear_stage(t.path(), Some("26.1.2-alpha"));
        let mut left: Vec<String> = std::fs::read_dir(t.path()).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        left.sort();
        assert_eq!(left, ["last-version", "pending.json", "photo.jpg", "update-26.1.2-alpha.bin", "update.txt"]);
        clear_stage(t.path(), None);
        let mut left: Vec<String> = std::fs::read_dir(t.path()).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        left.sort();
        assert_eq!(left, ["last-version", "photo.jpg", "update.txt"]);
    }

    #[test]
    fn clearing_a_folder_that_is_not_there_is_nothing() {
        let t = tempfile::tempdir().unwrap();
        clear_stage(&t.path().join("nope"), None);
    }

    #[test]
    fn the_receipt_fires_once_for_a_newer_version() {
        assert_eq!(receipt_for(None, "26.1.1-alpha", None), None, "first run");
        assert_eq!(receipt_for(Some("26.1.1-alpha"), "26.1.1-alpha", None), None, "same");
        assert_eq!(receipt_for(Some("26.1.2-alpha"), "26.1.1-alpha", None), None, "downgrade");
        let r = receipt_for(Some("26.1.0-alpha"), "26.1.1-alpha", Some(&staged("26.1.1-alpha", true))).unwrap();
        assert_eq!(r, UpdateReceipt { version: "26.1.1-alpha".into(), notes: "Notes".into() });
    }

    #[test]
    fn a_receipt_without_staged_notes_still_says_the_version() {
        let r = receipt_for(Some("26.1.0-alpha"), "26.1.1-alpha", None).unwrap();
        assert_eq!(r, UpdateReceipt { version: "26.1.1-alpha".into(), notes: String::new() });
        // Notes staged for another version are not this one's.
        let r = receipt_for(Some("26.1.0-alpha"), "26.1.1-alpha", Some(&staged("26.1.5-alpha", true))).unwrap();
        assert_eq!(r.notes, "");
    }

    #[test]
    fn a_broken_last_version_is_no_receipt() {
        assert_eq!(receipt_for(Some("garbage"), "26.1.1-alpha", None), None);
    }

    #[test]
    fn another_lumen_blocks_the_install() {
        let exe = r"C:\Apps\Lumen\lumen.exe";
        let p = |pid: u32, exe: &str, args: &[&str]| Proc { pid, exe: exe.into(), args: args.iter().map(|s| s.to_string()).collect() };
        let me = 10;
        // Only this window.
        assert!(!other_lumen_open(me, exe, &[p(10, exe, &[])]));
        // A second window of the same program.
        assert!(other_lumen_open(me, exe, &[p(10, exe, &[]), p(11, exe, &[])]));
        // The same path, however Windows cases it this time.
        assert!(other_lumen_open(me, exe, &[p(11, r"c:\apps\LUMEN\Lumen.EXE", &[])]));
        // A stdio bridge is safe to end: its client starts it again.
        assert!(!other_lumen_open(me, exe, &[p(10, exe, &[]), p(12, exe, &["--mcp"])]));
        // A build somewhere else is not what the installer replaces.
        assert!(!other_lumen_open(me, exe, &[p(13, r"X:\src\target\debug\lumen.exe", &[])]));
        assert!(!other_lumen_open(me, exe, &[p(14, r"C:\Windows\notepad.exe", &[])]));
    }

    #[test]
    fn the_install_waits_for_unsaved_work() {
        let now = Duration::from_millis(100);
        assert!(install_allowed(false, 0, false, now).is_ok());
        assert!(install_allowed(true, 0, false, now).is_err(), "an edit still unsaved");
        assert!(install_allowed(false, 1, false, now).is_err(), "an import still running");
        assert!(install_allowed(false, 0, true, now).is_err(), "another window open");
    }

    #[test]
    fn the_guard_knows_an_install_is_running_only_while_it_runs() {
        assert!(!install_running());
        {
            let _running = InstallRunning::begin();
            assert!(install_running());
        }
        assert!(!install_running(), "an install that stops early lets the exit guard work again");
    }

    #[test]
    fn a_late_close_retries_next_time() {
        assert!(install_allowed(false, 0, false, LATE_CLOSE).is_ok());
        assert!(install_allowed(false, 0, false, LATE_CLOSE + Duration::from_millis(1)).is_err());
    }

    #[test]
    fn a_staged_allowed_update_gives_the_exit_guard_the_install_grace() {
        use UpdateChannel::*;
        let s = staged("26.1.1-alpha", true);
        // Gate open and the stage still stands: the guard must not end a slow Close at 4 s.
        assert!(install_expected_at_close(true, Some(&s), &settings(Alpha, true)));
        // Nothing staged, a closed gate, or a stage the settings no longer allow: the short grace.
        assert!(!install_expected_at_close(true, None, &settings(Alpha, true)));
        assert!(!install_expected_at_close(false, Some(&s), &settings(Alpha, true)));
        assert!(!install_expected_at_close(true, Some(&s), &settings(Stable, true)));
    }

    #[test]
    fn quit_skips_install_on_exit() {
        use UpdateChannel::*;
        let s = staged("26.1.1-alpha", true);
        // Setup's --quit: Setup installs Lumen itself, so the staged update neither installs at this
        // exit nor holds the exit guard for the install grace.
        assert!(!installs_at_this_exit(true, true, Some(&s), &settings(Alpha, true)));
        // An ordinary Close is unchanged.
        assert!(installs_at_this_exit(false, true, Some(&s), &settings(Alpha, true)));
        assert!(!installs_at_this_exit(false, true, None, &settings(Alpha, true)));
        // And the install itself refuses, with a reason for the log; the stage is kept.
        assert_eq!(exit_install_refused(true), Some("Lumen Setup is installing"));
        assert_eq!(exit_install_refused(false), None);
    }

    #[test]
    fn the_guard_waits_for_an_expected_install_until_it_is_settled() {
        let t = Instant::now();
        expect_install(true);
        assert!(install_expected());
        assert!(!crate::window::force_exit_due(t, t + Duration::from_secs(5), install_expected()));
        expect_install(false);
        assert!(!install_expected());
        assert!(crate::window::force_exit_due(t, t + Duration::from_secs(5), install_expected()));
    }

    #[test]
    fn a_slow_close_can_still_install_within_the_install_grace() {
        use crate::window::{EXIT_GRACE, INSTALL_GRACE};
        assert!(LATE_CLOSE > EXIT_GRACE, "a Close slower than the short grace still installs");
        assert!(LATE_CLOSE < INSTALL_GRACE, "and leaves the guard room to let the installer start");
        assert!(install_allowed(false, 0, false, EXIT_GRACE + Duration::from_secs(1)).is_ok());
    }

    #[tokio::test]
    async fn a_refused_connection_is_a_dead_feed_not_an_offline_computer() {
        // The plugin's reqwest (0.13) differs from ours (0.12), so the classifier is tried on a real
        // error from ours; `check_fail` only routes `E::Reqwest` to it.
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        drop(l);
        let err = client().get(format!("http://127.0.0.1:{port}/alpha.json")).send().await.unwrap_err();
        assert!(turned_away(&err));
    }

    #[tokio::test]
    async fn no_answer_at_all_is_still_offline() {
        // Accepts the connection into its queue and never answers: the line is down, not the feed.
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        let c = reqwest::Client::builder().timeout(Duration::from_millis(150)).build().unwrap();
        let err = c.get(format!("http://127.0.0.1:{port}/alpha.json")).send().await.unwrap_err();
        assert!(!turned_away(&err));
        assert!(!turned_away(&std::io::Error::from(std::io::ErrorKind::TimedOut)));
        assert!(turned_away(&std::io::Error::from(std::io::ErrorKind::ConnectionRefused)));
        drop(l);
    }

    #[test]
    fn the_installer_waits_for_the_lumen_that_launched_it() {
        // Seen in the e2e (1 close in 5): just after Lumen exits, Windows' InventorySvc reads the
        // program for a moment. Restart Manager refuses to shut a critical service (`RmShutdown`
        // 350), and the template then aborts the install silently, so the hook waits that out.
        assert_eq!(installer_args(4242), ["/P".to_string(), "/LUMENPID=4242".to_string()]);
        let conf: serde_json::Value =
            serde_json::from_str(include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/tauri.conf.json"))).unwrap();
        let hooks = conf["bundle"]["windows"]["nsis"]["installerHooks"].as_str().expect("installerHooks is set");
        let nsh = std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(hooks)).unwrap();
        assert!(nsh.contains("!macro NSIS_HOOK_PREINSTALL"), "the wait runs before the running-app check");
        assert!(nsh.contains("\"/LUMENPID=\""), "the hook reads the argument Lumen passes");
        assert!(nsh.contains("WaitForSingleObject"), "it waits on the process itself");
        assert!(nsh.contains("RSTRTMGR::RmGetList"), "it asks Restart Manager who holds the program");
        assert!(nsh.contains("LUMEN_RM_CLEAR_MS"), "and waits until nothing unstoppable has held it for a while");
        assert!(nsh.contains("LUMEN_RM_MAX_MS"), "for a bounded time");
    }

    #[test]
    fn a_change_of_mind_discards_what_it_no_longer_allows() {
        use UpdateChannel::*;
        // Alpha staged, moved to Stable: it must not install.
        assert!(!keeps_staged(&staged("26.1.1-alpha", true), &settings(Stable, true)));
        assert!(!keeps_staged(&staged("26.1.1-alpha", false), &settings(Beta, true)));
        assert!(keeps_staged(&staged("26.1.1-beta", true), &settings(Beta, true)));
        assert!(keeps_staged(&staged("26.1.1-stable", true), &settings(Stable, true)));
        // Automatic off: what it fetched by itself goes, what the person asked for stays.
        assert!(!keeps_staged(&staged("26.1.1-alpha", true), &settings(Alpha, false)));
        assert!(keeps_staged(&staged("26.1.1-alpha", false), &settings(Alpha, false)));
    }

    #[test]
    fn a_failed_install_is_counted_and_said() {
        let t = tempfile::tempdir().unwrap();
        let mut s = staged("26.1.1-alpha", true);
        std::fs::write(stage_path(t.path(), &s.version), "installer").unwrap();
        s.save(t.path()).unwrap();
        s.attempts += 1;
        s.save(t.path()).unwrap();
        let back = Staged::load(t.path()).unwrap();
        assert_eq!(back.attempts, 1, "still staged, and the count survives the restart");
        assert!(matches!(ready_state(&back), UpdateState::Ready { attempts: 1, .. }));
    }

    #[test]
    fn check_failures_are_quiet_states() {
        use tauri_plugin_updater::Error as E;
        assert!(matches!(check_fail(&E::ReleaseNotFound), CheckFail::Why(FailWhy::CouldNotCheck)));
        assert!(matches!(check_fail(&E::TargetNotFound("windows-x86_64".into())), CheckFail::NoBuildHere));
        assert!(matches!(check_fail(&E::TargetsNotFound(vec!["darwin-aarch64".into()])), CheckFail::NoBuildHere));
    }

    #[test]
    fn a_bad_signature_is_told_from_other_install_failures() {
        use tauri_plugin_updater::Error as E;
        assert!(is_bad_signature(&E::SignedVersionMismatch { signed: "1".into(), announced: "2".into() }));
        assert!(is_bad_signature(&E::MissingSignedVersion));
        assert!(!is_bad_signature(&E::ReleaseNotFound));
        assert!(!is_bad_signature(&E::Io(std::io::Error::other("disk"))));
    }

    #[test]
    fn a_full_disk_is_told_from_other_write_failures() {
        assert_eq!(FetchError::NoSpace.why(), FailWhy::NoSpace);
        assert_eq!(FetchError::Io(std::io::ErrorKind::StorageFull.into()).why(), FailWhy::NoSpace);
        assert_eq!(FetchError::Io(std::io::Error::other("x")).why(), FailWhy::Other);
        assert_eq!(FetchError::Network("x".into()).why(), FailWhy::Offline);
        assert_eq!(FetchError::Http(503).why(), FailWhy::CouldNotCheck);
    }

    /// A local server for [`fetch_to_part`]: serves `body`, honours `Range`, breaks the first
    /// response after `break_after` bytes when set, and records each request's headers.
    mod server {
        use std::convert::Infallible;
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::sync::{Arc, Mutex};

        use axum::body::Body;
        use axum::extract::State;
        use axum::http::{header, HeaderMap, Response, StatusCode};
        use axum::routing::get;

        #[derive(Clone)]
        pub struct Serve {
            pub body: Arc<Vec<u8>>,
            pub break_after: Option<usize>,
            pub broke: Arc<AtomicBool>,
            pub status: u16,
            pub seen: Arc<Mutex<Vec<HeaderMap>>>,
        }

        async fn file(State(s): State<Serve>, headers: HeaderMap) -> Response<Body> {
            s.seen.lock().unwrap().push(headers.clone());
            if s.status != 200 {
                return Response::builder().status(s.status).body(Body::empty()).unwrap();
            }
            let from = headers
                .get(header::RANGE)
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.strip_prefix("bytes="))
                .and_then(|v| v.strip_suffix('-'))
                .and_then(|v| v.parse::<usize>().ok());
            let start = from.unwrap_or(0);
            let total = s.body.len();
            let mut b = Response::builder().header(header::ACCEPT_RANGES, "bytes");
            if from.is_some() {
                b = b.status(StatusCode::PARTIAL_CONTENT).header(header::CONTENT_RANGE, format!("bytes {start}-{}/{total}", total - 1));
            }
            let rest = s.body[start..].to_vec();
            if let (Some(n), false) = (s.break_after, s.broke.swap(true, Ordering::SeqCst)) {
                // Promise everything, send part, then fail the stream.
                let head = rest[..n].to_vec();
                // The cut comes a moment later: a reset right behind the data makes Windows drop
                // what the client had not read yet, which is not the break being tested.
                use futures_util::StreamExt;
                let stream = futures_util::stream::iter(vec![Ok::<_, std::io::Error>(head)]).chain(futures_util::stream::once(async {
                    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                    Err(std::io::Error::other("cut"))
                }));
                return b.header(header::CONTENT_LENGTH, rest.len()).body(Body::from_stream(stream)).unwrap();
            }
            b.header(header::CONTENT_LENGTH, rest.len()).body(Body::from(rest)).unwrap()
        }

        /// The server's address, for the life of the runtime.
        pub async fn start(s: Serve) -> String {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let addr = listener.local_addr().unwrap();
            let app = axum::Router::new().route("/file", get(file)).with_state(s);
            tokio::spawn(async move {
                let _ = axum::serve(listener, app).await;
                Ok::<_, Infallible>(())
            });
            format!("http://{addr}/file")
        }
    }

    fn serve(body: &[u8]) -> server::Serve {
        server::Serve { body: std::sync::Arc::new(body.to_vec()), break_after: None, broke: Default::default(), status: 200, seen: Default::default() }
    }

    fn body() -> Vec<u8> {
        (0..200_000u32).map(|i| (i % 251) as u8).collect()
    }

    fn client() -> reqwest::Client {
        update_client()
    }

    #[tokio::test]
    async fn a_full_download() {
        let t = tempfile::tempdir().unwrap();
        let part = t.path().join("update.part");
        let want = body();
        let url = server::start(serve(&want)).await;
        let mut seen = Vec::new();
        fetch_to_part(&client(), &url, &part, None, &mut |done, total| {
            seen.push((done, total));
            true
        })
        .await
        .unwrap();
        assert_eq!(std::fs::read(&part).unwrap(), want);
        assert_eq!(seen.last().unwrap().0, want.len() as u64);
        assert!(seen.iter().all(|(_, total)| *total == want.len() as u64));
    }

    #[tokio::test]
    async fn a_broken_connection_resumes() {
        let t = tempfile::tempdir().unwrap();
        let part = t.path().join("update.part");
        let want = body();
        let mut s = serve(&want);
        s.break_after = Some(50_000);
        let seen = s.seen.clone();
        let url = server::start(s).await;
        fetch_to_part(&client(), &url, &part, None, &mut |_, _| true).await.unwrap();
        assert_eq!(std::fs::read(&part).unwrap(), want);
        let requests = seen.lock().unwrap();
        assert_eq!(requests.len(), 2, "one break, one resume");
        assert_eq!(requests[1].get("range").unwrap(), "bytes=50000-");
    }

    #[tokio::test]
    async fn a_part_from_last_time_is_picked_up() {
        let t = tempfile::tempdir().unwrap();
        let part = t.path().join("update.part");
        let want = body();
        std::fs::write(&part, &want[..1000]).unwrap();
        let s = serve(&want);
        let seen = s.seen.clone();
        let url = server::start(s).await;
        fetch_to_part(&client(), &url, &part, None, &mut |_, _| true).await.unwrap();
        assert_eq!(std::fs::read(&part).unwrap(), want);
        assert_eq!(seen.lock().unwrap()[0].get("range").unwrap(), "bytes=1000-");
    }

    #[tokio::test]
    async fn a_404_is_a_failure_and_keeps_nothing() {
        let t = tempfile::tempdir().unwrap();
        let part = t.path().join("update.part");
        let mut s = serve(b"x");
        s.status = 404;
        let url = server::start(s).await;
        let err = fetch_to_part(&client(), &url, &part, None, &mut |_, _| true).await.unwrap_err();
        assert!(matches!(err, FetchError::Http(404)), "{err:?}");
        assert!(!part.exists());
    }

    #[tokio::test]
    async fn a_download_that_does_not_fit_is_refused_before_writing() {
        let t = tempfile::tempdir().unwrap();
        let part = t.path().join("update.part");
        let url = server::start(serve(&body())).await;
        let err = fetch_to_part(&client(), &url, &part, Some(10), &mut |_, _| true).await.unwrap_err();
        assert!(matches!(err, FetchError::NoSpace), "{err:?}");
        assert!(!part.exists());
    }

    #[tokio::test]
    async fn a_download_that_cannot_write_is_a_state() {
        let t = tempfile::tempdir().unwrap();
        // A file where the folder should be: nothing can be written.
        std::fs::write(t.path().join("in-the-way"), "x").unwrap();
        let part = t.path().join("in-the-way").join("update.part");
        let url = server::start(serve(&body())).await;
        let err = fetch_to_part(&client(), &url, &part, None, &mut |_, _| true).await.unwrap_err();
        assert!(matches!(err, FetchError::Io(_)), "{err:?}");
        assert_eq!(err.why(), FailWhy::Other);
    }

    #[tokio::test]
    async fn a_download_can_be_stopped() {
        let t = tempfile::tempdir().unwrap();
        let part = t.path().join("update.part");
        let url = server::start(serve(&body())).await;
        let err = fetch_to_part(&client(), &url, &part, None, &mut |_, _| false).await.unwrap_err();
        assert!(matches!(err, FetchError::Cancelled), "{err:?}");
    }

    #[tokio::test]
    async fn the_download_sends_no_identifier() {
        let t = tempfile::tempdir().unwrap();
        let s = serve(&body());
        let seen = s.seen.clone();
        let url = server::start(s).await;
        fetch_to_part(&client(), &url, &t.path().join("update.part"), None, &mut |_, _| true).await.unwrap();
        let requests = seen.lock().unwrap();
        let headers = &requests[0];
        assert_eq!(headers.get("user-agent").unwrap(), USER_AGENT, "no version, no id");
        for name in headers.keys() {
            assert!(["host", "user-agent", "accept", "range"].contains(&name.as_str()), "unexpected header {name}");
        }
    }
}
