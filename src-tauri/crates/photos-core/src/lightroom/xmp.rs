//! Camera Raw settings from XMP: a preset (`.xmp` in Camera Raw's Settings folder) or a sidecar
//! Lightroom wrote beside a photo. Only the `crs:` keys are read, from attributes and elements,
//! with `rdf:Seq` / `rdf:Bag` as lists, `rdf:Alt` as its first value, and nested descriptions
//! (masks) as tables.

use super::{Settings, Value};
use crate::{CoreError, Result};

pub fn read_xmp(text: &str) -> Result<Settings> {
    let root = parse(text)?;
    let mut out = Settings::new();
    descriptions(&root, &mut out);
    Ok(out)
}

/// Every top-level `rdf:Description`'s `crs:` settings. Descriptions inside a `crs:` property
/// (a mask) belong to that property's value.
fn descriptions(node: &Element, out: &mut Settings) {
    if node.name == "rdf:Description" {
        out.extend(table(node));
        return;
    }
    for child in &node.children {
        if !child.name.starts_with("crs:") {
            descriptions(child, out);
        }
    }
}

fn crs(name: &str) -> Option<&str> {
    name.strip_prefix("crs:")
}

/// A node's `crs:` attributes and `crs:` child properties.
fn table(node: &Element) -> Settings {
    let mut t = Settings::new();
    for (k, v) in &node.attrs {
        if let Some(k) = crs(k) {
            t.insert(k.to_string(), Value::from_text(v));
        }
    }
    for child in &node.children {
        if let Some(k) = crs(&child.name) {
            t.insert(k.to_string(), property(child));
        } else if child.name == "rdf:Description" {
            t.extend(table(child));
        }
    }
    t
}

/// A property element's (or an `rdf:li`'s) value.
fn property(node: &Element) -> Value {
    for child in &node.children {
        match child.name.as_str() {
            "rdf:Seq" | "rdf:Bag" => return Value::List(child.children.iter().filter(|c| c.name == "rdf:li").map(property).collect()),
            "rdf:Alt" => return child.children.iter().find(|c| c.name == "rdf:li").map(property).unwrap_or(Value::Str(String::new())),
            _ => {}
        }
    }
    let structured = node.attrs.iter().any(|(k, _)| k.starts_with("crs:")) || node.children.iter().any(|c| c.name == "rdf:Description" || c.name.starts_with("crs:"));
    if structured {
        Value::Table(table(node))
    } else {
        Value::from_text(&node.text)
    }
}

/// An XML element: its qualified name, attributes, child elements and the text directly in it.
#[derive(Debug, Default)]
struct Element {
    name: String,
    attrs: Vec<(String, String)>,
    children: Vec<Element>,
    text: String,
}

fn invalid(what: &str, at: usize) -> CoreError {
    CoreError::Invalid(format!("XMP: {what} at byte {at}"))
}

/// Just enough XML for XMP: elements, attributes, text, the five entities and character
/// references, comments, processing instructions, a doctype and CDATA.
fn parse(text: &str) -> Result<Element> {
    let b = text.as_bytes();
    let mut i = 0;
    let mut stack: Vec<Element> = vec![Element::default()];
    let mut root_seen = false;
    while i < b.len() {
        if b[i] != b'<' {
            let end = b[i..].iter().position(|c| *c == b'<').map_or(b.len(), |p| i + p);
            let chunk = unescape(&text[i..end], i)?;
            if stack.len() == 1 {
                if !chunk.trim().is_empty() {
                    return Err(invalid("text outside the root", i));
                }
            } else if let Some(top) = stack.last_mut() {
                top.text.push_str(&chunk);
            }
            i = end;
            continue;
        }
        let rest = &text[i..];
        if let Some(r) = rest.strip_prefix("<!--") {
            i += 4 + r.find("-->").ok_or_else(|| invalid("an unclosed comment", i))? + 3;
        } else if let Some(r) = rest.strip_prefix("<![CDATA[") {
            let end = r.find("]]>").ok_or_else(|| invalid("an unclosed CDATA", i))?;
            if stack.len() > 1 {
                if let Some(top) = stack.last_mut() {
                    top.text.push_str(&r[..end]);
                }
            }
            i += 9 + end + 3;
        } else if rest.starts_with("<?") {
            i += rest.find("?>").ok_or_else(|| invalid("an unclosed declaration", i))? + 2;
        } else if rest.starts_with("<!") {
            i += rest.find('>').ok_or_else(|| invalid("an unclosed doctype", i))? + 1;
        } else if let Some(r) = rest.strip_prefix("</") {
            let end = r.find('>').ok_or_else(|| invalid("an unclosed end tag", i))?;
            let name = r[..end].trim();
            let done = stack.pop().filter(|e| e.name == name && !stack.is_empty()).ok_or_else(|| invalid("a mismatched end tag", i))?;
            if let Some(parent) = stack.last_mut() {
                parent.children.push(done);
            }
            i += 2 + end + 1;
        } else {
            let (el, closed, len) = start_tag(text, i)?;
            if stack.len() == 1 {
                if root_seen {
                    return Err(invalid("a second root", i));
                }
                root_seen = true;
            }
            if closed {
                if let Some(parent) = stack.last_mut() {
                    parent.children.push(el);
                }
            } else {
                stack.push(el);
            }
            i += len;
        }
    }
    if stack.len() != 1 {
        return Err(invalid("an unclosed element", b.len()));
    }
    stack.pop().and_then(|mut doc| doc.children.pop()).ok_or_else(|| invalid("no root element", 0))
}

