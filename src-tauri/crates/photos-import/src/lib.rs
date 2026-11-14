//! Lumen import: sources (folders now; volumes and iPhones later) and the ingest pipeline.

// `ImportError` wraps `CoreError` (which carries `rusqlite::Error`); per-file results are
// short-lived, so the extra bytes on the error path are not worth a Box.
#![allow(clippy::result_large_err)]

pub mod apple;
pub mod budget;
pub mod check;
pub mod eta;
pub mod folder_albums;
pub mod ingest;
pub mod preview;
pub mod recover;
pub mod repair;
pub mod sidecar;
pub mod iphone;
pub mod lightroom;
pub mod macdisk;
pub mod source;
pub mod verify;
pub mod volume;

pub use ingest::{content_hash, run_batch, run_import, ImportOptions, ImportProgress, ImportSummary, Ingested};
pub use budget::{fit_newest_first, FreeSpace, IMPORT_FREE_BUFFER};
pub use recover::{recover, RecoverReport};
pub use source::FilteredSource;
pub use folder_albums::{FolderAlbumsSource, FolderTree};
pub use iphone::{IphoneError, IphoneInfo, IphoneSource};
pub use volume::VolumeSource;
pub use source::{FilesSource, FolderSource, ImportSource, SourceItem};

#[derive(Debug, thiserror::Error)]
pub enum ImportError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("catalogue error: {0}")]
    Core(#[from] photos_core::CoreError),
    #[error("media error: {0}")]
    Media(#[from] photos_media::MediaError),
    #[error("source error: {0}")]
    Source(String),
    /// The staged copy did not read back as the source's bytes (`verify`).
    #[error("{0}")]
    Verify(String),
}
