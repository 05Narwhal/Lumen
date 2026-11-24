//! Places for the Place review screen (needs `places.write`; feature `places`). A plugin
//! suggests; only the person's *Write N locations* stores anything.

/// The most candidates the host keeps per asset (spec §8.6).
pub const MAX_CANDIDATES: usize = 5;

#[cfg(all(target_arch = "wasm32", feature = "places"))]
mod imp {
    use crate::bindings::lumen::plugin::places as p;
    use crate::{PlaceHit, Suggestion};

    /// Sends suggestions (up to [`super::MAX_CANDIDATES`] candidates each, best first).
    pub fn suggest(batch: &[Suggestion]) -> Result<(), String> {
        p::suggest(batch)
    }
    /// Place names from Lumen's offline atlas matching `name` (e.g. a folder called `Bled 2024`).
    pub fn lookup(name: &str) -> Vec<PlaceHit> {
        p::lookup(name)
    }
}

#[cfg(all(target_arch = "wasm32", feature = "places"))]
pub use imp::*;
