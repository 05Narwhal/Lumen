//! Declarative providers (Round 24 §8.1, lane D2): a plugin's app answers file dialogs and
//! *Show in folder*. The program is launched with an argument vector (never a shell), only if a
//! `[[detect]]` entry resolved it, and any failure but *cancelled* falls back to the system.

use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

use photos_plugins::manifest::{Detect, Provider, ProviderKind};
use photos_plugins::{PickKind, PickOutcome, PickRequest, ProviderHost};
use serde::Deserialize;

use super::detect;
use super::Plugins;

/// How long a picker may stay open before Lumen gives up on it (the person may take a while).
pub const PICK_TIMEOUT: Duration = Duration::from_secs(10 * 60);

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ProviderError {
    #[error("the provider has no word for this kind of dialog")]
    NotProvided,
    #[error("the app was not found")]
    NoExe,
    #[error("the app could not start: {0}")]
    Spawn(String),
    #[error("the app did not answer in time")]
    Timeout,
    #[error("the app exited with {0}")]
    Exit(String),
    #[error("the app's answer was not understood: {0}")]
    BadOutput(String),
}

/// Whether `provider` can serve a dialog of `kind` (a kind missing from `modes` is not provided).
pub fn serves(provider: &Provider, kind: PickKind) -> bool {
    provider.kind == ProviderKind::FilePicker && provider.modes.contains_key(&kind)
}

fn single_placeholder(tok: &str) -> Option<&str> {
    let inner = tok.strip_prefix('{')?.strip_suffix('}')?;
    (!inner.contains(['{', '}'])).then_some(inner)
}

/// `{name}` placeholders inside a longer token are replaced in place (missing value → empty).
fn substitute(tok: &str, value: &dyn Fn(&str) -> Option<String>) -> String {
    let mut out = String::new();
    let mut rest = tok;
    while let Some(open) = rest.find('{') {
        let Some(len) = rest[open..].find('}') else { break };
        out.push_str(&rest[..open]);
        let name = &rest[open + 1..open + len];
        out.push_str(&value(name).unwrap_or_default());
        rest = &rest[open + len + 1..];
    }
    out.push_str(rest);
    out
}

/// Fills `args`: a lone placeholder with no value is dropped, and so is the flag before it (an
/// argument *pair*); `{filters...}` becomes one `--filter "Label|*.a;*.b"` per filter.
fn expand(args: &[String], filters: &[photos_plugins::PickFilter], value: &dyn Fn(&str) -> Option<String>) -> Vec<String> {
    let has = |n: &str| value(n).is_some_and(|v| !v.is_empty());
    let mut out = Vec::new();
    let mut i = 0;
    while i < args.len() {
        let tok = &args[i];
        if let Some(name) = single_placeholder(tok) {
            if name == "filters..." {
                for f in filters {
                    let pattern = f.extensions.iter().map(|e| format!("*.{e}")).collect::<Vec<_>>().join(";");
                    out.push("--filter".to_string());
                    out.push(format!("{}|{pattern}", f.label));
                }
            } else if has(name) {
                out.extend(value(name));
            }
            i += 1;
            continue;
        }
        if let Some(next) = args.get(i + 1).and_then(|n| single_placeholder(n)) {
            if next != "filters..." && !has(next) {
                i += 2; // the flag and its missing value go together
                continue;
            }
        }
        out.push(substitute(tok, value));
        i += 1;
    }
    out
}

/// The picker's argument vector. `parent` is the main window's handle (Windows only; the caller
/// passes `None` elsewhere).
pub fn expand_picker_args(
    provider: &Provider,
    req: &PickRequest,
    parent: Option<&str>,
) -> Result<Vec<String>, ProviderError> {
    let mode = provider.modes.get(&req.kind).ok_or(ProviderError::NotProvided)?;
    let value = |name: &str| -> Option<String> {
        match name {
            "mode" => Some(mode.clone()),
            "title" => req.title.clone(),
            "start" => req.start.clone(),
            "name" if req.kind == PickKind::Save => req.name.clone(),
            "parent" => parent.map(str::to_string),
            _ => None,
        }
    };
    Ok(expand(&provider.args, &req.filters, &value))
}

