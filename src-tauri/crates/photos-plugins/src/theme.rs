//! Plugin themes (spec §6): one token table, typed values, the CSS Rust generates and the contrast
//! check. A theme file never carries CSS; every value is parsed here and written back by us.

use std::collections::BTreeMap;

use photos_core::ThemeId;

use crate::package::{KindCheck, Report, Unpacked};
use crate::PluginKind;

/// What a token's value may be.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TokenType {
    Colour,
    /// `px`, within the range.
    Length {
        min: f32,
        max: f32,
    },
    Shadow,
    Wash,
    /// The heading font family: only the base theme or `[font]` sets it.
    Font,
    /// The heading weight, 100–900: set by `[font]`.
    Weight,
    /// The heading style, `normal` or `italic`: set by `[font]`.
    Style,
}

const fn len(min: f32, max: f32) -> TokenType {
    TokenType::Length { min, max }
}

/// Every custom property the four built-in partials set, once (checked against the SCSS by a test).
pub const TOKENS: &[(&str, TokenType)] = &[
    ("bg-base", TokenType::Colour),
    ("bg-raised", TokenType::Colour),
    ("bg-sunken", TokenType::Colour),
    ("develop-stage-bg", TokenType::Colour),
    ("bg-sidebar", TokenType::Colour),
    ("bg-overlay", TokenType::Colour),
    ("fg-primary", TokenType::Colour),
    ("fg-muted", TokenType::Colour),
    ("fg-subtle", TokenType::Colour),
    ("fg-faint", TokenType::Colour),
    ("accent", TokenType::Colour),
    ("accent-hover", TokenType::Colour),
    ("accent-fg", TokenType::Colour),
    ("accent-soft", TokenType::Colour),
    ("border", TokenType::Colour),
    ("border-width", len(0.0, 3.0)),
    ("scrollbar-thumb", TokenType::Colour),
    ("radius-sm", len(0.0, 32.0)),
    ("radius-md", len(0.0, 32.0)),
    ("radius-lg", len(0.0, 32.0)),
    // The pill is "as round as it goes"; every built-in theme uses 999px.
    ("radius-pill", len(0.0, 999.0)),
    ("tile-gap", len(0.0, 24.0)),
    ("shadow-sm", TokenType::Shadow),
    ("shadow-md", TokenType::Shadow),
    ("shadow-lg", TokenType::Shadow),
    ("blur", len(0.0, 40.0)),
    ("font-heading", TokenType::Font),
    ("heading-weight", TokenType::Weight),
    ("heading-style", TokenType::Style),
    ("wash", TokenType::Wash),
];

/// The type of a token, `None` when the table has no such token.
pub fn token_type(name: &str) -> Option<TokenType> {
    TOKENS.iter().find(|(n, _)| *n == name).map(|(_, t)| *t)
}

