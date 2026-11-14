//! Priority job scheduler: four lanes drained strictly in order, key coalescing,
//! cooperative cancellation, bounded queue with blocking backpressure.
//!
//! No Tauri, no async. Workers are plain threads; callers block on a
//! [`JobHandle`] when they need the result.

pub mod activity;
mod interactive;
mod priority;
pub use interactive::{interactive, user_busy};
pub use priority::background_priority;
use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

/// Drained strictly in this order: an Interactive job always runs before any Prefetch job, and so on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Lane {
    Interactive = 0,
    Prefetch = 1,
    Import = 2,
    Maintenance = 3,
}

#[derive(Debug, Clone, Default)]
pub struct CancelToken(Arc<AtomicBool>);

impl CancelToken {
    pub fn new() -> Self {
        Self::default()
    }
    /// A token over a flag someone else also holds and sets (a plugin importer's, which also
    /// interrupts the plugin's running call): cancelling either is cancelling both.
    pub fn from_flag(flag: Arc<AtomicBool>) -> Self {
        Self(flag)
    }
    pub fn cancel(&self) {
        self.0.store(true, Ordering::SeqCst);
    }
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
    /// The flag itself, for work in a crate that takes an `&AtomicBool` (`photos_ml`).
    pub fn flag(&self) -> &AtomicBool {
        &self.0
    }
}

#[derive(Default)]
struct Done {
    flag: Mutex<bool>,
    cv: Condvar,
}

#[derive(Clone)]
pub struct JobHandle {
    done: Arc<Done>,
    pub cancel: CancelToken,
}

impl JobHandle {
    pub fn wait(&self) {
        let mut f = self.done.flag.lock().unwrap();
        while !*f {
            f = self.done.cv.wait(f).unwrap();
        }
    }

    /// Returns `true` when the job finished within `d`.
    pub fn wait_timeout(&self, d: Duration) -> bool {
        let f = self.done.flag.lock().unwrap();
        let (f, _) = self.done.cv.wait_timeout_while(f, d, |done| !*done).unwrap();
        *f
    }

    pub fn is_done(&self) -> bool {
        *self.done.flag.lock().unwrap()
    }
}

type Task = Box<dyn FnOnce(&CancelToken) + Send + 'static>;

struct Queued {
    key: String,
    lane: Lane,
    task: Task,
    handle: JobHandle,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Stats {
    pub queued: [usize; 4],
    pub running: usize,
    /// Worker threads available; surfaced so the UI can say how wide the work is running.
    pub workers: usize,
}

/// A job a worker is running right now, and for how long (what the watchdog reads).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunningJob {
    pub key: String,
    pub lane: Lane,
    pub for_ms: u64,
}

struct Inner {
    queues: Mutex<[VecDeque<Queued>; 4]>,
    /// key → handle for every queued or running job (the coalescing table)
    inflight: Mutex<HashMap<String, JobHandle>>,
    work_cv: Condvar,
    space_cv: Condvar,
    capacity: usize,
    running: AtomicUsize,
    /// id → (key, lane, started) for every job a worker is running
    running_since: Mutex<HashMap<u64, (String, Lane, Instant)>>,
    next_id: AtomicU64,
    shutdown: AtomicBool,
}

#[derive(Clone)]
pub struct Scheduler {
    inner: Arc<Inner>,
}

impl Scheduler {
    pub fn new(workers: usize, capacity: usize) -> Self {
        let inner = Arc::new(Inner {
            queues: Mutex::new(Default::default()),
            inflight: Mutex::new(HashMap::new()),
            work_cv: Condvar::new(),
            space_cv: Condvar::new(),
            capacity: capacity.max(1),
            running: AtomicUsize::new(0),
            running_since: Mutex::new(HashMap::new()),
            next_id: AtomicU64::new(0),
            shutdown: AtomicBool::new(false),
        });
        for i in 0..workers.max(1) {
            let inner = inner.clone();
            std::thread::Builder::new()
                .name(format!("lumen-worker-{i}"))
                .spawn(move || {
                    priority::background_priority();
                    worker(inner, false)
                })
                .expect("spawn worker");
        }
        interactive::warm();
        // One thread only ever takes Interactive jobs, so an on-screen thumbnail never
        // waits behind an import that has every general worker busy.
        {
            let inner = inner.clone();
            std::thread::Builder::new()
                .name("lumen-worker-interactive".into())
                .spawn(move || worker(inner, true))
                .expect("spawn interactive worker");
        }
        Self { inner }
    }

