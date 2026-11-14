//! Lightroom Classic catalogs (`.lrcat`, Round 11), read-only, through a temp copy: their
//! originals with ratings, flags, colour labels, keywords, collections, captions and develop
//! settings.

pub mod catalog;
pub mod find;

pub use catalog::{CatalogSummary, LightroomCatalogSource};
pub use find::{find_lightroom_catalogs, CatalogRef};
