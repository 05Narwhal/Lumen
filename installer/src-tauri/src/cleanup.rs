//! What a finished Setup would leave in `%TEMP%`: its WebView2 profile and, as the uninstaller, the
//! copy of itself (`lumen-setup-remove-<pid>.exe`). Both are in use until Setup and WebView2's own
//! processes have ended, so at exit a hidden `cmd` waits a few seconds and deletes them. It only
//! ever names those two: the fixed profile folder in the temp folder and an exe of that pattern
//! directly in it.
use std::path::Path;

pub const PROFILE_NAME: &str = "Lumen Setup WebView";
pub const COPY_PREFIX: &str = "lumen-setup-remove-";

/// `cmd /d /c` arguments that wait about five seconds, then delete what is Setup's own; None
/// when nothing qualifies. A path holding `%` or `"` is left alone (cmd would read it).
pub fn command_line(temp: &Path, profile: &Path, own_exe: &Path) -> Option<String> {
    let safe = |p: &Path| p.to_str().filter(|s| !s.contains('%') && !s.contains('"')).map(str::to_string);
    let mut steps = Vec::new();
    if profile == temp.join(PROFILE_NAME) {
        if let Some(p) = safe(profile) {
            steps.push(format!("rd /s /q \"{p}\""));
        }
    }
    let is_copy = own_exe.parent() == Some(temp)
        && own_exe.file_name().and_then(|n| n.to_str()).is_some_and(|n| {
            let n = n.to_ascii_lowercase();
            n.starts_with(COPY_PREFIX) && n.ends_with(".exe")
        });
    if is_copy {
        if let Some(p) = safe(own_exe) {
            steps.push(format!("del /f /q \"{p}\""));
        }
    }
    if steps.is_empty() {
        return None;
    }
    Some(format!("/d /c ping -n 6 127.0.0.1 >nul & {}", steps.join(" & ")))
}

/// Start the delayed delete as Setup exits. Never fails Setup: a leftover is only a leftover.
#[cfg(windows)]
pub fn start_after_exit(profile: &Path) {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let temp = std::env::temp_dir();
    let (Ok(own), Some(root)) = (std::env::current_exe(), std::env::var_os("SystemRoot")) else { return };
    let Some(line) = command_line(&temp, profile, &own) else { return };
    // The full path: never a `cmd.exe` found beside this exe in the temp folder.
    let cmd = Path::new(&root).join("System32").join("cmd.exe");
    let _ = std::process::Command::new(cmd).raw_arg(line).current_dir(&root).creation_flags(CREATE_NO_WINDOW).spawn();
}

#[cfg(not(windows))]
pub fn start_after_exit(_profile: &Path) {}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn temp() -> PathBuf {
        PathBuf::from(r"C:\Users\A B\AppData\Local\Temp")
    }

    #[test]
    fn the_temp_copy_deletes_its_profile_and_itself() {
        let t = temp();
        let line = command_line(&t, &t.join(PROFILE_NAME), &t.join("lumen-setup-remove-42.exe")).unwrap();
        assert_eq!(
            line,
            r#"/d /c ping -n 6 127.0.0.1 >nul & rd /s /q "C:\Users\A B\AppData\Local\Temp\Lumen Setup WebView" & del /f /q "C:\Users\A B\AppData\Local\Temp\lumen-setup-remove-42.exe""#
        );
    }

    #[test]
    fn an_installed_or_downloaded_setup_deletes_only_the_profile() {
        let t = temp();
        for exe in [PathBuf::from(r"C:\Users\A B\AppData\Local\Lumen\lumen-setup.exe"), PathBuf::from(r"C:\Users\A B\Downloads\Lumen-Setup-26.1.0.exe"), t.join("lumen-setup.exe"), t.join("sub").join("lumen-setup-remove-1.exe")] {
            let line = command_line(&t, &t.join(PROFILE_NAME), &exe).unwrap();
            assert!(!line.contains("del "), "{exe:?}: {line}");
            assert!(line.contains("rd /s /q"));
        }
    }

    #[test]
    fn no_other_folder_is_ever_deleted() {
        let t = temp();
        let exe = PathBuf::from(r"C:\L\lumen-setup.exe");
        assert_eq!(command_line(&t, &t, &exe), None, "the temp folder itself");
        assert_eq!(command_line(&t, &t.join("Other"), &exe), None);
        assert_eq!(command_line(&t, &PathBuf::from(r"C:\Lumen Setup WebView"), &exe), None);
    }

    #[test]
    fn a_path_cmd_would_expand_is_left_alone() {
        let t = PathBuf::from(r"C:\Users\100%\Temp");
        assert_eq!(command_line(&t, &t.join(PROFILE_NAME), &t.join("lumen-setup-remove-1.exe")), None);
    }
}