    /// Coalesces on `key`: a second submit for a queued or running key returns a handle to the first.
    /// Blocks while the queue is at capacity, except on the Interactive lane: what the user is
    /// looking at must never wait for bulk background work to drain.
    pub fn submit(&self, key: &str, lane: Lane, f: impl FnOnce(&CancelToken) + Send + 'static) -> JobHandle {
        if let Some(h) = self.inner.inflight.lock().unwrap().get(key) {
            return h.clone();
        }
        let handle = JobHandle { done: Arc::new(Done::default()), cancel: CancelToken::new() };
        let mut queues = self.inner.queues.lock().unwrap();
        while lane != Lane::Interactive
            && queues.iter().map(|q| q.len()).sum::<usize>() >= self.inner.capacity
            && !self.inner.shutdown.load(Ordering::SeqCst)
        {
            queues = self.inner.space_cv.wait(queues).unwrap();
        }
        self.inner.inflight.lock().unwrap().insert(key.to_string(), handle.clone());
        queues[lane as usize].push_back(Queued { key: key.to_string(), lane, task: Box::new(f), handle: handle.clone() });
        drop(queues);
        self.inner.work_cv.notify_all();
        handle
    }

    /// Whether a job under `key` is queued or running.
    pub fn is_inflight(&self, key: &str) -> bool {
        self.inner.inflight.lock().unwrap().contains_key(key)
    }

    /// Flags the token; a job still queued is dropped without running.
    pub fn cancel(&self, key: &str) {
        let handle = self.inner.inflight.lock().unwrap().get(key).cloned();
        let Some(handle) = handle else { return };
        handle.cancel.cancel();
        let removed = {
            let mut queues = self.inner.queues.lock().unwrap();
            let mut removed = None;
            for q in queues.iter_mut() {
                if let Some(pos) = q.iter().position(|j| j.key == key) {
                    removed = q.remove(pos);
                    break;
                }
            }
            removed
        };
        if let Some(j) = removed {
            finish(&self.inner, &j.key, &j.handle);
            self.inner.space_cv.notify_one();
        }
    }

    pub fn stats(&self) -> Stats {
        let q = self.inner.queues.lock().unwrap();
        Stats {
            queued: [q[0].len(), q[1].len(), q[2].len(), q[3].len()],
            running: self.inner.running.load(Ordering::SeqCst),
            workers: self.inner.capacity,
        }
    }

    /// The jobs running right now, longest first.
    pub fn running(&self) -> Vec<RunningJob> {
        let mut v: Vec<RunningJob> = self
            .inner
            .running_since
            .lock()
            .unwrap()
            .values()
            .map(|(key, lane, at)| RunningJob { key: key.clone(), lane: *lane, for_ms: at.elapsed().as_millis() as u64 })
            .collect();
        v.sort_by_key(|j| std::cmp::Reverse(j.for_ms));
        v
    }

    pub fn shutdown(&self) {
        self.inner.shutdown.store(true, Ordering::SeqCst);
        self.inner.work_cv.notify_all();
        self.inner.space_cv.notify_all();
    }
}

fn finish(inner: &Inner, key: &str, handle: &JobHandle) {
    inner.inflight.lock().unwrap().remove(key);
    *handle.done.flag.lock().unwrap() = true;
    handle.done.cv.notify_all();
}

