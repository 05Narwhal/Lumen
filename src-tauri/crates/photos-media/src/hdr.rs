//! Which pictures carry more than standard dynamic range, and what we can honestly do
//! with each kind.
//!
//! Two unrelated things are both called "HDR photo":
//!
//! * **Gain map** (Apple HEIC, Google Ultra HDR JPEG). The file holds an ordinary SDR
//!   image plus a map saying how much brighter each pixel may go. Decoding it the plain
//!   way gives a correct SDR picture, which is why these never look wrong in a grid.
//! * **PQ / HLG** (AVIF, HEIF, video frames). The pixels themselves are in an HDR
//!   transfer function. Decoded as if they were SDR they look washed out and flat, so
//!   anything we render down has to be tone mapped.
//!
//! Detection is deliberately cheap: a bounded scan for the markers that identify a gain
//! map, plus the transfer characteristics ffprobe already gives us.

use std::io::Read;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::ffmpeg::Probe;
use crate::kind::MediaKind;

/// How much of the file we scan for gain-map markers. They live in the metadata near the
/// front (XMP, item properties), so this is generous.
const SCAN_BYTES: usize = 512 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HdrKind {
    #[default]
    None,
    /// SDR pixels plus a gain map: correct as-is, brighter on a capable display.
    GainMap,
    /// Perceptual quantizer (SMPTE ST 2084).
    Pq,
    /// Hybrid log-gamma (ARIB STD-B67).
    Hlg,
}

impl HdrKind {
    /// `assets.hdr`. Stable across versions; 0 always means "ordinary".
    pub fn code(self) -> i64 {
        match self {
            HdrKind::None => 0,
            HdrKind::GainMap => 1,
            HdrKind::Pq => 2,
            HdrKind::Hlg => 3,
        }
    }

    pub fn from_code(code: i64) -> Self {
        match code {
            1 => HdrKind::GainMap,
            2 => HdrKind::Pq,
            3 => HdrKind::Hlg,
            _ => HdrKind::None,
        }
    }

    pub fn is_hdr(self) -> bool {
        self != HdrKind::None
    }

    /// True when decoding the pixels as SDR would look wrong, so a derivative has to be
    /// tone mapped. Gain-map files decode to a correct SDR image already.
    pub fn needs_tone_mapping(self) -> bool {
        matches!(self, HdrKind::Pq | HdrKind::Hlg)
    }
}

/// Markers that identify a gain map. Apple writes the first two into HEIC; Google's Ultra
/// HDR and the ISO 21496-1 standard write the others into JPEG XMP.
const GAIN_MAP_MARKERS: &[&[u8]] = &[
    b"urn:com:apple:photo:2020:aux:hdrgainmap",
    b"hdrgainmap",
    b"urn:iso:std:iso:ts:21496",
    b"hdrgm:Version",
    b"http://ns.google.com/photos/1.0/container",
];

/// ffprobe's `color_transfer` for the two HDR transfer functions.
pub fn transfer_to_kind(transfer: Option<&str>) -> HdrKind {
    match transfer {
        Some("smpte2084") | Some("smpte-st-2084") => HdrKind::Pq,
        Some("arib-std-b67") => HdrKind::Hlg,
        _ => HdrKind::None,
    }
}

/// Scan the head of a file for gain-map markers.
pub fn has_gain_map(path: &Path) -> bool {
    let Ok(mut f) = std::fs::File::open(path) else { return false };
    let mut buf = vec![0u8; SCAN_BYTES];
    let Ok(n) = f.read(&mut buf) else { return false };
    scan_for_gain_map(&buf[..n])
}

/// Pure half of [`has_gain_map`], so the marker list can be tested without files.
pub fn scan_for_gain_map(bytes: &[u8]) -> bool {
    GAIN_MAP_MARKERS.iter().any(|m| find(bytes, m))
}

fn find(haystack: &[u8], needle: &[u8]) -> bool {
    if needle.is_empty() || haystack.len() < needle.len() {
        return false;
    }
    haystack.windows(needle.len()).any(|w| w == needle)
}

/// PNG carries its transfer function in a `cICP` chunk: four bytes of primaries,
/// transfer, matrix and range. Transfer 16 is PQ, 18 is HLG (CICP, ITU-T H.273).
pub fn png_cicp_kind(bytes: &[u8]) -> HdrKind {
    let Some(pos) = bytes.windows(4).position(|w| w == b"cICP") else { return HdrKind::None };
    match bytes.get(pos + 5) {
        Some(16) => HdrKind::Pq,
        Some(18) => HdrKind::Hlg,
        _ => HdrKind::None,
    }
}

