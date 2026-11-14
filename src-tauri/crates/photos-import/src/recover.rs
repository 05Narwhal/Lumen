//! Settling imports cut short by a crash or a closed app (round 3). An import journals each
//! file before moving it into `originals/` and clears the entry with the file's row, so an
//! entry left over names a file Lumen placed. When the library next opens:
//!
//! * the row exists → only the entry goes;
//! * no row, and the file there is byte for byte what was journalled → it is Lumen's own
//!   uncatalogued copy: removed, so importing again brings it in cleanly (the source was
//!   never marked seen for it);
//! * anything else (no file, or a file that is not the one placed) → only the entry goes.
//!   Nothing Lumen cannot prove it put there is ever deleted.

use photos_core::{assets, import_journal, Library};

use crate::ingest::content_hash;
use crate::ImportError;

#[derive(Debug, Default, Clone, PartialEq)]
pub struct RecoverReport {
    /// Uncatalogued copies removed.
    pub removed: usize,
    /// Journal entries settled in all.
    pub settled: usize,
}

pub fn recover(lib: &Library) -> Result<RecoverReport, ImportError> {
    let entries = lib.db.with_conn(import_journal::list)?;
    let mut report = RecoverReport::default();
    for e in entries {
        let catalogued = lib.db.with_conn(|c| assets::find_by_hash(c, &e.content_hash))?.is_some();
        if !catalogued {
            let path = lib.originals_dir().join(&e.relative_path);
            if path.is_file() && content_hash(&path).is_ok_and(|h| h == e.content_hash) {
                std::fs::remove_file(&path)?;
                report.removed += 1;
                tracing::info!(file = %e.relative_path, "removed a copy an interrupted import placed but never catalogued");
            }
        }
        lib.db.with_conn(|c| import_journal::clear(c, &[e.id]))?;
        report.settled += 1;
    }
    Ok(report)
}
