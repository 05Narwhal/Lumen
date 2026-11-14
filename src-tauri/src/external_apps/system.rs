//! The real `Lookup`: the Windows registry and known folders, macOS bundles, Linux `.desktop`
//! files. Everything only reads; nothing is launched here.

use std::path::{Path, PathBuf};

use photos_core::external_apps::{DesktopEntry, Lookup};

pub struct SystemLookup;

impl Lookup for SystemLookup {
    fn app_path(&self, exe: &str) -> Option<PathBuf> {
        windows_app_path(exe)
    }

    fn uninstall(&self) -> Vec<(String, Option<PathBuf>)> {
        windows_uninstall()
    }

    fn glob(&self, pattern: &str) -> Vec<PathBuf> {
        glob_files(&expand_vars(pattern))
    }

    fn bundle(&self, id: &str) -> Option<PathBuf> {
        mac_bundle(id)
    }

    fn desktop(&self, id: &str) -> Option<DesktopEntry> {
        let dirs = desktop_dirs();
        dirs.iter().find_map(|d| read_desktop(&d.join(format!("{id}.desktop"))))
    }

    fn exists(&self, path: &Path) -> bool {
        path.exists()
    }

    fn store_app(&self, family: &str) -> Option<PathBuf> {
        windows_store_app(family)
    }
}

/// A Store app's target: `shell:AppsFolder\<family>!<app id>` (it has no launchable file).
const STORE_PREFIX: &str = "shell:AppsFolder\\";

/// Whether `target` is a Store app's `shell:AppsFolder\…` form rather than a file.
pub fn is_store(target: &Path) -> bool {
    target.to_string_lossy().starts_with(STORE_PREFIX)
}

/// The registry key holding every Store package the user has, as `HKCU` sees it.
#[cfg(windows)]
const PACKAGES: &str = r"Software\Classes\Local Settings\Software\Microsoft\Windows\CurrentVersion\AppModel\Repository\Packages";

/// `(package key name, app id)` of the newest installed package of `family` (`Name_PublisherId`)
/// that declares file associations: package keys read `Name_<version>_<arch>__PublisherId`, and
/// the app id is the sub-key holding `Capabilities\FileAssociations`.
#[cfg(windows)]
fn find_store_package(family: &str) -> Option<(String, String)> {
    use registry::{open, Hive};
    let (name, publisher) = family.rsplit_once('_')?;
    let (head, tail) = (format!("{name}_"), format!("_{publisher}"));
    let packages = open(Hive::User, PACKAGES)?;
    let newest = packages
        .subkeys()
        .into_iter()
        .filter(|k| k.starts_with(&head) && k.ends_with(&tail))
        .max_by(|a, b| photos_core::external_apps::natural_cmp(a, b))?;
    let package = open(Hive::User, &format!(r"{PACKAGES}\{newest}"))?;
    let app = package
        .subkeys()
        .into_iter()
        .find(|app| open(Hive::User, &format!(r"{PACKAGES}\{newest}\{app}\Capabilities\FileAssociations")).is_some())?;
    Some((newest, app))
}

#[cfg(windows)]
fn windows_store_app(family: &str) -> Option<PathBuf> {
    let (_, app) = find_store_package(family)?;
    Some(PathBuf::from(format!("{STORE_PREFIX}{family}!{app}")))
}

#[cfg(not(windows))]
fn windows_store_app(_: &str) -> Option<PathBuf> {
    None
}

/// The ProgID a Store app registers for `ext` (no dot, any case), which the shell needs to open a
/// file *in that app* rather than in the default one.
#[cfg(windows)]
pub fn store_progid(target: &Path, ext: &str) -> Option<String> {
    use registry::{open, Hive};
    let text = target.to_string_lossy();
    let (family, app) = text.strip_prefix(STORE_PREFIX)?.split_once('!')?;
    let (package, _) = find_store_package(family)?;
    let key = open(Hive::User, &format!(r"{PACKAGES}\{package}\{app}\Capabilities\FileAssociations"))?;
    key.string(Some(&format!(".{}", ext.trim_start_matches('.').to_lowercase()))).filter(|p| !p.is_empty())
}

#[cfg(not(windows))]
pub fn store_progid(_: &Path, _: &str) -> Option<String> {
    None
}

// ---------- Windows registry ----------

#[cfg(windows)]
mod registry {
    use windows_sys::Win32::System::Registry::{
        RegCloseKey, RegEnumKeyExW, RegOpenKeyExW, RegQueryValueExW, HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ,
        REG_EXPAND_SZ, REG_SZ,
    };

    #[derive(Clone, Copy)]
    pub enum Hive {
        Machine,
        User,
    }

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    pub struct Key(HKEY);

