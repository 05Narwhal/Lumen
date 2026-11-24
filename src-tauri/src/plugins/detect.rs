//! App detection for plugins (Round 24 §8.2, lane D3): finds the program a `[[detect]]` entry
//! names, in the order *Locate…* choice, registry (Windows), declared folders, `PATH`, and reads
//! its version. Providers launch only what this resolves.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

use photos_plugins::manifest::Detect;
use photos_plugins::{DetectHost, DetectStatus};

use super::Plugins;

/// How long the declared `version` command may run.
pub const VERSION_TIMEOUT: Duration = Duration::from_secs(3);

/// The executable's file name this machine's entry declares (a `.app` name on macOS).
fn declared_name(detect: &Detect) -> Option<&str> {
    if cfg!(windows) {
        detect.windows.as_ref().map(|w| w.exe.as_str())
    } else if cfg!(target_os = "macos") {
        detect.macos.as_ref().map(|m| m.app.as_str())
    } else {
        detect.linux.as_ref().map(|u| u.exe.as_str())
    }
}

/// `%NAME%` → the variable's value (an unset one is left as written).
pub fn expand_env(s: &str) -> String {
    let mut out = String::new();
    let mut rest = s;
    while let Some(open) = rest.find('%') {
        let Some(close) = rest[open + 1..].find('%') else { break };
        let name = &rest[open + 1..open + 1 + close];
        out.push_str(&rest[..open]);
        match std::env::var(name) {
            Ok(v) if !name.is_empty() => out.push_str(&v),
            _ => out.push_str(&rest[open..open + close + 2]),
        }
        rest = &rest[open + close + 2..];
    }
    out.push_str(rest);
    out
}

/// The declared program found in the declared folders, then on `PATH` (no registry, no choice).
pub fn find_declared(detect: &Detect) -> Option<PathBuf> {
    let (exe, paths, path_env): (&str, &[String], bool) = if cfg!(windows) {
        let w = detect.windows.as_ref()?;
        (&w.exe, &w.paths, w.path_env)
    } else if cfg!(target_os = "macos") {
        let m = detect.macos.as_ref()?;
        let mut dirs: Vec<PathBuf> = m.paths.iter().map(|p| PathBuf::from(expand_env(p))).collect();
        dirs.push(PathBuf::from("/Applications"));
        if let Some(home) = std::env::var_os("HOME") {
            dirs.push(PathBuf::from(home).join("Applications"));
        }
        return dirs.into_iter().map(|d| d.join(&m.app)).find(|p| p.is_dir());
    } else {
        let u = detect.linux.as_ref()?;
        (&u.exe, &u.paths, u.path_env)
    };
    let mut dirs: Vec<PathBuf> = paths.iter().map(|p| PathBuf::from(expand_env(p))).collect();
    if path_env {
        if let Some(path) = std::env::var_os("PATH") {
            dirs.extend(std::env::split_paths(&path));
        }
    }
    dirs.into_iter().map(|d| d.join(exe)).find(|p| p.is_file())
}

/// The program's default value in each declared `HKCU/…` / `HKLM/…` key (App Paths), when it
/// is a file. Always `None` off Windows.
pub fn registry_app_paths(keys: &[String]) -> Option<PathBuf> {
    #[cfg(windows)]
    {
        keys.iter().find_map(|k| registry::default_value(k)).map(|v| PathBuf::from(expand_env(v.trim_matches('"')))).filter(|p| p.is_file())
    }
    #[cfg(not(windows))]
    {
        let _ = keys;
        None
    }
}

/// A chosen program is acceptable when it exists and is named like the declared one.
pub fn located_exe_must_match_name(detect: &Detect, exe: &Path) -> Result<(), String> {
    let want = declared_name(detect).ok_or_else(|| format!("{} isn't offered for this system.", detect.name))?;
    let exists = if cfg!(target_os = "macos") { exe.is_dir() } else { exe.is_file() };
    if !exists {
        return Err("That file doesn't exist.".into());
    }
    let got = exe.file_name().and_then(|n| n.to_str()).unwrap_or_default();
    let same = if cfg!(windows) { got.eq_ignore_ascii_case(want) } else { got == want };
    if same {
        Ok(())
    } else {
        Err(format!("Pick {want}."))
    }
}

