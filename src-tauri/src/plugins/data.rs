//! Presets, looks, apps and prompts from plugins (Round 23 lane B2): the three `KindCheck`s the
//! store calls at inspect and install, and the `plugin_*` readers the rest of Lumen asks for the
//! enabled plugins. Nothing here writes to the library: a plugin's presets are read from its
//! installed folder each time and shown read-only.

use std::fs;
use std::path::Path;
use photos_core::develop_presets::{self, Preset, PresetSource};
use photos_core::external_apps::{parse_plugin_apps, PluginApps};
use photos_core::edits::Develop;
use photos_core::develop::Panel;
use photos_core::lightroom;
use photos_plugins::package::{KindCheck, Report, Unpacked};
use photos_plugins::store::{Installed, Store};
use photos_plugins::PluginKind;

/// The binary's kind checkers, run by the store after the crate's own.
pub fn checks() -> Vec<Box<dyn KindCheck>> {
    vec![Box::new(PresetCheck), Box::new(AppsCheck), Box::new(PromptCheck)]
}

// Presets

/// One preset file read: what Lumen shows, the look it may describe, and what a conversion lost.
struct LoadedPreset {
    /// The file's name inside the presets folder (`warm.json`).
    file: String,
    name: String,
    develop: Develop,
    panels: Vec<Panel>,
    look: Option<serde_json::Value>,
    notes: Vec<String>,
}

/// Reads one preset file: Lumen's JSON through the recipe checks, or a Lightroom `.xmp` through
/// the converter (never written anywhere). Errors are one line naming the file.
fn read_preset(file: &str, bytes: &[u8]) -> Result<LoadedPreset, String> {
    let fail = |why: String| format!("{file}: {why}");
    let text = std::str::from_utf8(bytes).map_err(|_| fail("isn't UTF-8 text".into()))?;
    let ext = file.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default();
    match ext.as_str() {
        "json" => {
            let f = develop_presets::parse_plugin_preset(text).map_err(fail)?;
            Ok(LoadedPreset { file: file.into(), name: f.name, develop: f.develop, panels: f.panels, look: f.look, notes: Vec::new() })
        }
        "xmp" => {
            let settings = lightroom::read_xmp(text).map_err(|e| fail(e.to_string()))?;
            let c = lightroom::convert(&settings, None);
            if c.panels.is_empty() {
                return Err(fail("nothing in it is something a Lumen preset sets".into()));
            }
            let mut notes: Vec<String> = c.skipped.iter().map(|s| format!("{file}: {s} not converted")).collect();
            if c.geometry.is_some() {
                notes.push(format!("{file}: Crop and straighten not converted"));
            }
            let name = c.name.clone().map(|n| n.trim().to_string()).filter(|n| !n.is_empty()).unwrap_or_else(|| file.rsplit_once('.').map_or(file, |(stem, _)| stem).to_string());
            Ok(LoadedPreset { file: file.into(), name, develop: c.develop, panels: c.panels, look: None, notes })
        }
        _ => Err(fail("isn't a preset (.json or .xmp)".into())),
    }
}

/// The files directly inside `dir` among a package's files, by name.
fn files_in<'a>(pkg: &'a Unpacked, dir: &str) -> Vec<(&'a str, &'a str)> {
    let prefix = format!("{}/", dir.trim_matches('/'));
    pkg.files.keys().filter_map(|rel| rel.strip_prefix(&prefix).filter(|name| !name.contains('/')).map(|name| (rel.as_str(), name))).collect()
}

struct PresetCheck;

impl KindCheck for PresetCheck {
    fn kinds(&self) -> &'static [PluginKind] {
        &[PluginKind::Presets]
    }

    fn check(&self, pkg: &Unpacked, _dev: bool, report: &mut Report) {
        let Some(dir) = &pkg.manifest.presets else { return };
        let mut good = 0usize;
        for (rel, name) in files_in(pkg, &dir.dir) {
            match pkg.read(rel).map_err(|e| format!("{name}: {}", e.message)).and_then(|b| read_preset(name, &b)) {
                Ok(p) => {
                    good += 1;
                    report.notes.extend(p.notes);
                    if let Some(look) = &p.look {
                        if let Err(e) = crate::mcp::looks::plugin_look(&p.file, &p.name, look, &p.develop, &p.panels) {
                            report.problems.push(format!("{name}: {e}"));
                        }
                    }
                }
                Err(e) => report.problems.push(e),
            }
        }
        if good == 0 && report.problems.is_empty() {
            report.problems.push(format!("{} has no presets", dir.dir));
        }
        if good > 0 {
            report.adds.push(format!("{good} Develop preset{}", if good == 1 { "" } else { "s" }));
        }
    }
}

