//! Reading Google Takeout zips without loading them: the central directory is read through a
//! ranged `read_at`, so a 50 GB archive costs a few kilobytes of index.

mod cd;

pub use cd::{read_index, read_index_ticking};

/// One file in a zip. `skip` says why the entry cannot be imported (encrypted, or a method other
/// than stored or deflate); such entries are listed so the caller can report them.
#[derive(Debug, Clone, PartialEq)]
pub struct ZipEntry {
    pub name: String,
    pub method: u16,
    pub compressed: u64,
    pub size: u64,
    pub local_offset: u64,
    pub skip: Option<String>,
    /// Unix seconds: the extended-timestamp extra when present, else the DOS date and time read as
    /// UTC (zips do not say which zone). `None` for a zeroed or invalid DOS date.
    pub modified: Option<i64>,
}

mod fs;
mod stream;

pub use fs::{OverlayFs, ZipFs};
pub use stream::{make_key, ReadAt, Streams, ZipSet};
