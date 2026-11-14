//! Limits (spec §2): 30 s per call, each importer `progress` restarting the clock, cancel
//! stopping a call at the next tick, and 3 traps or timeouts in one session stopping a plugin.
//!
//! A store's epoch deadline is one tick ahead; at each tick [`on_tick`] looks at the call's
//! clock and the cancel flag and either grants one more tick or ends the call.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use wasmtime::component::{HasData, Linker, Resource};
use wasmtime::{ResourceLimiter, Store, StoreLimits, StoreLimitsBuilder, UpdateDeadline};
use wasmtime_wasi::clocks::{WasiClocksCtxView, WasiClocksView};
use wasmtime_wasi::p2::bindings::clocks::monotonic_clock;
use wasmtime_wasi::p2::DynPollable;

use super::ctx::HostState;
use super::{pool, OnStopped};
use crate::PluginError;

static CALL_SECS: AtomicU64 = AtomicU64::new(30);

/// Traps and timeouts that stop a plugin until Lumen restarts.
pub(crate) const MAX_TRIPS: u32 = 3;
/// The state a stopped plugin shows (spec §2, never an error dialog).
pub(crate) const STOPPED: &str = "Stopped working";

fn call_limit() -> Duration {
    Duration::from_secs(CALL_SECS.load(Ordering::Relaxed))
}

/// Shortens the per-call limit so timeout tests finish quickly.
#[doc(hidden)]
pub fn set_call_secs_for_tests(secs: u64) {
    CALL_SECS.store(secs, Ordering::Relaxed);
}

/// Why the host ended a call from its epoch callback.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Stop {
    Timeout,
    Cancelled,
}

impl std::fmt::Display for Stop {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Stop::Timeout => "timeout",
            Stop::Cancelled => "cancelled",
        })
    }
}

impl std::error::Error for Stop {}

/// The epoch callback: one more tick while the call is within its limit and not cancelled.
pub(crate) fn on_tick(state: &HostState) -> wasmtime::Result<UpdateDeadline> {
    if state.cancelled() {
        return Err(wasmtime::Error::new(Stop::Cancelled));
    }
    if state.started.elapsed() >= call_limit() {
        return Err(wasmtime::Error::new(Stop::Timeout));
    }
    Ok(UpdateDeadline::Continue(1))
}

/// What is left of the running call's time, plus one tick so the deadline has passed by then.
fn call_left(state: &HostState) -> Duration {
    if state.cancelled() {
        return Duration::ZERO;
    }
    call_limit().saturating_sub(state.started.elapsed()) + super::engine::TICK
}

/// `wasi:clocks/monotonic-clock` with every sleep cut to what is left of the call. A guest
/// blocked in a WASI sleep runs no guest code, so epochs alone would never stop it; cut short,
/// it wakes past its deadline and the next tick ends the call. Sleeps that fit are untouched.
pub(crate) struct CappedClock<'a> {
    inner: WasiClocksCtxView<'a>,
    left: Duration,
}

impl CappedClock<'_> {
    fn capped(&self, wait: Duration) -> Option<u64> {
        (wait > self.left).then(|| u64::try_from(self.left.as_nanos()).unwrap_or(u64::MAX))
    }
}

struct Capped;

impl HasData for Capped {
    type Data<'a> = CappedClock<'a>;
}

impl monotonic_clock::Host for CappedClock<'_> {
    fn now(&mut self) -> wasmtime::Result<u64> {
        monotonic_clock::Host::now(&mut self.inner)
    }

    fn resolution(&mut self) -> wasmtime::Result<u64> {
        monotonic_clock::Host::resolution(&mut self.inner)
    }

    fn subscribe_instant(&mut self, when: u64) -> wasmtime::Result<Resource<DynPollable>> {
        let now = monotonic_clock::Host::now(&mut self.inner)?;
        match self.capped(Duration::from_nanos(when.saturating_sub(now))) {
            Some(left) => monotonic_clock::Host::subscribe_duration(&mut self.inner, left),
            None => monotonic_clock::Host::subscribe_instant(&mut self.inner, when),
        }
    }

    fn subscribe_duration(&mut self, nanos: u64) -> wasmtime::Result<Resource<DynPollable>> {
        let nanos = self.capped(Duration::from_nanos(nanos)).unwrap_or(nanos);
        monotonic_clock::Host::subscribe_duration(&mut self.inner, nanos)
    }
}

/// Replaces WASI's monotonic clock in `l` (already linked by `add_to_linker_sync`).
pub(crate) fn link_capped_clock(l: &mut Linker<HostState>) -> wasmtime::Result<()> {
    l.allow_shadowing(true);
    let out = monotonic_clock::add_to_linker::<HostState, Capped>(l, |s| CappedClock { left: call_left(s), inner: s.clocks() });
    l.allow_shadowing(false);
    out
}

