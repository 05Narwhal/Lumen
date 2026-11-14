//! Finding an installed app from a profile, per OS, through a `Lookup`; and filling a `.desktop`
//! `Exec` line's field codes.

use std::cmp::Ordering;
use std::path::{Path, PathBuf};

use super::{AppProfile, AppSource, InstalledApp, Lookup, WinRule};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Os {
    Windows,
    Mac,
    Linux,
}

impl Os {
    pub fn current() -> Os {
        if cfg!(windows) {
            Os::Windows
        } else if cfg!(target_os = "macos") {
            Os::Mac
        } else {
            Os::Linux
        }
    }
}

/// Where a profile is installed on this OS: the target and a `.desktop` file's `Exec`.
pub fn resolve_profile(p: &AppProfile<'_>, os: Os, lookup: &dyn Lookup) -> Option<(PathBuf, Option<String>)> {
    match os {
        Os::Windows => p.win.iter().find_map(|rule| resolve_rule(rule, lookup)).map(|t| (t, None)),
        Os::Mac => p.mac.iter().find_map(|id| lookup.bundle(id)).map(|t| (t, None)),
        Os::Linux => p.linux.iter().find_map(|id| lookup.desktop(id)).map(|d| (d.path, Some(d.exec))),
    }
}

fn resolve_rule(rule: &WinRule<'_>, lookup: &dyn Lookup) -> Option<PathBuf> {
    match rule {
        WinRule::AppPath(exe) => lookup.app_path(exe),
        WinRule::Uninstall { name_starts, exe } => {
            let wanted = name_starts.to_lowercase();
            lookup.uninstall().into_iter().find_map(|(name, location)| {
                if !name.to_lowercase().starts_with(&wanted) {
                    return None;
                }
                // The file decides: *Adobe Photoshop Elements 2025* starts like *Adobe Photoshop*
                // but has no `Photoshop.exe` in its folder.
                let candidate = location?.join(exe);
                lookup.exists(&candidate).then_some(candidate)
            })
        }
        WinRule::Glob(pattern) => lookup.glob(pattern).into_iter().max_by(|a, b| natural_cmp(&a.to_string_lossy(), &b.to_string_lossy())),
        WinRule::Store { family } => lookup.store_app(family),
    }
}

fn digits(it: &mut std::iter::Peekable<std::str::Chars<'_>>) -> String {
    let mut s = String::new();
    while let Some(c) = it.peek().copied().filter(char::is_ascii_digit) {
        s.push(c);
        it.next();
    }
    s.trim_start_matches('0').to_string()
}

/// Compares text with digit runs as numbers, so `PhotoLab 10` sorts after `PhotoLab 9`.
pub fn natural_cmp(a: &str, b: &str) -> Ordering {
    let (mut a, mut b) = (a.chars().peekable(), b.chars().peekable());
    loop {
        match (a.peek().copied(), b.peek().copied()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(x), Some(y)) if x.is_ascii_digit() && y.is_ascii_digit() => {
                let (ra, rb) = (digits(&mut a), digits(&mut b));
                let ord = ra.len().cmp(&rb.len()).then_with(|| ra.cmp(&rb));
                if ord != Ordering::Equal {
                    return ord;
                }
            }
            (Some(x), Some(y)) => {
                let ord = x.to_lowercase().cmp(y.to_lowercase());
                if ord != Ordering::Equal {
                    return ord;
                }
                a.next();
                b.next();
            }
        }
    }
}

impl AppSource for super::BuiltIn {
    fn apps(&self, lookup: &dyn Lookup) -> Vec<InstalledApp> {
        self.profiles
            .iter()
            .filter_map(|p| {
                let (target, exec) = resolve_profile(p, self.os, lookup)?;
                Some(InstalledApp {
                    id: p.id.to_string(),
                    name: p.name.to_string(),
                    kind: p.kind,
                    accepts: p.accepts.to_vec(),
                    reads_raw: p.reads_raw,
                    writes: p.writes,
                    target,
                    exec,
                    custom: false,
                    added_by: None,
                })
            })
            .collect()
    }
}