/// The reveal provider's argument vector for `path` (`{folder}` is its parent folder).
pub fn expand_reveal_args(provider: &Provider, path: &Path) -> Vec<String> {
    let folder = path.parent().map(|p| p.to_string_lossy().into_owned());
    let value = |name: &str| -> Option<String> {
        match name {
            "path" => Some(path.to_string_lossy().into_owned()),
            "folder" => folder.clone(),
            _ => None,
        }
    };
    expand(&provider.args, &[], &value)
}

#[derive(Deserialize)]
struct Answer {
    paths: Vec<String>,
    #[serde(default)]
    filter: Option<u32>,
}

fn read_all(mut pipe: impl Read) -> String {
    let mut bytes = Vec::new();
    let _ = pipe.read_to_end(&mut bytes);
    String::from_utf8_lossy(&bytes).into_owned()
}

/// Runs the picker and waits for it (call from a blocking thread). Exit 1 is *cancelled* (no
/// fallback), 0 with one JSON line is the answer, anything else is a failure.
pub fn run_picker(
    provider: &Provider,
    exe: &Path,
    req: &PickRequest,
    parent: Option<&str>,
    timeout: Duration,
) -> Result<PickOutcome, ProviderError> {
    let args = expand_picker_args(provider, req, parent)?;
    let mut child = Command::new(exe)
        .args(&args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| ProviderError::Spawn(e.to_string()))?;
    let pipe = child.stdout.take().expect("stdout was piped");
    let reader = std::thread::spawn(move || read_all(pipe));
    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(s)) => break s,
            Ok(None) if started.elapsed() >= timeout => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(ProviderError::Timeout); // the reader ends when the pipe closes
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(20)),
            Err(e) => return Err(ProviderError::Spawn(e.to_string())),
        }
    };
    match status.code() {
        Some(1) => Ok(PickOutcome::Cancelled),
        Some(0) => {
            let out = reader.join().unwrap_or_default();
            let line = out.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("");
            let answer: Answer = serde_json::from_str(line).map_err(|e| ProviderError::BadOutput(e.to_string()))?;
            if answer.paths.is_empty() {
                return Err(ProviderError::BadOutput("no paths".into()));
            }
            Ok(PickOutcome::Picked { paths: answer.paths, filter: answer.filter })
        }
        other => Err(ProviderError::Exit(other.map_or("a signal".into(), |c| c.to_string()))),
    }
}

/// Starts the reveal provider and returns at once (the app is usually already open and hands
/// the path over); a spawn failure is the only failure.
pub fn run_reveal(provider: &Provider, exe: &Path, path: &Path) -> Result<(), ProviderError> {
    let args = expand_reveal_args(provider, path);
    let mut child = Command::new(exe)
        .args(&args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| ProviderError::Spawn(e.to_string()))?;
    std::thread::spawn(move || {
        let _ = child.wait(); // no zombie
    });
    Ok(())
}

/// A provider that could be active: its plugin, itself and the app it launches.
#[derive(Debug, Clone)]
pub struct Candidate {
    pub plugin: String,
    pub provider: Provider,
    /// The `[[detect]]` entry named by `provider.app`; `None` means the manifest names none (never launched).
    pub detect: Option<Detect>,
}

/// The one provider of `kind`: the person's choice when it is among them, else the first.
pub fn active<'a>(cands: &'a [Candidate], kind: ProviderKind, choice: Option<&str>) -> Option<&'a Candidate> {
    let of_kind: Vec<&Candidate> = cands.iter().filter(|c| c.provider.kind == kind).collect();
    choice
        .and_then(|id| of_kind.iter().copied().find(|c| c.plugin == id))
        .or_else(|| of_kind.first().copied())
}

/// Which plugin answers each kind when several could (`plugins/provider-choice.json`).
#[derive(Debug, Default, Clone, PartialEq, Eq, serde::Serialize, Deserialize)]
pub struct Choice(pub BTreeMap<String, String>);

impl Choice {
    fn file(dir: &Path) -> PathBuf {
        dir.join("provider-choice.json")
    }
    pub fn load(dir: &Path) -> Choice {
        std::fs::read(Self::file(dir)).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
    }
    pub fn save(&self, dir: &Path) -> std::io::Result<()> {
        std::fs::create_dir_all(dir)?;
        std::fs::write(Self::file(dir), serde_json::to_vec_pretty(self).unwrap_or_default())
    }
    pub fn get(&self, kind: ProviderKind) -> Option<&str> {
        self.0.get(kind_key(kind)).map(String::as_str)
    }
    pub fn set(&mut self, kind: ProviderKind, plugin: &str) {
        self.0.insert(kind_key(kind).to_string(), plugin.to_string());
    }
}

