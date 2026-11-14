//! Tauri commands: thin adapters over the decision modules. The removal lists come from
//! `removal::summary`, which only reads. A running Lumen is closed by `lumen::close_lumen` (never
//! killed) before the installer or uninstaller runs; `cancel_wait` ends that wait and nothing else.
use crate::folder::{self, FolderCheck};
use crate::registry::{read_installed, Identity, Registry};
use crate::install;
use crate::lumen::{self, Closing, System as _, LUMEN_EVENT};
use crate::models::ModelsJob;
use crate::options::{InstallOptions, InstallResult, ModelsOutcome};
use crate::payload;
use crate::removal;
use crate::shortcuts;
use crate::version::{compare, Installed};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use ts_rs::TS;

/// Set by *Cancel* while Setup waits for Lumen to close.
#[derive(Default)]
pub struct LumenWait(pub AtomicBool);

const LUMEN_POLL: Duration = Duration::from_millis(300);

/// Lumen's own disk need in bytes (the installed size plus headroom), baked by the release build.
/// The `setup://progress` event.
pub const PROGRESS_EVENT: &str = "setup://progress";

/// What a development build assumes Lumen takes on disk. A release build bakes the real size of
/// the files the NSIS installer lays down (`LUMEN_SETUP_INSTALLED_BYTES`, set by
/// `scripts/release/setup-build.mjs`).
const DEV_INSTALLED_BYTES: u64 = 400_000_000;

/// Lumen's installed size in bytes: the baked one, else the development placeholder.
pub(crate) fn installed_bytes() -> u64 {
    option_env!("LUMEN_SETUP_INSTALLED_BYTES").and_then(|s| s.parse().ok()).unwrap_or(DEV_INSTALLED_BYTES)
}

