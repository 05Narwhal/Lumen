//! Plugin MCP tools and prompts beside Lumen's own (Round 23 lane A4).

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, Weak};

use photos_core::Library;
use photos_plugins::host::{DraftsHost, HostCtx, PhotoMeta, PhotosHost, Tool, ToolsInstance};
use photos_plugins::PluginPermission;
use serde_json::{Map, Value};

use crate::error::{AppError, AppResult};
use crate::mcp::render::Compare;
use crate::mcp::{tools, Host, Shared};
use crate::plugins::Plugins;

/// The host of the MCP server that runs now. A plugin's tools instance lives as long as the
/// process (the engine caches it and rebuilds it after an idle spell or a trap), while the server
/// and its host are made again on every start, so the plugin hosts never keep a host of their own:
/// they ask this slot on every call. Empty while the server is off, which refuses every call.
#[derive(Default)]
pub struct HostSlot(Mutex<Option<Weak<dyn Host>>>);

impl HostSlot {
    fn lock(&self) -> MutexGuard<'_, Option<Weak<dyn Host>>> {
        self.0.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// The server that starts now. Weak: the server owns its host, and a host may hold `Shared`.
    pub fn set(&self, host: &Arc<dyn Host>) {
        *self.lock() = Some(Arc::downgrade(host));
    }

    /// The server stopped.
    pub fn clear(&self) {
        *self.lock() = None;
    }

    /// The running server's host, or `assistants_off` when there is none.
    fn get(&self) -> AppResult<Arc<dyn Host>> {
        self.lock().as_ref().and_then(Weak::upgrade).ok_or_else(assistants_off)
    }
}

/// `lumen:plugin/photos` for one plugin's tools: Develop's filmstrip through the built-in tools'
/// own scope, recipe and render, only while AI assistants are on (checked on every call).
pub(crate) struct McpPhotos {
    shared: Arc<Shared>,
    plugins: Weak<Plugins>,
    plugin: String,
}

/// `lumen:plugin/drafts`: a merge-patch through the built-in `edit`'s write path, as the history
/// step `AI: <plugin name>: <label>`. Writes `develop_drafts`, never `edits`.
pub(crate) struct McpDrafts {
    shared: Arc<Shared>,
    plugins: Weak<Plugins>,
    plugin: String,
    plugin_name: String,
}

impl McpPhotos {
    pub(crate) fn new(shared: Arc<Shared>, plugins: Weak<Plugins>, plugin: &str) -> Self {
        Self { shared, plugins, plugin: plugin.to_string() }
    }
}

impl McpDrafts {
    pub(crate) fn new(shared: Arc<Shared>, plugins: Weak<Plugins>, plugin: &str, plugin_name: &str) -> Self {
        Self { shared, plugins, plugin: plugin.to_string(), plugin_name: plugin_name.to_string() }
    }
}

/// The smallest preview a plugin gets; the largest is 1024 (spec §3).
const PREVIEW_MIN: u32 = 64;
const PREVIEW_MAX: u32 = 1024;
/// How much of a plugin's label reaches the history step.
const LABEL_MAX: usize = 80;
/// How much of a plugin's name reaches the history step.
const PLUGIN_NAME_MAX: usize = 40;

/// The refusal every MCP call gets while AI assistants are off, built-in or plugin.
pub(crate) fn assistants_off() -> AppError {
    AppError { code: "assistants_off".into(), message: "AI assistants are off in Lumen's Settings.".into(), detail: None }
}

/// The refusal a plugin's host gives once the plugin is off, removed or without the grant.
pub(crate) fn plugin_refused() -> AppError {
    AppError { code: "plugin_refused".into(), message: "This plugin is turned off or isn't allowed to do that.".into(), detail: None }
}

/// Whether plugin `id` is enabled now and holds `perm`: checked on every `photos` / `drafts` call,
/// because an instance (and the hosts in its context) outlives a Settings toggle. Fails closed
/// when the plugins are gone.
fn still_granted(plugins: &Weak<Plugins>, id: &str, perm: PluginPermission) -> AppResult<()> {
    let ok = plugins.upgrade().is_some_and(|p| p.read().enabled().iter().any(|i| i.manifest.id == id && i.granted.contains(&perm)));
    if ok {
        Ok(())
    } else {
        Err(plugin_refused())
    }
}

/// What every `photos` and `drafts` call checks first, at call time: AI assistants are on and
/// `id` is in Develop's filmstrip (the built-in tools' own check and refusal).
fn allowed(host: &dyn Host, shared: &Shared, id: i64) -> Result<(), AppError> {
    if !host.assistants_on() {
        return Err(assistants_off());
    }
    shared.scope.check(id)
}

/// The refusal a plugin sees: the message the built-in tools give the assistant.
fn text(e: AppError) -> String {
    e.message
}

/// A captured time as the wall-clock `YYYY-MM-DDTHH:MM:SS` it is stored as (no zone).
fn wall_clock(secs: i64) -> Option<String> {
    chrono::DateTime::from_timestamp(secs, 0).map(|d| d.naive_utc().format("%Y-%m-%dT%H:%M:%S").to_string())
}

/// `photo-meta` from the library: date, camera, lens, display-oriented size, rating, label, flag
/// and place, nothing else (no path, file name, hash or coordinates).
fn photo_meta(lib: &Library, id: i64) -> AppResult<PhotoMeta> {
    let a = lib.db.with_conn(|c| photos_core::assets::get_asset(c, id, false))?;
    let meta = |key: &str| a.metadata.as_ref().and_then(|m| m.get(key)).and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()).map(String::from);
    let (w, h) = (a.width.unwrap_or(0).clamp(0, u32::MAX.into()) as u32, a.height.unwrap_or(0).clamp(0, u32::MAX.into()) as u32);
    let (width, height) = if photos_media::orientation_swaps(a.orientation as u16) { (h, w) } else { (w, h) };
    let named = |s: &'static str| (s != "none").then(|| s.to_string());
    Ok(PhotoMeta {
        id,
        captured_at: a.captured_at.and_then(wall_clock),
        camera: meta("model").or_else(|| meta("make")),
        lens: meta("lens"),
        width,
        height,
        rating: a.rating.clamp(0, 5) as u8,
        label: named(crate::mcp::write::label_name(a.color_label)),
        flag: named(crate::mcp::write::flag_name(a.flag)),
        place: a.locality.or(a.place_name),
    })
}

/// `text` trimmed, on one line (control characters as spaces) and at most `max` characters.
fn one_line(text: &str, max: usize) -> String {
    let line: String = text.chars().map(|c| if c.is_control() { ' ' } else { c }).collect();
    let cut: String = line.trim().chars().take(max).collect();
    cut.trim_end().to_string()
}

/// `AI: <plugin name>: <label>`, each on one line, at most [`PLUGIN_NAME_MAX`] and [`LABEL_MAX`]
/// characters.
fn history_label(plugin_name: &str, label: &str) -> String {
    let (name, label) = (one_line(plugin_name, PLUGIN_NAME_MAX), one_line(label, LABEL_MAX));
    format!("AI: {}: {}", if name.is_empty() { "Plugin" } else { &name }, if label.is_empty() { "edit" } else { &label })
}

impl McpPhotos {
    /// The running server's host and the library, for a photo that may be read now.
    fn library(&self, id: i64) -> AppResult<(Arc<dyn Host>, Arc<Library>)> {
        still_granted(&self.plugins, &self.plugin, PluginPermission::PhotosRead)?;
        let host = self.shared.host.get()?;
        allowed(host.as_ref(), &self.shared, id)?;
        let lib = host.library()?;
        Ok((host, lib))
    }
}

impl PhotosHost for McpPhotos {
    fn scope(&self) -> Vec<i64> {
        if still_granted(&self.plugins, &self.plugin, PluginPermission::PhotosRead).is_err() {
            return Vec::new();
        }
        match self.shared.host.get() {
            Ok(host) if host.assistants_on() => self.shared.scope.get().order,
            _ => Vec::new(),
        }
    }

    fn metadata(&self, id: i64) -> Result<PhotoMeta, String> {
        self.library(id).and_then(|(_, lib)| photo_meta(&lib, id)).map_err(text)
    }

    /// The built-in `view`'s render (after, whole frame) of what the photo looks like now.
    fn preview(&self, id: i64, max: u32) -> Result<Vec<u8>, String> {
        let run = || -> AppResult<Vec<u8>> {
            let (host, lib) = self.library(id)?;
            let (host, shared) = (host.as_ref(), self.shared.as_ref());
            let r = tools::live_recipe(host, shared, &lib, id)?;
            let size = max.clamp(PREVIEW_MIN, PREVIEW_MAX);
            Ok(shared.renders.view(&lib, host.ffmpeg().as_ref(), id, &r, size, Compare::After, None, None)?.bytes)
        };
        run().map_err(|e| {
            tracing::info!(plugin = %self.plugin, id, error = %e.code, "plugin preview not given");
            text(e)
        })
    }

    /// The built-in `get_recipe`'s answer, as JSON text.
    fn recipe(&self, id: i64) -> Result<String, String> {
        let (host, lib) = self.library(id).map_err(text)?;
        let r = tools::live_recipe(host.as_ref(), &self.shared, &lib, id).map_err(text)?;
        Ok(photos_core::develop_mcp::summary(&r).to_string())
    }
}

impl DraftsHost for McpDrafts {
    /// Through the built-in `edit`'s write path: the same refusals, the same live page.
    fn patch(&self, id: i64, merge_patch_json: &str, label: &str) -> Result<(), String> {
        let run = || -> AppResult<()> {
            still_granted(&self.plugins, &self.plugin, PluginPermission::DevelopWriteDrafts)?;
            let host = self.shared.host.get()?;
            allowed(host.as_ref(), &self.shared, id)?;
            let lib = host.library()?;
            // Unparsable JSON is refused like any patch that is not an object.
            let patch = serde_json::from_str::<Value>(merge_patch_json).unwrap_or(Value::Null);
            crate::mcp::write::patch_draft(host.as_ref(), &self.shared, &lib, id, Some(&patch), &history_label(&self.plugin_name, label))?;
            Ok(())
        };
        let out = run();
        tracing::info!(plugin = %self.plugin, id, ok = out.is_ok(), "plugin draft");
        out.map_err(text)
    }
}

/// What goes between a plugin's prefix and its tool or prompt name. No built-in name contains
/// it, so a plugin's name can never equal a built-in one.
const SEPARATOR: &str = "__";

/// A prefix part with `.` made `_`, runs of `_` collapsed and none at the ends, so a prefix never
/// contains the separator.
fn clean(part: &str) -> String {
    let mut out = String::with_capacity(part.len());
    for c in part.chars().map(|c| if c == '.' { '_' } else { c }) {
        if c == '_' && out.ends_with('_') {
            continue;
        }
        out.push(c);
    }
    out.trim_matches('_').to_string()
}

/// Who gets which prefix among the enabled plugins.
#[derive(Debug, Default, PartialEq)]
pub(crate) struct Prefixes {
    /// Plugin id to its prefix, for every plugin whose prefix is its alone.
    pub usable: BTreeMap<String, String>,
    /// Plugin id to the prefix it would have, for plugins that share it with another: those list
    /// and answer nothing (never "the first one wins", which would let one shadow the other).
    pub clashed: BTreeMap<String, String>,
}

/// The prefix of each enabled plugin id: its last id segment, or, when two enabled plugins share
/// that segment, the full id with `.` as `_` for each of them. A prefix that two or more plugins
/// still end up with (`clean` collapses `_` and `valid_id` allows empty segments, so
/// `studio.jl.horizon` and `studio.jl..horizon` both give `studio_jl_horizon`) belongs to none of
/// them. The answer does not depend on the order of `enabled`.
pub(crate) fn assign_prefixes(enabled: &[&str]) -> Prefixes {
    let ids: BTreeSet<&str> = enabled.iter().copied().collect();
    let short_of = |id: &str| clean(id.rsplit('.').next().unwrap_or(id));
    let mut taken: BTreeMap<String, usize> = BTreeMap::new();
    for id in &ids {
        *taken.entry(short_of(id)).or_default() += 1;
    }
    let chosen: BTreeMap<String, String> = ids
        .into_iter()
        .map(|id| {
            let short = short_of(id);
            let prefix = if short.is_empty() || taken.get(&short).copied().unwrap_or(0) > 1 { clean(id) } else { short };
            (id.to_string(), prefix)
        })
        .collect();
    let mut users: BTreeMap<&str, usize> = BTreeMap::new();
    for prefix in chosen.values() {
        *users.entry(prefix.as_str()).or_default() += 1;
    }
    let shared: BTreeSet<String> = users.into_iter().filter(|(_, n)| *n > 1).map(|(p, _)| p.to_string()).collect();
    let (clashed, usable) = chosen.into_iter().partition(|(_, prefix)| shared.contains(prefix));
    Prefixes { usable, clashed }
}