/// A `.desktop` `Exec` line with the files in it: one command line per launch. `%f`/`%u` take one
/// file, so several files make several launches; `%F`/`%U` take them all; no code appends them.
/// `%i`, `%c`, `%k` and the deprecated codes are dropped, `%%` is a percent sign.
pub fn exec_launches(exec: &str, files: &[PathBuf]) -> Vec<Vec<String>> {
    let tokens = tokenize(exec);
    let is_single = |t: &str| matches!(t, "%f" | "%u");
    let is_multi = |t: &str| matches!(t, "%F" | "%U");
    let has_code = tokens.iter().any(|t| is_single(t) || is_multi(t));
    let single = tokens.iter().any(|t| is_single(t));
    let path_text = |p: &Path| p.to_string_lossy().into_owned();

    let build = |chunk: &[PathBuf]| -> Vec<String> {
        let mut out = Vec::new();
        for t in &tokens {
            if is_single(t) || is_multi(t) {
                out.extend(chunk.iter().map(|p| path_text(p)));
            } else if matches!(t.as_str(), "%i" | "%c" | "%k" | "%d" | "%D" | "%n" | "%N" | "%v" | "%m") {
                // dropped
            } else {
                out.push(t.replace("%%", "%"));
            }
        }
        if !has_code {
            out.extend(chunk.iter().map(|p| path_text(p)));
        }
        out
    };

    if single && files.len() > 1 {
        files.iter().map(|f| build(std::slice::from_ref(f))).collect()
    } else {
        vec![build(files)]
    }
}

/// The program a `.desktop` `Exec` line starts (its first word, unquoted).
pub fn exec_program(exec: &str) -> Option<String> {
    tokenize(exec).into_iter().next()
}

