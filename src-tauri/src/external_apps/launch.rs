//! Starting an app with files (Round 22 lane C, spec §8 *Launch*): the program itself on Windows,
//! `open -a` for a Mac bundle, the `.desktop` `Exec` line on Linux. No shell anywhere, so a path
//! with spaces or accents reaches the app as one argument. A Windows Store app has no file to run,
//! so each file goes through the shell with that app's own ProgID (`launch_store`).

use std::ffi::OsString;
use std::path::PathBuf;
use std::process::Command;

use photos_core::external_apps::{exec_launches, InstalledApp, Launcher, Os};

/// The launcher Lumen uses.
pub struct SystemLauncher;

/// One program and its arguments.
pub type Launch = (OsString, Vec<OsString>);

/// What `SystemLauncher` runs for `files` on `os`: one launch, or one per file when a Linux entry
/// takes a single file (`%f`).
pub fn launches(os: Os, app: &InstalledApp, files: &[PathBuf]) -> Vec<Launch> {
    let direct = || vec![(app.target.clone().into_os_string(), files.iter().map(|f| f.clone().into_os_string()).collect())];
    match os {
        Os::Mac if app.target.extension().is_some_and(|e| e.eq_ignore_ascii_case("app")) => {
            let mut args: Vec<OsString> = vec!["-a".into(), app.target.clone().into_os_string()];
            args.extend(files.iter().map(|f| f.clone().into_os_string()));
            vec![("open".into(), args)]
        }
        Os::Linux => match &app.exec {
            Some(exec) => exec_launches(exec, files)
                .into_iter()
                .filter_map(|mut argv| {
                    if argv.is_empty() {
                        return None;
                    }
                    let program = argv.remove(0);
                    Some((program.into(), argv.into_iter().map(OsString::from).collect()))
                })
                .collect(),
            None => direct(),
        },
        _ => direct(),
    }
}

impl Launcher for SystemLauncher {
    fn launch(&self, app: &InstalledApp, files: &[PathBuf]) -> Result<(), String> {
        #[cfg(windows)]
        if super::system::is_store(&app.target) {
            return launch_store(app, files);
        }
        for (program, args) in launches(Os::current(), app, files) {
            let mut child = Command::new(&program).args(&args).spawn().map_err(|e| format!("{}: {e}", program.to_string_lossy()))?;
            // Reaped in the background; the app lives on after Lumen closes (never put in
            // `child_jobs`, which ends its children with Lumen).
            std::thread::spawn(move || {
                let _ = child.wait();
            });
        }
        Ok(())
    }
}

/// One `ShellExecuteExW` per file, told to use the Store app's ProgID so the file opens in *that*
/// app and not the default one. An extension the app registers nothing for is an error (a toast),
/// checked for every file first so nothing opens half way.
#[cfg(windows)]
fn launch_store(app: &InstalledApp, files: &[PathBuf]) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED};
    use windows_sys::Win32::UI::Shell::{ShellExecuteExW, SEE_MASK_CLASSNAME, SEE_MASK_NOASYNC, SHELLEXECUTEINFOW};
    use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

    let wide = |s: &std::ffi::OsStr| -> Vec<u16> { s.encode_wide().chain(Some(0)).collect() };
    let mut jobs = Vec::new();
    for file in files {
        let ext = file.extension().map(|e| e.to_string_lossy().into_owned()).unwrap_or_default();
        let progid = super::system::store_progid(&app.target, &ext).ok_or_else(|| format!("{} can't open .{ext} files", app.name))?;
        jobs.push((wide(file.as_os_str()), wide(std::ffi::OsStr::new(&progid))));
    }
    let verb = wide(std::ffi::OsStr::new("open"));
    // SAFETY: COM is balanced below; a failed or already-initialised thread is fine for the shell.
    let com = unsafe { CoInitializeEx(std::ptr::null(), COINIT_APARTMENTTHREADED as u32) } >= 0;
    let mut result = Ok(());
    for (file, class) in &jobs {
        // SAFETY: every pointer is NUL-terminated and outlives the call; the struct is plain data.
        let ok = unsafe {
            let mut info: SHELLEXECUTEINFOW = std::mem::zeroed();
            info.cbSize = std::mem::size_of::<SHELLEXECUTEINFOW>() as u32;
            info.fMask = SEE_MASK_CLASSNAME | SEE_MASK_NOASYNC;
            info.lpVerb = verb.as_ptr();
            info.lpFile = file.as_ptr();
            info.lpClass = class.as_ptr();
            info.nShow = SW_SHOWNORMAL;
            ShellExecuteExW(&mut info) != 0
        };
        if !ok {
            result = Err(format!("{} could not open the file", app.name));
            break;
        }
    }
    if com {
        // SAFETY: pairs the successful `CoInitializeEx` above.
        unsafe { CoUninitialize() };
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use photos_core::external_apps::{AppKind, Media, Writes};

    fn app(target: &str, exec: Option<&str>) -> InstalledApp {
        InstalledApp {
            id: "x".into(),
            name: "X".into(),
            kind: AppKind::Editor,
            accepts: vec![Media::Photo],
            reads_raw: false,
            writes: Writes::InPlace,
            target: PathBuf::from(target),
            exec: exec.map(Into::into),
            custom: false,
            added_by: None,
        }
    }

    fn os(s: &[&str]) -> Vec<OsString> {
        s.iter().map(OsString::from).collect()
    }

    #[test]
    fn each_os_starts_the_app_with_whole_paths() {
        let files = [PathBuf::from("D:/Lib/originals/2024 x/Čaj ü 1-Edit.jpg"), PathBuf::from("D:/b.tif")];
        let win = launches(Os::Windows, &app("C:/Program Files/Adobe/Photoshop.exe", None), &files);
        assert_eq!(win, vec![("C:/Program Files/Adobe/Photoshop.exe".into(), os(&["D:/Lib/originals/2024 x/Čaj ü 1-Edit.jpg", "D:/b.tif"]))]);
        let mac = launches(Os::Mac, &app("/Applications/Pixelmator Pro.app", None), &files[1..]);
        assert_eq!(mac, vec![("open".into(), os(&["-a", "/Applications/Pixelmator Pro.app", "D:/b.tif"]))]);
        // `%f` takes one file per launch.
        let linux = launches(Os::Linux, &app("/usr/share/applications/gimp.desktop", Some("gimp-2.10 %f")), &files);
        assert_eq!(linux.len(), 2);
        assert_eq!(linux[1], ("gimp-2.10".into(), os(&["D:/b.tif"])));
        // A custom app on Linux has no Exec line: the program itself.
        assert_eq!(launches(Os::Linux, &app("/opt/x/editor", None), &files[1..]), vec![("/opt/x/editor".into(), os(&["D:/b.tif"]))]);
    }
}
