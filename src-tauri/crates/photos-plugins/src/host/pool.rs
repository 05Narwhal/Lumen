//! The `lumen-plugins` thread pool (spec §2): every guest call runs here at below-normal
//! priority, never on the caller's thread (a command's or a job's). Also the idle limit after
//! which a shared instance is dropped.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;
use std::time::Duration;

static IDLE_SECS: AtomicU64 = AtomicU64::new(60);

/// Wasm runs on the host thread's stack (wasmtime's default limit is 512 KiB of it).
const STACK: usize = 4 << 20;

fn pool() -> &'static rayon::ThreadPool {
    static POOL: OnceLock<rayon::ThreadPool> = OnceLock::new();
    POOL.get_or_init(|| {
        let n = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(2).clamp(2, 4);
        rayon::ThreadPoolBuilder::new()
            .num_threads(n)
            .thread_name(|i| format!("lumen-plugins-{i}"))
            .stack_size(STACK)
            .start_handler(|_| background_priority())
            .build()
            .expect("the lumen-plugins pool starts")
    })
}

/// Runs `f` on the pool and waits for it.
pub(crate) fn run<R: Send>(f: impl FnOnce() -> R + Send) -> R {
    pool().install(f)
}

/// How long a shared instance may sit unused before it is dropped.
pub(crate) fn idle() -> Duration {
    Duration::from_secs(IDLE_SECS.load(Ordering::Relaxed))
}

/// Shortens the idle limit so the drop test finishes quickly.
#[doc(hidden)]
pub fn set_idle_secs_for_tests(secs: u64) {
    IDLE_SECS.store(secs, Ordering::Relaxed);
}

/// Lowers the calling thread's priority, as `photos-jobs` does for its workers. Best effort.
fn background_priority() {
    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::System::Threading::{GetCurrentThread, SetThreadPriority, THREAD_PRIORITY_BELOW_NORMAL};
        SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_BELOW_NORMAL);
    }
    #[cfg(target_os = "linux")]
    unsafe {
        libc::setpriority(libc::PRIO_PROCESS, libc::gettid() as libc::id_t, 10);
    }
}