/// [`assign_prefixes`]'s usable prefixes only.
#[cfg(test)]
pub(crate) fn mcp_prefixes(enabled: &[&str]) -> BTreeMap<String, String> {
    assign_prefixes(enabled).usable
}

/// The name an assistant sees for a plugin's tool or prompt.
pub(crate) fn mcp_name(prefix: &str, name: &str) -> String {
    format!("{prefix}{SEPARATOR}{name}")
}

/// Splits `<prefix>__<name>` at the first separator; `None` for anything else, such as a
/// built-in name.
pub(crate) fn parse_mcp_name(s: &str) -> Option<(&str, &str)> {
    let (prefix, name) = s.split_once(SEPARATOR)?;
    (!prefix.is_empty() && !name.is_empty()).then_some((prefix, name))
}

// ---- Tools and prompts in the server (Task 3) ----

/// The state reason of a plugin whose `tools()` names aren't its `[[tools]]` (spec §10).
const TOOLS_MISMATCH: &str = "tools don't match plugin.toml";
/// MCP clients refuse longer tool names.
const NAME_MAX: usize = 64;
/// The most of a plugin tool's text an assistant gets, in bytes, before [`TRUNCATED`].
pub(crate) const RESULT_MAX: usize = 1 << 20;
pub(crate) const TRUNCATED: &str = "…(truncated by Lumen)";
/// The most a plugin tool's input schema may take, in bytes.
const SCHEMA_MAX: usize = 64 << 10;
/// The most all of one plugin's input schemas may take together, in bytes.
const SCHEMAS_MAX: usize = 256 << 10;
/// The most tools, and the most prompts, one plugin lists.
const TOOLS_MAX: usize = 32;
const PROMPTS_MAX: usize = 32;
/// The most of a tool's or prompt's description an assistant gets, in bytes (cut, not refused).
const DESCRIPTION_MAX: usize = 1 << 10;
/// The most of a plugin's error message an assistant gets, in bytes.
const ERROR_MAX: usize = 4 << 10;
/// The most of a `tools()` failure the plugin's *Problem* reason keeps, in bytes.
const REASON_MAX: usize = 200;

/// A plugin tool as `tools/list` shows it.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PluginTool {
    /// `<prefix>__<tool>`.
    pub name: String,
    /// Its `[[tools]]` description.
    pub description: String,
    pub schema: Value,
    plugin: String,
    tool: String,
    dir: PathBuf,
}

/// A plugin prompt as `prompts/list` shows it.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ListedPrompt {
    /// `<prefix>__<prompt>`.
    pub name: String,
    pub description: String,
    pub text: String,
}

type Checked = Vec<(String, Value)>;

/// What reading a plugin's `tools()` came to.
#[derive(Clone)]
enum Read {
    Tools(Checked),
    /// `tools()` itself failed; not tried again until the plugins change.
    Failed,
}

#[derive(Default)]
struct CacheState {
    reads: HashMap<(String, String), Read>,
    /// What was already said in a plugin's log since the last change.
    said: HashSet<String>,
}

/// Each plugin's checked `tools()` (name and schema, in its order) per (id, version), or that it
/// failed, and which notes were logged. Cleared whenever the plugins change ([`follow_changes`]),
/// so a reloaded dev folder is read again and a note may be said again.
#[derive(Default)]
pub struct ToolCache(Mutex<CacheState>);

impl ToolCache {
    fn lock(&self) -> MutexGuard<'_, CacheState> {
        self.0.lock().unwrap_or_else(|p| p.into_inner())
    }

    fn get(&self, key: &(String, String)) -> Option<Read> {
        self.lock().reads.get(key).cloned()
    }

    fn put(&self, key: (String, String), read: Read) {
        self.lock().reads.insert(key, read);
    }

    /// True the first time `note` is asked since the last [`clear`](Self::clear).
    fn first(&self, note: String) -> bool {
        self.lock().said.insert(note)
    }

    /// Forgets every plugin's tools and notes ([`follow_changes`] calls it).
    pub fn clear(&self) {
        let mut state = self.lock();
        state.reads.clear();
        state.said.clear();
    }
}

/// `text` cut to at most [`DESCRIPTION_MAX`] bytes, on a character boundary.
fn cut_description(text: &str) -> String {
    if text.len() <= DESCRIPTION_MAX {
        return text.to_string();
    }
    let mut end = DESCRIPTION_MAX;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text[..end].trim_end().to_string()
}

/// Whenever the plugin list, a state or a setting changes (enable, disable, remove, install, a
/// Problem): clears `shared`'s tool cache, then tells the connected sessions to list again.
pub(crate) fn follow_changes(plugins: &Plugins, shared: Arc<Shared>) {
    plugins.on_change(Box::new(move || {
        shared.plugin_tools.clear();
        shared.peers.notify_list_changed();
    }));
}

/// An enabled plugin that declares tools, as the store has it.
pub(crate) struct Offer {
    pub id: String,
    pub version: String,
    dir: PathBuf,
    /// `[[tools]]`: name and description.
    decls: Vec<(String, String)>,
}

/// Every enabled plugin's id (prefixes are chosen among them all) and those that offer tools.
fn offers(plugins: &Plugins) -> (Vec<String>, Vec<Offer>) {
    let store = plugins.read();
    let enabled = store.enabled();
    let ids = enabled.iter().map(|i| i.manifest.id.clone()).collect();
    let offers = enabled
        .into_iter()
        .filter(|i| !i.manifest.tools.is_empty())
        .map(|i| Offer {
            id: i.manifest.id.clone(),
            version: i.manifest.version.to_string(),
            dir: i.dir.clone(),
            decls: i.manifest.tools.iter().map(|t| (t.name.clone(), t.description.clone())).collect(),
        })
        .collect();
    (ids, offers)
}

fn prefixes_of(ids: &[String]) -> Prefixes {
    assign_prefixes(&ids.iter().map(String::as_str).collect::<Vec<_>>())
}

/// Says once per change (the cache is cleared on every one) why plugin `id` has no tools or
/// prompts listed because another enabled plugin has its name prefix.
fn say_clash(plugins: &Plugins, shared: &Shared, id: &str, prefix: &str) {
    if shared.plugin_tools.first(format!("clash {id}")) {
        plugins.log_sink(id)(&format!("its tools and prompts aren't offered to AI assistants: another enabled plugin has the same name prefix {prefix}"));
    }
}

/// Every built-in tool and prompt name.
fn builtin_names() -> BTreeSet<String> {
    let tools = tools::list().into_iter().map(|(n, ..)| n.to_string());
    tools.chain(crate::mcp::prompts().into_iter().map(|p| p.name)).collect()
}

/// Why `name` (with `prefix`) is never listed nor called, if it isn't fit: a built-in's name, an
/// empty prefix, over [`NAME_MAX`] characters (clients refuse those) or characters outside
/// `[A-Za-z0-9_-]`. The naming rules make most of these impossible; this holds regardless.
fn refused(name: &str, prefix: &str, builtin: &BTreeSet<String>) -> Option<&'static str> {
    if prefix.is_empty() {
        Some("its prefix is empty")
    } else if name.len() > NAME_MAX {
        Some("its name would be over 64 characters")
    } else if !name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-') {
        Some("its name has characters other than A-Z, a-z, 0-9, _ and -")
    } else if builtin.contains(name) {
        Some("its name is a built-in one")
    } else {
        None
    }
}

/// A plugin's `tools()` against its `[[tools]]`: the same names (each once), each schema a JSON
/// object of `"type": "object"` (MCP clients refuse any other tool input schema) of at most
/// [`SCHEMA_MAX`] bytes, at most [`TOOLS_MAX`] tools and [`SCHEMAS_MAX`] bytes of schemas in all.
/// `Err` is the plugin's *Problem* reason.
fn checked(decls: &[(String, String)], exported: Vec<Tool>) -> Result<Checked, String> {
    if exported.len() > TOOLS_MAX {
        return Err(format!("it lists more than {TOOLS_MAX} tools"));
    }
    let mut want: Vec<&str> = decls.iter().map(|(n, _)| n.as_str()).collect();
    let mut got: Vec<&str> = exported.iter().map(|t| t.name.as_str()).collect();
    want.sort_unstable();
    got.sort_unstable();
    if want != got {
        return Err(TOOLS_MISMATCH.into());
    }
    if exported.iter().map(|t| t.input_schema.len()).sum::<usize>() > SCHEMAS_MAX {
        return Err("its tool schemas are over 256 KiB together".into());
    }
    exported
        .into_iter()
        .map(|t| {
            if t.input_schema.len() > SCHEMA_MAX {
                return Err(format!("the input schema of the tool {} is over 64 KiB", t.name));
            }
            match serde_json::from_str::<Value>(&t.input_schema) {
                Ok(schema @ Value::Object(_)) if schema["type"] == "object" => Ok((t.name, schema)),
                _ => Err(format!("the input schema of the tool {} isn't a JSON schema of type object", t.name)),
            }
        })
        .collect()
}

/// A plugin's error as the assistant gets it: one line, at most [`ERROR_MAX`] bytes (cut on a
/// character boundary, ending with `…`).
fn plugin_error(e: photos_plugins::PluginError) -> AppError {
    let mut e = AppError::from(e);
    e.message = capped_line(&e.message, ERROR_MAX);
    e
}

/// `text` on one line (control characters as spaces), trimmed, of at most `max` bytes.
fn capped_line(text: &str, max: usize) -> String {
    let line: String = text.chars().map(|c| if c.is_control() { ' ' } else { c }).collect();
    let line = line.trim();
    if line.len() <= max {
        return line.to_string();
    }
    let mut end = max - '…'.len_utf8();
    while !line.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", line[..end].trim_end())
}

/// A plugin tool's text as the assistant gets it: over [`RESULT_MAX`] bytes, the first
/// [`RESULT_MAX`] bytes (cut back to a character boundary) followed by [`TRUNCATED`], so a cut
/// result is at most `RESULT_MAX + TRUNCATED.len()` bytes.
pub(crate) fn truncated(mut text: String) -> String {
    if text.len() <= RESULT_MAX {
        return text;
    }
    let mut end = RESULT_MAX;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text.truncate(end);
    text.push_str(TRUNCATED);
    text
}

/// What a plugin's tools instance is given: its id, grants, settings, data folder and log, and
/// the `photos` / `drafts` hosts only when `photos.read` / `develop.write-drafts` are granted.
/// `None` when the plugin is no longer in the store. The hosts reach the running server through
/// `shared.host` on every call, so a context stays right across server restarts.
pub(crate) fn tools_ctx(plugins: &Arc<Plugins>, shared: &Arc<Shared>, id: &str) -> Option<HostCtx> {
    let store = plugins.read();
    let inst = store.get(id)?;
    let granted = inst.granted.clone();
    let photos = granted.contains(&PluginPermission::PhotosRead).then(|| Arc::new(McpPhotos::new(shared.clone(), Arc::downgrade(plugins), id)) as Arc<dyn PhotosHost>);
    let drafts = granted.contains(&PluginPermission::DevelopWriteDrafts).then(|| Arc::new(McpDrafts::new(shared.clone(), Arc::downgrade(plugins), id, &inst.manifest.name)) as Arc<dyn DraftsHost>);
    Some(HostCtx {
        plugin: id.to_string(),
        granted,
        settings: inst.info.settings.iter().map(|s| (s.key.clone(), s.value.clone())).collect(),
        data_dir: store.data_dir(id),
        log: plugins.log_sink(id),
        cancel: None,
        chosen: None,
        photos,
        drafts,
    })
}

/// The plugin's shared tools instance; its context is read from the store each time it is built.
fn instance(plugins: &Arc<Plugins>, shared: &Arc<Shared>, id: &str, dir: &Path) -> AppResult<Arc<ToolsInstance>> {
    let engine = plugins.engine()?;
    engine.tools_at(id, &dir.join("plugin.wasm"), ctx_fn(plugins, shared, id)).map_err(plugin_error)
}

