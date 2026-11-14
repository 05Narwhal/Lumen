//! The compiled-in model list, the check that a file is the one the list names, and the
//! resuming download. Shared by Lumen, `photos-ml` and Lumen Setup; depends on no workspace crate.

pub mod fetch;
pub mod manifest;
pub mod store;

pub use manifest::*;
pub use store::{part_path, range_from, verify, ModelStore};
