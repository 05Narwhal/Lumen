//! Which detections are people's faces (Round 4B). YuNet finds faces on screens, in drawings
//! and Memoji, on statues and in printed photos; the photo's CLIP embedding cannot tell (a
//! screen of a video call is a "photo of people"), but the face's own crop can.
//!
//! Measured on Julian's library (plan Task 1–2, `scripts/ml/people_quality.py`): the
//! detector's score, the face's size and the photo's junk share do not separate the faces he
//! removed from the ones he kept, so none of them rejects a face. The crop check does: with
//! screen, drawing, print or statue beating "a real person's face" by `CROP_MARGIN`, it takes
//! 1 of 422 named faces (a chat screenshot) and 50 of the hidden people's 912 (avatars,
//! cartoons, statues, faces on screens). "Meme" is not a reason: it fired on real children.

/// Kinds that are not a person in front of the camera, as named by the `face:*` prototypes.
pub const NOT_REAL: [&str; 4] = ["screen", "drawing", "print", "statue"];
/// How far a non-real kind's share must beat `real`'s.
pub const CROP_MARGIN: f32 = 0.2;
/// The softmax temperature over `face:*`, as measured.
pub const CROP_TEMPERATURE: f32 = 0.01;

/// What is known about one detection.
#[derive(Debug, Clone, PartialEq)]
pub struct FaceSignals {
    /// YuNet's score, 0..1.
    pub score: f32,
    /// The face's longer side in the 1024 px scan.
    pub side_px: f32,
    /// The crop's softmax over the `face:*` prototypes, names without the prefix. Empty
    /// without the image model: the crop check then passes the face.
    pub crop: Vec<(String, f32)>,
}

/// `Ok(quality)` for a person's face, 0..1 (ranks exemplars and covers), or `Err(reason)`.
pub fn judge(s: &FaceSignals) -> Result<f32, &'static str> {
    if let Some(reason) = crop_reason(&s.crop) {
        return Err(reason);
    }
    Ok(quality(s.score, s.side_px))
}

/// The non-real kind that beats `real` by the margin, if any.
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

/// The detector's confidence, discounted for faces too small to describe well (full at 80 px).
pub fn quality(score: f32, side_px: f32) -> f32 {
    let size = (side_px / 80.0).clamp(0.25, 1.0);
    (score.clamp(0.0, 1.0) * size).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn real() -> FaceSignals {
        FaceSignals { score: 0.92, side_px: 90.0, crop: vec![("real".into(), 0.8), ("screen".into(), 0.1), ("meme".into(), 0.1)] }
    }

    #[test]
    fn a_clear_face_is_kept_with_high_quality() {
        assert!(judge(&real()).unwrap() > 0.85);
    }

    #[test]
    fn a_face_on_a_screen_or_a_statue_is_not_a_person() {
        let screen = FaceSignals { crop: vec![("real".into(), 0.2), ("screen".into(), 0.7)], ..real() };
        assert_eq!(judge(&screen), Err("screen"));
        let statue = FaceSignals { crop: vec![("real".into(), 0.1), ("statue".into(), 0.5), ("drawing".into(), 0.35)], ..real() };
        assert_eq!(judge(&statue), Err("statue"));
    }

    #[test]
    fn a_narrow_lead_is_not_enough() {
        let close = FaceSignals { crop: vec![("real".into(), 0.4), ("drawing".into(), 0.55)], ..real() };
        assert!(judge(&close).is_ok());
    }

    #[test]
    fn meme_is_never_a_reason() {
        let meme = FaceSignals { crop: vec![("real".into(), 0.05), ("meme".into(), 0.95)], ..real() };
        assert!(judge(&meme).is_ok(), "it fired on real children in Julian's photos");
    }

    #[test]
    fn without_the_model_the_crop_check_passes() {
        assert!(judge(&FaceSignals { crop: vec![], ..real() }).is_ok());
    }

    #[test]
    fn small_faces_are_kept_but_rank_lower() {
        let small = judge(&FaceSignals { side_px: 30.0, ..real() }).unwrap();
        let large = judge(&real()).unwrap();
        assert!(small < large && small > 0.0);
    }
}