/// The first line of the program's stdout for the declared version arguments; `None` when it
/// fails, prints nothing or runs past `timeout` (then it is killed).
pub fn version_first_line(exe: &Path, args: &[String], timeout: Duration) -> Option<String> {
    let mut child = Command::new(exe)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if start.elapsed() < timeout => std::thread::sleep(Duration::from_millis(15)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
    let mut out = String::new();
    child.stdout.take()?.take(64 * 1024).read_to_string(&mut out).ok()?;
    let line = out.lines().next()?.trim();
    (!line.is_empty()).then(|| line.chars().take(120).collect())
}

/// The program to launch for a found path: a macOS bundle's inner executable, else the path.
pub fn launchable(found: &Path) -> PathBuf {
    if found.extension().is_some_and(|e| e == "app") && found.is_dir() {
        let stem = found.file_stem().and_then(|s| s.to_str()).unwrap_or_default();
        let inner = found.join("Contents").join("MacOS").join(stem);
        if inner.is_file() {
            return inner;
        }
    }
    found.to_path_buf()
}

/// Where the program is: the person's choice if it is still valid, else registry, folders, `PATH`.
pub fn find(detect: &Detect, located: Option<&Path>) -> Option<(PathBuf, bool)> {
    if let Some(p) = located.filter(|p| located_exe_must_match_name(detect, p).is_ok()) {
        return Some((p.to_path_buf(), true));
    }
    let keys = detect.windows.as_ref().map(|w| w.registry.as_slice()).unwrap_or_default();
    registry_app_paths(keys).or_else(|| find_declared(detect)).map(|p| (p, false))
}

/// One `[[detect]]` entry resolved on this machine.
pub fn resolve(detect: &Detect, located: Option<&Path>) -> DetectStatus {
    let found = find(detect, located);
    let version = found.as_ref().and_then(|(p, _)| {
        let args = detect.version.as_deref()?;
        version_first_line(&launchable(p), args, VERSION_TIMEOUT)
    });
    DetectStatus {
        id: detect.id.clone(),
        name: detect.name.clone(),
        path: found.as_ref().map(|(p, _)| p.to_string_lossy().into_owned()),
        version,
        located: found.as_ref().is_some_and(|(_, l)| *l),
    }
}

/// The binary's [`DetectHost`]: the manifest's entries, the person's *Locate…* choices.
pub struct PluginDetect {
    plugins: Arc<Plugins>,
}

impl PluginDetect {
    pub fn new(plugins: Arc<Plugins>) -> Self {
        PluginDetect { plugins }
    }

    fn entry(&self, plugin: &str, detect_id: &str) -> Option<(Detect, Option<PathBuf>)> {
        let store = self.plugins.read();
        let inst = store.get(plugin)?;
        let d = inst.manifest.detect.iter().find(|d| d.id == detect_id)?.clone();
        Some((d, store.located(plugin, detect_id)))
    }

    /// The program a provider of `plugin` should launch for `detect`, to be launched as is.
    pub fn exe(&self, plugin: &str, detect: &Detect) -> Option<PathBuf> {
        let located = self.plugins.read().located(plugin, &detect.id);
        find(detect, located.as_deref()).map(|(p, _)| launchable(&p))
    }
}

impl DetectHost for PluginDetect {
    fn detect(&self, plugin: &str) -> Vec<DetectStatus> {
        let entries: Vec<(Detect, Option<PathBuf>)> = {
            let store = self.plugins.read();
            let Some(inst) = store.get(plugin) else { return Vec::new() };
            inst.manifest.detect.iter().map(|d| (d.clone(), store.located(plugin, &d.id))).collect()
        };
        // Run outside the lock: the version command may take its 3 s.
        entries.iter().map(|(d, l)| resolve(d, l.as_deref())).collect()
    }

    fn locate(&self, plugin: &str, detect_id: &str, exe: &Path) -> Result<DetectStatus, String> {
        let (detect, _) = self.entry(plugin, detect_id).ok_or_else(|| format!("{plugin} has no app {detect_id}."))?;
        located_exe_must_match_name(&detect, exe)?;
        self.plugins.write().set_located(plugin, detect_id, exe).map_err(|e| e.to_string())?;
        Ok(resolve(&detect, Some(exe)))
    }
}

#[cfg(windows)]
mod registry {
    use windows_sys::Win32::System::Registry::*;

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(Some(0)).collect()
    }

    fn hive(name: &str) -> Option<HKEY> {
        match name {
            "HKCU" => Some(HKEY_CURRENT_USER),
            "HKLM" => Some(HKEY_LOCAL_MACHINE),
            _ => None,
        }
    }

    /// The default value of `HKCU/a/b` (`/` or `\` separated) when it is a string.
    pub fn default_value(key: &str) -> Option<String> {
        let (root, sub) = key.split_once(['/', '\\'])?;
        let sub = sub.replace('/', "\\");
        let mut h: HKEY = std::ptr::null_mut();
        // SAFETY: plain registry reads into buffers this function owns.
        unsafe {
            if RegOpenKeyExW(hive(root)?, wide(&sub).as_ptr(), 0, KEY_READ, &mut h) != 0 {
                return None;
            }
            let mut len: u32 = 0;
            let ok = RegQueryValueExW(h, std::ptr::null(), std::ptr::null(), std::ptr::null_mut(), std::ptr::null_mut(), &mut len) == 0;
            let mut buf = vec![0u16; (len as usize / 2) + 1];
            let mut size = (buf.len() * 2) as u32;
            let mut kind = 0u32;
            let got = ok
                && RegQueryValueExW(h, std::ptr::null(), std::ptr::null(), &mut kind, buf.as_mut_ptr() as *mut u8, &mut size) == 0
                && (kind == REG_SZ || kind == REG_EXPAND_SZ);
            RegCloseKey(h);
            if !got {
                return None;
            }
            let n = (size as usize / 2).min(buf.len());
            let s = String::from_utf16_lossy(&buf[..n]);
            Some(s.trim_end_matches('\0').to_string())
        }
    }

    /// Test helper: sets (creating) the default value of an HKCU key.
    #[cfg(test)]
    pub fn set_default(sub: &str, value: &str) {
        let mut h: HKEY = std::ptr::null_mut();
        // SAFETY: creates a key under HKCU the test deletes afterwards.
        unsafe {
            assert_eq!(RegCreateKeyExW(HKEY_CURRENT_USER, wide(sub).as_ptr(), 0, std::ptr::null(), 0, KEY_WRITE, std::ptr::null(), &mut h, std::ptr::null_mut()), 0);
            let v = wide(value);
            assert_eq!(RegSetValueExW(h, std::ptr::null(), 0, REG_SZ, v.as_ptr() as *const u8, (v.len() * 2) as u32), 0);
            RegCloseKey(h);
        }
    }

    #[cfg(test)]
    pub fn delete(sub: &str) {
        // SAFETY: removes the key `set_default` made.
        unsafe {
            RegDeleteKeyW(HKEY_CURRENT_USER, wide(sub).as_ptr());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use photos_plugins::manifest::{DetectMac, DetectUnix, DetectWin};

    /// A `Detect` declaring `exe` in `dirs` (and `PATH` when asked) for the system under test.
    fn detect(exe: &str, dirs: Vec<String>, path_env: bool, registry: Vec<String>) -> Detect {
        Detect {
            id: "fx".into(),
            name: "Fx".into(),
            windows: Some(DetectWin { exe: exe.into(), paths: dirs.clone(), path_env, registry }),
            linux: Some(DetectUnix { exe: exe.into(), paths: dirs.clone(), path_env }),
            macos: Some(DetectMac { app: exe.into(), paths: dirs }),
            version: None,
        }
    }

    /// A stand-in program printing `out` (and a second line) for any arguments; sleeps first.
    fn fixture(dir: &Path, name: &str, out: &str, sleep_secs: u32) -> PathBuf {
        #[cfg(windows)]
        {
            let p = dir.join(format!("{name}.cmd"));
            let mut s = String::from("@echo off\r\n");
            if sleep_secs > 0 {
                s += &format!("ping -n {} 127.0.0.1 >nul\r\n", sleep_secs + 1);
            }
            s += &format!("echo {out}\r\necho second line\r\n");
            std::fs::write(&p, s).unwrap();
            p
        }
        #[cfg(not(windows))]
        {
            use std::os::unix::fs::PermissionsExt;
            let p = dir.join(name);
            let mut s = String::from("#!/bin/sh\n");
            if sleep_secs > 0 {
                s += &format!("sleep {sleep_secs}\n");
            }
            s += &format!("echo '{out}'\necho 'second line'\n");
            std::fs::write(&p, s).unwrap();
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
            p
        }
    }

    #[test]
    fn expands_env_paths() {
        std::env::set_var("LUMEN_D3_TEST_DIR", "/opt/atlas");
        assert_eq!(expand_env("%LUMEN_D3_TEST_DIR%/bin"), "/opt/atlas/bin");
        assert_eq!(expand_env("%LUMEN_D3_UNSET%/bin"), "%LUMEN_D3_UNSET%/bin");
        assert_eq!(expand_env("no vars, 100% sure"), "no vars, 100% sure");
    }

    #[test]
    fn finds_on_path() {
        if cfg!(target_os = "macos") {
            return; // a bundle is found by folder, not PATH
        }
        let dir = tempfile::tempdir().unwrap();
        let exe = fixture(dir.path(), "d3-on-path", "x", 0);
        let name = exe.file_name().unwrap().to_str().unwrap();
        let old = std::env::var_os("PATH").unwrap_or_default();
        let mut parts: Vec<PathBuf> = std::env::split_paths(&old).collect();
        parts.insert(0, dir.path().to_path_buf());
        std::env::set_var("PATH", std::env::join_paths(parts).unwrap());
        let found = find_declared(&detect(name, vec![], true, vec![]));
        let off = find_declared(&detect(name, vec![], false, vec![]));
        std::env::set_var("PATH", old);
        assert_eq!(found.as_deref(), Some(exe.as_path()));
        assert_eq!(off, None);
    }

    #[test]
    fn located_choice_wins_and_must_match_name() {
        if cfg!(target_os = "macos") {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let declared = fixture(dir.path(), "d3-declared", "x", 0);
        let name = declared.file_name().unwrap().to_str().unwrap().to_string();
        let d = detect(&name, vec![], false, vec![]);
        assert_eq!(located_exe_must_match_name(&d, &declared), Ok(()));
        // Another file name is refused, as is a file that isn't there.
        let other = fixture(dir.path(), "d3-other", "x", 0);
        assert!(located_exe_must_match_name(&d, &other).is_err());
        assert!(located_exe_must_match_name(&d, &dir.path().join(&name).with_extension("gone")).is_err());
        // The same name in another folder is the person's choice and beats the declared folder.
        let alt = tempfile::tempdir().unwrap();
        let chosen = fixture(alt.path(), "d3-declared", "x", 0);
        let d = detect(&name, vec![dir.path().to_string_lossy().into_owned()], false, vec![]);
        assert_eq!(find(&d, None), Some((declared.clone(), false)));
        assert_eq!(find(&d, Some(&chosen)), Some((chosen.clone(), true)));
        // A choice that has gone away falls back to the declared folders.
        std::fs::remove_file(&chosen).unwrap();
        assert_eq!(find(&d, Some(&chosen)), Some((declared, false)));
    }

    #[test]
    fn version_first_line_reads_one_line_and_times_out() {
        let dir = tempfile::tempdir().unwrap();
        let exe = fixture(dir.path(), "d3-version", "Fx 0.4.2", 0);
        assert_eq!(version_first_line(&exe, &["--version".into()], Duration::from_secs(10)), Some("Fx 0.4.2".into()));
        let slow = fixture(dir.path(), "d3-slow", "late", 5);
        let t = Instant::now();
        assert_eq!(version_first_line(&slow, &[], Duration::from_millis(300)), None);
        assert!(t.elapsed() < Duration::from_secs(4), "the slow program was killed, not awaited");
        assert_eq!(version_first_line(&dir.path().join("missing"), &[], Duration::from_secs(1)), None);
    }

    #[test]
    fn resolve_reports_found_version_and_not_found() {
        if cfg!(target_os = "macos") {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let exe = fixture(dir.path(), "d3-resolve", "Fx 1.2.3", 0);
        let name = exe.file_name().unwrap().to_str().unwrap();
        let mut d = detect(name, vec![dir.path().to_string_lossy().into_owned()], false, vec![]);
        d.version = Some(vec!["--version".into()]);
        let s = resolve(&d, None);
        assert_eq!((s.id.as_str(), s.version.as_deref(), s.located), ("fx", Some("Fx 1.2.3"), false));
        assert_eq!(s.path.as_deref(), Some(exe.to_string_lossy().as_ref()));
        let none = resolve(&detect("not-installed-anywhere.exe", vec![], false, vec![]), None);
        assert_eq!((none.path, none.version), (None, None));
    }

    #[cfg(windows)]
    #[test]
    fn registry_app_paths_reads_the_default_value() {
        let dir = tempfile::tempdir().unwrap();
        let exe = fixture(dir.path(), "d3-registry", "x", 0);
        let sub = "Software\\LumenD3Test\\App Paths\\d3.exe";
        registry::set_default(sub, &format!("\"{}\"", exe.display()));
        let key = "HKCU/Software/LumenD3Test/App Paths/d3.exe".to_string();
        let found = registry_app_paths(std::slice::from_ref(&key));
        let via_find = find(&detect("d3-registry.cmd", vec![], false, vec![key]), None);
        let missing = registry_app_paths(&["HKCU/Software/LumenD3Test/Nope".to_string()]);
        registry::delete(sub);
        registry::delete("Software\\LumenD3Test\\App Paths");
        registry::delete("Software\\LumenD3Test");
        assert_eq!(found.as_deref(), Some(exe.as_path()));
        assert_eq!(via_find, Some((exe, false)));
        assert_eq!(missing, None);
    }
}