/// What kind of high dynamic range, if any, this file holds. `probe` is ffprobe's answer
/// when we have one; without it PQ and HLG can still be found in PNG's `cICP` chunk, and
/// gain maps are found by their markers in every format.
pub fn detect(path: &Path, kind: MediaKind, probe: Option<&Probe>) -> HdrKind {
    if let Some(p) = probe {
        let from_probe = transfer_to_kind(p.color_transfer.as_deref());
        if from_probe.is_hdr() {
            return from_probe;
        }
    }
    if kind == MediaKind::Image {
        if let Ok(mut f) = std::fs::File::open(path) {
            let mut head = vec![0u8; 8192];
            if let Ok(n) = f.read(&mut head) {
                let from_png = png_cicp_kind(&head[..n]);
                if from_png.is_hdr() {
                    return from_png;
                }
            }
        }
    }
    // A gain map is the common case for phone photos, in both HEIC and JPEG.
    if matches!(kind, MediaKind::Image | MediaKind::Heic) && has_gain_map(path) {
        return HdrKind::GainMap;
    }
    HdrKind::None
}

/// Whether the webview can show this file's own bytes, so the viewer can serve the
/// original and keep the dynamic range instead of a rendered SDR copy. Chromium reads
/// JPEG gain maps, PNG `cICP` and AVIF; it cannot read HEIC at all.
pub fn browser_can_render(extension: &str) -> bool {
    matches!(extension.to_ascii_lowercase().as_str(), "jpg" | "jpeg" | "jpe" | "png" | "avif" | "webp" | "gif")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_round_trip_and_default_to_none() {
        for k in [HdrKind::None, HdrKind::GainMap, HdrKind::Pq, HdrKind::Hlg] {
            assert_eq!(HdrKind::from_code(k.code()), k);
        }
        assert_eq!(HdrKind::from_code(99), HdrKind::None);
        assert!(!HdrKind::None.is_hdr());
        assert!(HdrKind::GainMap.is_hdr());
        // A gain map decodes to a correct SDR image; PQ and HLG do not.
        assert!(!HdrKind::GainMap.needs_tone_mapping());
        assert!(HdrKind::Pq.needs_tone_mapping());
        assert!(HdrKind::Hlg.needs_tone_mapping());
    }

    #[test]
    fn reads_transfer_functions_and_markers() {
        assert_eq!(transfer_to_kind(Some("smpte2084")), HdrKind::Pq);
        assert_eq!(transfer_to_kind(Some("arib-std-b67")), HdrKind::Hlg);
        assert_eq!(transfer_to_kind(Some("bt709")), HdrKind::None);
        assert_eq!(transfer_to_kind(None), HdrKind::None);

        assert!(scan_for_gain_map(b"....urn:com:apple:photo:2020:aux:hdrgainmap...."));
        assert!(scan_for_gain_map(b"<x:xmpmeta hdrgm:Version=\"1.0\">"));
        assert!(!scan_for_gain_map(b"an ordinary jpeg with no markers"));
        assert!(!scan_for_gain_map(b""));
    }

    #[test]
    fn reads_png_cicp_transfer() {
        // chunk length, type, then primaries/transfer/matrix/range.
        let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
        png.extend_from_slice(&[0, 0, 0, 4]);
        png.extend_from_slice(b"cICP");
        png.extend_from_slice(&[9, 16, 0, 1]);
        assert_eq!(png_cicp_kind(&png), HdrKind::Pq);
        let transfer_byte = png.len() - 3;
        png[transfer_byte] = 18;
        assert_eq!(png_cicp_kind(&png), HdrKind::Hlg);
        png[transfer_byte] = 1;
        assert_eq!(png_cicp_kind(&png), HdrKind::None);
        assert_eq!(png_cicp_kind(b"no chunk here"), HdrKind::None);
    }

    #[test]
    fn knows_what_the_webview_can_show() {
        assert!(browser_can_render("JPG"));
        assert!(browser_can_render("avif"));
        assert!(!browser_can_render("heic"), "Chromium cannot decode HEIC");
        assert!(!browser_can_render("nef"));
    }
}
