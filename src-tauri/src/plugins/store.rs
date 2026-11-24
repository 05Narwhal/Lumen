//! The plugin store's backend (Round 24 lane C, spec §7): the index from the public
//! `lumen-plugins` repo, verified downloads and background updates. Installing still goes through
//! the existing install function, so signature, key pinning and permissions are unchanged.
//!
//! Requests go through Rust (`reqwest`), never the webview. The index is cached on disk so the page
//! shows the last list offline.

use std::collections::HashSet;
use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use photos_plugins::theme_pkg::ThemeSwatches;
use photos_plugins::{PluginError, PluginInfo, PluginTrust};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter};
use ts_rs::TS;

use super::Plugins;

/// `<app>/plugins/store-cache/`: the images and READMEs the index lists, named by hash.
pub fn store_cache_dir(plugins_dir: &Path) -> PathBuf {
    plugins_dir.join("store-cache")
}

pub const INDEX_URL: &str = "https://raw.githubusercontent.com/05Narwhal/lumen-plugins/main/index.json";

/// The one index version this Lumen reads.
const INDEX_VERSION: u32 = 1;

/// A package lists at most this many screenshots; an index with more is cut to this.
pub const MAX_SCREENSHOTS: usize = 6;
/// The largest image or README Lumen keeps from the index (the package checks are stricter).
const MAX_IMAGE: usize = 2 * 1024 * 1024;
const MAX_README: usize = 64 * 1024;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct StoreIndex {
    pub version: u32,
    /// The id of the plugin Discover leads with; dropped when the index doesn't list it.
    #[serde(default)]
    pub featured: Option<String>,
    pub plugins: Vec<StoreEntry>,
    /// `.lumentheme` packages (Round 26); absent in a Round 24 index.
    #[serde(default)]
    pub themes: Vec<StoreTheme>,
    /// The theme the Themes tab leads with; dropped when the index doesn't list it.
    #[serde(default)]
    pub featured_theme: Option<String>,
}

/// The tags a theme listing may carry.
pub const THEME_TAGS: [&str; 4] = ["dark", "light", "layout", "colourful"];

/// A theme in the index: a plugin entry's fields minus `permissions` / `kinds`, plus `tags` and
/// the swatches the store's miniature is painted from (`add-theme.mjs` writes them from theme.toml).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct StoreTheme {
    pub id: String,
    pub name: String,
    pub author: String,
    #[serde(default)]
    pub jl: bool,
    pub summary: String,
    #[serde(default)]
    pub icon: Option<String>,
    #[serde(default)]
    pub homepage: Option<String>,
    #[serde(default)]
    pub readme: Option<String>,
    #[serde(default)]
    pub screenshots: Vec<String>,
    #[serde(default)]
    pub author_url: Option<String>,
    #[serde(default)]
    pub repository: Option<String>,
    /// `dark`, `light`, `layout`, `colourful`.
    #[serde(default)]
    pub tags: Vec<String>,
    pub tokens: ThemeSwatches,
    pub latest: StoreRelease,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct StoreEntry {
    pub id: String,
    pub name: String,
    pub author: String,
    /// The index's claim of a JL Studios plugin. What [`store_index`](super::StoreService::index)
    /// returns is already checked against what is installed; an install refuses the claim
    /// unless the package is signed by the JL key.
    #[serde(default)]
    pub jl: bool,
    pub summary: String,
    #[serde(default)]
    pub kinds: Vec<String>,
    /// Permission names as the manifest writes them (`library.read`); the page words them.
    #[serde(default)]
    pub permissions: Vec<String>,
    /// Written as an `https` URL in the index; what the page gets is a `photos://…/store-image/<hash>`
    /// of a copy Lumen downloaded, or nothing ([`localise_images`]).
    #[serde(default)]
    pub icon: Option<String>,
    #[serde(default)]
    pub homepage: Option<String>,
    /// A Markdown file's URL. The page reads its text through `store_readme`, never the URL.
    #[serde(default)]
    pub readme: Option<String>,
    /// At most [`MAX_SCREENSHOTS`]; same treatment as `icon`.
    #[serde(default)]
    pub screenshots: Vec<String>,
    #[serde(default)]
    pub author_url: Option<String>,
    #[serde(default)]
    pub repository: Option<String>,
    pub latest: StoreRelease,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct StoreRelease {
    pub version: String,
    pub api: String,
    pub url: String,
    pub blake3: String,
    #[ts(type = "number")]
    pub size: u64,
    pub released: String,
}

/// A newer version in the index than the installed one.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct StoreUpdate {
    pub id: String,
    pub name: String,
    pub from: String,
    pub to: String,
    /// The update asks for permissions the installed version doesn't have: it is never installed
    /// in the background; *Update* opens the install sheet.
    pub needs_review: bool,
}

/// A background update that was installed (the event `app://plugins-updated` carries a list).
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct StoreUpdated {
    pub id: String,
    pub name: String,
    pub version: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct StoreStatus {
    /// The last attempt to reach the index failed.
    pub offline: bool,
    /// A cached index exists on disk.
    pub has_cache: bool,
    pub checked_at: Option<String>,
    pub auto_update: bool,
    pub updates: Vec<StoreUpdate>,
    /// Installed by the background task this session.
    pub updated: Vec<StoreUpdated>,
}

/// What `store_install` / `store_update` did.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct StoreDownload {
    /// The verified package, for the install sheet. `None` once it has been installed.
    pub path: Option<String>,
    pub installed: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct StoreError {
    pub code: &'static str,
    pub message: String,
}

impl StoreError {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self { code, message: message.into() }
    }
    pub fn offline() -> Self {
        Self::new("store_offline", "You're offline. The plugin store needs the internet.")
    }
    fn index(message: impl Into<String>) -> Self {
        Self::new("store_index", message)
    }
    fn download(message: impl Into<String>) -> Self {
        Self::new("store_download", message)
    }
}

impl From<PluginError> for StoreError {
    fn from(e: PluginError) -> Self {
        Self { code: e.code, message: e.message }
    }
}

impl std::fmt::Display for StoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl From<StoreError> for crate::error::AppError {
    fn from(e: StoreError) -> Self {
        Self { code: e.code.into(), message: e.message, detail: None }
    }
}

/// Reads an index. Only version 1 is understood; every package URL must be `https` and every
/// hash a blake3 hex digest.
pub fn parse_index(text: &str) -> Result<StoreIndex, StoreError> {
    let value: serde_json::Value =
        serde_json::from_str(text).map_err(|e| StoreError::index(format!("The plugin list isn't readable: {e}")))?;
    let version = value.get("version").and_then(|v| v.as_u64()).unwrap_or(0);
    if version != u64::from(INDEX_VERSION) {
        return Err(StoreError::index("The plugin list is from a newer Lumen. Update Lumen to use the store."));
    }
    let mut index: StoreIndex =
        serde_json::from_value(value).map_err(|e| StoreError::index(format!("The plugin list isn't readable: {e}")))?;
    for p in &mut index.plugins {
        p.screenshots.truncate(MAX_SCREENSHOTS);
        let links = [p.icon.as_ref(), p.homepage.as_ref(), p.readme.as_ref(), p.author_url.as_ref(), p.repository.as_ref()];
        check_listing(&p.name, &p.latest, links, &p.screenshots)?;
    }
    for t in &mut index.themes {
        t.screenshots.truncate(MAX_SCREENSHOTS);
        let links = [t.icon.as_ref(), t.homepage.as_ref(), t.readme.as_ref(), t.author_url.as_ref(), t.repository.as_ref()];
        check_listing(&t.name, &t.latest, links, &t.screenshots)?;
        if let Some(bad) = t.tags.iter().find(|g| !THEME_TAGS.contains(&g.as_str())) {
            return Err(StoreError::index(format!("{}'s tag \"{bad}\" isn't known.", t.name)));
        }
    }
    if index.featured_theme.as_ref().is_some_and(|f| !index.themes.iter().any(|t| &t.id == f)) {
        index.featured_theme = None;
    }
    if index.featured.as_ref().is_some_and(|f| !index.plugins.iter().any(|p| &p.id == f)) {
        index.featured = None;
    }
    Ok(index)
}