pub fn kind_key(kind: ProviderKind) -> &'static str {
    match kind {
        ProviderKind::FilePicker => "file-picker",
        ProviderKind::Reveal => "reveal",
    }
}

type Lookup = Box<dyn Fn(&str, &Detect) -> Option<PathBuf> + Send + Sync>;

/// The binary's [`ProviderHost`]: reads the enabled plugins' providers on every call, so turning
/// a plugin or its switch off takes effect at once.
pub struct PluginProviders {
    plugins: Arc<Plugins>,
    dir: PathBuf,
    parent: Box<dyn Fn() -> Option<String> + Send + Sync>,
    lookup: Lookup,
    timeout: Duration,
}

impl PluginProviders {
    /// `dir` is `<app>/plugins/`; `parent` gives the main window's handle on Windows.
    pub fn new(
        plugins: Arc<Plugins>,
        dir: PathBuf,
        parent: impl Fn() -> Option<String> + Send + Sync + 'static,
    ) -> Self {
        PluginProviders {
            plugins,
            dir,
            parent: Box::new(parent),
            lookup: Box::new(|_, d| detect::find_declared(d)),
            timeout: PICK_TIMEOUT,
        }
    }

    /// `<app>/plugins/`, where the choice file lives.
    pub fn dir(&self) -> PathBuf {
        self.dir.clone()
    }

    /// The resolver (located exe, registry, folders, `PATH`) in place of the plain lookup.
    pub fn with_lookup(mut self, f: impl Fn(&str, &Detect) -> Option<PathBuf> + Send + Sync + 'static) -> Self {
        self.lookup = Box::new(f);
        self
    }

    /// Providers of enabled plugins whose own switch (`enabled_by`) is on.
    pub fn candidates(&self) -> Vec<Candidate> {
        let store = self.plugins.read();
        let mut out = Vec::new();
        for inst in store.enabled() {
            for p in &inst.manifest.providers {
                let on = match &p.enabled_by {
                    None => true,
                    Some(key) => store.setting(&inst.info.id, key).and_then(|v| v.as_bool()).unwrap_or(false),
                };
                if on {
                    out.push(Candidate {
                        plugin: inst.info.id.clone(),
                        provider: p.clone(),
                        detect: inst.manifest.detect.iter().find(|d| d.id == p.app).cloned(),
                    });
                }
            }
        }
        out
    }

    fn exe_of(&self, c: &Candidate) -> Result<PathBuf, ProviderError> {
        let detect = c.detect.as_ref().ok_or(ProviderError::NoExe)?;
        (self.lookup)(&c.plugin, detect).ok_or(ProviderError::NoExe)
    }

    fn chosen(&self, kind: ProviderKind) -> Option<Candidate> {
        let choice = Choice::load(&self.dir);
        active(&self.candidates(), kind, choice.get(kind)).cloned()
    }
}

impl ProviderHost for PluginProviders {
    fn pick(&self, req: &PickRequest) -> Option<Result<PickOutcome, String>> {
        let c = self.chosen(ProviderKind::FilePicker).filter(|c| serves(&c.provider, req.kind))?;
        let parent = (self.parent)();
        let result = self
            .exe_of(&c)
            .and_then(|exe| run_picker(&c.provider, &exe, req, parent.as_deref(), self.timeout));
        Some(result.map_err(|e| format!("{}: {e}", c.plugin)))
    }

