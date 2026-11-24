//! The shared tools instance of a plugin (spec §2, §10): one per plugin, its calls run one at a
//! time, built on the first call and dropped after the idle limit (or after a trap).

use std::sync::{Arc, Mutex, MutexGuard, Weak};
use std::time::Instant;

use wasmtime::component::InstancePre;
use wasmtime::Store;

use super::bindings::exports::lumen::plugin::{review, ui};
use super::bindings::exports::lumen::plugin::tools::{Guest, GuestIndices};
use super::bindings::lumen::plugin::types::{ReviewEvent, Suggestion, Tool};
use super::ctx::{HostCtx, HostState};
use super::limits::Trips;
use super::{linker, pool};
use crate::PluginError;

/// Builds the context each time the instance is (re)built.
pub(crate) type CtxFor = Box<dyn Fn() -> HostCtx + Send + Sync>;

struct Live {
    store: Store<HostState>,
    guest: Guest,
    /// The `review` export (API 1.1), when the plugin has one.
    review: Option<review::Guest>,
    /// The `ui` export (API 1.2), when the plugin has one.
    ui: Option<ui::Guest>,
    used: Instant,
}

pub struct ToolsInstance {
    pub(crate) wasm_hash: blake3::Hash,
    engine: wasmtime::Engine,
    pre: InstancePre<HostState>,
    indices: GuestIndices,
    review: Option<review::GuestIndices>,
    ui: Option<ui::GuestIndices>,
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
        let review = review::GuestIndices::new(&pre).ok();
        let ui = ui::GuestIndices::new(&pre).ok();
        Ok(ToolsInstance {
            wasm_hash,
            engine: engine.clone(),
            pre,
            indices,
            review,
            ui,
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
        f: impl FnOnce(&Guest, Option<&review::Guest>, Option<&ui::Guest>, &mut Store<HostState>) -> wasmtime::Result<R> + Send,
    ) -> Result<R, PluginError> {
        let mut slot = self.slot();
        if slot.is_none() {
            let first = self.first.lock().unwrap_or_else(|p| p.into_inner()).take();
            let mut store = linker::store(&self.engine, first.unwrap_or_else(|| (self.ctx_for)()));
            let (pre, indices, review, ui) = (&self.pre, &self.indices, &self.review, &self.ui);
            let (guest, review, ui) = self.trips.run(&mut store, |s| {
                let instance = pre.instantiate(&mut *s)?;
                let guest = indices.load(&mut *s, &instance)?;
                let review = review.as_ref().map(|r| r.load(&mut *s, &instance)).transpose()?;
                let ui = ui.as_ref().map(|u| u.load(&mut *s, &instance)).transpose()?;
                Ok((guest, review, ui))
            })?;
            *slot = Some(Live { store, guest, review, ui, used: Instant::now() });
        }
        let live = slot.as_mut().expect("built above");
        let (guest, review, ui) = (&live.guest, live.review.as_ref(), live.ui.as_ref());
        let out = self.trips.run(&mut live.store, |s| f(guest, review, ui, s));
        live.used = Instant::now();
        if matches!(&out, Err(e) if e.code != "plugin_invalid") {
            *slot = None;
        }
        out
    }

    /// The tools the plugin offers.
    pub fn list(&self) -> Result<Vec<Tool>, PluginError> {
        self.with(|g, _, _, s| g.call_tools(s))
    }

    /// Calls one tool with its arguments as JSON text; returns the plugin's JSON text.
    pub fn call(&self, name: &str, args_json: &str) -> Result<String, PluginError> {
        self.with(|g, _, _, s| g.call_call(s, name, args_json))?.map_err(PluginError::invalid)
    }

    /// Whether the plugin answers the Place review (exports `review`, API 1.1).
    pub fn has_review(&self) -> bool {
        self.review.is_some()
    }

    /// Sends one Place review action; returns only the suggestions it changed (spec §8.6).
    pub fn on_review(&self, event: &ReviewEvent) -> Result<Vec<Suggestion>, PluginError> {
        if self.review.is_none() {
            return Err(PluginError::invalid("This plugin doesn't answer the Place review"));
        }
        self.with(|_, r, _, s| r.expect("checked above").call_on_review(s, event))
    }

    /// Whether the plugin answers its settings page's buttons, stats and lists (exports `ui`, API 1.2).
    pub fn has_ui(&self) -> bool {
        self.ui.is_some()
    }

    /// Runs a `button` control's action; the text is what the page shows as the outcome.
    pub fn ui_action(&self, name: &str) -> Result<String, PluginError> {
        if self.ui.is_none() {
            return Err(PluginError::invalid("This plugin has no buttons"));
        }
        self.with(|_, _, u, s| u.expect("checked above").call_action(s, name))?.map_err(PluginError::invalid)
    }

    /// Reads a `stat` or `list` control's value (plain text / a JSON array).
    pub fn ui_value(&self, key: &str) -> Result<String, PluginError> {
        if self.ui.is_none() {
            return Err(PluginError::invalid("This plugin has no live values"));
        }
        self.with(|_, _, u, s| u.expect("checked above").call_value(s, key))?.map_err(PluginError::invalid)
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
