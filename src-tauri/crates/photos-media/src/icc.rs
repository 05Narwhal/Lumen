//! ICC profiles built in code, so every export declares its colour without shipping a
//! profile file (Phase 8 spec §5). Version 2 matrix/TRC display profiles are what every
//! decoder and colour-managed viewer reads; the numbers are the published sRGB primaries
//! Bradford-adapted to the D50 profile connection space and the IEC 61966-2-1 curve.

use std::sync::OnceLock;

/// What the profile calls itself; `ProfileDescription` reads this back.
pub const SRGB_DESCRIPTION: &str = "sRGB IEC61966-2.1 (Lumen)";

const D50: [f64; 3] = [0.9642, 1.0, 0.8249];
/// sRGB's red, green and blue in XYZ, adapted from D65 to D50 (Bradford).
const SRGB_PRIMARIES_D50: [[f64; 3]; 3] = [[0.4360747, 0.2225045, 0.0139322], [0.3850649, 0.7168786, 0.0971045], [0.1430804, 0.0606169, 0.7141733]];
/// Entries in each tone curve: fine enough that 8- and 16-bit round trips are exact.
const CURVE_POINTS: usize = 1024;

/// The sRGB profile, built once.
pub fn srgb() -> &'static [u8] {
    static P: OnceLock<Vec<u8>> = OnceLock::new();
    P.get_or_init(|| build(SRGB_DESCRIPTION, SRGB_PRIMARIES_D50, srgb_decode))
}

/// The sRGB transfer function: encoded 0–1 → linear 0–1.
pub fn srgb_decode(v: f64) -> f64 {
    if v <= 0.04045 {
        v / 12.92
    } else {
        ((v + 0.055) / 1.055).powf(2.4)
    }
}

fn s15f16(v: f64) -> [u8; 4] {
    ((v * 65536.0).round() as i32).to_be_bytes()
}

fn xyz_tag(v: [f64; 3]) -> Vec<u8> {
    let mut t = b"XYZ \0\0\0\0".to_vec();
    for c in v {
        t.extend_from_slice(&s15f16(c));
    }
    t
}

/// `textDescriptionType`: the ASCII description plus empty Unicode and ScriptCode parts.
fn desc_tag(text: &str) -> Vec<u8> {
    let mut t = b"desc\0\0\0\0".to_vec();
    t.extend_from_slice(&(text.len() as u32 + 1).to_be_bytes());
    t.extend_from_slice(text.as_bytes());
    t.push(0);
    t.extend_from_slice(&[0; 4]); // Unicode language code
    t.extend_from_slice(&[0; 4]); // Unicode count
    t.extend_from_slice(&[0; 2]); // ScriptCode code
    t.push(0); // ScriptCode count
    t.extend_from_slice(&[0; 67]);
    t
}

fn text_tag(text: &str) -> Vec<u8> {
    let mut t = b"text\0\0\0\0".to_vec();
    t.extend_from_slice(text.as_bytes());
    t.push(0);
    t
}

fn curve_tag(decode: fn(f64) -> f64) -> Vec<u8> {
    let mut t = b"curv\0\0\0\0".to_vec();
    t.extend_from_slice(&(CURVE_POINTS as u32).to_be_bytes());
    for i in 0..CURVE_POINTS {
        let v = decode(i as f64 / (CURVE_POINTS - 1) as f64);
        t.extend_from_slice(&((v.clamp(0.0, 1.0) * 65535.0).round() as u16).to_be_bytes());
    }
    t
}

fn build(description: &str, primaries: [[f64; 3]; 3], decode: fn(f64) -> f64) -> Vec<u8> {
    let curve = curve_tag(decode);
    // The three tone curves share one block of data, which the format allows.
    let tags: Vec<(&[u8; 4], Vec<u8>)> = vec![
        (b"desc", desc_tag(description)),
        (b"cprt", text_tag("No copyright, use freely")),
        (b"wtpt", xyz_tag(D50)),
        (b"rXYZ", xyz_tag(primaries[0])),
        (b"gXYZ", xyz_tag(primaries[1])),
        (b"bXYZ", xyz_tag(primaries[2])),
        (b"rTRC", curve.clone()),
    ];
    let shared = [b"gTRC", b"bTRC"];
    let count = tags.len() + shared.len();
    let table_end = 128 + 4 + 12 * count;
    let mut data = Vec::new();
    let mut entries: Vec<([u8; 4], u32, u32)> = Vec::new();
    for (sig, body) in &tags {
        while !(table_end + data.len()).is_multiple_of(4) {
            data.push(0);
        }
        entries.push((**sig, (table_end + data.len()) as u32, body.len() as u32));
        data.extend_from_slice(body);
    }
    let (_, trc_offset, trc_len) = *entries.last().expect("rTRC");
    for sig in shared {
        entries.push((*sig, trc_offset, trc_len));
    }
    while !data.len().is_multiple_of(4) {
        data.push(0);
    }
    let size = table_end + data.len();

    let mut p = Vec::with_capacity(size);
    p.extend_from_slice(&(size as u32).to_be_bytes());
    p.extend_from_slice(b"lcms"); // preferred CMM: advisory only
    p.extend_from_slice(&[0x02, 0x10, 0, 0]); // version 2.1
    p.extend_from_slice(b"mntrRGB XYZ ");
    // A fixed date keeps the profile, and so every export, byte-for-byte reproducible.
    for v in [2026u16, 9, 22, 0, 0, 0] {
        p.extend_from_slice(&v.to_be_bytes());
    }
    p.extend_from_slice(b"acsp");
    p.extend_from_slice(&[0; 4 * 5]); // platform, flags, manufacturer, model, attributes (hi)
    p.extend_from_slice(&[0; 4]); // attributes (lo)
    p.extend_from_slice(&0u32.to_be_bytes()); // perceptual
    for c in D50 {
        p.extend_from_slice(&s15f16(c));
    }
    p.extend_from_slice(&[0; 4]); // creator
    p.resize(128, 0); // profile id (v4 only) and reserved
    p.extend_from_slice(&(count as u32).to_be_bytes());
    for (sig, off, len) in &entries {
        p.extend_from_slice(sig);
        p.extend_from_slice(&off.to_be_bytes());
        p.extend_from_slice(&len.to_be_bytes());
    }
    p.extend_from_slice(&data);
    debug_assert_eq!(p.len(), size);
    p
}