/// The closure the engine keeps for a plugin's tools instance for the life of the process, and
/// calls each time it rebuilds the instance (after an idle spell or a trap). It reads the store
/// afresh and holds no host: `photos` / `drafts` find the running server through `shared.host`.
fn ctx_fn(plugins: &Arc<Plugins>, shared: &Arc<Shared>, id: &str) -> impl Fn() -> HostCtx + Send + Sync + 'static {
    // Weak: the engine (inside `Plugins`) keeps this closure.
    let (plugins_w, shared, owner) = (Arc::downgrade(plugins), shared.clone(), id.to_string());
    let (data_dir, log) = (plugins.read().data_dir(id), plugins.log_sink(id));
    move || {
        plugins_w.upgrade().and_then(|p| tools_ctx(&p, &shared, &owner)).unwrap_or_else(|| HostCtx {
            // Removed meanwhile: nothing granted, no photos, no drafts.
            plugin: owner.clone(),
            granted: Vec::new(),
            settings: Map::new(),
            data_dir: data_dir.clone(),
            log: log.clone(),
            cancel: None,
            chosen: None,
            photos: None,
            drafts: None,
        })
    }
}

/// Every enabled plugin's tools while AI assistants are on, after the built-ins in `tools/list`.
/// A plugin whose `tools()` don't match its manifest, or give a schema that isn't a JSON object,
/// is marked *Problem* and lists nothing; the others list as usual.
pub(crate) fn plugin_tools(host: &Arc<dyn Host>, shared: &Arc<Shared>) -> Vec<PluginTool> {
    let Some(plugins) = host.plugins() else { return Vec::new() };
    let exported = |o: &Offer| -> AppResult<Vec<Tool>> { instance(&plugins, shared, &o.id, &o.dir)?.list().map_err(plugin_error) };
    plugin_tools_with(host.as_ref(), shared, &plugins, &exported)
}

/// How many tools enabled plugins add: Settings → AI assistants' *Plugins add N tools*. Zero
/// while AI assistants are off (nothing is listed then).
pub(crate) fn plugin_tool_count(host: &Arc<dyn Host>, shared: &Arc<Shared>) -> u32 {
    plugin_tools(host, shared).len() as u32
}

/// [`plugin_tools`] with `exported` reading a plugin's `tools()` (a test's own lists).
fn plugin_tools_with(host: &dyn Host, shared: &Shared, plugins: &Plugins, exported: &dyn Fn(&Offer) -> AppResult<Vec<Tool>>) -> Vec<PluginTool> {
    if !host.assistants_on() {
        return Vec::new();
    }
    let (ids, offers) = offers(plugins);
    let prefixes = prefixes_of(&ids);
    let builtin = builtin_names();
    let mut out: Vec<PluginTool> = Vec::new();
    for o in offers {
        // A prefix two plugins share belongs to neither, and neither's wasm runs for a listing.
        let Some(prefix) = prefixes.usable.get(&o.id).map(String::as_str) else {
            if let Some(prefix) = prefixes.clashed.get(&o.id) {
                say_clash(plugins, shared, &o.id, prefix);
            }
            continue;
        };
        let key = (o.id.clone(), o.version.clone());
        let cached = shared.plugin_tools.get(&key);
        let fresh = cached.is_none();
        let tools = match cached {
            Some(Read::Tools(t)) => t,
            Some(Read::Failed) => continue,
            None => match exported(&o).map(|list| checked(&o.decls, list)) {
                Ok(Ok(t)) => {
                    shared.plugin_tools.put(key, Read::Tools(t.clone()));
                    t
                }
                Ok(Err(reason)) => {
                    plugins.log_sink(&o.id)(&format!("its tools aren't offered to AI assistants: {reason}"));
                    plugins.mark_problem(&o.id, &reason);
                    continue;
                }
                Err(e) => {
                    // Only its tools are lost (its themes, presets and looks keep working), and the
                    // failure is remembered so that it is not instantiated again on every listing
                    // and call: until the plugins change.
                    plugins.log_sink(&o.id)(&format!("its tools couldn't be read: {}", capped_line(&e.message, REASON_MAX)));
                    shared.plugin_tools.put(key, Read::Failed);
                    continue;
                }
            },
        };
        for (tool, schema) in tools {
            let name = mcp_name(prefix, &tool);
            let why = refused(&name, prefix, &builtin).or_else(|| out.iter().any(|t| t.name == name).then_some("another tool has the same name"));
            if let Some(why) = why {
                if fresh {
                    plugins.log_sink(&o.id)(&format!("the tool {tool} isn't offered to AI assistants: {why}"));
                }
                continue;
            }
            let description = o.decls.iter().find(|(n, _)| *n == tool).map(|(_, d)| cut_description(d)).unwrap_or_default();
            out.push(PluginTool { name, description, schema, plugin: o.id.clone(), tool, dir: o.dir.clone() });
        }
    }
    out
}

/// One `tools/call` of a plugin tool: only a tool [`plugin_tools`] lists now (else the built-in
/// unknown-tool error), through the plugin's shared instance (30 s per call), its text cut at
/// [`RESULT_MAX`].
pub(crate) fn call_plugin_tool(host: &Arc<dyn Host>, shared: &Arc<Shared>, name: &str, args: &Map<String, Value>) -> AppResult<String> {
    if !host.assistants_on() {
        return Err(assistants_off());
    }
    let (Some(plugins), Some(_)) = (host.plugins(), parse_mcp_name(name)) else { return Err(tools::unknown(name)) };
    let Some(t) = plugin_tools(host, shared).into_iter().find(|t| t.name == name) else { return Err(tools::unknown(name)) };
    let out = instance(&plugins, shared, &t.plugin, &t.dir)?.call(&t.tool, &Value::Object(args.clone()).to_string()).map_err(plugin_error)?;
    Ok(truncated(out))
}