/// A colour in the forms a theme may spell: kept as the CSS we write back and as linear sRGB for
/// the contrast maths.
#[derive(Debug, Clone, PartialEq)]
pub struct Colour {
    pub css: String,
    /// Linear sRGB, clamped to the gamut.
    pub lin: [f32; 3],
    pub alpha: f32,
    /// `[L (0–1), C, h (degrees)]` for colours written in oklch, so derived colours keep precision.
    pub ok: Option<[f32; 3]>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ShadowLayer {
    pub x: f32,
    pub y: f32,
    pub blur: f32,
    pub colour: Colour,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Radial {
    pub w: f32,
    pub h: f32,
    pub x: f32,
    pub y: f32,
    pub from: Colour,
    pub from_at: f32,
    pub to_at: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Colour(Colour),
    Length(f32),
    Shadow(Vec<ShadowLayer>),
    /// Empty: `none`.
    Wash(Vec<Radial>),
    /// A font family copied from a built-in theme.
    Raw(String),
    Weight(u16),
    Style(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct FontDecl {
    pub file: String,
    pub weight: u16,
    pub style: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ThemeFile {
    pub id: String,
    pub name: String,
    pub tagline: String,
    pub base: ThemeId,
    pub default_scheme: String,
    pub light: BTreeMap<String, Value>,
    pub dark: BTreeMap<String, Value>,
    pub font: Option<FontDecl>,
}

// ---------------------------------------------------------------------------------------------
// Numbers and splitting

/// A number as it reads in CSS: four decimals at most, no `-0`, no trailing zeros.
fn fmt(x: f32) -> String {
    let r = (x * 10000.0).round() / 10000.0;
    if r == 0.0 {
        "0".into()
    } else {
        format!("{r}")
    }
}

fn num(s: &str) -> Option<f32> {
    s.parse::<f32>().ok().filter(|v| v.is_finite())
}

/// Splits on `sep` outside parentheses.
fn split_top(s: &str, sep: char) -> Vec<&str> {
    let (mut out, mut depth, mut start) = (Vec::new(), 0i32, 0);
    for (i, c) in s.char_indices() {
        match c {
            '(' => depth += 1,
            ')' => depth -= 1,
            c if c == sep && depth == 0 => {
                out.push(s[start..i].trim());
                start = i + c.len_utf8();
            }
            _ => {}
        }
    }
    out.push(s[start..].trim());
    out
}

/// Splits on whitespace outside parentheses.
fn split_ws(s: &str) -> Vec<&str> {
    let (mut out, mut depth, mut start) = (Vec::new(), 0i32, None::<usize>);
    for (i, c) in s.char_indices() {
        match c {
            '(' => depth += 1,
            ')' => depth -= 1,
            _ => {}
        }
        if c.is_whitespace() && depth == 0 {
            if let Some(st) = start.take() {
                out.push(&s[st..i]);
            }
        } else if start.is_none() {
            start = Some(i);
        }
    }
    if let Some(st) = start {
        out.push(&s[st..]);
    }
    out
}

/// A theme value is never CSS: nothing that could end a declaration, open a rule, fetch a file or
/// reach another variable gets past this.
fn no_raw_css(s: &str) -> Result<(), String> {
    let lower = s.to_ascii_lowercase();
    if s.contains([';', '{', '}', '<', '>', '"', '\'', '\\', '@', '`'])
        || lower.contains("url(")
        || lower.contains("var(")
        || lower.contains("/*")
        || lower.contains("expression")
        || lower.contains("image(")
        || lower.contains("image-set(")
    {
        return Err("raw CSS isn't allowed here".into());
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// Colours

const COLOUR_HINT: &str = "use a colour value (#hex, rgb(), hsl() or oklch())";

fn srgb_decode(c: f32) -> f32 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

/// `[L (0–1), C, h (degrees)]` → linear sRGB, clamped to the gamut (the TS test's maths).
#[allow(clippy::excessive_precision)] // Björn Ottosson's published constants, as `contrast.test.ts` has them
fn oklch_to_linear(l: f32, c: f32, h: f32) -> [f32; 3] {
    let hr = h.to_radians();
    let (a, b) = (c * hr.cos(), c * hr.sin());
    let l_ = (l + 0.3963377774 * a + 0.2158037573 * b).powi(3);
    let m_ = (l - 0.1055613458 * a - 0.0638541728 * b).powi(3);
    let s_ = (l - 0.0894841775 * a - 1.291485548 * b).powi(3);
    [
        4.0767416621 * l_ - 3.3077115913 * m_ + 0.2309699292 * s_,
        -1.2684380046 * l_ + 2.6097574011 * m_ - 0.3413193965 * s_,
        -0.0041960863 * l_ - 0.7034186147 * m_ + 1.707614701 * s_,
    ]
    .map(|v| v.clamp(0.0, 1.0))
}

#[allow(clippy::excessive_precision)] // Björn Ottosson's published constants, as `contrast.test.ts` has them
fn linear_to_oklch(rgb: [f32; 3]) -> [f32; 3] {
    let [r, g, b] = rgb;
    let l = (0.4122214708 * r + 0.5363325363 * g + 0.0514459929 * b).cbrt();
    let m = (0.2119034982 * r + 0.6806995451 * g + 0.1073969566 * b).cbrt();
    let s = (0.0883024619 * r + 0.2817188376 * g + 0.6299787005 * b).cbrt();
    let ll = 0.2104542553 * l + 0.793617785 * m - 0.0040720468 * s;
    let a = 1.9779984951 * l - 2.428592205 * m + 0.4505937099 * s;
    let bb = 0.0259040371 * l + 0.7827717662 * m - 0.808675766 * s;
    [ll, a.hypot(bb), bb.atan2(a).to_degrees().rem_euclid(360.0)]
}

fn alpha_suffix(a: f32) -> String {
    if a < 1.0 {
        format!(" / {}", fmt(a))
    } else {
        String::new()
    }
}

impl Colour {
    /// An oklch colour from its parts (`l` 0–1), written the way the partials write them.
    pub fn from_oklch(l: f32, c: f32, h: f32, alpha: f32) -> Colour {
        // Plain non-negative numbers, the hue within 0–360: the page reads `--bg-base` back with
        // `oklchToRgb` (src/lib/oklch.ts) to paint the native window, and a hue of `-10` or `1e30`
        // would be refused there (or written as `inf`).
        let (l, c) = (l.clamp(0.0, 1.0), c.max(0.0));
        let h = if fmt(c) == "0" {
            0.0
        } else {
            h.rem_euclid(360.0)
        };
        Colour {
            css: format!(
                "oklch({}% {} {}{})",
                fmt(l * 100.0),
                fmt(c),
                fmt(h),
                alpha_suffix(alpha)
            ),
            lin: oklch_to_linear(l, c, h),
            alpha,
            ok: Some([l, c, h]),
        }
    }

    /// `[L, C, h]`, exact for a colour written in oklch.
    pub fn oklch(&self) -> [f32; 3] {
        self.ok.unwrap_or_else(|| linear_to_oklch(self.lin))
    }

    /// An sRGB colour, written as oklch like every other (its linear value stays exact).
    fn from_srgb(rgb: [f32; 3], alpha: f32) -> Colour {
        let lin = rgb.map(|c| srgb_decode(c.clamp(0.0, 1.0)));
        let [l, c, h] = linear_to_oklch(lin);
        Colour {
            lin,
            ..Colour::from_oklch(l, c, h, alpha)
        }
    }
}

fn parse_alpha(s: &str) -> Result<f32, String> {
    let a = match s.strip_suffix('%') {
        Some(p) => num(p).map(|v| v / 100.0),
        None => num(s),
    };
    match a {
        Some(a) if (0.0..=1.0).contains(&a) => Ok(a),
        _ => Err("the alpha must be 0–1 (or 0–100%)".into()),
    }
}

fn parse_hex(h: &str) -> Option<Colour> {
    if !h.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let h = h.to_ascii_lowercase();
    let full: String = match h.len() {
        3 | 4 => h.chars().flat_map(|c| [c, c]).collect(),
        6 | 8 => h.clone(),
        _ => return None,
    };
    let byte = |i: usize| u8::from_str_radix(&full[i * 2..i * 2 + 2], 16).ok();
    let (r, g, b) = (byte(0)?, byte(1)?, byte(2)?);
    let alpha = if full.len() == 8 {
        f32::from(byte(3)?) / 255.0
    } else {
        1.0
    };
    Some(Colour::from_srgb(
        [
            f32::from(r) / 255.0,
            f32::from(g) / 255.0,
            f32::from(b) / 255.0,
        ],
        alpha,
    ))
}

fn hsl_to_rgb(h: f32, s: f32, l: f32) -> [f32; 3] {
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let hp = h.rem_euclid(360.0) / 60.0;
    let x = c * (1.0 - (hp % 2.0 - 1.0).abs());
    let (r, g, b) = match hp as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = l - c / 2.0;
    [r + m, g + m, b + m]
}

/// `#hex`, `rgb()`, `hsl()` or `oklch()`, with alpha.
pub fn parse_colour(s: &str) -> Result<Colour, String> {
    let s = s.trim();
    let hint = || COLOUR_HINT.to_string();
    if let Some(h) = s.strip_prefix('#') {
        return parse_hex(h).ok_or_else(hint);
    }
    let open = s.find('(').ok_or_else(hint)?;
    if !s.ends_with(')') {
        return Err(hint());
    }
    let name = s[..open].trim().to_ascii_lowercase();
    let inner = &s[open + 1..s.len() - 1];
    if inner.contains(['(', ')']) {
        return Err(hint());
    }
    let (main, mut alpha_s) = match inner.split_once('/') {
        Some((m, a)) => (m, Some(a.trim())),
        None => (inner, None),
    };
    let mut parts: Vec<&str> = main
        .split(|c: char| c == ',' || c.is_whitespace())
        .filter(|p| !p.is_empty())
        .collect();
    if alpha_s.is_none() && parts.len() == 4 {
        alpha_s = parts.pop();
    }
    if parts.len() != 3 {
        return Err(hint());
    }
    let alpha = alpha_s.map_or(Ok(1.0), parse_alpha)?;
    let deg = |p: &str| num(p.strip_suffix("deg").unwrap_or(p));
    match name.as_str() {
        "rgb" | "rgba" => {
            let mut c = [0.0f32; 3];
            for (i, p) in parts.iter().enumerate() {
                let v = match p.strip_suffix('%') {
                    Some(pc) => num(pc).map(|v| v * 2.55),
                    None => num(p),
                }
                .ok_or_else(hint)?;
                c[i] = v.clamp(0.0, 255.0).round();
            }
            Ok(Colour::from_srgb(c.map(|v| v / 255.0), alpha))
        }
        "hsl" | "hsla" => {
            let h = deg(parts[0]).ok_or_else(hint)?;
            let pct = |p: &str| {
                p.strip_suffix('%')
                    .and_then(num)
                    .filter(|v| (0.0..=100.0).contains(v))
            };
            let (sat, lig) = (
                pct(parts[1]).ok_or_else(hint)?,
                pct(parts[2]).ok_or_else(hint)?,
            );
            Ok(Colour::from_srgb(
                hsl_to_rgb(h, sat / 100.0, lig / 100.0),
                alpha,
            ))
        }
        "oklch" => {
            let l = match parts[0].strip_suffix('%') {
                Some(p) => num(p).map(|v| v / 100.0),
                None => num(parts[0]),
            }
            .filter(|v| (0.0..=1.0).contains(v))
            .ok_or_else(hint)?;
            let c = match parts[1].strip_suffix('%') {
                Some(p) => num(p).map(|v| v * 0.004),
                None => num(parts[1]),
            }
            .filter(|v| (0.0..=0.5).contains(v))
            .ok_or_else(hint)?;
            let h = deg(parts[2]).ok_or_else(hint)?;
            Ok(Colour::from_oklch(l, c, h, alpha))
        }
        _ => Err(hint()),
    }
}

// ---------------------------------------------------------------------------------------------
// Lengths, shadows and the wash

fn px(s: &str) -> Option<f32> {
    if s == "0" {
        return Some(0.0);
    }
    s.strip_suffix("px").and_then(num)
}

fn length_css(x: f32) -> String {
    format!("{}px", fmt(x))
}

/// A shadow offset or blur: `0` or `<n>px`.
fn shadow_len_css(x: f32) -> String {
    if x == 0.0 {
        "0".into()
    } else {
        length_css(x)
    }
}

fn parse_length(s: &str, min: f32, max: f32) -> Result<Value, String> {
    let v = px(s).ok_or("use a length in px, like 12px")?;
    if !(min..=max).contains(&v) {
        return Err(format!("{s} is outside {}–{}px", fmt(min), fmt(max)));
    }
    Ok(Value::Length(v))
}

fn parse_shadow(s: &str) -> Result<Value, String> {
    if s == "none" {
        return Ok(Value::Shadow(vec![]));
    }
    let layers = split_top(s, ',');
    if layers.len() > 3 {
        return Err("a shadow has at most 3 layers".into());
    }
    let mut out = Vec::new();
    for l in layers {
        let parts = split_ws(l);
        let [x, y, blur, colour] = parts[..] else {
            return Err("a shadow layer is: x y blur colour (like 0 6px 16px #0003)".into());
        };
        let (x, y, blur) = (
            px(x).filter(|v| v.abs() <= 100.0),
            px(y).filter(|v| v.abs() <= 100.0),
            px(blur).filter(|v| (0.0..=100.0).contains(v)),
        );
        let (Some(x), Some(y), Some(blur)) = (x, y, blur) else {
            return Err("a shadow's offsets are within ±100px and its blur 0–100px".into());
        };
        out.push(ShadowLayer {
            x,
            y,
            blur,
            colour: parse_colour(colour)?,
        });
    }
    Ok(Value::Shadow(out))
}

fn parse_wash(s: &str) -> Result<Value, String> {
    if s == "none" {
        return Ok(Value::Wash(vec![]));
    }
    let layers = split_top(s, ',');
    if layers.len() > 3 {
        return Err("a wash has at most 3 gradients".into());
    }
    let shape =
        "a wash gradient is: radial-gradient(60% 50% at 85% 0%, <colour> 0%, transparent 60%)";
    let pct = |p: &str, lo: f32, hi: f32| {
        p.strip_suffix('%')
            .and_then(num)
            .filter(|v| (lo..=hi).contains(v))
    };
    let mut out = Vec::new();
    for l in layers {
        let inner = l
            .strip_prefix("radial-gradient(")
            .and_then(|r| r.strip_suffix(')'))
            .ok_or(shape)?;
        let args = split_top(inner, ',');
        let [geo, from, to] = args[..] else {
            return Err(shape.into());
        };
        let g = split_ws(geo);
        let [w, h, "at", x, y] = g[..] else {
            return Err(shape.into());
        };
        let from_parts = split_ws(from);
        let from_at = from_parts.last().ok_or(shape)?;
        let to_parts = split_ws(to);
        let ["transparent", to_at] = to_parts[..] else {
            return Err(shape.into());
        };
        let bad =
            || "a wash's sizes are 0–200%, its centre −50–150% and its stops 0–100%".to_string();
        out.push(Radial {
            w: pct(w, 0.0, 200.0).ok_or_else(bad)?,
            h: pct(h, 0.0, 200.0).ok_or_else(bad)?,
            x: pct(x, -50.0, 150.0).ok_or_else(bad)?,
            y: pct(y, -50.0, 150.0).ok_or_else(bad)?,
            from: parse_colour(&from_parts[..from_parts.len() - 1].join(" "))?,
            from_at: pct(from_at, 0.0, 100.0).ok_or_else(bad)?,
            to_at: pct(to_at, 0.0, 100.0).ok_or_else(bad)?,
        });
    }
    Ok(Value::Wash(out))
}

impl Value {
    /// The CSS this value is written as.
    pub fn css(&self) -> String {
        match self {
            Value::Colour(c) => c.css.clone(),
            Value::Length(x) => length_css(*x),
            Value::Shadow(layers) if layers.is_empty() => "none".into(),
            Value::Shadow(layers) => layers
                .iter()
                .map(|l| {
                    format!(
                        "{} {} {} {}",
                        shadow_len_css(l.x),
                        shadow_len_css(l.y),
                        shadow_len_css(l.blur),
                        l.colour.css
                    )
                })
                .collect::<Vec<_>>()
                .join(", "),
            Value::Wash(r) if r.is_empty() => "none".into(),
            Value::Wash(r) => r
                .iter()
                .map(|r| {
                    format!(
                        "radial-gradient({}% {}% at {}% {}%, {} {}%, transparent {}%)",
                        fmt(r.w),
                        fmt(r.h),
                        fmt(r.x),
                        fmt(r.y),
                        r.from.css,
                        fmt(r.from_at),
                        fmt(r.to_at)
                    )
                })
                .collect::<Vec<_>>()
                .join(", "),
            Value::Raw(s) | Value::Style(s) => s.clone(),
            Value::Weight(w) => w.to_string(),
        }
    }
}

/// A value a theme file may set. Fonts, weight and style are not among them (they come from
/// `[font]`).
pub fn parse_value(tok: TokenType, s: &str) -> Result<Value, String> {
    let s = s.trim();
    no_raw_css(s)?;
    match tok {
        TokenType::Colour => parse_colour(s).map(Value::Colour),
        TokenType::Length { min, max } => parse_length(s, min, max),
        TokenType::Shadow => parse_shadow(s),
        TokenType::Wash => parse_wash(s),
        TokenType::Font | TokenType::Weight | TokenType::Style => {
            Err("set the heading font in [font]".into())
        }
    }
}

/// A value from a built-in partial: the same parser, plus the heading font, weight and style.
pub fn parse_base_value(name: &str, s: &str) -> Result<Value, String> {
    let s = s.trim();
    match token_type(name) {
        Some(TokenType::Font) if s == "var(--font-body)" || s == "var(--font-serif)" => {
            Ok(Value::Raw(s.into()))
        }
        Some(TokenType::Weight) => s
            .parse::<u16>()
            .map(Value::Weight)
            .map_err(|e| e.to_string()),
        Some(TokenType::Style) if s == "normal" || s == "italic" => Ok(Value::Style(s.into())),
        Some(TokenType::Font | TokenType::Style) => {
            Err(format!("unexpected base value for {name}: {s}"))
        }
        Some(tok) => parse_value(tok, s),
        None => Err(format!("unknown token {name}")),
    }
}

// ---------------------------------------------------------------------------------------------
// The base themes and the theme file

fn theme_key(id: ThemeId) -> &'static str {
    match id {
        ThemeId::Gallery => "gallery",
        ThemeId::Darkroom => "darkroom",
        ThemeId::Paper => "paper",
        ThemeId::Glass => "glass",
    }
}

fn theme_from_key(s: &str) -> Option<ThemeId> {
    [
        ThemeId::Gallery,
        ThemeId::Darkroom,
        ThemeId::Paper,
        ThemeId::Glass,
    ]
    .into_iter()
    .find(|t| theme_key(*t) == s)
}

type BaseTable = BTreeMap<String, BTreeMap<String, BTreeMap<String, Value>>>;

/// Every token of a built-in theme in one scheme, as the partials set it (`theme_base.json`, kept
/// equal to the SCSS by a test).
pub fn base_values(base: ThemeId, dark: bool) -> &'static BTreeMap<String, Value> {
    static BASE: std::sync::OnceLock<BaseTable> = std::sync::OnceLock::new();
    let table = BASE.get_or_init(|| {
        let raw: BTreeMap<String, BTreeMap<String, BTreeMap<String, String>>> =
            serde_json::from_str(include_str!("theme_base.json")).expect("theme_base.json");
        raw.into_iter()
            .map(|(theme, schemes)| {
                let schemes = schemes
                    .into_iter()
                    .map(|(scheme, tokens)| {
                        let tokens = tokens
                            .into_iter()
                            .map(|(k, v)| {
                                let value = parse_base_value(&k, &v)
                                    .unwrap_or_else(|e| panic!("{theme} {k}: {e}"));
                                (k, value)
                            })
                            .collect();
                        (scheme, tokens)
                    })
                    .collect();
                (theme, schemes)
            })
            .collect()
    });
    &table[theme_key(base)][if dark { "dark" } else { "light" }]
}

fn valid_theme_id(id: &str) -> bool {
    (1..=40).contains(&id.len())
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

/// A path inside the package: relative, plain segments, no `..`.
pub(crate) fn safe_rel_path(p: &str) -> bool {
    !p.is_empty()
        && p.len() <= 200
        && p.split('/').all(|seg| {
            !seg.is_empty()
                && seg != "."
                && seg != ".."
                && seg
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-' || b == b'_')
        })
}

fn string_field(t: &toml::Table, key: &str) -> Result<Option<String>, String> {
    match t.get(key) {
        None => Ok(None),
        Some(toml::Value::String(s)) => Ok(Some(s.clone())),
        Some(_) => Err(format!("{key} must be text")),
    }
}

fn token_table(t: &toml::Table, scheme: &str) -> Result<BTreeMap<String, Value>, String> {
    let Some(v) = t.get(scheme) else {
        return Ok(BTreeMap::new());
    };
    let toml::Value::Table(entries) = v else {
        return Err(format!("[{scheme}] must be a table"));
    };
    let mut out = BTreeMap::new();
    for (name, v) in entries {
        let tok = token_type(name).ok_or_else(|| format!("unknown token {name}"))?;
        let toml::Value::String(s) = v else {
            return Err(format!(
                "[{scheme}] {name}: write the value as text, like \"12px\""
            ));
        };
        let value = parse_value(tok, s).map_err(|e| format!("[{scheme}] {name}: {e}"))?;
        out.insert(name.clone(), value);
    }
    Ok(out)
}

pub fn parse_theme(text: &str) -> Result<ThemeFile, String> {
    let t: toml::Table = text
        .parse()
        .map_err(|e: toml::de::Error| e.message().to_string())?;
    const KEYS: [&str; 8] = [
        "id",
        "name",
        "tagline",
        "base",
        "default_scheme",
        "light",
        "dark",
        "font",
    ];
    if let Some(k) = t.keys().find(|k| !KEYS.contains(&k.as_str())) {
        return Err(format!("unknown key {k}"));
    }
    let id = string_field(&t, "id")?.ok_or("a theme needs an id")?;
    if !valid_theme_id(&id) {
        return Err(format!(
            "theme id \"{id}\" must be 1–40 letters a–z, digits or -"
        ));
    }
    let name = string_field(&t, "name")?.ok_or("a theme needs a name")?;
    if name.trim().is_empty() || name.chars().count() > 40 {
        return Err("a theme's name is 1–40 characters".into());
    }
    let tagline = string_field(&t, "tagline")?.unwrap_or_default();
    if tagline.chars().count() > 100 {
        return Err("a theme's tagline is at most 100 characters".into());
    }
    let base = string_field(&t, "base")?.ok_or("a theme needs a base")?;
    let base = theme_from_key(&base).ok_or("base must be gallery, darkroom, paper or glass")?;
    let default_scheme = string_field(&t, "default_scheme")?.unwrap_or_else(|| "light".into());
    if default_scheme != "light" && default_scheme != "dark" {
        return Err("default_scheme must be light or dark".into());
    }
    let font = match t.get("font") {
        None => None,
        Some(toml::Value::Table(f)) => {
            if let Some(k) = f
                .keys()
                .find(|k| !["file", "weight", "style"].contains(&k.as_str()))
            {
                return Err(format!("[font] has no key {k}"));
            }
            let file = string_field(f, "file")?.ok_or("[font] needs a file")?;
            if !file.to_ascii_lowercase().ends_with(".woff2") || !safe_rel_path(&file) {
                return Err("[font] file must be a .woff2 inside the package".into());
            }
            let weight = match f.get("weight") {
                None => 600,
                Some(toml::Value::Integer(w)) if (100..=900).contains(w) => *w as u16,
                Some(_) => return Err("[font] weight must be a number from 100 to 900".into()),
            };
            let style = string_field(f, "style")?.unwrap_or_else(|| "normal".into());
            if style != "normal" && style != "italic" {
                return Err("[font] style must be normal or italic".into());
            }
            Some(FontDecl {
                file,
                weight,
                style,
            })
        }
        Some(_) => return Err("[font] must be a table".into()),
    };
    Ok(ThemeFile {
        id,
        name,
        tagline,
        base,
        default_scheme,
        light: token_table(&t, "light")?,
        dark: token_table(&t, "dark")?,
        font,
    })
}

/// What the app's theme list shows for one plugin theme.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PluginThemeMeta {
    pub plugin: String,
    pub id: String,
    pub name: String,
    pub tagline: String,
    pub base: ThemeId,
    /// `light` or `dark`: applied the first time the theme is picked.
    pub default_scheme: String,
}

fn safe_plugin_id(id: &str) -> bool {
    !id.is_empty()
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'.' || b == b'-')
}

fn safe_asset_base(base: &str) -> bool {
    !base.is_empty()
        && base
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b":/._-".contains(&b))
}

/// The family name a theme's heading font is served under, stable per plugin and theme.
fn font_family(plugin: &str, theme: &str) -> String {
    let hash = blake3::hash(format!("{plugin}/{theme}").as_bytes()).to_hex();
    format!("lumen-plugin-{}", &hash.as_str()[..8])
}

fn rule(out: &mut String, selector: &str, decls: &[(String, String)]) {
    out.push_str(selector);
    out.push_str(" {\n");
    for (k, v) in decls {
        out.push_str(&format!("  --{k}: {v};\n"));
    }
    out.push_str("}\n");
}

/// What one scheme's table sets, before the base fills the rest.
fn own_tokens(file: &ThemeFile, dark: bool) -> BTreeMap<String, Value> {
    let mut tokens = if dark {
        file.dark.clone()
    } else {
        file.light.clone()
    };
    // The partials write `accent-hover` and `accent-soft` by hand beside `accent`: a theme that
    // sets only the accent gets the same tints (hover a step darker in light, lighter in dark).
    if let Some(Value::Colour(a)) = tokens.get("accent").cloned() {
        let [l, c, h] = a.oklch();
        let (step, soft) = if dark { (0.05, 0.15) } else { (-0.05, 0.12) };
        tokens.entry("accent-hover".into()).or_insert_with(|| {
            Value::Colour(Colour::from_oklch(
                (l + step).clamp(0.0, 1.0),
                c,
                h,
                a.alpha,
            ))
        });
        tokens
            .entry("accent-soft".into())
            .or_insert_with(|| Value::Colour(Colour::from_oklch(l, c, h, soft)));
    }
    tokens
}

fn is_length(name: &str) -> bool {
    matches!(token_type(name), Some(TokenType::Length { .. }))
}

/// The CSS for every enabled plugin theme (`plugin id`, plugin version, file), deterministic. Rust writes every
/// byte of it: ids are checked before they reach a selector and fonts are served from one place.
pub fn css(themes: &[(String, String, ThemeFile)], asset_base: &str) -> String {
    let mut out = String::new();
    for (plugin, version, file) in themes {
        if !safe_plugin_id(plugin) || !valid_theme_id(&file.id) {
            continue;
        }
        let selector = format!(
            "[data-theme=\"{}\"][data-plugin-theme=\"{plugin}/{}\"]",
            theme_key(file.base),
            file.id
        );
        // One attribute-level part less than the accessibility blocks of globals.scss
        // (`:root[data-theme][data-mantine-color-scheme]`), so Increase Contrast and Reduce
        // Transparency still win in dark; later in the page than the base's dark rule, so it wins
        // that tie.
        let dark_selector = format!(
            ":where([data-theme=\"{}\"])[data-plugin-theme=\"{plugin}/{}\"][data-mantine-color-scheme=\"dark\"]",
            theme_key(file.base),
            file.id
        );
        let light = own_tokens(file, false);
        let dark = own_tokens(file, true);

        let mut light_decls: Vec<(String, String)> =
            light.iter().map(|(k, v)| (k.clone(), v.css())).collect();
        if let Some(font) = file
            .font
            .as_ref()
            .filter(|f| safe_asset_base(asset_base) && safe_rel_path(&f.file))
        {
            let family = font_family(plugin, &file.id);
            out.push_str(&format!(
                "@font-face {{\n  font-family: \"{family}\";\n  src: url(\"{asset_base}/plugin-asset/{plugin}/{}?v={version}\") format(\"woff2\");\n  font-weight: {};\n  font-style: {};\n}}\n",
                font.file, font.weight, font.style
            ));
            let base_font = base_values(file.base, false)["font-heading"].css();
            light_decls.push(("font-heading".into(), format!("\"{family}\", {base_font}")));
            light_decls.push(("heading-weight".into(), font.weight.to_string()));
            light_decls.push(("heading-style".into(), font.style.clone()));
        }
        if !light_decls.is_empty() {
            rule(&mut out, &selector, &light_decls);
        }

        // The light rule applies in dark too (it has the same specificity as the base's dark
        // rule), so every colour or shadow it sets and dark doesn't gets the base's dark value back.
        let base_dark = base_values(file.base, true);
        let mut dark_decls: Vec<(String, String)> = Vec::new();
        for name in light
            .keys()
            .filter(|n| !is_length(n) && !dark.contains_key(*n))
        {
            dark_decls.push((name.clone(), base_dark[name].css()));
        }
        dark_decls.extend(dark.iter().map(|(k, v)| (k.clone(), v.css())));
        dark_decls.sort_by(|a, b| a.0.cmp(&b.0));
        if !dark_decls.is_empty() {
            rule(&mut out, &dark_selector, &dark_decls);
        }
    }
    out
}

/// The themes a plugin's manifest lists, read from its folder.
pub fn themes_of(
    dir: &std::path::Path,
    m: &crate::manifest::Manifest,
) -> Result<Vec<(PluginThemeMeta, ThemeFile)>, String> {
    let mut out: Vec<(PluginThemeMeta, ThemeFile)> = Vec::new();
    for f in &m.themes {
        if !safe_rel_path(&f.file) {
            return Err(format!("{}: not a path inside the package", f.file));
        }
        let text =
            std::fs::read_to_string(dir.join(&f.file)).map_err(|e| format!("{}: {e}", f.file))?;
        let file = parse_theme(&text).map_err(|e| format!("{}: {e}", f.file))?;
        if out.iter().any(|(meta, _)| meta.id == file.id) {
            return Err(format!(
                "{}: another theme in this plugin is already called \"{}\"",
                f.file, file.id
            ));
        }
        let meta = PluginThemeMeta {
            plugin: m.id.clone(),
            id: file.id.clone(),
            name: file.name.clone(),
            tagline: file.tagline.clone(),
            base: file.base,
            default_scheme: file.default_scheme.clone(),
        };
        out.push((meta, file));
    }
    Ok(out)
}

// ---------------------------------------------------------------------------------------------
// Contrast

/// A text or fill pair that has to stay readable, as `src/styles/contrast.test.ts` states them.
#[derive(Debug, Clone, Copy)]
pub struct Pair {
    pub fg: &'static str,
    pub bg: &'static str,
    pub min: f64,
    /// Both laid over the page's opaque `bg-base` (text), else plain luminances (fills).
    pub over_base: bool,
}

const fn fill(fg: &'static str, bg: &'static str, min: f64) -> Pair {
    Pair {
        fg,
        bg,
        min,
        over_base: false,
    }
}

const fn text(fg: &'static str, bg: &'static str) -> Pair {
    Pair {
        fg,
        bg,
        min: 4.5,
        over_base: true,
    }
}

/// The pairs and thresholds of `contrast.test.ts`, both schemes. Borders and the wash aren't
/// checked; `fg-faint` draws hairlines and tracks, never words.
pub const PAIRS: &[Pair] = &[
    // Filled buttons: the label on the accent and on its hover state.
    fill("accent-fg", "accent", 4.5),
    fill("accent-fg", "accent-hover", 4.5),
    // A disabled button is the sunken surface with muted text.
    fill("fg-muted", "bg-sunken", 3.0),
    // Develop's stage.
    fill("fg-muted", "develop-stage-bg", 4.5),
    fill("fg-primary", "develop-stage-bg", 4.5),
    // Every ink that sets words, on every surface it is set on.
    text("fg-primary", "bg-base"),
    text("fg-primary", "bg-raised"),
    text("fg-primary", "bg-sunken"),
    text("fg-primary", "bg-sidebar"),
    text("fg-muted", "bg-base"),
    text("fg-muted", "bg-raised"),
    text("fg-muted", "bg-sunken"),
    text("fg-muted", "bg-sidebar"),
    text("fg-subtle", "bg-base"),
    text("fg-subtle", "bg-raised"),
    text("fg-subtle", "bg-sunken"),
    text("fg-subtle", "bg-sidebar"),
    // The accent sets words too: links, the active toggle's label, a toast's action.
    text("accent", "bg-base"),
    text("accent", "bg-raised"),
];

fn luminance(lin: [f64; 3]) -> f64 {
    0.2126 * lin[0] + 0.7152 * lin[1] + 0.0722 * lin[2]
}

fn lin64(c: &Colour) -> [f64; 3] {
    c.lin.map(f64::from)
}

fn ratio(a: f64, b: f64) -> f64 {
    let (hi, lo) = if a > b { (a, b) } else { (b, a) };
    (hi + 0.05) / (lo + 0.05)
}

/// WCAG contrast of two colours, alpha ignored.
pub fn contrast(a: &Colour, b: &Colour) -> f64 {
    ratio(luminance(lin64(a)), luminance(lin64(b)))
}

fn encode(c: f64) -> f64 {
    if c <= 0.0031308 {
        12.92 * c
    } else {
        1.055 * c.powf(1.0 / 2.4) - 0.055
    }
}

fn decode(c: f64) -> f64 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

/// The luminance of `top` painted over the opaque `under`, as the browser composites it (in
/// encoded sRGB).
fn luminance_over(top: &Colour, under: &Colour) -> f64 {
    let a = f64::from(top.alpha);
    let (t, u) = (lin64(top).map(encode), lin64(under).map(encode));
    luminance([0, 1, 2].map(|i| decode(t[i] * a + u[i] * (1.0 - a))))
}

/// Contrast of text `fg` on `surface`, both laid over the page's opaque `base`.
pub fn text_contrast(fg: &Colour, surface: &Colour, base: &Colour) -> f64 {
    let s = luminance_over(surface, base);
    // Text is opaque ink in every theme; a translucent one would composite over the surface.
    let f = if fg.alpha >= 1.0 {
        luminance(lin64(fg))
    } else {
        s
    };
    ratio(f, s)
}

/// A scheme's tokens: the base theme's, then what the theme sets (with the accent's tints derived).
pub fn effective_tokens(file: &ThemeFile, dark: bool) -> BTreeMap<String, Value> {
    let mut tokens = base_values(file.base, dark).clone();
    tokens.extend(own_tokens(file, dark));
    tokens
}

/// Every pair that fails in either scheme, as `"<fg> on <bg> (dark): 3.10 < 4.5"`.
pub fn contrast_failures(file: &ThemeFile) -> Vec<String> {
    let mut out = Vec::new();
    for dark in [false, true] {
        let tokens = effective_tokens(file, dark);
        let colour = |name: &str| match tokens.get(name) {
            Some(Value::Colour(c)) => Some(c),
            _ => None,
        };
        for p in PAIRS {
            let (Some(fg), Some(bg), Some(base)) = (colour(p.fg), colour(p.bg), colour("bg-base"))
            else {
                continue;
            };
            let got = if p.over_base {
                text_contrast(fg, bg, base)
            } else {
                contrast(fg, bg)
            };
            if got < p.min {
                let shown = (got * 100.0).floor() / 100.0;
                let scheme = if dark { "dark" } else { "light" };
                out.push(format!(
                    "{} on {} ({scheme}): {shown:.2} < {}",
                    p.fg,
                    p.bg,
                    fmt(p.min as f32)
                ));
            }
        }
    }
    out
}

const MAX_FONT_BYTES: usize = 2 * 1024 * 1024;

/// The theme checker: parses every `[[themes]]` file, checks its font and its contrast.
pub struct ThemeCheck;

impl KindCheck for ThemeCheck {
    fn kinds(&self) -> &'static [PluginKind] {
        &[PluginKind::Theme]
    }

    fn check(&self, pkg: &Unpacked, dev: bool, report: &mut Report) {
        let mut seen: Vec<String> = Vec::new();
        for f in &pkg.manifest.themes {
            let text = match pkg.read(&f.file).map(String::from_utf8) {
                Ok(Ok(t)) => t,
                Ok(Err(_)) => {
                    report.problems.push(format!("{}: not text", f.file));
                    continue;
                }
                Err(e) => {
                    report.problems.push(format!("{}: {}", f.file, e.message));
                    continue;
                }
            };
            let file = match parse_theme(&text) {
                Ok(t) => t,
                Err(e) => {
                    report.problems.push(format!("{}: {e}", f.file));
                    continue;
                }
            };
            if seen.contains(&file.id) {
                report.problems.push(format!(
                    "{}: another theme in this plugin is already called \"{}\"",
                    f.file, file.id
                ));
                continue;
            }
            seen.push(file.id.clone());
            let label = format!("Theme \"{}\"", file.name);
            if let Some(font) = &file.font {
                if let Some(p) = font_problem(pkg, &font.file) {
                    report.problems.push(format!("{label}: {p}"));
                }
            }
            for failure in contrast_failures(&file) {
                let line = format!("{label}: {failure}");
                if dev {
                    report.notes.push(line);
                } else {
                    report.problems.push(line);
                }
            }
            report.adds.push(format!("A theme: \"{}\"", file.name));
        }
    }
}

/// Why a theme's font can't be used, if it can't.
fn font_problem(pkg: &Unpacked, path: &str) -> Option<String> {
    if !pkg.manifest.assets.iter().any(|a| a.path == path) {
        return Some(format!(
            "the font {path} must be listed under [[assets]] with its licence"
        ));
    }
    match pkg.read(path) {
        Err(e) => Some(format!("{path}: {}", e.message)),
        Ok(b) if b.len() > MAX_FONT_BYTES => Some(format!("the font {path} is over 2 MB")),
        Ok(b) if !b.starts_with(b"wOF2") => Some(format!("{path} isn't a woff2 font")),
        Ok(_) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;
    use std::path::PathBuf;

    fn repo() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../..")
    }

    /// `{theme: {light|dark: {token: raw value}}}` read from the SCSS partials; dark is light + its overrides.
    fn scss_base() -> BTreeMap<String, BTreeMap<String, BTreeMap<String, String>>> {
        fn block(scss: &str, selector: &str) -> BTreeMap<String, String> {
            let Some(at) = scss.find(&format!("{selector} {{")) else {
                return BTreeMap::new();
            };
            let body = &scss[at + selector.len() + 2..];
            let body = &body[..body.find('}').unwrap()];
            body.split(';')
                .filter_map(|d| d.trim().strip_prefix("--").map(str::to_string))
                .filter_map(|d| {
                    d.split_once(':')
                        .map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))
                })
                .collect()
        }
        let dir = repo().join("src/styles/themes");
        let mut out = BTreeMap::new();
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            let name = path.file_name().unwrap().to_string_lossy().to_string();
            let Some(theme) = name.strip_prefix('_').and_then(|n| n.strip_suffix(".scss")) else {
                continue;
            };
            let scss = std::fs::read_to_string(&path).unwrap();
            let light = block(&scss, &format!("[data-theme=\"{theme}\"]"));
            let mut dark = light.clone();
            dark.extend(block(
                &scss,
                &format!("[data-theme=\"{theme}\"][data-mantine-color-scheme=\"dark\"]"),
            ));
            out.insert(
                theme.to_string(),
                BTreeMap::from([("light".to_string(), light), ("dark".to_string(), dark)]),
            );
        }
        out
    }

    #[test]
    fn token_table_matches_scss() {
        let mut union = BTreeSet::new();
        for theme in scss_base().values() {
            for scheme in theme.values() {
                union.extend(scheme.keys().cloned());
            }
        }
        let table: BTreeSet<String> = TOKENS.iter().map(|(n, _)| n.to_string()).collect();
        assert_eq!(table, union);
        assert_eq!(TOKENS.len(), table.len(), "a token listed twice");
    }

    #[test]
    fn base_tokens_match_scss() {
        let want = scss_base();
        let path = repo().join("src-tauri/crates/photos-plugins/src/theme_base.json");
        if std::env::var("LUMEN_REGEN_GOLDEN").is_ok() {
            std::fs::write(&path, serde_json::to_string_pretty(&want).unwrap() + "\n").unwrap();
        }
        let have: BTreeMap<String, BTreeMap<String, BTreeMap<String, String>>> =
            serde_json::from_str(
                &std::fs::read_to_string(&path).expect("run with LUMEN_REGEN_GOLDEN=1"),
            )
            .unwrap();
        assert_eq!(
            have, want,
            "theme_base.json is stale: run with LUMEN_REGEN_GOLDEN=1"
        );
    }

    fn colour(s: &str) -> Colour {
        match parse_value(TokenType::Colour, s) {
            Ok(Value::Colour(c)) => c,
            other => panic!("{s}: {other:?}"),
        }
    }

    #[test]
    fn colour_forms() {
        let hex = colour("#5E81AC");
        assert!(hex.css.starts_with("oklch("), "{}", hex.css);
        assert_eq!(colour("#abc").lin, colour("#aabbcc").lin);
        let rgb = colour("rgb(94 129 172 / 0.5)");
        assert_eq!(rgb.alpha, 0.5);
        assert!(rgb.css.ends_with(" / 0.5)"), "{}", rgb.css);
        let ok = colour("oklch(96% 0.01 240 / 50%)");
        assert_eq!(ok.alpha, 0.5);
        assert_eq!(ok.css, "oklch(96% 0.01 240 / 0.5)");
        assert_eq!(colour("oklch(96% 0.01 240)").css, "oklch(96% 0.01 240)");
        let hsl = colour("hsl(210deg 30% 50%)");
        assert!(
            (hsl.lin[2] - colour("#6689a6").lin[2]).abs() < 0.01,
            "hsl(210 30% 50%) is about #6689a6"
        );
        // The same colour spelled three ways has the same linear value.
        let a = colour("#5e81ac");
        let b = colour("rgb(94, 129, 172)");
        assert_eq!(a.lin, b.lin);
        let err = parse_value(TokenType::Colour, "red").unwrap_err();
        assert!(err.contains("use a colour value"), "{err}");
    }

    /// The page paints the native window from `--bg-base` with `oklchToRgb` (src/lib/oklch.ts),
    /// which reads only `oklch(L% C H [/ a])` with plain non-negative numbers: every colour a
    /// theme may spell is written that way, hue within 0–360.
    #[test]
    fn every_colour_is_written_as_plain_oklch() {
        let plain = |css: &str| {
            let inner = css
                .strip_prefix("oklch(")
                .and_then(|r| r.strip_suffix(')'))
                .unwrap_or_else(|| panic!("{css}"));
            let (main, alpha) = inner.split_once(" / ").unwrap_or((inner, "1"));
            let parts: Vec<&str> = main.split(' ').collect();
            assert_eq!(parts.len(), 3, "{css}");
            assert!(parts[0].ends_with('%'), "{css}");
            for p in [parts[0].trim_end_matches('%'), parts[1], parts[2], alpha] {
                assert!(
                    p.parse::<f32>().is_ok_and(|v| v >= 0.0 && v.is_finite()),
                    "{css}: {p}"
                );
            }
            assert!(parts[2].parse::<f32>().unwrap() < 360.0, "{css}");
        };
        for form in [
            "#2e3440",
            "#fff",
            "#000000",
            "#5e81ac80",
            "rgb(46 52 64)",
            "rgb(100% 0% 0%)",
            "hsl(210 30% 50%)",
            "hsl(-30 30% 50%)",
            "oklch(50% 0.1 -10)",
            "oklch(50% 0.1 725)",
            "oklch(96% 0.01 240 / 50%)",
        ] {
            plain(&colour(form).css);
        }
        // A huge hue is brought into range, never written as `inf`.
        if let Ok(v) = parse_value(TokenType::Colour, "oklch(50% 0.1 1e30)") {
            plain(&v.css());
        }
        // And a theme's whole generated CSS carries nothing else.
        let text = "id = \"nord\"\nname = \"N\"\nbase = \"darkroom\"\n[dark]\nbg-base = \"#2e3440\"\nshadow-md = \"0 6px 16px #00000080\"\n";
        let css = nord_css(text);
        assert!(css.contains("--bg-base: oklch("), "{css}");
        assert!(!css.contains('#'), "no hex in the CSS: {css}");
    }

    /// The accessibility blocks of globals.scss (`:root[data-theme][data-mantine-color-scheme]`)
    /// must keep outweighing a plugin theme, in dark as in light.
    #[test]
    fn plugin_rules_stay_below_the_accessibility_rules() {
        let globals = std::fs::read_to_string(repo().join("src/styles/globals.scss")).unwrap();
        assert!(globals.contains(":root[data-theme][data-mantine-color-scheme] {"));
        let accessibility = 3; // `:root` counts like an attribute selector
        let css = nord_css(NORD);
        let mut rules = 0;
        for line in css
            .lines()
            .filter(|l| l.ends_with(" {") && l.contains("[data-plugin-theme="))
        {
            let mut rest = line.to_string();
            // `:where(...)` adds nothing.
            while let Some(a) = rest.find(":where(") {
                let end = a + rest[a..].find(')').unwrap() + 1;
                rest.replace_range(a..end, "");
            }
            let specificity = rest.matches('[').count();
            assert!(
                specificity < accessibility,
                "{line} outweighs the accessibility rules"
            );
            rules += 1;
        }
        assert_eq!(rules, 2, "a light and a dark rule: {css}");
    }

    #[test]
    fn refuses_raw_css() {
        for bad in [
            "#fff; }",
            "url(x)",
            "var(--a)",
            "oklch(50% 0.1 10)}",
            "URL(x)",
            "#fff /* x */",
        ] {
            assert!(parse_value(TokenType::Colour, bad).is_err(), "{bad}");
        }
        assert!(parse_value(TokenType::Shadow, "0 1px 2px #000; color: red").is_err());
        assert!(parse_value(TokenType::Wash, "url(a.png)").is_err());
    }

    #[test]
    fn length_ranges() {
        let tok = TokenType::Length {
            min: 0.0,
            max: 32.0,
        };
        assert_eq!(parse_value(tok, "12px"), Ok(Value::Length(12.0)));
        assert_eq!(parse_value(tok, "0"), Ok(Value::Length(0.0)));
        let err = parse_value(tok, "33px").unwrap_err();
        assert!(err.contains("0–32"), "{err}");
        assert!(parse_value(tok, "1em").is_err());
        assert!(parse_value(tok, "-1px").is_err());
    }

    #[test]
    fn shadow_and_wash_limits() {
        let layer = "0 1px 2px #000";
        assert!(parse_value(TokenType::Shadow, &[layer; 3].join(", ")).is_ok());
        let err = parse_value(TokenType::Shadow, &[layer; 4].join(", ")).unwrap_err();
        assert!(err.contains("3"), "{err}");
        let radial = "radial-gradient(60% 50% at 85% 0%, oklch(90% 0.06 40) 0%, transparent 60%)";
        assert!(parse_value(TokenType::Wash, &[radial; 3].join(", ")).is_ok());
        assert!(parse_value(TokenType::Wash, &[radial; 4].join(", ")).is_err());
        assert_eq!(
            parse_value(TokenType::Wash, "none"),
            Ok(Value::Wash(vec![]))
        );
        assert!(parse_value(TokenType::Wash, "linear-gradient(red, blue)").is_err());
    }

    #[test]
    fn unknown_token_refused() {
        let text = "id = \"a\"\nname = \"A\"\nbase = \"gallery\"\n[light]\nbg-baze = \"#fff\"\n";
        let err = parse_theme(text).unwrap_err();
        assert!(err.contains("unknown token bg-baze"), "{err}");
    }

    #[test]
    fn theme_file_parses() {
        let text = r##"
id = "nord"
name = "Nord"
tagline = "Arctic blues on slate."
base = "darkroom"
default_scheme = "dark"
[light]
bg-base = "oklch(96% 0.01 240)"
accent = "#5e81ac"
radius-md = "10px"
[dark]
bg-base = "oklch(24% 0.02 240)"
[font]
file = "assets/fonts/Lora.woff2"
weight = 600
style = "normal"
"##;
        let t = parse_theme(text).unwrap();
        assert_eq!(
            (t.id.as_str(), t.base, t.default_scheme.as_str()),
            ("nord", ThemeId::Darkroom, "dark")
        );
        assert_eq!(t.light.len(), 3);
        assert_eq!(t.dark.len(), 1);
        assert_eq!(t.font.unwrap().file, "assets/fonts/Lora.woff2");
    }

    #[test]
    fn theme_header_rules() {
        let head = |extra: &str| format!("id = \"a\"\nname = \"A\"\nbase = \"gallery\"\n{extra}");
        assert!(parse_theme(&head("")).is_ok());
        assert!(
            parse_theme("id = \"A\"\nname = \"A\"\nbase = \"gallery\"\n").is_err(),
            "uppercase id"
        );
        assert!(
            parse_theme("id = \"a\"\nname = \"A\"\nbase = \"neon\"\n").is_err(),
            "unknown base"
        );
        assert!(parse_theme(&head("default_scheme = \"auto\"\n")).is_err());
        assert!(
            parse_theme(&head("[light]\nfont-heading = \"serif\"\n")).is_err(),
            "fonts go in [font]"
        );
        assert!(
            parse_theme(&head("[font]\nfile = \"a.ttf\"\n")).is_err(),
            "woff2 only"
        );
        assert!(parse_theme(&head("[font]\nfile = \"../a.woff2\"\n")).is_err());
        assert!(parse_theme(&head("surprise = 1\n")).is_err());
    }

    #[test]
    fn base_values_parse_and_write_back_as_the_partials_do() {
        for (theme, schemes) in scss_base() {
            let id = theme_from_key(&theme).unwrap();
            for (scheme, tokens) in schemes {
                let have = base_values(id, scheme == "dark");
                assert_eq!(
                    have.len(),
                    TOKENS.len(),
                    "{theme} {scheme} sets every token"
                );
                for (k, raw) in tokens {
                    assert_eq!(have[&k].css(), raw, "{theme} {scheme} {k}");
                }
            }
        }
    }

    const NORD: &str = r##"
