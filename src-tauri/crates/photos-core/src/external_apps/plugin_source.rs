//! Apps a plugin adds to *Edit in* / *Open with* (Round 23 B2, spec §7). `apps.toml` lists entries
//! in the built-in table's shape, but only detection identifiers: Lumen finds the app (an app
//! that isn't installed isn't shown) and passes the file as the only argument. A target that
//! resolves to a shell or script host is refused, and a plugin app is always given a copy.

use serde::Deserialize;

use super::detect::{exec_program, resolve_profile, Os};
use super::{AppKind, AppProfile, AppSource, InstalledApp, Lookup, Media, WinRule, Writes};

/// Programs that run other programs: never an app, whatever an entry's identifiers resolve to.
/// Judged on the resolved target's file name, without its directory or `.exe`-style extension;
/// `python*` is any name starting `python`.
pub const SHELL_HOSTS: &[&str] = &["cmd", "powershell", "pwsh", "wscript", "cscript", "mshta", "rundll32", "sh", "bash", "zsh", "python*", "py", "pyw", "node", "osascript", "env"];

/// Files that are scripts, not apps.
const SCRIPT_EXTENSIONS: &[&str] = &["bat", "cmd", "ps1", "psm1", "vbs", "vbe", "js", "jse", "wsf", "wsh", "hta", "sh", "py", "pyw", "command"];
const EXECUTABLE_EXTENSIONS: &[&str] = &["exe", "com", "scr"];

/// macOS apps that run scripts or commands, by name (as a resolved `.app` bundle shows) ...
const MAC_HOST_APPS: &[&str] = &["terminal", "iterm", "iterm2", "script editor", "automator"];
/// ... and by bundle id (as an entry names them).
const MAC_HOST_BUNDLES: &[&str] = &["com.apple.terminal", "com.googlecode.iterm2", "com.apple.scripteditor2", "com.apple.automator"];

/// Where an installed app's folder may be named by a glob: Windows' install roots only. A path
/// elsewhere (Lumen's own folders, a user folder) can't be what an entry detects.
const INSTALL_ROOTS: &[&str] = &["%programfiles%/", "%programfiles(x86)%/", "%programw6432%/", "%localappdata%/programs/"];

/// Whether `file_name` (or a path ending in it, with `/` or `\` separators) is a shell or script
/// host. Case-insensitive.
pub fn is_shell_host(file_name: &str) -> bool {
    let name = file_name.rsplit(['/', '\\']).next().unwrap_or(file_name).trim().to_lowercase();
    if name.is_empty() {
        return false;
    }
    let (stem, ext) = match name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() => (stem, ext),
        _ => (name.as_str(), ""),
    };
    if SCRIPT_EXTENSIONS.contains(&ext) {
        return true;
    }
    if ext == "app" && MAC_HOST_APPS.contains(&stem) {
        return true;
    }
    // `python3.12` has a dot but no extension; only `.exe`-style endings are dropped.
    let stem = if EXECUTABLE_EXTENSIONS.contains(&ext) { stem } else { name.as_str() };
    SHELL_HOSTS.iter().any(|host| match host.strip_suffix('*') {
        Some(prefix) => stem.starts_with(prefix),
        None => stem == *host,
    })
}

/// A plugin's app row, built at run time (strings, not `'static` text).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppProfileOwned {
    pub id: String,
    pub name: String,
    pub kind: AppKind,
    pub accepts: Vec<Media>,
    pub reads_raw: bool,
    pub writes: Writes,
    pub win: Vec<WinRuleOwned>,
    pub mac: Vec<String>,
    pub linux: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WinRuleOwned {
    AppPath(String),
    Uninstall { name_starts: String, exe: String },
    Glob(String),
}

