//! The linker a plugin gets (spec §2): WASI p2 always, `host` always, and each other
//! `lumen:plugin` interface only when its permission is granted. A component importing an
//! interface that isn't linked is refused before it is instantiated.

use wasmtime::component::{Component, HasSelf, Linker};
use wasmtime::Store;

use super::bindings::lumen::plugin::{chosen, drafts, host, photos, storage, types};
use super::ctx::{HostCtx, HostState};
use super::imports::{need, Need};
use super::storage::Storage;
use super::wasi;
use crate::{PluginError, PluginPermission};

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
        }
        .map_err(link_err)?;
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
