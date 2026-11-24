//! The binary's API 1.1 hosts (Round 24 §8). Stubs that answer *unsupported* until their lanes
//! replace them: `ModelHost`, `LibraryHost`, `PlacesHost` (W3), `ProviderHost` (D2),
//! `DetectHost` (D3).

use std::path::Path;
use std::sync::Arc;

use tauri::{AppHandle, Manager};

use super::library::{LibraryAssets, Selection};
use super::models::{AppModels, AssetImage, ModelLookup};
use super::places::AppPlaces;
use super::state::Plugins;
use crate::derivatives::RenderSpec;
use crate::state::AppState;

use photos_plugins::host::{
    AssetDetails, AssetFilter, AssetPage, LibraryHost, ModelHost, Neighbour, PlaceHit, PlacesHost, ProposalsHost, Suggestion, Tensor,
    VectorSource,
};
use photos_core::proposals::NewProposal;
use photos_plugins::{DetectHost, DetectStatus, PickOutcome, PickRequest, PluginPermission, ProviderHost};

const UNSUPPORTED: &str = "unsupported: this Lumen doesn't offer it yet";

/// Answers every API 1.1 call with *unsupported* (no provider, nothing detected).
pub struct Unsupported;

impl ModelHost for Unsupported {
    fn run(&self, _: &str, _: &str, _: Vec<Tensor>) -> Result<Vec<Tensor>, String> {
        Err(UNSUPPORTED.into())
    }
    fn run_on_asset(&self, _: &str, _: &str, _: u64, _: &str) -> Result<Vec<Tensor>, String> {
        Err(UNSUPPORTED.into())
    }
    fn nearest(&self, _: &str, _: &VectorSource, _: &[f32], _: u32) -> Result<Vec<Neighbour>, String> {
        Err(UNSUPPORTED.into())
    }
}

impl LibraryHost for Unsupported {
    fn details(&self, _: &[u64]) -> Result<Vec<AssetDetails>, String> {
        Err(UNSUPPORTED.into())
    }
    fn assets(&self, _: &AssetFilter, _: Option<&str>) -> Result<AssetPage, String> {
        Err(UNSUPPORTED.into())
    }
}

impl PlacesHost for Unsupported {
    fn suggest(&self, _: &str, _: Vec<Suggestion>) -> Result<(), String> {
        Err(UNSUPPORTED.into())
    }
    fn lookup(&self, _: &str) -> Vec<PlaceHit> {
        Vec::new()
    }
}

impl ProposalsHost for Unsupported {
    fn submit(&self, _: &str, _: Vec<NewProposal>) -> Result<(), String> {
        Err(UNSUPPORTED.into())
    }
}

impl ProviderHost for Unsupported {
    fn pick(&self, _: &PickRequest) -> Option<Result<PickOutcome, String>> {
        None
    }
    fn reveal(&self, _: &Path) -> Option<Result<(), String>> {
        None
    }
}

impl DetectHost for Unsupported {
    fn detect(&self, _: &str) -> Vec<DetectStatus> {
        Vec::new()
    }
    fn locate(&self, _: &str, _: &str, _: &Path) -> Result<DetectStatus, String> {
        Err(UNSUPPORTED.into())
    }
}

/// `library` over whichever library is open now, with Develop's filmstrip as the *current
/// selection* (the one the app reports to the backend), unless a tool call from a grid or the
/// viewer named its own photos (`ToolSelection`).
struct LiveLibrary(AppHandle);

impl LibraryHost for LiveLibrary {
    fn assets(&self, f: &AssetFilter, cursor: Option<&str>) -> Result<AssetPage, String> {
        let lib = self.0.state::<AppState>().library().map_err(|e| e.message)?;
        let app = self.0.clone();
        let selection: Selection = Arc::new(move || {
            let shared = &app.state::<AppState>().mcp.shared;
            shared.tool_selection.current().unwrap_or_else(|| shared.scope.get().order)
        });
        LibraryAssets::new(lib, selection).assets(f, cursor)
    }

