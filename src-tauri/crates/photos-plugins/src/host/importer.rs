//! One importer instance (spec §10): built per import, its calls run one after another on the
//! `lumen-plugins` pool while the import job's thread waits.

use std::path::Path;
use std::sync::Arc;

use wasmtime::component::Instance;
use wasmtime::Store;

use super::bindings::exports::lumen::plugin::importer::{Guest, GuestIndices};
use super::bindings::lumen::plugin::types::{Extras, Item};
use super::ctx::{HostCtx, HostState};
use super::limits::Trips;
use super::{linker, Engine};
use crate::store::Installed;
use crate::PluginError;

pub struct ImporterInstance {
    store: Store<HostState>,
    guest: Guest,
    trips: Arc<Trips>,
    /// The first trap, timeout or cancel. wasmtime won't enter an instance after one, so every
    /// later call returns it again instead of counting another trip.
    spent: Option<PluginError>,
    _instance: Instance,
}

impl Engine {
    /// A fresh importer for one import of an installed plugin.
    pub fn importer(&self, inst: &Installed, ctx: HostCtx) -> Result<ImporterInstance, PluginError> {
        self.importer_at(&inst.dir.join("plugin.wasm"), ctx)
    }

    /// [`Engine::importer`] over a bare `.wasm` file (tests and Developer mode).
    #[doc(hidden)]
    pub fn importer_at(&self, wasm: &Path, ctx: HostCtx) -> Result<ImporterInstance, PluginError> {
        self.trips.check(&ctx.plugin)?;
        let component = self.component(wasm)?;
        linker::preflight(&self.wasm, &component, &ctx.granted)?;
        let linker = linker::build(&self.wasm, &ctx.granted)?;
        let pre = linker
            .instantiate_pre(&component)
            .map_err(|e| PluginError::permission(format!("plugin.wasm can't run here: {e}")))?;
        let indices = GuestIndices::new(&pre)
            .map_err(|_| PluginError::invalid("plugin.wasm doesn't export an importer"))?;
        let mut store = linker::store(&self.wasm, ctx);
        let (instance, guest) = self.trips.run(&mut store, |s| {
            let instance = pre.instantiate(&mut *s)?;
            let guest = indices.load(&mut *s, &instance)?;
            Ok((instance, guest))
        })?;
        Ok(ImporterInstance { store, guest, trips: self.trips.clone(), spent: None, _instance: instance })
    }
}

impl ImporterInstance {
    /// Runs one guest call unless the instance is spent; a host-side failure spends it.
    fn call<R: Send>(&mut self, f: impl FnOnce(&Guest, &mut Store<HostState>) -> wasmtime::Result<R> + Send) -> Result<R, PluginError> {
        if let Some(e) = &self.spent {
            return Err(e.clone());
        }
        let g = &self.guest;
        let out = self.trips.run(&mut self.store, |s| f(g, s));
        if let Err(e) = &out {
            self.spent = Some(e.clone());
        }
        out
    }

    pub fn enumerate(&mut self) -> Result<Vec<Item>, PluginError> {
        self.call(|g, s| g.call_enumerate(s))?.map_err(PluginError::invalid)
    }

    pub fn read(&mut self, key: &str, offset: u64, len: u32) -> Result<Vec<u8>, PluginError> {
        self.call(|g, s| g.call_read(s, key, offset, len))?.map_err(PluginError::invalid)
    }

    pub fn extras(&mut self, key: &str) -> Result<Option<Extras>, PluginError> {
        self.call(|g, s| g.call_extras(s, key))
    }

    pub fn sidecars(&mut self, key: &str) -> Result<Vec<Item>, PluginError> {
        self.call(|g, s| g.call_sidecars(s, key))
    }
}