/// The colour space named by the ICC profile embedded in `bytes` (a file's first bytes), as
/// people call it: *sRGB*, *Display P3*, *Adobe RGB* (Round 9A). The profile is found by its
/// `acsp` signature wherever the container keeps it (a JPEG's APP2, a HEIC's `colr` box, a
/// TIFF's tag), so no container is parsed. `None` without a whole profile.
pub fn color_space_in(bytes: &[u8]) -> Option<String> {
    let mut from = 0;
    while let Some(at) = bytes[from..].windows(4).position(|w| w == b"acsp") {
        let sig = from + at;
        from = sig + 4;
        if let Some(d) = sig.checked_sub(36).and_then(|start| description(&bytes[start..])) {
            return Some(friendly(&d));
        }
    }
    None
}

fn be32(p: &[u8], at: usize) -> Option<usize> {
    Some(u32::from_be_bytes(p.get(at..at + 4)?.try_into().ok()?) as usize)
}

/// A profile's `desc` tag, v2 (`desc`, ASCII) or v4 (`mluc`, UTF-16, the first record).
fn description(p: &[u8]) -> Option<String> {
    let size = be32(p, 0)?;
    if size < 132 || size > p.len() {
        return None;
    }
    let p = &p[..size];
    for i in 0..be32(p, 128)?.min(64) {
        let entry = 132 + i * 12;
        if p.get(entry..entry + 4)? != b"desc" {
            continue;
        }
        let (off, len) = (be32(p, entry + 4)?, be32(p, entry + 8)?);
        let tag = p.get(off..off.checked_add(len)?)?;
        let text = match tag.get(..4)? {
            b"desc" => {
                let n = be32(tag, 8)?;
                String::from_utf8_lossy(tag.get(12..12 + n)?).into_owned()
            }
            b"mluc" => {
                let (n, at) = (be32(tag, 20)?, be32(tag, 24)?);
                let units: Vec<u16> = tag.get(at..at + n)?.as_chunks::<2>().0.iter().map(|c| u16::from_be_bytes(*c)).collect();
                String::from_utf16_lossy(&units)
            }
            _ => return None,
        };
        let text = text.trim_matches(|c: char| c == '\0' || c.is_whitespace()).to_string();
        return (!text.is_empty()).then_some(text);
    }
    None
}

