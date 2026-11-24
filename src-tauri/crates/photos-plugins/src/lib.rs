//! Lumen's plugin packages (Round 23): the manifest, the package checks, signatures, licences
//! and the on-disk store under `config_dir/JL-Studios/Lumen/plugins/`. No wasm here yet (A2).
//!
//! Depends on `photos-core` only, never `tauri`.

pub mod host;
pub mod licences;
pub mod manifest;
pub mod package;
pub mod platform;
pub mod signature;
pub mod store;
pub mod theme;
pub mod theme_css;
pub mod theme_pkg;
pub mod types;

pub use types::*;

pub use manifest::ApiVersion;
pub use platform::{DetectHost, DetectStatus, PickFilter, PickOutcome, PickRequest, ProviderHost};

/// The plugin API this Lumen speaks (API 1.2 adds `[[ui.section]]` settings pages; 1.0 and 1.1
/// plugins keep working). A manifest with a newer `api` waits for a newer Lumen.
pub const API_VERSION: ApiVersion = ApiVersion { major: 1, minor: 4 };
