//! The linker a plugin gets (spec §2): WASI p2 always, `host` always, and each other
//! `lumen:plugin` interface only when its permission is granted. A component importing an
//! interface that isn't linked is refused before it is instantiated.

use wasmtime::component::{Component, HasSelf, Linker};
use wasmtime::Store;

use super::bindings::lumen::plugin::{chosen, drafts, host, library, models, photos, places, proposals, storage, types};
use super::ctx::{HostCtx, HostState};
use super::imports::{need, Need};
use super::storage::Storage;
use super::wasi;
use crate::{PluginError, PluginPermission};
use photos_core::proposals::{NewProposal, ProposalValue};

/// Memory one instance may hold (spec §2).
pub(crate) const MEMORY: usize = 512 << 20;

fn link_err(e: wasmtime::Error) -> PluginError {
    PluginError::invalid(format!("The plugin engine couldn't link a plugin: {e}"))
}

pub(crate) fn build(engine: &wasmtime::Engine, granted: &[PluginPermission]) -> Result<Linker<HostState>, PluginError> {
    let mut l = Linker::<HostState>::new(engine);
    wasmtime_wasi::p2::add_to_linker_sync(&mut l).map_err(link_err)?;
    super::limits::link_capped_clock(&mut l).map_err(link_err)?;
    types::add_to_linker::<_, HasSelf<_>>(&mut l, |s| s).map_err(link_err)?;
    host::add_to_linker::<_, HasSelf<_>>(&mut l, |s| s).map_err(link_err)?;
    for p in granted {
        match p {
            PluginPermission::ImportReadChosen => chosen::add_to_linker::<_, HasSelf<_>>(&mut l, |s| s),
            PluginPermission::PhotosRead => photos::add_to_linker::<_, HasSelf<_>>(&mut l, |s| s),
            PluginPermission::DevelopWriteDrafts => drafts::add_to_linker::<_, HasSelf<_>>(&mut l, |s| s),
            PluginPermission::Storage => storage::add_to_linker::<_, HasSelf<_>>(&mut l, |s| s),
            PluginPermission::Models => models::add_to_linker::<_, HasSelf<_>>(&mut l, |s| s),
            PluginPermission::LibraryRead => library::add_to_linker::<_, HasSelf<_>>(&mut l, |s| s),
            PluginPermission::PlacesWrite => places::add_to_linker::<_, HasSelf<_>>(&mut l, |s| s),
            // No interface: the host writes GPS into originals on the person's *Write* (§8.7).
            // `proposals` is linked once below; these only gate its cases.
            PluginPermission::OriginalsGps
            | PluginPermission::DatesWrite
            | PluginPermission::TagsWrite
            | PluginPermission::CaptionsWrite
            | PluginPermission::PeopleWrite => Ok(()),
        }
        .map_err(link_err)?;
    }
    if PluginPermission::PROPOSING.iter().any(|p| granted.contains(p)) {
        proposals::add_to_linker::<_, HasSelf<_>>(&mut l, |s| s).map_err(link_err)?;
    }
    Ok(l)
}

/// Refuses a component that imports a `lumen:plugin` interface it wasn't granted, or anything
/// Lumen doesn't offer. Network imports are linked (and fail) so the install check owns that.
pub(crate) fn preflight(engine: &wasmtime::Engine, c: &Component, granted: &[PluginPermission]) -> Result<(), PluginError> {
    for (name, _) in c.component_type().imports(engine) {
        let shown = name.split('@').next().unwrap_or(name);
        match need(name) {
            Need::Permission(p) if !granted.contains(&p) => {
                return Err(PluginError::permission(format!("plugin.wasm uses {shown} without asking for it")))
            }
            Need::AnyOf(ps) if !ps.iter().any(|p| granted.contains(p)) => {
                return Err(PluginError::permission(format!("plugin.wasm uses {shown} without asking for it")))
            }
            Need::Unknown => {
                return Err(PluginError::permission(format!(
                    "plugin.wasm uses {shown}, which Lumen doesn't offer to plugins"
                )))
            }
            _ => {}
        }
    }
    Ok(())
}

pub(crate) fn store(engine: &wasmtime::Engine, ctx: HostCtx) -> Store<HostState> {
    let state = HostState {
        wasi: wasi::ctx(&ctx.plugin, &ctx.log),
        storage: Storage::new(ctx.data_dir.clone()),
        table: Default::default(),
        limits: super::limits::Budget::new(MEMORY),
        started: std::time::Instant::now(),
        ctx,
    };
    let mut store = Store::new(engine, state);
    store.limiter(|s| &mut s.limits);
    // An epoch-interrupted store's deadline starts at 0; every call re-arms it one tick ahead.
    store.epoch_deadline_callback(|s| super::limits::on_tick(s.data()));
    store.set_epoch_deadline(1);
    store
}

impl types::Host for HostState {}

impl host::Host for HostState {
    fn log(&mut self, line: String) {
        HostState::log(self, &line);
    }

    /// A setting's value as JSON text (`true`, `3`, `"text"`), or none when it isn't set.
    fn setting(&mut self, key: String) -> Option<String> {
        self.ctx.settings.get(&key).map(|v| v.to_string())
    }
}

impl photos::Host for HostState {
    fn scope(&mut self) -> Vec<i64> {
        self.ctx.photos.as_ref().map(|p| p.scope()).unwrap_or_default()
    }

