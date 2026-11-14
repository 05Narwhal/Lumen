//! Running the stock NSIS installer from Setup: write the embedded payload to a temp file, run
//! it with `nsis_args`, report progress by the bytes that appear in the install folder, then
//! settle the desktop shortcut and the Run value. Kills nothing; touches no library.
use crate::options::{nsis_args, InstallMode, InstallOptions, SetupProgress, SetupTask};
use crate::registry::{set_start_with_windows, Identity, Registry};
use crate::shortcuts::{desktop_shortcut_path, settle_desktop_shortcut, ShortcutFs};
use std::ffi::OsString;
use std::io;
use std::path::{Path, PathBuf};

#[derive(Debug)]
pub enum InstallError {
    /// The payload could not be written or the installer could not start.
    Start(String),
    /// The installer ran and said no.
    Failed { code: Option<i32> },
}

impl std::fmt::Display for InstallError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            InstallError::Start(e) => write!(f, "Setup could not start the installer: {e}. Try again."),
            InstallError::Failed { code: Some(c) } => write!(f, "The installer stopped with code {c}. Try again."),
            InstallError::Failed { code: None } => write!(f, "The installer was ended before it finished. Try again."),
        }
    }
}

impl std::error::Error for InstallError {}

/// The arguments the installer gets: `nsis_args`, without `/S` when Windows lacks WebView2 (the
/// installer then shows its own pages and fetches it).
pub fn installer_args(opts: &InstallOptions, silent: bool) -> Vec<OsString> {
    let mut args = nsis_args(opts);
    if !silent {
        args.retain(|a| a != "/S");
    }
    args
}

/// What `install_with` needs from the machine; tests pass temp folders and fakes.
pub struct Env<'a> {
    pub id: &'a Identity,
    pub temp: &'a Path,
    pub desktop: Option<&'a Path>,
    /// The installed size the progress is measured against.
    pub installed_bytes: u64,
    pub silent: bool,
}

/// Run one install. `write_payload` puts the installer in `Env::temp` and returns its path.
pub fn install_with(
    env: &Env,
    write_payload: &dyn Fn(&Path) -> io::Result<PathBuf>,
    reg: &mut dyn Registry,
    fs: &mut dyn ShortcutFs,
    opts: &InstallOptions,
    progress: &mut dyn FnMut(SetupProgress),
) -> Result<(), InstallError> {
    // Read before the installer runs: an update keeps a shortcut the person already had.
    let link = env.desktop.map(|d| desktop_shortcut_path(d, env.id));
    let existed = link.as_ref().is_some_and(|l| fs.exists(l));

    let payload = write_payload(env.temp).map_err(|e| InstallError::Start(e.to_string()))?;
    let ran = run_installer(&payload, opts, env, progress);
    let _ = std::fs::remove_file(&payload);
    ran?;

    // Only after a successful install: remove the shortcut the person did not ask for, and (on a
    // fresh install only; update and repair keep it as it was) set or clear our own Run value.
    // Neither is fatal; the install itself worked.
    if let Some(desktop) = env.desktop {
        let _ = settle_desktop_shortcut(fs, desktop, env.id, opts, existed);
    }
    if opts.mode == InstallMode::Install {
        let exe = opts.folder.join(env.id.exe_name());
        let _ = set_start_with_windows(reg, env.id, &exe, opts.start_with_windows);
    }
    Ok(())
}

/// Start the installer and report the bytes in the install folder against the installed size
/// until it exits. The last report is the whole, so the bar ends full.
fn run_installer(
    payload: &Path,
    opts: &InstallOptions,
    env: &Env,
    progress: &mut dyn FnMut(SetupProgress),
) -> Result<(), InstallError> {
    let total = env.installed_bytes.max(1);
    let mut child = std::process::Command::new(payload)
        .args(installer_args(opts, env.silent))
        .spawn()
        .map_err(|e| InstallError::Start(e.to_string()))?;
    let mut last = u64::MAX;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => {
                let done = crate::commands::dir_size(&opts.folder).min(total);
                if done != last {
                    last = done;
                    progress(SetupProgress { task: SetupTask::Lumen, done, total });
                }
                std::thread::sleep(std::time::Duration::from_millis(150));
            }
            Err(e) => return Err(InstallError::Start(e.to_string())),
        }
    };
    if !status.success() {
        return Err(InstallError::Failed { code: status.code() });
    }
    progress(SetupProgress { task: SetupTask::Lumen, done: total, total });
    Ok(())
}

