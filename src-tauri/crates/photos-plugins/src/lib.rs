//! Lumen's plugin packages (Round 23): the manifest, the package checks, signatures, licences
//! and the on-disk store under `config_dir/JL-Studios/Lumen/plugins/`. No wasm here yet (A2).
//!
//! Depends on `photos-core` only, never `tauri`.

pub mod host;
pub mod licences;
pub mod manifest;
pub mod package;
pub mod signature;
pub mod store;
pub mod theme;
pub mod types;

pub use types::*;

/// The plugin API this Lumen speaks (the WIT package `lumen:plugin@1.0.0`).
pub const API_VERSION: u32 = 1;