/// Every link of a listing is `https` and its package hash a blake3 digest.
fn check_listing(name: &str, latest: &StoreRelease, links: [Option<&String>; 5], shots: &[String]) -> Result<(), StoreError> {
    const WHAT: [&str; 5] = ["icon", "homepage", "README", "author link", "repository link"];
    let all = std::iter::once(("package", Some(&latest.url)))
        .chain(WHAT.into_iter().zip(links))
        .chain(shots.iter().map(|u| ("screenshot", Some(u))));
    for (what, url) in all {
        if url.is_some_and(|u| !u.starts_with("https://")) {
            return Err(StoreError::index(format!("{name}'s {what} isn't served over https.")));
        }
    }
    if !is_hash(&latest.blake3) {
        return Err(StoreError::index(format!("{name}'s package hash isn't a blake3 digest.")));
    }
    Ok(())
}

/// Whether `latest` is a newer release than `installed` (semver, a pre-release is older than
/// its release). Anything that isn't semver is never newer.
pub fn newer_version(installed: &str, latest: &str) -> bool {
    match (semver::Version::parse(installed), semver::Version::parse(latest)) {
        (Ok(a), Ok(b)) => b > a,
        _ => false,
    }
}

/// Whether a card shows *By JL Studios*. An installed plugin counts only by its verified trust;
/// one that isn't installed shows the index's claim, which an install refuses unless the package
/// is signed by the JL key ([`check_claim`]).
pub fn badge(claimed: bool, installed: Option<&PluginTrust>) -> bool {
    match installed {
        Some(trust) => claimed && *trust == PluginTrust::JlStudios,
        None => claimed,
    }
}

/// Refuses a package whose index entry claims JL Studios but isn't signed by the JL key.
pub fn check_claim(claimed: bool, trust: &PluginTrust) -> Result<(), StoreError> {
    if claimed && *trust != PluginTrust::JlStudios {
        return Err(PluginError::signature("The plugin list says this is from JL Studios, but the package isn't signed with JL Studios' key.").into());
    }
    Ok(())
}

/// Downloads `entry`'s package into `dir` as `<id>-<version>.lumenplugin`: to a `.part` file first,
/// checked for size and blake3, then moved into place. Any failure leaves nothing behind.
pub async fn download(client: &reqwest::Client, entry: &StoreEntry, dir: &Path) -> Result<PathBuf, StoreError> {
    download_package(client, &Source::Web, &entry.package("lumenplugin"), dir).await
}

/// What a download needs of an index entry, plugin or theme.
pub struct Package<'a> {
    pub id: &'a str,
    pub name: &'a str,
    pub latest: &'a StoreRelease,
    pub ext: &'static str,
}

impl StoreEntry {
    pub fn package(&self, ext: &'static str) -> Package<'_> {
        Package { id: &self.id, name: &self.name, latest: &self.latest, ext }
    }
}

impl StoreTheme {
    pub fn package(&self) -> Package<'_> {
        Package { id: &self.id, name: &self.name, latest: &self.latest, ext: "lumentheme" }
    }
}

/// Where the store's files come from: the web, or a local store folder (Round 26 dev store).
#[derive(Debug, Clone, PartialEq)]
pub enum Source {
    Web,
    Local(PathBuf),
}

/// The file a URL of the index names in a local store folder: `dist/<file name>`, else `<file name>`.
pub fn local_file(dir: &Path, url: &str) -> Option<PathBuf> {
    let name = url.rsplit('/').next().filter(|n| !n.is_empty() && *n != ".." && !n.contains('\\'))?;
    [dir.join("dist").join(name), dir.join(name)].into_iter().find(|p| p.is_file())
}

/// Downloads (or, from a local store, copies) `pkg` into `dir` as `<id>-<version>.<ext>`: to a
/// `.part` file first, checked for size and blake3, then moved into place. Any failure leaves
/// nothing behind.
pub async fn download_package(client: &reqwest::Client, source: &Source, pkg: &Package<'_>, dir: &Path) -> Result<PathBuf, StoreError> {
    fs::create_dir_all(dir).map_err(|e| StoreError::download(format!("The download folder couldn't be made: {e}")))?;
    let name = format!("{}-{}.{}", pkg.id, pkg.latest.version, pkg.ext);
    // An id or version with a separator in it would leave `dir`.
    if name.contains(['/', '\\']) || name.contains("..") {
        return Err(StoreError::download("The plugin's name isn't valid."));
    }
    let part = dir.join(format!("{name}.part"));
    let dest = dir.join(&name);
    let r = match source {
        Source::Web => download_to(client, pkg, &part).await,
        Source::Local(root) => copy_to(root, pkg, &part),
    };
    match r {
        Ok(()) => fs::rename(&part, &dest).map(|_| dest).map_err(|e| {
            let _ = fs::remove_file(&part);
            StoreError::download(format!("The package couldn't be put in place: {e}"))
        }),
        Err(e) => {
            let _ = fs::remove_file(&part);
            Err(e)
        }
    }
}

/// A local store's package, held to the same size and blake3 as a download.
fn copy_to(root: &Path, pkg: &Package<'_>, part: &Path) -> Result<(), StoreError> {
    let cut = |what: &str| StoreError::download(format!("{} from the local store: {what}", pkg.name));
    let src = local_file(root, &pkg.latest.url).ok_or_else(|| cut("the package isn't in the store folder (run pack.mjs)"))?;
    let bytes = fs::read(&src).map_err(|e| cut(&e.to_string()))?;
    if bytes.len() as u64 != pkg.latest.size || !blake3::hash(&bytes).to_hex().eq_ignore_ascii_case(&pkg.latest.blake3) {
        return Err(cut("the file doesn't match the list's checksum (re-run add-plugin.mjs / add-theme.mjs)"));
    }
    fs::write(part, &bytes).map_err(|e| cut(&e.to_string()))
}

