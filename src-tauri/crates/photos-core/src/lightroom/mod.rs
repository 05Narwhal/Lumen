//! Lightroom → Lumen (Round 11): Camera Raw settings (`crs:` names) read from XMP presets and
//! sidecars, `.lrtemplate` presets and a catalog's develop text, and converted into a Lumen
//! `Develop`. Pure: no I/O beyond the string each reader is given.
//!
//! The mapping is the Round 11 spec's table (`docs/superpowers/specs/2026-10-06-round-11-design.md`
//! §11A); `tests/lightroom-golden.json` pins it, and Session 11B's training scripts read the
//! same JSON through `examples/lr2lumen.rs`.

mod lua;
mod map;
mod xmp;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::develop::Panel;
use crate::edits::{Crop, Develop};

pub use lua::{read_lrtemplate, read_lua_table};
pub use map::convert;
pub use xmp::read_xmp;

/// The settings in a file, read by its extension: `.xmp`, `.lrtemplate`, else a Lua table (a
/// catalog's develop text).
pub fn read_settings(file_name: &str, text: &str) -> crate::Result<Settings> {
    let ext = file_name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default();
    match ext.as_str() {
        "xmp" => read_xmp(text),
        "lrtemplate" => read_lrtemplate(text),
        _ => read_lua_table(text),
    }
}

/// Camera Raw settings by name, without the `crs:` prefix (`Exposure2012`, `ToneCurvePV2012`…).
pub type Settings = std::collections::BTreeMap<String, Value>;

/// A setting's value as the file wrote it. XMP writes every scalar as text; the readers turn
/// numbers and booleans into `Num` and `Bool`, so the mapping reads both formats alike.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Value {
    Num(f64),
    Str(String),
    Bool(bool),
    List(Vec<Value>),
    Table(Settings),
}

impl Value {
    /// A number, or text that reads as one (`"+1.00"`).
    pub fn num(&self) -> Option<f64> {
        match self {
            Value::Num(n) => Some(*n),
            Value::Str(s) => s.trim().trim_start_matches('+').parse().ok(),
            Value::Bool(b) => Some(if *b { 1.0 } else { 0.0 }),
            _ => None,
        }
    }

    /// A boolean, or text / a number that reads as one (`"True"`, `1`).
    pub fn bool(&self) -> Option<bool> {
        match self {
            Value::Bool(b) => Some(*b),
            Value::Num(n) => Some(*n != 0.0),
            Value::Str(s) => match s.trim().to_ascii_lowercase().as_str() {
                "true" | "1" => Some(true),
                "false" | "0" => Some(false),
                _ => None,
            },
            _ => None,
        }
    }

    pub fn str(&self) -> Option<&str> {
        match self {
            Value::Str(s) => Some(s),
            _ => None,
        }
    }

    /// What XMP writes as text, as a number or a boolean when it reads as one.
    pub(crate) fn from_text(text: &str) -> Value {
        let t = text.trim();
        match t {
            "True" | "true" => return Value::Bool(true),
            "False" | "false" => return Value::Bool(false),
            _ => {}
        }
        let digits = t.strip_prefix('+').unwrap_or(t);
        if !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit() || matches!(b, b'.' | b'-' | b'e' | b'E')) {
            if let Ok(n) = digits.parse::<f64>() {
                return Value::Num(n);
            }
        }
        Value::Str(text.to_string())
    }
}

/// The crop and straighten a Lightroom edit sets, in Lumen's terms: the crop as shares of the
/// unrotated photo, the straighten in degrees.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Geometry {
    pub crop: Option<Crop>,
    pub straighten: f32,
}

/// One preset or one photo's settings, converted. `panels` lists the Develop panels the
/// settings touched, so a preset sets only those; `skipped` names what has no Lumen
/// equivalent (profiles, lens and transform corrections, brush and AI masks…) or was only
/// approximated.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Converted {
    pub develop: Develop,
    pub geometry: Option<Geometry>,
    pub panels: Vec<Panel>,
    pub skipped: Vec<String>,
    /// `crs:Name` (an XMP preset) or the `.lrtemplate`'s title.
    pub name: Option<String>,
    /// `crs:Group`: the preset's folder in Lightroom.
    pub group: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_reads_as_numbers_and_booleans() {
        assert_eq!(Value::from_text("+1.00"), Value::Num(1.0));
        assert_eq!(Value::from_text("-30"), Value::Num(-30.0));
        assert_eq!(Value::from_text("True"), Value::Bool(true));
        assert_eq!(Value::from_text("Adobe Standard"), Value::Str("Adobe Standard".into()));
        assert_eq!(Value::from_text("0, 0"), Value::Str("0, 0".into()));
        assert_eq!(Value::Str("+0.5".into()).num(), Some(0.5));
        assert_eq!(Value::Str("False".into()).bool(), Some(false));
    }
}
