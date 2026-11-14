//! A running Lumen and Setup (spec *Running Lumen*). Setup never kills Lumen: it starts
//! `<exe> --quit`, which Lumen's single-instance check hands to the open window, and then waits
//! for that window's process to end, with no time limit. While Lumen asks about a running import it
//! writes `%TEMP%\lumen-quit-held-<pid>` (`window::quit_held_marker` in the app), and Setup says so.
//! *Cancel* stops the wait and nothing else.
use serde::Serialize;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use ts_rs::TS;

/// The `setup://lumen` event.
pub const LUMEN_EVENT: &str = "setup://lumen";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Proc {
    pub pid: u32,
    pub exe: PathBuf,
    pub args: Vec<String>,
}

/// What Setup asks of the machine, so tests run on a fake process list.
pub trait System {
    fn processes(&self) -> Vec<Proc>;
    /// Start `<exe> --quit` and do not wait for it.
    fn ask_to_quit(&self, exe: &Path) -> io::Result<()>;
}

/// What the Progress and Remove screens say while Lumen closes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "kebab-case")]
#[ts(export)]
pub enum Closing {
    /// "Closing Lumen…"
    Closing,
    /// "Lumen is asking about the import that's running. Answer there and Setup continues."
    AskingImport,
    Closed,
}

#[derive(Debug, PartialEq, Eq)]
pub enum CloseError {
    /// `<exe> --quit` could not be started.
    Start(String),
    /// The person pressed Cancel.
    Cancelled,
}

impl std::fmt::Display for CloseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CloseError::Start(e) => write!(f, "Setup could not ask Lumen to close: {e}. Close Lumen yourself, then try again."),
            CloseError::Cancelled => write!(f, "Setup was cancelled. Nothing was changed."),
        }
    }
}