    impl Drop for Key {
        fn drop(&mut self) {
            // SAFETY: the handle was opened by `open` and is closed once.
            unsafe { RegCloseKey(self.0) };
        }
    }

    pub fn open(hive: Hive, path: &str) -> Option<Key> {
        let root = match hive {
            Hive::Machine => HKEY_LOCAL_MACHINE,
            Hive::User => HKEY_CURRENT_USER,
        };
        let mut out: HKEY = std::ptr::null_mut();
        // SAFETY: `path` is NUL-terminated and `out` is a valid out-pointer.
        let status = unsafe { RegOpenKeyExW(root, wide(path).as_ptr(), 0, KEY_READ, &mut out) };
        (status == 0).then_some(Key(out))
    }

    impl Key {
        /// A string value (`None` name = the default value).
        pub fn string(&self, name: Option<&str>) -> Option<String> {
            let wname = wide(name.unwrap_or(""));
            let mut kind = 0u32;
            let mut size = 0u32;
            // SAFETY: a size query with no buffer.
            let status = unsafe { RegQueryValueExW(self.0, wname.as_ptr(), std::ptr::null(), &mut kind, std::ptr::null_mut(), &mut size) };
            if status != 0 || !(kind == REG_SZ || kind == REG_EXPAND_SZ) || size == 0 {
                return None;
            }
            let mut buf = vec![0u16; (size as usize).div_ceil(2) + 1];
            let mut got = (buf.len() * 2) as u32;
            // SAFETY: `buf` holds `got` bytes.
            let status = unsafe { RegQueryValueExW(self.0, wname.as_ptr(), std::ptr::null(), &mut kind, buf.as_mut_ptr() as *mut u8, &mut got) };
            if status != 0 {
                return None;
            }
            let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
            Some(String::from_utf16_lossy(&buf[..len]))
        }

        pub fn subkeys(&self) -> Vec<String> {
            let mut names = Vec::new();
            let mut index = 0u32;
            loop {
                let mut buf = [0u16; 512];
                let mut len = buf.len() as u32;
                // SAFETY: `buf` holds `len` UTF-16 units.
                let status = unsafe {
                    RegEnumKeyExW(self.0, index, buf.as_mut_ptr(), &mut len, std::ptr::null(), std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null_mut())
                };
                if status != 0 {
                    break;
                }
                names.push(String::from_utf16_lossy(&buf[..len as usize]));
                index += 1;
            }
            names
        }
    }
}

/// `App Paths\<exe>`: HKLM, its 32-bit view, then HKCU; a quoted value is unquoted; the file must
/// exist.
#[cfg(windows)]
fn windows_app_path(exe: &str) -> Option<PathBuf> {
    use registry::{open, Hive};
    let base = r"SOFTWARE\Microsoft\Windows\CurrentVersion\App Paths";
    let wow = r"SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\App Paths";
    [(Hive::Machine, base), (Hive::Machine, wow), (Hive::User, base)].into_iter().find_map(|(hive, root)| {
        let value = open(hive, &format!(r"{root}\{exe}"))?.string(None)?;
        let path = PathBuf::from(unquote(&expand_vars(&value)));
        path.is_file().then_some(path)
    })
}

#[cfg(not(windows))]
fn windows_app_path(_: &str) -> Option<PathBuf> {
    None
}

/// Add/Remove Programs in HKLM, its 32-bit view and HKCU: (DisplayName, InstallLocation).
#[cfg(windows)]
fn windows_uninstall() -> Vec<(String, Option<PathBuf>)> {
    use registry::{open, Hive};
    let base = r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall";
    let wow = r"SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall";
    let mut out = Vec::new();
    for (hive, root) in [(Hive::Machine, base), (Hive::Machine, wow), (Hive::User, base)] {
        let Some(key) = open(hive, root) else { continue };
        for sub in key.subkeys() {
            let Some(entry) = open(hive, &format!(r"{root}\{sub}")) else { continue };
            let Some(name) = entry.string(Some("DisplayName")).filter(|n| !n.is_empty()) else { continue };
            let location = entry
                .string(Some("InstallLocation"))
                .map(|l| unquote(&expand_vars(&l)).to_string())
                .filter(|l| !l.is_empty())
                .map(PathBuf::from);
            out.push((name, location));
        }
    }
    out
}

#[cfg(not(windows))]
fn windows_uninstall() -> Vec<(String, Option<PathBuf>)> {
    Vec::new()
}

fn unquote(s: &str) -> &str {
    s.trim().trim_matches('"')
}