/// The presets of one installed plugin, read from its folder; a file that no longer reads is
/// left out (the install checks refused it, so this only happens to a changed dev folder).
fn presets_of(inst: &Installed) -> Vec<(Preset, Option<serde_json::Value>)> {
    let Some(dir) = &inst.manifest.presets else { return Vec::new() };
    let Ok(entries) = fs::read_dir(inst.dir.join(&dir.dir)) else { return Vec::new() };
    let mut names: Vec<String> = entries.flatten().filter(|e| e.file_type().is_ok_and(|t| t.is_file())).filter_map(|e| e.file_name().into_string().ok()).collect();
    names.sort();
    names
        .into_iter()
        .filter_map(|file| {
            let bytes = fs::read(inst.dir.join(&dir.dir).join(&file)).ok()?;
            let p = read_preset(&file, &bytes).ok()?;
            let preset = Preset {
                id: 0,
                key: format!("plugin:{}/{}", inst.manifest.id, p.file),
                name: p.name,
                develop: p.develop,
                panels: p.panels,
                builtin: false,
                source: PresetSource::Plugin { plugin: inst.manifest.id.clone(), name: inst.manifest.name.clone() },
            };
            Some((preset, p.look))
        })
        .collect()
}

/// Every enabled plugin's presets, by plugin id then file name. Read-only: ids are 0 and the
/// `key` tells them apart.
pub fn plugin_presets(store: &Store) -> Vec<Preset> {
    let mut enabled = store.enabled();
    enabled.sort_by(|a, b| a.manifest.id.cmp(&b.manifest.id));
    enabled.into_iter().flat_map(|i| presets_of(i).into_iter().map(|(p, _)| p)).collect()
}

/// The looks enabled plugins' presets describe (`look = {...}` in the preset JSON), for the MCP
/// catalogue. A look that no longer reads is left out (the install check refused it).
pub fn plugin_looks(store: &Store) -> Vec<crate::mcp::looks::Look> {
    let mut enabled = store.enabled();
    enabled.sort_by(|a, b| a.manifest.id.cmp(&b.manifest.id));
    enabled
        .into_iter()
        .flat_map(presets_of)
        .filter_map(|(p, look)| crate::mcp::looks::plugin_look(&p.key, &p.name, &look?, &p.develop, &p.panels).ok())
        .collect()
}

/// One enabled plugin's preset by its `key` (`plugin:<id>/<file>`).
pub fn plugin_preset(store: &Store, key: &str) -> Option<Preset> {
    let rest = key.strip_prefix("plugin:")?;
    let (id, _) = rest.split_once('/')?;
    let inst = store.enabled().into_iter().find(|i| i.manifest.id == id)?;
    presets_of(inst).into_iter().map(|(p, _)| p).find(|p| p.key == key)
}

// Apps

struct AppsCheck;

impl KindCheck for AppsCheck {
    fn kinds(&self) -> &'static [PluginKind] {
        &[PluginKind::Apps]
    }

    fn check(&self, pkg: &Unpacked, _dev: bool, report: &mut Report) {
        let Some(file) = &pkg.manifest.apps else { return };
        let text = match pkg.read(&file.file).map_err(|e| e.message).and_then(|b| String::from_utf8(b).map_err(|_| "isn't UTF-8 text".to_string())) {
            Ok(t) => t,
            Err(e) => return report.problems.push(format!("{}: {e}", file.file)),
        };
        match parse_plugin_apps(&pkg.manifest.id, &text) {
            Ok(apps) => report.adds.extend(apps.iter().map(|a| format!("An app: \"{}\"", a.name))),
            Err(e) => report.problems.push(format!("{}: {e}", file.file)),
        }
    }
}