/// Every enabled plugin's prompts while AI assistants are on, named like their tools. A plugin
/// with more than [`PROMPTS_MAX`] prompts is marked *Problem* and lists none; a name that comes
/// twice is listed once; each refusal is logged once per change of the plugins.
pub(crate) fn plugin_prompts(host: &dyn Host, shared: &Shared) -> Vec<ListedPrompt> {
    if !host.assistants_on() {
        return Vec::new();
    }
    let Some(plugins) = host.plugins() else { return Vec::new() };
    let (ids, prompts) = {
        let store = plugins.read();
        (store.enabled().iter().map(|i| i.manifest.id.clone()).collect::<Vec<_>>(), super::data::plugin_prompts(&store))
    };
    let prefixes = prefixes_of(&ids);
    let builtin = builtin_names();
    let mut per_plugin: BTreeMap<&str, usize> = BTreeMap::new();
    for p in &prompts {
        *per_plugin.entry(p.plugin.as_str()).or_default() += 1;
    }
    let mut out: Vec<ListedPrompt> = Vec::new();
    let mut over: BTreeSet<&str> = BTreeSet::new();
    for p in &prompts {
        if per_plugin[p.plugin.as_str()] > PROMPTS_MAX {
            if over.insert(p.plugin.as_str()) {
                let reason = format!("it has more than {PROMPTS_MAX} prompts");
                plugins.log_sink(&p.plugin)(&format!("its prompts aren't offered to AI assistants: {reason}"));
                plugins.mark_problem(&p.plugin, &reason);
            }
            continue;
        }
        let Some(prefix) = prefixes.usable.get(&p.plugin).map(String::as_str) else {
            if let Some(prefix) = prefixes.clashed.get(&p.plugin) {
                say_clash(&plugins, shared, &p.plugin, prefix);
            }
            continue;
        };
        let name = mcp_name(prefix, &p.name);
        let why = refused(&name, prefix, &builtin).or_else(|| out.iter().any(|l| l.name == name).then_some("another prompt has the same name"));
        if let Some(why) = why {
            if shared.plugin_tools.first(format!("prompt {} {}", p.plugin, p.name)) {
                plugins.log_sink(&p.plugin)(&format!("the prompt {} isn't offered to AI assistants: {why}", p.name));
            }
            continue;
        }
        out.push(ListedPrompt { name, description: cut_description(&p.description), text: p.text.clone() });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::Mutex;

    use photos_core::assets::{insert_assets, NewAsset};
    use photos_core::edits::Recipe;
    use photos_core::Library;
    use serde_json::{json, Value};

    use crate::error::AppResult;
    use crate::mcp::{bridge, tools, write};

    /// The `mcp` tests' temp-library host, plus the switch, the change notices and the page.
    struct PluginTestHost {
        lib: Arc<Library>,
        shared: Arc<Shared>,
        on: AtomicBool,
        changed: AtomicUsize,
        /// The labels the page was asked to apply (the photo open in Develop on screen).
        asked: Mutex<Vec<String>>,
        plugins: Option<Arc<Plugins>>,
    }

    impl Host for PluginTestHost {
        fn library(&self) -> AppResult<Arc<Library>> {
            Ok(self.lib.clone())
        }
        fn ffmpeg(&self) -> Option<photos_media::Ffmpeg> {
            None
        }
        /// The page: runs the op on its draft (identity here) and answers, as mcp-bridge.ts does.
        fn emit(&self, req: &bridge::McpRequest) -> AppResult<()> {
            self.asked.lock().unwrap().push(req.label.clone());
            let r = write::transform(self, &self.lib, req.asset_id, &Recipe::identity(), &req.op)?;
            self.shared.bridge.reply(req.req, bridge::McpReply { recipe: Some(r), error: None, message: None });
            Ok(())
        }
        fn changed(&self) {
            self.changed.fetch_add(1, Ordering::SeqCst);
        }
        fn auto(&self, _: i64, _: &Recipe, model: &str) -> AppResult<Recipe> {
            Err(write::no_model(model))
        }
        fn assistants_on(&self) -> bool {
            self.on.load(Ordering::SeqCst)
        }
        fn plugins(&self) -> Option<Arc<Plugins>> {
            self.plugins.clone()
        }
    }

    struct Fixture {
        _tmp: tempfile::TempDir,
        lib: Arc<Library>,
        host: Arc<PluginTestHost>,
        shared: Arc<Shared>,
        /// Two filmstrip photos (`a` 1400×900 with camera, lens, date and place; `b` 600×400) and
        /// `c`, a library photo outside the strip.
        a: i64,
        b: i64,
        c: i64,
        /// The plugin `studio.jl.horizon` (photos.read, develop.write-drafts) that `photos()` and
        /// `drafts()` speak for, apart from `host.plugins`.
        own: Arc<Plugins>,
        _own_dirs: (tempfile::TempDir, tempfile::TempDir),
    }

    impl Fixture {
        fn photos(&self) -> McpPhotos {
            McpPhotos::new(self.shared.clone(), Arc::downgrade(&self.own), "studio.jl.horizon")
        }
        fn drafts(&self) -> McpDrafts {
            McpDrafts::new(self.shared.clone(), Arc::downgrade(&self.own), "studio.jl.horizon", "Horizon")
        }
        fn history(&self, id: i64) -> Vec<String> {
            self.lib.db.with_conn(|c| photos_core::develop_history::list_history(c, id)).unwrap().into_iter().map(|s| s.label).collect()
        }
        fn draft(&self, id: i64) -> Option<Recipe> {
            self.lib.db.with_conn(|c| photos_core::edits::get_draft(c, id)).unwrap().map(|(r, _)| r)
        }
        fn edit_row(&self, id: i64) -> Option<Recipe> {
            self.lib.db.with_conn(|c| photos_core::edits::get_recipe(c, id)).unwrap().map(|(r, _)| r)
        }
        /// The built-in tool's answer, as the assistant would get it.
        fn builtin(&self, name: &str, args: Value) -> AppResult<tools::Output> {
            tools::call(self.host.as_ref(), &self.shared, name, args.as_object().unwrap())
        }
        fn dyn_host(&self) -> Arc<dyn Host> {
            self.host.clone()
        }
        /// What `tools/list` names, built-ins first.
        fn listed(&self) -> Vec<String> {
            crate::mcp::listed_tools(&self.dyn_host(), &self.shared).into_iter().map(|t| t.name.to_string()).collect()
        }
        fn prompts(&self) -> Vec<String> {
            crate::mcp::listed_prompts(self.host.as_ref(), &self.shared).into_iter().map(|p| p.name).collect()
        }
        /// One `tools/call` as the server runs it.
        fn call(&self, name: &str, args: Value) -> AppResult<tools::Output> {
            crate::mcp::run_call(&self.dyn_host(), &self.shared, name, args.as_object().unwrap())
        }
        fn text(&self, name: &str, args: Value) -> String {
            match self.call(name, args) {
                Ok(tools::Output::Text(t)) => t,
                Ok(_) => panic!("not text"),
                Err(e) => panic!("{}: {}", e.code, e.message),
            }
        }
    }

    /// A temp library as the `mcp` tests build it, with AI assistants on and `a`, `b` in the strip.
    fn fixture() -> Fixture {
        fixture_with(None)
    }

    /// The same, with these plugins.
    fn fixture_with(plugins: Option<Arc<Plugins>>) -> Fixture {
        let tmp = tempfile::tempdir().unwrap();
        let lib = Arc::new(Library::create(&tmp.path().join("lib")).unwrap());
        let mut rows = Vec::new();
        for (name, w, h) in [("a.jpg", 1400u32, 900u32), ("b.jpg", 600, 400), ("c.jpg", 600, 400)] {
            let img = image::RgbImage::from_fn(w, h, |x, y| image::Rgb([(x % 256) as u8, (y % 256) as u8, 128]));
            let bytes = photos_media::encode_jpeg_bytes(&image::DynamicImage::ImageRgb8(img), 90).unwrap();
            std::fs::write(lib.originals_dir().join(name), bytes).unwrap();
            rows.push(NewAsset {
                content_hash: format!("hash-{name}"),
                relative_path: name.into(),
                filename: name.into(),
                extension: "jpg".into(),
                width: Some(w.into()),
                height: Some(h.into()),
                orientation: 1,
                ..Default::default()
            });
        }
        rows[0].captured_at = Some(1_700_000_000);
        rows[0].gps_lat = Some(46.0);
        rows[0].gps_lon = Some(14.03);
        rows[0].metadata_json = Some(r#"{"make":"NIKON CORPORATION","model":"NIKON Z 6_2","lens":"NIKKOR Z 50mm f/1.8 S","iso":400}"#.into());
        let ids = lib.db.with_conn(|c| insert_assets(c, &rows)).unwrap();
        let (a, b, c) = (ids[0], ids[1], ids[2]);
        lib.db
            .with_conn(|c| -> photos_core::Result<()> {
                photos_core::assets::set_places(c, &[(a, "Idrija, Slovenia".into())])?;
                photos_core::assets::set_rating(c, &[a], 4)?;
                photos_core::labels::set_color_label(c, &[a], 4)?;
                photos_core::labels::set_flag(c, &[a], photos_core::labels::FLAG_PICK)?;
                Ok(())
            })
            .unwrap();
        let shared = Arc::new(Shared::default());
        shared.scope.set(vec![b, a], Some(a));
        let host = Arc::new(PluginTestHost { lib: lib.clone(), shared: shared.clone(), on: AtomicBool::new(true), changed: AtomicUsize::new(0), asked: Mutex::new(Vec::new()), plugins });
        shared.host.set(&(host.clone() as Arc<dyn Host>));
        let root = tempfile::tempdir().unwrap();
        let (app, own) = plugins_with(&[dev_plugin(root.path(), "studio.jl.horizon", "1.0.0", r#"["photos.read", "develop.write-drafts"]"#, &[], "")]);
        Fixture { _tmp: tmp, lib, host, shared, a, b, c, own, _own_dirs: (root, app) }
    }

    /// Every field of `photo-meta` by name; the destructuring breaks the build if one is added.
    fn meta_json(m: &PhotoMeta) -> Value {
        let PhotoMeta { id, captured_at, camera, lens, width, height, rating, label, flag, place } = m;
        json!({ "id": id, "captured_at": captured_at, "camera": camera, "lens": lens, "width": width, "height": height, "rating": rating, "label": label, "flag": flag, "place": place })
    }

    fn long_side(jpeg: &[u8]) -> u32 {
        let img = image::load_from_memory_with_format(jpeg, image::ImageFormat::Jpeg).unwrap();
        img.width().max(img.height())
    }

    #[test]
    fn scope_is_filmstrip() {
        let f = fixture();
        assert_eq!(f.photos().scope(), vec![f.b, f.a], "the strip, in its order");
        f.shared.scope.set(vec![f.a], Some(f.a));
        assert_eq!(f.photos().scope(), vec![f.a], "read at call time");
    }

    #[test]
    fn metadata_fields_only() {
        let f = fixture();
        let m = f.photos().metadata(f.a).unwrap();
        assert_eq!(
            meta_json(&m),
            json!({
                "id": f.a, "captured_at": "2023-11-14T22:13:20", "camera": "NIKON Z 6_2", "lens": "NIKKOR Z 50mm f/1.8 S",
                "width": 1400, "height": 900, "rating": 4, "label": "blue", "flag": "pick", "place": "Idrija, Slovenia"
            })
        );
        let text = format!("{m:?}");
        for leak in ["a.jpg", "hash-a", "originals", "46.0", "14.03"] {
            assert!(!text.contains(leak), "{leak} in {text}");
        }
        let plain = meta_json(&f.photos().metadata(f.b).unwrap());
        assert_eq!(plain, json!({ "id": f.b, "captured_at": null, "camera": null, "lens": null, "width": 600, "height": 400, "rating": 0, "label": null, "flag": null, "place": null }));
    }

    #[test]
    fn preview_capped() {
        let f = fixture();
        let (photos, drafts) = (f.photos(), f.drafts());
        let before = photos.preview(f.a, 4000).unwrap();
        assert_eq!(long_side(&before), 1024, "capped at 1024");
        assert_eq!(long_side(&photos.preview(f.a, 300).unwrap()), 300);
        assert_eq!(long_side(&photos.preview(f.b, 4000).unwrap()), 600, "never enlarged");

        drafts.patch(f.a, r#"{"develop":{"basic":{"exposure":1}}}"#, "Brighter").unwrap();
        let after = photos.preview(f.a, 4000).unwrap();
        assert_ne!(after, before, "the draft is applied");
        let Ok(tools::Output::Image(view, _)) = f.builtin("view", json!({ "id": f.a, "size": 1024 })) else { panic!() };
        assert_eq!(after, view.bytes, "the built-in view's render at 1024");
    }

    #[test]
    fn out_of_scope_refused() {
        let f = fixture();
        let (photos, drafts) = (f.photos(), f.drafts());
        for id in [f.c, 9_999] {
            let builtin = f.builtin("view", json!({ "id": id })).err().unwrap().message;
            assert_eq!(builtin, crate::mcp::scope::not_in_develop(id).message);
            assert_eq!(photos.metadata(id).err(), Some(builtin.clone()), "metadata {id}");
            assert_eq!(photos.preview(id, 512).err(), Some(builtin.clone()), "preview {id}");
            assert_eq!(photos.recipe(id).err(), Some(builtin.clone()), "recipe {id}");
            assert_eq!(drafts.patch(id, r#"{"develop":{"basic":{"exposure":1}}}"#, "x").err(), Some(builtin), "patch {id}");
        }
        assert_eq!((f.draft(f.c), f.history(f.c).len(), f.host.changed.load(Ordering::SeqCst)), (None, 0, 0));
    }

    #[test]
    fn drafts_refused_after_off() {
        let f = fixture();
        let (photos, drafts) = (f.photos(), f.drafts());
        drafts.patch(f.b, r#"{"develop":{"basic":{"exposure":0.5}}}"#, "One").unwrap();
        let kept = f.draft(f.b);
        assert!(kept.is_some());

        // Turned off between two calls (as when the switch flips while a plugin call runs): every
        // later call is refused, since each one reads the switch.
        f.host.on.store(false, Ordering::SeqCst);
        let err = drafts.patch(f.b, r#"{"develop":{"basic":{"exposure":2}}}"#, "Two").unwrap_err();
        assert!(err.contains("AI assistants"), "{err}");
        assert_eq!(f.draft(f.b), kept, "no draft written");
        assert_eq!(f.history(f.b), ["AI: Horizon: One"]);
        assert_eq!(photos.scope(), Vec::<i64>::new());
        assert!(photos.metadata(f.a).is_err() && photos.preview(f.a, 256).is_err() && photos.recipe(f.a).is_err());

        f.host.on.store(true, Ordering::SeqCst);
        assert!(drafts.patch(f.b, r#"{"develop":{"basic":{"exposure":2}}}"#, "Two").is_ok(), "not cached at construction");
    }

    #[test]
    fn refused_patch_unchanged() {
        let f = fixture();
        let drafts = f.drafts();
        drafts.patch(f.b, r#"{"develop":{"basic":{"shadows":0.2}}}"#, "Start").unwrap();
        let (kept, steps, notices) = (f.draft(f.b), f.history(f.b), f.host.changed.load(Ordering::SeqCst));

        let refused = |patch: Value| f.builtin("edit", json!({ "id": f.b, "patch": patch, "label": "x" })).err().unwrap().message;
        let cases = [
            (r#"{"develop":{"masks":[]}}"#, refused(json!({ "develop": { "masks": [] } }))),
            (r#"{"develop":{"basic":{"warmth":1}}}"#, refused(json!({ "develop": { "basic": { "warmth": 1 } } }))),
            ("[1]", refused(json!([1]))),
            ("not json", refused(json!("not json"))),
        ];
        for (patch, builtin) in cases {
            assert_eq!(drafts.patch(f.b, patch, "Bad").err(), Some(builtin), "{patch}");
        }
        assert_eq!((f.draft(f.b), f.history(f.b), f.host.changed.load(Ordering::SeqCst)), (kept, steps, notices), "draft unchanged");
        assert_eq!(f.edit_row(f.b), None);
    }

    #[test]
    fn history_label_is_one_short_line() {
        assert_eq!(history_label("Horizon", "  Level "), "AI: Horizon: Level");
        assert_eq!(history_label("Horizon", "a\nb\tc"), "AI: Horizon: a b c");
        assert_eq!(history_label("Horizon", " \n "), "AI: Horizon: edit", "as the built-in edit's empty label");
        let long = history_label("Horizon", &"é".repeat(500));
        assert_eq!(long.chars().count(), "AI: Horizon: ".len() + LABEL_MAX);
        // The plugin's name is the plugin author's text too.
        assert_eq!(history_label(" Hori\nzon\u{7} ", "Level"), "AI: Hori zon: Level");
        let named = history_label(&"n".repeat(500), "Level");
        assert_eq!(named, format!("AI: {}: Level", "n".repeat(PLUGIN_NAME_MAX)));
        assert_eq!(history_label(" \t ", "Level"), "AI: Plugin: Level");
    }

    #[test]
    fn patch_labels_history() {
        let f = fixture();
        let drafts = f.drafts();
        drafts.patch(f.b, r#"{"develop":{"basic":{"exposure":0.3}}}"#, "Level").unwrap();
        assert_eq!(f.history(f.b).last().map(String::as_str), Some("AI: Horizon: Level"));
        assert_eq!(f.host.changed.load(Ordering::SeqCst), 1, "the change notice the built-in edit sends");
        assert_eq!(f.draft(f.b).unwrap().develop.unwrap().basic.exposure, 0.3);
        assert_eq!(f.edit_row(f.b), None, "develop_drafts only, never edits");

        let Ok(_) = f.builtin("edit", json!({ "id": f.b, "patch": { "develop": { "basic": { "exposure": 0.4 } } }, "label": "same" })) else { panic!() };
        assert_eq!(f.host.changed.load(Ordering::SeqCst), 2, "the built-in edit sends the same one");

        // The photo open in Develop on screen changes through the page, which records the step.
        f.shared.scope.set_mounted(true);
        drafts.patch(f.a, r#"{"develop":{"basic":{"exposure":0.3}}}"#, "  Level  ").unwrap();
        assert_eq!(*f.host.asked.lock().unwrap(), ["AI: Horizon: Level"]);
        assert!(f.history(f.a).is_empty(), "the page records the step, not Rust");
        assert_eq!(f.edit_row(f.a), None);
    }

    #[test]
    fn short_name_used() {
        let p = mcp_prefixes(&["studio.jl.horizon", "studio.jl.bokeh"]);
        assert_eq!(p["studio.jl.horizon"], "horizon");
        assert_eq!(p["studio.jl.bokeh"], "bokeh");
        assert_eq!(mcp_name(&p["studio.jl.horizon"], "level_horizon"), "horizon__level_horizon");
    }

    #[test]
    fn collision_names_stable() {
        let a = mcp_prefixes(&["studio.jl.horizon", "other.horizon", "studio.jl.bokeh"]);
        let b = mcp_prefixes(&["studio.jl.bokeh", "other.horizon", "studio.jl.horizon", "other.horizon"]);
        assert_eq!(a, b, "the order and repeats of the input change nothing");
        assert_eq!(a["studio.jl.horizon"], "studio_jl_horizon");
        assert_eq!(a["other.horizon"], "other_horizon");
        assert_eq!(a["studio.jl.bokeh"], "bokeh", "only the colliding plugins use the full id");
    }

    #[test]
    fn same_last_segment_uses_full_ids() {
        let p = mcp_prefixes(&["b.y.tools", "a.x.tools"]);
        assert_eq!(p["a.x.tools"], "a_x_tools");
        assert_eq!(p["b.y.tools"], "b_y_tools");
    }

    #[test]
    fn prefixes_never_contain_the_separator() {
        // `valid_id` accepts empty segments (`a..b`, `trailing.`, `.leading`) and no `_`; a prefix
        // must hold either way (`clean` collapses and trims `_`, so two ids can end in one prefix:
        // `prefixes_shared_by_two_are_dropped`).
        for id in ["a._b", "a..b", "a.b_", "x__y", "trailing.", ".leading", "a-b.c-d"] {
            let p = mcp_prefixes(&[id, "other.b"]);
            assert!(!p[id].contains(SEPARATOR), "{id} -> {}", p[id]);
            assert!(!p[id].is_empty() && !p[id].starts_with('_') && !p[id].ends_with('_'), "{id} -> {}", p[id]);
            let name = mcp_name(&p[id], "tool");
            assert_eq!(parse_mcp_name(&name), Some((p[id].as_str(), "tool")));
        }
    }

    #[test]
    fn parse_round_trip() {
        let name = mcp_name("horizon", "level_horizon");
        assert_eq!(parse_mcp_name(&name), Some(("horizon", "level_horizon")));
        assert_eq!(parse_mcp_name("a__b__c"), Some(("a", "b__c")), "the first separator splits");
        assert_eq!(parse_mcp_name("a___x"), Some(("a", "_x")));
        assert_eq!(parse_mcp_name("__x"), None);
        assert_eq!(parse_mcp_name("a__"), None);
        assert_eq!(parse_mcp_name(""), None);
    }

    #[test]
    fn builtin_names_never_parsed() {
        for name in ["view", "edit"] {
            assert_eq!(parse_mcp_name(name), None, "{name}");
        }
    }

    #[test]
    fn builtin_names_have_no_separator() {
        let tools: Vec<String> = crate::mcp::tools::list().into_iter().map(|(n, ..)| n.to_string()).collect();
        let prompts: Vec<String> = crate::mcp::prompts().into_iter().map(|p| p.name.to_string()).collect();
        assert!(tools.len() > 5 && prompts.len() >= 2, "the real lists were read");
        for name in tools.iter().chain(&prompts) {
            assert!(!name.contains(SEPARATOR), "built-in {name} contains {SEPARATOR}");
            assert_eq!(parse_mcp_name(name), None, "{name}");
        }
    }

    // ---- Task 3: tools and prompts in the server ----

    use std::path::PathBuf;

    use photos_plugins::PluginState;

    const TOOLS_OK: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/crates/photos-plugins/tests/wasm/tools_ok.wasm");
    const ECHO_PERMS: &str = r#"["photos.read", "storage"]"#;
    const FILM: &str = "[[prompts]]\nname = \"film-look\"\ndescription = \"Retouch toward a warm film look\"\nfile = \"prompts/film-look.md\"\n";
    const FILM_TEXT: &str = "Warm the light like Portra 400, lift the blacks a little.";

    /// A Developer-mode plugin folder: `plugin.toml` with `perms`, a `[[tools]]` per name (and
    /// tools_ok's wasm when there is one) and `extra` tables; the film-look prompt's file.
    fn dev_plugin(root: &Path, id: &str, version: &str, perms: &str, tools: &[&str], extra: &str) -> PathBuf {
        let dir = root.join(id);
        std::fs::create_dir_all(dir.join("prompts")).unwrap();
        let mut toml = format!("id = \"{id}\"\nname = \"Echo\"\nversion = \"{version}\"\nauthor = \"T\"\ndescription = \"d\"\nlicense = \"MIT\"\nlumen = \">=26.0.0\"\napi = 1\npermissions = {perms}\n\n{extra}\n");
        for t in tools {
            toml.push_str(&format!("[[tools]]\nname = \"{t}\"\ndescription = \"The {t} tool\"\n\n"));
        }
        if !tools.is_empty() {
            std::fs::copy(TOOLS_OK, dir.join("plugin.wasm")).unwrap();
        }
        std::fs::write(dir.join("plugin.toml"), toml).unwrap();
        std::fs::write(dir.join("prompts/film-look.md"), FILM_TEXT).unwrap();
        dir
    }

    /// Plugins in Developer mode in a temp app dir, with these folders loaded.
    fn plugins_with(folders: &[PathBuf]) -> (tempfile::TempDir, Arc<Plugins>) {
        let app = tempfile::tempdir().unwrap();
        let plugins = Arc::new(Plugins::load(app.path()));
        plugins.write().set_developer_mode(true).unwrap();
        let ctx = plugins.ctx();
        for f in folders {
            plugins.write().load_dev(f, &ctx).unwrap_or_else(|e| panic!("{}: {e}", f.display()));
        }
        (app, plugins)
    }

    /// The `com.test.echo` plugin (tools_ok's `echo` and `record`, the film-look prompt) in a
    /// fixture.
    fn echo_fixture() -> (tempfile::TempDir, tempfile::TempDir, Arc<Plugins>, Fixture) {
        let root = tempfile::tempdir().unwrap();
        let (app, plugins) = plugins_with(&[dev_plugin(root.path(), "com.test.echo", "1.0.0", ECHO_PERMS, &["echo", "record"], FILM)]);
        let f = fixture_with(Some(plugins.clone()));
        (root, app, plugins, f)
    }

    fn state_of(plugins: &Plugins, id: &str) -> PluginState {
        plugins.read().list().into_iter().find(|p| p.id == id).unwrap().state
    }

    fn builtins() -> Vec<String> {
        tools::list().into_iter().map(|(n, ..)| n.to_string()).collect()
    }

    std::thread_local! {
        static CAPTURE: std::cell::RefCell<Option<Vec<u8>>> = const { std::cell::RefCell::new(None) };
    }

    /// Writes to the calling thread's capture, if it has one.
    struct ThreadBuf;

    impl std::io::Write for ThreadBuf {
        fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
            CAPTURE.with(|c| c.borrow_mut().as_mut().map(|v| v.extend_from_slice(b)));
            Ok(b.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    /// What `f` logged on this thread, as Lumen's log file would show it. The subscriber is the
    /// test binary's global one: a scoped one (`with_default`) misses events when another test's
    /// thread registers the same callsite first.
    fn captured<R>(f: impl FnOnce() -> R) -> (R, String) {
        static INIT: std::sync::Once = std::sync::Once::new();
        INIT.call_once(|| {
            // If another test in this binary already set a global subscriber, it keeps its place.
            let _ = tracing_subscriber::fmt().with_ansi(false).with_max_level(tracing::Level::INFO).with_writer(|| ThreadBuf).try_init();
        });
        CAPTURE.with(|c| *c.borrow_mut() = Some(Vec::new()));
        let r = f();
        let text = CAPTURE.with(|c| c.borrow_mut().take()).unwrap_or_default();
        (r, String::from_utf8(text).unwrap())
    }

    #[test]
    fn tools_list_includes_plugin() {
        let (_root, _app, _plugins, f) = echo_fixture();
        let tools = crate::mcp::listed_tools(&f.dyn_host(), &f.shared);
        let names: Vec<String> = tools.iter().map(|t| t.name.to_string()).collect();
        assert_eq!(names[..builtins().len()], builtins()[..], "the built-ins first, as before");
        assert_eq!(names[builtins().len()..], ["echo__echo", "echo__record"]);
        let echo = tools.iter().find(|t| t.name == "echo__echo").unwrap();
        assert_eq!(Value::Object((*echo.input_schema).clone()), json!({ "type": "object" }), "the fixture's schema");
        assert_eq!(echo.description.as_deref(), Some("The echo tool"), "plugin.toml's description");
    }

    #[test]
    fn call_routes_to_plugin() {
        let (_root, _app, _plugins, f) = echo_fixture();
        let (out, log) = captured(|| f.call("echo__echo", json!({ "a": 1 })));
        let Ok(tools::Output::Text(text)) = out else { panic!("not text") };
        assert_eq!(text, r#"{"args":{"a":1},"scope":2}"#, "photos.scope() is the filmstrip");
        let lines: Vec<&str> = log.lines().filter(|l| l.contains("mcp call")).collect();
        assert_eq!(lines.len(), 1, "{log}");
        assert!(lines[0].contains("tool=echo__echo") && lines[0].contains("ok=true") && lines[0].contains("ms="), "{}", lines[0]);

        // Only a listed plugin tool is called; anything else is the built-in unknown-tool error.
        for name in ["echo__nothing", "nope__echo", "echo__Echo", "echo__echo__x"] {
            let (out, log) = captured(|| f.call(name, json!({})));
            let err = out.err().unwrap_or_else(|| panic!("{name}"));
            assert_eq!((err.code.as_str(), err.message), ("unknown_tool", tools::unknown(name).message), "{name}");
            assert!(log.lines().any(|l| l.contains("mcp call") && l.contains("ok=false")), "{log}");
        }
    }

    #[test]
    fn manifest_mismatch_problem() {
        let root = tempfile::tempdir().unwrap();
        let (_app, plugins) = plugins_with(&[dev_plugin(root.path(), "com.test.echo", "1.0.0", ECHO_PERMS, &["echo2", "record"], "")]);
        let f = fixture_with(Some(plugins.clone()));
        let listed = f.listed();
        assert_eq!(listed, builtins(), "the plugin's tools are not listed, the built-ins are");
        assert_eq!(state_of(&plugins, "com.test.echo"), PluginState::Problem { reason: "tools don't match plugin.toml".into() });
        assert!(plugins.log_lines("com.test.echo").iter().any(|l| l.contains("tools don't match plugin.toml")));
        assert_eq!(f.call("echo__echo2", json!({})).err().unwrap().code, "unknown_tool");
        assert_eq!(f.call("echo__record", json!({})).err().unwrap().code, "unknown_tool");
    }

    #[test]
    fn checked_tools_match_and_are_objects() {
        let decls = |names: &[&str]| -> Vec<(String, String)> { names.iter().map(|n| (n.to_string(), format!("The {n} tool"))).collect() };
        let tool = |name: &str, schema: &str| Tool { name: name.into(), input_schema: schema.into() };
        const OBJ: &str = r#"{"type":"object"}"#;
        let ok = checked(&decls(&["echo", "record"]), vec![tool("record", r#"{"type":"object","properties":{}}"#), tool("echo", OBJ)]).unwrap();
        assert_eq!(ok, [("record".to_string(), json!({ "type": "object", "properties": {} })), ("echo".to_string(), json!({ "type": "object" }))], "the plugin's order");
        let mismatch = Err(TOOLS_MISMATCH.to_string());
        assert_eq!(checked(&decls(&["echo"]), vec![tool("echo", OBJ), tool("record", OBJ)]), mismatch);
        assert_eq!(checked(&decls(&["echo", "record"]), vec![tool("echo", OBJ)]), mismatch);
        assert_eq!(checked(&decls(&["echo", "record"]), vec![tool("echo", OBJ), tool("echo", OBJ)]), mismatch, "a name twice");
        // Not JSON, not an object, or an object that isn't an object schema (`"type":"object"`).
        let not_object = ["not json", "[1]", "\"object\"", "null", "", "{\"type\":", "{}", r#"{"type":"string"}"#, r#"{"type":["object"]}"#, r#"{"type":null}"#, r#"{"properties":{}}"#];
        for bad in not_object {
            let err = checked(&decls(&["echo"]), vec![tool("echo", bad)]).unwrap_err();
            assert!(err.contains("echo") && err.contains("schema"), "{bad}: {err}");
        }
    }

    #[test]
    fn oversized_schema_refused() {
        let decls = vec![("echo".to_string(), "d".to_string())];
        let with_pad = |n: usize| Tool { name: "echo".into(), input_schema: format!(r#"{{"type":"object","description":"{}"}}"#, "x".repeat(n)) };
        let fixed = with_pad(0).input_schema.len();
        let at_cap = with_pad(SCHEMA_MAX - fixed);
        assert_eq!(at_cap.input_schema.len(), SCHEMA_MAX);
        assert!(checked(&decls, vec![at_cap]).is_ok(), "64 KiB is allowed");
        let err = checked(&decls, vec![with_pad(SCHEMA_MAX - fixed + 1)]).unwrap_err();
        assert!(err.contains("echo") && err.contains("64 KiB"), "{err}");

        // Through the seam: the one plugin is a Problem, the other still lists.
        let root = tempfile::tempdir().unwrap();
        let folders = [dev_plugin(root.path(), "com.test.echo", "1.0.0", ECHO_PERMS, &["echo", "record"], ""), dev_plugin(root.path(), "com.test.big", "1.0.0", ECHO_PERMS, &["echo", "record"], "")];
        let (_app, plugins) = plugins_with(&folders);
        let f = fixture_with(Some(plugins.clone()));
        let exported = |o: &Offer| -> AppResult<Vec<Tool>> {
            let pad = if o.id == "com.test.big" { SCHEMA_MAX } else { 0 };
            Ok(["echo", "record"].map(|n| Tool { name: n.into(), input_schema: with_pad(pad).input_schema }).to_vec())
        };
        let listed: Vec<String> = plugin_tools_with(f.host.as_ref(), &f.shared, &plugins, &exported).into_iter().map(|t| t.name).collect();
        assert_eq!(listed, ["echo__echo", "echo__record"]);
        let PluginState::Problem { reason } = state_of(&plugins, "com.test.big") else { panic!("not a problem") };
        assert!(reason.contains("64 KiB"), "{reason}");
        assert_eq!(state_of(&plugins, "com.test.echo"), PluginState::Ready);
        assert_eq!(f.listed()[..builtins().len()], builtins()[..], "the built-ins list as usual");
    }

    #[test]
    fn bad_schema_isolated() {
        let root = tempfile::tempdir().unwrap();
        let folders = [
            dev_plugin(root.path(), "com.test.echo", "1.0.0", ECHO_PERMS, &["echo", "record"], ""),
            dev_plugin(root.path(), "com.test.second", "1.0.0", ECHO_PERMS, &["echo", "record"], ""),
        ];
        let (_app, plugins) = plugins_with(&folders);
        let f = fixture_with(Some(plugins.clone()));
        // `second`'s tools() gives a schema that isn't JSON; `echo`'s are fine.
        let exported = |o: &Offer| -> AppResult<Vec<Tool>> {
            let schema = if o.id == "com.test.second" { "{ not json" } else { r#"{"type":"object"}"# };
            Ok(["echo", "record"].map(|n| Tool { name: n.into(), input_schema: schema.into() }).to_vec())
        };
        let listed: Vec<String> = plugin_tools_with(f.host.as_ref(), &f.shared, &plugins, &exported).into_iter().map(|t| t.name).collect();
        assert_eq!(listed, ["echo__echo", "echo__record"]);
        let PluginState::Problem { reason } = state_of(&plugins, "com.test.second") else { panic!("{:?}", state_of(&plugins, "com.test.second")) };
        assert!(reason.contains("schema"), "{reason}");
        assert_eq!(state_of(&plugins, "com.test.echo"), PluginState::Ready);

        // The server's list: the built-ins and the other plugin, never `second`.
        let all = f.listed();
        assert_eq!(all[..builtins().len()], builtins()[..]);
        assert_eq!(all[builtins().len()..], ["echo__echo", "echo__record"]);
        assert_eq!(f.text("echo__echo", json!({})), r#"{"args":{},"scope":2}"#);
    }

    #[test]
    fn result_truncated() {
        // Cut on a character boundary: 1 MiB of text, then the marker.
        let even = truncated("é".repeat(RESULT_MAX));
        assert_eq!(even.len(), RESULT_MAX + TRUNCATED.len());
        assert!(even.ends_with("é…(truncated by Lumen)"));
        let odd = truncated(format!("a{}", "é".repeat(RESULT_MAX)));
        assert_eq!(odd.len(), RESULT_MAX - 1 + TRUNCATED.len(), "never half a character");
        let exact = "a".repeat(RESULT_MAX);
        assert_eq!(truncated(exact.clone()), exact, "1 MiB is not over 1 MiB");

        // Through the server: a 2 MB result from the plugin.
        let (_root, _app, _plugins, f) = echo_fixture();
        let text = f.text("echo__echo", json!({ "s": "x".repeat(2 << 20) }));
        assert_eq!(text.len(), RESULT_MAX + TRUNCATED.len());
        assert!(text.starts_with(r#"{"args":{"s":"xxx"#) && text.ends_with("x…(truncated by Lumen)"));
    }

    #[test]
    fn off_means_absent() {
        let (_root, _app, _plugins, f) = echo_fixture();
        let on_tools = f.listed();
        assert!(on_tools.contains(&"echo__echo".to_string()) && f.prompts().contains(&"echo__film-look".to_string()));

        f.host.on.store(false, Ordering::SeqCst);
        assert_eq!(f.listed(), builtins());
        assert_eq!(f.prompts(), ["edit_photo", "retouch"]);
        assert!(plugin_tools(&f.dyn_host(), &f.shared).is_empty() && plugin_prompts(f.host.as_ref(), &f.shared).is_empty());
        for name in ["echo__echo", "list_photos"] {
            let err = f.call(name, json!({})).err().unwrap();
            assert_eq!((err.code.as_str(), err.message.as_str()), ("assistants_off", assistants_off().message.as_str()), "{name}");
        }
        assert_eq!(crate::mcp::prompt_get(f.host.as_ref(), &f.shared, "echo__film-look", None), None);

        f.host.on.store(true, Ordering::SeqCst);
        assert_eq!(f.listed(), on_tools, "back, from the cache");
    }

    #[test]
    fn plugin_prompt_listed() {
        let (_root, _app, _plugins, f) = echo_fixture();
        let prompts = crate::mcp::listed_prompts(f.host.as_ref(), &f.shared);
        let names: Vec<&str> = prompts.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["edit_photo", "retouch", "echo__film-look"]);
        assert_eq!(prompts[2].description.as_deref(), Some("Retouch toward a warm film look"));
        assert_eq!(crate::mcp::prompt_get(f.host.as_ref(), &f.shared, "echo__film-look", None).as_deref(), Some(FILM_TEXT));
        assert_eq!(crate::mcp::prompt_get(f.host.as_ref(), &f.shared, "echo__other", None), None);
        assert!(crate::mcp::prompt_get(f.host.as_ref(), &f.shared, "edit_photo", Some("moody")).unwrap().contains("moody"), "built-ins unchanged");
    }

    #[test]
    fn hosts_follow_grants() {
        let root = tempfile::tempdir().unwrap();
        let folders = [
            dev_plugin(root.path(), "com.test.echo", "1.0.0", ECHO_PERMS, &["echo", "record"], ""),
            dev_plugin(root.path(), "com.test.writer", "1.0.0", r#"["photos.read", "storage", "develop.write-drafts"]"#, &["echo", "record"], ""),
            dev_plugin(root.path(), "com.test.bare", "1.0.0", "[]", &[], FILM),
        ];
        let (_app, plugins) = plugins_with(&folders);
        let f = fixture_with(Some(plugins.clone()));
        let hosts = |id: &str| {
            let ctx = tools_ctx(&plugins, &f.shared, id).unwrap();
            (ctx.photos.is_some(), ctx.drafts.is_some())
        };
        assert_eq!(hosts("com.test.echo"), (true, false), "photos.read only");
        assert_eq!(hosts("com.test.writer"), (true, true));
        assert_eq!(hosts("com.test.bare"), (false, false), "nothing granted, no hosts");
        assert!(tools_ctx(&plugins, &f.shared, "com.test.gone").is_none());
        let ctx = tools_ctx(&plugins, &f.shared, "com.test.echo").unwrap();
        assert_eq!((ctx.plugin.as_str(), ctx.data_dir.clone()), ("com.test.echo", plugins.read().data_dir("com.test.echo")));
    }

    #[test]
    fn unsafe_names_never_listed() {
        let builtin: BTreeSet<String> = builtin_names();
        assert!(builtin.contains("view") && builtin.contains("edit_photo"), "tools and prompts");
        assert_eq!(refused("echo__echo", "echo", &builtin), None);
        assert_eq!(refused("a-b__film-look", "a-b", &builtin), None);
        assert_eq!(refused(&format!("{}__ab", "p".repeat(60)), &"p".repeat(60), &builtin), None, "64 is fine");
        for (name, prefix) in [("view", "x"), ("edit_photo", "x"), ("__echo", ""), (&*format!("{}__abc", "p".repeat(60)), &*"p".repeat(60)), ("é__echo", "é"), ("a b__echo", "a b"), ("a.b__echo", "a.b")] {
            assert!(refused(name, prefix, &builtin).is_some(), "{name}");
        }

        // A plugin whose names come out over 64 characters lists none of them, says why in its
        // log, and its tools can't be called.
        let root = tempfile::tempdir().unwrap();
        let id = format!("com.{}", "a".repeat(60));
        let (_app, plugins) = plugins_with(&[dev_plugin(root.path(), &id, "1.0.0", ECHO_PERMS, &["echo", "record"], FILM)]);
        let f = fixture_with(Some(plugins.clone()));
        assert_eq!(f.listed(), builtins());
        assert_eq!(f.prompts(), ["edit_photo", "retouch"]);
        let name = format!("{}__echo", "a".repeat(60));
        assert_eq!(f.call(&name, json!({})).err().unwrap().code, "unknown_tool");
        let log = plugins.log_lines(&id);
        assert!(log.iter().any(|l| l.contains("echo") && l.contains("64")), "{log:?}");
        assert!(log.iter().any(|l| l.contains("film-look") && l.contains("64")), "{log:?}");

        // Said once per change, not on every listing.
        let said = |plugins: &Plugins| plugins.log_lines(&id).iter().filter(|l| l.contains("film-look")).count();
        f.prompts();
        f.prompts();
        assert_eq!(said(&plugins), 1, "{:?}", plugins.log_lines(&id));
        f.shared.plugin_tools.clear();
        f.prompts();
        assert_eq!(said(&plugins), 2, "again after a change");
    }

    #[test]
    fn cache_per_version_cleared() {
        let root = tempfile::tempdir().unwrap();
        let folders = [
            dev_plugin(root.path(), "com.test.echo", "1.0.0", ECHO_PERMS, &["echo", "record"], ""),
            dev_plugin(root.path(), "com.test.second", "1.0.0", ECHO_PERMS, &["echo", "record"], ""),
        ];
        let (_app, plugins) = plugins_with(&folders);
        let f = fixture_with(Some(plugins.clone()));
        let reads = Mutex::new(Vec::<String>::new());
        let exported = |o: &Offer| -> AppResult<Vec<Tool>> {
            reads.lock().unwrap().push(format!("{}@{}", o.id, o.version));
            Ok(["echo", "record"].map(|n| Tool { name: n.into(), input_schema: r#"{"type":"object"}"#.into() }).to_vec())
        };
        let list = || plugin_tools_with(f.host.as_ref(), &f.shared, &plugins, &exported).len();
        assert_eq!((list(), list()), (4, 4));
        assert_eq!(*reads.lock().unwrap(), ["com.test.echo@1.0.0", "com.test.second@1.0.0"], "read once per (id, version)");

        // A new version is read again.
        let toml = folders[0].join("plugin.toml");
        std::fs::write(&toml, std::fs::read_to_string(&toml).unwrap().replace("version = \"1.0.0\"", "version = \"1.0.1\"")).unwrap();
        let ctx = plugins.ctx();
        plugins.write().load_dev(&folders[0], &ctx).unwrap();
        list();
        assert_eq!(reads.lock().unwrap().last().map(String::as_str), Some("com.test.echo@1.0.1"));
        assert_eq!(reads.lock().unwrap().len(), 3);

        // Any change of the plugins clears it, once the server follows them.
        follow_changes(&plugins, f.shared.clone());
        plugins.mark_problem("com.test.second", "test");
        assert_eq!(list(), 2);
        assert_eq!(reads.lock().unwrap().len(), 4, "echo read again after the change");
        f.shared.plugin_tools.clear();
        list();
        assert_eq!(reads.lock().unwrap().len(), 5);
    }

    /// A second host on the fixture's library, plugins and switch state: what `McpState::follow`
    /// builds on every server start.
    fn another_host(f: &Fixture) -> Arc<PluginTestHost> {
        Arc::new(PluginTestHost { lib: f.lib.clone(), shared: f.shared.clone(), on: AtomicBool::new(true), changed: AtomicUsize::new(0), asked: Mutex::new(Vec::new()), plugins: f.host.plugins.clone() })
    }

    fn writer_fixture() -> (tempfile::TempDir, tempfile::TempDir, Arc<Plugins>, Fixture) {
        let root = tempfile::tempdir().unwrap();
        let perms = r#"["photos.read", "storage", "develop.write-drafts"]"#;
        let (app, plugins) = plugins_with(&[dev_plugin(root.path(), "com.test.writer", "1.0.0", perms, &["echo", "record"], "")]);
        let f = fixture_with(Some(plugins.clone()));
        (root, app, plugins, f)
    }

    #[test]
    fn hosts_survive_a_server_restart() {
        let (_root, _app, plugins, f) = writer_fixture();
        // The context closure the engine keeps for the life of the process.
        let ctx_for = ctx_fn(&plugins, &f.shared, "com.test.writer");

        // Server start 1: its host is put in the slot, then the server stops and its host is gone.
        let first = another_host(&f);
        f.shared.host.set(&(first.clone() as Arc<dyn Host>));
        assert!(ctx_for().photos.is_some());
        drop(first);
        f.shared.host.clear();

        // Server start 2 (assistants off then on, or a new port or key), then an instance rebuild
        // calls the same closure: the grants and both hosts are there, and they work.
        let second = another_host(&f);
        f.shared.host.set(&(second.clone() as Arc<dyn Host>));
        let ctx = ctx_for();
        assert_eq!(ctx.granted.len(), 3, "grants: {:?}", ctx.granted);
        let (photos, drafts) = (ctx.photos.expect("photos host"), ctx.drafts.expect("drafts host"));
        assert_eq!(photos.scope(), vec![f.b, f.a]);
        assert!(photos.metadata(f.a).is_ok());
        drafts.patch(f.b, r#"{"develop":{"basic":{"exposure":0.5}}}"#, "After restart").unwrap();
        assert_eq!(f.history(f.b), ["AI: Echo: After restart"]);
        assert_eq!(f.edit_row(f.b), None, "develop_drafts only");

        // The engine's cached instance keeps working through the restart.
        let text = match crate::mcp::run_call(&(second.clone() as Arc<dyn Host>), &f.shared, "writer__echo", &Map::new()) {
            Ok(tools::Output::Text(t)) => t,
            other => panic!("{:?}", other.err().map(|e| e.message)),
        };
        assert_eq!(text, r#"{"args":{},"scope":2}"#);
    }

    #[test]
    fn hosts_fail_closed_without_a_host() {
        let (_root, _app, plugins, f) = writer_fixture();
        let ctx_for = ctx_fn(&plugins, &f.shared, "com.test.writer");

        // No server running (assistants off): the grants stay, nothing reaches a photo.
        f.shared.host.clear();
        let ctx = ctx_for();
        assert_eq!(ctx.granted.len(), 3);
        let (photos, drafts) = (ctx.photos.unwrap(), ctx.drafts.unwrap());
        assert_eq!(photos.scope(), Vec::<i64>::new());
        let refused = assistants_off().message;
        assert_eq!(photos.metadata(f.a).err(), Some(refused.clone()));
        assert_eq!(photos.preview(f.a, 256).err(), Some(refused.clone()));
        assert_eq!(photos.recipe(f.a).err(), Some(refused.clone()));
        assert_eq!(drafts.patch(f.b, r#"{"develop":{"basic":{"exposure":0.5}}}"#, "x").err(), Some(refused));
        assert_eq!((f.draft(f.b), f.history(f.b).len()), (None, 0));

        // A host that is dropped without being cleared reads the same.
        let gone = another_host(&f);
        f.shared.host.set(&(gone.clone() as Arc<dyn Host>));
        drop(gone);
        assert_eq!(ctx_for().photos.unwrap().scope(), Vec::<i64>::new());

        // The switch off while the host is still there: refused too.
        let live = another_host(&f);
        f.shared.host.set(&(live.clone() as Arc<dyn Host>));
        live.on.store(false, Ordering::SeqCst);
        assert_eq!(ctx_for().photos.unwrap().scope(), Vec::<i64>::new());
    }

    #[test]
    fn plugin_error_is_one_capped_line() {
        use photos_plugins::PluginError;
        let e = plugin_error(PluginError::invalid("first\nsecond\r\n\tthird"));
        assert_eq!((e.code.as_str(), e.message.as_str()), ("plugin_invalid", "first second   third"));
        assert_eq!(plugin_error(PluginError::invalid("  short  ")).message, "short");

        let long = plugin_error(PluginError::invalid(format!("{}\n{}", "é".repeat(5000), "tail"))).message;
        assert!(long.len() <= ERROR_MAX, "{} bytes", long.len());
        assert!(!long.contains('\n') && long.ends_with('…') && long.starts_with("éé"));
        let odd = plugin_error(PluginError::invalid(format!("a{}", "é".repeat(5000)))).message;
        assert!(odd.len() <= ERROR_MAX && odd.is_char_boundary(odd.len()), "never half a character");
        let exact = "x".repeat(ERROR_MAX);
        assert_eq!(plugin_error(PluginError::invalid(exact.clone())).message, exact, "at the cap, untouched");
    }

    #[test]
    fn failed_tools_read_keeps_the_plugin_enabled() {
        let root = tempfile::tempdir().unwrap();
        let folders = [
            dev_plugin(root.path(), "com.test.echo", "1.0.0", ECHO_PERMS, &["echo", "record"], ""),
            dev_plugin(root.path(), "com.test.broken", "1.0.0", ECHO_PERMS, &["echo", "record"], FILM),
        ];
        let (_app, plugins) = plugins_with(&folders);
        let f = fixture_with(Some(plugins.clone()));
        let reads = Mutex::new(Vec::<String>::new());
        let exported = |o: &Offer| -> AppResult<Vec<Tool>> {
            reads.lock().unwrap().push(o.id.clone());
            if o.id == "com.test.broken" {
                return Err(AppError { code: "plugin_trap".into(), message: "it trapped\nat wasm backtrace".into(), detail: None });
            }
            Ok(["echo", "record"].map(|n| Tool { name: n.into(), input_schema: r#"{"type":"object"}"#.into() }).to_vec())
        };
        let list = || -> Vec<String> { plugin_tools_with(f.host.as_ref(), &f.shared, &plugins, &exported).into_iter().map(|t| t.name).collect() };
        let broken_reads = || reads.lock().unwrap().iter().filter(|i| *i == "com.test.broken").count();
        assert_eq!(list(), ["echo__echo", "echo__record"]);
        // Only its tools are lost: its themes, presets, looks and prompts stay (it is still Ready).
        assert_eq!(state_of(&plugins, "com.test.broken"), PluginState::Ready);
        assert_eq!(f.prompts(), ["edit_photo", "retouch", "broken__film-look"]);
        let log = plugins.log_lines("com.test.broken");
        assert!(log.iter().any(|l| l.contains("it trapped at wasm backtrace")), "{log:?}");

        // Not read again, nor logged again, on the next listings: the failure is cached.
        assert_eq!(list(), ["echo__echo", "echo__record"]);
        assert_eq!(list().len(), 2);
        assert_eq!(broken_reads(), 1, "{:?}", reads.lock().unwrap());
        assert_eq!(plugins.log_lines("com.test.broken").iter().filter(|l| l.contains("it trapped")).count(), 1);

        // Any change of the plugins clears it and the plugin is tried again.
        f.shared.plugin_tools.clear();
        list();
        assert_eq!(broken_reads(), 2);
    }

    #[test]
    fn prefixes_shared_by_two_are_dropped() {
        // `valid_id` accepts empty segments and `clean` collapses `_`: both end in `studio_jl_horizon`.
        let p = mcp_prefixes(&["studio.jl.horizon", "studio.jl..horizon", "studio.jl.bokeh"]);
        assert_eq!(p.keys().map(String::as_str).collect::<Vec<_>>(), ["studio.jl.bokeh"], "neither of the two, whatever the order");
        assert_eq!(p, mcp_prefixes(&["studio.jl.bokeh", "studio.jl..horizon", "studio.jl.horizon"]));
        let q = mcp_prefixes(&["horizon", ".horizon"]);
        assert!(q.is_empty(), "{q:?}");
        let plan = prefixes_of(&["studio.jl.horizon".into(), "studio.jl..horizon".into()]);
        assert_eq!(plan.clashed.len(), 2);
        assert!(plan.clashed.values().all(|p| p == "studio_jl_horizon"));
    }

    #[test]
    fn prefix_clash_lists_neither() {
        let root = tempfile::tempdir().unwrap();
        let folders = [
            dev_plugin(root.path(), "studio.jl.horizon", "1.0.0", ECHO_PERMS, &["echo", "record"], FILM),
            dev_plugin(root.path(), "studio.jl..horizon", "1.0.0", ECHO_PERMS, &["echo", "record"], FILM),
            dev_plugin(root.path(), "com.test.echo", "1.0.0", ECHO_PERMS, &["echo", "record"], ""),
        ];
        let (_app, plugins) = plugins_with(&folders);
        let f = fixture_with(Some(plugins.clone()));
        // The impostor must not shadow the first one, nor the other way round: neither is listed.
        assert_eq!(f.listed()[builtins().len()..], ["echo__echo", "echo__record"], "the unrelated plugin lists");
        assert_eq!(f.listed()[..builtins().len()], builtins()[..], "the built-ins too");
        assert_eq!(f.prompts(), ["edit_photo", "retouch"]);
        for name in ["studio_jl_horizon__echo", "studio_jl_horizon__record"] {
            assert_eq!(f.call(name, json!({})).err().unwrap().code, "unknown_tool", "{name}");
        }
        assert_eq!(crate::mcp::prompt_get(f.host.as_ref(), &f.shared, "studio_jl_horizon__film-look", None), None);
        // Each says why in its own log, once however often the list is read.
        for id in ["studio.jl.horizon", "studio.jl..horizon"] {
            let log = plugins.log_lines(id);
            assert_eq!(log.iter().filter(|l| l.contains("studio_jl_horizon")).count(), 1, "{id}: {log:?}");
            assert_eq!(state_of(&plugins, id), PluginState::Ready, "only unlisted, not a Problem");
        }
        // One of them off: the other is the only one with that prefix and lists.
        plugins.write().set_enabled("studio.jl..horizon", false).unwrap();
        f.shared.plugin_tools.clear();
        assert!(f.listed().contains(&"horizon__echo".to_string()), "{:?}", f.listed());
        assert!(f.prompts().contains(&"horizon__film-look".to_string()));
    }

    #[test]
    fn listing_is_capped() {
        // Count, total schema bytes and description length, in `checked` and through the seam.
        let names: Vec<String> = (0..=TOOLS_MAX).map(|i| format!("t{i}")).collect();
        let decls = |n: usize| -> Vec<(String, String)> { names[..n].iter().map(|n| (n.clone(), "d".into())).collect() };
        let tools = |n: usize, schema: &str| -> Vec<Tool> { names[..n].iter().map(|n| Tool { name: n.clone(), input_schema: schema.into() }).collect() };
        const OBJ: &str = r#"{"type":"object"}"#;
        assert_eq!(checked(&decls(TOOLS_MAX), tools(TOOLS_MAX, OBJ)).unwrap().len(), TOOLS_MAX, "32 is allowed");
        let err = checked(&decls(TOOLS_MAX + 1), tools(TOOLS_MAX + 1, OBJ)).unwrap_err();
        assert!(err.contains("32"), "{err}");

        let pad = |n: usize| format!(r#"{{"type":"object","description":"{}"}}"#, "x".repeat(n));
        let fixed = pad(0).len();
        let at_cap = pad(SCHEMA_MAX - fixed);
        assert_eq!(at_cap.len(), SCHEMA_MAX);
        let fits = SCHEMAS_MAX / SCHEMA_MAX;
        assert!(checked(&decls(fits), tools(fits, &at_cap)).is_ok(), "{fits} schemas of 64 KiB are exactly 256 KiB");
        let err = checked(&decls(fits + 1), tools(fits + 1, &at_cap)).unwrap_err();
        assert!(err.contains("256 KiB"), "{err}");

        let long = format!("{}é", "a".repeat(DESCRIPTION_MAX - 1));
        assert_eq!(long.len(), DESCRIPTION_MAX + 1);
        assert_eq!(cut_description(&long), "a".repeat(DESCRIPTION_MAX - 1), "back to a character boundary, not over 1 KiB");
        assert_eq!(cut_description("short"), "short");
        assert_eq!(cut_description(&"a".repeat(DESCRIPTION_MAX)).len(), DESCRIPTION_MAX);

        // Through the seam: too many tools is a Problem; a long description is cut, not a Problem.
        let root = tempfile::tempdir().unwrap();
        let folders = [dev_plugin(root.path(), "com.test.many", "1.0.0", ECHO_PERMS, &["echo", "record"], ""), dev_plugin(root.path(), "com.test.echo", "1.0.0", ECHO_PERMS, &["echo", "record"], "")];
        let toml = folders[1].join("plugin.toml");
        std::fs::write(&toml, std::fs::read_to_string(&toml).unwrap().replace("The echo tool", &"é".repeat(3000))).unwrap();
        let (_app, plugins) = plugins_with(&folders);
        let f = fixture_with(Some(plugins.clone()));
        let exported = |o: &Offer| -> AppResult<Vec<Tool>> {
            Ok(if o.id == "com.test.many" { tools(TOOLS_MAX + 1, OBJ) } else { ["echo", "record"].map(|n| Tool { name: n.into(), input_schema: OBJ.into() }).to_vec() })
        };
        let listed = plugin_tools_with(f.host.as_ref(), &f.shared, &plugins, &exported);
        assert_eq!(listed.iter().map(|t| t.name.as_str()).collect::<Vec<_>>(), ["echo__echo", "echo__record"]);
        assert!(listed[0].description.len() <= DESCRIPTION_MAX && listed[0].description.starts_with('é') && listed[1].description == "The record tool");
        let PluginState::Problem { reason } = state_of(&plugins, "com.test.many") else { panic!("not a problem") };
        assert!(reason.contains("32"), "{reason}");
        assert_eq!(state_of(&plugins, "com.test.echo"), PluginState::Ready, "a cut description is not a Problem");
    }

    #[test]
    fn prompts_are_capped_and_described_briefly() {
        let prompt = |name: &str, description: &str| format!("[[prompts]]\nname = \"{name}\"\ndescription = \"{description}\"\nfile = \"prompts/film-look.md\"\n\n");
        let root = tempfile::tempdir().unwrap();
        let many: String = (0..=PROMPTS_MAX).map(|i| prompt(&format!("p{i}"), "d")).collect();
        let exact: String = (0..PROMPTS_MAX).map(|i| prompt(&format!("p{i}"), "d")).collect();
        let twice = format!("{}{}", prompt("look", &"é".repeat(3000)), prompt("other", "d"));
        let folders = [
            dev_plugin(root.path(), "com.test.many", "1.0.0", "[]", &[], &many),
            dev_plugin(root.path(), "com.test.exact", "1.0.0", "[]", &[], &exact),
            dev_plugin(root.path(), "com.test.twice", "1.0.0", "[]", &[], &twice),
        ];
        let (_app, plugins) = plugins_with(&folders);
        let f = fixture_with(Some(plugins.clone()));
        let listed = crate::mcp::listed_prompts(f.host.as_ref(), &f.shared);
        let names: Vec<&str> = listed.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names.iter().filter(|n| n.starts_with("exact__")).count(), PROMPTS_MAX, "32 is allowed");
        assert!(!names.iter().any(|n| n.starts_with("many__")), "33 is too many: none of them");
        let PluginState::Problem { reason } = state_of(&plugins, "com.test.many") else { panic!("not a problem") };
        assert!(reason.contains("32"), "{reason}");
        assert_eq!(state_of(&plugins, "com.test.exact"), PluginState::Ready);
        // The description is cut to 1 KiB, not refused. (A name twice in one manifest never gets
        // here: the manifest refuses it; the listing drops one all the same.)
        assert!(names.contains(&"twice__look") && names.contains(&"twice__other"), "{names:?}");
        let look = listed.iter().find(|p| p.name == "twice__look").unwrap();
        assert!(look.description.as_deref().is_some_and(|d| d.len() <= DESCRIPTION_MAX && d.starts_with('é')));
    }

    #[test]
    fn hosts_refuse_a_plugin_turned_off_or_not_granted() {
        let root = tempfile::tempdir().unwrap();
        let folders = [
            dev_plugin(root.path(), "com.test.writer", "1.0.0", r#"["photos.read", "storage", "develop.write-drafts"]"#, &["echo", "record"], ""),
            dev_plugin(root.path(), "com.test.reader", "1.0.0", ECHO_PERMS, &["echo", "record"], ""),
        ];
        let (_app, plugins) = plugins_with(&folders);
        let f = fixture_with(Some(plugins.clone()));
        let ctx = tools_ctx(&plugins, &f.shared, "com.test.writer").unwrap();
        let (photos, drafts) = (ctx.photos.unwrap(), ctx.drafts.unwrap());
        let patch = r#"{"develop":{"basic":{"exposure":0.5}}}"#;
        assert_eq!(photos.scope(), vec![f.b, f.a]);

        // Turned off while its hosts are held (an instance outlives a Settings toggle).
        plugins.write().set_enabled("com.test.writer", false).unwrap();
        let refused = plugin_refused().message;
        assert_eq!(photos.scope(), Vec::<i64>::new());
        assert_eq!(photos.metadata(f.a).err(), Some(refused.clone()));
        assert_eq!(photos.preview(f.a, 256).err(), Some(refused.clone()));
        assert_eq!(photos.recipe(f.a).err(), Some(refused.clone()));
        assert_eq!(drafts.patch(f.b, patch, "x").err(), Some(refused.clone()));
        assert_eq!((f.draft(f.b), f.history(f.b).len(), f.host.changed.load(Ordering::SeqCst)), (None, 0, 0), "nothing written");
        plugins.write().set_enabled("com.test.writer", true).unwrap();
        drafts.patch(f.b, patch, "Back").unwrap();
        assert_eq!(f.history(f.b), ["AI: Echo: Back"]);

        // A host built for a plugin without the grant is refused too, whatever its tools instance had.
        let weak = Arc::downgrade(&plugins);
        let forged = McpDrafts::new(f.shared.clone(), weak.clone(), "com.test.reader", "Reader");
        assert_eq!(forged.patch(f.a, patch, "x").err(), Some(refused.clone()), "photos.read only");
        let nobody = McpPhotos::new(f.shared.clone(), weak, "com.test.gone");
        assert_eq!((nobody.scope(), nobody.metadata(f.a).err()), (Vec::new(), Some(refused)));
        assert_eq!(f.history(f.a).len(), 0);
    }

    /// One MCP session over HTTP: its id and the open `GET` event stream.
    struct Session {
        id: String,
        events: reqwest::Response,
    }

    const KEY: &str = "secret";

    fn post(client: &reqwest::Client, url: &str, session: Option<&str>, body: &str) -> reqwest::RequestBuilder {
        let r = client.post(url).bearer_auth(KEY).header("content-type", "application/json").header("accept", "application/json, text/event-stream").body(body.to_string());
        match session {
            Some(id) => r.header("mcp-session-id", id),
            None => r,
        }
    }

    /// Initialises a session and opens its server-to-client stream. Also returns what `initialize` said.
    async fn open_session(client: &reqwest::Client, url: &str) -> (Session, String) {
        let init = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"test","version":"1"}}}"#;
        let resp = post(client, url, None, init).send().await.unwrap();
        assert_eq!(resp.status(), 200);
        let id = resp.headers()["mcp-session-id"].to_str().unwrap().to_string();
        let said = resp.text().await.unwrap();
        let ok = post(client, url, Some(&id), r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#).send().await.unwrap();
        assert_eq!(ok.status(), 202);
        let events = client.get(url).bearer_auth(KEY).header("accept", "text/event-stream").header("mcp-session-id", &id).send().await.unwrap();
        assert_eq!(events.status(), 200);
        (Session { id, events }, said)
    }

    /// Reads the session's stream until both list-changed notifications arrived (or 10 s pass).
    async fn both_list_changed(s: &mut Session) -> String {
        let mut seen = String::new();
        let read = async {
            while !(seen.contains("notifications/tools/list_changed") && seen.contains("notifications/prompts/list_changed")) {
                match s.events.chunk().await.unwrap() {
                    Some(bytes) => seen.push_str(&String::from_utf8_lossy(&bytes)),
                    None => break,
                }
            }
        };
        let _ = tokio::time::timeout(std::time::Duration::from_secs(10), read).await;
        seen
    }

    async fn tools_named(client: &reqwest::Client, url: &str, s: &Session) -> String {
        post(client, url, Some(&s.id), r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#).send().await.unwrap().text().await.unwrap()
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn toggle_sends_list_changed() {
        let (_root, _app, plugins, f) = echo_fixture();
        follow_changes(&plugins, f.shared.clone());
        let running = crate::mcp::start(f.dyn_host(), f.shared.clone(), 0, KEY.into()).unwrap();
        let url = format!("http://127.0.0.1:{}/mcp", running.port);
        let client = reqwest::Client::new();
        let (mut session, said) = open_session(&client, &url).await;
        assert!(said.contains(r#""tools":{"listChanged":true}"#) && said.contains(r#""prompts":{"listChanged":true}"#), "advertised: {said}");
        assert!(tools_named(&client, &url, &session).await.contains("echo__echo"));

        // What the Settings switch (or Remove) does: the store changes, then the listeners run.
        crate::commands::plugins::logic::set_enabled(&plugins, "com.test.echo", false).unwrap();
        plugins.run_listeners();
        let seen = both_list_changed(&mut session).await;
        assert!(seen.contains("notifications/tools/list_changed") && seen.contains("notifications/prompts/list_changed"), "{seen}");
        assert!(!tools_named(&client, &url, &session).await.contains("echo__echo"), "the cache was cleared before the client asked again");

        // Enabling again and removing it tell the session too.
        crate::commands::plugins::logic::set_enabled(&plugins, "com.test.echo", true).unwrap();
        plugins.run_listeners();
        assert!(both_list_changed(&mut session).await.contains("prompts/list_changed"));
        assert!(tools_named(&client, &url, &session).await.contains("echo__echo"));
        crate::commands::plugins::logic::remove(&plugins, "com.test.echo").unwrap();
        plugins.run_listeners();
        assert!(both_list_changed(&mut session).await.contains("prompts/list_changed"));
        assert!(!tools_named(&client, &url, &session).await.contains("echo__echo"));
        running.cancel.cancel();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_closed_session_does_not_stop_the_others() {
        let (_root, _app, plugins, f) = echo_fixture();
        follow_changes(&plugins, f.shared.clone());
        let running = crate::mcp::start(f.dyn_host(), f.shared.clone(), 0, KEY.into()).unwrap();
        let url = format!("http://127.0.0.1:{}/mcp", running.port);
        let client = reqwest::Client::new();
        let (gone, _) = open_session(&client, &url).await;
        let (mut stays, _) = open_session(&client, &url).await;
        // One client closes its connection and ends its session; the other stays.
        let ended = client.delete(&url).bearer_auth(KEY).header("mcp-session-id", &gone.id).send().await.unwrap();
        drop(gone);
        assert!(ended.status().is_success(), "{}", ended.status());
        crate::commands::plugins::logic::set_enabled(&plugins, "com.test.echo", false).unwrap();
        plugins.run_listeners();
        let seen = both_list_changed(&mut stays).await;
        assert!(seen.contains("notifications/tools/list_changed") && seen.contains("notifications/prompts/list_changed"), "{seen}");
        // The session that ended is pruned; the one that stays is still counted.
        assert_eq!(f.shared.peers.len(), 1, "the ended session was dropped from the list");
        running.cancel.cancel();
    }

    #[test]
    fn tool_count_is_zero_while_assistants_are_off() {
        let (_root, _app, _plugins, f) = echo_fixture();
        assert_eq!(plugin_tool_count(&f.dyn_host(), &f.shared), 2, "echo and record");
        f.host.on.store(false, Ordering::SeqCst);
        assert_eq!(plugin_tool_count(&f.dyn_host(), &f.shared), 0);
        f.host.on.store(true, Ordering::SeqCst);
        let (_app2, bare) = plugins_with(&[]);
        let g = fixture_with(Some(bare));
        assert_eq!(plugin_tool_count(&g.dyn_host(), &g.shared), 0, "no plugins");
    }
}
