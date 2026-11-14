//! The shared tools instance of a plugin (spec §2, §10): one per plugin, its calls run one at a
//! time, built on the first call and dropped after the idle limit (or after a trap).

use std::sync::{Arc, Mutex, MutexGuard, Weak};
use std::time::Instant;

use wasmtime::component::InstancePre;
use wasmtime::Store;

use super::bindings::exports::lumen::plugin::tools::{Guest, GuestIndices};
use super::bindings::lumen::plugin::types::Tool;
use super::ctx::{HostCtx, HostState};
use super::limits::Trips;
use super::{linker, pool};
use crate::PluginError;

/// Builds the context each time the instance is (re)built.
pub(crate) type CtxFor = Box<dyn Fn() -> HostCtx + Send + Sync>;

struct Live {
    store: Store<HostState>,
    guest: Guest,
    used: Instant,
}

pub struct ToolsInstance {
    pub(crate) wasm_hash: blake3::Hash,
    engine: wasmtime::Engine,
    pre: InstancePre<HostState>,
    indices: GuestIndices,
    ctx_for: CtxFor,
    /// The context [super::Engine::tools] built to read the grants, used for the first build.
    first: Mutex<Option<HostCtx>>,
    trips: Arc<Trips>,
    live: Mutex<Option<Live>>,
}

impl ToolsInstance {
    pub(crate) fn new(
        engine: &wasmtime::Engine,
        wasm_hash: blake3::Hash,
        pre: InstancePre<HostState>,
        first: HostCtx,
        ctx_for: CtxFor,
        trips: Arc<Trips>,
    ) -> Result<ToolsInstance, PluginError> {
        let indices = GuestIndices::new(&pre).map_err(|_| PluginError::invalid("plugin.wasm doesn't export tools"))?;
        Ok(ToolsInstance {
            wasm_hash,
            engine: engine.clone(),
            pre,
            indices,
            ctx_for,
            first: Mutex::new(Some(first)),
            trips,
            live: Mutex::new(None),
        })
    }

    fn slot(&self) -> MutexGuard<'_, Option<Live>> {
        self.live.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// Runs one call with the instance held, building it first when it isn't live. A trap drops
    /// it so the next call starts clean.
    fn with<R: Send>(
        &self,
        f: impl FnOnce(&Guest, &mut Store<HostState>) -> wasmtime::Result<R> + Send,
    ) -> Result<R, PluginError> {
        let mut slot = self.slot();
        if slot.is_none() {
            let first = self.first.lock().unwrap_or_else(|p| p.into_inner()).take();
            let mut store = linker::store(&self.engine, first.unwrap_or_else(|| (self.ctx_for)()));
            let (pre, indices) = (&self.pre, &self.indices);
            let guest = self.trips.run(&mut store, |s| {
                let instance = pre.instantiate(&mut *s)?;
                indices.load(&mut *s, &instance)
            })?;
            *slot = Some(Live { store, guest, used: Instant::now() });
        }
        let live = slot.as_mut().expect("built above");
        let guest = &live.guest;
        let out = self.trips.run(&mut live.store, |s| f(guest, s));
        live.used = Instant::now();
        if matches!(&out, Err(e) if e.code != "plugin_invalid") {
            *slot = None;
        }
        out
    }

    /// The tools the plugin offers.
    pub fn list(&self) -> Result<Vec<Tool>, PluginError> {
        self.with(|g, s| g.call_tools(s))
    }

    /// Calls one tool with its arguments as JSON text; returns the plugin's JSON text.
    pub fn call(&self, name: &str, args_json: &str) -> Result<String, PluginError> {
        self.with(|g, s| g.call_call(s, name, args_json))?.map_err(PluginError::invalid)
    }

    /// Whether an instance is held right now (it is dropped when idle).
    #[doc(hidden)]
    pub fn is_live(&self) -> bool {
        self.slot().is_some()
    }

    /// Drops the instance when it has been idle past the limit; skips one that is busy.
    fn reap(&self) {
        if let Ok(mut slot) = self.live.try_lock() {
            if slot.as_ref().is_some_and(|l| l.used.elapsed() >= pool::idle()) {
                *slot = None;
            }
        }
    }
}

/// The shared instances the engine's ticker looks at for idleness.
#[derive(Default)]
pub(crate) struct Reaper(Mutex<Vec<Weak<ToolsInstance>>>);

impl Reaper {
    pub(crate) fn watch(&self, t: &Arc<ToolsInstance>) {
        self.0.lock().unwrap_or_else(|p| p.into_inner()).push(Arc::downgrade(t));
    }

    pub(crate) fn reap(&self) {
        let alive: Vec<Arc<ToolsInstance>> = {
            let mut list = self.0.lock().unwrap_or_else(|p| p.into_inner());
            list.retain(|w| w.strong_count() > 0);
            list.iter().filter_map(Weak::upgrade).collect()
        };
        for t in alive {
            t.reap();
        }
    }
}
