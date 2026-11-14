//! The model list moved to `photos-models` (shared with Lumen Setup); re-exported here so the
//! `photos_ml::manifest::…` call sites keep compiling.

pub use photos_models::manifest::*;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aesthetic_head_matches_embedding_dim() {
        let clip = manifest().by_role("image_embedding").expect("an image model");
        assert_eq!(clip.aesthetic.as_ref().unwrap().weights.len(), photos_core::embedding::DIM);
    }
}