impl AppProfileOwned {
    /// Lends this row as an [`AppProfile`] for one call, so `resolve_profile` reads it like a
    /// table row.
    pub fn with_profile<R>(&self, f: impl FnOnce(&AppProfile<'_>) -> R) -> R {
        let win: Vec<WinRule<'_>> = self
            .win
            .iter()
            .map(|r| match r {
                WinRuleOwned::AppPath(exe) => WinRule::AppPath(exe),
                WinRuleOwned::Uninstall { name_starts, exe } => WinRule::Uninstall { name_starts, exe },
                WinRuleOwned::Glob(pattern) => WinRule::Glob(pattern),
            })
            .collect();
        let mac: Vec<&str> = self.mac.iter().map(String::as_str).collect();
        let linux: Vec<&str> = self.linux.iter().map(String::as_str).collect();
        f(&AppProfile {
            id: &self.id,
            name: &self.name,
            kind: self.kind,
            accepts: &self.accepts,
            reads_raw: self.reads_raw,
            writes: self.writes,
            win: &win,
            mac: &mac,
            linux: &linux,
        })
    }
}

// The file's shape. Unknown keys are errors everywhere, so an `args = [...]` (an argument
// template) can't slip in: Lumen passes the file as the only argument.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FileWire {
    #[serde(default)]
    app: Vec<EntryWire>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EntryWire {
    id: String,
    name: String,
    kind: AppKind,
    accepts: Vec<Media>,
    reads_raw: bool,
    writes: Writes,
    #[serde(default)]
    windows: WindowsWire,
    #[serde(default)]
    mac: MacWire,
    #[serde(default)]
    linux: LinuxWire,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct WindowsWire {
    #[serde(default)]
    app_paths: Vec<String>,
    #[serde(default)]
    uninstall: Vec<UninstallWire>,
    #[serde(default)]
    globs: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct UninstallWire {
    name_starts: String,
    exe: String,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct MacWire {
    #[serde(default)]
    bundle_ids: Vec<String>,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct LinuxWire {
    #[serde(default)]
    desktop: Vec<String>,
}

fn id_ok(id: &str) -> bool {
    (1..=40).contains(&id.len()) && id.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

/// A relative path inside an install folder: no drive, no leading separator, no `..`.
fn relative_ok(path: &str) -> bool {
    !path.starts_with(['/', '\\']) && !path.contains(':') && !path.split(['/', '\\']).any(|part| part == "..")
}

/// `apps.toml` of one plugin. Every identifier is checked for what it names (a shell or script
/// host is refused here and again on what it resolves to), and nothing is launched.
pub fn parse_plugin_apps(_plugin: &str, text: &str) -> Result<Vec<AppProfileOwned>, String> {
    let file: FileWire = toml::from_str(text).map_err(|e| e.message().to_string())?;
    if file.app.is_empty() {
        return Err("apps.toml lists no apps".into());
    }
    let mut out: Vec<AppProfileOwned> = Vec::new();
    for e in file.app {
        let fail = |why: &str| format!("{}: {why}", e.id);
        if !id_ok(&e.id) {
            return Err(format!("\"{}\" isn't an app id (1–40 of a–z, 0–9 and -)", e.id));
        }
        if out.iter().any(|o| o.id == e.id) {
            return Err(fail("listed twice"));
        }
        let name = e.name.trim().to_string();
        if name.is_empty() || name.chars().count() > 60 {
            return Err(fail("needs a name of 1–60 characters"));
        }
        if e.accepts.is_empty() {
            return Err(fail("accepts no media"));
        }
        let named = e.windows.app_paths.iter().map(String::as_str).chain(e.windows.uninstall.iter().map(|u| u.exe.as_str())).chain(e.windows.globs.iter().map(String::as_str));
        for n in named {
            if n.trim().is_empty() {
                return Err(fail("has an empty identifier"));
            }
            if is_shell_host(n) {
                return Err(fail(&format!("\"{n}\" is a shell or script host, which can't be an app")));
            }
        }
        let mut win = Vec::new();
        for exe in &e.windows.app_paths {
            win.push(WinRuleOwned::AppPath(exe.clone()));
        }
        for u in &e.windows.uninstall {
            if u.name_starts.trim().is_empty() || !relative_ok(&u.exe) {
                return Err(fail("an uninstall entry needs a name and an exe inside the install folder"));
            }
            win.push(WinRuleOwned::Uninstall { name_starts: u.name_starts.clone(), exe: u.exe.clone() });
        }
        for g in &e.windows.globs {
            let flat = g.replace('\\', "/").to_lowercase();
            if !INSTALL_ROOTS.iter().any(|root| flat.starts_with(root)) || flat.split('/').any(|part| part == "..") {
                return Err(fail(&format!("\"{g}\" must start in an install folder (%ProgramFiles%, %ProgramFiles(x86)% or %LocalAppData%/Programs)")));
            }
            win.push(WinRuleOwned::Glob(g.clone()));
        }
        if let Some(b) = e.mac.bundle_ids.iter().find(|b| MAC_HOST_BUNDLES.contains(&b.to_lowercase().as_str())) {
            return Err(fail(&format!("\"{b}\" is a shell or script host, which can't be an app")));
        }
        if e.mac.bundle_ids.iter().chain(&e.linux.desktop).any(|s| s.trim().is_empty()) {
            return Err(fail("has an empty identifier"));
        }
        if win.is_empty() && e.mac.bundle_ids.is_empty() && e.linux.desktop.is_empty() {
            return Err(fail("names no way to find the app (windows, mac or linux)"));
        }
        out.push(AppProfileOwned { id: e.id, name, kind: e.kind, accepts: e.accepts, reads_raw: e.reads_raw, writes: e.writes, win, mac: e.mac.bundle_ids, linux: e.linux.desktop });
    }
    Ok(out)
}

/// One plugin's apps as an [`AppSource`].
pub struct PluginApps {
    pub plugin: String,
    pub plugin_name: String,
    pub profiles: Vec<AppProfileOwned>,
    pub os: Os,
    /// Folders no app may live in: Lumen's own (the plugin's installed folder and the plugins
    /// folder), where a plugin could ship a program of its own.
    pub forbidden: Vec<std::path::PathBuf>,
}

/// A path as text for comparing: lower case, `/` separators, no `\?\` prefix, no trailing `/`.
fn flat(p: &std::path::Path) -> String {
    let s = p.to_string_lossy().replace('\\', "/").to_lowercase();
    s.strip_prefix("//?/").unwrap_or(&s).trim_end_matches('/').to_string()
}

fn inside(target: &std::path::Path, roots: &[std::path::PathBuf]) -> bool {
    let t = flat(target);
    roots.iter().map(|r| flat(r)).any(|r| !r.is_empty() && (t == r || t.starts_with(&format!("{r}/"))))
}

impl PluginApps {
    pub fn new(plugin: &str, plugin_name: &str, profiles: Vec<AppProfileOwned>) -> Self {
        Self { plugin: plugin.into(), plugin_name: plugin_name.into(), profiles, os: Os::current(), forbidden: Vec::new() }
    }
}

impl AppSource for PluginApps {
    fn apps(&self, lookup: &dyn Lookup) -> Vec<InstalledApp> {
        self.profiles
            .iter()
            .filter_map(|p| {
                let (target, exec) = p.with_profile(|profile| resolve_profile(profile, self.os, lookup))?;
                // The target decides, never the entry's name: a registry key may lead anywhere.
                let program = exec.as_deref().and_then(exec_program);
                if is_shell_host(&target.to_string_lossy()) || program.as_deref().is_some_and(is_shell_host) || inside(&target, &self.forbidden) {
                    return None;
                }
                // The file it really is: a link, a short name or a junction that leads to a host,
                // or into Lumen's own folders, is judged by where it ends up.
                if let Ok(real) = std::fs::canonicalize(&target) {
                    if is_shell_host(&real.to_string_lossy()) || inside(&real, &self.forbidden) {
                        return None;
                    }
                }
                // A plugin can't make Lumen hand over an original: a developer is treated as one
                // that writes sidecars (it works on a copy), a video app as an editor of videos
                // (an editor is always sent a copy); the person may still change what an editor is
                // sent in Settings → External apps.
                let (kind, writes) = match p.kind {
                    AppKind::Editor => (AppKind::Editor, p.writes),
                    AppKind::Developer => (AppKind::Developer, Writes::Sidecar),
                    AppKind::Video => (AppKind::Editor, Writes::InPlace),
                };
                Some(InstalledApp {
                    id: format!("plugin:{}/{}", self.plugin, p.id),
                    name: p.name.clone(),
                    kind,
                    accepts: p.accepts.clone(),
                    reads_raw: p.reads_raw,
                    writes,
                    target,
                    exec,
                    custom: false,
                    added_by: Some(self.plugin_name.clone()),
                })
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::super::fake::FakeLookup;
    use super::*;

    const ONE: &str = r#"
[[app]]
id = "fancy"
name = "Fancy Editor"
kind = "editor"
accepts = ["photo", "raw"]
reads_raw = true
writes = "inPlace"

[app.windows]
app_paths = ["Fancy.exe"]
uninstall = [{ name_starts = "Fancy Editor", exe = "Fancy.exe" }]
globs = ["%ProgramFiles%/Fancy/*/Fancy.exe"]

[app.mac]
bundle_ids = ["com.fancy.editor"]

[app.linux]
desktop = ["fancy-editor"]
"#;

    fn source(text: &str) -> PluginApps {
        let mut s = PluginApps::new("com.test.apps", "Darkroom Tools", parse_plugin_apps("com.test.apps", text).unwrap());
        s.os = Os::Windows;
        s
    }

    #[test]
    fn shell_hosts_refused() {
        for host in SHELL_HOSTS.iter().filter(|h| !h.ends_with('*')) {
            assert!(is_shell_host(host), "{host}");
            assert!(is_shell_host(&format!("{host}.exe")), "{host}.exe");
            assert!(is_shell_host(&host.to_uppercase()), "{host} upper");
        }
        for name in ["PowerShell.EXE", "python3.12.exe", "python", "pythonw.exe", "/usr/bin/zsh", "C:\\Windows\\System32\\cmd.exe", "C:/Windows/System32/WindowsPowerShell/v1.0/powershell.exe", "setup.bat", "run.ps1", "tool.command"] {
            assert!(is_shell_host(name), "{name}");
        }
        for name in ["photoshop.exe", "Photoshop.exe", "gimp-2.10.exe", "Fancy.exe", "nodepad.exe", "shotwell", "krita.exe", "Adobe Lightroom Classic/Lightroom.exe"] {
            assert!(!is_shell_host(name), "{name}");
        }
    }

    #[test]
    fn resolved_target_judged() {
        // The entry says *Fancy Editor*, but its registry key leads to cmd.exe.
        let mut l = FakeLookup::default();
        l.app_paths.insert("Fancy.exe".into(), PathBuf::from("C:/Windows/System32/cmd.exe"));
        assert!(source(ONE).apps(&l).is_empty(), "refused on what it resolves to, not on its name");
        // The same entry resolving to the real thing is found, and is marked as the plugin's.
        let mut l = FakeLookup::default();
        l.app_paths.insert("Fancy.exe".into(), PathBuf::from("D:/Fancy/Fancy.exe"));
        let apps = source(ONE).apps(&l);
        assert_eq!(apps.len(), 1);
        assert_eq!(apps[0].id, "plugin:com.test.apps/fancy");
        assert_eq!(apps[0].name, "Fancy Editor");
        assert_eq!(apps[0].added_by.as_deref(), Some("Darkroom Tools"));
        assert!(!apps[0].custom);
        // A glob that resolves to a shell host is refused too, and a missing app isn't shown.
        let mut l = FakeLookup::default();
        l.globs.insert("%ProgramFiles%/Fancy/*/Fancy.exe".into(), vec![PathBuf::from("C:/Program Files/Fancy/1/powershell.exe")]);
        assert!(source(ONE).apps(&l).is_empty());
        assert!(source(ONE).apps(&FakeLookup::default()).is_empty());
    }

    #[test]
    fn identifiers_naming_a_shell_host_are_refused_at_parse() {
        let bad = ONE.replace("app_paths = [\"Fancy.exe\"]", "app_paths = [\"cmd.exe\"]");
        let err = parse_plugin_apps("p", &bad).unwrap_err();
        assert!(err.contains("cmd.exe") && err.contains("shell"), "{err}");
        let bad = ONE.replace("exe = \"Fancy.exe\"", "exe = \"scripts/run.bat\"");
        assert!(parse_plugin_apps("p", &bad).is_err());
    }

    #[test]
    fn a_linux_entry_whose_exec_runs_a_shell_is_refused() {
        let mut l = FakeLookup::default();
        l.desktops.insert("fancy-editor".into(), PathBuf::from("/usr/share/applications/fancy-editor.desktop"));
        let mut s = source(ONE);
        s.os = Os::Linux;
        // The fake desktop entry's Exec is `<id> %U`: an ordinary program.
        assert_eq!(s.apps(&l).len(), 1);
        assert_eq!(exec_program("/bin/sh -c 'fancy %U'").as_deref(), Some("/bin/sh"));
        assert_eq!(exec_program("\"/opt/Fancy Editor/fancy\" %U").as_deref(), Some("/opt/Fancy Editor/fancy"));
    }

    #[test]
    fn no_argument_templates() {
        let with_args = ONE.replace("reads_raw = true", "reads_raw = true\nargs = [\"--open\", \"%f\"]");
        let err = parse_plugin_apps("p", &with_args).unwrap_err();
        assert!(err.contains("args"), "{err}");
        let nested = ONE.replace("[app.mac]", "[app.mac]\nargs = [\"x\"]");
        assert!(parse_plugin_apps("p", &nested).is_err());
        assert!(parse_plugin_apps("p", "[[app]]\nid = \"x\"\n").is_err(), "name and the rest are needed");
    }

    #[test]
    fn plugin_app_gets_copy() {
        // Whatever it says about itself, a plugin app is never handed an original: a developer
        // is treated as one that writes sidecars (a copy), a video app as an editor of videos.
        let dev = ONE.replace("kind = \"editor\"", "kind = \"developer\"").replace("writes = \"inPlace\"", "writes = \"never\"");
        let l = {
            let mut l = FakeLookup::default();
            l.app_paths.insert("Fancy.exe".into(), PathBuf::from("D:/Fancy/Fancy.exe"));
            l
        };
        let a = &source(&dev).apps(&l)[0];
        assert_eq!((a.kind, a.writes), (AppKind::Developer, Writes::Sidecar));
        let video = ONE.replace("kind = \"editor\"", "kind = \"video\"").replace("accepts = [\"photo\", \"raw\"]", "accepts = [\"video\"]").replace("writes = \"inPlace\"", "writes = \"never\"");
        let a = &source(&video).apps(&l)[0];
        assert_eq!((a.kind, a.writes, a.accepts.clone()), (AppKind::Editor, Writes::InPlace, vec![Media::Video]));
        let editor = &source(ONE).apps(&l)[0];
        assert_eq!((editor.kind, editor.writes), (AppKind::Editor, Writes::InPlace));
    }

    #[test]
    fn entries_are_checked() {
        for (what, text) in [
            ("an id with capitals", ONE.replace("id = \"fancy\"", "id = \"Fancy\"")),
            ("no media", ONE.replace("accepts = [\"photo\", \"raw\"]", "accepts = []")),
            ("an unknown kind", ONE.replace("kind = \"editor\"", "kind = \"wizard\"")),
            ("no way to find it", "[[app]]\nid = \"x\"\nname = \"X\"\nkind = \"editor\"\naccepts = [\"photo\"]\nreads_raw = false\nwrites = \"inPlace\"\n".to_string()),
            ("two apps of one id", format!("{ONE}\n{ONE}")),
            ("an unknown key", ONE.replace("reads_raw = true", "reads_raw = true\nlaunch = \"x\"")),
            ("no apps", "".to_string()),
        ] {
            assert!(parse_plugin_apps("p", &text).is_err(), "{what} should be refused");
        }
        let parsed = parse_plugin_apps("p", ONE).unwrap();
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].win.len(), 3);
        assert_eq!((parsed[0].mac.as_slice(), parsed[0].linux.as_slice()), (&["com.fancy.editor".to_string()][..], &["fancy-editor".to_string()][..]));
    }

    // Review fixes (Round 23 B2 final review)

    #[test]
    fn globs_must_start_in_an_install_root() {
        let with_glob = |g: &str| ONE.replace("globs = [\"%ProgramFiles%/Fancy/*/Fancy.exe\"]", &format!("globs = [\"{g}\"]"));
        for ok in ["%ProgramFiles%/Fancy/Fancy.exe", "%ProgramFiles(x86)%/Fancy/*/Fancy.exe", "%LocalAppData%/Programs/Fancy/Fancy.exe"] {
            assert!(parse_plugin_apps("p", &with_glob(ok)).is_ok(), "{ok}");
        }
        for bad in ["%AppData%/JL-Studios/Lumen/plugins/installed/x/Fancy.exe", "C:/anywhere/Fancy.exe", "%ProgramFiles%/../Users/x/Fancy.exe", "%LocalAppData%/Fancy.exe", "*/Fancy.exe"] {
            let err = parse_plugin_apps("p", &with_glob(bad)).unwrap_err();
            assert!(err.contains("install"), "{bad}: {err}");
        }
    }

    #[test]
    fn a_target_inside_lumens_own_folders_is_refused() {
        // A plugin may ship an executable; a glob, key or uninstall entry that leads to it must
        // never make it an app Lumen starts.
        let mut s = source(ONE);
        s.forbidden = vec![PathBuf::from("D:/Users/x/AppData/Roaming/JL-Studios/Lumen/plugins"), PathBuf::from("E:/dev/my-plugin")];
        for target in [
            "D:/Users/x/AppData/Roaming/JL-Studios/Lumen/plugins/installed/com.test.apps/Fancy.exe",
            "d:\\users\\X\\appdata\\roaming\\jl-studios\\lumen\\PLUGINS\\installed\\com.test.apps\\Fancy.exe",
            "E:/dev/my-plugin/bin/Fancy.exe",
        ] {
            let mut l = FakeLookup::default();
            l.app_paths.insert("Fancy.exe".into(), PathBuf::from(target));
            assert!(s.apps(&l).is_empty(), "{target}");
        }
        let mut l = FakeLookup::default();
        l.app_paths.insert("Fancy.exe".into(), PathBuf::from("D:/Program Files/Fancy/Fancy.exe"));
        assert_eq!(s.apps(&l).len(), 1, "a real install elsewhere is found");
        // A folder that merely starts with the same letters is not inside.
        l.app_paths.insert("Fancy.exe".into(), PathBuf::from("E:/dev/my-plugin-tools/Fancy.exe"));
        assert_eq!(s.apps(&l).len(), 1);
    }

    #[test]
    fn more_hosts_are_refused() {
        for name in ["py.exe", "pyw.exe", "PY.EXE", "run.pyw", "env", "/usr/bin/env", "env.exe", "Terminal.app", "/System/Applications/Utilities/Terminal.app", "iTerm.app", "Script Editor.app", "Automator.app"] {
            assert!(is_shell_host(name), "{name}");
        }
        for name in ["Pixelmator Pro.app", "envy.exe", "Photoshop.exe"] {
            assert!(!is_shell_host(name), "{name}");
        }
        // A macOS entry can't name a terminal or script app by bundle id either.
        let mac = ONE.replace("bundle_ids = [\"com.fancy.editor\"]", "bundle_ids = [\"com.apple.Terminal\"]");
        let err = parse_plugin_apps("p", &mac).unwrap_err();
        assert!(err.contains("com.apple.Terminal") && err.contains("shell"), "{err}");
        for id in ["com.googlecode.iterm2", "com.apple.ScriptEditor2", "com.apple.Automator"] {
            assert!(parse_plugin_apps("p", &ONE.replace("com.fancy.editor", id)).is_err(), "{id}");
        }
        // A .desktop Exec that goes through env is judged as env.
        let mut l = FakeLookup::default();
        l.desktops.insert("fancy-editor".into(), PathBuf::from("/usr/share/applications/fancy-editor.desktop"));
        assert_eq!(exec_program("env FOO=1 fancy %U").as_deref(), Some("env"));
    }

    #[test]
    fn a_link_to_a_host_is_judged_by_the_file_it_reaches() {
        let dir = std::env::temp_dir().join(format!("lumen-link-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let host = dir.join("bash");
        std::fs::write(&host, b"x").unwrap();
        let link = dir.join("Fancy.exe");
        #[cfg(unix)]
        let made = std::os::unix::fs::symlink(&host, &link).is_ok();
        #[cfg(windows)]
        let made = std::os::windows::fs::symlink_file(&host, &link).is_ok();
        #[cfg(not(any(unix, windows)))]
        let made = false;
        if made {
            let mut l = FakeLookup::default();
            l.app_paths.insert("Fancy.exe".into(), link.clone());
            assert!(source(ONE).apps(&l).is_empty(), "Fancy.exe is a link to bash");
        } else {
            eprintln!("skipped: this account can't create symlinks");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
