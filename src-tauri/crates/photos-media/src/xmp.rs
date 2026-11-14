//! The XMP packet: what Lightroom, Capture One and Bridge write, and often the only place
//! an exported file still remembers when the photograph was taken.
//!
//! This is a scanner, not a parser. The packet is XML, but everything wanted here is one
//! attribute or element, and a photo library must never hand a 40 MB TIFF to an XML parser
//! to learn one date.

use std::io::Read;
use std::path::Path;

/// How much of a file is searched for the packet. XMP is written near the front in JPEG
/// (APP1), PNG (an `iTXt` chunk) and HEIC (a `mime` item), and a packet that starts later
/// than this belongs to an embedded thumbnail, not the image.
const SCAN_BYTES: u64 = 1_000_000;

/// The properties that hold a capture time, best first. `xmp:ModifyDate` is deliberately
/// absent: it is the moment of export.
const PROPERTIES: &[&str] = &["photoshop:DateCreated", "xmp:CreateDate", "exif:DateTimeOriginal"];

/// The capture date in `path`'s XMP packet, as it is written (ISO-8601).
pub fn capture_date(path: &Path) -> Option<String> {
    let mut head = Vec::new();
    std::fs::File::open(path).ok()?.take(SCAN_BYTES).read_to_end(&mut head).ok()?;
    capture_date_in(&head)
}

pub fn capture_date_in(bytes: &[u8]) -> Option<String> {
    let start = find(bytes, b"<x:xmpmeta")?;
    let end = find(&bytes[start..], b"</x:xmpmeta>").map_or(bytes.len(), |e| start + e);
    let packet = String::from_utf8_lossy(&bytes[start..end]);
    PROPERTIES.iter().find_map(|p| attribute(&packet, p).or_else(|| element(&packet, p)))
}

/// The app that made the file, as XMP names it (`xmp:CreatorTool`): an export's *Exported
/// from* when its EXIF has no `Software` (Round 7).
pub fn creator_tool(path: &Path) -> Option<String> {
    let mut head = Vec::new();
    std::fs::File::open(path).ok()?.take(SCAN_BYTES).read_to_end(&mut head).ok()?;
    creator_tool_in(&head)
}

pub fn creator_tool_in(bytes: &[u8]) -> Option<String> {
    let start = find(bytes, b"<x:xmpmeta")?;
    let end = find(&bytes[start..], b"</x:xmpmeta>").map_or(bytes.len(), |e| start + e);
    let packet = String::from_utf8_lossy(&bytes[start..end]);
    attribute(&packet, "xmp:CreatorTool").or_else(|| element(&packet, "xmp:CreatorTool"))
}

/// `photoshop:DateCreated="2026-09-10T16:37:17"`
fn attribute(packet: &str, property: &str) -> Option<String> {
    let at = packet.find(&format!("{property}=\""))? + property.len() + 2;
    let rest = &packet[at..];
    let value = rest[..rest.find('"')?].trim();
    (!value.is_empty()).then(|| value.to_string())
}

/// `<xmp:CreateDate>2026-09-10T16:37:17</xmp:CreateDate>`
fn element(packet: &str, property: &str) -> Option<String> {
    let at = packet.find(&format!("<{property}>"))? + property.len() + 2;
    let rest = &packet[at..];
    let value = rest[..rest.find('<')?].trim();
    (!value.is_empty()).then(|| value.to_string())
}

/// What a photographer marked a photo with, as an XMP packet records it: the stars, a colour
/// label, a reject, keywords, a title and a caption (Round 19 C3). `rating` 1–5 only and
/// `color_label` in Lumen's order (1 red … 5 purple), so a mark of nothing is `None`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Marks {
    pub rating: Option<i64>,
    /// `Some(-1)` when `xmp:Rating` is -1 (Lightroom's reject).
    pub flag: Option<i64>,
    pub color_label: Option<i64>,
    pub keywords: Vec<String>,
    pub title: Option<String>,
    pub caption: Option<String>,
}

impl Marks {
    pub fn is_empty(&self) -> bool {
        *self == Marks::default()
    }
}

/// The marks in `path`'s own XMP packet (a JPEG from Lightroom keeps them there).
pub fn marks(path: &Path) -> Marks {
    let mut head = Vec::new();
    if std::fs::File::open(path).and_then(|f| f.take(SCAN_BYTES).read_to_end(&mut head)).is_err() {
        return Marks::default();
    }
    marks_in(&head)
}

/// The marks in an `.xmp` sidecar file (the whole file is the packet).
pub fn marks_of_sidecar(path: &Path) -> Marks {
    std::fs::read(path).map(|b| marks_in(&b)).unwrap_or_default()
}

pub fn marks_in(bytes: &[u8]) -> Marks {
    let Some(start) = find(bytes, b"<x:xmpmeta") else { return Marks::default() };
    let end = find(&bytes[start..], b"</x:xmpmeta>").map_or(bytes.len(), |e| start + e);
    let packet = String::from_utf8_lossy(&bytes[start..end]);
    let value = |p: &str| attribute(&packet, p).or_else(|| element(&packet, p));
    let stars = value("xmp:Rating").and_then(|v| v.trim().parse::<i64>().ok());
    let label = value("xmp:Label").map(|v| v.trim().to_ascii_lowercase());
    Marks {
        rating: stars.filter(|s| (1..=5).contains(s)),
        flag: stars.filter(|s| *s == -1),
        color_label: label.and_then(|l| ["red", "yellow", "green", "blue", "purple"].iter().position(|n| *n == l)).map(|i| i as i64 + 1),
        keywords: list_items(&packet, "dc:subject"),
        title: list_items(&packet, "dc:title").into_iter().next(),
        caption: list_items(&packet, "dc:description").into_iter().next(),
    }
}

