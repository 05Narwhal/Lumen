//! The slim Setup as Lumen's uninstaller (spec *Uninstall*). Windows' *Installed apps* runs
//! `"<folder>\lumen-setup.exe" --uninstall` (written by `NSIS_HOOK_POSTINSTALL`):
//! - plain `--uninstall`: copy itself to `%TEMP%` and start the copy with `--from <folder>`, so the
//!   install folder can go; the copy shows screen 6.
//! - `/UPDATE`, `/P` or `_?=<dir>` added (the stock installer's reinstall page): no window; run
//!   `<dir>\uninstall.exe` with them, in place, wait, and return its exit code.
//!
//! Removal runs NSIS's own `uninstall.exe /S _?=<dir>` (which never touches app data silently),
//! then removes only `uninstall.exe`, `lumen-setup.exe` and the folder if empty, Setup's Run value
//! and desktop shortcut, and app data only through `removal` when the box is ticked.
use crate::registry::{Identity, Registry, RUN_KEY};
use crate::removal;
use crate::shortcuts::{desktop_shortcut_path, ShortcutFs};
use std::io;
use std::path::{Path, PathBuf};

pub const SETUP_EXE: &str = "lumen-setup.exe";
pub const UNINSTALL_EXE: &str = "uninstall.exe";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Launch {
    /// Setup as itself (install, update, repair, or Remove… from screen 5).
    Window,
    /// `--uninstall` from the install folder: copy to `%TEMP%` and start the copy.
    CopyToTemp { dir: PathBuf },
    /// The `%TEMP%` copy: screen 6 for the install in `dir`.
    Remove { dir: PathBuf },
    /// `/UPDATE`, `/P` or `_?=<dir>`: the stock uninstaller with the same flags, no window.
    PassThrough { dir: PathBuf, flags: Vec<String> },
}

/// Read Setup's arguments (without the program name). `raw` is the whole command line as Windows
/// gave it: NSIS appends `_?=<dir>` unquoted, so a folder with spaces arrives split over several
/// arguments, and only the raw line keeps it whole. `own_dir` is the folder Setup runs from.
pub fn parse(args: &[String], raw: &str, own_dir: &Path) -> Launch {
    let Some(at) = args.iter().position(|a| a == "--uninstall") else { return Launch::Window };
    let rest = &args[at + 1..];
    if let Some(dir) = rest.iter().position(|a| a == "--from").and_then(|i| rest.get(i + 1)) {
        return Launch::Remove { dir: dir.into() };
    }
    let marker = rest.iter().position(|a| a.starts_with("_?="));
    let flags: Vec<String> = rest[..marker.unwrap_or(rest.len())]
        .iter()
        .filter(|a| a.eq_ignore_ascii_case("/UPDATE") || a.eq_ignore_ascii_case("/P"))
        .cloned()
        .collect();
    if marker.is_none() && flags.is_empty() {
        return Launch::CopyToTemp { dir: own_dir.into() };
    }
    Launch::PassThrough { dir: dir_after_marker(raw).unwrap_or_else(|| own_dir.into()), flags }
}

/// Everything after `_?=` (NSIS requires it last), trimmed; quotes around it are dropped.
pub fn dir_after_marker(raw: &str) -> Option<PathBuf> {
    // `?` cannot be in a Windows path, so the first `_?=` is NSIS's.
    let at = raw.find("_?=")?;
    let dir = raw[at + 3..].trim().trim_matches('"');
    (!dir.is_empty()).then(|| PathBuf::from(dir))
}

/// Run the stock uninstaller with the reinstall page's flags, in place, and wait. `_?=<dir>` goes
/// last and unquoted, as NSIS requires.
pub fn run_pass_through(uninstaller: &Path, flags: &[String], dir: &Path) -> io::Result<i32> {
    let mut cmd = std::process::Command::new(uninstaller);
    cmd.args(flags);
    let marker = format!("_?={}", dir.display());
    #[cfg(windows)]
    std::os::windows::process::CommandExt::raw_arg(&mut cmd, marker);
    #[cfg(not(windows))]
    cmd.arg(marker);
    // An uninstaller ended without a code counts as failed.
    Ok(cmd.status()?.code().unwrap_or(1))
}

/// Copy the running Setup to `temp` so the install folder can be removed while screen 6 runs.
pub fn copy_to_temp(own_exe: &Path, temp: &Path, pid: u32) -> io::Result<PathBuf> {
    let to = temp.join(format!("lumen-setup-remove-{pid}.exe"));
    std::fs::copy(own_exe, &to)?;
    Ok(to)
}

