//! The render pool for what the user is waiting on (Round 5A): Develop's preview and zoomed
//! view, the quick editor's preview, and every Interactive-lane job (on-screen thumbnails,
//! the viewer's full size). Their pixel loops are rayon's, and on the global pool they queued
//! behind an import's or a face scan's — the app lagged whenever background work ran. This
//! pool runs at normal priority and is idle otherwise; the global pool stays below normal.

use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::LazyLock;
use std::time::{Duration, Instant};

static POOL: LazyLock<rayon::ThreadPool> = LazyLock::new(|| {
    let cores = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(2);
    rayon::ThreadPoolBuilder::new()
        .num_threads(cores)
        .thread_name(|i| format!("lumen-render-{i}"))
        .build()
        .expect("render pool")
});

/// Starts the pool's threads now, so the first render does not pay for them.
pub fn warm() {
    LazyLock::force(&POOL);
}

static EPOCH: LazyLock<Instant> = LazyLock::new(Instant::now);
/// Renders the user is waiting on right now, and when the last one began or ended (ms since
/// `EPOCH`): what background scans watch to step aside (Round 8A).
static RUNNING: AtomicUsize = AtomicUsize::new(0);
static LAST: AtomicU64 = AtomicU64::new(0);

/// Starts at 1, so a render in `EPOCH`'s first millisecond is never read as `LAST`'s 0 (none yet).
fn now_ms() -> u64 {
    EPOCH.elapsed().as_millis() as u64 + 1
}

/// Runs `f` with its parallel work on the render pool, not the global one.
pub fn interactive<R: Send>(f: impl FnOnce() -> R + Send) -> R {
    struct Running;
    impl Drop for Running {
        fn drop(&mut self) {
            RUNNING.fetch_sub(1, Ordering::Relaxed);
            LAST.store(now_ms(), Ordering::Relaxed);
        }
    }
    RUNNING.fetch_add(1, Ordering::Relaxed);
    LAST.store(now_ms(), Ordering::Relaxed);
    let _running = Running;
    POOL.install(f)
}

/// Whether the user is waiting on a render, or was within `within`: a background scan that
/// can wait pauses until this is false.
pub fn user_busy(within: Duration) -> bool {
    LazyLock::force(&EPOCH);
    RUNNING.load(Ordering::Relaxed) > 0 || (LAST.load(Ordering::Relaxed) > 0 && now_ms().saturating_sub(LAST.load(Ordering::Relaxed)) <= within.as_millis() as u64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rayon::prelude::*;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::time::Duration;

    #[test]
    fn a_render_marks_the_user_busy_for_a_while() {
        interactive(|| assert!(user_busy(Duration::ZERO), "busy while it runs"));
        assert!(user_busy(Duration::from_secs(60)), "and just after");
    }

    #[test]
    fn a_render_never_waits_for_background_work() {
        // Every thread of the global pool is busy until released.
        let hold = Arc::new(AtomicBool::new(true));
        let started = Arc::new(AtomicUsize::new(0));
        let n = rayon::current_num_threads();
        for _ in 0..n {
            let (hold, started) = (hold.clone(), started.clone());
            rayon::spawn(move || {
                started.fetch_add(1, Ordering::SeqCst);
                while hold.load(Ordering::SeqCst) {
                    std::thread::sleep(Duration::from_millis(1));
                }
            });
        }
        while started.load(Ordering::SeqCst) < n {
            std::thread::sleep(Duration::from_millis(1));
        }

        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let sum: u64 = interactive(|| (0..100_000u64).into_par_iter().sum());
            let _ = tx.send(sum);
        });
        let got = rx.recv_timeout(Duration::from_secs(5));
        hold.store(false, Ordering::SeqCst);
        assert_eq!(got, Ok(4_999_950_000), "the render ran while the global pool was full");
    }
}