/// Memory one instance may hold (spec §2), counted over all its linear memories together:
/// wasmtime's `StoreLimits` caps each memory on its own, and a component can carry several.
pub(crate) struct Budget {
    max: usize,
    used: usize,
    inner: StoreLimits,
}

impl Budget {
    pub(crate) fn new(max: usize) -> Budget {
        Budget { max, used: 0, inner: StoreLimitsBuilder::new().memory_size(max).trap_on_grow_failure(true).build() }
    }
}

impl ResourceLimiter for Budget {
    fn memory_growing(&mut self, current: usize, desired: usize, maximum: Option<usize>) -> wasmtime::Result<bool> {
        let total = self.used.saturating_sub(current).saturating_add(desired);
        if total > self.max {
            return Err(wasmtime::Error::msg(format!("the plugin asked for more than {} MB of memory", self.max >> 20)));
        }
        let ok = self.inner.memory_growing(current, desired, maximum)?;
        if ok {
            self.used = total;
        }
        Ok(ok)
    }

    fn memory_grow_failed(&mut self, error: wasmtime::Error) -> wasmtime::Result<()> {
        self.inner.memory_grow_failed(error)
    }

    fn table_growing(&mut self, current: usize, desired: usize, maximum: Option<usize>) -> wasmtime::Result<bool> {
        self.inner.table_growing(current, desired, maximum)
    }

    fn table_grow_failed(&mut self, error: wasmtime::Error) -> wasmtime::Result<()> {
        self.inner.table_grow_failed(error)
    }
}

/// Per-plugin trips of one Lumen session.
pub(crate) struct Trips {
    counts: Mutex<HashMap<String, u32>>,
    stopped: Mutex<HashMap<String, String>>,
    on_stopped: OnStopped,
}

impl Trips {
    pub(crate) fn new(on_stopped: OnStopped) -> Trips {
        Trips { counts: Mutex::default(), stopped: Mutex::default(), on_stopped }
    }

    pub(crate) fn count(&self, id: &str) -> u32 {
        self.counts.lock().unwrap_or_else(|p| p.into_inner()).get(id).copied().unwrap_or(0)
    }

    pub(crate) fn stopped(&self, id: &str) -> Option<String> {
        self.stopped.lock().unwrap_or_else(|p| p.into_inner()).get(id).cloned()
    }

    /// Refuses to run a stopped plugin.
    pub(crate) fn check(&self, id: &str) -> Result<(), PluginError> {
        match self.stopped(id) {
            Some(_) => Err(PluginError::trap("This plugin stopped working. It runs again after Lumen restarts.")),
            None => Ok(()),
        }
    }

    fn trip(&self, id: &str) {
        let n = {
            let mut counts = self.counts.lock().unwrap_or_else(|p| p.into_inner());
            let n = counts.entry(id.to_string()).or_default();
            *n += 1;
            *n
        };
        if n < MAX_TRIPS {
            return;
        }
        let newly = self
            .stopped
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(id.to_string(), STOPPED.to_string())
            .is_none();
        if newly {
            tracing::warn!(plugin = id, trips = n, "plugin stopped working");
            (self.on_stopped)(id, STOPPED.to_string());
        }
    }

    /// Maps a wasmtime error to the plugin error it means, counting traps and timeouts.
    pub(crate) fn failed(&self, id: &str, e: wasmtime::Error) -> PluginError {
        match e.downcast_ref::<Stop>() {
            Some(Stop::Cancelled) => PluginError::invalid("cancelled"),
            Some(Stop::Timeout) => {
                self.trip(id);
                PluginError::timeout(format!("The plugin took longer than {} s.", call_limit().as_secs()))
            }
            None => {
                self.trip(id);
                tracing::warn!(plugin = id, error = %e, "plugin trapped");
                PluginError::trap(format!("The plugin stopped: {e}"))
            }
        }
    }

    /// Runs one call into `store` on the `lumen-plugins` pool, with a fresh clock.
    /// `Ok(Err(text))` is the guest's own error.
    pub(crate) fn run<R: Send>(
        &self,
        store: &mut Store<HostState>,
        f: impl FnOnce(&mut Store<HostState>) -> wasmtime::Result<R> + Send,
    ) -> Result<R, PluginError> {
        let id = store.data().ctx.plugin.clone();
        self.check(&id)?;
        store.data_mut().started = Instant::now();
        store.set_epoch_deadline(1);
        pool::run(|| f(store)).map_err(|e| self.failed(&id, e))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn budget_counts_every_memory() {
        let mb = 1 << 20;
        let mut b = Budget::new(512 * mb);
        assert!(b.memory_growing(0, 300 * mb, None).unwrap());
        assert!(b.memory_growing(300 * mb, 400 * mb, None).unwrap());
        // A second memory gets only what the first left.
        assert!(b.memory_growing(0, 200 * mb, None).is_err());
        assert!(b.memory_growing(0, 100 * mb, None).unwrap());
        assert!(b.memory_growing(400 * mb, 413 * mb, None).is_err());
    }
}
