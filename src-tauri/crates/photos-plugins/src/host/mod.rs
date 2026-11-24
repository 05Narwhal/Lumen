//! The wasm host (spec §2, §10): one wasmtime [`Engine`] with epoch interruption and its own
//! compile cache, a linker built per plugin from its granted permissions, and WASI p2 with no
//! preopens, no env, no args and no sockets. The WIT is `wit/lumen-plugin.wit`.

mod cache;
mod chosen;
mod ctx;
mod engine;
mod importer;
mod imports;
pub mod limits;
mod linker;
pub mod pool;
mod storage;
mod tools;
mod wasi;

use std::sync::Arc;

pub use bindings::lumen::plugin::types::{
    Asset, AssetDetails, AssetFilter, AssetPage, Candidate, Entry, Extras, Gps, GpsSource, Item, MediaKind, MoveTo, Neighbour,
    PhotoMeta, PlaceFlag, PlaceHit, PlaceState, ReviewEvent, SplitAt, Suggestion, Tensor, Tool, VectorSource,
};
pub use chosen::ChosenRoot;
pub use ctx::{DraftsHost, HostCtx, LibraryHost, LogSink, ModelHost, PhotosHost, PlacesHost, ProposalsHost};
pub use engine::Engine;
pub use importer::ImporterInstance;
pub use imports::ImportsCheck;
pub use tools::ToolsInstance;

/// The host side of `lumen:plugin@1.1.0`, generated from the `host-all` world (it names
/// every interface; a plugin's linker gets only the ones its permissions grant). A 1.0.0
/// component's imports and exports resolve against these by semver (`api10_*` tests).
#[allow(clippy::all)]
pub(crate) mod bindings {
    wasmtime::component::bindgen!({ world: "host-all", path: "wit" });
}

/// Called with a plugin id and a reason when the engine stops that plugin.
pub type OnStopped = Arc<dyn Fn(&str, String) + Send + Sync>;