fn worker(inner: Arc<Inner>, interactive_only: bool) {
    loop {
        let job = {
            let mut queues = inner.queues.lock().unwrap();
            loop {
                if inner.shutdown.load(Ordering::SeqCst) {
                    return;
                }
                let next = if interactive_only {
                    queues[Lane::Interactive as usize].pop_front()
                } else {
                    queues.iter_mut().find_map(|q| q.pop_front())
                };
                if let Some(j) = next {
                    break j;
                }
                queues = inner.work_cv.wait(queues).unwrap();
            }
        };
        inner.space_cv.notify_one();
        inner.running.fetch_add(1, Ordering::SeqCst);
        let id = inner.next_id.fetch_add(1, Ordering::Relaxed);
        inner.running_since.lock().unwrap().insert(id, (job.key.clone(), job.lane, Instant::now()));
        if !job.handle.cancel.is_cancelled() {
            let cancel = job.handle.cancel.clone();
            let task = job.task;
            // What the user is waiting on renders on the render pool, whichever worker took it.
            let run = move || task(&cancel);
            let result = if job.lane == Lane::Interactive {
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || interactive(run)))
            } else {
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(run))
            };
            if result.is_err() {
                tracing::error!(key = %job.key, "job panicked");
            }
        }
        inner.running_since.lock().unwrap().remove(&id);
        inner.running.fetch_sub(1, Ordering::SeqCst);
        finish(&inner, &job.key, &job.handle);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gate() -> Arc<(Mutex<bool>, Condvar)> {
        Arc::new((Mutex::new(false), Condvar::new()))
    }
    fn hold(g: &Arc<(Mutex<bool>, Condvar)>) {
        let (m, c) = &**g;
        let mut open = m.lock().unwrap();
        while !*open {
            open = c.wait(open).unwrap();
        }
    }
    fn release(g: &Arc<(Mutex<bool>, Condvar)>) {
        let (m, c) = &**g;
        *m.lock().unwrap() = true;
        c.notify_all();
    }

    #[test]
    fn drains_lanes_in_priority_order() {
        let s = Scheduler::new(1, 64);
        let order = Arc::new(Mutex::new(Vec::new()));
        let g = gate();
        let g2 = g.clone();
        let blocker = s.submit("block", Lane::Import, move |_| hold(&g2));
        std::thread::sleep(Duration::from_millis(30));
        let mut handles = Vec::new();
        for (k, lane) in [("m", Lane::Maintenance), ("i", Lane::Import), ("p", Lane::Prefetch), ("x", Lane::Interactive)] {
            let o = order.clone();
            handles.push(s.submit(k, lane, move |_| o.lock().unwrap().push(k)));
        }
        // The Interactive job runs on its reserved worker while the only general one is
        // still held by the import.
        assert!(handles[3].wait_timeout(Duration::from_secs(5)), "interactive waited behind an import");
        release(&g);
        blocker.wait();
        let last = s.submit("last", Lane::Maintenance, |_| {});
        last.wait();
        s.shutdown();
        assert_eq!(*order.lock().unwrap(), vec!["x", "p", "i", "m"]);
    }

    #[test]
    fn coalesces_same_key_and_cancels_queued() {
        let s = Scheduler::new(1, 64);
        let runs = Arc::new(AtomicUsize::new(0));
        let g = gate();
        let g2 = g.clone();
        let blocker = s.submit("block", Lane::Import, move |_| hold(&g2));
        std::thread::sleep(Duration::from_millis(30));
        let r1 = runs.clone();
        let h1 = s.submit("thumb:1", Lane::Import, move |_| {
            r1.fetch_add(1, Ordering::SeqCst);
        });
        let r2 = runs.clone();
        let h2 = s.submit("thumb:1", Lane::Import, move |_| {
            r2.fetch_add(1, Ordering::SeqCst);
        });
        let r3 = runs.clone();
        let h3 = s.submit("thumb:2", Lane::Import, move |_| {
            r3.fetch_add(1, Ordering::SeqCst);
        });
        s.cancel("thumb:2");
        assert!(h3.is_done(), "a cancelled queued job completes immediately");
        release(&g);
        blocker.wait();
        h1.wait();
        h2.wait();
        s.shutdown();
        assert_eq!(runs.load(Ordering::SeqCst), 1, "coalesced to one run; cancelled job never ran");
    }

    #[test]
    fn a_key_is_in_flight_from_submission_until_it_finishes() {
        let s = Scheduler::new(1, 64);
        let g = gate();
        let g2 = g.clone();
        assert!(!s.is_inflight("denoise:1"));
        let h = s.submit("denoise:1", Lane::Import, move |_| hold(&g2));
        assert!(s.is_inflight("denoise:1"));
        release(&g);
        h.wait();
        std::thread::sleep(Duration::from_millis(20));
        assert!(!s.is_inflight("denoise:1"));
        s.shutdown();
    }

    #[test]
    fn interactive_never_waits_behind_import() {
        let s = Scheduler::new(1, 64);
        let g = gate();
        let g2 = g.clone();
        // The only general worker is stuck on an import job.
        let g3 = g.clone();
        s.submit("import", Lane::Import, move |_| hold(&g3));
        std::thread::sleep(Duration::from_millis(30));
        let h = s.submit("thumb", Lane::Interactive, |_| {});
        assert!(h.wait_timeout(Duration::from_millis(200)), "interactive job ran on the reserved worker");
        release(&g2);
        s.shutdown();
    }

    #[test]
    fn a_running_job_is_listed_with_its_age() {
        let s = Scheduler::new(1, 16);
        let g = gate();
        let g2 = g.clone();
        let h = s.submit("denoise:7", Lane::Import, move |_| hold(&g2));
        std::thread::sleep(Duration::from_millis(80));
        let r = s.running();
        assert_eq!(r.len(), 1);
        assert_eq!((r[0].key.as_str(), r[0].lane), ("denoise:7", Lane::Import));
        assert!(r[0].for_ms >= 50, "{}", r[0].for_ms);
        release(&g);
        h.wait();
        assert!(s.running().is_empty());
        s.shutdown();
    }

    #[test]
    fn submit_blocks_at_capacity() {
        let s = Scheduler::new(1, 2);
        let g = gate();
        let g2 = g.clone();
        s.submit("block", Lane::Import, move |_| hold(&g2));
        std::thread::sleep(Duration::from_millis(30));
        s.submit("a", Lane::Import, |_| {});
        s.submit("b", Lane::Import, |_| {});
        let s2 = s.clone();
        let t = std::thread::spawn(move || {
            s2.submit("c", Lane::Import, |_| {});
        });
        std::thread::sleep(Duration::from_millis(50));
        assert!(!t.is_finished(), "third submit must block while the queue is full");
        release(&g);
        t.join().unwrap();
        assert!(s.stats().queued.iter().sum::<usize>() <= 2);
        s.shutdown();
    }

    #[test]
    fn an_interactive_submit_never_waits_for_a_full_queue() {
        let s = Scheduler::new(1, 2);
        let g = gate();
        let g2 = g.clone();
        // The only general worker is held and background work fills the queue.
        s.submit("transcode", Lane::Maintenance, move |_| hold(&g2));
        std::thread::sleep(Duration::from_millis(30));
        s.submit("a", Lane::Maintenance, |_| {});
        s.submit("b", Lane::Maintenance, |_| {});
        let s2 = s.clone();
        let t = std::thread::spawn(move || s2.submit("render", Lane::Interactive, |_| {}));
        std::thread::sleep(Duration::from_millis(100));
        let finished = t.is_finished();
        release(&g);
        let h = t.join().unwrap();
        assert!(finished, "an Interactive submit waited for queue space");
        assert!(h.wait_timeout(Duration::from_secs(5)));
        s.shutdown();
    }
}
