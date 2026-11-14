//! The on-disk store under `config_dir/JL-Studios/Lumen/plugins/` (spec §5):
//!
//! ```text
//! plugins.json      per id {version, enabled, granted, key, trust, installedAt, dev, settings}
//!                   + {developerMode, devFolders}
//! installed/<id>/   the unpacked package; never written after install
//! data/<id>/        the plugin's `storage`
//! removed/<id>/     waiting for undo; deleted at next start
//! ```
//!
//! Installing unpacks to `installed/<n>.tmp/`, checks everything, then renames into place;
//! `plugins.json` is written to a temp file and renamed. Unknown fields in `plugins.json` (a
//! newer Lumen's) are kept on rewrite.

use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::manifest::{self, Manifest};
use crate::package::{self, KindCheck, Report, Unpacked};
use crate::signature::{self, Keys};
use crate::{
    host, theme, PluginAsset, PluginError, PluginInfo, PluginInspect, PluginPermission,
    PluginSetting, PluginState, PluginTrust, PluginUpdate, API_VERSION,
};

const STATE: &str = "plugins.json";
const STATE_TMP: &str = "plugins.json.tmp";
const KEY_CHANGED: &str = "This update was signed by a different key.";
const JL_PREFIX: &str = "studio.jl.";

/// What a check needs from the running Lumen.
pub struct Ctx {
    pub lumen: semver::Version,
    pub keys: Keys,
    /// The binary's checkers (presets, apps, prompts); the store adds the crate's own.
    pub checks: Vec<Box<dyn KindCheck>>,
}

/// A plugin the store has loaded.
pub struct Installed {
    pub info: PluginInfo,
    pub manifest: Manifest,
    pub dir: PathBuf,
    pub granted: Vec<PluginPermission>,
}