async fn download_to(client: &reqwest::Client, entry: &Package<'_>, part: &Path) -> Result<(), StoreError> {
    let cut = |what: &str| StoreError::download(format!("The download of {} failed: {what}", entry.name));
    let mut resp = client.get(&entry.latest.url).send().await.map_err(|_| StoreError::offline())?;
    if !resp.status().is_success() {
        return Err(cut(&format!("the server answered {}", resp.status())));
    }
    let mut file = fs::File::create(part).map_err(|e| cut(&e.to_string()))?;
    let mut hasher = blake3::Hasher::new();
    let mut got: u64 = 0;
    while let Some(chunk) = resp.chunk().await.map_err(|_| cut("the connection ended early"))? {
        got += chunk.len() as u64;
        if got > entry.latest.size {
            return Err(cut("the file is larger than the list says"));
        }
        hasher.update(&chunk);
        file.write_all(&chunk).map_err(|e| cut(&e.to_string()))?;
    }
    file.flush().map_err(|e| cut(&e.to_string()))?;
    drop(file);
    if got != entry.latest.size {
        return Err(cut("the file is shorter than the list says"));
    }
    if !hasher.finalize().to_hex().eq_ignore_ascii_case(&entry.latest.blake3) {
        return Err(cut("the file doesn't match the list's checksum"));
    }
    Ok(())
}

/// Fetches the index and caches it at `cache`. A network failure is [`StoreError::offline`]; a
/// reply that isn't a valid index is an error and leaves the cache as it was.
pub async fn fetch_index(client: &reqwest::Client, url: &str, cache: &Path) -> Result<StoreIndex, StoreError> {
    let resp = client.get(url).send().await.map_err(|_| StoreError::offline())?;
    if !resp.status().is_success() {
        return Err(StoreError::offline());
    }
    let text = resp.text().await.map_err(|_| StoreError::offline())?;
    let index = parse_index(&text)?;
    if let Some(parent) = cache.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let tmp = cache.with_extension("json.tmp");
    if fs::write(&tmp, &text).is_ok() && fs::rename(&tmp, cache).is_err() {
        let _ = fs::remove_file(&tmp);
    }
    Ok(index)
}

// ---------------------------------------------------------------------------------------------
// Store images and READMEs (Round 25 P1): downloaded by Rust into `store-cache/`, named by the
// blake3 of the URL they came from, so the page can show them offline and never loads a remote URL.
// ---------------------------------------------------------------------------------------------

/// The cache name of what `url` serves.
pub fn image_hash(url: &str) -> String {
    blake3::hash(url.as_bytes()).to_hex().to_string()
}

/// `png` or `jpg` by the file's first bytes (an index can't make Lumen keep anything else).
fn image_ext(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("png")
    } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some("jpg")
    } else {
        None
    }
}

fn is_hash(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit())
}

/// The cached image for `hash` (a 64-digit hash, nothing else), whichever type it is.
pub fn find_cached(dir: &Path, hash: &str) -> Option<PathBuf> {
    if !is_hash(hash) {
        return None;
    }
    ["png", "jpg"].iter().map(|e| dir.join(format!("{hash}.{e}"))).find(|p| p.is_file())
}

/// `photos://…/store-image/<hash>` in the form this OS's webview can load.
pub fn store_image_url_for(windows: bool, hash: &str) -> String {
    let base = if windows { "http://photos.localhost" } else { "photos://localhost" };
    format!("{base}/store-image/{hash}")
}