#[derive(Debug)]
pub enum RemoveError {
    Start(String),
    Failed { code: i32 },
}

impl std::fmt::Display for RemoveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RemoveError::Start(e) => write!(f, "Setup could not start Lumen's uninstaller: {e}. Lumen is still installed; try again."),
            RemoveError::Failed { code } => write!(f, "Lumen's uninstaller stopped with code {code}. Lumen is still installed; try again."),
        }
    }
}

/// What removal needs from the machine; tests pass temp folders and fakes.
pub struct RemoveEnv<'a> {
    pub id: &'a Identity,
    /// The install folder.
    pub dir: &'a Path,
    /// `<dir>\uninstall.exe`; tests pass a script.
    pub uninstaller: &'a Path,
    pub desktop: Option<&'a Path>,
    pub app_data: Option<&'a Path>,
    pub webview_data: Option<&'a Path>,
}

#[derive(Debug, Default)]
pub struct Removed {
    /// The install folder still holds something Setup did not put there.
    pub folder_left: bool,
    /// What the ticked box removed; None when it was off.
    pub data: Option<removal::Report>,
}

/// Remove Lumen once it is closed. Stops at the first step if NSIS's uninstaller fails, so a
/// failed removal leaves the Run value, the shortcut and every byte of app data.
pub fn remove_lumen(env: &RemoveEnv, reg: &mut dyn Registry, fs: &mut dyn ShortcutFs, remove_data: bool) -> Result<Removed, RemoveError> {
    let code = run_pass_through(env.uninstaller, &["/S".to_string()], env.dir).map_err(|e| RemoveError::Start(e.to_string()))?;
    if code != 0 {
        return Err(RemoveError::Failed { code });
    }
    // With `_?=` NSIS leaves its uninstaller and the folder; these two files are ours to remove,
    // and the folder goes only when nothing else is in it (never a recursive delete).
    for name in [UNINSTALL_EXE, SETUP_EXE] {
        let _ = std::fs::remove_file(env.dir.join(name));
    }
    let folder_left = env.dir.exists() && std::fs::remove_dir(env.dir).is_err();
    let _ = reg.remove(RUN_KEY, env.id.run_value_name());
    // NSIS's uninstaller leaves `Software\<publisher>\<product>` (the install folder and its
    // language page's choice); only those two values, then the key if that left it empty.
    let _ = reg.remove(&env.id.product_key(), "");
    let _ = reg.remove(&env.id.product_key(), "Installer Language");
    let _ = reg.remove_key_if_empty(&env.id.product_key());
    if let Some(desktop) = env.desktop {
        let link = desktop_shortcut_path(desktop, env.id);
        if fs.exists(&link) {
            let _ = fs.remove(&link);
        }
    }
    let data = match (remove_data, env.app_data, env.webview_data) {
        (true, Some(app), Some(web)) => Some(removal::execute(&removal::plan_for(app, web, true))),
        _ => None,
    };
    Ok(Removed { folder_left, data })
}

/// Set when Setup is the `%TEMP%` copy removing the install in this folder (screen 6 first).
pub static REMOVE_DIR: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();

/// The command line exactly as Windows gave it.
#[cfg(windows)]
pub fn raw_command_line() -> String {
    use windows_sys::Win32::System::Environment::GetCommandLineW;
    // SAFETY: GetCommandLineW returns a NUL-terminated string owned by the process, valid for
    // its life; it is copied here.
    unsafe {
        let p = GetCommandLineW();
        if p.is_null() {
            return String::new();
        }
        let len = (0..).take_while(|&i| *p.add(i) != 0).count();
        String::from_utf16_lossy(std::slice::from_raw_parts(p, len))
    }
}

#[cfg(not(windows))]
pub fn raw_command_line() -> String {
    std::env::args().collect::<Vec<_>>().join(" ")
}