/// `<name a="1" b='2'>` or `<name/>` at `at`: the element, whether it closed itself, its length.
fn start_tag(text: &str, at: usize) -> Result<(Element, bool, usize)> {
    let b = text.as_bytes();
    let mut i = at + 1;
    let name_end = |i: usize| b[i..].iter().position(|c| c.is_ascii_whitespace() || matches!(c, b'>' | b'/')).map(|p| i + p);
    let end = name_end(i).ok_or_else(|| invalid("an unclosed tag", at))?;
    let name = &text[i..end];
    if name.is_empty() {
        return Err(invalid("a tag without a name", at));
    }
    let mut el = Element { name: name.to_string(), ..Element::default() };
    i = end;
    loop {
        while i < b.len() && b[i].is_ascii_whitespace() {
            i += 1;
        }
        match b.get(i) {
            None => return Err(invalid("an unclosed tag", at)),
            Some(b'>') => return Ok((el, false, i + 1 - at)),
            Some(b'/') if b.get(i + 1) == Some(&b'>') => return Ok((el, true, i + 2 - at)),
            _ => {}
        }
        let eq = b[i..].iter().position(|c| *c == b'=' || *c == b'>').map(|p| i + p).filter(|p| b[*p] == b'=').ok_or_else(|| invalid("an attribute without a value", i))?;
        let key = text[i..eq].trim().to_string();
        let mut j = eq + 1;
        while j < b.len() && b[j].is_ascii_whitespace() {
            j += 1;
        }
        let quote = *b.get(j).filter(|q| matches!(q, b'"' | b'\'')).ok_or_else(|| invalid("an unquoted attribute", j))?;
        let close = b[j + 1..].iter().position(|c| *c == quote).map(|p| j + 1 + p).ok_or_else(|| invalid("an unclosed attribute", j))?;
        el.attrs.push((key, unescape(&text[j + 1..close], j + 1)?));
        i = close + 1;
    }
}