/// Free space the install needs: the installed files, the installer written to the temp folder
/// first (`payload` bytes, none for a slim Setup) and 5 % headroom. The models come on top.
pub(crate) fn need_bytes(installed: u64, payload: u64) -> u64 {
    installed + payload + installed / 20
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct InstallState {
    pub installed: Installed,
    /// The version this Setup installs.
    pub ours: String,
    pub lumen_running: bool,
    #[ts(type = "string")]
    pub default_folder: PathBuf,
    /// Lumen's installed size in bytes, for the Welcome line.
    #[ts(type = "number")]
    pub size_on_disk: u64,
    /// `photos_models::Manifest::default_bytes`.
    #[ts(type = "number")]
    pub models_bytes: u64,
    /// Space Lumen itself needs on disk (the models come on top when they are on).
    #[ts(type = "number")]
    pub need_bytes: u64,
    /// Every registered library path, read from libraries.json; Remove lists them.
    pub libraries: Vec<String>,
    /// Size of the settings, caches and models Remove's box would take (its allow-list).
    #[ts(type = "number")]
    pub app_data_bytes: u64,
    /// `backups/pre-migration/` when it holds anything; always kept.
    pub safety_copies: Option<String>,
    /// Started from *Installed apps* (`--uninstall`): screen 6 first, and Cancel closes Setup.
    pub remove_only: bool,
}

/// What the machine knows that the decision does not read from the registry.
pub struct Machine {
    pub default_folder: PathBuf,
    /// What this Setup's Lumen takes once installed (the Welcome line when nothing is installed).
    pub install_bytes: u64,
    pub models_bytes: u64,
    pub need_bytes: u64,
    pub lumen_running: bool,
    /// `removal::summary` of Lumen's app data.
    pub removal: removal::Summary,
    pub remove_only: bool,
}

pub fn build_state(reg: &dyn Registry, id: &Identity, ours: &str, machine: Machine, size_of: impl Fn(&Path) -> u64) -> InstallState {
    let (installed, size_on_disk) = match read_installed(reg, id) {
        None => (Installed::None, machine.install_bytes),
        Some((version, location)) => {
            let size = size_of(Path::new(&location));
            (compare(version.as_deref(), &location, ours), size)
        }
    };
    InstallState {
        installed,
        ours: ours.to_string(),
        lumen_running: machine.lumen_running,
        default_folder: machine.default_folder,
        size_on_disk,
        models_bytes: machine.models_bytes,
        need_bytes: machine.need_bytes,
        libraries: machine.removal.libraries.iter().map(|p| p.to_string_lossy().into_owned()).collect(),
        app_data_bytes: machine.removal.app_data_bytes,
        safety_copies: machine.removal.safety_copies.map(|p| p.to_string_lossy().into_owned()),
        remove_only: machine.remove_only,
    }
}

pub(crate) fn dir_size(dir: &Path) -> u64 {
    let Ok(entries) = std::fs::read_dir(dir) else { return 0 };
    entries
        .flatten()
        .map(|e| match e.metadata() {
            Ok(m) if m.is_dir() => dir_size(&e.path()),
            Ok(m) => m.len(),
            Err(_) => 0,
        })
        .sum()
}

fn default_folder(id: &Identity) -> PathBuf {
    let base = std::env::var_os("LOCALAPPDATA").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."));
    default_folder_in(&base, id)
}

/// `%LOCALAPPDATA%\<product>`, where Tauri's per-user NSIS template puts it.
fn default_folder_in(local_app_data: &Path, id: &Identity) -> PathBuf {
    local_app_data.join(&id.product)
}

#[cfg(windows)]
fn real_registry() -> crate::registry::HkcuRegistry {
    crate::registry::HkcuRegistry
}

#[cfg(not(windows))]
fn real_registry() -> crate::registry::MemoryRegistry {
    crate::registry::MemoryRegistry::default()
}

/// The folder screen 6 removes: the one the `--uninstall` copy was given, else the registered one.
fn install_dir(reg: &dyn Registry, id: &Identity) -> Option<PathBuf> {
    crate::uninstall::REMOVE_DIR.get().cloned().or_else(|| read_installed(reg, id).map(|(_, l)| PathBuf::from(l)))
}

/// Close the Lumen running from `exe`, reporting `setup://lumen`; whether one was open.
fn close_running_lumen(app: &tauri::AppHandle, exe: &Path) -> Result<bool, String> {
    use tauri::{Emitter, Manager};
    let wait = app.state::<LumenWait>();
    wait.0.store(false, Ordering::SeqCst);
    lumen::close_lumen(&lumen::RealSystem, exe, &std::env::temp_dir(), &wait.0, LUMEN_POLL, &mut |c: Closing| {
        let _ = app.emit(LUMEN_EVENT, c);
    })
    .map_err(|e| e.to_string())
}

#[tauri::command(async)]
pub fn get_install_state() -> InstallState {
    let id = Identity::baked();
    let reg = real_registry();
    let lumen_running = install_dir(&reg, &id)
        .is_some_and(|dir| !lumen::running(&lumen::RealSystem.processes(), &dir.join(id.exe_name()), std::process::id()).is_empty());
    let machine = Machine {
        default_folder: default_folder(&id),
        install_bytes: installed_bytes(),
        models_bytes: photos_models::manifest().default_bytes(),
        need_bytes: need_bytes(installed_bytes(), payload::len()),
        lumen_running,
        removal: match (removal::app_data_dir(), removal::webview_data_dir()) {
            (Some(app), Some(web)) => removal::summary(&app, &web),
            _ => removal::Summary::default(),
        },
        remove_only: crate::uninstall::REMOVE_DIR.get().is_some(),
    };
    build_state(&reg, &id, env!("CARGO_PKG_VERSION"), machine, dir_size)
}

#[tauri::command(async)]
pub fn check_folder(path: String) -> FolderCheck {
    folder::folder_check(Path::new(&path), need_bytes(installed_bytes(), payload::len()), &Identity::baked().exe_name())
}

#[tauri::command(async)]
pub fn choose_folder(app: tauri::AppHandle, start: Option<String>) -> Option<String> {
    use tauri_plugin_dialog::DialogExt;
    let mut dialog = app.dialog().file();
    if let Some(s) = start.filter(|s| Path::new(s).is_dir()) {
        dialog = dialog.set_directory(s);
    }
    dialog.blocking_pick_folder().and_then(|p| p.into_path().ok()).map(|p| p.to_string_lossy().into_owned())
}

/// Run the install, then (if asked) download the models. Rejects with a sentence when the
/// installer fails; a model that does not arrive resolves as `later`.
#[tauri::command(async)]
pub async fn install(
    app: tauri::AppHandle,
    job: tauri::State<'_, ModelsJob>,
    opts: InstallOptions,
) -> Result<InstallResult, String> {
    use tauri::Emitter;
    job.reset();
    let id = Identity::baked();
    let silent = install::webview2_present();
    let installer_opts = opts.clone();
    let emitter = app.clone();
    let exe = opts.folder.join(id.exe_name());
    let was_open = {
        let (app, exe) = (app.clone(), exe.clone());
        tauri::async_runtime::spawn_blocking(move || close_running_lumen(&app, &exe)).await.map_err(|e| e.to_string())??
    };
    tauri::async_runtime::spawn_blocking(move || {
        let temp = std::env::temp_dir().join(format!("lumen-setup-{}", std::process::id()));
        std::fs::create_dir_all(&temp).map_err(|e| install::InstallError::Start(e.to_string()))?;
        let desktop = shortcuts::desktop_dir();
        let env = install::Env { id: &id, temp: &temp, desktop: desktop.as_deref(), installed_bytes: installed_bytes(), silent };
        let mut reg = real_registry();
        let result = install::install_with(
            &env,
            &payload::write,
            &mut reg,
            &mut shortcuts::RealFs,
            &installer_opts,
            &mut |p| {
                let _ = emitter.emit(PROGRESS_EVENT, p);
            },
        );
        let _ = std::fs::remove_dir(&temp);
        result
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(|e| e.to_string())?;

    // Lumen was open: it comes back now and fetches the models itself (Setup and Lumen never
    // download at once, as with Open Lumen).
    if was_open {
        let _ = lumen::reopen(&exe);
        return Ok(InstallResult { models: ModelsOutcome::Later });
    }
    if !opts.download_models {
        return Ok(InstallResult { models: ModelsOutcome::Later });
    }
    let Some(dir) = removal::app_data_dir().map(|d| d.join("models")) else {
        return Ok(InstallResult { models: ModelsOutcome::Later });
    };
    let m = photos_models::manifest();
    let entries: Vec<_> = m.default_set().cloned().collect();
    let emitter = app.clone();
    let models = job
        .download(&photos_models::ModelStore { dir }, &entries, &|e| m.url_of(e), &mut |p| {
            let _ = emitter.emit(PROGRESS_EVENT, p);
        })
        .await;
    Ok(InstallResult { models })
}

/// Skip: stop the download and keep its `.part`; Lumen fetches the rest itself.
#[tauri::command(async)]
pub fn skip_models(job: tauri::State<'_, ModelsJob>) {
    job.request_stop();
}

/// Stop Setup's download first (so Lumen can resume the `.part`), then start the installed Lumen.
#[tauri::command(async)]
pub async fn open_lumen(job: tauri::State<'_, ModelsJob>) -> Result<(), String> {
    job.stop().await;
    let id = Identity::baked();
    let (_, location) = read_installed(&real_registry(), &id).ok_or("Lumen is not installed yet.")?;
    std::process::Command::new(PathBuf::from(location).join(id.exe_name()))
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("Setup could not start Lumen: {e}. Open it from the Start menu."))
}

/// *Cancel* while Lumen closes: Setup stops waiting and changes nothing.
#[tauri::command(async)]
pub fn cancel_wait(wait: tauri::State<'_, LumenWait>) {
    wait.0.store(true, Ordering::SeqCst);
}

/// Screen 6: close Lumen, run NSIS's uninstaller, then the Run value, the desktop shortcut and,
/// only when `remove_data`, app data by allow-list. Rejects with a sentence.
/// Screen 6b: every registered library with its photos and size, or why it can't be deleted here.
#[tauri::command(async)]
pub async fn library_details() -> Vec<crate::libraries::LibraryDetail> {
    tauri::async_runtime::spawn_blocking(|| {
        let Some(app_data) = removal::app_data_dir() else { return Vec::new() };
        let protected = crate::libraries::Protected::real(install_dir(&real_registry(), &Identity::baked()));
        crate::libraries::registered(&app_data).iter().map(|(name, path)| crate::libraries::detail(name, path, &protected)).collect()
    })
    .await
    .unwrap_or_default()
}

/// Screen 6b, before anything is deleted: do the chosen libraries fit the Recycle Bin?
#[tauri::command(async)]
pub async fn check_recycle_bin(paths: Vec<String>) -> crate::libraries::BinCheck {
    tauri::async_runtime::spawn_blocking(move || {
        let protected = crate::libraries::Protected::real(install_dir(&real_registry(), &Identity::baked()));
        let paths: Vec<PathBuf> = paths.iter().map(PathBuf::from).collect();
        crate::libraries::bin_check(&paths, &protected, &crate::libraries::ShellBin)
    })
    .await
    .unwrap_or(crate::libraries::BinCheck { fits: false, bytes: 0, room: 0 })
}

/// The libraries screen 6b confirmed (typed), and how.
#[derive(Debug, Clone, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DeleteChoice {
    pub paths: Vec<String>,
    pub mode: crate::libraries::Mode,
}

/// Only registered libraries are ever deleted, whatever the page sends.
pub fn chosen_libraries(choice: &DeleteChoice, registered: &[(String, PathBuf)]) -> Vec<PathBuf> {
    registered
        .iter()
        .map(|(_, p)| p.clone())
        .filter(|p| choice.paths.iter().any(|c| removal::norm(Path::new(c)) == removal::norm(p)))
        .collect()
}

#[tauri::command(async)]
pub async fn uninstall(app: tauri::AppHandle, remove_data: bool, delete_libraries: Option<DeleteChoice>) -> Result<Vec<crate::libraries::Deleted>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let id = Identity::baked();
        let mut reg = real_registry();
        let dir = install_dir(&reg, &id).ok_or("Setup could not find where Lumen is installed. Remove it from Installed apps.")?;
        // Read before the ticked box can remove `libraries.json`.
        let registered = removal::app_data_dir().map(|a| crate::libraries::registered(&a)).unwrap_or_default();
        let libraries = delete_libraries.as_ref().map(|c| chosen_libraries(c, &registered)).unwrap_or_default();
        close_running_lumen(&app, &dir.join(id.exe_name()))?;
        let desktop = shortcuts::desktop_dir();
        let (app_data, web) = (removal::app_data_dir(), removal::webview_data_dir());
        let uninstaller = dir.join(crate::uninstall::UNINSTALL_EXE);
        let env = crate::uninstall::RemoveEnv {
            id: &id,
            dir: &dir,
            uninstaller: &uninstaller,
            desktop: desktop.as_deref(),
            app_data: app_data.as_deref(),
            webview_data: web.as_deref(),
        };
        crate::uninstall::remove_lumen(&env, &mut reg, &mut shortcuts::RealFs, remove_data).map_err(|e| e.to_string())?;
        // Lumen is closed and uninstalled: nothing holds a library open now.
        let Some(choice) = delete_libraries else { return Ok(Vec::new()) };
        let protected = crate::libraries::Protected { install_dir: Some(dir.clone()), ..crate::libraries::Protected::real(None) };
        let done: Vec<_> = libraries.iter().map(|l| crate::libraries::delete(l, &protected, choice.mode, &crate::libraries::ShellBin)).collect();
        if !remove_data {
            if let Some(app_data) = app_data.as_deref() {
                let gone: Vec<PathBuf> = done.iter().filter(|d| d.problem.is_none()).map(|d| PathBuf::from(&d.path)).collect();
                let _ = crate::libraries::forget(app_data, &gone);
            }
        }
        Ok(done)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::MemoryRegistry;

    #[test]
    fn only_registered_libraries_are_ever_chosen() {
        let registered = vec![("A".to_string(), PathBuf::from(r"C:\Users\A\Pictures\Lumen Library")), ("B".to_string(), PathBuf::from(r"D:\Photos"))];
        let choice = DeleteChoice {
            paths: vec![r"c:/users/a/pictures/lumen library".into(), r"C:\Windows".into(), r"C:\Users\A".into()],
            mode: crate::libraries::Mode::Recycle,
        };
        assert_eq!(chosen_libraries(&choice, &registered), vec![PathBuf::from(r"C:\Users\A\Pictures\Lumen Library")]);
        assert!(chosen_libraries(&DeleteChoice { paths: vec![], mode: crate::libraries::Mode::Erase }, &registered).is_empty());
    }

    #[test]
    fn the_need_is_installed_files_plus_the_temp_installer_plus_headroom() {
        assert_eq!(need_bytes(300_000_000, 80_000_000), 395_000_000);
        assert_eq!(need_bytes(300_000_000, 0), 315_000_000);
    }

    #[test]
    fn a_build_without_a_baked_size_assumes_the_placeholder() {
        // The test build sets no LUMEN_SETUP_INSTALLED_BYTES.
        if option_env!("LUMEN_SETUP_INSTALLED_BYTES").is_none() {
            assert_eq!(installed_bytes(), DEV_INSTALLED_BYTES);
        }
    }

    fn machine() -> Machine {
        Machine {
            default_folder: PathBuf::from(r"C:\Users\A\AppData\Local\Lumen"),
            install_bytes: 280,
            models_bytes: 500,
            need_bytes: 300,
            lumen_running: false,
            removal: removal::Summary::default(),
            remove_only: false,
        }
    }

    #[test]
    fn the_remove_screen_gets_libraries_size_and_safety_copies() {
        let t = tempfile::tempdir().unwrap();
        let (app, web) = (t.path().join("app"), t.path().join("web"));
        let lib = t.path().join("Lumen Library");
        std::fs::create_dir_all(app.join("backups/pre-migration/k")).unwrap();
        std::fs::write(app.join("backups/pre-migration/k/library-1.db"), "copy").unwrap();
        let registry = serde_json::json!({ "version": 2, "libraries": [{ "id": "a", "name": "L", "path": lib.to_string_lossy(), "lastOpenedAt": 0 }], "current": "a" });
        std::fs::write(app.join("libraries.json"), registry.to_string()).unwrap();
        let size = std::fs::metadata(app.join("libraries.json")).unwrap().len();
        let m = Machine { removal: removal::summary(&app, &web), ..machine() };
        let s = build_state(&MemoryRegistry::default(), &Identity::named("Lumen"), "26.1.0", m, |_| 0);
        assert_eq!(s.libraries, vec![lib.to_string_lossy().into_owned()]);
        assert_eq!(s.app_data_bytes, size);
        // Shown on screen 6, so in the system's own separators (it read `…\backups/pre-migration`).
        assert_eq!(s.safety_copies, Some(app.join("backups").join("pre-migration").to_string_lossy().into_owned()));
        assert!(app.join("libraries.json").is_file(), "reading the state deletes nothing");
    }

    #[test]
    fn the_default_folder_is_the_stock_installers() {
        // Tauri's per-user NSIS template installs to $LOCALAPPDATA\<product>; Lumen is there today.
        let local = Path::new(r"C:\Users\A\AppData\Local");
        assert_eq!(default_folder_in(local, &Identity::named("Lumen")), local.join("Lumen"));
        assert_eq!(default_folder_in(local, &Identity::named("Lumen Test")), local.join("Lumen Test"));
    }

    #[test]
    fn nothing_installed_is_none_with_the_default_folder() {
        let reg = MemoryRegistry::default();
        let s = build_state(&reg, &Identity::named("Lumen"), "26.1.0", machine(), |_| panic!("no folder to size"));
        assert_eq!(s.installed, Installed::None);
        // Phase 8: Welcome said "1 MB on this PC"; nothing installed yet means what this Setup lays down.
        assert_eq!(s.size_on_disk, 280);
        assert_eq!((s.models_bytes, s.need_bytes, s.ours.as_str()), (500, 300, "26.1.0"));
        assert!(s.libraries.is_empty() && s.safety_copies.is_none());
    }

    #[test]
    fn an_older_install_reports_its_version_location_and_size() {
        let id = Identity::named("Lumen");
        let mut reg = MemoryRegistry::default();
        reg.write(&id.uninstall_key(), "InstallLocation", r"C:\L").unwrap();
        reg.write(&id.uninstall_key(), "DisplayVersion", "26.1.0-alpha").unwrap();
        let s = build_state(&reg, &id, "26.1.0", machine(), |p| if p == Path::new(r"C:\L") { 288 } else { 0 });
        assert_eq!(s.installed, Installed::Older { version: "26.1.0-alpha".into(), location: r"C:\L".into() });
        assert_eq!(s.size_on_disk, 288);
    }

    #[test]
    fn a_garbled_version_is_an_update_not_an_error() {
        let id = Identity::named("Lumen");
        let mut reg = MemoryRegistry::default();
        reg.write(&id.uninstall_key(), "InstallLocation", r"C:\L").unwrap();
        reg.write(&id.uninstall_key(), "DisplayVersion", "???").unwrap();
        let s = build_state(&reg, &id, "26.1.0", machine(), |_| 1);
        assert!(matches!(s.installed, Installed::Older { .. }));
    }
}