/// Decide what this launch is before any window. `Some(code)`: exit with it, no window.
pub fn route() -> Option<i32> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let own = std::env::current_exe().ok();
    let own_dir = own.as_deref().and_then(Path::parent).map(Path::to_path_buf).unwrap_or_default();
    match parse(&args, &raw_command_line(), &own_dir) {
        Launch::Window => None,
        Launch::Remove { dir } => {
            let _ = REMOVE_DIR.set(dir);
            None
        }
        Launch::PassThrough { dir, flags } => Some(run_pass_through(&dir.join(UNINSTALL_EXE), &flags, &dir).unwrap_or(2)),
        Launch::CopyToTemp { dir } => {
            let started = own.as_deref().ok_or_else(|| io::Error::other("no own path")).and_then(|own| {
                let copy = copy_to_temp(own, &std::env::temp_dir(), std::process::id())?;
                std::process::Command::new(copy).arg("--uninstall").arg("--from").arg(&dir).spawn()
            });
            match started {
                Ok(_) => Some(0),
                // No copy: screen 6 from here still removes everything but this file and folder.
                Err(_) => {
                    let _ = REMOVE_DIR.set(dir);
                    None
                }
            }
        }
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use crate::registry::MemoryRegistry;
    use crate::shortcuts::tests::FakeFs;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    const OWN: &str = r"C:\Users\A\AppData\Local\Lumen";

    #[test]
    fn plain_setup_is_a_window() {
        assert_eq!(parse(&[], "\"C:\\S\\lumen-setup.exe\"", Path::new(OWN)), Launch::Window);
    }

    #[test]
    fn uninstall_alone_copies_to_temp() {
        let raw = format!("\"{OWN}\\lumen-setup.exe\" --uninstall");
        assert_eq!(parse(&s(&["--uninstall"]), &raw, Path::new(OWN)), Launch::CopyToTemp { dir: OWN.into() });
    }

    #[test]
    fn the_temp_copy_removes_the_folder_it_was_given() {
        let dir = r"C:\Users\A B\Apps\Lumen";
        let raw = format!("\"C:\\T\\lumen-setup-remove-4.exe\" --uninstall --from \"{dir}\"");
        assert_eq!(parse(&s(&["--uninstall", "--from", dir]), &raw, Path::new(r"C:\T")), Launch::Remove { dir: dir.into() });
    }

    #[test]
    fn the_reinstall_page_passes_through_with_a_folder_that_has_spaces() {
        // NSIS: ExecWait '"<UninstallString>" /UPDATE /P _?=<dir>' with <dir> unquoted.
        let raw = format!("\"{OWN}\\lumen-setup.exe\" --uninstall /UPDATE /P _?=C:\\Users\\A B\\My  Apps\\Lumen");
        let args = s(&["--uninstall", "/UPDATE", "/P", r"_?=C:\Users\A", "B\\My", r"Apps\Lumen"]);
        assert_eq!(
            parse(&args, &raw, Path::new(OWN)),
            Launch::PassThrough { dir: r"C:\Users\A B\My  Apps\Lumen".into(), flags: s(&["/UPDATE", "/P"]) },
            "the folder comes whole from the raw line, double space kept"
        );
    }

    #[test]
    fn any_of_the_three_means_pass_through() {
        let p = Path::new(OWN);
        assert_eq!(parse(&s(&["--uninstall", "_?=C:\\L"]), "x --uninstall _?=C:\\L", p), Launch::PassThrough { dir: r"C:\L".into(), flags: vec![] });
        assert_eq!(parse(&s(&["--uninstall", "/P"]), "x --uninstall /P", p), Launch::PassThrough { dir: OWN.into(), flags: s(&["/P"]) });
        assert_eq!(parse(&s(&["--uninstall", "/UPDATE"]), "x --uninstall /UPDATE", p), Launch::PassThrough { dir: OWN.into(), flags: s(&["/UPDATE"]) });
    }

    #[test]
    fn dir_after_marker_takes_the_rest_of_the_line() {
        assert_eq!(dir_after_marker("a /P _?=C:\\A B\\L  "), Some(PathBuf::from(r"C:\A B\L")));
        assert_eq!(dir_after_marker("a _?=\"C:\\A B\""), Some(PathBuf::from(r"C:\A B")));
        assert_eq!(dir_after_marker("a /P"), None);
        assert_eq!(dir_after_marker("a _?="), None);
    }

    /// A stand-in `uninstall.exe`: writes its arguments to `<out>`, deletes `<exe>` (as NSIS removes
    /// Lumen's files) and exits with `code`.
    fn fake_uninstaller(at: &Path, out: &Path, exe: Option<&Path>, code: i32) -> PathBuf {
        let p = at.join("fake-uninstall.bat");
        let del = exe.map(|e| format!("del \"{}\"\r\n", e.display())).unwrap_or_default();
        std::fs::write(&p, format!("@echo off\r\necho %*> \"{}\"\r\n{del}exit /b {code}\r\n", out.display())).unwrap();
        p
    }

    #[test]
    fn pass_through_keeps_the_flags_in_order_and_returns_the_code() {
        let t = tempfile::tempdir().unwrap();
        let dir = t.path().join("My Apps").join("Lumen");
        std::fs::create_dir_all(&dir).unwrap();
        let out = t.path().join("args.txt");
        let un = fake_uninstaller(t.path(), &out, None, 3);
        let code = run_pass_through(&un, &s(&["/UPDATE", "/P"]), &dir).unwrap();
        assert_eq!(code, 3);
        assert_eq!(std::fs::read_to_string(&out).unwrap().trim(), format!("/UPDATE /P _?={}", dir.display()));
    }

    #[test]
    fn copy_to_temp_makes_a_runnable_copy() {
        let t = tempfile::tempdir().unwrap();
        let own = t.path().join(SETUP_EXE);
        std::fs::write(&own, b"MZ").unwrap();
        let temp = t.path().join("temp");
        std::fs::create_dir(&temp).unwrap();
        let copy = copy_to_temp(&own, &temp, 4).unwrap();
        assert_eq!(copy, temp.join("lumen-setup-remove-4.exe"));
        assert_eq!(std::fs::read(&copy).unwrap(), b"MZ");
        assert!(own.exists(), "the original stays until NSIS's uninstaller runs");
    }

    struct World {
        _t: tempfile::TempDir,
        id: Identity,
        dir: PathBuf,
        un: PathBuf,
        out: PathBuf,
        desktop: PathBuf,
        app: PathBuf,
        web: PathBuf,
        library: PathBuf,
        reg: MemoryRegistry,
        fs: FakeFs,
    }

    /// An install folder (with a space), a desktop shortcut, Run values, app data with a library
    /// inside it and a WebView2 folder.
    fn world(code: i32) -> World {
        let t = tempfile::tempdir().unwrap();
        let id = Identity::named("Lumen Test");
        let dir = t.path().join("Apps").join("Lumen Test");
        std::fs::create_dir_all(&dir).unwrap();
        let exe = dir.join(id.exe_name());
        for f in [UNINSTALL_EXE, SETUP_EXE, &id.exe_name()] {
            std::fs::write(dir.join(f), b"x").unwrap();
        }
        let out = t.path().join("args.txt");
        let un = fake_uninstaller(t.path(), &out, Some(&exe), code);
        let desktop = t.path().join("Desktop");
        let fs = FakeFs::default();
        fs.0.borrow_mut().insert(desktop_shortcut_path(&desktop, &id));
        let mut reg = MemoryRegistry::default();
        reg.write(RUN_KEY, id.run_value_name(), "\"x\" --minimized").unwrap();
        reg.write(RUN_KEY, "Other", "other.exe").unwrap();
        let app = t.path().join("app");
        let library = app.join("My Library");
        std::fs::create_dir_all(library.join("originals")).unwrap();
        std::fs::write(library.join("library.db"), b"db").unwrap();
        std::fs::write(app.join("settings.json"), b"{}").unwrap();
        std::fs::create_dir_all(app.join("logs")).unwrap();
        std::fs::write(app.join("logs/lumen.log"), b"l").unwrap();
        let registry = serde_json::json!({ "version": 2, "libraries": [{ "id": "a", "name": "L", "path": library.to_string_lossy(), "lastOpenedAt": 0 }], "current": "a" });
        std::fs::write(app.join("libraries.json"), registry.to_string()).unwrap();
        let web = t.path().join("web");
        std::fs::create_dir_all(web.join("EBWebView")).unwrap();
        World { _t: t, id, dir, un, out, desktop, app, web, library, reg, fs }
    }

    fn remove(w: &mut World, tick: bool) -> Result<Removed, RemoveError> {
        let env = RemoveEnv {
            id: &w.id,
            dir: &w.dir,
            uninstaller: &w.un,
            desktop: Some(&w.desktop),
            app_data: Some(&w.app),
            webview_data: Some(&w.web),
        };
        remove_lumen(&env, &mut w.reg, &mut w.fs, tick)
    }

    #[test]
    fn the_uninstaller_runs_silently_in_place() {
        let mut w = world(0);
        remove(&mut w, false).unwrap();
        assert_eq!(std::fs::read_to_string(&w.out).unwrap().trim(), format!("/S _?={}", w.dir.display()));
    }

    #[test]
    fn our_two_files_and_the_empty_folder_go() {
        let mut w = world(0);
        let r = remove(&mut w, false).unwrap();
        assert!(!w.dir.exists(), "uninstall.exe, lumen-setup.exe and then the empty folder");
        assert!(!r.folder_left);
    }

    #[test]
    fn a_folder_with_something_else_in_it_stays_with_that_thing() {
        let mut w = world(0);
        std::fs::write(w.dir.join("notes.txt"), b"mine").unwrap();
        let r = remove(&mut w, false).unwrap();
        assert!(r.folder_left);
        assert_eq!(std::fs::read(w.dir.join("notes.txt")).unwrap(), b"mine");
        assert!(!w.dir.join(UNINSTALL_EXE).exists() && !w.dir.join(SETUP_EXE).exists());
    }

    #[test]
    fn our_run_value_and_desktop_shortcut_go_and_nothing_else() {
        let mut w = world(0);
        remove(&mut w, false).unwrap();
        assert_eq!(w.reg.read(RUN_KEY, w.id.run_value_name()), None);
        assert_eq!(w.reg.read(RUN_KEY, "Other").as_deref(), Some("other.exe"));
        assert!(!w.fs.exists(&desktop_shortcut_path(&w.desktop, &w.id)));
    }

    #[test]
    fn the_product_key_nsis_leaves_goes_and_its_neighbours_stay() {
        let mut w = world(0);
        let key = w.id.product_key();
        w.reg.write(&key, "", &w.dir.display().to_string()).unwrap();
        // NSIS's language page remembers its choice here too (seen on a real install).
        w.reg.write(&key, "Installer Language", "1033").unwrap();
        w.reg.write(&key, "Something Else", "kept").unwrap();
        w.reg.write(r"Software\JL Studios\Other App", "", "kept").unwrap();
        remove(&mut w, false).unwrap();
        assert_eq!(w.reg.read(&key, ""), None);
        assert_eq!(w.reg.read(&key, "Installer Language"), None);
        assert_eq!(w.reg.read(&key, "Something Else").as_deref(), Some("kept"), "only NSIS's two values");
        assert_eq!(w.reg.read(r"Software\JL Studios\Other App", "").as_deref(), Some("kept"));
    }

    #[test]
    fn a_failed_uninstaller_keeps_the_product_key() {
        let mut w = world(3);
        let key = w.id.product_key();
        w.reg.write(&key, "", "C:/L").unwrap();
        assert!(remove(&mut w, false).is_err());
        assert_eq!(w.reg.read(&key, "").as_deref(), Some("C:/L"));
    }

    #[test]
    fn app_data_is_untouched_unless_the_box_is_ticked() {
        let mut w = world(0);
        let r = remove(&mut w, false).unwrap();
        assert!(r.data.is_none());
        for p in ["settings.json", "libraries.json", "logs/lumen.log"] {
            assert!(w.app.join(p).exists(), "{p} stays with the box off");
        }
        assert!(w.web.join("EBWebView").exists());
    }

    #[test]
    fn the_ticked_box_removes_by_allow_list_and_the_library_inside_survives() {
        let mut w = world(0);
        let r = remove(&mut w, true).unwrap();
        let report = r.data.expect("the box was ticked");
        assert!(report.failed.is_empty(), "{:?}", report.failed);
        assert!(!w.app.join("settings.json").exists() && !w.app.join("logs").exists());
        assert!(!w.web.join("EBWebView").exists());
        assert_eq!(std::fs::read(w.library.join("library.db")).unwrap(), b"db");
        assert!(w.library.join("originals").is_dir());
    }

    #[test]
    fn a_failed_uninstaller_stops_before_anything_else() {
        let mut w = world(4);
        let e = remove(&mut w, true).unwrap_err();
        assert!(matches!(e, RemoveError::Failed { code: 4 }), "{e:?}");
        assert!(e.to_string().contains("still installed"));
        assert!(w.dir.join(SETUP_EXE).exists() && w.dir.join(UNINSTALL_EXE).exists());
        assert!(w.reg.read(RUN_KEY, w.id.run_value_name()).is_some());
        assert!(w.fs.exists(&desktop_shortcut_path(&w.desktop, &w.id)));
        assert!(w.app.join("settings.json").exists() && w.app.join("libraries.json").exists());
    }
}
