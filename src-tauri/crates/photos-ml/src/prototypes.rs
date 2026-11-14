use std::collections::BTreeMap;

use photos_core::embedding::{quantize, Prototypes};
use serde::Deserialize;

#[derive(Deserialize)]
struct File {
    model: String,
    prototypes: BTreeMap<String, Vec<f32>>,
}

/// The reference phrases, embedded offline by `scripts/ml/export_models.py` (ML design §3).
pub fn prototypes() -> Prototypes {
    let f: File = serde_json::from_str(include_str!("../prototypes.json")).expect("prototypes.json is valid");
    Prototypes { model: f.model, items: f.prototypes.into_iter().map(|(n, v)| (n, quantize(&v))).collect() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_phrase_the_export_writes_is_here_and_matches_the_model() {
        let p = prototypes();
        for name in [
            "mood:happy", "mood:calm", "mood:romantic", "mood:epic", "mood:melancholic", "mood:energetic", "mood:dreamy",
            "mood:playful", "junk:screenshot", "junk:document", "junk:whiteboard", "junk:blurry", "junk:accidental", "good",
            "face:real", "face:drawing", "face:print", "face:screen", "face:meme", "face:statue",
            "scene:sunset", "scene:night", "scene:snow", "scene:beach", "scene:backlit", "scene:portrait", "scene:foggy",
            "scene:indoor", "scene:daylight",
        ] {
            assert_eq!(p.get(name).map(|v| v.len()), Some(photos_core::embedding::DIM), "{name}");
        }
        assert_eq!(p.model, crate::manifest::manifest().by_role("image_embedding").unwrap().id);
    }

    #[test]
    fn search_knows_every_scene_tag() {
        let p = prototypes();
        let mut tags: Vec<&str> = p.items.iter().filter_map(|(n, _)| n.strip_prefix("tag:")).collect();
        tags.sort_unstable();
        let mut known = photos_core::embedding::SCENE_TAGS.to_vec();
        known.sort_unstable();
        assert_eq!(tags, known, "embedding::SCENE_TAGS is what query_parse reads tags with");
    }
}
