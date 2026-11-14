//! The one wasmtime engine (spec §2): component model on cranelift, epoch interruption ticked
//! every 100 ms by a thread that ends with the engine, Lumen's own compile cache, the session's
//! trip counts and the shared tools instances.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use wasmtime::component::Component;

use super::ctx::HostCtx;
use super::limits::Trips;
use super::tools::{Reaper, ToolsInstance};
use super::{cache, linker, OnStopped};
use crate::store::Installed;
use crate::PluginError;

/// How often the epoch advances; the call clock and cancel are looked at once per tick.
pub(crate) const TICK: Duration = Duration::from_millis(100);
/// Ticks between two looks for idle shared instances.
const REAP_EVERY: u32 = 10;

/// Runs code plugins: one per Lumen, built on first use.
pub struct Engine {
    pub(crate) wasm: wasmtime::Engine,
    cache_dir: PathBuf,
    cache_key: String,
    compiles: AtomicU64,
    pub(crate) trips: Arc<Trips>,
    tools: Mutex<HashMap<String, Arc<ToolsInstance>>>,
    reaper: Arc<Reaper>,
}

impl Engine {
    pub fn new(cache_dir: &Path, on_stopped: OnStopped) -> Result<Engine, PluginError> {
        let mut config = wasmtime::Config::new();
        config
            .wasm_component_model(true)
            .epoch_interruption(true)
            .cranelift_opt_level(wasmtime::OptLevel::Speed);
        let wasm = wasmtime::Engine::new(&config)
            .map_err(|e| PluginError::invalid(format!("The plugin engine couldn't start: {e}")))?;
        let weak = wasm.weak();
        let reaper = Arc::new(Reaper::default());
        let reaping = reaper.clone();
        std::thread::Builder::new()
            .name("lumen-plugins-epoch".into())
            .spawn(move || {
                let mut n = 0u32;
                while let Some(engine) = weak.upgrade() {
                    engine.increment_epoch();
                    drop(engine);
                    n = n.wrapping_add(1);
                    if n.is_multiple_of(REAP_EVERY) {
                        reaping.reap();
                    }
                    std::thread::sleep(TICK);
                }
            })
            .map_err(|e| PluginError::invalid(format!("The plugin engine couldn't start: {e}")))?;
        Ok(Engine {
            cache_key: cache::engine_key(&wasm),
            wasm,
            cache_dir: cache_dir.to_path_buf(),
            compiles: AtomicU64::new(0),
            trips: Arc::new(Trips::new(on_stopped)),
            tools: Mutex::default(),
            reaper,
        })
    }

    /// The compiled component for a `.wasm` file, from the cache when it is intact.
    pub fn component(&self, wasm_file: &Path) -> Result<Component, PluginError> {
        self.component_from(&read_wasm(wasm_file)?)
    }

    pub(crate) fn component_from(&self, bytes: &[u8]) -> Result<Component, PluginError> {
        if let Some(c) = cache::load(&self.wasm, &self.cache_dir, bytes, &self.cache_key) {
            return Ok(c);
        }
        let c = Component::new(&self.wasm, bytes)
            .map_err(|e| PluginError::invalid(format!("plugin.wasm isn't a valid component: {e}")))?;
        self.compiles.fetch_add(1, Ordering::Relaxed);
        cache::store(&c, &self.cache_dir, bytes, &self.cache_key);
        Ok(c)
    }

    /// How many components this engine compiled (rather than read from the cache).
    #[doc(hidden)]
    pub fn compiles(&self) -> u64 {
        self.compiles.load(Ordering::Relaxed)
    }

    /// Traps and timeouts of a plugin in this session.
    pub fn trips(&self, id: &str) -> u32 {
        self.trips.count(id)
    }

    /// The reason a plugin was stopped in this session (*Stopped working*), if it was.
    pub fn stopped(&self, id: &str) -> Option<String> {
        self.trips.stopped(id)
    }

    /// The shared tools instance of an installed plugin; `ctx_for` builds its context each time
    /// the instance is (re)built.
    pub fn tools(
        &self,
        inst: &Installed,
        ctx_for: impl Fn() -> HostCtx + Send + Sync + 'static,
    ) -> Result<Arc<ToolsInstance>, PluginError> {
        self.tools_at(&inst.manifest.id, &inst.dir.join("plugin.wasm"), ctx_for)
    }

    /// [`Engine::tools`] over a bare `.wasm` file (tests and Developer mode). The same plugin id
    /// and the same file give the same instance; a changed file replaces it.
    #[doc(hidden)]
    pub fn tools_at(
        &self,
        id: &str,
        wasm: &Path,
        ctx_for: impl Fn() -> HostCtx + Send + Sync + 'static,
    ) -> Result<Arc<ToolsInstance>, PluginError> {
        self.trips.check(id)?;
        let bytes = read_wasm(wasm)?;
        let hash = blake3::hash(&bytes);
        let mut map = self.tools.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(t) = map.get(id).filter(|t| t.wasm_hash == hash) {
            return Ok(t.clone());
        }
        let component = self.component_from(&bytes)?;
        let first = ctx_for();
        linker::preflight(&self.wasm, &component, &first.granted)?;
        let pre = linker::build(&self.wasm, &first.granted)?
            .instantiate_pre(&component)
            .map_err(|e| PluginError::permission(format!("plugin.wasm can't run here: {e}")))?;
        let t = Arc::new(ToolsInstance::new(&self.wasm, hash, pre, first, Box::new(ctx_for), self.trips.clone())?);
        self.reaper.watch(&t);
        map.insert(id.to_string(), t.clone());
        Ok(t)
    }
}

fn read_wasm(path: &Path) -> Result<Vec<u8>, PluginError> {
    std::fs::read(path).map_err(|e| PluginError::invalid(format!("plugin.wasm couldn't be read: {e}")))
}
