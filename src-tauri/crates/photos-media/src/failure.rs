//! Sorting the reader's own message about a file it could not open into a cause a person can
//! act on (Round 22 §5). The text is what `DerivativeStore::failure` keeps per hash; this module
//! only reads it. Pure: no file, no tool.

use serde::Serialize;
use ts_rs::TS;

/// Why a photo or video does not open.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum UnreadableCause {
    /// The file ends early: a copy that stopped, a card pulled out.
    Incomplete,
    /// What is inside is not an image or a video the reader knows.
    NotAnImage,
    /// More pixels than the reader will take at once.
    TooLarge,
    Unknown,
}

/// Sorts `text` (a decoder's or FFmpeg's message), ignoring case.
pub fn cause(text: &str) -> UnreadableCause {
    let t = text.to_lowercase();
    let has = |needles: &[&str]| needles.iter().any(|n| t.contains(n));
    if has(&["memory limit exceeded", "too large", "dimensions are too large"]) {
        UnreadableCause::TooLarge
    } else if has(&["unexpected end of file", "partial file", "invalid nal unit size", "error number -1145393733", "moov atom not found", "truncated"]) {
        UnreadableCause::Incomplete
    } else if has(&["invalid data found when processing input", "illegal start bytes", "invalid png signature", "not an image", "unrecognized", "unsupported"]) {
        UnreadableCause::NotAnImage
    } else {
        UnreadableCause::Unknown
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_texts_the_real_library_logged() {
        for t in ["unexpected end of file", "partial file", "invalid nal unit size", "error number -1145393733", "moov atom not found"] {
            assert_eq!(cause(t), UnreadableCause::Incomplete, "{t}");
        }
        for t in ["invalid data found when processing input", "illegal start bytes", "invalid png signature"] {
            assert_eq!(cause(t), UnreadableCause::NotAnImage, "{t}");
        }
        assert_eq!(cause("Memory limit exceeded"), UnreadableCause::TooLarge);
        assert_eq!(cause("something nobody has seen"), UnreadableCause::Unknown);
        assert_eq!(cause(""), UnreadableCause::Unknown);
    }

    #[test]
    fn matching_ignores_case() {
        assert_eq!(cause("Invalid Data Found When Processing Input"), UnreadableCause::NotAnImage);
        assert_eq!(cause("MOOV ATOM NOT FOUND"), UnreadableCause::Incomplete);
    }
}