    fn metadata(&mut self, id: i64) -> Result<types::PhotoMeta, String> {
        self.ctx.photos.as_ref().ok_or("no photos here")?.metadata(id)
    }

    fn preview(&mut self, id: i64, max: u32) -> Result<Vec<u8>, String> {
        self.ctx.photos.as_ref().ok_or("no photos here")?.preview(id, max.min(1024))
    }

    fn recipe(&mut self, id: i64) -> Result<String, String> {
        self.ctx.photos.as_ref().ok_or("no photos here")?.recipe(id)
    }
}

impl drafts::Host for HostState {
    fn patch(&mut self, id: i64, merge_patch: String, label: String) -> Result<(), String> {
        self.ctx.drafts.as_ref().ok_or("no drafts here")?.patch(id, &merge_patch, &label)
    }
}

/// Why a 1.1 host call can't be answered: Lumen gave this instance no such host. The linker only
/// links an interface whose permission is granted, so this is a host that isn't built yet here.
fn missing(what: &str) -> String {
    format!("{what} isn't available to plugins here")
}

impl models::Host for HostState {
    fn run(&mut self, model: String, inputs: Vec<types::Tensor>) -> Result<Vec<types::Tensor>, String> {
        let h = self.ctx.models.clone().ok_or_else(|| missing("models"))?;
        h.run(&self.ctx.plugin, &model, inputs)
    }

    fn run_on_asset(&mut self, model: String, asset: u64, input_name: String) -> Result<Vec<types::Tensor>, String> {
        let h = self.ctx.models.clone().ok_or_else(|| missing("models"))?;
        h.run_on_asset(&self.ctx.plugin, &model, asset, &input_name)
    }

    fn nearest(&mut self, source: types::VectorSource, query: Vec<f32>, k: u32) -> Result<Vec<types::Neighbour>, String> {
        let h = self.ctx.models.clone().ok_or_else(|| missing("models"))?;
        h.nearest(&self.ctx.plugin, &source, &query, k)
    }
}

impl library::Host for HostState {
    fn assets(&mut self, filter: types::AssetFilter, cursor: Option<String>) -> Result<types::AssetPage, String> {
        let h = self.ctx.library.clone().ok_or_else(|| missing("library"))?;
        h.assets(&filter, cursor.as_deref())
    }

    fn details(&mut self, ids: Vec<u64>) -> Result<Vec<types::AssetDetails>, String> {
        let h = self.ctx.library.clone().ok_or_else(|| missing("library"))?;
        if ids.len() > 200 {
            return Err("details takes at most 200 assets a call".to_string());
        }
        h.details(&ids)
    }
}

impl places::Host for HostState {
    fn suggest(&mut self, batch: Vec<types::Suggestion>) -> Result<(), String> {
        let h = self.ctx.places.clone().ok_or_else(|| missing("places"))?;
        h.suggest(&self.ctx.plugin, batch)
    }

    fn lookup(&mut self, name: String) -> Vec<types::PlaceHit> {
        self.ctx.places.as_ref().map(|h| h.lookup(&name)).unwrap_or_default()
    }
}

/// The permission a proposal's kind needs.
fn proposal_permission(v: &types::Proposed) -> PluginPermission {
    match v {
        types::Proposed::Location(_) => PluginPermission::PlacesWrite,
        types::Proposed::Date(_) => PluginPermission::DatesWrite,
        types::Proposed::Tags(_) | types::Proposed::Label(_) => PluginPermission::TagsWrite,
        types::Proposed::Title(_) | types::Proposed::Caption(_) => PluginPermission::CaptionsWrite,
        types::Proposed::Person(_) => PluginPermission::PeopleWrite,
    }
}

/// A WIT proposal as `photos-core` stores it, checked against the instance's grants.
pub(crate) fn to_core(p: types::Proposal, granted: &[PluginPermission]) -> Result<NewProposal, String> {
    let need = proposal_permission(&p.value);
    if !granted.contains(&need) {
        return Err(format!("this plugin hasn't been given {}", need.id()));
    }
    let id = |n: u64, what: &str| i64::try_from(n).map_err(|_| format!("bad {what} id {n}"));
    let value = match p.value {
        types::Proposed::Location(l) => ProposalValue::Location { lat: l.lat, lon: l.lon },
        types::Proposed::Date(d) => ProposalValue::Date(d),
        types::Proposed::Tags(t) => ProposalValue::Tags(t),
        types::Proposed::Label(l) => ProposalValue::Label(l),
        types::Proposed::Person(f) => ProposalValue::Person { face: id(f.face, "face")?, name: f.name },
        types::Proposed::Title(t) => ProposalValue::Title(t),
        types::Proposed::Caption(c) => ProposalValue::Caption(c),
    };
    Ok(NewProposal { asset_id: id(p.asset, "asset")?, value, confidence: p.confidence, reason: p.reason })
}

impl proposals::Host for HostState {
    fn submit(&mut self, batch: Vec<types::Proposal>) -> Result<(), String> {
        if batch.len() > photos_core::proposals::MAX_BATCH {
            return Err(format!("a batch has more than {} proposals", photos_core::proposals::MAX_BATCH));
        }
        let h = self.ctx.proposals.clone().ok_or_else(|| missing("proposals"))?;
        let core = batch.into_iter().map(|p| to_core(p, &self.ctx.granted)).collect::<Result<Vec<_>, _>>()?;
        let checked = photos_core::proposals::normalise_batch(core)?;
        h.submit(&self.ctx.plugin, checked)
    }
}