/// Undoes a [`Store::remove`] in this session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoveToken(pub String);

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StateFile {
    #[serde(default)]
    developer_mode: bool,
    /// Developer-mode folders by plugin id.
    #[serde(default)]
    dev_folders: BTreeMap<String, PathBuf>,
    #[serde(default)]
    plugins: BTreeMap<String, Entry>,
    #[serde(flatten)]
    extra: Map<String, Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Entry {
    version: String,
    enabled: bool,
    #[serde(default)]
    granted: Vec<PluginPermission>,
    /// The pinned signing key's id (hex); `None` for unsigned.
    #[serde(default)]
    key: Option<String>,
    #[serde(default = "unsigned")]
    trust: PluginTrust,
    installed_at: String,
    #[serde(default)]
    dev: bool,
    #[serde(default)]
    settings: Map<String, Value>,
    #[serde(flatten)]
    extra: Map<String, Value>,
}

fn unsigned() -> PluginTrust {
    PluginTrust::Unsigned
}

fn invalid(msg: impl Into<String>) -> PluginError {
    PluginError::invalid(msg)
}

fn now() -> String {
    chrono::Utc::now().to_rfc3339()
}

/// Whether this Lumen can run `m`: its API and its `lumen` range. The range is matched against
/// the release version without its pre-release tag, so `>=26.0.0` accepts `26.2.0-alpha`.
fn runs_on(m: &Manifest, lumen: &semver::Version) -> bool {
    let release = semver::Version::new(lumen.major, lumen.minor, lumen.patch);
    m.api <= API_VERSION && m.lumen.matches(&release)
}

/// What [`Store::prepare`] found about a package.
struct Prepared {
    trust: PluginTrust,
    key: Option<String>,
    report: Report,
    update: Option<PluginUpdate>,
    older: Option<String>,
}

pub struct Store {
    dir: PathBuf,
    lumen: semver::Version,
    state: StateFile,
    installed: BTreeMap<String, Installed>,
    /// Entries whose folder or manifest couldn't be read: id → reason.
    broken: BTreeMap<String, String>,
    /// Runtime only: id → why it stopped.
    stopped: HashMap<String, String>,
    /// Runtime only: id → the problem Lumen found running it (its tools don't match).
    problems: HashMap<String, String>,
    /// Runtime only: token → (id, entry, dev folder).
    removed: HashMap<String, (String, Entry, Option<PathBuf>)>,
}

impl Store {
    /// Opens the store in `dir` (`…/plugins/`): reads `plugins.json` and each plugin's
    /// `plugin.toml`, never unpacks or verifies. Leftovers of an interrupted install are removed.
    pub fn open(dir: &Path) -> Result<Store, PluginError> {
        fs::create_dir_all(dir.join("installed"))
            .map_err(|e| invalid(format!("The plugins folder couldn't be made: {e}")))?;
        clean_leftovers(dir);
        let state = match fs::read(dir.join(STATE)) {
            Ok(b) => serde_json::from_slice(&b)
                .map_err(|e| invalid(format!("plugins.json couldn't be read: {e}")))?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => StateFile::default(),
            Err(e) => return Err(invalid(format!("plugins.json couldn't be read: {e}"))),
        };
        let mut s = Store::empty(dir);
        s.state = state;
        let ids: Vec<String> = s.state.plugins.keys().cloned().collect();
        for id in ids {
            s.load_entry(&id);
        }
        Ok(s)
    }

    /// A store with nothing loaded and nothing read, for when [`Store::open`] fails: Lumen's
    /// boot never fails on plugins.
    pub fn empty(dir: &Path) -> Store {
        Store {
            dir: dir.to_path_buf(),
            lumen: env!("CARGO_PKG_VERSION")
                .parse()
                .expect("the crate version is semver"),
            state: StateFile::default(),
            installed: BTreeMap::new(),
            broken: BTreeMap::new(),
            stopped: HashMap::new(),
            problems: HashMap::new(),
            removed: HashMap::new(),
        }
    }

    /// Every plugin Developer mode lets load, by id.
    pub fn list(&self) -> Vec<PluginInfo> {
        let mut out: BTreeMap<&str, PluginInfo> = BTreeMap::new();
        for (id, i) in &self.installed {
            if self.visible(id) {
                out.insert(id, i.info.clone());
            }
        }
        for (id, reason) in &self.broken {
            if let Some(e) = self.state.plugins.get(id).filter(|_| self.visible(id)) {
                out.insert(id, broken_info(id, e, reason));
            }
        }
        out.into_values().collect()
    }

    pub fn get(&self, id: &str) -> Option<&Installed> {
        self.installed.get(id).filter(|_| self.visible(id))
    }

    /// Plugins that are on and ready to use.
    pub fn enabled(&self) -> Vec<&Installed> {
        self.installed
            .iter()
            .filter(|(id, i)| self.visible(id) && i.info.state == PluginState::Ready)
            .map(|(_, i)| i)
            .collect()
    }

    /// What the install sheet shows for `file`; nothing is installed.
    pub fn inspect(&self, file: &Path, ctx: &Ctx) -> Result<PluginInspect, PluginError> {
        let tmp = self.scratch()?;
        let r = (|| {
            let pkg = package::open_package(file, &tmp)?;
            let p = self.prepare(&pkg, ctx)?;
            let m = &pkg.manifest;
            let prev = self.state.plugins.get(&m.id);
            let entry = Entry {
                version: m.version.to_string(),
                enabled: true,
                granted: m.permissions.clone(),
                key: p.key.clone(),
                trust: p.trust.clone(),
                installed_at: String::new(),
                dev: false,
                settings: prev.map(|e| e.settings.clone()).unwrap_or_default(),
                extra: Map::new(),
            };
            Ok(PluginInspect {
                info: make_info(m, &entry, None, None, None, &ctx.lumen),
                adds: p.report.adds,
                problems: p.report.problems,
                notes: p.report.notes,
                update: p.update,
                older: p.older,
            })
        })();
        let _ = fs::remove_dir_all(&tmp);
        r
    }

    /// Installs or updates from `file`. Refused on any problem the sheet would show.
    pub fn install(&mut self, file: &Path, ctx: &Ctx) -> Result<PluginInfo, PluginError> {
        let tmp = self.scratch()?;
        let r = self.install_from(file, &tmp, ctx);
        if r.is_err() {
            let _ = fs::remove_dir_all(&tmp);
        }
        r
    }

    fn install_from(
        &mut self,
        file: &Path,
        tmp: &Path,
        ctx: &Ctx,
    ) -> Result<PluginInfo, PluginError> {
        let pkg = package::open_package(file, tmp)?;
        let p = self.prepare(&pkg, ctx)?;
        if !p.report.problems.is_empty() {
            return Err(invalid(p.report.problems.join("\n")));
        }
        let m = pkg.manifest;
        let id = m.id.clone();
        let installed = self.dir.join("installed");
        let dest = installed.join(&id);
        let old = installed.join(format!("{id}.old"));
        if dest.exists() {
            let _ = fs::remove_dir_all(&old);
            fs::rename(&dest, &old)
                .map_err(|e| invalid(format!("The installed version couldn't be moved: {e}")))?;
        }
        if let Err(e) = fs::rename(tmp, &dest) {
            if old.exists() {
                let _ = fs::rename(&old, &dest);
            }
            return Err(invalid(format!("The plugin couldn't be put in place: {e}")));
        }
        let _ = fs::remove_dir_all(&old);
        // A reinstall replaces a removal waiting for undo; its data stays.
        let _ = fs::remove_dir_all(self.dir.join("removed").join(&id));
        self.removed.retain(|_, r| r.0 != id);

        let prev = self.state.plugins.get(&id).filter(|e| !e.dev);
        let entry = Entry {
            version: m.version.to_string(),
            enabled: prev.map(|e| e.enabled).unwrap_or(true),
            granted: m.permissions.clone(),
            key: p.key,
            trust: p.trust,
            installed_at: now(),
            dev: false,
            settings: prev.map(|e| e.settings.clone()).unwrap_or_default(),
            extra: prev.map(|e| e.extra.clone()).unwrap_or_default(),
        };
        self.state.plugins.insert(id.clone(), entry);
        self.state.dev_folders.remove(&id);
        self.save()?;
        self.stopped.remove(&id);
        self.problems.remove(&id);
        self.load_entry(&id);
        self.info_of(&id)
    }

    pub fn set_enabled(&mut self, id: &str, on: bool) -> Result<(), PluginError> {
        self.entry_mut(id)?.enabled = on;
        self.save()?;
        self.refresh(id);
        Ok(())
    }

    pub fn developer_mode(&self) -> bool {
        self.state.developer_mode
    }

    /// Off unloads dev folders and unsigned plugins from every list; their files stay.
    pub fn set_developer_mode(&mut self, on: bool) -> Result<(), PluginError> {
        self.state.developer_mode = on;
        self.save()
    }

    pub fn asset_path(&self, id: &str, rel: &str) -> Option<PathBuf> {
        let i = self.get(id)?;
        i.manifest
            .assets
            .iter()
            .any(|a| a.path == rel)
            .then(|| i.dir.join(rel))
            .filter(|p| p.is_file())
    }

    pub fn icon_path(&self, id: &str) -> Option<PathBuf> {
        Some(self.get(id)?.dir.join("icon.png")).filter(|p| p.is_file())
    }

    /// `plugins/data/<id>/`, the plugin's `storage` (not created here).
    pub fn data_dir(&self, id: &str) -> PathBuf {
        self.dir.join("data").join(id)
    }

    /// Marks a plugin *Stopped working* until Lumen restarts (never persisted).
    pub fn mark_stopped(&mut self, id: &str, reason: String) {
        self.stopped.insert(id.to_string(), reason);
        self.refresh(id);
    }

    /// Marks a plugin *Problem* (`reason`) until it is installed or loaded again, or Lumen
    /// restarts (never persisted): A4's check that its `tools()` match its `plugin.toml`.
    pub fn mark_problem(&mut self, id: &str, reason: String) {
        self.problems.insert(id.to_string(), reason);
        self.refresh(id);
    }

    /// Moves the plugin to `removed/<id>/` (a dev folder is only forgotten; an empty
    /// `removed/<id>/` marks its data for the sweep). Undo with the token in this session.
    pub fn remove(&mut self, id: &str) -> Result<RemoveToken, PluginError> {
        self.entry_mut(id)?;
        let removed = self.dir.join("removed").join(id);
        let _ = fs::remove_dir_all(&removed);
        fs::create_dir_all(self.dir.join("removed"))
            .map_err(|e| invalid(format!("The plugin couldn't be removed: {e}")))?;
        let dev = self.state.plugins.get(id).is_some_and(|e| e.dev);
        let moved = if dev {
            fs::create_dir(&removed)
        } else {
            fs::rename(self.dir.join("installed").join(id), &removed)
        };
        moved.map_err(|e| invalid(format!("The plugin couldn't be removed: {e}")))?;
        let entry = self.state.plugins.remove(id).expect("checked above");
        let folder = self.state.dev_folders.remove(id);
        let token = RemoveToken(format!("{id}#{}", unique()));
        self.removed
            .insert(token.0.clone(), (id.to_string(), entry, folder));
        self.installed.remove(id);
        self.broken.remove(id);
        self.stopped.remove(id);
        self.problems.remove(id);
        self.save()?;
        Ok(token)
    }

    pub fn undo_remove(&mut self, token: &RemoveToken) -> Result<(), PluginError> {
        let (id, entry, folder) = self
            .removed
            .remove(&token.0)
            .ok_or_else(|| invalid("There's nothing to undo"))?;
        let removed = self.dir.join("removed").join(&id);
        let back = if entry.dev {
            fs::remove_dir_all(&removed)
        } else {
            fs::rename(&removed, self.dir.join("installed").join(&id))
        };
        if let Err(e) = back {
            self.removed.insert(token.0.clone(), (id, entry, folder));
            return Err(invalid(format!("The plugin couldn't be put back: {e}")));
        }
        if let Some(f) = folder {
            self.state.dev_folders.insert(id.clone(), f);
        }
        self.state.plugins.insert(id.clone(), entry);
        self.save()?;
        self.load_entry(&id);
        Ok(())
    }

    /// At start, before [`Store::open`]: deletes every `removed/<id>/` and the `data/<id>/` of
    /// ids no longer in `plugins.json` (a reinstalled id keeps its data).
    pub fn sweep_removed(dir: &Path) {
        let Ok(rd) = fs::read_dir(dir.join("removed")) else {
            return;
        };
        // Unreadable state → keep every data folder (deleting is the irreversible part).
        let known: Option<StateFile> = fs::read(dir.join(STATE))
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .or_else(|| (!dir.join(STATE).exists()).then(StateFile::default));
        for e in rd.flatten() {
            let _ = fs::remove_dir_all(e.path());
            let Some(id) = e.file_name().to_str().map(str::to_string) else {
                continue;
            };
            if known.as_ref().is_some_and(|s| !s.plugins.contains_key(&id)) {
                let _ = fs::remove_dir_all(dir.join("data").join(&id));
            }
        }
    }

    pub fn set_setting(&mut self, id: &str, key: &str, value: Value) -> Result<(), PluginError> {
        let decl = self
            .get(id)
            .ok_or_else(|| invalid(format!("No plugin {id}")))?
            .manifest
            .settings
            .iter()
            .find(|d| d.key == key)
            .ok_or_else(|| invalid(format!("{key} isn't a setting of {id}")))?;
        if !decl.accepts(&value) {
            return Err(invalid(format!(
                "{key} needs a {} value",
                match decl.kind {
                    crate::SettingKind::Bool => "true or false",
                    crate::SettingKind::Number => "number",
                    crate::SettingKind::Text => "text",
                    crate::SettingKind::Choice => "listed",
                }
            )));
        }
        self.entry_mut(id)?.settings.insert(key.to_string(), value);
        self.save()?;
        self.refresh(id);
        Ok(())
    }

    /// The stored value, else the manifest's default; `None` for an undeclared key.
    pub fn setting(&self, id: &str, key: &str) -> Option<Value> {
        self.get(id)?
            .info
            .settings
            .iter()
            .find(|s| s.key == key)
            .map(|s| s.value.clone())
    }

    /// Loads an unpacked plugin folder (Developer mode): the same checks, read in place.
    pub fn load_dev(&mut self, folder: &Path, ctx: &Ctx) -> Result<PluginInfo, PluginError> {
        if !self.state.developer_mode {
            return Err(PluginError::unsigned(
                "Turn on Developer mode to load a plugin folder.",
            ));
        }
        let pkg = package::open_folder(folder)?;
        let id = pkg.manifest.id.clone();
        let prev = self.state.plugins.get(&id);
        if prev.is_some_and(|e| !e.dev) {
            return Err(invalid(format!(
                "{} is already installed. Remove it before loading its folder.",
                pkg.manifest.name
            )));
        }
        let p = self.prepare(&pkg, ctx)?;
        if !p.report.problems.is_empty() {
            return Err(invalid(p.report.problems.join("\n")));
        }
        let entry = Entry {
            version: pkg.manifest.version.to_string(),
            enabled: prev.map(|e| e.enabled).unwrap_or(true),
            granted: pkg.manifest.permissions.clone(),
            key: p.key,
            trust: p.trust,
            installed_at: prev.map(|e| e.installed_at.clone()).unwrap_or_else(now),
            dev: true,
            settings: prev.map(|e| e.settings.clone()).unwrap_or_default(),
            extra: prev.map(|e| e.extra.clone()).unwrap_or_default(),
        };
        self.state.plugins.insert(id.clone(), entry);
        self.state
            .dev_folders
            .insert(id.clone(), folder.to_path_buf());
        self.save()?;
        self.stopped.remove(&id);
        self.problems.remove(&id);
        self.load_entry(&id);
        self.info_of(&id)
    }

    /// Re-reads a dev folder (its id must not change).
    pub fn reload_dev(&mut self, id: &str, ctx: &Ctx) -> Result<PluginInfo, PluginError> {
        let folder = self
            .state
            .dev_folders
            .get(id)
            .filter(|_| self.visible(id))
            .cloned()
            .ok_or_else(|| invalid(format!("{id} isn't a loaded folder")))?;
        let m = fs::read_to_string(folder.join("plugin.toml"))
            .map_err(|e| invalid(format!("plugin.toml couldn't be read: {e}")))
            .and_then(|t| manifest::parse(&t))?;
        if m.id != id {
            return Err(invalid(format!(
                "The folder's plugin.toml now names {}, not {id}",
                m.id
            )));
        }
        self.load_dev(&folder, ctx)
    }

    // ---- internals ----

    /// Signature, trust, pinned key, kind checks, versions. `Err` for what no sheet may offer
    /// to install (bad or missing signature, a changed key); `report.problems` for the rest.
    fn prepare(&self, pkg: &Unpacked, ctx: &Ctx) -> Result<Prepared, PluginError> {
        let m = &pkg.manifest;
        let dev_mode = self.state.developer_mode;
        let (trust, key) = signature::verify(pkg, &ctx.keys)?;
        if trust == PluginTrust::Unsigned && !dev_mode {
            return Err(PluginError::unsigned(
                "This plugin isn't signed. Unsigned plugins install only in Developer mode.",
            ));
        }
        let studio = m.id.starts_with(JL_PREFIX);
        let jl = trust == PluginTrust::JlStudios;
        if studio && ctx.keys.jl.is_some() && !jl && !dev_mode {
            return Err(PluginError::signature(format!(
                "Ids starting with {JL_PREFIX} belong to JL Studios, and this package isn't signed by JL Studios."
            )));
        }
        let prev = self.state.plugins.get(&m.id).filter(|e| !e.dev);
        if let Some(old) = prev.and_then(|e| e.key.as_ref()) {
            if key.as_ref() != Some(old) && !(studio && jl) {
                return Err(PluginError::key_changed(KEY_CHANGED));
            }
        }

        let mut report = Report::default();
        let builtin: [&dyn KindCheck; 2] = [&theme::ThemeCheck, &host::ImportsCheck];
        let checks: Vec<&dyn KindCheck> = builtin
            .into_iter()
            .chain(ctx.checks.iter().map(|c| c.as_ref()))
            .collect();
        for k in m.kinds() {
            if !checks.iter().any(|c| c.kinds().contains(&k)) {
                report
                    .problems
                    .push(format!("{} aren't supported by this Lumen", k.plural()));
            }
        }
        for c in &checks {
            c.check(pkg, dev_mode, &mut report);
        }
        if !runs_on(m, &ctx.lumen) {
            report.notes.push(
                "This plugin needs a newer Lumen. It stays off until Lumen is updated.".into(),
            );
        }

        let (mut update, mut older) = (None, None);
        if let Some(e) = prev {
            if let Ok(v) = semver::Version::parse(&e.version) {
                if m.version < v {
                    report
                        .problems
                        .push(format!("Older than the installed {}", e.version));
                    older = Some(e.version.clone());
                } else if m.version > v {
                    update = Some(PluginUpdate {
                        from: e.version.clone(),
                        new_permissions: m
                            .permissions
                            .iter()
                            .filter(|p| !e.granted.contains(p))
                            .copied()
                            .collect(),
                    });
                }
            }
        }
        Ok(Prepared {
            trust,
            key,
            report,
            update,
            older,
        })
    }

    /// Developer mode shows everything; otherwise dev folders and unsigned plugins are unloaded.
    fn visible(&self, id: &str) -> bool {
        self.state.developer_mode
            || self
                .state
                .plugins
                .get(id)
                .is_some_and(|e| !e.dev && e.trust != PluginTrust::Unsigned)
    }

    fn entry_mut(&mut self, id: &str) -> Result<&mut Entry, PluginError> {
        if !self.visible(id) {
            return Err(invalid(format!("No plugin {id}")));
        }
        self.state
            .plugins
            .get_mut(id)
            .ok_or_else(|| invalid(format!("No plugin {id}")))
    }

    fn info_of(&self, id: &str) -> Result<PluginInfo, PluginError> {
        match (self.installed.get(id), self.broken.get(id)) {
            (Some(i), _) => Ok(i.info.clone()),
            (None, Some(r)) => Err(invalid(r.clone())),
            _ => Err(invalid(format!("No plugin {id}"))),
        }
    }

    /// (Re)reads one entry's `plugin.toml` into `installed`, or records why it can't.
    fn load_entry(&mut self, id: &str) {
        let Some(e) = self.state.plugins.get(id) else {
            return;
        };
        let dir = if e.dev {
            match self.state.dev_folders.get(id) {
                Some(f) => f.clone(),
                None => {
                    self.installed.remove(id);
                    self.broken
                        .insert(id.into(), "The folder of this plugin isn't known".into());
                    return;
                }
            }
        } else {
            self.dir.join("installed").join(id)
        };
        let read = fs::read_to_string(dir.join("plugin.toml"))
            .map_err(|e| e.to_string())
            .and_then(|t| manifest::parse(&t).map_err(|e| e.message));
        match read {
            Ok(m) if m.id == id => {
                let granted = e.granted.clone();
                let placeholder = broken_info(id, e, "");
                self.broken.remove(id);
                self.installed.insert(
                    id.into(),
                    Installed {
                        info: placeholder,
                        manifest: m,
                        dir,
                        granted,
                    },
                );
                self.refresh(id);
            }
            other => {
                let reason = match other {
                    Ok(m) => format!("Its plugin.toml names another id ({})", m.id),
                    Err(r) => format!("This plugin couldn't be read: {r}"),
                };
                self.installed.remove(id);
                self.broken.insert(id.into(), reason);
            }
        }
    }

    /// Recomputes `installed[id].info` from the manifest, the entry and the runtime state.
    fn refresh(&mut self, id: &str) {
        let (Some(i), Some(e)) = (self.installed.get(id), self.state.plugins.get(id)) else {
            return;
        };
        let icon = i
            .dir
            .join("icon.png")
            .is_file()
            .then(|| format!("photos://plugin-icon/{id}"));
        let info = make_info(&i.manifest, e, icon, self.stopped.get(id), self.problems.get(id), &self.lumen);
        if let Some(i) = self.installed.get_mut(id) {
            i.granted = e.granted.clone();
            i.info = info;
        }
    }

    fn save(&self) -> Result<(), PluginError> {
        let bytes = serde_json::to_vec_pretty(&self.state)
            .map_err(|e| invalid(format!("plugins.json couldn't be written: {e}")))?;
        let tmp = self.dir.join(STATE_TMP);
        fs::write(&tmp, bytes)
            .and_then(|_| fs::rename(&tmp, self.dir.join(STATE)))
            .map_err(|e| invalid(format!("plugins.json couldn't be written: {e}")))
    }

    /// A fresh `installed/<n>.tmp/` path (removed by `open` if left behind).
    fn scratch(&self) -> Result<PathBuf, PluginError> {
        let p = self
            .dir
            .join("installed")
            .join(format!(".incoming-{}.tmp", unique()));
        let _ = fs::remove_dir_all(&p);
        Ok(p)
    }
}

/// Unique within this process: the time in nanoseconds and a counter.
fn unique() -> String {
    static N: AtomicU64 = AtomicU64::new(0);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("{nanos}-{}", N.fetch_add(1, Ordering::Relaxed))
}

fn make_info(
    m: &Manifest,
    e: &Entry,
    icon_url: Option<String>,
    stopped: Option<&String>,
    problem: Option<&String>,
    lumen: &semver::Version,
) -> PluginInfo {
    let state = if !runs_on(m, lumen) {
        PluginState::NeedsNewerLumen
    } else if !e.enabled {
        PluginState::Off
    } else if let Some(r) = stopped {
        PluginState::Stopped { reason: r.clone() }
    } else if let Some(r) = problem {
        PluginState::Problem { reason: r.clone() }
    } else {
        PluginState::Ready
    };
    PluginInfo {
        id: m.id.clone(),
        name: m.name.clone(),
        version: m.version.to_string(),
        author: m.author.clone(),
        description: m.description.clone(),
        license: m.license.clone(),
        trust: e.trust.clone(),
        kinds: m.kinds(),
        permissions: m.permissions.clone(),
        enabled: e.enabled,
        dev: e.dev,
        state,
        icon_url,
        settings: m
            .settings
            .iter()
            .map(|d| PluginSetting {
                key: d.key.clone(),
                label: d.label.clone(),
                kind: d.kind,
                value: e
                    .settings
                    .get(&d.key)
                    .filter(|v| d.accepts(v))
                    .cloned()
                    .unwrap_or_else(|| d.default.clone()),
                choices: d.choices.clone(),
            })
            .collect(),
        assets: m
            .assets
            .iter()
            .map(|a| PluginAsset {
                path: a.path.clone(),
                license: a.license.clone(),
            })
            .collect(),
    }
}

fn broken_info(id: &str, e: &Entry, reason: &str) -> PluginInfo {
    PluginInfo {
        id: id.into(),
        name: id.into(),
        version: e.version.clone(),
        author: String::new(),
        description: String::new(),
        license: String::new(),
        trust: e.trust.clone(),
        kinds: Vec::new(),
        permissions: e.granted.clone(),
        enabled: e.enabled,
        dev: e.dev,
        state: PluginState::Problem {
            reason: reason.into(),
        },
        icon_url: None,
        settings: Vec::new(),
        assets: Vec::new(),
    }
}

/// Removes what an interrupted install left: `installed/*.tmp/`, `plugins.json.tmp`, and an
/// `installed/<id>.old/` (put back if the new folder never arrived).
fn clean_leftovers(dir: &Path) {
    let _ = fs::remove_file(dir.join(STATE_TMP));
    let Ok(rd) = fs::read_dir(dir.join("installed")) else {
        return;
    };
    for e in rd.flatten() {
        let path = e.path();
        let Some(name) = e.file_name().to_str().map(str::to_string) else {
            continue;
        };
        if name.ends_with(".tmp") {
            let _ = fs::remove_dir_all(&path).or_else(|_| fs::remove_file(&path));
        } else if let Some(id) = name.strip_suffix(".old") {
            let live = dir.join("installed").join(id);
            if live.exists() {
                let _ = fs::remove_dir_all(&path);
            } else {
                let _ = fs::rename(&path, &live);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::package::fixture::{manifest, pack, png};
    use crate::signature::fixture::TestKey;
    use crate::PluginKind::{self, *};
    use tempfile::TempDir;

    struct Claim {
        kinds: &'static [PluginKind],
        problem: Option<&'static str>,
        note: Option<&'static str>,
    }

    impl KindCheck for Claim {
        fn kinds(&self) -> &'static [PluginKind] {
            self.kinds
        }
        fn check(&self, _: &Unpacked, _: bool, r: &mut Report) {
            r.problems.extend(self.problem.map(String::from));
            r.notes.extend(self.note.map(String::from));
        }
    }

    const ALL: &[PluginKind] = &[Theme, Presets, Apps, Prompts, Importer, Tools];

    fn claim(kinds: &'static [PluginKind]) -> Box<dyn KindCheck> {
        Box::new(Claim {
            kinds,
            problem: None,
            note: None,
        })
    }

    fn ctx_with(jl: Option<&TestKey>, checks: Vec<Box<dyn KindCheck>>) -> Ctx {
        Ctx {
            // A pre-release, as Lumen's own versions are; BASE's `>=26.0.0` must still match.
            lumen: "26.1.1-alpha".parse().unwrap(),
            keys: Keys::from_base64(jl.map(|k| k.pub_b64()).as_deref()).unwrap(),
            checks,
        }
    }

    /// The one theme every test plugin carries; the real `ThemeCheck` reads it.
    const THEME_A: &[u8] = b"id = \"a\"\nname = \"A\"\nbase = \"gallery\"\n";

    fn ctx() -> Ctx {
        ctx_with(None, vec![claim(ALL)])
    }

    enum Sign<'a> {
        No,
        Author(&'a TestKey),
        Jl(&'a TestKey),
    }

    /// A theme plugin's manifest; `extra` goes before `[[themes]]` (top-level keys first).
    fn theme_toml(id: &str, version: &str, extra: &str) -> String {
        manifest(&format!("{extra}\n[[themes]]\nfile = \"themes/a.toml\""))
            .replace("com.test.x", id)
            .replace("version = \"1.0.0\"", &format!("version = \"{version}\""))
    }

    fn package(m: &str, more: &[(&str, &[u8])], sign: Sign) -> (TempDir, PathBuf) {
        let mut files: Vec<(&str, &[u8])> =
            vec![("plugin.toml", m.as_bytes()), ("themes/a.toml", THEME_A)];
        files.extend_from_slice(more);
        let sig = match sign {
            Sign::No => None,
            Sign::Author(k) => Some(k.sign_entries(&files)),
            Sign::Jl(k) => Some(k.sign_entries_as_jl(&files)),
        };
        let mut entries = files.clone();
        if let Some(s) = &sig {
            entries.push(("SIGNATURE", s.as_bytes()));
        }
        pack(&entries)
    }

    fn signed(id: &str, version: &str, extra: &str, k: &TestKey) -> (TempDir, PathBuf) {
        package(&theme_toml(id, version, extra), &[], Sign::Author(k))
    }

    fn app() -> (TempDir, PathBuf) {
        let t = tempfile::tempdir().unwrap();
        let d = t.path().join("plugins");
        (t, d)
    }

    fn state_json(dir: &Path) -> Value {
        serde_json::from_slice(&fs::read(dir.join(STATE)).unwrap()).unwrap()
    }

    #[test]
    fn install_then_list() {
        let (_a, dir) = app();
        let k = TestKey::generate();
        let mut s = Store::open(&dir).unwrap();
        let (_t, p) = signed("com.test.x", "1.0.0", "", &k);
        let info = s.install(&p, &ctx()).unwrap();
        assert!(dir.join("installed/com.test.x/plugin.toml").is_file());
        let e = &state_json(&dir)["plugins"]["com.test.x"];
        assert_eq!(e["version"], "1.0.0");
        assert_eq!(e["enabled"], true);
        assert_eq!(e["granted"], serde_json::json!([]));
        assert!(e["key"].is_string());
        assert!(e["installedAt"].is_string());
        assert_eq!(e["dev"], false);
        assert_eq!(e["settings"], serde_json::json!({}));
        assert!(matches!(info.trust, PluginTrust::Author { .. }));
        assert_eq!(s.list()[0].state, PluginState::Ready);
        assert_eq!(s.list(), Store::open(&dir).unwrap().list());
        // Nothing left in installed/ but the plugin.
        let left: Vec<_> = fs::read_dir(dir.join("installed")).unwrap().collect();
        assert_eq!(left.len(), 1);
    }

    #[test]
    fn unclaimed_kind_fails_closed() {
        let (_a, dir) = app();
        let k = TestKey::generate();
        let mut s = Store::open(&dir).unwrap();
        // Apps are claimed by the binary's data checks, never by this crate's own.
        let m = manifest("[apps]\nfile = \"apps.toml\"");
        let (_t, p) = package(&m, &[("apps.toml", b"")], Sign::Author(&k));
        let c = ctx_with(None, vec![]);
        let i = s.inspect(&p, &c).unwrap();
        assert_eq!(i.problems, vec!["Apps aren't supported by this Lumen"]);
        assert_eq!(s.install(&p, &c).unwrap_err().code, "plugin_invalid");
        assert!(s.list().is_empty());
        assert_eq!(fs::read_dir(dir.join("installed")).unwrap().count(), 0);
    }

    #[test]
    fn checker_problems_block_install() {
        let (_a, dir) = app();
        let k = TestKey::generate();
        let mut s = Store::open(&dir).unwrap();
        let (_t, p) = signed("com.test.x", "1.0.0", "", &k);
        let bad = |problem, note| {
            ctx_with(
                None,
                vec![
                    claim(&[Theme]),
                    Box::new(Claim {
                        kinds: &[Presets],
                        problem,
                        note,
                    }),
                ],
            )
        };
        let e = s.install(&p, &bad(Some("Bad preset"), None)).unwrap_err();
        assert!(e.message.contains("Bad preset"), "{e}");
        let c = bad(None, Some("A note"));
        assert_eq!(s.inspect(&p, &c).unwrap().notes, vec!["A note"]);
        s.install(&p, &c).unwrap();
    }

    #[test]
    fn unsigned_needs_developer_mode() {
        let (_a, dir) = app();
        let mut s = Store::open(&dir).unwrap();
        let (_t, p) = package(&theme_toml("com.test.x", "1.0.0", ""), &[], Sign::No);
        assert_eq!(s.install(&p, &ctx()).unwrap_err().code, "plugin_unsigned");
        s.set_developer_mode(true).unwrap();
        assert_eq!(s.install(&p, &ctx()).unwrap().trust, PluginTrust::Unsigned);
    }

    #[test]
    fn key_pinned_on_update() {
        let (_a, dir) = app();
        let (a, b) = (TestKey::generate(), TestKey::generate());
        let mut s = Store::open(&dir).unwrap();
        s.install(&signed("com.test.x", "1.0.0", "", &a).1, &ctx())
            .unwrap();
        let (_t, p) = signed("com.test.x", "1.1.0", "", &b);
        let e = s.install(&p, &ctx()).unwrap_err();
        assert_eq!(e.code, "plugin_key_changed");
        assert_eq!(e.message, "This update was signed by a different key.");
        assert_eq!(s.list()[0].version, "1.0.0");
    }

    #[test]
    fn jl_key_may_repin_studio_ids_only() {
        let (_a, dir) = app();
        let (a, jl) = (TestKey::generate(), TestKey::generate());
        let mut s = Store::open(&dir).unwrap();
        let jl_ctx = ctx_with(Some(&jl), vec![claim(ALL)]);
        for id in ["studio.jl.x", "com.other.x"] {
            // First signed by a local author key, before the JL key was built in.
            let (_t, p) = signed(id, "1.0.0", "", &a);
            s.install(&p, &ctx()).unwrap();
            let (_t, p) = package(&theme_toml(id, "1.1.0", ""), &[], Sign::Jl(&jl));
            let r = s.install(&p, &jl_ctx);
            if id == "studio.jl.x" {
                assert_eq!(r.unwrap().trust, PluginTrust::JlStudios);
            } else {
                assert_eq!(r.unwrap_err().code, "plugin_key_changed");
            }
        }
    }

    #[test]
    fn studio_ids_need_jl_key() {
        let (_a, dir) = app();
        let (a, jl) = (TestKey::generate(), TestKey::generate());
        let mut s = Store::open(&dir).unwrap();
        let (_t, p) = signed("studio.jl.x", "1.0.0", "", &a);
        let jl_ctx = ctx_with(Some(&jl), vec![claim(ALL)]);
        assert_eq!(s.install(&p, &jl_ctx).unwrap_err().code, "plugin_signature");
        assert!(matches!(
            s.install(&p, &ctx()).unwrap().trust,
            PluginTrust::Author { .. }
        ));
    }

    #[test]
    fn update_reports_new_permissions() {
        let (_a, dir) = app();
        let k = TestKey::generate();
        let mut s = Store::open(&dir).unwrap();
        s.install(&signed("com.test.x", "1.0.0", "", &k).1, &ctx())
            .unwrap();
        let (_t, p) = signed("com.test.x", "1.1.0", "permissions = [\"storage\"]", &k);
        let i = s.inspect(&p, &ctx()).unwrap();
        assert_eq!(
            i.update,
            Some(PluginUpdate {
                from: "1.0.0".into(),
                new_permissions: vec![PluginPermission::Storage],
            })
        );
        assert_eq!(i.older, None);
    }

    #[test]
    fn older_version_flagged() {
        let (_a, dir) = app();
        let k = TestKey::generate();
        let mut s = Store::open(&dir).unwrap();
        s.install(&signed("com.test.x", "1.1.0", "", &k).1, &ctx())
            .unwrap();
        let (_t, p) = signed("com.test.x", "1.0.0", "", &k);
        let i = s.inspect(&p, &ctx()).unwrap();
        assert_eq!(i.older.as_deref(), Some("1.1.0"));
        assert!(i
            .problems
            .contains(&"Older than the installed 1.1.0".to_string()));
        assert_eq!(i.update, None);
        assert_eq!(s.install(&p, &ctx()).unwrap_err().code, "plugin_invalid");
        // The same version reinstalls (a repair).
        let (_t, same) = signed("com.test.x", "1.1.0", "", &k);
        assert_eq!(s.inspect(&same, &ctx()).unwrap().older, None);
        s.install(&same, &ctx()).unwrap();
    }

    #[test]
    fn update_keeps_data() {
        let (_a, dir) = app();
        let k = TestKey::generate();
        let mut s = Store::open(&dir).unwrap();
        s.install(&signed("com.test.x", "1.0.0", "", &k).1, &ctx())
            .unwrap();
        let data = s.data_dir("com.test.x");
        fs::create_dir_all(&data).unwrap();
        fs::write(data.join("x"), b"kept").unwrap();
        s.install(&signed("com.test.x", "1.1.0", "", &k).1, &ctx())
            .unwrap();
        assert_eq!(fs::read(data.join("x")).unwrap(), b"kept");
        assert_eq!(s.list()[0].version, "1.1.0");
        assert!(!dir.join("installed/com.test.x.old").exists());
    }

    #[test]
    fn needs_newer_lumen() {
        let (_a, dir) = app();
        let k = TestKey::generate();
        let mut s = Store::open(&dir).unwrap();
        for (id, from, to) in [
            ("com.test.api", "api = 1", "api = 2"),
            (
                "com.test.range",
                "lumen = \">=26.0.0\"",
                "lumen = \">=99.0.0\"",
            ),
        ] {
            let m = theme_toml(id, "1.0.0", "").replace(from, to);
            let (_t, p) = package(&m, &[], Sign::Author(&k));
            assert!(!s.inspect(&p, &ctx()).unwrap().notes.is_empty());
            assert_eq!(
                s.install(&p, &ctx()).unwrap().state,
                PluginState::NeedsNewerLumen
            );
        }
        assert!(s.enabled().is_empty());
        assert_eq!(s.list().len(), 2);
    }

    #[test]
    fn asset_path_only_manifest_assets() {
        let (_a, dir) = app();
        let k = TestKey::generate();
        let mut s = Store::open(&dir).unwrap();
        let m = theme_toml(
            "com.test.x",
            "1.0.0",
            "[[assets]]\npath = \"assets/fonts/Lora.woff2\"\nlicense = \"OFL-1.1\"",
        );
        let icon = png(64, 64);
        let (_t, p) = package(
            &m,
            &[("assets/fonts/Lora.woff2", b"wOF2"), ("icon.png", &icon)],
            Sign::Author(&k),
        );
        let info = s.install(&p, &ctx()).unwrap();
        assert_eq!(
            info.icon_url.as_deref(),
            Some("photos://plugin-icon/com.test.x")
        );
        assert!(s.icon_path("com.test.x").is_some());
        assert!(s
            .asset_path("com.test.x", "assets/fonts/Lora.woff2")
            .is_some_and(|p| p.is_file()));
        assert_eq!(s.asset_path("com.test.x", "plugin.toml"), None);
        assert_eq!(s.asset_path("com.test.x", "../x"), None);
        assert_eq!(s.asset_path("com.none", "assets/fonts/Lora.woff2"), None);
    }

    #[test]
    fn leftover_tmp_ignored() {
        let (_a, dir) = app();
        let k = TestKey::generate();
        let mut s = Store::open(&dir).unwrap();
        s.install(&signed("com.test.x", "1.0.0", "", &k).1, &ctx())
            .unwrap();
        let before = s.list();
        fs::create_dir_all(dir.join("installed/x.tmp/themes")).unwrap();
        fs::write(dir.join("installed/x.tmp/plugin.toml"), b"half").unwrap();
        fs::write(dir.join("plugins.json.tmp"), b"{ half").unwrap();
        let s = Store::open(&dir).unwrap();
        assert_eq!(s.list(), before);
        assert!(!dir.join("installed/x.tmp").exists());
        assert!(!dir.join("plugins.json.tmp").exists());
    }

    #[test]
    fn interrupted_update_puts_old_back() {
        let (_a, dir) = app();
        let k = TestKey::generate();
        let mut s = Store::open(&dir).unwrap();
        s.install(&signed("com.test.x", "1.0.0", "", &k).1, &ctx())
            .unwrap();
        // A crash after the old folder moved aside, before the new one arrived.
        fs::rename(
            dir.join("installed/com.test.x"),
            dir.join("installed/com.test.x.old"),
        )
        .unwrap();
        let s = Store::open(&dir).unwrap();
        assert_eq!(s.list()[0].state, PluginState::Ready);
        assert!(!dir.join("installed/com.test.x.old").exists());
    }

    #[test]
    fn unknown_state_fields_kept() {
        let (_a, dir) = app();
        let k = TestKey::generate();
        let mut s = Store::open(&dir).unwrap();
        s.install(&signed("com.test.x", "1.0.0", "", &k).1, &ctx())
            .unwrap();
        let mut j = state_json(&dir);
        j["futureTop"] = serde_json::json!(1);
        j["plugins"]["com.test.x"]["futureField"] = serde_json::json!("x");
        fs::write(dir.join(STATE), serde_json::to_vec(&j).unwrap()).unwrap();
        let mut s = Store::open(&dir).unwrap();
        s.set_enabled("com.test.x", false).unwrap();
        let j = state_json(&dir);
        assert_eq!(j["futureTop"], 1);
        assert_eq!(j["plugins"]["com.test.x"]["futureField"], "x");
        assert_eq!(j["plugins"]["com.test.x"]["enabled"], false);
    }

    // ---- Task 7 ----

    fn installed_one(dir: &Path, extra: &str) -> Store {
        let k = TestKey::generate();
        let mut s = Store::open(dir).unwrap();
        s.install(&signed("com.test.x", "1.0.0", extra, &k).1, &ctx())
            .unwrap();
        s
    }

    #[test]
    fn toggle() {
        let (_a, dir) = app();
        let mut s = installed_one(&dir, "");
        s.set_enabled("com.test.x", false).unwrap();
        assert_eq!(s.list()[0].state, PluginState::Off);
        assert!(!s.list()[0].enabled);
        assert!(s.enabled().is_empty());
        assert_eq!(Store::open(&dir).unwrap().list()[0].state, PluginState::Off);
        s.set_enabled("com.test.x", true).unwrap();
        assert_eq!(s.enabled().len(), 1);
        assert_eq!(
            s.set_enabled("com.none", true).unwrap_err().code,
            "plugin_invalid"
        );
    }

    #[test]
    fn remove_and_undo() {
        let (_a, dir) = app();
        let mut s = installed_one(&dir, "");
        let before = s.list();
        let toml = fs::read(dir.join("installed/com.test.x/plugin.toml")).unwrap();
        let t = s.remove("com.test.x").unwrap();
        assert!(dir.join("removed/com.test.x/plugin.toml").is_file());
        assert!(!dir.join("installed/com.test.x").exists());
        assert!(s.list().is_empty());
        assert!(Store::open(&dir).unwrap().list().is_empty());
        s.undo_remove(&t).unwrap();
        assert_eq!(s.list(), before);
        assert_eq!(
            fs::read(dir.join("installed/com.test.x/plugin.toml")).unwrap(),
            toml
        );
        assert_eq!(Store::open(&dir).unwrap().list(), before);
        assert_eq!(s.undo_remove(&t).unwrap_err().code, "plugin_invalid");
    }

    #[test]
    fn sweep_deletes_removed_and_data() {
        let (_a, dir) = app();
        let mut s = installed_one(&dir, "");
        let data = s.data_dir("com.test.x");
        fs::create_dir_all(&data).unwrap();
        fs::write(data.join("x"), b"x").unwrap();
        s.remove("com.test.x").unwrap();
        drop(s);
        Store::sweep_removed(&dir);
        assert!(!dir.join("removed/com.test.x").exists());
        assert!(!data.exists());
    }

    #[test]
    fn sweep_keeps_data_of_a_reinstalled_id() {
        let (_a, dir) = app();
        let mut s = installed_one(&dir, "");
        let data = s.data_dir("com.test.x");
        fs::create_dir_all(&data).unwrap();
        s.remove("com.test.x").unwrap();
        // Reinstalled before the sweep: the data belongs to it again, even if a removed/
        // folder of the same id is still there.
        let k = TestKey::generate();
        s.install(&signed("com.test.x", "1.0.0", "", &k).1, &ctx())
            .unwrap();
        fs::create_dir_all(dir.join("removed/com.test.x")).unwrap();
        Store::sweep_removed(&dir);
        assert!(data.exists());
        assert!(!dir.join("removed/com.test.x").exists());
    }

    const SETTING: &str = "[[settings]]\nkey = \"albums_from_folders\"\nlabel = \"Use Takeout's folders as albums\"\ntype = \"bool\"\ndefault = true";

    #[test]
    fn settings_defaults_and_set() {
        let (_a, dir) = app();
        let mut s = installed_one(&dir, SETTING);
        let key = "albums_from_folders";
        assert_eq!(s.setting("com.test.x", key), Some(Value::Bool(true)));
        assert_eq!(s.list()[0].settings[0].value, Value::Bool(true));
        s.set_setting("com.test.x", key, Value::Bool(false))
            .unwrap();
        let s2 = Store::open(&dir).unwrap();
        assert_eq!(s2.setting("com.test.x", key), Some(Value::Bool(false)));
        assert_eq!(s2.list()[0].settings[0].value, Value::Bool(false));
        let e = s
            .set_setting("com.test.x", key, Value::from("no"))
            .unwrap_err();
        assert_eq!(e.code, "plugin_invalid");
        let e = s
            .set_setting("com.test.x", "nope", Value::Bool(true))
            .unwrap_err();
        assert_eq!(e.code, "plugin_invalid");
        assert_eq!(s.setting("com.test.x", "nope"), None);
    }

    fn dev_folder(root: &Path, version: &str) -> PathBuf {
        let f = root.join("my-plugin");
        fs::create_dir_all(f.join("themes")).unwrap();
        fs::write(f.join("plugin.toml"), theme_toml("com.dev.x", version, "")).unwrap();
        fs::write(f.join("themes/a.toml"), THEME_A).unwrap();
        f
    }

    fn files_in(dir: &Path) -> Vec<(PathBuf, Vec<u8>)> {
        let mut out = Vec::new();
        for e in fs::read_dir(dir).unwrap().flatten() {
            if e.path().is_dir() {
                out.extend(files_in(&e.path()));
            } else {
                out.push((e.path(), fs::read(e.path()).unwrap()));
            }
        }
        out.sort();
        out
    }

    #[test]
    fn dev_folder_load_and_reload() {
        let (a, dir) = app();
        let f = dev_folder(a.path(), "1.0.0");
        let mut s = Store::open(&dir).unwrap();
        assert_eq!(s.load_dev(&f, &ctx()).unwrap_err().code, "plugin_unsigned");
        s.set_developer_mode(true).unwrap();
        let info = s.load_dev(&f, &ctx()).unwrap();
        assert!(info.dev);
        assert_eq!(info.trust, PluginTrust::Unsigned);
        assert_eq!(Store::open(&dir).unwrap().list(), s.list());
        fs::write(f.join("plugin.toml"), theme_toml("com.dev.x", "1.1.0", "")).unwrap();
        let before = files_in(&f);
        assert_eq!(s.reload_dev("com.dev.x", &ctx()).unwrap().version, "1.1.0");
        assert_eq!(s.list()[0].version, "1.1.0");
        assert_eq!(files_in(&f), before);
        assert!(!dir.join("installed/com.dev.x").exists());
    }

    #[test]
    fn dev_folder_cannot_shadow_an_installed_plugin() {
        let (a, dir) = app();
        let mut s = installed_one(&dir, "");
        s.set_developer_mode(true).unwrap();
        let f = a.path().join("shadow");
        fs::create_dir_all(f.join("themes")).unwrap();
        fs::write(f.join("plugin.toml"), theme_toml("com.test.x", "2.0.0", "")).unwrap();
        fs::write(f.join("themes/a.toml"), THEME_A).unwrap();
        assert_eq!(s.load_dev(&f, &ctx()).unwrap_err().code, "plugin_invalid");
        assert_eq!(s.list()[0].version, "1.0.0");
    }

    #[test]
    fn developer_mode_off_unloads() {
        let (a, dir) = app();
        let mut s = Store::open(&dir).unwrap();
        s.set_developer_mode(true).unwrap();
        s.load_dev(&dev_folder(a.path(), "1.0.0"), &ctx()).unwrap();
        let (_t, p) = package(&theme_toml("com.test.u", "1.0.0", ""), &[], Sign::No);
        s.install(&p, &ctx()).unwrap();
        assert_eq!(s.list().len(), 2);
        s.set_developer_mode(false).unwrap();
        assert!(s.list().is_empty());
        assert!(s.enabled().is_empty());
        assert!(s.get("com.test.u").is_none());
        assert!(dir.join("installed/com.test.u/plugin.toml").is_file());
        assert!(a.path().join("my-plugin/plugin.toml").is_file());
        assert!(Store::open(&dir).unwrap().list().is_empty());
        s.set_developer_mode(true).unwrap();
        assert_eq!(s.list().len(), 2);
    }

    #[test]
    fn mark_stopped_runtime_only() {
        let (_a, dir) = app();
        let mut s = installed_one(&dir, "");
        s.mark_stopped("com.test.x", "It stopped responding".into());
        assert_eq!(
            s.list()[0].state,
            PluginState::Stopped {
                reason: "It stopped responding".into()
            }
        );
        assert!(s.enabled().is_empty());
        assert_eq!(
            Store::open(&dir).unwrap().list()[0].state,
            PluginState::Ready
        );
    }

    #[test]
    fn mark_problem_runtime_only() {
        let (_a, dir) = app();
        let mut s = installed_one(&dir, "");
        s.mark_problem("com.test.x", "tools don't match plugin.toml".into());
        assert_eq!(
            s.list()[0].state,
            PluginState::Problem {
                reason: "tools don't match plugin.toml".into()
            }
        );
        assert!(s.enabled().is_empty());
        s.set_enabled("com.test.x", false).unwrap();
        assert_eq!(s.list()[0].state, PluginState::Off, "off wins");
        s.set_enabled("com.test.x", true).unwrap();
        assert!(s.enabled().is_empty(), "kept until loaded again");
        assert_eq!(
            Store::open(&dir).unwrap().list()[0].state,
            PluginState::Ready
        );
    }
}