/// `%Name%` replaced by the environment variable (a missing one keeps its text).
pub(crate) fn expand_vars(text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(start) = rest.find('%') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        match after.find('%') {
            Some(end) if end > 0 => match std::env::var(&after[..end]) {
                Ok(v) => {
                    out.push_str(&v);
                    rest = &after[end + 1..];
                }
                Err(_) => {
                    out.push('%');
                    rest = after;
                }
            },
            _ => {
                out.push('%');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

// ---------- globs ----------

/// `*` matches any run, `?` one character; case-insensitive (file names on Windows and macOS).
pub(crate) fn wildcard(pattern: &str, name: &str) -> bool {
    fn go(p: &[char], n: &[char]) -> bool {
        match p.split_first() {
            None => n.is_empty(),
            Some(('*', rest)) => (0..=n.len()).any(|i| go(rest, &n[i..])),
            Some(('?', rest)) => !n.is_empty() && go(rest, &n[1..]),
            Some((c, rest)) => n.first().is_some_and(|x| x.to_lowercase().eq(c.to_lowercase())) && go(rest, &n[1..]),
        }
    }
    let p: Vec<char> = pattern.chars().collect();
    let n: Vec<char> = name.chars().collect();
    go(&p, &n)
}

/// Files matching a path whose segments may hold `*` and `?`; the folders on the way are read, never
/// guessed at. A pattern with no wildcard is a plain existence check.
pub(crate) fn glob_files(pattern: &str) -> Vec<PathBuf> {
    let segments: Vec<&str> = pattern.split(['/', '\\']).filter(|s| !s.is_empty()).collect();
    let Some((first, rest)) = segments.split_first() else { return Vec::new() };
    // A drive (`C:`) or an absolute Unix path starts the walk.
    let root = if first.ends_with(':') { PathBuf::from(format!("{first}/")) } else if pattern.starts_with(['/', '\\']) { PathBuf::from("/").join(first) } else { PathBuf::from(first) };
    let mut level = vec![root];
    for (i, seg) in rest.iter().enumerate() {
        let last = i + 1 == rest.len();
        let mut next = Vec::new();
        for dir in &level {
            if seg.contains(['*', '?']) {
                let Ok(read) = std::fs::read_dir(dir) else { continue };
                for entry in read.flatten() {
                    if wildcard(seg, &entry.file_name().to_string_lossy()) {
                        next.push(entry.path());
                    }
                }
            } else {
                next.push(dir.join(seg));
            }
        }
        level = next.into_iter().filter(|p| if last { p.is_file() } else { p.is_dir() }).collect();
        if level.is_empty() {
            break;
        }
    }
    if rest.is_empty() {
        level.retain(|p| p.is_file());
    }
    level
}

// ---------- macOS bundles ----------

/// `mdfind` first (Spotlight knows every bundle, wherever it lives), then the usual folders.
fn mac_bundle(id: &str) -> Option<PathBuf> {
    if !cfg!(target_os = "macos") {
        return None;
    }
    if id.contains(['\'', '"', '\\']) {
        return None;
    }
    let found = std::process::Command::new("mdfind")
        .arg(format!("kMDItemCFBundleIdentifier == '{id}'"))
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).lines().map(PathBuf::from).collect::<Vec<_>>())
        .unwrap_or_default();
    if let Some(p) = found.into_iter().find(|p| p.extension().is_some_and(|e| e == "app") && p.is_dir()) {
        return Some(p);
    }
    let home = dirs::home_dir();
    let roots = [PathBuf::from("/Applications"), PathBuf::from("/System/Applications")].into_iter().chain(home.map(|h| h.join("Applications")));
    roots.into_iter().find_map(|root| {
        std::fs::read_dir(root).ok()?.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|e| e == "app")).find(|app| plist_bundle_id(&app.join("Contents/Info.plist")).as_deref() == Some(id))
    })
}

/// `CFBundleIdentifier` from an XML `Info.plist` (a binary one is skipped; `mdfind` covers it).
fn plist_bundle_id(path: &Path) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    let after = &text[text.find("<key>CFBundleIdentifier</key>")? + "<key>CFBundleIdentifier</key>".len()..];
    let start = after.find("<string>")? + "<string>".len();
    let end = after[start..].find("</string>")?;
    Some(after[start..start + end].trim().to_string())
}

// ---------- Linux .desktop files ----------

/// The data dirs' `applications` folders, the Flatpak exports and Snap's.
fn desktop_dirs() -> Vec<PathBuf> {
    let home = dirs::home_dir();
    let mut dirs: Vec<PathBuf> = Vec::new();
    let data_home = std::env::var_os("XDG_DATA_HOME").map(PathBuf::from).filter(|p| p.is_absolute()).or_else(|| home.as_ref().map(|h| h.join(".local/share")));
    dirs.extend(data_home.map(|d| d.join("applications")));
    let data_dirs = std::env::var("XDG_DATA_DIRS").ok().filter(|v| !v.is_empty()).unwrap_or_else(|| "/usr/local/share:/usr/share".to_string());
    dirs.extend(data_dirs.split(':').filter(|d| !d.is_empty()).map(|d| Path::new(d).join("applications")));
    dirs.push(PathBuf::from("/var/lib/flatpak/exports/share/applications"));
    dirs.extend(home.map(|h| h.join(".local/share/flatpak/exports/share/applications")));
    dirs.push(PathBuf::from("/var/lib/snapd/desktop/applications"));
    dirs
}

