//! What a plugin instance sees of Lumen (spec §3): its id, granted permissions, settings, data
//! folder, log sink, the chosen root of an import and the photos / drafts hosts of a tools call.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;

use wasmtime::component::ResourceTable;
use wasmtime_wasi::{WasiCtx, WasiCtxView, WasiView};

use super::bindings::lumen::plugin::types::{
    AssetDetails, AssetFilter, AssetPage, Neighbour, PhotoMeta, PlaceHit, Suggestion, Tensor, VectorSource,
};
use super::chosen::ChosenRoot;
use super::limits::Budget;
use super::storage::Storage;
use crate::PluginPermission;

/// Receives one finished log line, already prefixed `plugin=<id> `.
pub type LogSink = Arc<dyn Fn(&str) + Send + Sync>;

/// The library side of `lumen:plugin/photos` (A4 implements it over the Develop filmstrip).
pub trait PhotosHost: Send + Sync {
    fn scope(&self) -> Vec<i64>;
    fn metadata(&self, id: i64) -> Result<PhotoMeta, String>;
    /// A JPEG at most `max` (≤ 1024) pixels on its long side.
    fn preview(&self, id: i64, max: u32) -> Result<Vec<u8>, String>;
    fn recipe(&self, id: i64) -> Result<String, String>;
}

/// The draft side of `lumen:plugin/drafts`: writes `develop_drafts`, never `edits`.
pub trait DraftsHost: Send + Sync {
    fn patch(&self, id: i64, merge_patch_json: &str, label: &str) -> Result<(), String>;
}

/// The host side of `lumen:plugin/models` (API 1.1, `models`; W3 implements it on `photos-ml`).
/// `model` is the plugin-local id of a `[[models]]` entry.
pub trait ModelHost: Send + Sync {
    fn run(&self, plugin: &str, model: &str, inputs: Vec<Tensor>) -> Result<Vec<Tensor>, String>;
    fn run_on_asset(&self, plugin: &str, model: &str, asset: u64, input: &str) -> Result<Vec<Tensor>, String>;
    fn nearest(&self, plugin: &str, source: &VectorSource, query: &[f32], k: u32) -> Result<Vec<Neighbour>, String>;
}

/// The host side of `lumen:plugin/library` (API 1.1, `library.read`; W3). Pages of 200.
pub trait LibraryHost: Send + Sync {
    fn assets(&self, filter: &AssetFilter, cursor: Option<&str>) -> Result<AssetPage, String>;
    /// API 1.4: details for at most 200 ids; an id that isn't in the library is left out.
    fn details(&self, _ids: &[u64]) -> Result<Vec<AssetDetails>, String> {
        Err("this Lumen can't read photo details".to_string())
    }
}

/// The host side of `lumen:plugin/places` (API 1.1, `places.write`; W3): suggestions feed the
/// Place review screen, nothing is written until the person writes them.
pub trait PlacesHost: Send + Sync {
    fn suggest(&self, plugin: &str, batch: Vec<Suggestion>) -> Result<(), String>;
    fn lookup(&self, name: &str) -> Vec<PlaceHit>;
}

/// The host side of `lumen:plugin/proposals` (API 1.3): stores a checked batch for the review
/// screen. Nothing is written to a photo until the person accepts.
pub trait ProposalsHost: Send + Sync {
    fn submit(&self, plugin: &str, batch: Vec<photos_core::proposals::NewProposal>) -> Result<(), String>;
}

/// Everything one instance is given. Built by the binary per import or per tools instance.
pub struct HostCtx {
    pub plugin: String,
    pub granted: Vec<PluginPermission>,
    pub settings: serde_json::Map<String, serde_json::Value>,
    pub data_dir: PathBuf,
    pub log: LogSink,
    /// Set by the importer (A3) when the person cancels; the next `progress` fails.
    pub cancel: Option<Arc<AtomicBool>>,
    pub chosen: Option<ChosenRoot>,
    pub photos: Option<Arc<dyn PhotosHost>>,
    pub drafts: Option<Arc<dyn DraftsHost>>,
    /// API 1.1; each is reached only when its permission is granted (the linker gates it).
    pub models: Option<Arc<dyn ModelHost>>,
    pub library: Option<Arc<dyn LibraryHost>>,
    pub places: Option<Arc<dyn PlacesHost>>,
    /// API 1.3; linked when any proposal permission is granted.
    pub proposals: Option<Arc<dyn ProposalsHost>>,
}

/// A wasmtime store's data: the context plus WASI and the store limits.
pub(crate) struct HostState {
    pub(crate) ctx: HostCtx,
    pub(crate) storage: Storage,
    pub(crate) wasi: WasiCtx,
    pub(crate) table: ResourceTable,
    pub(crate) limits: Budget,
    /// When the running call's clock started: its start, or the last importer `progress`.
    pub(crate) started: Instant,
}

impl HostState {
    pub(crate) fn cancelled(&self) -> bool {
        self.ctx.cancel.as_ref().is_some_and(|c| c.load(Ordering::Relaxed))
    }

    pub(crate) fn log(&self, line: &str) {
        (self.ctx.log)(&format!("plugin={} {line}", self.ctx.plugin));
    }
}

impl WasiView for HostState {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView { ctx: &mut self.wasi, table: &mut self.table }
    }
}
