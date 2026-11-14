//! Child processes that end with Lumen (Round 15). On Windows a child outlives its parent, so
//! an FFmpeg transcode kept running (and writing) after Lumen closed or was ended. Each
//! long-running child is put in one kill-on-close Job Object whose handle is never closed:
//! when Lumen's process ends, however it ends, the OS closes the handle and ends the children.
//!
//! A job per child, not Lumen itself in a job: a Lumen relaunched by *Restart* must not die
//! with the old one.

/// Puts `child` in the kill-on-close job. Best effort: a failure is logged, never an error.
/// A no-op off Windows, where FFmpeg loops stop on cancel.
pub fn adopt(child: &std::process::Child) {
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::System::JobObjects::AssignProcessToJobObject;
        let Some(job) = job() else { return };
        // SAFETY: both handles are valid for the call: the job's is never closed, and the
        // child's lives as long as `child`.
        if unsafe { AssignProcessToJobObject(job as _, child.as_raw_handle() as _) } == 0 {
            tracing::warn!("child_jobs: could not adopt pid {}: {}", child.id(), std::io::Error::last_os_error());
        }
    }
    #[cfg(not(windows))]
    let _ = child;
}

/// Ends process `pid` and waits up to `wait` for it to be gone; whether it is. Windows only
/// (the watchdog ends a hung WebView2 renderer this way); `false` elsewhere.
pub fn end_process(pid: u32, wait: std::time::Duration) -> bool {
    #[cfg(windows)]
    {
        use windows_sys::Win32::Foundation::{CloseHandle, WAIT_OBJECT_0};
        use windows_sys::Win32::System::Threading::{
            OpenProcess, TerminateProcess, WaitForSingleObject, PROCESS_SYNCHRONIZE, PROCESS_TERMINATE,
        };
        // SAFETY: the handle is checked before use and closed once.
        unsafe {
            let h = OpenProcess(PROCESS_TERMINATE | PROCESS_SYNCHRONIZE, 0, pid);
            if h.is_null() {
                return false;
            }
            TerminateProcess(h, 1);
            let gone = WaitForSingleObject(h, wait.as_millis().min(u32::MAX as u128) as u32) == WAIT_OBJECT_0;
            CloseHandle(h);
            gone
        }
    }
    #[cfg(not(windows))]
    {
        let _ = (pid, wait);
        false
    }
}

/// Waits up to `wait` for process `pid` to end; whether it is gone (a pid that cannot be
/// opened is gone). A Lumen started by *Restart* waits for the old one this way. Windows
/// only; `true` elsewhere.
pub fn wait_for_exit(pid: u32, wait: std::time::Duration) -> bool {
    #[cfg(windows)]
    {
        use windows_sys::Win32::Foundation::{CloseHandle, WAIT_OBJECT_0};
        use windows_sys::Win32::System::Threading::{OpenProcess, WaitForSingleObject, PROCESS_SYNCHRONIZE};
        // SAFETY: the handle is checked before use and closed once.
        unsafe {
            let h = OpenProcess(PROCESS_SYNCHRONIZE, 0, pid);
            if h.is_null() {
                return true;
            }
            let gone = WaitForSingleObject(h, wait.as_millis().min(u32::MAX as u128) as u32) == WAIT_OBJECT_0;
            CloseHandle(h);
            gone
        }
    }
    #[cfg(not(windows))]
    {
        let _ = (pid, wait);
        true
    }
}

/// Ends this process at once, without running exit handlers or waiting for threads (which
/// may be stuck in a driver): the exit guard's last step after Close.
pub fn end_this_process() -> ! {
    #[cfg(windows)]
    {
        use windows_sys::Win32::System::Threading::{GetCurrentProcess, TerminateProcess};
        // SAFETY: the pseudo-handle of the current process is always valid.
        unsafe {
            TerminateProcess(GetCurrentProcess(), 0);
        }
    }
    std::process::exit(0)
}

/// The kill-on-close job, made once (as an `isize` handle, so it can live in a static).
#[cfg(windows)]
fn job() -> Option<isize> {
    use std::sync::OnceLock;
    use windows_sys::Win32::System::JobObjects::{
        CreateJobObjectW, JobObjectExtendedLimitInformation, SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
        JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    };
    static JOB: OnceLock<Option<isize>> = OnceLock::new();
    *JOB.get_or_init(|| {
        // SAFETY: null attributes and name make an unnamed job with default security; the
        // limit struct is zeroed and then given the one flag, as the API expects.
        unsafe {
            let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            if job.is_null() {
                tracing::warn!("child_jobs: no job object: {}", std::io::Error::last_os_error());
                return None;
            }
            let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
            info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            let ok = SetInformationJobObject(
                job,
                JobObjectExtendedLimitInformation,
                &info as *const _ as *const _,
                std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            );
            if ok == 0 {
                tracing::warn!("child_jobs: kill-on-close refused: {}", std::io::Error::last_os_error());
                return None;
            }
            Some(job as isize)
        }
    })
}

#[cfg(all(test, windows))]
mod tests {
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};

    fn alive(pid: u32) -> bool {
        let out = Command::new("tasklist").args(["/FI", &format!("PID eq {pid}"), "/NH"]).output().unwrap();
        String::from_utf8_lossy(&out.stdout).contains(&pid.to_string())
    }

    /// Run by `a_child_dies_with_its_parent` in a subprocess: spawns a long child, adopts it,
    /// prints its pid and exits without waiting for it.
    #[test]
    #[ignore = "helper: run only as a subprocess of a_child_dies_with_its_parent"]
    #[allow(clippy::zombie_processes, reason = "the point: the parent exits without waiting")]
    fn helper() {
        let child = Command::new("ping").args(["-n", "30", "127.0.0.1"]).stdout(Stdio::null()).spawn().unwrap();
        super::adopt(&child);
        println!("CHILD_PID={}", child.id());
    }

    #[test]
    fn end_process_ends_it_and_says_so() {
        let mut child = Command::new("ping").args(["-n", "30", "127.0.0.1"]).stdout(Stdio::null()).spawn().unwrap();
        assert!(super::end_process(child.id(), Duration::from_secs(2)));
        assert!(child.try_wait().unwrap().is_some(), "still running");
        assert!(!super::end_process(u32::MAX - 1, Duration::from_millis(10)), "no such process");
    }

    #[test]
    fn wait_for_exit_waits_until_the_process_is_gone() {
        let mut child = Command::new("ping").args(["-n", "30", "127.0.0.1"]).stdout(Stdio::null()).spawn().unwrap();
        assert!(!super::wait_for_exit(child.id(), Duration::from_millis(200)), "still running");
        child.kill().unwrap();
        assert!(super::wait_for_exit(child.id(), Duration::from_secs(2)));
        child.wait().unwrap();
        assert!(super::wait_for_exit(u32::MAX - 1, Duration::from_millis(10)), "no such process");
    }

    #[test]
    fn a_child_dies_with_its_parent() {
        let out = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "child_jobs::tests::helper", "--ignored", "--nocapture"])
            .output()
            .unwrap();
        let text = String::from_utf8_lossy(&out.stdout);
        let pid: u32 = text.split("CHILD_PID=").nth(1).and_then(|s| s.split_whitespace().next()).and_then(|s| s.parse().ok()).unwrap_or_else(|| panic!("no pid in {text}"));
        let until = Instant::now() + Duration::from_secs(2);
        while alive(pid) && Instant::now() < until {
            std::thread::sleep(Duration::from_millis(100));
        }
        assert!(!alive(pid), "the child outlived its parent");
    }
}