fn unescape(s: &str, at: usize) -> Result<String> {
    if !s.contains('&') {
        return Ok(s.to_string());
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(p) = rest.find('&') {
        out.push_str(&rest[..p]);
        let after = &rest[p + 1..];
        let end = after.find(';').ok_or_else(|| invalid("an unclosed entity", at + p))?;
        let entity = &after[..end];
        let ch = match entity {
            "amp" => '&',
            "lt" => '<',
            "gt" => '>',
            "quot" => '"',
            "apos" => '\'',
            _ => {
                let code = if let Some(hex) = entity.strip_prefix("#x").or_else(|| entity.strip_prefix("#X")) {
                    u32::from_str_radix(hex, 16).ok()
                } else {
                    entity.strip_prefix('#').and_then(|d| d.parse().ok())
                };
                code.and_then(char::from_u32).ok_or_else(|| invalid("an unknown entity", at + p))?
            }
        };
        out.push(ch);
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const PRESET_ATTRIBUTES: &str = r#"<x:xmpmeta xmlns:x="adobe:ns:meta/" x:xmptk="Adobe XMP Core 7.0-c000 1.000000, 0000/00/00-00:00:00        ">
 <rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
  <rdf:Description rdf:about=""
    xmlns:crs="http://ns.adobe.com/camera-raw-settings/1.0/"
   crs:PresetType="Normal"
   crs:Cluster=""
   crs:UUID="3B2F0000000000000000000000000001"
   crs:SupportsAmount="False"
   crs:Version="15.0"
   crs:ProcessVersion="11.0"
   crs:Exposure2012="+0.50"
   crs:Contrast2012="-12"
   crs:HasSettings="True">
   <crs:Name>
    <rdf:Alt>
     <rdf:li xml:lang="x-default">Soft &amp; Warm</rdf:li>
    </rdf:Alt>
   </crs:Name>
   <crs:Group>
    <rdf:Alt>
     <rdf:li xml:lang="x-default">Portrait</rdf:li>
    </rdf:Alt>
   </crs:Group>
   <crs:ToneCurvePV2012>
    <rdf:Seq>
     <rdf:li>0, 0</rdf:li>
     <rdf:li>64, 50</rdf:li>
     <rdf:li>255, 255</rdf:li>
    </rdf:Seq>
   </crs:ToneCurvePV2012>
  </rdf:Description>
 </rdf:RDF>
</x:xmpmeta>
"#;

    #[test]
    fn reads_a_presets_attributes_and_elements() {
        let s = read_xmp(PRESET_ATTRIBUTES).unwrap();
        assert_eq!(s["Exposure2012"], Value::Num(0.5));
        assert_eq!(s["Contrast2012"], Value::Num(-12.0));
        assert_eq!(s["HasSettings"], Value::Bool(true));
        assert_eq!(s["Name"], Value::Str("Soft & Warm".into()));
        assert_eq!(s["Group"], Value::Str("Portrait".into()));
        assert_eq!(s["ToneCurvePV2012"], Value::List(vec![Value::Str("0, 0".into()), Value::Str("64, 50".into()), Value::Str("255, 255".into())]));
        assert!(!s.contains_key("about"), "only crs: keys");
    }

    #[test]
    fn reads_settings_written_as_elements() {
        let s = read_xmp(
            r#"<?xpacket begin="" id="W5M0MpCehiHzreSzNTczkc9d"?>
<x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
<rdf:Description rdf:about="" xmlns:crs="http://ns.adobe.com/camera-raw-settings/1.0/">
<crs:Exposure2012>-1.25</crs:Exposure2012>
<crs:Vibrance>+20</crs:Vibrance>
</rdf:Description></rdf:RDF></x:xmpmeta>
<?xpacket end="w"?>"#,
        )
        .unwrap();
        assert_eq!(s["Exposure2012"], Value::Num(-1.25));
        assert_eq!(s["Vibrance"], Value::Num(20.0));
    }

    #[test]
    fn a_lightroom_classic_sidecar_gives_only_its_crs_keys_and_its_masks() {
        let s = read_xmp(
            r#"<x:xmpmeta xmlns:x="adobe:ns:meta/" x:xmptk="Adobe XMP Core 7.0-c000">
 <rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
  <rdf:Description rdf:about=""
    xmlns:xmp="http://ns.adobe.com/xap/1.0/"
    xmlns:tiff="http://ns.adobe.com/tiff/1.0/"
    xmlns:exif="http://ns.adobe.com/exif/1.0/"
    xmlns:dc="http://purl.org/dc/elements/1.1/"
    xmlns:photoshop="http://ns.adobe.com/photoshop/1.0/"
    xmlns:crs="http://ns.adobe.com/camera-raw-settings/1.0/"
   xmp:CreatorTool="Adobe Photoshop Lightroom Classic 13.0 (Windows)"
   xmp:Rating="4"
   tiff:Make="Canon"
   exif:ExposureTime="1/250"
   photoshop:DateCreated="2024-06-01T10:00:00"
   crs:RawFileName="IMG_0001.CR3"
   crs:WhiteBalance="As Shot"
   crs:Temperature="5250"
   crs:Exposure2012="+0.30">
   <dc:subject>
    <rdf:Bag>
     <rdf:li>beach</rdf:li>
    </rdf:Bag>
   </dc:subject>
   <crs:GradientBasedCorrections>
    <rdf:Seq>
     <rdf:li>
      <rdf:Description
       crs:What="Correction"
       crs:CorrectionAmount="1.000000"
       crs:CorrectionActive="true"
       crs:LocalExposure2012="-0.500000">
      <crs:CorrectionMasks>
       <rdf:Seq>
        <rdf:li
         crs:What="Mask/Gradient"
         crs:MaskValue="1.000000"
         crs:ZeroX="0.5"
         crs:ZeroY="0.6"
         crs:FullX="0.5"
         crs:FullY="0.2"/>
       </rdf:Seq>
      </crs:CorrectionMasks>
      </rdf:Description>
     </rdf:li>
    </rdf:Seq>
   </crs:GradientBasedCorrections>
  </rdf:Description>
 </rdf:RDF>
</x:xmpmeta>"#,
        )
        .unwrap();
        let keys: Vec<&str> = s.keys().map(String::as_str).collect();
        assert_eq!(keys, vec!["Exposure2012", "GradientBasedCorrections", "RawFileName", "Temperature", "WhiteBalance"]);
        let Value::List(corrections) = &s["GradientBasedCorrections"] else { panic!("{:?}", s["GradientBasedCorrections"]) };
        let Value::Table(c) = &corrections[0] else { panic!() };
        assert_eq!(c["LocalExposure2012"], Value::Num(-0.5));
        let Value::List(masks) = &c["CorrectionMasks"] else { panic!() };
        let Value::Table(m) = &masks[0] else { panic!() };
        assert_eq!(m["What"], Value::Str("Mask/Gradient".into()));
        assert_eq!(m["FullY"], Value::Num(0.2));
    }

    #[test]
    fn malformed_xml_is_an_error_with_a_stable_code() {
        for bad in ["", "not xml", "<x:xmpmeta><rdf:RDF>", "<a crs:X=\"1></a>", "<a></b>", "<crs:Exposure2012>1</crs:Exposure2012", "<a>&bogus;</a>"] {
            let e = read_xmp(bad).unwrap_err();
            assert_eq!(e.code(), "invalid", "{bad:?}");
        }
    }
}
