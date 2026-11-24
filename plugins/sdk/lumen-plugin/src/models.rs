//! Models the manifest declares in `[[models]]`, run natively by Lumen (needs `models`;
//! feature `places`). Wasm never holds the weights or the photo's bytes. Each call counts once
//! against the 30 s limit, so run at most 32 assets per tool step.

#[cfg(all(target_arch = "wasm32", feature = "places"))]
mod imp {
    use crate::bindings::lumen::plugin::models as m;
    use crate::{Neighbour, Tensor, VectorSource};

    /// Runs `model` (a `[[models]]` id) on f32 tensors.
    pub fn run(model: &str, inputs: &[Tensor]) -> Result<Vec<Tensor>, String> {
        m::run(model, inputs)
    }
    /// Runs `model` on one photo: Lumen decodes it upright, resizes and normalises it per the
    /// manifest's `inputs.<input>` and feeds it as that input.
    pub fn run_on_asset(model: &str, asset: u64, input: &str) -> Result<Vec<Tensor>, String> {
        m::run_on_asset(model, asset, input)
    }
    /// The `k` rows of `source` with the highest dot product with `query`, best first.
    pub fn nearest(source: &VectorSource, query: &[f32], k: u32) -> Result<Vec<Neighbour>, String> {
        m::nearest(source, query, k)
    }
}

#[cfg(all(target_arch = "wasm32", feature = "places"))]
pub use imp::*;