/// Splits on spaces, keeping double-quoted text together (the desktop entry quoting rules).
fn tokenize(exec: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut in_quote = false;
    let mut chars = exec.chars().peekable();
    let mut started = false;
    while let Some(c) = chars.next() {
        match c {
            '"' => {
                in_quote = !in_quote;
                started = true;
            }
            '\\' if in_quote => {
                if let Some(n) = chars.next() {
                    cur.push(n);
                }
            }
            c if c.is_whitespace() && !in_quote => {
                if started {
                    out.push(std::mem::take(&mut cur));
                    started = false;
                }
            }
            c => {
                cur.push(c);
                started = true;
            }
        }
    }
    if started {
        out.push(cur);
    }
    out
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::super::fake::FakeLookup;
    use super::super::*;
    use super::*;

    const PS: AppProfile<'static> = AppProfile {
        id: "photoshop",
        name: "Adobe Photoshop",
        kind: AppKind::Editor,
        accepts: &[Media::Photo, Media::Raw],
        reads_raw: true,
        writes: Writes::InPlace,
        win: &[
            WinRule::AppPath("Photoshop.exe"),
            WinRule::Uninstall { name_starts: "Adobe Photoshop", exe: "Photoshop.exe" },
        ],
        mac: &["com.adobe.Photoshop"],
        linux: &["photoshop"],
    };

    const GIMP: AppProfile<'static> = AppProfile {
        id: "gimp",
        name: "GIMP",
        kind: AppKind::Editor,
        accepts: &[Media::Photo],
        reads_raw: false,
        writes: Writes::InPlace,
        win: &[WinRule::Glob("%ProgramFiles%/GIMP */bin/gimp-*.exe")],
        mac: &[],
        linux: &[],
    };

    fn one(p: &'static AppProfile<'static>) -> BuiltIn {
        BuiltIn::with(std::slice::from_ref(p), Os::Windows)
    }

    #[test]
    fn an_app_found_by_app_paths() {
        let mut l = FakeLookup::default();
        l.app_paths.insert("Photoshop.exe".into(), PathBuf::from("X:/Adobe/Photoshop.exe"));
        let apps = one(&PS).apps(&l);
        assert_eq!(apps.len(), 1);
        assert_eq!(apps[0].target, PathBuf::from("X:/Adobe/Photoshop.exe"));
        assert_eq!((apps[0].id.as_str(), apps[0].custom), ("photoshop", false));
    }

    #[test]
    fn an_app_found_by_its_uninstall_name() {
        let mut l = FakeLookup::default();
        l.uninstall.push(("Adobe Photoshop 2026".into(), Some(PathBuf::from("D:/Adobe/Photoshop"))));
        // The entry alone is not enough: the exe must be there.
        assert!(one(&PS).apps(&l).is_empty());
        l.files.insert(PathBuf::from("D:/Adobe/Photoshop/Photoshop.exe"));
        let apps = one(&PS).apps(&l);
        assert_eq!(apps[0].target, PathBuf::from("D:/Adobe/Photoshop/Photoshop.exe"));
    }

    #[test]
    fn photoshop_elements_is_not_photoshop() {
        let mut l = FakeLookup::default();
        l.uninstall.push(("Adobe Photoshop Elements 2025".into(), Some(PathBuf::from("D:/Adobe/Elements"))));
        l.files.insert(PathBuf::from("D:/Adobe/Elements/PhotoshopElementsEditor.exe"));
        assert!(one(&PS).apps(&l).is_empty());
    }

    #[test]
    fn an_app_found_by_glob_takes_the_newest() {
        let mut l = FakeLookup::default();
        l.globs.insert(
            "%ProgramFiles%/GIMP */bin/gimp-*.exe".into(),
            vec![PathBuf::from("C:/GIMP 2/bin/gimp-2.10.exe"), PathBuf::from("C:/GIMP 10/bin/gimp-10.0.exe"), PathBuf::from("C:/GIMP 9/bin/gimp-9.0.exe")],
        );
        let apps = one(&GIMP).apps(&l);
        assert_eq!(apps[0].target, PathBuf::from("C:/GIMP 10/bin/gimp-10.0.exe"));
        let two = vec![PathBuf::from("C:/GIMP 2/bin/gimp-2.10.exe"), PathBuf::from("C:/GIMP 3/bin/gimp-3.0.exe")];
        l.globs.insert("%ProgramFiles%/GIMP */bin/gimp-*.exe".into(), two);
        assert_eq!(one(&GIMP).apps(&l)[0].target, PathBuf::from("C:/GIMP 3/bin/gimp-3.0.exe"));
    }

    #[test]
    fn a_missing_app_is_absent() {
        let l = FakeLookup::default();
        for os in [Os::Windows, Os::Mac, Os::Linux] {
            assert!(BuiltIn::new(os).apps(&l).is_empty(), "{os:?}");
        }
    }

    #[test]
    fn mac_bundles_and_linux_desktop_entries_resolve() {
        let mut l = FakeLookup::default();
        l.bundles.insert("com.adobe.Photoshop".into(), PathBuf::from("/Applications/Adobe Photoshop 2026/Adobe Photoshop 2026.app"));
        l.desktops.insert("photoshop".into(), PathBuf::from("/usr/share/applications/photoshop.desktop"));
        let mac = BuiltIn::with(std::slice::from_ref(&PS), Os::Mac).apps(&l);
        assert_eq!(mac[0].target, PathBuf::from("/Applications/Adobe Photoshop 2026/Adobe Photoshop 2026.app"));
        assert_eq!(mac[0].exec, None);
        let linux = BuiltIn::with(std::slice::from_ref(&PS), Os::Linux).apps(&l);
        assert_eq!(linux[0].target, PathBuf::from("/usr/share/applications/photoshop.desktop"));
        assert_eq!(linux[0].exec.as_deref(), Some("photoshop %U"));
    }

    #[test]
    fn ids_are_unique_and_sorted_by_name() {
        let mut l = FakeLookup::default();
        l.app_paths.insert("Photoshop.exe".into(), PathBuf::from("C:/ps.exe"));
        l.globs.insert("%ProgramFiles%/GIMP */bin/gimp-*.exe".into(), vec![PathBuf::from("C:/gimp.exe")]);
        let all = [PS, GIMP];
        let builtin = BuiltIn::with(Box::leak(Box::new(all)), Os::Windows);
        // A second source offering the same id loses to the first.
        let dup = one(&PS);
        let apps = installed(&[&builtin, &dup], &l);
        let names: Vec<&str> = apps.iter().map(|a| a.name.as_str()).collect();
        assert_eq!(names, ["Adobe Photoshop", "GIMP"]);
    }

    #[test]
    fn natural_order_reads_numbers() {
        assert_eq!(natural_cmp("PhotoLab 9", "PhotoLab 10"), Ordering::Less);
        assert_eq!(natural_cmp("a2", "A2"), Ordering::Equal);
        assert_eq!(natural_cmp("v1.10", "v1.9"), Ordering::Greater);
    }

    fn p(s: &str) -> PathBuf {
        PathBuf::from(s)
    }

    #[test]
    fn linux_exec_fills_its_field_codes() {
        let files = [p("/a.jpg"), p("/b.jpg")];
        assert_eq!(exec_launches("gimp-2.10 %U", &files), vec![vec!["gimp-2.10", "/a.jpg", "/b.jpg"]]);
        // %f takes one path per launch.
        assert_eq!(exec_launches("gimp %f", &files), vec![vec!["gimp", "/a.jpg"], vec!["gimp", "/b.jpg"]]);
        // No code: the paths are appended.
        assert_eq!(exec_launches("krita", &files), vec![vec!["krita", "/a.jpg", "/b.jpg"]]);
        // %i, %c and %k are dropped; quotes keep a spaced name together; %% is a percent sign.
        assert_eq!(
            exec_launches("\"/opt/My App/app\" %i %c %k --scale=100%% %F", &files[..1]),
            vec![vec!["/opt/My App/app", "--scale=100%", "/a.jpg"]]
        );
    }
}