/// One source per enabled plugin that lists apps. A file that no longer reads is left out (the
/// install checks refused it, so this only happens to a changed dev folder). Ids are
/// `plugin:<plugin id>/<app id>`, so they never meet a built-in's or a custom app's.
pub fn plugin_apps(store: &Store) -> Vec<PluginApps> {
    let mut enabled = store.enabled();
    enabled.sort_by(|a, b| a.manifest.id.cmp(&b.manifest.id));
    enabled
        .into_iter()
        .filter_map(|inst| {
            let file = inst.manifest.apps.as_ref()?;
            let text = fs::read_to_string(inst.dir.join(&file.file)).ok()?;
            let profiles = parse_plugin_apps(&inst.manifest.id, &text).ok()?;
            let mut source = PluginApps::new(&inst.manifest.id, &inst.manifest.name, profiles);
            // A plugin may ship a program of its own; one that lives in its folder, or anywhere
            // in Lumen's plugins folder, is never an app Lumen starts.
            source.forbidden.push(inst.dir.clone());
            if let Some(plugins_dir) = store.data_dir(&inst.manifest.id).parent().and_then(Path::parent) {
                source.forbidden.push(plugins_dir.to_path_buf());
            }
            Some(source)
        })
        .collect()
}

// Prompts

/// The most a prompt file may hold (spec §3, *Limits*).
const MAX_PROMPT: usize = 32 * 1024;

fn prompt_name_ok(name: &str) -> bool {
    (1..=40).contains(&name.len()) && name.bytes().all(|b| matches!(b, b'a'..=b'z' | b'0'..=b'9' | b'_' | b'-'))
}

/// A prompt file's text: UTF-8 up to 32 KB, never garbled (UTF-16 has a byte-order mark or NULs).
fn read_prompt(bytes: &[u8]) -> Result<String, String> {
    if bytes.len() > MAX_PROMPT {
        return Err("is over 32 KB".into());
    }
    let text = std::str::from_utf8(bytes).map_err(|_| "isn't UTF-8 text".to_string())?;
    if text.chars().any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t')) {
        return Err("isn't UTF-8 text (it has control characters; is it UTF-16?)".into());
    }
    if text.trim().is_empty() {
        return Err("is empty".into());
    }
    Ok(text.to_string())
}

struct PromptCheck;

impl KindCheck for PromptCheck {
    fn kinds(&self) -> &'static [PluginKind] {
        &[PluginKind::Prompts]
    }

    fn check(&self, pkg: &Unpacked, _dev: bool, report: &mut Report) {
        let mut seen: Vec<&str> = Vec::new();
        for decl in &pkg.manifest.prompts {
            if !prompt_name_ok(&decl.name) {
                report.problems.push(format!("prompt \"{}\": name must be 1-40 of a-z, 0-9, _ and -", decl.name));
                continue;
            }
            if seen.contains(&decl.name.as_str()) {
                report.problems.push(format!("prompt \"{}\" is listed twice", decl.name));
                continue;
            }
            seen.push(&decl.name);
            if decl.description.trim().is_empty() {
                report.problems.push(format!("prompt \"{}\" needs a description", decl.name));
                continue;
            }
            match pkg.read(&decl.file).map_err(|e| e.message).and_then(|b| read_prompt(&b)) {
                Ok(_) => report.adds.push(format!("A prompt for AI assistants: \"{}\"", decl.name)),
                Err(e) => report.problems.push(format!("{}: {e}", decl.file)),
            }
        }
    }
}

/// A prompt an enabled plugin offers AI assistants. A4 names it `<short>__<name>` (falling back
/// to the plugin id on a clash) and lists it while AI assistants are on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginPrompt {
    /// The plugin's id.
    pub plugin: String,
    /// The last segment of the id.
    pub short: String,
    pub name: String,
    pub description: String,
    pub text: String,
}

