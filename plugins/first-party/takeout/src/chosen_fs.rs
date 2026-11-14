//! [`Fs`] over the host's `chosen` interface (wasm32 only). Calls `progress` every 500 reads and
//! lists, so the host can show a count and stop the scan (a failed `progress` fails the next call).

use crate::fs::{Entry, Fs, Ticking};
use lumen_plugin::chosen;
use std::cell::Cell;

const EVERY: u64 = crate::fs::PROGRESS_EVERY;

#[derive(Default)]
pub struct ChosenFs {
    calls: Cell<u64>,
}

impl ChosenFs {
    fn tick(&self) -> Result<(), String> {
        let n = self.calls.get() + 1;
        self.calls.set(n);
        if n.is_multiple_of(EVERY) {
            chosen::progress(n, None)?;
        }
        Ok(())
    }
}

impl Fs for ChosenFs {
    fn list(&self, dir: &str) -> Result<Vec<Entry>, String> {
        self.tick()?;
        Ok(chosen::list(dir)?
            .into_iter()
            .map(|e| Entry { name: e.name, is_dir: e.is_dir, size: e.size, modified: e.modified })
            .collect())
    }

    fn read(&self, path: &str) -> Result<Vec<u8>, String> {
        self.tick()?;
        chosen::read_all(path)
    }
}

/// The zips' tree reporting progress to the host like [`ChosenFs`] does.
pub fn ticking(inner: &dyn Fs) -> Ticking<'_> {
    Ticking::new(inner, Box::new(|n| chosen::progress(n, None)))
}
