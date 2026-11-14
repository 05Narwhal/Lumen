//! Logs: to the console as before, and to `<app data>/logs/lumen-<time>.log`. A file is started
//! at each launch and whenever the current one passes `MAX_BYTES`; only the newest `KEEP` stay.

use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use tracing_subscriber::fmt::MakeWriter;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

/// Log files kept in the folder.
pub const KEEP: usize = 20;
/// A file is closed and a new one started past this size.
pub const MAX_BYTES: u64 = 10 * 1024 * 1024;

struct Current {
    dir: PathBuf,
    file: File,
    written: u64,
}

impl Current {
    fn open(dir: &Path) -> io::Result<Self> {
        fs::create_dir_all(dir)?;
        let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S%.3f");
        let file = File::create(dir.join(format!("lumen-{stamp}.log")))?;
        prune(dir, KEEP);
        Ok(Self { dir: dir.to_path_buf(), file, written: 0 })
    }
}

/// Deletes all but the newest `keep` `lumen-*.log` files (their names sort by time).
pub fn prune(dir: &Path, keep: usize) {
    let Ok(entries) = fs::read_dir(dir) else { return };
    let mut logs: Vec<PathBuf> = entries
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("lumen-") && n.ends_with(".log"))
        })
        .collect();
    logs.sort();
    let excess = logs.len().saturating_sub(keep);
    for old in &logs[..excess] {
        let _ = fs::remove_file(old);
    }
}

/// The file writer, shared by every event; it rolls over by size.
#[derive(Clone)]
pub struct RollingFile(Arc<Mutex<Current>>);

impl RollingFile {
    pub fn new(dir: &Path) -> io::Result<Self> {
        Ok(Self(Arc::new(Mutex::new(Current::open(dir)?))))
    }
}

impl Write for RollingFile {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let mut cur = self.0.lock().unwrap_or_else(|p| p.into_inner());
        if cur.written >= MAX_BYTES {
            let dir = cur.dir.clone();
            if let Ok(next) = Current::open(&dir) {
                *cur = next;
            }
        }
        let n = cur.file.write(buf)?;
        cur.written += n as u64;
        Ok(n)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.0.lock().unwrap_or_else(|p| p.into_inner()).file.flush()
    }
}

impl<'a> MakeWriter<'a> for RollingFile {
    type Writer = RollingFile;
    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

/// Starts logging. Without a writable folder it logs to the console only.
pub fn init() {
    let filter = || {
        // ONNX Runtime logs every graph rewrite at INFO: hundreds of lines per session. Its
        // own log (`ort::logging`) also warns once per weight of a model exported with its
        // initializers listed as graph inputs (the face embedder): ~130 lines, no action.
        tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info,ort=warn,ort::logging=error".into())
    };
    let dir = photos_core::paths::AppPaths::resolve().ok().map(|p| p.app_data.join("logs"));
    let file = dir.as_deref().and_then(|d| RollingFile::new(d).ok());
    let registry = tracing_subscriber::registry().with(filter()).with(tracing_subscriber::fmt::layer());
    match file {
        Some(file) => {
            registry.with(tracing_subscriber::fmt::layer().with_ansi(false).with_writer(file)).init();
            let default_hook = std::panic::take_hook();
            std::panic::set_hook(Box::new(move |info| {
                tracing::error!("panic: {info}");
                default_hook(info);
            }));
            tracing::info!(version = env!("CARGO_PKG_VERSION"), "Lumen started");
        }
        None => registry.init(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_only_the_newest_logs() {
        let dir = tempfile::tempdir().unwrap();
        for i in 0..25 {
            fs::write(dir.path().join(format!("lumen-2026010{}-{i:02}.log", i / 10)), "x").unwrap();
        }
        fs::write(dir.path().join("other.txt"), "x").unwrap();
        prune(dir.path(), KEEP);
        let mut left: Vec<String> = fs::read_dir(dir.path()).unwrap().map(|e| e.unwrap().file_name().into_string().unwrap()).collect();
        left.sort();
        assert_eq!(left.len(), KEEP + 1);
        assert!(left.contains(&"other.txt".to_string()));
        assert!(!left.contains(&"lumen-20260100-04.log".to_string()));
        assert!(left.contains(&"lumen-20260100-05.log".to_string()));
    }

    #[test]
    fn writes_and_rolls_over() {
        let dir = tempfile::tempdir().unwrap();
        let mut w = RollingFile::new(dir.path()).unwrap();
        w.write_all(b"hello\n").unwrap();
        w.0.lock().unwrap().written = MAX_BYTES;
        std::thread::sleep(std::time::Duration::from_millis(5));
        w.write_all(b"again\n").unwrap();
        w.flush().unwrap();
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 2);
    }
}