id = "nord"
name = "Nord"
tagline = "Arctic blues on slate."
base = "darkroom"
default_scheme = "dark"
[light]
bg-base = "oklch(96% 0.01 240)"
accent = "#5e81ac"
radius-md = "10px"
[dark]
bg-base = "oklch(24% 0.02 240)"
"##;
    const LIGHT_SEL: &str = r#"[data-theme="darkroom"][data-plugin-theme="studio.jl.nord/nord"]"#;
    const DARK_SEL: &str = r#":where([data-theme="darkroom"])[data-plugin-theme="studio.jl.nord/nord"][data-mantine-color-scheme="dark"]"#;

    fn nord_css(text: &str) -> String {
        css(
            &[("studio.jl.nord".to_string(), "1.2.0".to_string(), parse_theme(text).unwrap())],
            "http://photos.localhost",
        )
    }

    #[test]
    fn css_shape() {
        let css = nord_css(NORD);
        assert!(css.contains(&format!("{LIGHT_SEL} {{")), "{css}");
        assert!(css.contains("  --bg-base: oklch(96% 0.01 240);"), "{css}");
        assert!(css.contains(&format!("{DARK_SEL} {{")), "{css}");
        assert!(css.contains("  --bg-base: oklch(24% 0.02 240);"), "{css}");
        // The built-in partials write their dark rule as `[data-theme][data-mantine-color-scheme]`.
        let partial =
            std::fs::read_to_string(repo().join("src/styles/themes/_darkroom.scss")).unwrap();
        assert!(partial.contains(r#"[data-theme="darkroom"][data-mantine-color-scheme="dark"] {"#));
    }

    #[test]
    fn one_scheme_only() {
        let text = "id = \"nord\"\nname = \"Nord\"\nbase = \"darkroom\"\n[dark]\nbg-base = \"oklch(24% 0.02 240)\"\n";
        let css = nord_css(text);
        assert!(
            !css.contains(&format!("{LIGHT_SEL} {{")),
            "no light rule: {css}"
        );
        assert!(css.contains(&format!("{DARK_SEL} {{")), "{css}");
    }

    #[test]
    fn dark_restates_base_for_light_only_tokens() {
        // `accent` is set for light only: the light rule would otherwise leak into dark (equal
        // specificity with the base's dark rule), so the dark rule puts the base's dark accent back.
        let text = "id = \"nord\"\nname = \"Nord\"\nbase = \"darkroom\"\n[light]\naccent = \"#5e81ac\"\nradius-md = \"10px\"\n";
        let css = nord_css(text);
        let dark = &css[css.find(&format!("{DARK_SEL} {{")).expect(&css)..];
        assert!(dark.contains("  --accent: oklch(80% 0.14 80);"), "{css}");
        // Lengths live in the light block only in every partial, so dark keeps inheriting them.
        assert!(!dark.contains("--radius-md"), "{css}");
    }

    #[test]
    fn font_face() {
        let text = format!(
            "{NORD}[font]\nfile = \"assets/fonts/Lora.woff2\"\nweight = 600\nstyle = \"normal\"\n"
        );
        let css = nord_css(&text);
        let family = css
            .split("font-family: \"")
            .nth(1)
            .unwrap()
            .split('"')
            .next()
            .unwrap()
            .to_string();
        assert!(
            family.starts_with("lumen-plugin-") && family.len() == "lumen-plugin-".len() + 8,
            "{family}"
        );
        assert!(css.contains("@font-face {"), "{css}");
        assert!(
            css.contains("src: url(\"http://photos.localhost/plugin-asset/studio.jl.nord/assets/fonts/Lora.woff2?v=1.2.0\") format(\"woff2\");"),
            "{css}"
        );
        assert!(
            css.contains("font-weight: 600;") && css.contains("font-style: normal;"),
            "{css}"
        );
        assert!(
            css.contains(&format!(
                "  --font-heading: \"{family}\", var(--font-body);"
            )),
            "{css}"
        );
        assert!(css.contains("  --heading-weight: 600;"), "{css}");
        // Only the one place serves fonts.
        assert_eq!(css.matches("url(").count(), 1, "{css}");
    }

    #[test]
    fn ids_never_break_selectors() {
        assert!(parse_theme("id = 'a\"]b'\nname = \"A\"\nbase = \"gallery\"\n").is_err());
        assert!(parse_theme("id = \"a b\"\nname = \"A\"\nbase = \"gallery\"\n").is_err());
        // A file built by hand (not through the parser) still can't reach the selector.
        let mut bad = parse_theme(NORD).unwrap();
        bad.id = "a\"] { x".into();
        assert_eq!(css(&[("p.q".into(), "1".into(), bad)], "http://photos.localhost"), "");
        let ok = parse_theme(NORD).unwrap();
        assert_eq!(css(&[("p\"] {".into(), "1".into(), ok)], "http://photos.localhost"), "");
    }

    #[test]
    fn css_is_deterministic() {
        let a = parse_theme(NORD).unwrap();
        let b = parse_theme(&NORD.replace("nord", "other")).unwrap();
        let one = css(
            &[
                ("studio.jl.nord".into(), "1".into(), a.clone()),
                ("studio.jl.other".into(), "1".into(), b.clone()),
            ],
            "photos://localhost",
        );
        let two = css(
            &[("studio.jl.nord".into(), "1".into(), a), ("studio.jl.other".into(), "1".into(), b)],
            "photos://localhost",
        );
        assert_eq!(one, two);
        assert!(one.contains("studio.jl.other/other"));
    }

    /// `{pairs, samples}`: what `contrast.test.ts` reads. Ratios to three decimals.
    fn golden() -> serde_json::Value {
        let r3 = |x: f64| (x * 1000.0).round() / 1000.0;
        let pairs: Vec<_> = PAIRS
            .iter()
            .map(|p| {
                serde_json::json!({ "fg": format!("--{}", p.fg), "bg": format!("--{}", p.bg), "min": p.min, "overBase": p.over_base })
            })
            .collect();
        // One sample per colour form, each against white.
        let forms = [
            "#5e81ac",
            "#abc",
            "#5e81ac80",
            "rgb(94 129 172)",
            "rgb(94, 129, 172)",
            "rgb(94 129 172 / 0.5)",
            "hsl(210deg 30% 50%)",
            "hsl(24 80% 40%)",
            "oklch(52% 0.15 70)",
            "oklch(96% 0.01 240 / 50%)",
        ];
        let samples: Vec<_> = forms
            .iter()
            .map(|f| {
                let ratio = contrast(&colour(f), &colour("#ffffff"));
                serde_json::json!({ "fg": f, "bg": "#ffffff", "ratio": r3(ratio) })
            })
            .collect();
        serde_json::json!({ "note": "Regenerated by the photos-plugins theme tests (LUMEN_REGEN_GOLDEN=1); never edited by hand.", "pairs": pairs, "samples": samples })
    }

    fn theme_with(extra: &str) -> ThemeFile {
        parse_theme(&format!(
            "id = \"t\"\nname = \"T\"\nbase = \"darkroom\"\n{extra}"
        ))
        .unwrap()
    }

    #[test]
    fn builtins_pass() {
        for base in ["gallery", "darkroom", "paper", "glass"] {
            let file =
                parse_theme(&format!("id = \"t\"\nname = \"T\"\nbase = \"{base}\"\n")).unwrap();
            assert_eq!(contrast_failures(&file), Vec::<String>::new(), "{base}");
        }
    }

    #[test]
    fn low_contrast_refused() {
        // The plan names `fg-base`; the token is `fg-primary`.
        let fails = contrast_failures(&theme_with("[dark]\nfg-primary = \"#333333\"\n"));
        let f = fails
            .iter()
            .find(|f| f.starts_with("fg-primary on bg-base (dark)"))
            .unwrap_or_else(|| panic!("{fails:?}"));
        assert!(f.contains(" < 4.5"), "{f}");
        assert!(
            !fails.iter().any(|f| f.contains("(light)")),
            "light is the base unchanged: {fails:?}"
        );
    }

    #[test]
    fn derived_tokens_follow_overrides() {
        // Only `accent` is set: the hover and soft tints the partials write by hand follow it, and
        // contrast checks the derived hover, not the base's.
        let file = theme_with("[light]\naccent = \"oklch(95% 0.02 70)\"\n");
        let eff = effective_tokens(&file, false);
        assert_eq!(eff["accent-hover"].css(), "oklch(90% 0.02 70)");
        assert_eq!(eff["accent-soft"].css(), "oklch(95% 0.02 70 / 0.12)");
        let fails = contrast_failures(&file);
        assert!(
            fails
                .iter()
                .any(|f| f.starts_with("accent-fg on accent-hover (light)")),
            "{fails:?}"
        );
        assert!(!fails.iter().any(|f| f.contains("(dark)")), "{fails:?}");
        // Dark goes the other way (lighter), a tint of 15 %, and an explicit value always wins.
        let dark = effective_tokens(
            &theme_with("[dark]\naccent = \"oklch(60% 0.1 70)\"\n"),
            true,
        );
        assert_eq!(dark["accent-hover"].css(), "oklch(65% 0.1 70)");
        assert_eq!(dark["accent-soft"].css(), "oklch(60% 0.1 70 / 0.15)");
        let own = effective_tokens(
            &theme_with("[light]\naccent = \"#5e81ac\"\naccent-hover = \"oklch(0% 0 0)\"\n"),
            false,
        );
        assert_eq!(own["accent-hover"].css(), "oklch(0% 0 0)");
        // Light-only accent in the written CSS: dark puts the base's dark tints back.
        let css = nord_css(
            "id = \"nord\"\nname = \"N\"\nbase = \"darkroom\"\n[light]\naccent = \"#5e81ac\"\n",
        );
        assert!(css.contains("  --accent-hover: oklch("), "{css}");
        let dark_rule = &css[css.find(&format!("{DARK_SEL} {{")).unwrap()..];
        assert!(
            dark_rule.contains("  --accent-hover: oklch(85% 0.14 80);"),
            "{css}"
        );
    }

    #[test]
    fn contrast_golden() {
        let path = repo().join("src/styles/contrast-golden.json");
        let want = golden();
        if std::env::var("LUMEN_REGEN_GOLDEN").is_ok() {
            std::fs::write(&path, serde_json::to_string_pretty(&want).unwrap() + "\n").unwrap();
        }
        let have: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(&path).expect("run with LUMEN_REGEN_GOLDEN=1"),
        )
        .unwrap();
        assert_eq!(
            have, want,
            "contrast-golden.json is stale: run with LUMEN_REGEN_GOLDEN=1"
        );
    }

    /// A theme folder opened as a package: `plugin.toml`, the theme, and an optional font.
    fn open_theme_pkg(
        theme: &str,
        font: Option<(&str, &[u8])>,
        listed: bool,
    ) -> (tempfile::TempDir, Unpacked) {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("themes")).unwrap();
        std::fs::write(dir.path().join("themes/nord.toml"), theme).unwrap();
        let mut extra = String::from("[[themes]]\nfile = \"themes/nord.toml\"\n");
        if let Some((path, bytes)) = font {
            std::fs::create_dir_all(dir.path().join(path).parent().unwrap()).unwrap();
            std::fs::write(dir.path().join(path), bytes).unwrap();
            if listed {
                extra.push_str(&format!(
                    "[[assets]]\npath = \"{path}\"\nlicense = \"OFL-1.1\"\n"
                ));
            }
        }
        std::fs::write(
            dir.path().join("plugin.toml"),
            crate::package::fixture::manifest(&extra),
        )
        .unwrap();
        let pkg = crate::package::open_folder(dir.path()).unwrap();
        (dir, pkg)
    }

    fn run_check(pkg: &Unpacked, dev: bool) -> Report {
        let mut report = Report::default();
        ThemeCheck.check(pkg, dev, &mut report);
        report
    }

    const FONT_THEME: &str = "id = \"nord\"\nname = \"Nord\"\nbase = \"gallery\"\n[font]\nfile = \"assets/fonts/Lora.woff2\"\n";

    /// A theme that reads: the base's palette with a dimmer dark page.
    const GOOD: &str = "id = \"nord\"\nname = \"Nord\"\nbase = \"darkroom\"\n[light]\nradius-md = \"10px\"\n[dark]\nbg-base = \"oklch(16% 0.004 60)\"\n";

    #[test]
    fn theme_check_claims_themes_and_reports_them() {
        assert_eq!(ThemeCheck.kinds(), &[PluginKind::Theme]);
        let (_d, pkg) = open_theme_pkg(GOOD, None, true);
        let report = run_check(&pkg, false);
        assert_eq!(report.adds, vec!["A theme: \"Nord\"".to_string()]);
        assert!(
            report.problems.is_empty() && report.notes.is_empty(),
            "{report:?}"
        );
        let (_d, bad) = open_theme_pkg("id = 3", None, true);
        let report = run_check(&bad, false);
        assert!(
            report
                .problems
                .iter()
                .any(|p| p.contains("themes/nord.toml")),
            "{report:?}"
        );
    }

    #[test]
    fn dev_mode_notes() {
        let low = GOOD.replace("[dark]\n", "[dark]\nfg-primary = \"#333333\"\n");
        let (_d, pkg) = open_theme_pkg(&low, None, true);
        let strict = run_check(&pkg, false);
        assert!(
            strict
                .problems
                .iter()
                .any(|p| p.contains("fg-primary on bg-base (dark)")),
            "{strict:?}"
        );
        assert!(strict.notes.is_empty());
        let dev = run_check(&pkg, true);
        assert!(dev.problems.is_empty(), "{dev:?}");
        assert!(
            dev.notes
                .iter()
                .any(|p| p.contains("fg-primary on bg-base (dark)")),
            "{dev:?}"
        );
        assert_eq!(
            dev.adds,
            vec!["A theme: \"Nord\"".to_string()],
            "still loads, still listed"
        );
    }

    #[test]
    fn font_asset_checked() {
        let mut woff2 = b"wOF2".to_vec();
        woff2.extend_from_slice(&[0; 64]);
        let font = Some(("assets/fonts/Lora.woff2", woff2.as_slice()));
        let (_d, ok) = open_theme_pkg(FONT_THEME, font, true);
        assert!(run_check(&ok, false).problems.is_empty());
        let (_d, unlisted) = open_theme_pkg(FONT_THEME, font, false);
        let r = run_check(&unlisted, false);
        assert!(r.problems.iter().any(|p| p.contains("[[assets]]")), "{r:?}");
        let not_woff = Some(("assets/fonts/Lora.woff2", &b"OTTO0000"[..]));
        let (_d, bad) = open_theme_pkg(FONT_THEME, not_woff, true);
        let r = run_check(&bad, false);
        assert!(r.problems.iter().any(|p| p.contains("woff2")), "{r:?}");
        let mut big = b"wOF2".to_vec();
        big.resize(2 * 1024 * 1024 + 1, 0);
        let (_d, huge) = open_theme_pkg(
            FONT_THEME,
            Some(("assets/fonts/Lora.woff2", big.as_slice())),
            true,
        );
        let r = run_check(&huge, false);
        assert!(r.problems.iter().any(|p| p.contains("2 MB")), "{r:?}");
        // Font problems are never softened by Developer mode.
        let r = run_check(&huge, true);
        assert!(r.problems.iter().any(|p| p.contains("2 MB")), "{r:?}");
    }

    fn manifest(themes: &[&str]) -> crate::manifest::Manifest {
        let files: String = themes
            .iter()
            .map(|f| format!("[[themes]]\nfile = \"{f}\"\n"))
            .collect();
        crate::manifest::parse(&format!(
            "id = \"studio.jl.nord\"\nname = \"Nord\"\nversion = \"1.0.0\"\nauthor = \"JL\"\ndescription = \"d\"\nlicense = \"MIT\"\nlumen = \">=26.2.0\"\napi = 1\n{files}"
        ))
        .unwrap()
    }

    #[test]
    fn themes_of_reads_each_listed_file() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("themes")).unwrap();
        std::fs::write(dir.path().join("themes/nord.toml"), NORD).unwrap();
        std::fs::write(
            dir.path().join("themes/two.toml"),
            NORD.replace("\"nord\"", "\"two\""),
        )
        .unwrap();
        let got = themes_of(
            dir.path(),
            &manifest(&["themes/nord.toml", "themes/two.toml"]),
        )
        .unwrap();
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].0.plugin, "studio.jl.nord");
        assert_eq!(
            (
                got[0].0.id.as_str(),
                got[0].0.base,
                got[0].0.default_scheme.as_str()
            ),
            ("nord", ThemeId::Darkroom, "dark")
        );
        assert_eq!(got[1].0.id, "two");
        // The same id twice, a missing file and a file that isn't a theme all say which file.
        std::fs::write(dir.path().join("themes/two.toml"), NORD).unwrap();
        let dup = themes_of(
            dir.path(),
            &manifest(&["themes/nord.toml", "themes/two.toml"]),
        )
        .unwrap_err();
        assert!(
            dup.contains("themes/two.toml") && dup.contains("nord"),
            "{dup}"
        );
        let missing = themes_of(dir.path(), &manifest(&["themes/none.toml"])).unwrap_err();
        assert!(missing.contains("themes/none.toml"), "{missing}");
        std::fs::write(dir.path().join("themes/two.toml"), "id = 3").unwrap();
        assert!(themes_of(dir.path(), &manifest(&["themes/two.toml"]))
            .unwrap_err()
            .contains("themes/two.toml"));
    }
}
