//! Background threads yield to the UI. The webview renders in its own processes at normal
//! priority; decoding, face scans and embeddings at normal priority beside it starve it of
//! cores on a busy library, and scrolling stutters. Below normal, they take what is idle.

/// Lowers the calling thread's scheduling priority. Best effort: a refusal is ignored.
pub fn background_priority() {
    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::System::Threading::{GetCurrentThread, SetThreadPriority, THREAD_PRIORITY_BELOW_NORMAL};
        SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_BELOW_NORMAL);
    }
    // Linux schedules threads as tasks, so a nice value on the thread id is per thread.
    #[cfg(target_os = "linux")]
    unsafe {
        libc::setpriority(libc::PRIO_PROCESS, libc::gettid() as libc::id_t, 10);
    }
    // macOS: a thread's QoS would be the tool; left at the default for now.
}