    fn details(&self, ids: &[u64]) -> Result<Vec<AssetDetails>, String> {
        let lib = self.0.state::<AppState>().library().map_err(|e| e.message)?;
        LibraryAssets::new(lib, Arc::new(Vec::new)).details(ids)
    }
}

/// `proposals` into the open library's `plugin_proposals` (derived data, migration 0041).
struct LiveProposals(AppHandle);

impl ProposalsHost for LiveProposals {
    fn submit(&self, plugin: &str, batch: Vec<NewProposal>) -> Result<(), String> {
        let lib = self.0.state::<AppState>().library().map_err(|e| e.message)?;
        let now = chrono::Utc::now().timestamp();
        let n = lib.db.with_conn(|c| photos_core::proposals::submit(c, plugin, &batch, now)).map_err(|e| e.to_string())?;
        tracing::info!(plugin, submitted = batch.len(), stored = n, "plugin proposals");
        Ok(())
    }
}

/// A photo as the model should see it: upright and edited, from `decode_display_full`.
fn asset_image(app: AppHandle) -> AssetImage {
    Arc::new(move |id| {
        let state = app.state::<AppState>();
        let lib = state.library().map_err(|e| e.message)?;
        let id = i64::try_from(id).map_err(|_| "bad asset id".to_string())?;
        let info = lib.db.with_conn(|c| photos_core::assets::render_info(c, id)).map_err(|e| e.to_string())?;
        let source = lib.originals_dir().join(&info.relative_path);
        let store = photos_media::DerivativeStore::new(&lib.root);
        let spec = RenderSpec::from_info(&info, &source);
        crate::derivatives::decode_display_full(&store, &spec, state.ffmpeg().as_ref()).map_err(|e| e.message)
    })
}

/// What reaches the open library, built from the app handle by [`Plugins::attach`].
#[derive(Clone)]
pub struct Live {
    library: Arc<dyn LibraryHost>,
    proposals: Arc<dyn ProposalsHost>,
    image: AssetImage,
}

pub fn live(app: AppHandle) -> Live {
    Live {
        library: Arc::new(LiveLibrary(app.clone())),
        proposals: Arc::new(LiveProposals(app.clone())),
        image: asset_image(app),
    }
}

/// What a tools instance is given beyond 1.0: one host per granted permission.
pub type Grants = (
    Option<Arc<dyn ModelHost>>,
    Option<Arc<dyn LibraryHost>>,
    Option<Arc<dyn PlacesHost>>,
    Option<Arc<dyn ProposalsHost>>,
);

/// The 1.1 hosts of a tools instance: one per granted permission, `None` otherwise. Before the app
/// is attached (unit tests) `models`, `library` and `proposals` answer *unsupported*; `places`
/// needs only the plugins' own review store. `proposals` comes with any proposal permission
/// (the linker checks each proposal's own).
pub fn for_grants(granted: &[PluginPermission], plugins: &Arc<Plugins>) -> Grants {
    let has = |p| granted.contains(&p);
    let live = plugins.live().cloned();
    let models = has(PluginPermission::Models).then(|| match &live {
        Some(live) => {
            let weak = Arc::downgrade(plugins);
            let lookup: ModelLookup = Arc::new(move |id| {
                let p = weak.upgrade()?;
                let store = p.read();
                let inst = store.get(id)?;
                Some((inst.manifest.models.clone(), store.data_dir(id)))
            });
            Arc::new(AppModels::new(lookup, live.image.clone(), cfg!(windows))) as Arc<dyn ModelHost>
        }
        None => Arc::new(Unsupported) as Arc<dyn ModelHost>,
    });
    let library = has(PluginPermission::LibraryRead).then(|| match &live {
        Some(live) => live.library.clone(),
        None => Arc::new(Unsupported) as Arc<dyn LibraryHost>,
    });
    let places = has(PluginPermission::PlacesWrite).then(|| Arc::new(AppPlaces { store: plugins.reviews().clone() }) as Arc<dyn PlacesHost>);
    let proposals = PluginPermission::PROPOSING.into_iter().any(has).then(|| match &live {
        Some(live) => live.proposals.clone(),
        None => Arc::new(Unsupported) as Arc<dyn ProposalsHost>,
    });
    (models, library, places, proposals)
}
