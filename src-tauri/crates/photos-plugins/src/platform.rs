//! The declarative side of API 1.1 (Round 24 §8.1, §8.2): file-picker / reveal providers and app
//! detection. No wasm runs here; the binary implements these traits (D2 `ProviderHost`, D3
//! `DetectHost`) and the commands `dialog_pick`, `reveal_path`, `plugin_detect`, `plugin_locate`
//! speak these types.

use std::path::Path;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::PickKind;

/// One filter of a file dialog: `Images` with `["jpg", "png"]` (extensions without the dot).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PickFilter {
    pub label: String,
    pub extensions: Vec<String>,
}

/// What a file dialog asks (`dialog_pick`). Unset fields drop their argument pair (§8.1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PickRequest {
    pub kind: PickKind,
    #[serde(default)]
    pub title: Option<String>,
    /// The folder (or, for `save`, the file) the dialog starts at.
    #[serde(default)]
    pub start: Option<String>,
    /// The suggested file name; `save` only.
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub filters: Vec<PickFilter>,
}

/// A provider's answer. `Err` from [`ProviderHost::pick`] means *failed*: the system dialog opens.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PickOutcome {
    Picked { paths: Vec<String>, filter: Option<u32> },
    /// Exit 1: the person cancelled; no fallback.
    Cancelled,
}

/// One `[[detect]]` entry resolved on this machine (§8.2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DetectStatus {
    pub id: String,
    pub name: String,
    /// The executable (or `.app`) found, `None` when not found.
    pub path: Option<String>,
    /// The first line of the declared `version` command's stdout.
    pub version: Option<String>,
    /// The path is one the person chose with *Locate…*.
    pub located: bool,
}

/// File dialogs and *Show in folder* through a plugin's app (D2). `plugin` is the plugin whose
/// provider is active for the kind; the host resolves the exe through [`DetectHost`].
pub trait ProviderHost: Send + Sync {
    /// `None` when no provider is active for `req.kind` (the system dialog opens).
    fn pick(&self, req: &PickRequest) -> Option<Result<PickOutcome, String>>;
    /// `None` when no reveal provider is active; `Some(Err)` falls back to the system reveal.
    fn reveal(&self, path: &Path) -> Option<Result<(), String>>;
}

/// App detection for a plugin's `[[detect]]` entries (D3).
pub trait DetectHost: Send + Sync {
    fn detect(&self, plugin: &str) -> Vec<DetectStatus>;
    /// Stores a person-chosen exe for `detect_id`; it must exist and match the declared `exe`.
    fn locate(&self, plugin: &str, detect_id: &str, exe: &Path) -> Result<DetectStatus, String>;
}
