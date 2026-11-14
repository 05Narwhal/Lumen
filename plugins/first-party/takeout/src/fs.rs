//! The two file-system calls the scan needs, so it runs natively in tests and over the
//! host's `chosen` interface in the plugin. Paths are relative to the chosen folder, joined
//! with `/`; `""` is the chosen root.

/// Mirrors the WIT `entry` record.
#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub name: String,
    pub is_dir: bool,
    pub size: u64,
    pub modified: Option<i64>,
}

pub trait Fs {
    fn list(&self, dir: &str) -> Result<Vec<Entry>, String>;
    fn read(&self, path: &str) -> Result<Vec<u8>, String>;
    /// The key of a file that lives inside a zip (its bytes are served by key, not by path);
    /// `None` for a real file.
    fn virtual_key(&self, _path: &str) -> Option<String> {
        None
    }
}

/// `join("a", "b")` is `a/b`; an empty `dir` is the root.
pub fn join(dir: &str, name: &str) -> String {
    if dir.is_empty() {
        name.to_string()
    } else {
        format!("{dir}/{name}")
    }
}

/// How many reads and lists pass between two progress reports.
pub const PROGRESS_EVERY: u64 = 500;

/// Counts the reads and lists of an [`Fs`] and calls `report(count)` every [`PROGRESS_EVERY`]
/// of them, so the host can show a count and stop a long scan (a failed report fails the call).
pub struct Ticking<'a> {
    inner: &'a dyn Fs,
    calls: std::cell::Cell<u64>,
    report: Box<dyn Fn(u64) -> Result<(), String> + 'a>,
}

impl<'a> Ticking<'a> {
    pub fn new(inner: &'a dyn Fs, report: Box<dyn Fn(u64) -> Result<(), String> + 'a>) -> Ticking<'a> {
        Ticking { inner, calls: std::cell::Cell::new(0), report }
    }

    fn tick(&self) -> Result<(), String> {
        let n = self.calls.get() + 1;
        self.calls.set(n);
        if n.is_multiple_of(PROGRESS_EVERY) {
            (self.report)(n)?;
        }
        Ok(())
    }
}

impl Fs for Ticking<'_> {
    fn list(&self, dir: &str) -> Result<Vec<Entry>, String> {
        self.tick()?;
        self.inner.list(dir)
    }

    fn read(&self, path: &str) -> Result<Vec<u8>, String> {
        self.tick()?;
        self.inner.read(path)
    }

    fn virtual_key(&self, path: &str) -> Option<String> {
        self.inner.virtual_key(path)
    }
}