fn friendly(d: &str) -> String {
    let named = [("Display P3", "Display P3"), ("sRGB", "sRGB"), ("Adobe RGB", "Adobe RGB"), ("A98", "Adobe RGB"), ("ProPhoto", "ProPhoto RGB"), ("2020", "Rec. 2020")];
    named.iter().find(|(k, _)| d.contains(k)).map_or_else(|| d.to_string(), |(_, v)| v.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A v4 profile whose description is a `mluc` record in UTF-16, as Apple's Display P3 is.
    fn v4_profile(description: &str) -> Vec<u8> {
        let text: Vec<u8> = description.encode_utf16().flat_map(|u| u.to_be_bytes()).collect();
        let mut mluc = b"mluc\0\0\0\0".to_vec();
        mluc.extend(1u32.to_be_bytes()); // records
        mluc.extend(12u32.to_be_bytes()); // record size
        mluc.extend(b"enUS");
        mluc.extend((text.len() as u32).to_be_bytes());
        mluc.extend(28u32.to_be_bytes()); // offset from the tag's start
        mluc.extend(&text);
        let mut p = vec![0u8; 128];
        p[36..40].copy_from_slice(b"acsp");
        p.extend(1u32.to_be_bytes());
        p.extend(b"desc");
        p.extend(144u32.to_be_bytes());
        p.extend((mluc.len() as u32).to_be_bytes());
        p.extend(&mluc);
        let size = p.len() as u32;
        p[..4].copy_from_slice(&size.to_be_bytes());
        p
    }

    #[test]
    fn a_files_colour_space_is_read_from_its_profile() {
        // Round 9A: wherever the profile sits in a file's first bytes (a JPEG's APP2, a HEIC's
        // `colr` box), its description names the colour space.
        let mut file = b"\xFF\xD8\xFF\xE2junk ICC_PROFILE\0\x01\x01".to_vec();
        file.extend(v4_profile("Display P3"));
        file.extend(b"more of the file");
        assert_eq!(color_space_in(&file).as_deref(), Some("Display P3"));
        let mut v2 = b"ftypheic....colrprof".to_vec();
        v2.extend(srgb());
        assert_eq!(color_space_in(&v2).as_deref(), Some("sRGB"));
        assert_eq!(color_space_in(&v4_profile("Adobe RGB (1998)")).as_deref(), Some("Adobe RGB"));
        assert_eq!(color_space_in(&v4_profile("A98C")).as_deref(), Some("Adobe RGB"), "as one of Julian's files names it");
        assert_eq!(color_space_in(&v4_profile("Camera RGB Profile")).as_deref(), Some("Camera RGB Profile"), "anything else as it is called");
        assert_eq!(color_space_in(b"no profile here, acsp alone"), None);
        let mut cut = v4_profile("Display P3");
        cut.truncate(150);
        assert_eq!(color_space_in(&cut), None, "a profile cut short is not read");
    }

    fn u32_at(p: &[u8], at: usize) -> u32 {
        u32::from_be_bytes(p[at..at + 4].try_into().unwrap())
    }

    fn tag<'a>(p: &'a [u8], sig: &[u8; 4]) -> &'a [u8] {
        let n = u32_at(p, 128) as usize;
        for i in 0..n {
            let e = 132 + 12 * i;
            if &p[e..e + 4] == sig {
                let (off, len) = (u32_at(p, e + 4) as usize, u32_at(p, e + 8) as usize);
                return &p[off..off + len];
            }
        }
        panic!("no {sig:?}");
    }

    #[test]
    fn the_header_describes_the_profile() {
        let p = srgb();
        assert_eq!(u32_at(p, 0) as usize, p.len());
        assert_eq!(&p[12..24], b"mntrRGB XYZ ");
        assert_eq!(&p[36..40], b"acsp");
        assert!(p.len().is_multiple_of(4));
    }

    #[test]
    fn every_tag_lies_inside_the_profile_and_is_aligned() {
        let p = srgb();
        let n = u32_at(p, 128) as usize;
        assert_eq!(n, 9);
        for i in 0..n {
            let e = 132 + 12 * i;
            let (off, len) = (u32_at(p, e + 4) as usize, u32_at(p, e + 8) as usize);
            assert!(off.is_multiple_of(4));
            assert!(off + len <= p.len());
        }
        assert_eq!(tag(p, b"gTRC"), tag(p, b"rTRC"), "the curves share their data");
    }

    #[test]
    fn the_curve_is_srgb_and_the_primaries_add_up_to_white() {
        let p = srgb();
        let c = tag(p, b"rTRC");
        assert_eq!(&c[..4], b"curv");
        assert_eq!(u32_at(c, 8) as usize, CURVE_POINTS);
        let at = |i: usize| u16::from_be_bytes([c[12 + 2 * i], c[13 + 2 * i]]);
        assert_eq!(at(0), 0);
        assert_eq!(at(CURVE_POINTS - 1), 65535);
        assert!((1..CURVE_POINTS).all(|i| at(i) >= at(i - 1)), "monotone");
        // Mid-grey: sRGB 0.5 decodes to linear 0.2140.
        let mid = at(CURVE_POINTS / 2) as f64 / 65535.0;
        assert!((mid - srgb_decode((CURVE_POINTS / 2) as f64 / (CURVE_POINTS - 1) as f64)).abs() < 1e-4);
        // The three primaries sum to the D50 white.
        let sum: Vec<f64> = (0..3).map(|k| SRGB_PRIMARIES_D50.iter().map(|p| p[k]).sum()).collect();
        for (s, w) in sum.iter().zip(D50) {
            assert!((s - w).abs() < 1e-3, "{sum:?}");
        }
    }

    #[test]
    #[ignore = "needs ExifTool"]
    fn exiftool_reads_the_profile() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("srgb.icc");
        std::fs::write(&path, srgb()).unwrap();
        let mut s = crate::ExifTool::locate().expect("ExifTool").session().unwrap();
        let v = s.read_json(&path, &["ProfileDescription", "ProfileClass", "ColorSpaceData", "ProfileVersion"]).unwrap();
        assert_eq!(v["ICC-header:ProfileClass"], "mntr");
        assert_eq!(v["ICC_Profile:ProfileDescription"], SRGB_DESCRIPTION);
    }
}
