//! Develop drafts (needs `develop.write-drafts`; feature `tools`). Drafts only: the library,
//! viewer and thumbnails keep showing the saved edit until the person applies the draft.

#[cfg(all(target_arch = "wasm32", feature = "tools"))]
mod imp {
    use crate::bindings::lumen::plugin::drafts as d;

    /// Merge-patches (RFC 7386) the photo's draft recipe; `label` names the step in undo.
    pub fn patch(id: i64, merge_patch: &str, label: &str) -> Result<(), String> {
        d::patch(id, merge_patch, label)
    }
}

#[cfg(all(target_arch = "wasm32", feature = "tools"))]
pub use imp::*;