    fn reveal(&self, path: &Path) -> Option<Result<(), String>> {
        let c = self.chosen(ProviderKind::Reveal)?;
        let result = self.exe_of(&c).and_then(|exe| run_reveal(&c.provider, &exe, path));
        Some(result.map_err(|e| format!("{}: {e}", c.plugin)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use photos_plugins::PickFilter;

    fn picker(args: &[&str]) -> Provider {
        Provider {
            kind: ProviderKind::FilePicker,
            app: "fx".into(),
            args: args.iter().map(|s| s.to_string()).collect(),
            modes: BTreeMap::from([
                (PickKind::OpenFile, "open".to_string()),
                (PickKind::OpenFolder, "folder".to_string()),
                (PickKind::Save, "save".to_string()),
            ]),
            output: Default::default(),
            enabled_by: None,
        }
    }

    fn req(kind: PickKind) -> PickRequest {
        PickRequest { kind, title: None, start: None, name: None, filters: vec![] }
    }

    const FULL: [&str; 11] = [
        "--pick", "{mode}", "--title", "{title}", "--start", "{start}", "--name", "{name}", "{filters...}", "--parent",
        "{parent}",
    ];

    #[test]
    fn expands_args() {
        let mut r = req(PickKind::Save);
        r.title = Some("Export".into());
        r.start = Some("C:/Pictures".into());
        r.name = Some("a.jpg".into());
        r.filters = vec![
            PickFilter { label: "Images".into(), extensions: vec!["jpg".into(), "png".into()] },
            PickFilter { label: "All".into(), extensions: vec!["*".into()] },
        ];
        let args = expand_picker_args(&picker(&FULL), &r, Some("1234")).unwrap();
        assert_eq!(
            args,
            [
                "--pick", "save", "--title", "Export", "--start", "C:/Pictures", "--name", "a.jpg", "--filter",
                "Images|*.jpg;*.png", "--filter", "All|*.*", "--parent", "1234"
            ]
        );
        // Unset values drop their pair; `--name` only exists for save; parent only where given.
        let mut open = req(PickKind::OpenFolder);
        open.name = Some("ignored".into());
        let args = expand_picker_args(&picker(&FULL), &open, None).unwrap();
        assert_eq!(args, ["--pick", "folder"]);
        // A kind with no word is not provided.
        assert!(!serves(&picker(&FULL), PickKind::OpenMultiple));
        assert_eq!(expand_picker_args(&picker(&FULL), &req(PickKind::OpenMultiple), None), Err(ProviderError::NotProvided));
        // Reveal: `{path}` whole, `{folder}` its parent.
        let reveal = Provider { kind: ProviderKind::Reveal, ..picker(&["--select", "{path}", "--in", "{folder}"]) };
        let args = expand_reveal_args(&reveal, Path::new("/a/b/c.jpg"));
        assert_eq!(args[0], "--select");
        assert_eq!(args[1], Path::new("/a/b/c.jpg").to_string_lossy());
        assert_eq!(args[3], Path::new("/a/b").to_string_lossy());
    }

    /// A stand-in for the picker app: prints `stdout`, exits with `code` after `sleep_secs`.
    fn fixture(dir: &Path, stdout: &str, code: i32, sleep_secs: u32) -> PathBuf {
        #[cfg(windows)]
        {
            let p = dir.join("picker.cmd");
            let mut s = String::from("@echo off\r\n");
            if sleep_secs > 0 {
                s += &format!("ping -n {} 127.0.0.1 >nul\r\n", sleep_secs + 1);
            }
            if !stdout.is_empty() {
                s += &format!("echo {stdout}\r\n");
            }
            s += &format!("exit /b {code}\r\n");
            std::fs::write(&p, s).unwrap();
            p
        }
        #[cfg(not(windows))]
        {
            use std::os::unix::fs::PermissionsExt;
            let p = dir.join("picker.sh");
            let mut s = String::from("#!/bin/sh\n");
            if sleep_secs > 0 {
                s += &format!("sleep {sleep_secs}\n");
            }
            if !stdout.is_empty() {
                s += &format!("echo '{stdout}'\n");
            }
            s += &format!("exit {code}\n");
            std::fs::write(&p, s).unwrap();
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
            p
        }
    }

    const ANSWER: &str = r#"{"paths":["/x/a.jpg","/x/b.jpg"],"filter":1}"#;
    const WAIT: Duration = Duration::from_secs(20);

    fn run(stdout: &str, code: i32, sleep: u32, timeout: Duration) -> Result<PickOutcome, ProviderError> {
        let dir = tempfile::tempdir().unwrap();
        let exe = fixture(dir.path(), stdout, code, sleep);
        run_picker(&picker(&FULL), &exe, &req(PickKind::OpenFile), None, timeout)
    }

    #[test]
    fn picked_paths_come_back() {
        assert_eq!(
            run(ANSWER, 0, 0, WAIT),
            Ok(PickOutcome::Picked { paths: vec!["/x/a.jpg".into(), "/x/b.jpg".into()], filter: Some(1) })
        );
    }

    #[test]
    fn exit_1_is_cancel() {
        assert_eq!(run("", 1, 0, WAIT), Ok(PickOutcome::Cancelled));
    }

    #[test]
    fn exit_2_falls_back() {
        assert!(matches!(run("", 2, 0, WAIT), Err(ProviderError::Exit(c)) if c == "2"));
    }

    #[test]
    fn bad_json_falls_back() {
        assert!(matches!(run("not json", 0, 0, WAIT), Err(ProviderError::BadOutput(_))));
        assert!(matches!(run(r#"{"paths":[]}"#, 0, 0, WAIT), Err(ProviderError::BadOutput(_))));
    }

    #[test]
    fn provider_timeout_falls_back() {
        let began = Instant::now();
        let r = run(ANSWER, 0, 8, Duration::from_millis(300));
        assert_eq!(r, Err(ProviderError::Timeout));
        assert!(began.elapsed() < Duration::from_secs(6), "the wait ended at the timeout, not the program");
    }

    #[test]
    fn missing_exe_is_a_spawn_failure() {
        let r = run_picker(&picker(&FULL), Path::new("/no/such/app"), &req(PickKind::OpenFile), None, WAIT);
        assert!(matches!(r, Err(ProviderError::Spawn(_))));
    }

    fn detect(exe: &str, dirs: Vec<String>) -> Detect {
        use photos_plugins::manifest::{DetectUnix, DetectWin};
        Detect {
            id: "fx".into(),
            name: "Fixture".into(),
            windows: Some(DetectWin { exe: exe.into(), paths: dirs.clone(), path_env: false, registry: vec![] }),
            linux: Some(DetectUnix { exe: exe.into(), paths: dirs, path_env: false }),
            macos: None,
            version: None,
        }
    }

    fn candidate(plugin: &str, kind: ProviderKind, detect: Option<Detect>) -> Candidate {
        Candidate { plugin: plugin.into(), provider: Provider { kind, ..picker(&FULL) }, detect }
    }

    #[test]
    fn only_detected_exe_launches() {
        let dir = tempfile::tempdir().unwrap();
        let exe = fixture(dir.path(), ANSWER, 0, 0);
        let name = exe.file_name().unwrap().to_str().unwrap();
        // Found through the declared folder: that file and no other.
        let found = detect::find_declared(&detect(name, vec![dir.path().to_string_lossy().into_owned()]));
        assert_eq!(found.as_deref(), Some(exe.as_path()));
        // A declared name that isn't there → nothing to launch.
        assert_eq!(detect::find_declared(&detect("nothing-here.exe", vec![dir.path().to_string_lossy().into_owned()])), None);
        // A provider whose `app` names no [[detect]] entry has no exe at all.
        let host = PluginProviders::new(
            Arc::new(Plugins::load(dir.path())),
            dir.path().to_path_buf(),
            || None,
        );
        assert_eq!(host.exe_of(&candidate("p", ProviderKind::FilePicker, None)), Err(ProviderError::NoExe));
    }

    #[test]
    fn choice_picks_the_chosen_plugin_else_the_first() {
        let cands = [
            candidate("a.one", ProviderKind::FilePicker, None),
            candidate("b.two", ProviderKind::FilePicker, None),
            candidate("c.reveal", ProviderKind::Reveal, None),
        ];
        let id = |c: Option<&Candidate>| c.map(|c| c.plugin.clone());
        assert_eq!(id(active(&cands, ProviderKind::FilePicker, None)), Some("a.one".into()));
        assert_eq!(id(active(&cands, ProviderKind::FilePicker, Some("b.two"))), Some("b.two".into()));
        assert_eq!(id(active(&cands, ProviderKind::FilePicker, Some("gone"))), Some("a.one".into()));
        assert_eq!(id(active(&cands, ProviderKind::Reveal, Some("b.two"))), Some("c.reveal".into()));
        assert_eq!(id(active(&[], ProviderKind::Reveal, None)), None);
        // The choice survives a restart.
        let dir = tempfile::tempdir().unwrap();
        let mut c = Choice::default();
        c.set(ProviderKind::FilePicker, "b.two");
        c.save(dir.path()).unwrap();
        assert_eq!(Choice::load(dir.path()).get(ProviderKind::FilePicker), Some("b.two"));
        assert_eq!(Choice::load(dir.path()).get(ProviderKind::Reveal), None);
    }
}