/// The `<rdf:li>` texts inside `<property>…</property>`, entities decoded, repeats dropped.
fn list_items(packet: &str, property: &str) -> Vec<String> {
    let Some(open) = packet.find(&format!("<{property}>")) else { return Vec::new() };
    let body = &packet[open..];
    let body = &body[..body.find(&format!("</{property}>")).unwrap_or(body.len())];
    let mut out: Vec<String> = Vec::new();
    let mut rest = body;
    while let Some(at) = rest.find("<rdf:li") {
        rest = &rest[at..];
        let Some(gt) = rest.find('>') else { break };
        if rest[..gt].ends_with('/') {
            rest = &rest[gt..];
            continue;
        }
        let text = &rest[gt + 1..];
        let Some(lt) = text.find('<') else { break };
        let item = decode_entities(text[..lt].trim());
        if !item.is_empty() && !out.contains(&item) {
            out.push(item);
        }
        rest = &text[lt..];
    }
    out
}

fn decode_entities(s: &str) -> String {
    let mut out = s.replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&apos;", "'");
    while let Some(at) = out.find("&#") {
        let Some(semi) = out[at..].find(';') else { break };
        let code = &out[at + 2..at + semi];
        let ch = code.strip_prefix('x').map_or_else(|| code.parse().ok(), |h| u32::from_str_radix(h, 16).ok()).and_then(char::from_u32);
        out.replace_range(at..at + semi + 1, &ch.map_or(String::new(), String::from));
    }
    out.replace("&amp;", "&")
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

#[cfg(test)]
mod tests {
    use super::*;

    const PACKET: &[u8] = br#"<?xpacket begin="" id="W5M0MpCehiHzreSzNTczkc9d"?>
      <x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:RDF><rdf:Description
        xmp:ModifyDate="2026-09-18T14:42:48+02:00"
        photoshop:DateCreated="2026-09-10T16:37:17"
        xmp:CreateDate="2026-09-10T16:37:17"/></rdf:RDF></x:xmpmeta><?xpacket end="w"?>"#;

    #[test]
    fn reads_the_capture_date_and_ignores_the_edit_date() {
        assert_eq!(capture_date_in(PACKET).as_deref(), Some("2026-09-10T16:37:17"));
    }

    #[test]
    fn reads_an_element_form_packet() {
        let packet = br#"<x:xmpmeta><rdf:Description>
            <xmp:CreateDate>2026-09-09T12:31:29.20</xmp:CreateDate></rdf:Description></x:xmpmeta>"#;
        assert_eq!(capture_date_in(packet).as_deref(), Some("2026-09-09T12:31:29.20"));
    }


    const MARKED: &[u8] = br#"<x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:RDF><rdf:Description
        xmp:Rating="4" xmp:Label="Green">
        <dc:title><rdf:Alt><rdf:li xml:lang="x-default">Harbour &amp; fog</rdf:li></rdf:Alt></dc:title>
        <dc:description><rdf:Alt><rdf:li xml:lang="x-default">Early morning at the quay</rdf:li></rdf:Alt></dc:description>
        <dc:subject><rdf:Bag><rdf:li>boats</rdf:li><rdf:li>morning light</rdf:li><rdf:li>boats</rdf:li></rdf:Bag></dc:subject>
        </rdf:Description></rdf:RDF></x:xmpmeta>"#;

    #[test]
    fn reads_rating_label_keywords_title_and_caption() {
        let m = marks_in(MARKED);
        assert_eq!(m.rating, Some(4));
        assert_eq!(m.flag, None);
        assert_eq!(m.color_label, Some(3), "Green is 3 in Lumen's order");
        assert_eq!(m.title.as_deref(), Some("Harbour & fog"));
        assert_eq!(m.caption.as_deref(), Some("Early morning at the quay"));
        assert_eq!(m.keywords, ["boats", "morning light"], "in order, without repeats");
        assert!(!m.is_empty());
    }

    #[test]
    fn a_rejected_rating_is_a_flag_and_no_packet_has_no_marks() {
        let m = marks_in(br#"<x:xmpmeta><rdf:Description><xmp:Rating>-1</xmp:Rating><xmp:Label>Second</xmp:Label></rdf:Description></x:xmpmeta>"#);
        assert_eq!((m.rating, m.flag, m.color_label), (None, Some(-1), None));
        assert!(marks_in(b"plain jpeg bytes").is_empty());
        assert_eq!(marks_in(b"<x:xmpmeta><rdf:Description xmp:Rating=\"0\"/></x:xmpmeta>").rating, None, "0 stars is no mark");
    }

    #[test]
    fn a_file_without_a_packet_has_no_date() {
        assert_eq!(capture_date_in(b"\xff\xd8\xff\xe0 plain jpeg bytes"), None);
    }
}