/// Whether the WebView2 runtime is installed (machine-wide or per user).
#[cfg(windows)]
pub fn webview2_present() -> bool {
    use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};
    use winreg::RegKey;
    const GUID: &str = "{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}";
    let wow = format!(r"Software\WOW6432Node\Microsoft\EdgeUpdate\Clients\{GUID}");
    let plain = format!(r"Software\Microsoft\EdgeUpdate\Clients\{GUID}");
    [(HKEY_LOCAL_MACHINE, wow.as_str()), (HKEY_LOCAL_MACHINE, plain.as_str()), (HKEY_CURRENT_USER, plain.as_str())].iter().any(|(hive, key)| {
        RegKey::predef(*hive)
            .open_subkey(key)
            .and_then(|k| k.get_value::<String, _>("pv"))
            .is_ok_and(|v| !v.is_empty() && v != "0.0.0.0")
    })
}

#[cfg(not(windows))]
pub fn webview2_present() -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::options::InstallMode;
    use crate::registry::{MemoryRegistry, RUN_KEY};
    use crate::shortcuts::tests::FakeFs;

    fn opts(folder: &Path, mode: InstallMode, desktop: bool, start: bool) -> InstallOptions {
        InstallOptions {
            folder: folder.to_path_buf(),
            desktop_shortcut: desktop,
            start_with_windows: start,
            download_models: false,
            mode,
        }
    }

    /// A stand-in installer: reads the folder from `/D=`, writes one small file there and exits
    /// with `code`.
    fn fake_payload(code: i32) -> impl Fn(&Path) -> io::Result<PathBuf> {
        move |dir: &Path| {
            #[cfg(windows)]
            {
                let p = dir.join("fake-installer.bat");
                let script = format!(
                    "@echo off\r\nfor %%a in (%*) do set \"d=%%~a\"\r\nset \"d=%d:~3%\"\r\nmkdir \"%d%\"\r\necho 0123456789> \"%d%\\app.bin\"\r\nexit /b {code}\r\n"
                );
                std::fs::write(&p, script)?;
                Ok(p)
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let p = dir.join("fake-installer.sh");
                let script = format!("#!/bin/sh\nfor a; do d=\"$a\"; done
d=\"${{d#/D=}}\"\nmkdir -p \"$d\"\necho 0123456789 > \"$d/app.bin\"\nexit {code}\n");
                std::fs::write(&p, script)?;
                std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755))?;
                Ok(p)
            }
        }
    }

    struct Run {
        result: Result<(), InstallError>,
        progress: Vec<SetupProgress>,
        reg: MemoryRegistry,
        fs: FakeFs,
        folder: PathBuf,
        id: Identity,
        desktop: PathBuf,
        _tmp: tempfile::TempDir,
    }

    fn run(code: i32, mode: InstallMode, desktop_on: bool, start: bool, shortcut_before: bool, installer_makes_shortcut: bool) -> Run {
        let tmp = tempfile::tempdir().unwrap();
        let temp = tmp.path().join("temp");
        std::fs::create_dir(&temp).unwrap();
        let folder = tmp.path().join("Apps").join("Lumen");
        let id = Identity::named("Lumen Test");
        let desktop = tmp.path().join("Desktop");
        let link = desktop_shortcut_path(&desktop, &id);
        let fs = FakeFs::default();
        if shortcut_before {
            fs.0.borrow_mut().insert(link.clone());
        }
        let mut reg = MemoryRegistry::default();
        let mut progress = Vec::new();
        let env = Env { id: &id, temp: &temp, desktop: Some(&desktop), installed_bytes: 12, silent: true };
        // Writing the payload happens after `existed` is read, so its side effect plays the NSIS
        // template creating the shortcut.
        let payload = fake_payload(code);
        let shared = fs.clone();
        let write = move |d: &Path| {
            if installer_makes_shortcut {
                shared.0.borrow_mut().insert(link.clone());
            }
            payload(d)
        };
        let mut my_fs = fs.clone();
        let o = opts(&folder, mode, desktop_on, start);
        let result = install_with(&env, &write, &mut reg, &mut my_fs, &o, &mut |p| progress.push(p));
        Run { result, progress, reg, fs, folder, id, desktop, _tmp: tmp }
    }

    #[test]
    fn installer_args_drop_silent_only_without_webview2() {
        let o = opts(Path::new("C:/x"), InstallMode::Install, true, false);
        assert_eq!(installer_args(&o, true).first().unwrap(), "/S");
        let loud = installer_args(&o, false);
        assert!(!loud.iter().any(|a| a == "/S"));
        assert!(loud.last().unwrap().to_string_lossy().starts_with("/D="), "/D= stays last");
    }

    #[test]
    fn a_successful_install_reports_progress_up_to_the_whole() {
        let r = run(0, InstallMode::Install, true, false, false, false);
        r.result.unwrap();
        assert!(r.folder.join("app.bin").exists());
        let last = r.progress.last().expect("progress");
        assert_eq!((last.task, last.done, last.total), (SetupTask::Lumen, 12, 12));
        assert!(r.progress.iter().all(|p| p.task == SetupTask::Lumen && p.done <= p.total));
    }

    #[test]
    fn a_failed_installer_is_an_error_and_changes_nothing_else() {
        let r = run(1, InstallMode::Install, false, true, false, true);
        match r.result {
            Err(InstallError::Failed { code: Some(1) }) => {}
            other => panic!("expected the installer's exit code, got {other:?}"),
        }
        assert!(r.reg.0.is_empty(), "no Run value after a failed install");
        assert!(r.fs.exists(&desktop_shortcut_path(&r.desktop, &r.id)), "the shortcut is left as it was");
    }

    #[test]
    fn a_silent_install_removes_the_desktop_shortcut_when_it_is_off() {
        let r = run(0, InstallMode::Install, false, false, false, true);
        r.result.unwrap();
        assert!(!r.fs.exists(&desktop_shortcut_path(&r.desktop, &r.id)));
    }

    #[test]
    fn an_update_leaves_the_shortcut_the_person_had() {
        let r = run(0, InstallMode::Update, false, false, true, true);
        r.result.unwrap();
        assert!(r.fs.exists(&desktop_shortcut_path(&r.desktop, &r.id)));
    }

    #[test]
    fn start_with_windows_writes_or_removes_only_our_run_value() {
        let on = run(0, InstallMode::Install, true, true, false, false);
        on.result.unwrap();
        let v = on.reg.read(RUN_KEY, on.id.run_value_name()).expect("Run value written");
        assert!(v.ends_with("--minimized") && v.starts_with('"'));
        assert!(v.contains(&on.id.exe_name()));
        let off = run(0, InstallMode::Install, true, false, false, false);
        off.result.unwrap();
        assert!(off.reg.read(RUN_KEY, off.id.run_value_name()).is_none());
    }

    #[test]
    fn an_update_or_repair_leaves_start_with_windows_as_it_was() {
        // The Installed screen has no switches: Update and Repair send start = false.
        for mode in [InstallMode::Update, InstallMode::Repair] {
            for had in [true, false] {
                let tmp = tempfile::tempdir().unwrap();
                let temp = tmp.path().join("t");
                std::fs::create_dir(&temp).unwrap();
                let id = Identity::named("Lumen Test");
                let env = Env { id: &id, temp: &temp, desktop: None, installed_bytes: 12, silent: true };
                let folder = tmp.path().join("L");
                let mut reg = MemoryRegistry::default();
                if had {
                    reg.write(RUN_KEY, id.run_value_name(), "\"C:\\old\\lumen-test.exe\" --minimized").unwrap();
                }
                let mut fs = FakeFs::default();
                let mut o = opts(&folder, mode, true, false);
                o.start_with_windows = !had;
                install_with(&env, &fake_payload(0), &mut reg, &mut fs, &o, &mut |_| {}).unwrap();
                assert_eq!(reg.read(RUN_KEY, id.run_value_name()).is_some(), had, "{mode:?}, had {had}");
            }
        }
    }

    #[test]
    fn the_payload_is_removed_from_temp_afterwards() {
        let tmp = tempfile::tempdir().unwrap();
        let temp = tmp.path().join("t");
        std::fs::create_dir(&temp).unwrap();
        let id = Identity::named("Lumen Test");
        let env = Env { id: &id, temp: &temp, desktop: None, installed_bytes: 12, silent: true };
        let folder = tmp.path().join("L");
        let mut reg = MemoryRegistry::default();
        let mut fs = FakeFs::default();
        install_with(&env, &fake_payload(0), &mut reg, &mut fs, &opts(&folder, InstallMode::Install, true, false), &mut |_| {}).unwrap();
        assert_eq!(std::fs::read_dir(&temp).unwrap().count(), 0);
    }

    #[test]
    fn a_missing_payload_says_so_in_plain_words() {
        let tmp = tempfile::tempdir().unwrap();
        let id = Identity::named("Lumen Test");
        let env = Env { id: &id, temp: tmp.path(), desktop: None, installed_bytes: 12, silent: true };
        let mut reg = MemoryRegistry::default();
        let mut fs = FakeFs::default();
        let none = |_: &Path| -> io::Result<PathBuf> { Err(io::Error::new(io::ErrorKind::NotFound, "this Setup carries no installer")) };
        let err = install_with(&env, &none, &mut reg, &mut fs, &opts(tmp.path(), InstallMode::Install, true, false), &mut |_| {}).unwrap_err();
        assert!(err.to_string().contains("carries no installer"));
    }
}
