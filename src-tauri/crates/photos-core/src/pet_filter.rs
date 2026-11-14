//! Which detections are pets (Round 4C), as [`crate::face_filter`] does for faces: the crop's
//! CLIP softmax over the `pet:*` prototypes.
//!
//! Measured on COCO val2017 (`scripts/ml/eval_pets.py filter`), never on Julian's photos:
//! with *drawing* or *toy* beating *a real cat or dog* by [`CROP_MARGIN`], 3 of 382 real cats
//! and dogs are rejected. *Screen* and *print* are not reasons: COCO's cats sit on laptops and
//! in front of TVs, and *screen* won on 164 of the 382 real ones.

/// Kinds that are not an animal in front of the camera, as named by the `pet:*` prototypes.
pub const NOT_REAL: [&str; 2] = ["drawing", "toy"];
/// How far a non-real kind's share must beat `real`'s.
pub const CROP_MARGIN: f32 = 0.2;
/// The softmax temperature over `pet:*`.
pub const CROP_TEMPERATURE: f32 = 0.01;

/// The non-real kind that beats `real` by the margin, if any. Without the shares (no image
/// model) a detection passes.
pub fn crop_reason(crop: &[(String, f32)]) -> Option<&'static str> {
    let share = |k: &str| crop.iter().find(|(n, _)| n == k).map(|(_, v)| *v);
    let real = share("real")?;
    NOT_REAL
        .iter()
        .filter_map(|k| share(k).map(|v| (*k, v)))
        .filter(|(_, v)| v - real > CROP_MARGIN)
        .max_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(k, _)| k)
}

/// How sure CLIP must be (its softmax over `species:*`) to overrule the detector's species.
///
/// D-FINE-S calls some black cats dogs (Socks the Cat at 0.93). Measured on COCO val2017
/// (`scripts/ml/eval_pets.py species`, 375 animals): overruling at 0.95 fixes 3 and breaks
/// none, 98.1 % → 98.9 % right; at 0.8 it breaks one.
pub const SPECIES_CONFIDENCE: f32 = 0.95;

/// "cat" or "dog": the detector's reading unless the crop's CLIP shares over `species:*` are
/// sure of the other.
pub fn species<'a>(detected: &'a str, shares: &[(String, f32)]) -> &'a str {
    let other = if detected == "cat" { "dog" } else { "cat" };
    match shares.iter().find(|(n, _)| n == other) {
        Some((_, s)) if *s >= SPECIES_CONFIDENCE => other,
        _ => detected,
    }
}

/// The detector's confidence, discounted for animals too small to describe well (full at
/// 160 px on their short side in the photo as scanned).
pub fn quality(score: f32, short_side_px: f32) -> f32 {
    let size = (short_side_px / 160.0).clamp(0.25, 1.0);
    (score.clamp(0.0, 1.0) * size).clamp(0.0, 1.0)
}

/// The least share of `tag:dog` + `tag:cat` (softmax over `tag:*` at the scene tags'
/// temperature) a photo's own CLIP embedding needs before the detector looks at it (Round 8A).
///
/// Measured on COCO val2017's pets / other animals / no animals subset
/// (`photos-ml --test pet_prefilter_public`), never on Julian's photos: at 0.001 it keeps 99.4 %
/// of the photos where D-FINE finds a pet the scan keeps (2 of 320 lost) and skips 49 % of
/// photos with no animal, 25 % of those with another animal.
pub const PREFILTER_SHARE: f32 = 0.001;

/// Whether a photo may show a cat or a dog, judged from its CLIP embedding: the pet scan
/// skips the detector for the rest.
pub fn may_show_a_pet(v: &[i8], protos: &crate::embedding::Prototypes) -> bool {
    let share: f32 = crate::embedding::softmax(v, protos, "tag:", 0.01).iter().filter(|(n, _)| n == "dog" || n == "cat").map(|(_, s)| s).sum();
    share >= PREFILTER_SHARE
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_prefilter_keeps_what_looks_like_a_dog_or_a_cat() {
        let axis = |k: usize| {
            let mut v = vec![0i8; 8];
            v[k] = 127;
            v
        };
        let protos = crate::embedding::Prototypes {
            model: "m".into(),
            items: vec![("tag:dog".into(), axis(0)), ("tag:cat".into(), axis(1)), ("tag:beach".into(), axis(2)), ("tag:car".into(), axis(3))],
        };
        assert!(may_show_a_pet(&axis(0), &protos));
        assert!(may_show_a_pet(&axis(1), &protos));
        assert!(!may_show_a_pet(&axis(2), &protos), "a beach");
    }

    fn shares(v: &[(&str, f32)]) -> Vec<(String, f32)> {
        v.iter().map(|(k, s)| (k.to_string(), *s)).collect()
    }

    #[test]
    fn a_plush_or_a_cartoon_is_not_a_pet() {
        assert_eq!(crop_reason(&shares(&[("real", 0.1), ("toy", 0.8)])), Some("toy"));
        assert_eq!(crop_reason(&shares(&[("real", 0.2), ("drawing", 0.6), ("toy", 0.2)])), Some("drawing"));
    }

    #[test]
    fn a_cat_in_front_of_a_screen_stays_a_cat() {
        assert_eq!(crop_reason(&shares(&[("real", 0.3), ("screen", 0.7)])), None);
        assert_eq!(crop_reason(&shares(&[("real", 0.45), ("toy", 0.55)])), None, "a narrow lead is not enough");
        assert_eq!(crop_reason(&[]), None, "no model, no judgement");
    }

    #[test]
    fn clip_overrules_the_detectors_species_only_when_sure() {
        // Socks the Cat, read as a dog by D-FINE at 0.93: CLIP is 97 % sure he is a cat.
        assert_eq!(species("dog", &shares(&[("cat", 0.97), ("dog", 0.03)])), "cat");
        assert_eq!(species("cat", &shares(&[("cat", 0.02), ("dog", 0.98)])), "dog");
        assert_eq!(species("dog", &shares(&[("cat", 0.9), ("dog", 0.1)])), "dog", "90 % is not sure enough");
        assert_eq!(species("cat", &shares(&[("cat", 0.6), ("dog", 0.4)])), "cat");
        assert_eq!(species("dog", &[]), "dog", "no model, the detector's word");
    }

    #[test]
    fn quality_falls_off_for_small_animals() {
        assert!((quality(0.9, 300.0) - 0.9).abs() < 1e-6);
        assert!((quality(0.9, 80.0) - 0.45).abs() < 1e-6);
        assert!((quality(0.9, 10.0) - 0.225).abs() < 1e-6);
    }
}