/// Every enabled plugin's prompts, by plugin id then declaration order. One that no longer reads
/// is left out (the install checks refused it, so this only happens to a changed dev folder).
pub fn plugin_prompts(store: &Store) -> Vec<PluginPrompt> {
    let mut enabled = store.enabled();
    enabled.sort_by(|a, b| a.manifest.id.cmp(&b.manifest.id));
    enabled
        .into_iter()
        .flat_map(|inst| {
            inst.manifest.prompts.iter().filter_map(move |decl| {
                if !prompt_name_ok(&decl.name) {
                    return None;
                }
                let text = fs::read(inst.dir.join(&decl.file)).ok().and_then(|b| read_prompt(&b).ok())?;
                Some(PluginPrompt { plugin: inst.manifest.id.clone(), short: inst.manifest.short_name().to_string(), name: decl.name.clone(), description: decl.description.clone(), text })
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;

    use photos_core::develop_presets::{self, PresetSource};

    use super::*;
    use crate::plugins::Plugins;

    /// A plugin folder with `plugin.toml` (id, name and the `extra` tables) and `files`.
    fn folder(root: &Path, id: &str, name: &str, extra: &str, files: &[(&str, &str)]) -> std::path::PathBuf {
        let dir = root.join(id);
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("plugin.toml"),
            format!("id = \"{id}\"\nname = \"{name}\"\nversion = \"1.0.0\"\nauthor = \"T\"\ndescription = \"d\"\nlicense = \"MIT\"\nlumen = \">=26.0.0\"\napi = 1\n{extra}\n"),
        )
        .unwrap();
        for (rel, text) in files {
            let p = dir.join(rel);
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            fs::write(p, text).unwrap();
        }
        dir
    }

    const PRESETS: &str = "[presets]\ndir = \"presets\"\n";

    fn json(name: &str, temperature: f64) -> String {
        format!(r#"{{ "recipeVersion": 3, "name": "{name}", "develop": {{ "basic": {{ "temperature": {temperature} }} }}, "panels": ["basic"] }}"#)
    }

    const XMP: &str = r#"<x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#"><rdf:Description xmlns:crs="http://ns.adobe.com/camera-raw-settings/1.0/"
   crs:PresetType="Normal" crs:Name="Soft Warm" crs:Contrast2012="+20" crs:GrainAmount="30" crs:LensProfileEnable="1"/></rdf:RDF></x:xmpmeta>"#;

    /// Plugins in developer mode (so unsigned folders load) in a temp app dir.
    fn plugins_with(folders: &[&Path]) -> (tempfile::TempDir, Plugins) {
        let app = tempfile::tempdir().unwrap();
        let plugins = Plugins::load(app.path());
        plugins.write().set_developer_mode(true).unwrap();
        let ctx = plugins.ctx();
        for f in folders {
            plugins.write().load_dev(f, &ctx).unwrap();
        }
        (app, plugins)
    }

    fn report_of(dir: &Path) -> photos_plugins::package::Report {
        let pkg = photos_plugins::package::open_folder(dir).unwrap();
        let mut report = photos_plugins::package::Report::default();
        for c in checks() {
            c.check(&pkg, true, &mut report);
        }
        report
    }

    #[test]
    fn preset_check_adds_and_notes() {
        let t = tempfile::tempdir().unwrap();
        let dir = folder(
            t.path(),
            "com.test.looks",
            "Looks",
            PRESETS,
            &[("presets/a.json", &json("A", 5.0)), ("presets/b.json", &json("B", 6.0)), ("presets/warm.xmp", XMP)],
        );
        let report = report_of(&dir);
        assert_eq!(report.problems, Vec::<String>::new());
        assert_eq!(report.adds, ["3 Develop presets"]);
        assert_eq!(report.notes, ["warm.xmp: LensProfileEnable not converted"], "{:?}", report.notes);
    }

    #[test]
    fn preset_check_names_the_file_of_every_problem() {
        let t = tempfile::tempdir().unwrap();
        let dir = folder(
            t.path(),
            "com.test.bad",
            "Bad",
            PRESETS,
            &[
                ("presets/new.json", r#"{"recipeVersion":9,"name":"N","develop":{},"panels":["basic"]}"#),
                ("presets/broken.json", "{ not json"),
                ("presets/readme.txt", "hello"),
                ("presets/lens.xmp", r#"<x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#"><rdf:Description xmlns:crs="http://ns.adobe.com/camera-raw-settings/1.0/" crs:LensProfileEnable="1"/></rdf:RDF></x:xmpmeta>"#),
            ],
        );
        let report = report_of(&dir);
        assert!(report.adds.is_empty(), "{:?}", report.adds);
        assert_eq!(report.problems.len(), 4, "{:?}", report.problems);
        for file in ["new.json", "broken.json", "readme.txt", "lens.xmp"] {
            assert!(report.problems.iter().any(|p| p.starts_with(&format!("{file}:"))), "{file}: {:?}", report.problems);
        }
        assert!(report.problems.iter().any(|p| p.contains("newer")), "{:?}", report.problems);
    }

    #[test]
    fn xmp_preset_converts() {
        let t = tempfile::tempdir().unwrap();
        let dir = folder(t.path(), "com.test.lr", "LR", PRESETS, &[("presets/warm.xmp", XMP)]);
        let (_app, plugins) = plugins_with(&[&dir]);
        let presets = plugin_presets(&plugins.read());
        assert_eq!(presets.len(), 1);
        let want = photos_core::lightroom::convert(&photos_core::lightroom::read_xmp(XMP).unwrap(), None);
        assert_eq!(presets[0].develop, want.develop);
        assert_eq!(presets[0].panels, want.panels);
        assert_eq!(presets[0].name, "Soft Warm");
        assert_eq!(presets[0].key, "plugin:com.test.lr/warm.xmp");
        assert_eq!(presets[0].source, PresetSource::Plugin { plugin: "com.test.lr".into(), name: "LR".into() });
        assert!(presets[0].develop.basic.contrast > 0.0);
    }

    #[test]
    fn same_file_two_plugins() {
        let t = tempfile::tempdir().unwrap();
        let one = folder(t.path(), "com.test.one", "One", PRESETS, &[("presets/warm.json", &json("Warm", 5.0))]);
        let two = folder(t.path(), "com.test.two", "Two", PRESETS, &[("presets/warm.json", &json("Warm", 9.0))]);
        let (_app, plugins) = plugins_with(&[&one, &two]);
        let presets = plugin_presets(&plugins.read());
        let keys: Vec<&str> = presets.iter().map(|p| p.key.as_str()).collect();
        assert_eq!(keys, ["plugin:com.test.one/warm.json", "plugin:com.test.two/warm.json"]);
        let groups: Vec<&PresetSource> = presets.iter().map(|p| &p.source).collect();
        assert_eq!(groups[0], &PresetSource::Plugin { plugin: "com.test.one".into(), name: "One".into() });
        assert_eq!(groups[1], &PresetSource::Plugin { plugin: "com.test.two".into(), name: "Two".into() });
        assert_ne!(presets[0].develop, presets[1].develop);
        assert!(presets.iter().all(|p| p.id == 0 && !p.builtin));
    }

    #[test]
    fn disabled_plugin_presets_gone() {
        let t = tempfile::tempdir().unwrap();
        let dir = folder(t.path(), "com.test.off", "Off", PRESETS, &[("presets/a.json", &json("A", 5.0))]);
        let (_app, plugins) = plugins_with(&[&dir]);
        assert_eq!(plugin_presets(&plugins.read()).len(), 1);
        plugins.write().set_enabled("com.test.off", false).unwrap();
        assert!(plugin_presets(&plugins.read()).is_empty());
        plugins.write().set_enabled("com.test.off", true).unwrap();
        assert_eq!(plugin_presets(&plugins.read()).len(), 1);
    }

    #[test]
    fn duplicate_survives_removal() {
        let t = tempfile::tempdir().unwrap();
        let dir = folder(t.path(), "com.test.gone", "Gone", PRESETS, &[("presets/warm.json", &json("Warm", 5.0))]);
        let (_app, plugins) = plugins_with(&[&dir]);
        let db = photos_core::Db::open_in_memory().unwrap();
        let source = plugin_preset(&plugins.read(), "plugin:com.test.gone/warm.json").expect("found by key");
        let copy = db.with_conn(|c| develop_presets::duplicate_preset(c, &source, 0)).unwrap();
        plugins.write().remove("com.test.gone").unwrap();
        assert!(plugin_presets(&plugins.read()).is_empty());
        assert!(plugin_preset(&plugins.read(), "plugin:com.test.gone/warm.json").is_none());
        let kept = db.with_conn(develop_presets::list_presets).unwrap();
        let mine = kept.iter().find(|p| p.id == copy.id).expect("the copy is still there");
        assert_eq!((mine.name.as_str(), &mine.develop, &mine.source), ("Warm", &source.develop, &PresetSource::Mine));
    }

    const APPS: &str = "[apps]
file = \"apps.toml\"
";

    fn apps_toml(entries: &[(&str, &str, &str)]) -> String {
        entries
            .iter()
            .map(|(id, kind, writes)| {
                format!("[[app]]
id = \"{id}\"
name = \"App {id}\"
kind = \"{kind}\"
accepts = [\"photo\", \"raw\", \"video\"]
reads_raw = true
writes = \"{writes}\"
[app.windows]
app_paths = [\"{id}.exe\"]

")
            })
            .collect()
    }

    /// Every `App Paths` name is installed under D:/Apps/.
    struct Everything;
    impl photos_core::external_apps::Lookup for Everything {
        fn app_path(&self, exe: &str) -> Option<std::path::PathBuf> {
            Some(std::path::PathBuf::from(format!("D:/Apps/{exe}")))
        }
        fn uninstall(&self) -> Vec<(String, Option<std::path::PathBuf>)> {
            Vec::new()
        }
        fn glob(&self, _: &str) -> Vec<std::path::PathBuf> {
            Vec::new()
        }
        fn bundle(&self, _: &str) -> Option<std::path::PathBuf> {
            None
        }
        fn desktop(&self, _: &str) -> Option<photos_core::external_apps::DesktopEntry> {
            None
        }
        fn exists(&self, _: &std::path::Path) -> bool {
            true
        }
    }

    #[test]
    fn apps_check_adds_a_line_per_app_and_names_every_problem() {
        let t = tempfile::tempdir().unwrap();
        let good = folder(t.path(), "com.test.apps", "Apps", APPS, &[("apps.toml", &apps_toml(&[("fancy", "editor", "inPlace"), ("neat", "developer", "sidecar")]))]);
        let report = report_of(&good);
        assert_eq!(report.problems, Vec::<String>::new());
        assert_eq!(report.adds, ["An app: \"App fancy\"", "An app: \"App neat\""]);
        let bad = folder(t.path(), "com.test.bad", "Bad", APPS, &[("apps.toml", &apps_toml(&[("shell", "editor", "inPlace")]).replace("shell.exe", "powershell.exe"))]);
        let report = report_of(&bad);
        assert!(report.adds.is_empty());
        assert!(report.problems.len() == 1 && report.problems[0].starts_with("apps.toml:") && report.problems[0].contains("powershell.exe"), "{:?}", report.problems);
        let args = folder(t.path(), "com.test.args", "Args", APPS, &[("apps.toml", &(apps_toml(&[("x", "editor", "inPlace")]) + "[[app.args]]
"))]);
        assert_eq!(report_of(&args).problems.len(), 1);
    }

    #[test]
    fn plugin_app_gets_copy() {
        use photos_core::external_apps::{AppKind, AppSource, Media, Os, SendMode};

        use crate::external_edits::{what_to_send, Sent};

        let t = tempfile::tempdir().unwrap();
        let entries: Vec<(String, &str, &str)> = ["editor", "developer", "video"]
            .into_iter()
            .flat_map(|k| ["inPlace", "sidecar", "never"].into_iter().map(move |w| (format!("{k}-{w}").to_lowercase(), k, w)))
            .collect();
        let refs: Vec<(&str, &str, &str)> = entries.iter().map(|(id, k, w)| (id.as_str(), *k, *w)).collect();
        let dir = folder(t.path(), "com.test.copies", "Copies", APPS, &[("apps.toml", &apps_toml(&refs))]);
        let (_app, plugins) = plugins_with(&[&dir]);
        let mut sources = plugin_apps(&plugins.read());
        assert_eq!(sources.len(), 1);
        sources[0].os = Os::Windows;
        let apps = sources[0].apps(&Everything);
        assert_eq!(apps.len(), 9);
        for a in &apps {
            assert_eq!(a.added_by.as_deref(), Some("Copies"));
            for media in [Media::Photo, Media::Raw, Media::Video] {
                for adjusted in [false, true] {
                    let sent = what_to_send(a.kind, a.writes, SendMode::Automatic, media, adjusted, a.reads_raw);
                    assert_ne!(sent, Sent::Original, "{} {media:?}: a plugin app is never handed an original", a.id);
                }
            }
            // The person's own choice, in Settings → External apps, still applies to an editor.
            if a.kind == AppKind::Editor {
                assert_eq!(what_to_send(a.kind, a.writes, SendMode::Rendered, Media::Photo, false, true), Sent::CopyRendered);
                assert_eq!(what_to_send(a.kind, a.writes, SendMode::OriginalFile, Media::Photo, true, true), Sent::CopyOriginal);
            }
        }
    }

    #[test]
    fn disabled_plugin_apps_gone() {
        let t = tempfile::tempdir().unwrap();
        let dir = folder(t.path(), "com.test.offapps", "Off", APPS, &[("apps.toml", &apps_toml(&[("a", "editor", "inPlace")]))]);
        let (_app, plugins) = plugins_with(&[&dir]);
        assert_eq!(plugin_apps(&plugins.read()).len(), 1);
        plugins.write().set_enabled("com.test.offapps", false).unwrap();
        assert!(plugin_apps(&plugins.read()).is_empty());
    }

    #[test]
    fn looks_join_the_catalogue_only_from_enabled_plugins() {
        let t = tempfile::tempdir().unwrap();
        let with_look = r#"{ "recipeVersion": 3, "name": "Dusk", "develop": { "basic": { "temperature": -8.0, "contrast": 0.1 } }, "panels": ["basic"],
            "look": { "intent": "Cool blue dusk", "hues": [250.0], "key": "low", "skinSafe": false } }"#;
        let dir = folder(t.path(), "com.test.dusk", "Dusk", PRESETS, &[("presets/dusk.json", with_look), ("presets/plain.json", &json("Plain", 5.0))]);
        let (_app, plugins) = plugins_with(&[&dir]);
        let looks = plugin_looks(&plugins.read());
        assert_eq!(looks.len(), 1, "only the preset that describes a look");
        assert_eq!((looks[0].id.as_str(), looks[0].name.as_str(), looks[0].safe), ("plugin:com.test.dusk/dusk.json", "Dusk", false));
        assert_eq!(looks[0].patch["develop"]["basic"]["temperature"], -8.0);
        plugins.write().set_enabled("com.test.dusk", false).unwrap();
        assert!(plugin_looks(&plugins.read()).is_empty());
        // The install check names a look that can't be used.
        let bad = r#"{ "recipeVersion": 3, "name": "Bad", "develop": {}, "panels": ["basic"], "look": { "intent": "x", "key": "huge" } }"#;
        let dir = folder(t.path(), "com.test.badlook", "Bad", PRESETS, &[("presets/bad.json", bad)]);
        let report = report_of(&dir);
        assert!(report.problems.len() == 1 && report.problems[0].starts_with("bad.json:") && report.problems[0].contains("key"), "{:?}", report.problems);
    }

    fn prompt_decl(name: &str, file: &str) -> String {
        format!("[[prompts]]\nname = \"{name}\"\ndescription = \"Retouch toward a look\"\nfile = \"{file}\"\n")
    }

    #[test]
    fn adds_prompt_line() {
        let t = tempfile::tempdir().unwrap();
        let dir = folder(t.path(), "com.test.say", "Say", &prompt_decl("film-look", "prompts/film-look.md"), &[("prompts/film-look.md", "Make it warm, like film.")]);
        let report = report_of(&dir);
        assert_eq!(report.problems, Vec::<String>::new());
        assert_eq!(report.adds, ["A prompt for AI assistants: \"film-look\""]);
    }

    #[test]
    fn prompt_limits() {
        let t = tempfile::tempdir().unwrap();
        let problems = |id: &str, extra: String, file: &str, bytes: &[u8]| {
            let dir = folder(t.path(), id, "P", &extra, &[]);
            let target = dir.join(file);
            std::fs::create_dir_all(target.parent().unwrap()).unwrap();
            std::fs::write(target, bytes).unwrap();
            report_of(&dir).problems
        };
        let decl = prompt_decl("p", "prompts/p.md");
        // Exactly 32 KB is fine; one byte more is a problem.
        assert!(problems("com.test.p1", decl.clone(), "prompts/p.md", &vec![b'a'; 32 * 1024]).is_empty());
        let over = problems("com.test.p2", decl.clone(), "prompts/p.md", &vec![b'a'; 32 * 1024 + 1]);
        assert!(over.len() == 1 && over[0].starts_with("prompts/p.md:") && over[0].contains("32 KB"), "{over:?}");
        // UTF-16, with or without a byte-order mark, is never turned into a garbled prompt.
        let utf16: Vec<u8> = "Make it warm".encode_utf16().flat_map(u16::to_le_bytes).collect();
        let with_bom: Vec<u8> = [&[0xFF, 0xFE][..], &utf16].concat();
        for (id, bytes) in [("com.test.p3", with_bom), ("com.test.p4", utf16)] {
            let p = problems(id, decl.clone(), "prompts/p.md", &bytes);
            assert!(p.len() == 1 && p[0].contains("UTF-8"), "{p:?}");
        }
        assert_eq!(problems("com.test.p5", decl.clone(), "prompts/p.md", b"  \n").len(), 1, "an empty prompt");
        // A name must be a slug; a description is needed; one name once.
        let named = |name: &str| problems("com.test.p6", prompt_decl(name, "prompts/p.md"), "prompts/p.md", b"text");
        for bad in ["Film Look", "", "film.look", &"a".repeat(41)] {
            let p = named(bad);
            assert!(p.iter().any(|m| m.contains("name")), "{bad:?}: {p:?}");
        }
        assert!(named("film_look-2").is_empty());
        let twice = format!("{decl}{decl}");
        assert!(problems("com.test.p7", twice, "prompts/p.md", b"text").iter().any(|m| m.contains("twice")));
        let no_description = decl.replace("description = \"Retouch toward a look\"", "description = \" \"");
        assert!(!problems("com.test.p8", no_description, "prompts/p.md", b"text").is_empty());
    }

    #[test]
    fn prompts_from_enabled_only() {
        let t = tempfile::tempdir().unwrap();
        let dir = folder(t.path(), "studio.example.film", "Film", &prompt_decl("film-look", "prompts/film-look.md"), &[("prompts/film-look.md", "Make it warm, like film.\n")]);
        let (_app, plugins) = plugins_with(&[&dir]);
        let prompts = plugin_prompts(&plugins.read());
        assert_eq!(prompts.len(), 1);
        let p = &prompts[0];
        assert_eq!((p.plugin.as_str(), p.short.as_str(), p.name.as_str(), p.description.as_str()), ("studio.example.film", "film", "film-look", "Retouch toward a look"));
        assert_eq!(p.text, "Make it warm, like film.\n");
        plugins.write().set_enabled("studio.example.film", false).unwrap();
        assert!(plugin_prompts(&plugins.read()).is_empty());
    }

    #[test]
    fn a_program_the_plugin_ships_is_never_its_app() {
        use photos_core::external_apps::{AppSource, Os};

        /// Every `App Paths` name resolves to a file inside the plugin's own folder.
        struct Inside(std::path::PathBuf);
        impl photos_core::external_apps::Lookup for Inside {
            fn app_path(&self, exe: &str) -> Option<std::path::PathBuf> {
                Some(self.0.join(exe))
            }
            fn uninstall(&self) -> Vec<(String, Option<std::path::PathBuf>)> {
                Vec::new()
            }
            fn glob(&self, _: &str) -> Vec<std::path::PathBuf> {
                Vec::new()
            }
            fn bundle(&self, _: &str) -> Option<std::path::PathBuf> {
                None
            }
            fn desktop(&self, _: &str) -> Option<photos_core::external_apps::DesktopEntry> {
                None
            }
            fn exists(&self, _: &std::path::Path) -> bool {
                true
            }
        }

        let t = tempfile::tempdir().unwrap();
        let dir = folder(t.path(), "com.test.ships", "Ships", APPS, &[("apps.toml", &apps_toml(&[("fancy", "editor", "inPlace")]))]);
        let (app, plugins) = plugins_with(&[&dir]);
        let store = plugins.read();
        let mut sources = plugin_apps(&store);
        sources[0].os = Os::Windows;
        // Resolving into the plugin's own folder (a dev folder here) is refused ...
        assert!(sources[0].apps(&Inside(dir.clone())).is_empty(), "the plugin's own folder");
        // ... and so is Lumen's plugins folder, where installed plugins live.
        assert!(sources[0].apps(&Inside(app.path().join("plugins").join("installed").join("com.test.ships"))).is_empty(), "the plugins folder");
        // An app somewhere else is found.
        assert_eq!(sources[0].apps(&Inside("D:/Apps".into())).len(), 1);
    }
}