/// The same file, as Windows compares paths (separators and case don't matter).
pub fn same_exe(a: &Path, b: &Path) -> bool {
    let norm = |p: &Path| {
        let s = p.to_string_lossy().replace('/', "\\");
        let s = s.strip_prefix(r"\\?\").unwrap_or(&s).to_string();
        if cfg!(windows) {
            s.to_lowercase()
        } else {
            s
        }
    };
    norm(a) == norm(b)
}

/// The Lumen windows running from `exe`: not Setup itself and not an `--mcp` bridge (no window;
/// its AI client starts it again, and NSIS's Restart Manager step ends it).
pub fn running(procs: &[Proc], exe: &Path, me: u32) -> Vec<u32> {
    procs
        .iter()
        .filter(|p| p.pid != me && same_exe(&p.exe, exe) && !p.args.iter().any(|a| a == "--mcp"))
        .map(|p| p.pid)
        .collect()
}

/// The marker Lumen writes while its `--quit` close waits on the import question.
pub fn quit_held_marker(dir: &Path, pid: u32) -> PathBuf {
    dir.join(format!("lumen-quit-held-{pid}"))
}

/// Close the Lumen running from `exe`, if any. `Ok(true)` when one was open and has now ended,
/// `Ok(false)` when none was. Waits as long as Lumen takes; only `cancel` ends the wait early.
pub fn close_lumen(
    sys: &dyn System,
    exe: &Path,
    marker_dir: &Path,
    cancel: &AtomicBool,
    poll: Duration,
    report: &mut dyn FnMut(Closing),
) -> Result<bool, CloseError> {
    let me = std::process::id();
    let open = running(&sys.processes(), exe, me);
    if open.is_empty() {
        return Ok(false);
    }
    sys.ask_to_quit(exe).map_err(|e| CloseError::Start(e.to_string()))?;
    let mut last = None;
    loop {
        if cancel.load(Ordering::SeqCst) {
            return Err(CloseError::Cancelled);
        }
        // Only the windows that were open when Setup asked: a Lumen started since is not waited on.
        let still: Vec<u32> = running(&sys.processes(), exe, me).into_iter().filter(|p| open.contains(p)).collect();
        let now = if still.is_empty() {
            Closing::Closed
        } else if still.iter().any(|&pid| quit_held_marker(marker_dir, pid).exists()) {
            Closing::AskingImport
        } else {
            Closing::Closing
        };
        if last != Some(now) {
            report(now);
            last = Some(now);
        }
        if now == Closing::Closed {
            return Ok(true);
        }
        std::thread::sleep(poll);
    }
}

/// Start Lumen again after an update or repair (it was open before).
pub fn reopen(exe: &Path) -> io::Result<()> {
    std::process::Command::new(exe).spawn().map(|_| ())
}

/// The real machine: the process list from `sysinfo` (path and arguments), `--quit` by spawning.
pub struct RealSystem;

impl System for RealSystem {
    fn processes(&self) -> Vec<Proc> {
        use sysinfo::{ProcessRefreshKind, RefreshKind, UpdateKind};
        let sys = sysinfo::System::new_with_specifics(
            RefreshKind::nothing().with_processes(ProcessRefreshKind::nothing().with_exe(UpdateKind::Always).with_cmd(UpdateKind::Always)),
        );
        sys.processes()
            .values()
            .map(|p| Proc {
                pid: p.pid().as_u32(),
                exe: p.exe().map(Path::to_path_buf).unwrap_or_default(),
                args: p.cmd().iter().map(|a| a.to_string_lossy().into_owned()).collect(),
            })
            .collect()
    }
    fn ask_to_quit(&self, exe: &Path) -> io::Result<()> {
        std::process::Command::new(exe).arg("--quit").spawn().map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::{Cell, RefCell};

    const EXE: &str = r"C:\Users\A\AppData\Local\Lumen Test\lumen-test.exe";

    fn proc(pid: u32, exe: &str, args: &[&str]) -> Proc {
        Proc { pid, exe: PathBuf::from(exe), args: args.iter().map(|s| s.to_string()).collect() }
    }

    /// Plays Lumen: after `--quit`, it ends on the `ends_after`-th look at the process list.
    /// `on_look` runs before each look, to write or remove the marker.
    struct FakeLumen<'a> {
        procs: RefCell<Vec<Proc>>,
        asked: RefCell<Vec<PathBuf>>,
        looks: Cell<u32>,
        ends_after: Option<u32>,
        fail_start: bool,
        on_look: Box<dyn Fn(u32) + 'a>,
    }

    impl<'a> FakeLumen<'a> {
        fn new(procs: Vec<Proc>, ends_after: Option<u32>) -> Self {
            Self { procs: RefCell::new(procs), asked: RefCell::default(), looks: Cell::new(0), ends_after, fail_start: false, on_look: Box::new(|_| {}) }
        }
    }

    impl System for FakeLumen<'_> {
        fn processes(&self) -> Vec<Proc> {
            if !self.asked.borrow().is_empty() {
                let n = self.looks.get() + 1;
                self.looks.set(n);
                (self.on_look)(n);
                if self.ends_after.is_some_and(|e| n >= e) {
                    self.procs.borrow_mut().retain(|p| p.pid != 7);
                }
            }
            self.procs.borrow().clone()
        }
        fn ask_to_quit(&self, exe: &Path) -> io::Result<()> {
            if self.fail_start {
                return Err(io::Error::new(io::ErrorKind::NotFound, "no such file"));
            }
            self.asked.borrow_mut().push(exe.to_path_buf());
            Ok(())
        }
    }

    fn close(sys: &dyn System, dir: &Path, cancel: &AtomicBool) -> (Result<bool, CloseError>, Vec<Closing>) {
        let mut seen = Vec::new();
        let r = close_lumen(sys, Path::new(EXE), dir, cancel, Duration::from_millis(1), &mut |c| seen.push(c));
        (r, seen)
    }

    #[test]
    fn running_finds_only_lumen_windows_from_that_exe() {
        let procs = vec![
            proc(7, EXE, &[EXE]),
            proc(8, &EXE.to_uppercase().replace('\\', "/"), &[]),
            proc(9, EXE, &[EXE, "--mcp"]),
            proc(10, r"C:\Other\lumen-test.exe", &[]),
            proc(11, EXE, &[]),
        ];
        let mut found = running(&procs, Path::new(EXE), 11);
        found.sort();
        assert_eq!(found, vec![7, 8], "same file in any case, no --mcp bridge, not Setup itself, not another folder");
    }

    #[test]
    fn no_lumen_running_asks_nothing() {
        let t = tempfile::tempdir().unwrap();
        let sys = FakeLumen::new(vec![proc(3, r"C:\x\other.exe", &[])], None);
        let (r, seen) = close(&sys, t.path(), &AtomicBool::new(false));
        assert_eq!(r, Ok(false));
        assert!(sys.asked.borrow().is_empty() && seen.is_empty());
    }

    #[test]
    fn quit_is_asked_of_the_same_exe_and_waits_until_it_ends() {
        let t = tempfile::tempdir().unwrap();
        let sys = FakeLumen::new(vec![proc(7, EXE, &[])], Some(5));
        let (r, seen) = close(&sys, t.path(), &AtomicBool::new(false));
        assert_eq!(r, Ok(true));
        assert_eq!(*sys.asked.borrow(), vec![PathBuf::from(EXE)]);
        assert_eq!(seen, vec![Closing::Closing, Closing::Closed], "each state once, in order");
        assert!(sys.looks.get() >= 5);
    }

    #[test]
    fn quit_waits_on_the_import_question() {
        // Lumen holds the close on its import question for a long while (no time limit), then the
        // person answers and it ends.
        let t = tempfile::tempdir().unwrap();
        let dir = t.path().to_path_buf();
        let marker = quit_held_marker(&dir, 7);
        let mut sys = FakeLumen::new(vec![proc(7, EXE, &[])], Some(400));
        sys.on_look = Box::new(move |n| match n {
            3 => std::fs::write(&marker, b"import").unwrap(),
            399 => std::fs::remove_file(&marker).unwrap(),
            _ => {}
        });
        let (r, seen) = close(&sys, &dir, &AtomicBool::new(false));
        assert_eq!(r, Ok(true));
        assert_eq!(seen, vec![Closing::Closing, Closing::AskingImport, Closing::Closing, Closing::Closed]);
    }

    #[test]
    fn cancel_ends_the_wait_and_changes_nothing() {
        let t = tempfile::tempdir().unwrap();
        let cancel = AtomicBool::new(false);
        let mut sys = FakeLumen::new(vec![proc(7, EXE, &[])], None);
        let c = &cancel;
        sys.on_look = Box::new(move |n| {
            if n == 20 {
                c.store(true, Ordering::SeqCst);
            }
        });
        let (r, _) = close(&sys, t.path(), &cancel);
        assert_eq!(r, Err(CloseError::Cancelled));
        assert_eq!(sys.procs.borrow().len(), 1, "Lumen keeps running; Setup killed nothing");
        assert_eq!(sys.asked.borrow().len(), 1, "asked once, never again");
    }

    #[test]
    fn a_quit_that_cannot_start_is_a_sentence() {
        let t = tempfile::tempdir().unwrap();
        let mut sys = FakeLumen::new(vec![proc(7, EXE, &[])], None);
        sys.fail_start = true;
        let (r, _) = close(&sys, t.path(), &AtomicBool::new(false));
        let e = r.unwrap_err();
        assert!(matches!(e, CloseError::Start(_)));
        assert!(e.to_string().contains("Close Lumen yourself"));
    }

    #[test]
    fn a_marker_of_another_pid_is_not_this_lumen() {
        let t = tempfile::tempdir().unwrap();
        std::fs::write(quit_held_marker(t.path(), 99), b"import").unwrap();
        let sys = FakeLumen::new(vec![proc(7, EXE, &[])], Some(3));
        let (_, seen) = close(&sys, t.path(), &AtomicBool::new(false));
        assert!(!seen.contains(&Closing::AskingImport));
    }

    #[test]
    fn the_marker_name_matches_lumens() {
        // window::quit_held_marker in src-tauri writes exactly this name.
        assert_eq!(quit_held_marker(Path::new("T"), 42), Path::new("T").join("lumen-quit-held-42"));
    }
}