/// Reads at most `max` bytes of `url`'s body.
async fn fetch_capped(client: &reqwest::Client, url: &str, max: usize) -> Result<Vec<u8>, StoreError> {
    let mut resp = client.get(url).send().await.map_err(|_| StoreError::offline())?;
    if !resp.status().is_success() {
        return Err(StoreError::download(format!("the server answered {}", resp.status())));
    }
    let mut body = Vec::new();
    while let Some(chunk) = resp.chunk().await.map_err(|_| StoreError::download("the connection ended early"))? {
        if body.len() + chunk.len() > max {
            return Err(StoreError::download("the file is larger than a store listing may use"));
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

/// Writes `bytes` to `dest` through a `.part` file; a failure leaves nothing.
fn write_whole(dest: &Path, bytes: &[u8]) -> Result<(), StoreError> {
    let io = |e: std::io::Error| StoreError::download(format!("The store cache couldn't be written: {e}"));
    let part = dest.with_extension(format!("{}.part", dest.extension().and_then(|e| e.to_str()).unwrap_or("")));
    fs::write(&part, bytes).map_err(io)?;
    fs::rename(&part, dest).map_err(|e| {
        let _ = fs::remove_file(&part);
        io(e)
    })
}

/// Keeps `url`'s image in `dir` (a PNG or JPEG of at most 2 MB). Already there: no request.
pub async fn cache_image(client: &reqwest::Client, url: &str, dir: &Path) -> Result<PathBuf, StoreError> {
    let hash = image_hash(url);
    if let Some(p) = find_cached(dir, &hash) {
        return Ok(p);
    }
    let bytes = fetch_capped(client, url, MAX_IMAGE).await?;
    let ext = image_ext(&bytes).ok_or_else(|| StoreError::download("the file isn't a PNG or JPEG image"))?;
    fs::create_dir_all(dir).map_err(|e| StoreError::download(format!("The store cache couldn't be made: {e}")))?;
    let dest = dir.join(format!("{hash}.{ext}"));
    write_whole(&dest, &bytes)?;
    Ok(dest)
}

fn image_urls(index: &StoreIndex) -> impl Iterator<Item = &String> {
    let plugins = index.plugins.iter().flat_map(|p| p.icon.iter().chain(p.screenshots.iter()));
    plugins.chain(index.themes.iter().flat_map(|t| t.icon.iter().chain(t.screenshots.iter())))
}

/// Downloads every icon and screenshot the index lists that isn't cached; one that fails is
/// logged and left out (the card shows a letter instead), never an error for the page.
pub async fn cache_images(client: &reqwest::Client, index: &StoreIndex, dir: &Path) {
    let missing: Vec<&String> = image_urls(index).filter(|u| find_cached(dir, &image_hash(u)).is_none()).collect();
    let results = futures_util::future::join_all(missing.iter().map(|u| cache_image(client, u, dir))).await;
    for (url, r) in missing.iter().zip(results) {
        if let Err(e) = r {
            tracing::warn!(url = %url, error = %e, "a store image couldn't be kept");
        }
    }
}

/// The index as the page gets it: each icon and screenshot is the `store-image` URL of a copy on
/// disk, or gone when there is none. No remote URL reaches the page.
pub fn localise_images(mut index: StoreIndex, dir: &Path, windows: bool) -> StoreIndex {
    let local = |url: &String| find_cached(dir, &image_hash(url)).map(|_| store_image_url_for(windows, &image_hash(url)));
    for p in &mut index.plugins {
        p.icon = p.icon.as_ref().and_then(local);
        p.screenshots = p.screenshots.iter().filter_map(local).collect();
    }
    for t in &mut index.themes {
        t.icon = t.icon.as_ref().and_then(local);
        t.screenshots = t.screenshots.iter().filter_map(local).collect();
    }
    index
}

/// Deletes what the index no longer lists (and half-written files), so the cache stays the size of
/// the current index.
pub fn sweep_images(dir: &Path, index: &StoreIndex) {
    let keep: HashSet<String> =
        image_urls(index)
        .chain(index.plugins.iter().flat_map(|p| p.readme.iter()))
        .chain(index.themes.iter().flat_map(|t| t.readme.iter()))
        .map(|u| image_hash(u)).collect();
    let Ok(rd) = fs::read_dir(dir) else { return };
    for f in rd.flatten() {
        let name = f.file_name().to_string_lossy().into_owned();
        let stem = name.strip_suffix(".png").or_else(|| name.strip_suffix(".jpg")).or_else(|| name.strip_suffix(".md"));
        if !stem.is_some_and(|s| keep.contains(s)) {
            let _ = fs::remove_file(f.path());
        }
    }
}

/// A README's text: from the network (and cached), else the cached copy, else offline. Over 64 KB
/// or not UTF-8 is refused.
pub async fn fetch_readme(client: &reqwest::Client, url: &str, dir: &Path) -> Result<String, StoreError> {
    let file = dir.join(format!("{}.md", image_hash(url)));
    match fetch_capped(client, url, MAX_README).await {
        Ok(bytes) => {
            let text = String::from_utf8(bytes).map_err(|_| StoreError::download("the README isn't text"))?;
            let _ = fs::create_dir_all(dir);
            let _ = write_whole(&file, text.as_bytes());
            Ok(text)
        }
        Err(e) if e.code == "store_offline" => fs::read_to_string(&file).map_err(|_| e),
        Err(e) => Err(e),
    }
}

/// The cached index, if there is one and it still parses.
pub fn load_cache(cache: &Path) -> Option<StoreIndex> {
    parse_index(&fs::read_to_string(cache).ok()?).ok()
}

/// The installed plugins (not Developer-mode folders) that the index has a newer version of.
pub fn compute_updates(index: &StoreIndex, installed: &[PluginInfo]) -> Vec<StoreUpdate> {
    index
        .plugins
        .iter()
        .filter_map(|e| {
            let have = installed.iter().find(|i| i.id == e.id && !i.dev)?;
            newer_version(&have.version, &e.latest.version).then(|| StoreUpdate {
                id: e.id.clone(),
                name: e.name.clone(),
                from: have.version.clone(),
                to: e.latest.version.clone(),
                needs_review: false,
            })
        })
        .collect()
}

// ---------------------------------------------------------------------------------------------
// The service: the index in memory, the status, the background task.
// ---------------------------------------------------------------------------------------------

/// The event the background task sends after it installed updates; the payload is a
/// `Vec<StoreUpdated>`.
pub const UPDATED_EVENT: &str = "app://plugins-updated";

const FIRST_CHECK: Duration = Duration::from_secs(30);
const CHECK_EVERY: Duration = Duration::from_secs(6 * 60 * 60);
/// A downloaded package nobody opened the sheet on is swept after this long.
const KEEP_DOWNLOADS: Duration = Duration::from_secs(24 * 60 * 60);

#[derive(Default)]
struct Inner {
    index: Option<StoreIndex>,
    offline: bool,
    checked_at: Option<String>,
    updated: Vec<StoreUpdated>,
    /// Ids whose update asks for new permissions.
    review: HashSet<String>,
}

pub struct StoreService {
    client: reqwest::Client,
    /// `LUMEN_STORE_DIR` (Round 26 dev store): honoured in a debug build, or with Developer mode on.
    env_store: Option<PathBuf>,
    /// `<app>/plugins/`: `store-index.json`, `store-settings.json`, `store-downloads/`.
    dir: PathBuf,
    inner: Mutex<Inner>,
}

impl StoreService {
    pub fn new(app_dir: &Path) -> Self {
        let client = reqwest::Client::builder()
            .user_agent(format!("Lumen/{} (JL Studios photo library)", env!("CARGO_PKG_VERSION")))
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(300))
            .build()
            .expect("http client");
        let env_store = std::env::var_os("LUMEN_STORE_DIR").filter(|v| !v.is_empty()).map(PathBuf::from);
        Self { client, env_store, dir: app_dir.join("plugins"), inner: Mutex::new(Inner::default()) }
    }

    /// Where the index, packages and images come from: a local store folder (the env var in a
    /// debug build or with Developer mode on; Developer mode's *Use a local store folder…*), else
    /// the web. A release build without Developer mode always reads the web.
    pub fn source(&self, developer_mode: bool) -> Source {
        let chosen = read_settings(&self.dir).local_store;
        pick_source(self.env_store.as_deref(), chosen.as_deref(), cfg!(debug_assertions), developer_mode)
    }

    fn source_of(&self, plugins: &Plugins) -> Source {
        let dev = plugins.read().developer_mode();
        self.source(dev)
    }

    /// The local store folder Developer mode chose (`None`: the web), as saved.
    pub fn local_store(&self) -> Option<PathBuf> {
        read_settings(&self.dir).local_store
    }

    /// *Use a local store folder…* / *Use the online store*. The folder must hold an `index.json`.
    pub fn set_local_store(&self, dir: Option<PathBuf>) -> Result<(), StoreError> {
        if dir.as_ref().is_some_and(|d| !d.join("index.json").is_file()) {
            return Err(StoreError::index("That folder has no index.json."));
        }
        let mut st = read_settings(&self.dir);
        st.local_store = dir;
        write_settings(&self.dir, &st)?;
        self.lock().index = None;
        Ok(())
    }

    fn cache(&self) -> PathBuf {
        self.dir.join("store-index.json")
    }

    fn downloads(&self) -> PathBuf {
        self.dir.join("store-downloads")
    }

    fn images(&self) -> PathBuf {
        store_cache_dir(&self.dir)
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// *Update plugins automatically*: on unless the person turned it off. Kept in a small file of
    /// its own beside `plugins.json`, so a damaged one never touches the installed list.
    pub fn auto_update(&self) -> bool {
        read_settings(&self.dir).auto_update
    }

    pub fn set_auto_update(&self, on: bool) -> Result<(), StoreError> {
        let mut st = read_settings(&self.dir);
        st.auto_update = on;
        write_settings(&self.dir, &st)
    }

    /// The index for the page: from the network (and cached), else the cache with the status
    /// marked offline, else the offline error. `jl` is settled against what is installed.
    pub async fn index(&self, plugins: &Plugins) -> Result<StoreIndex, StoreError> {
        let source = self.source_of(plugins);
        let fetched = match &source {
            Source::Web => fetch_index(&self.client, INDEX_URL, &self.cache()).await,
            Source::Local(dir) => read_local_index(dir, &self.images()),
        };
        let index = match fetched {
            Ok(i) if source != Source::Web => {
                sweep_images(&self.images(), &i);
                i
            }
            Ok(i) => {
                {
                    let mut st = self.lock();
                    st.offline = false;
                    st.checked_at = Some(chrono::Utc::now().to_rfc3339());
                }
                // Online: keep this index's images, drop the ones it no longer lists.
                cache_images(&self.client, &i, &self.images()).await;
                sweep_images(&self.images(), &i);
                i
            }
            Err(e) if e.code == "store_offline" => {
                let cached = load_cache(&self.cache());
                self.lock().offline = true;
                cached.ok_or(e)?
            }
            Err(e) => return Err(e),
        };
        self.lock().index = Some(index.clone());
        let settled = settle_badges(index, &plugins.read().list());
        Ok(localise_images(settled, &self.images(), cfg!(windows)))
    }

    /// A plugin's README text for its page: `None` when the index lists none.
    pub async fn readme(&self, plugins: &Plugins, id: &str) -> Result<Option<String>, StoreError> {
        let index = self.known_index(plugins).await?;
        let url = index
            .plugins
            .iter()
            .find(|e| e.id == id)
            .map(|e| e.readme.clone())
            .or_else(|| index.themes.iter().find(|t| t.id == id).map(|t| t.readme.clone()))
            .ok_or_else(|| StoreError::index("That isn't in the store any more."))?;
        let Some(url) = url else { return Ok(None) };
        match self.source_of(plugins) {
            Source::Web => fetch_readme(&self.client, &url, &self.images()).await.map(Some),
            Source::Local(dir) => {
                let file = local_file(&dir, &url).ok_or_else(|| StoreError::download("The README isn't in the store folder."))?;
                let bytes = fs::read(file).map_err(|e| StoreError::download(e.to_string()))?;
                if bytes.len() > MAX_README {
                    return Err(StoreError::download("the file is larger than a store listing may use"));
                }
                String::from_utf8(bytes).map(Some).map_err(|_| StoreError::download("the README isn't text"))
            }
        }
    }

    pub fn status(&self, plugins: &Plugins) -> StoreStatus {
        let st = self.lock();
        let mut updates = st
            .index
            .as_ref()
            .map(|i| compute_updates(i, &plugins.read().list()))
            .unwrap_or_default();
        for u in &mut updates {
            u.needs_review = st.review.contains(&u.id);
        }
        StoreStatus {
            offline: st.offline,
            has_cache: self.cache().is_file(),
            checked_at: st.checked_at.clone(),
            auto_update: self.auto_update(),
            updates,
            updated: st.updated.clone(),
        }
    }

    async fn known_index(&self, plugins: &Plugins) -> Result<StoreIndex, StoreError> {
        let known = self.lock().index.clone();
        match known {
            Some(i) => Ok(i),
            None => self.index(plugins).await,
        }
    }

    /// *Try* / *Get* of a theme: its `.lumentheme` downloaded (or copied from the local store) and
    /// checked against the index's size and blake3. Returns the file and its hash.
    pub async fn fetch_theme(&self, plugins: &Plugins, id: &str) -> Result<(PathBuf, String), StoreError> {
        let theme = self
            .known_index(plugins)
            .await?
            .themes
            .into_iter()
            .find(|t| t.id == id)
            .ok_or_else(|| StoreError::index("That theme isn't in the store any more."))?;
        sweep_downloads(&self.downloads(), KEEP_DOWNLOADS);
        let source = self.source_of(plugins);
        let path = download_package(&self.client, &source, &theme.package(), &self.downloads()).await?;
        Ok((path, theme.latest.blake3.to_ascii_lowercase()))
    }

    async fn entry(&self, plugins: &Plugins, id: &str) -> Result<StoreEntry, StoreError> {
        let index = self.known_index(plugins).await?;
        index
            .plugins
            .into_iter()
            .find(|e| e.id == id)
            .ok_or_else(|| StoreError::index("That plugin isn't in the store any more."))
    }

    /// Downloads and checks `entry`'s package; nothing is installed. Returns the file and what
    /// the install sheet would show.
    async fn fetch_package(
        &self,
        plugins: &Arc<Plugins>,
        entry: &StoreEntry,
    ) -> Result<(PathBuf, photos_plugins::PluginInspect), StoreError> {
        sweep_downloads(&self.downloads(), KEEP_DOWNLOADS);
        let source = self.source_of(plugins);
        let path = download_package(&self.client, &source, &entry.package("lumenplugin"), &self.downloads()).await?;
        let (p, file) = (plugins.clone(), path.clone());
        let inspected = tauri::async_runtime::spawn_blocking(move || crate::commands::plugins::logic::inspect(&p, &file))
            .await
            .map_err(|e| StoreError::download(e.to_string()))
            .and_then(|r| r.map_err(StoreError::from))
            .and_then(|i| check_claim(entry.jl, &i.info.trust).map(|_| i));
        match inspected {
            Ok(i) => Ok((path, i)),
            Err(e) => {
                let _ = fs::remove_file(&path);
                Err(e)
            }
        }
    }

    /// *Get*: downloads, verifies and returns the package for the install sheet.
    pub async fn install(&self, plugins: &Arc<Plugins>, id: &str) -> Result<StoreDownload, StoreError> {
        let entry = self.entry(plugins, id).await?;
        let (path, _) = self.fetch_package(plugins, &entry).await?;
        Ok(StoreDownload { path: Some(path.display().to_string()), installed: false })
    }

    /// *Update*: installs at once when the new version asks for nothing new; otherwise hands
    /// back the package for the install sheet.
    pub async fn update(&self, plugins: &Arc<Plugins>, app: &AppHandle, id: &str) -> Result<StoreDownload, StoreError> {
        let entry = self.entry(plugins, id).await?;
        let (path, inspected) = self.fetch_package(plugins, &entry).await?;
        let asks = inspected.update.as_ref().is_none_or(|u| !u.new_permissions.is_empty());
        if asks {
            self.lock().review.insert(id.to_string());
            return Ok(StoreDownload { path: Some(path.display().to_string()), installed: false });
        }
        let (p, file) = (plugins.clone(), path.clone());
        let done = tauri::async_runtime::spawn_blocking(move || crate::commands::plugins::logic::install(&p, &file))
            .await
            .map_err(|e| StoreError::download(e.to_string()))
            .and_then(|r| r.map_err(StoreError::from));
        let _ = fs::remove_file(&path);
        let info = done?;
        self.lock().review.remove(id);
        plugins.changed(app);
        tracing::info!(plugin = %info.id, version = %info.version, "plugin updated from the store");
        Ok(StoreDownload { path: None, installed: true })
    }

    /// One pass of the background task: fetch, compare, and (auto-update on) install what asks
    /// for nothing new. Offline keeps the cache and the status says so.
    pub async fn check(&self, plugins: &Arc<Plugins>, app: &AppHandle) {
        let Ok(index) = self.index(plugins).await else { return };
        if !self.auto_update() {
            return;
        }
        let mut done = Vec::new();
        // The list is bound first: a read guard in the loop's header would live across the awaits.
        let installed = plugins.read().list();
        for u in compute_updates(&index, &installed) {
            match self.update(plugins, app, &u.id).await {
                Ok(d) if d.installed => done.push(StoreUpdated { id: u.id, name: u.name, version: u.to }),
                Ok(d) => {
                    // It needs the person's look: not installed in the background.
                    if let Some(p) = d.path {
                        let _ = fs::remove_file(p);
                    }
                }
                Err(e) => tracing::warn!(plugin = %u.id, error = %e, "a plugin update failed"),
            }
        }
        if !done.is_empty() {
            self.lock().updated.extend(done.iter().cloned());
            let _ = app.emit(UPDATED_EVENT, &done);
        }
    }
}

/// At launch (+30 s), then every 6 h while Lumen runs.
pub fn spawn_background(app: AppHandle, service: Arc<StoreService>, plugins: Arc<Plugins>) {
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(FIRST_CHECK).await;
        loop {
            service.check(&plugins, &app).await;
            tokio::time::sleep(CHECK_EVERY).await;
        }
    });
}

/// Each index entry's `jl` settled: an installed plugin shows what its signature verified.
fn settle_badges(mut index: StoreIndex, installed: &[PluginInfo]) -> StoreIndex {
    for e in &mut index.plugins {
        let trust = installed.iter().find(|i| i.id == e.id && !i.dev).map(|i| &i.trust);
        e.jl = badge(e.jl, trust);
    }
    index
}

fn sweep_downloads(dir: &Path, older_than: Duration) {
    let Ok(rd) = fs::read_dir(dir) else { return };
    for f in rd.flatten() {
        let old = f
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.elapsed().ok())
            .is_some_and(|age| age > older_than);
        if old {
            let _ = fs::remove_file(f.path());
        }
    }
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoreSettings {
    #[serde(default = "yes")]
    auto_update: bool,
    /// Developer mode's local store folder (Round 26).
    #[serde(default)]
    local_store: Option<PathBuf>,
}

fn yes() -> bool {
    true
}

fn read_settings(dir: &Path) -> StoreSettings {
    fs::read_to_string(dir.join("store-settings.json"))
        .ok()
        .and_then(|t| serde_json::from_str::<StoreSettings>(&t).ok())
        .unwrap_or(StoreSettings { auto_update: true, local_store: None })
}

fn write_settings(dir: &Path, st: &StoreSettings) -> Result<(), StoreError> {
    let bad = |e: std::io::Error| StoreError::new("io", format!("The setting couldn't be saved: {e}"));
    fs::create_dir_all(dir).map_err(bad)?;
    let tmp = dir.join("store-settings.json.tmp");
    let text = serde_json::to_string(st).expect("settings serialise");
    fs::write(&tmp, text).map_err(bad)?;
    fs::rename(&tmp, dir.join("store-settings.json")).map_err(bad)
}

/// The dev store rule: `LUMEN_STORE_DIR` in a debug build or with Developer mode on; else the
/// chosen folder with Developer mode on; else the web.
pub fn pick_source(env: Option<&Path>, chosen: Option<&Path>, debug_build: bool, developer_mode: bool) -> Source {
    match (env, chosen) {
        (Some(e), _) if debug_build || developer_mode => Source::Local(e.to_path_buf()),
        (_, Some(c)) if developer_mode => Source::Local(c.to_path_buf()),
        _ => Source::Web,
    }
}

/// A local store folder's `index.json`, with every icon and screenshot it lists that is in the
/// folder copied into the image cache (same checks as a download), so the page sees the usual
/// `store-image` URLs. Nothing is fetched and the web index's cache is left alone.
pub fn read_local_index(dir: &Path, images: &Path) -> Result<StoreIndex, StoreError> {
    let text = fs::read_to_string(dir.join("index.json"))
        .map_err(|e| StoreError::index(format!("The local store's index.json couldn't be read: {e}")))?;
    let index = parse_index(&text)?;
    for url in image_urls(&index) {
        let hash = image_hash(url);
        if find_cached(images, &hash).is_some() {
            continue;
        }
        let Some(bytes) = local_file(dir, url).and_then(|f| fs::read(f).ok()) else { continue };
        let Some(ext) = image_ext(&bytes).filter(|_| bytes.len() <= MAX_IMAGE) else { continue };
        let _ = fs::create_dir_all(images);
        if let Err(e) = write_whole(&images.join(format!("{hash}.{ext}")), &bytes) {
            tracing::warn!(url = %url, error = %e, "a local store image couldn't be kept");
        }
    }
    Ok(index)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;

    const INDEX: &str = r#"{ "version": 1, "plugins": [ {
      "id": "jl.wayfinder", "name": "Wayfinder", "author": "JL Studios", "jl": true,
      "summary": "Find where photos without GPS were taken.", "kinds": ["tools"],
      "permissions": ["library.read", "places.write", "originals.gps"],
      "icon": "https://example.com/icon.png", "homepage": "https://example.com",
      "latest": { "version": "1.0.0", "api": "1.1.0",
                  "url": "https://github.com/05Narwhal/lumen-plugins/releases/download/w/wayfinder-1.0.0.lumenplugin",
                  "blake3": "0000000000000000000000000000000000000000000000000000000000000000",
                  "size": 123, "released": "2026-11-21" }
    } ] }"#;

    #[test]
    fn parses_index_v1() {
        let i = parse_index(INDEX).unwrap();
        assert_eq!(i.version, 1);
        assert_eq!(i.plugins.len(), 1);
        let p = &i.plugins[0];
        assert_eq!(p.id, "jl.wayfinder");
        assert!(p.jl);
        assert_eq!(p.kinds, ["tools"]);
        assert_eq!(p.permissions, ["library.read", "places.write", "originals.gps"]);
        assert_eq!(p.latest.size, 123);
        assert_eq!(p.latest.api, "1.1.0");
    }

    #[test]
    fn rejects_unknown_version() {
        let e = parse_index(&INDEX.replace("\"version\": 1,", "\"version\": 2,")).unwrap_err();
        assert_eq!(e.code, "store_index");
        assert!(parse_index("not json").is_err());
        // A plain-http package URL is never read.
        assert!(parse_index(&INDEX.replace("https://github.com", "http://github.com")).is_err());
    }

    #[test]
    fn newer_version_detected() {
        assert!(newer_version("1.0.0", "1.0.1"));
        assert!(newer_version("1.0.0", "1.1.0"));
        assert!(newer_version("1.0.0-alpha", "1.0.0"));
        assert!(newer_version("1.0.0-alpha", "1.0.0-beta"));
        assert!(!newer_version("1.0.0", "1.0.0-rc1"));
        assert!(!newer_version("1.2.0", "1.2.0"));
        assert!(!newer_version("1.2.0", "1.1.9"));
        assert!(!newer_version("1.0.0", "garbage"));
    }

    #[test]
    fn jl_badge_needs_jl_signature() {
        let author = PluginTrust::Author { name: "A".into(), fingerprint: "AAAA-BBBB".into() };
        // Installed: the verified trust decides, whatever the index says.
        assert!(badge(true, Some(&PluginTrust::JlStudios)));
        assert!(!badge(true, Some(&author)));
        assert!(!badge(true, Some(&PluginTrust::Unsigned)));
        // Not installed: the claim shows, an install checks it.
        assert!(badge(true, None));
        assert!(!badge(false, None));
        // A claim without the JL key's signature is refused at install.
        assert_eq!(check_claim(true, &author).unwrap_err().code, "plugin_signature");
        assert!(check_claim(true, &PluginTrust::Unsigned).is_err());
        assert!(check_claim(true, &PluginTrust::JlStudios).is_ok());
        assert!(check_claim(false, &author).is_ok());
    }

    /// One reply on a local port: `head` then `body`, then the connection closes.
    fn serve(head: String, body: Vec<u8>) -> String {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/pkg.lumenplugin", l.local_addr().unwrap());
        std::thread::spawn(move || {
            if let Ok((mut s, _)) = l.accept() {
                let mut buf = [0u8; 2048];
                let _ = s.read(&mut buf);
                let _ = s.write_all(head.as_bytes());
                let _ = s.write_all(&body);
                let _ = s.flush();
            }
        });
        url
    }

    fn entry_for(url: String, body: &[u8], hash: Option<String>) -> StoreEntry {
        let mut e = parse_index(INDEX).unwrap().plugins.remove(0);
        e.latest.url = url;
        e.latest.size = body.len() as u64;
        e.latest.blake3 = hash.unwrap_or_else(|| blake3::hash(body).to_hex().to_string());
        e
    }

    fn leftovers(dir: &Path) -> Vec<String> {
        fs::read_dir(dir)
            .map(|r| r.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect())
            .unwrap_or_default()
    }

    #[tokio::test]
    async fn downloads_and_verifies() {
        let body = b"PK pretend package".to_vec();
        let url = serve(format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len()), body.clone());
        let dir = tempfile::tempdir().unwrap();
        let e = entry_for(url, &body, None);
        let p = download(&reqwest::Client::new(), &e, dir.path()).await.unwrap();
        assert_eq!(fs::read(&p).unwrap(), body);
        assert_eq!(leftovers(dir.path()), ["jl.wayfinder-1.0.0.lumenplugin"]);
    }

    #[tokio::test]
    async fn partial_download_discarded() {
        let body = b"0123456789".to_vec();
        // Promises 100 bytes, sends 10, closes.
        let url = serve("HTTP/1.1 200 OK\r\nContent-Length: 100\r\nConnection: close\r\n\r\n".into(), body.clone());
        let dir = tempfile::tempdir().unwrap();
        let mut e = entry_for(url, &body, None);
        e.latest.size = 100;
        let r = download(&reqwest::Client::new(), &e, dir.path()).await;
        assert!(r.is_err());
        assert!(leftovers(dir.path()).is_empty(), "{:?}", leftovers(dir.path()));
    }

    #[tokio::test]
    async fn blake3_mismatch_refused() {
        let body = b"a package whose hash is wrong".to_vec();
        let url = serve(format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len()), body.clone());
        let dir = tempfile::tempdir().unwrap();
        let e = entry_for(url, &body, Some("f".repeat(64)));
        let r = download(&reqwest::Client::new(), &e, dir.path()).await;
        assert_eq!(r.unwrap_err().code, "store_download");
        assert!(leftovers(dir.path()).is_empty());
    }

    #[tokio::test]
    async fn index_is_cached_and_offline_is_reported() {
        let dir = tempfile::tempdir().unwrap();
        let cache = dir.path().join("store-index.json");
        let body = INDEX.as_bytes().to_vec();
        let url = serve(format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len()), body);
        let c = reqwest::Client::new();
        let i = fetch_index(&c, &url, &cache).await.unwrap();
        assert_eq!(load_cache(&cache), Some(i));
        // Nothing listens there any more: offline, and the cache stays.
        let e = fetch_index(&c, &url, &cache).await.unwrap_err();
        assert_eq!(e.code, "store_offline");
        assert!(load_cache(&cache).is_some());
        // A reply that isn't an index leaves the cache alone.
        let junk = serve("HTTP/1.1 200 OK\r\nContent-Length: 4\r\nConnection: close\r\n\r\n".into(), b"nope".to_vec());
        assert_eq!(fetch_index(&c, &junk, &cache).await.unwrap_err().code, "store_index");
        assert!(load_cache(&cache).is_some());
    }

    // ---- Round 25 P1 step 3: listing fields and the image cache

    const PNG: &[u8] = b"\x89PNG\r\n\x1a\n pretend image";

    fn listed() -> String {
        INDEX
            .replace(
                "\"homepage\": \"https://example.com\",",
                "\"homepage\": \"https://example.com\", \"readme\": \"https://example.com/README.md\", \
                 \"screenshots\": [\"https://example.com/1.png\", \"https://example.com/2.jpg\"], \
                 \"authorUrl\": \"https://example.com/jl\", \"repository\": \"https://github.com/05Narwhal/wayfinder\",",
            )
            .replace("{ \"version\": 1,", "{ \"version\": 1, \"featured\": \"jl.wayfinder\",")
    }

    #[test]
    fn parses_listing_fields_and_featured() {
        let i = parse_index(&listed()).unwrap();
        assert_eq!(i.featured.as_deref(), Some("jl.wayfinder"));
        let p = &i.plugins[0];
        assert_eq!(p.readme.as_deref(), Some("https://example.com/README.md"));
        assert_eq!(p.screenshots, ["https://example.com/1.png", "https://example.com/2.jpg"]);
        assert_eq!(p.author_url.as_deref(), Some("https://example.com/jl"));
        assert_eq!(p.repository.as_deref(), Some("https://github.com/05Narwhal/wayfinder"));
        // An index without them still reads (the Round 24 form).
        let old = parse_index(INDEX).unwrap();
        assert!(old.featured.is_none() && old.plugins[0].screenshots.is_empty() && old.plugins[0].readme.is_none());
        // A featured id the index doesn't list is dropped, not an error.
        let gone = parse_index(&listed().replace("\"featured\": \"jl.wayfinder\"", "\"featured\": \"jl.nothing\"")).unwrap();
        assert!(gone.featured.is_none());
    }

    #[test]
    fn every_listing_url_must_be_https() {
        for (from, to) in [
            ("https://example.com/icon.png", "http://example.com/icon.png"),
            ("https://example.com\"", "http://example.com\""),
            ("https://example.com/README.md", "http://example.com/README.md"),
            ("https://example.com/2.jpg", "http://example.com/2.jpg"),
            ("https://example.com/jl", "http://example.com/jl"),
            ("https://github.com/05Narwhal/wayfinder", "javascript:alert(1)"),
        ] {
            let e = parse_index(&listed().replace(from, to)).unwrap_err();
            assert_eq!(e.code, "store_index", "{from}");
        }
    }

    #[test]
    fn at_most_six_screenshots_are_kept() {
        let many: Vec<String> = (0..9).map(|n| format!("\"https://example.com/{n}.png\"")).collect();
        let text = listed().replace("\"https://example.com/1.png\", \"https://example.com/2.jpg\"", &many.join(","));
        assert_eq!(parse_index(&text).unwrap().plugins[0].screenshots.len(), 6);
    }

    fn png_server(body: &[u8]) -> String {
        serve(format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len()), body.to_vec())
    }

    #[tokio::test]
    async fn image_is_cached_under_a_hash_and_served_offline() {
        let dir = tempfile::tempdir().unwrap();
        let url = png_server(PNG);
        let p = cache_image(&reqwest::Client::new(), &url, dir.path()).await.unwrap();
        assert_eq!(p.file_name().unwrap().to_string_lossy(), format!("{}.png", image_hash(&url)));
        assert_eq!(fs::read(&p).unwrap(), PNG);
        // The server is gone: the same URL is answered from the cache, no request made.
        assert_eq!(cache_image(&reqwest::Client::new(), &url, dir.path()).await.unwrap(), p);
        assert_eq!(find_cached(dir.path(), &image_hash(&url)), Some(p));
        assert_eq!(leftovers(dir.path()).len(), 1);
    }

    #[tokio::test]
    async fn not_an_image_or_too_big_is_refused_and_leaves_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let c = reqwest::Client::new();
        let html = png_server(b"<html>not a png</html>");
        assert_eq!(cache_image(&c, &html, dir.path()).await.unwrap_err().code, "store_download");
        let mut big = PNG.to_vec();
        big.resize(MAX_IMAGE + 1, 0);
        let big = png_server(&big);
        assert!(cache_image(&c, &big, dir.path()).await.is_err());
        let gone = serve("HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".into(), vec![]);
        assert!(cache_image(&c, &gone, dir.path()).await.is_err());
        assert!(leftovers(dir.path()).is_empty(), "{:?}", leftovers(dir.path()));
    }

    #[test]
    fn find_cached_takes_only_a_hash() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("secret.png"), PNG).unwrap();
        for bad in ["secret", "../secret", "", &"g".repeat(64), &"a".repeat(63)] {
            assert_eq!(find_cached(dir.path(), bad), None, "{bad}");
        }
    }

    #[test]
    fn page_gets_store_image_urls_only_for_what_is_cached() {
        let dir = tempfile::tempdir().unwrap();
        let mut index = parse_index(&listed()).unwrap();
        let icon = "https://example.com/icon.png";
        fs::write(dir.path().join(format!("{}.png", image_hash(icon))), PNG).unwrap();
        fs::write(dir.path().join(format!("{}.png", image_hash("https://example.com/1.png"))), PNG).unwrap();
        let out = localise_images(index.clone(), dir.path(), true);
        let p = &out.plugins[0];
        assert_eq!(p.icon.as_deref(), Some(format!("http://photos.localhost/store-image/{}", image_hash(icon)).as_str()));
        // Only the screenshot that is on disk survives; nothing remote reaches the page.
        assert_eq!(p.screenshots, [format!("http://photos.localhost/store-image/{}", image_hash("https://example.com/1.png"))]);
        index.plugins[0].icon = Some("https://example.com/other.png".into());
        let out = localise_images(index, dir.path(), false);
        assert_eq!(out.plugins[0].icon, None);
        assert!(out.plugins[0].screenshots[0].starts_with("photos://localhost/store-image/"));
    }

    #[test]
    fn sweep_keeps_only_what_the_index_lists() {
        let dir = tempfile::tempdir().unwrap();
        let index = parse_index(&listed()).unwrap();
        let keep = image_hash("https://example.com/icon.png");
        fs::write(dir.path().join(format!("{keep}.png")), PNG).unwrap();
        fs::write(dir.path().join(format!("{}.png", image_hash("https://old.example/x.png"))), PNG).unwrap();
        fs::write(dir.path().join("half.png.part"), b"x").unwrap();
        sweep_images(dir.path(), &index);
        assert_eq!(leftovers(dir.path()), [format!("{keep}.png")]);
    }

    #[tokio::test]
    async fn readme_is_fetched_cached_and_read_offline() {
        let dir = tempfile::tempdir().unwrap();
        let c = reqwest::Client::new();
        let body = "# Wayfinder\n\nFinds places.".as_bytes();
        let url = png_server(body);
        assert_eq!(fetch_readme(&c, &url, dir.path()).await.unwrap(), "# Wayfinder\n\nFinds places.");
        // Offline: the cached text.
        assert_eq!(fetch_readme(&c, &url, dir.path()).await.unwrap(), "# Wayfinder\n\nFinds places.");
        // Never fetched and unreachable: offline.
        assert_eq!(fetch_readme(&c, "http://127.0.0.1:9/none.md", dir.path()).await.unwrap_err().code, "store_offline");
        // Over 64 KB is refused.
        let big = png_server(&vec![b'a'; MAX_README + 1]);
        assert!(fetch_readme(&c, &big, tempfile::tempdir().unwrap().path()).await.is_err());
    }

    // ---- Round 26: themes in the index and the dev store

    fn with_theme(tag: &str, url: &str, hash: &str, size: usize) -> String {
        let tok = r##"{"bg":"#000","surface":"#111","sidebar":"#111","fg":"#fff","fg2":"#ccc","accent":"#88c0d0","placeholder":"#222","radius":"8px","gap":"4px"}"##;
        INDEX.trim_end().trim_end_matches('}').to_string()
            + &format!(
                r#", "featuredTheme": "studio.nord", "themes": [ {{ "id": "studio.nord", "name": "Nord", "author": "A",
                  "summary": "Cool blues.", "tags": ["{tag}"], "tokens": {{ "light": {tok}, "dark": {tok} }},
                  "latest": {{ "version": "1.0.0", "api": "1.4.0", "url": "{url}", "blake3": "{hash}", "size": {size}, "released": "2026-11-24" }} }} ] }}"#
            )
    }

    #[test]
    fn a_round_24_index_without_themes_still_parses() {
        let i = parse_index(INDEX).unwrap();
        assert!(i.themes.is_empty() && i.featured_theme.is_none());
    }

    #[test]
    fn themes_parse_with_tags_tokens_and_featured() {
        let i = parse_index(&with_theme("dark", "https://x/nord.lumentheme", &"a".repeat(64), 10)).unwrap();
        assert_eq!(i.themes[0].tokens.dark.accent, "#88c0d0");
        assert_eq!(i.featured_theme.as_deref(), Some("studio.nord"));
        assert!(parse_index(&with_theme("neon", "https://x/nord.lumentheme", &"a".repeat(64), 10)).is_err());
        assert!(parse_index(&with_theme("dark", "http://x/nord.lumentheme", &"a".repeat(64), 10)).is_err());
    }

    #[test]
    fn dev_store_only_with_a_debug_build_or_developer_mode() {
        let (e, c) = (Path::new("/env"), Path::new("/chosen"));
        assert_eq!(pick_source(Some(e), None, false, false), Source::Web);
        assert_eq!(pick_source(Some(e), Some(c), true, false), Source::Local(e.into()));
        assert_eq!(pick_source(Some(e), Some(c), false, true), Source::Local(e.into()));
        assert_eq!(pick_source(None, Some(c), true, false), Source::Web);
        assert_eq!(pick_source(None, Some(c), false, true), Source::Local(c.into()));
    }

    #[tokio::test]
    async fn dev_store_reads_index_images_and_packages_from_disk() {
        let store = tempfile::tempdir().unwrap();
        let pkg = b"PK theme bytes".to_vec();
        let hash = blake3::hash(&pkg).to_hex().to_string();
        let url = "https://github.com/05Narwhal/lumen-plugins/releases/download/t/nord.lumentheme";
        let mut index: serde_json::Value = serde_json::from_str(&with_theme("dark", url, &hash, pkg.len())).unwrap();
        index["themes"][0]["icon"] = "https://example.com/rel/nord-icon.png".into();
        fs::write(store.path().join("index.json"), index.to_string()).unwrap();
        fs::create_dir_all(store.path().join("dist")).unwrap();
        fs::write(store.path().join("dist/nord.lumentheme"), &pkg).unwrap();
        fs::write(store.path().join("dist/nord-icon.png"), b"\x89PNG\r\n\x1a\n....").unwrap();

        let images = tempfile::tempdir().unwrap();
        let i = read_local_index(store.path(), images.path()).unwrap();
        let local = localise_images(i.clone(), images.path(), false);
        assert!(local.themes[0].icon.as_deref().unwrap().starts_with("photos://localhost/store-image/"));

        let c = reqwest::Client::new();
        let src = Source::Local(store.path().to_path_buf());
        let out = tempfile::tempdir().unwrap();
        let got = download_package(&c, &src, &i.themes[0].package(), out.path()).await.unwrap();
        assert_eq!(fs::read(&got).unwrap(), pkg);
        assert!(got.ends_with("studio.nord-1.0.0.lumentheme"));

        // A file that doesn't match the index's blake3 is refused and leaves nothing.
        fs::write(store.path().join("dist/nord.lumentheme"), b"PK theme bytez").unwrap();
        fs::remove_file(&got).unwrap();
        let e = download_package(&c, &src, &i.themes[0].package(), out.path()).await.unwrap_err();
        assert!(e.message.contains("checksum"), "{e}");
        assert!(leftovers(out.path()).is_empty());
    }
}