fn read_desktop(path: &Path) -> Option<DesktopEntry> {
    let text = std::fs::read_to_string(path).ok()?;
    let (exec, icon) = parse_desktop(&text)?;
    Some(DesktopEntry { exec, icon, path: path.to_path_buf() })
}

/// `Exec` and `Icon` from the `[Desktop Entry]` group; an entry without `Exec`, or a hidden one, is
/// not an app.
pub(crate) fn parse_desktop(text: &str) -> Option<(String, Option<String>)> {
    let mut in_entry = false;
    let (mut exec, mut icon) = (None, None);
    for line in text.lines().map(str::trim) {
        if line.starts_with('[') {
            in_entry = line == "[Desktop Entry]";
        } else if in_entry {
            if let Some(v) = line.strip_prefix("Exec=") {
                exec.get_or_insert(v.trim().to_string());
            } else if let Some(v) = line.strip_prefix("Icon=") {
                icon.get_or_insert(v.trim().to_string());
            } else if line == "Hidden=true" {
                return None;
            }
        }
    }
    exec.filter(|e| !e.is_empty()).map(|e| (e, icon.filter(|i| !i.is_empty())))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wildcards_match_case_insensitively() {
        assert!(wildcard("GIMP *", "GIMP 2.10"));
        assert!(wildcard("gimp-?.*.exe", "GIMP-2.10.exe"));
        assert!(!wildcard("GIMP *", "Krita"));
        assert!(wildcard("*", ""));
    }

    #[test]
    fn a_glob_walks_real_folders_and_only_returns_files() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join("GIMP 2/bin")).unwrap();
        std::fs::create_dir_all(root.join("GIMP 3/bin")).unwrap();
        std::fs::create_dir_all(root.join("Krita/bin")).unwrap();
        std::fs::write(root.join("GIMP 2/bin/gimp-2.10.exe"), b"x").unwrap();
        std::fs::write(root.join("GIMP 3/bin/gimp-3.0.exe"), b"x").unwrap();
        // A folder that matches the last segment is not a file.
        std::fs::create_dir_all(root.join("GIMP 3/bin/gimp-dir.exe")).unwrap();
        let pattern = format!("{}/GIMP */bin/gimp-*.exe", root.to_string_lossy().replace('\\', "/"));
        let mut found = glob_files(&pattern);
        found.sort();
        assert_eq!(found.len(), 2, "{found:?}");
        assert!(found[0].ends_with("gimp-2.10.exe") && found[1].ends_with("gimp-3.0.exe"));
        assert!(glob_files(&format!("{}/Nothing */bin/x.exe", root.to_string_lossy())).is_empty());
    }

    #[test]
    fn variables_expand_and_unknown_ones_stay() {
        std::env::set_var("LUMEN_TEST_PF", "C:/Program Files");
        assert_eq!(expand_vars("%LUMEN_TEST_PF%/GIMP"), "C:/Program Files/GIMP");
        assert_eq!(expand_vars("%LUMEN_NOT_SET_X%/a"), "%LUMEN_NOT_SET_X%/a");
        assert_eq!(expand_vars("100% sure"), "100% sure");
    }

    #[test]
    fn a_desktop_file_gives_its_exec_and_icon() {
        let text = "[Desktop Entry]\nName=GIMP\nExec=gimp-2.10 %U\nIcon=gimp\n[Desktop Action new]\nExec=other\n";
        assert_eq!(parse_desktop(text), Some(("gimp-2.10 %U".to_string(), Some("gimp".to_string()))));
        assert_eq!(parse_desktop("[Desktop Entry]\nName=x\n"), None);
    }

    #[test]
    fn a_bundle_id_is_read_from_an_xml_plist() {
        let dir = tempfile::tempdir().unwrap();
        let plist = dir.path().join("Info.plist");
        std::fs::write(&plist, "<plist><dict><key>CFBundleName</key><string>Krita</string><key>CFBundleIdentifier</key>\n<string>org.krita</string></dict></plist>").unwrap();
        assert_eq!(plist_bundle_id(&plist).as_deref(), Some("org.krita"));
    }

    #[cfg(windows)]
    #[test]
    fn the_registry_answers_for_a_missing_and_a_present_app() {
        assert_eq!(windows_app_path("lumen-no-such-app-xyz.exe"), None);
        // What is installed varies by machine: only the call's safety is asserted.
        let _ = windows_uninstall();
    }
}
