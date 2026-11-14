//! The photos a tools plugin may look at (needs `photos.read`; feature `tools`). `scope` is the
//! photos the person is working on (Develop's filmstrip), nothing else.

#[cfg(all(target_arch = "wasm32", feature = "tools"))]
mod imp {
    use crate::bindings::lumen::plugin::photos as p;
    use crate::PhotoMeta;

    pub fn scope() -> Vec<i64> {
        p::scope()
    }
    pub fn metadata(id: i64) -> Result<PhotoMeta, String> {
        p::metadata(id)
    }
    /// A preview image no larger than `max` pixels on its long side.
    pub fn preview(id: i64, max: u32) -> Result<Vec<u8>, String> {
        p::preview(id, max)
    }
    /// The photo's current Develop recipe as JSON text.
    pub fn recipe(id: i64) -> Result<String, String> {
        p::recipe(id)
    }
}

#[cfg(all(target_arch = "wasm32", feature = "tools"))]
pub use imp::*;
