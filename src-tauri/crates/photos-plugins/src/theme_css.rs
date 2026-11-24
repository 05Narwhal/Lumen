//! A `.lumentheme`'s `theme.css`, checked (Round 26 goal 4). Not trusted: a small CSS reader that
//! accepts only rules on Lumen's published part hooks (`[data-part="…"]`), an allowlist of
//! properties and functions, and `url()`s naming the package's own fonts and images. Everything it
//! accepts is written back by Rust, scoped to the theme and kept off the protected parts, so the
//! page never receives a byte the theme wrote unchecked. A refusal names the line and the rule.
//! The lists are the contract documented in `docs/THEMES.md` (a test keeps them in lockstep).

use std::collections::BTreeSet;

/// Parts a theme may style. `shell` is a CSS grid with named areas (`titlebar`, `sidebar`, `main`).
pub const PARTS: &[&str] = &[
    "shell", "titlebar", "sidebar", "sidebar-item", "main", "toolbar", "header", "grid", "tile", "viewer",
    "panel", "card", "button", "input", "statusbar",
];

/// Parts nothing in a theme may select, hide, cover, move or relabel (nor anything inside them).
pub const PROTECTED: &[&str] = &["window-controls", "dialog", "confirm", "toast", "menu", "install-sheet", "palette"];

/// Parts that hold protected ones: a theme may lay them out but not fade, filter, transform or lift them.
const CONTAINERS: &[&str] = &["shell", "titlebar"];
const CONTAINER_REFUSED: &[&str] = &["opacity", "filter", "backdrop-filter", "transform", "translate", "scale", "rotate", "position"];

const PSEUDO: &[&str] = &["hover", "active", "focus-visible", "focus-within", "first-child", "last-child"];

/// Properties a theme may set: exact names, then prefixes (`border` covers `border-top-left-radius`).
const PROPS: &[&str] = &[
    "display", "order", "gap", "row-gap", "column-gap", "width", "height", "min-width", "min-height", "max-width",
    "max-height", "aspect-ratio", "box-sizing", "overflow", "overflow-x", "overflow-y", "position", "top", "right",
    "bottom", "left", "box-shadow", "color", "font-family", "font-size", "font-weight", "font-style", "letter-spacing",
    "line-height", "text-transform", "text-align", "text-shadow", "filter", "backdrop-filter", "transform",
    "transform-origin", "translate", "scale", "rotate", "opacity", "mix-blend-mode", "isolation", "object-fit",
];
const PROP_PREFIXES: &[&str] = &[
    "flex", "grid", "align-", "justify-", "place-", "margin", "padding", "inset", "border", "outline", "background",
    "transition", "animation",
];

const FUNCS: &[&str] = &[
    "rgb", "rgba", "hsl", "hsla", "oklch", "oklab", "color-mix", "var", "calc", "min", "max", "clamp",
    "linear-gradient", "radial-gradient", "conic-gradient", "repeating-linear-gradient", "repeat", "minmax",
    "fit-content", "translate", "translatex", "translatey", "scale", "scalex", "scaley", "rotate", "skew", "skewx",
    "skewy", "blur", "brightness", "contrast", "saturate", "grayscale", "sepia", "hue-rotate", "drop-shadow",
    "cubic-bezier", "steps", "format", "url",
];

const MEDIA_FEATURES: &[&str] = &[
    "prefers-reduced-motion", "prefers-color-scheme", "prefers-contrast", "min-width", "max-width", "min-height",
    "max-height", "orientation",
];

const OFFSET_PROPS: &[&str] = &["top", "right", "bottom", "left", "translate", "transform"];
pub const MAX_FONT_BYTES: u64 = 512 * 1024;
pub const MAX_IMAGE_BYTES: u64 = 2 * 1024 * 1024;
const MAX_FAMILIES: usize = 2;
const MAX_CSS_BYTES: usize = 128 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refusal {
    pub line: u32,
    /// A short rule name, the same one `docs/THEMES.md` lists.
    pub rule: &'static str,
    pub message: String,
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "theme.css line {}: {} ({})", self.line, self.message, self.rule)
    }
}

fn refuse<T>(line: u32, rule: &'static str, message: impl Into<String>) -> Result<T, Refusal> {
    Err(Refusal { line, rule, message: message.into() })
}

/// What the checker needs from the package.
pub trait Files {
    /// The file's first bytes (at least 4) and size, `None` when the package hasn't it.
    fn head(&self, rel: &str) -> Option<(Vec<u8>, u64)>;
}

/// The checked CSS and the asset paths it names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Checked {
    pub css: String,
    pub assets: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Ident(String),
    At(String),
    Hash(String),
    Str(String),
    Num(String),
    Func(String),
    Url(String),
    Delim(char),
    Ws,
}

struct Lexed {
    tok: Tok,
    line: u32,
}

fn lex(src: &str) -> Result<Vec<Lexed>, Refusal> {
    let b: Vec<char> = src.chars().collect();
    let mut out = Vec::new();
    let (mut i, mut line) = (0usize, 1u32);
    let ident_char = |c: char| c.is_ascii_alphanumeric() || c == '-' || c == '_';
    while i < b.len() {
        let c = b[i];
        let at = line;
        if c == '\n' {
            line += 1;
        }
        if c == '/' && b.get(i + 1) == Some(&'*') {
            let mut j = i + 2;
            while j + 1 < b.len() && !(b[j] == '*' && b[j + 1] == '/') {
                if b[j] == '\n' {
                    line += 1;
                }
                j += 1;
            }
            if j + 1 >= b.len() {
                return refuse(at, "syntax", "a comment is never closed");
            }
            i = j + 2;
            continue;
        }
        if c.is_whitespace() {
            if !matches!(out.last(), Some(Lexed { tok: Tok::Ws, .. })) {
                out.push(Lexed { tok: Tok::Ws, line: at });
            }
            i += 1;
            continue;
        }
        if c == '\\' {
            return refuse(at, "escapes", "escapes (\\) aren't allowed");
        }
        if !c.is_ascii() || c == '<' || c == '`' {
            return refuse(at, "syntax", format!("the character {c:?} isn't allowed outside a string"));
        }
        if c == '"' || c == '\'' {
            let mut j = i + 1;
            let mut s = String::new();
            while j < b.len() && b[j] != c {
                if b[j] == '\\' {
                    return refuse(at, "escapes", "escapes (\\) aren't allowed");
                }
                if b[j] == '\n' || !(b[j].is_ascii_graphic() || b[j] == ' ') {
                    return refuse(at, "syntax", "a string may hold only plain printable text on one line");
                }
                s.push(b[j]);
                j += 1;
            }
            if j >= b.len() {
                return refuse(at, "syntax", "a string is never closed");
            }
            out.push(Lexed { tok: Tok::Str(s), line: at });
            i = j + 1;
            continue;
        }
        let starts_num = c.is_ascii_digit()
            || (c == '.' && b.get(i + 1).is_some_and(char::is_ascii_digit))
            || ((c == '-' || c == '+') && b.get(i + 1).is_some_and(|d| d.is_ascii_digit() || *d == '.'));
        if starts_num {
            let mut j = i + 1;
            while j < b.len() && (b[j].is_ascii_digit() || b[j] == '.') {
                j += 1;
            }
            while j < b.len() && (b[j].is_ascii_alphabetic() || b[j] == '%') {
                j += 1;
            }
            out.push(Lexed { tok: Tok::Num(b[i..j].iter().collect::<String>().to_ascii_lowercase()), line: at });
            i = j;
            continue;
        }
        if c == '@' || c == '#' || ident_char(c) {
            let start = if c == '@' || c == '#' { i + 1 } else { i };
            let mut j = start;
            while j < b.len() && ident_char(b[j]) {
                j += 1;
            }
            let word: String = b[start..j].iter().collect();
            if word.is_empty() {
                return refuse(at, "syntax", format!("a lone {c}"));
            }
            if c == '@' {
                out.push(Lexed { tok: Tok::At(word.to_ascii_lowercase()), line: at });
            } else if c == '#' {
                out.push(Lexed { tok: Tok::Hash(word), line: at });
            } else if b.get(j) == Some(&'(') {
                let name = word.to_ascii_lowercase();
                if name == "url" {
                    // url(path) unquoted, or url("path") as a function with one string.
                    let mut k = j + 1;
                    while k < b.len() && b[k].is_whitespace() {
                        k += 1;
                    }
                    if b.get(k) != Some(&'"') && b.get(k) != Some(&'\'') {
                        let mut s = String::new();
                        while k < b.len() && b[k] != ')' {
                            if b[k] == '\\' {
                                return refuse(at, "escapes", "escapes (\\) aren't allowed");
                            }
                            s.push(b[k]);
                            k += 1;
                        }
                        if k >= b.len() {
                            return refuse(at, "syntax", "url( is never closed");
                        }
                        out.push(Lexed { tok: Tok::Url(s.trim().to_string()), line: at });
                        i = k + 1;
                        continue;
                    }
                }
                out.push(Lexed { tok: Tok::Func(name), line: at });
                i = j + 1;
                continue;
            } else {
                out.push(Lexed { tok: Tok::Ident(word), line: at });
            }
            i = j;
            continue;
        }
        if "{}()[]:;,>+~*=/!.%|^$".contains(c) {
            out.push(Lexed { tok: Tok::Delim(c), line: at });
            i += 1;
            continue;
        }
        return refuse(at, "syntax", format!("the character {c:?} isn't allowed"));
    }
    Ok(out)
}

/// Checks `src` and writes it back scoped to `scope` (the `data-plugin-theme` value). `asset_url`
/// turns a package path into the address the page loads it from; `salt` keeps the theme's font and
/// animation names apart from the app's and other themes'.
pub fn check(src: &str, scope: &str, salt: &str, files: &dyn Files, asset_url: &dyn Fn(&str) -> String) -> Result<Checked, Refusal> {
    if src.len() > MAX_CSS_BYTES {
        return refuse(1, "size", "theme.css can be at most 128 KB");
    }
    let toks = lex(src)?;
    let hash = blake3::hash(salt.as_bytes()).to_hex();
    let mut p = Parser {
        toks,
        i: 0,
        scope: scope.replace(['"', '\\'], ""),
        prefix: format!("lt{}", &hash.as_str()[..8]),
        files,
        asset_url,
        keyframes: BTreeSet::new(),
        families: BTreeSet::new(),
        assets: BTreeSet::new(),
        out: String::new(),
    };
    // Names declared by @keyframes and @font-face are known before any rule uses them.
    for w in p.toks.windows(3) {
        if let (Tok::At(a), Tok::Ws, Tok::Ident(n)) = (&w[0].tok, &w[1].tok, &w[2].tok) {
            if a == "keyframes" {
                p.keyframes.insert(n.clone());
            }
        }
    }
    p.sheet(false)?;
    Ok(Checked { css: p.out, assets: p.assets.into_iter().collect() })
}

struct Parser<'a> {
    toks: Vec<Lexed>,
    i: usize,
    scope: String,
    prefix: String,
    files: &'a dyn Files,
    asset_url: &'a dyn Fn(&str) -> String,
    keyframes: BTreeSet<String>,
    families: BTreeSet<String>,
    assets: BTreeSet<String>,
    out: String,
}

impl Parser<'_> {
    fn line(&self) -> u32 {
        self.toks.get(self.i).or(self.toks.last()).map_or(1, |t| t.line)
    }
    fn peek(&self) -> Option<&Tok> {
        self.toks.get(self.i).map(|t| &t.tok)
    }
    fn skip_ws(&mut self) {
        while matches!(self.peek(), Some(Tok::Ws)) {
            self.i += 1;
        }
    }
    /// Tokens up to (not including) the first of `stops` at nesting depth 0.
    fn until(&mut self, stops: &[char]) -> Result<Vec<Lexed>, Refusal> {
        let mut out = Vec::new();
        let mut depth = 0i32;
        while let Some(t) = self.toks.get(self.i) {
            match &t.tok {
                Tok::Delim(c) if depth == 0 && stops.contains(c) => return Ok(out),
                Tok::Delim('(') | Tok::Func(_) | Tok::Delim('[') => depth += 1,
                Tok::Delim(')') | Tok::Delim(']') => depth -= 1,
                Tok::Delim('{') | Tok::Delim('}') => return refuse(t.line, "syntax", "a brace where it doesn't belong"),
                _ => {}
            }
            out.push(Lexed { tok: t.tok.clone(), line: t.line });
            self.i += 1;
        }
        Ok(out)
    }
    fn expect(&mut self, c: char) -> Result<(), Refusal> {
        self.skip_ws();
        if self.peek() == Some(&Tok::Delim(c)) {
            self.i += 1;
            Ok(())
        } else {
            refuse(self.line(), "syntax", format!("expected {c}"))
        }
    }

    fn sheet(&mut self, nested: bool) -> Result<(), Refusal> {
        loop {
            self.skip_ws();
            match self.peek() {
                None if nested => return refuse(self.line(), "syntax", "a block is never closed"),
                None => return Ok(()),
                Some(Tok::Delim('}')) if nested => return Ok(()),
                Some(Tok::At(a)) => {
                    let (a, line) = (a.clone(), self.line());
                    self.i += 1;
                    match a.as_str() {
                        "media" if !nested => self.media()?,
                        "keyframes" if !nested => self.keyframes_rule()?,
                        "font-face" if !nested => self.font_face()?,
                        "import" => return refuse(line, "import", "@import isn't allowed: a theme brings everything in its package"),
                        _ => return refuse(line, "at-rule", format!("@{a} isn't allowed here")),
                    }
                }
                Some(_) => self.style_rule()?,
            }
        }
    }

    fn style_rule(&mut self) -> Result<(), Refusal> {
        let line = self.line();
        let sel = self.until(&['{'])?;
        let (selector, subjects) = self.selector(&sel, line)?;
        self.expect('{')?;
        let decls = self.decls(&subjects)?;
        self.expect('}')?;
        if !decls.is_empty() {
            self.out.push_str(&format!("{selector} {{\n{decls}}}\n"));
        }
        Ok(())
    }

    /// The scoped selector list and the part each complex selector styles.
    fn selector(&self, toks: &[Lexed], line: u32) -> Result<(String, Vec<String>), Refusal> {
        let protected = PROTECTED.iter().map(|p| format!("[data-part=\"{p}\"]")).collect::<Vec<_>>().join(", ");
        let mut list = Vec::new();
        let mut subjects = Vec::new();
        for complex in toks.split(|t| t.tok == Tok::Delim(',')) {
            let mut out = String::new();
            let mut subject = None;
            let mut compound_has_part = false;
            let mut i = 0;
            let toks: Vec<&Lexed> = complex.iter().collect();
            let trimmed: Vec<&Lexed> = {
                let s = toks.iter().position(|t| t.tok != Tok::Ws).unwrap_or(toks.len());
                let e = toks.iter().rposition(|t| t.tok != Tok::Ws).map_or(s, |e| e + 1);
                toks[s..e.max(s)].to_vec()
            };
            if trimmed.is_empty() {
                return refuse(line, "selector", "an empty selector");
            }
            while i < trimmed.len() {
                let t = trimmed[i];
                match &t.tok {
                    Tok::Delim('[') => {
                        let (Some(n), Some(Tok::Delim('=')), Some(v), Some(Tok::Delim(']'))) = (
                            trimmed.get(i + 1).map(|t| &t.tok),
                            trimmed.get(i + 2).map(|t| &t.tok),
                            trimmed.get(i + 3).map(|t| &t.tok),
                            trimmed.get(i + 4).map(|t| &t.tok),
                        ) else {
                            return refuse(t.line, "selector", "only [data-part=\"…\"] attribute selectors are allowed");
                        };
                        let (Tok::Ident(n), Tok::Str(v) | Tok::Ident(v)) = (n, v) else {
                            return refuse(t.line, "selector", "only [data-part=\"…\"] attribute selectors are allowed");
                        };
                        if n != "data-part" {
                            return refuse(t.line, "selector", format!("[{n}] isn't a part hook; use [data-part=\"…\"]"));
                        }
                        if PROTECTED.contains(&v.as_str()) {
                            return refuse(t.line, "protected", format!("\"{v}\" is a protected part: a theme can't style it or anything in it"));
                        }
                        if !PARTS.contains(&v.as_str()) {
                            return refuse(t.line, "unknown-part", format!("\"{v}\" isn't one of Lumen's parts"));
                        }
                        out.push_str(&format!("[data-part=\"{v}\"]"));
                        subject = Some(v.clone());
                        compound_has_part = true;
                        i += 5;
                    }
                    Tok::Delim(':') => {
                        if !compound_has_part {
                            return refuse(t.line, "selector", "a pseudo-class must follow a [data-part] selector");
                        }
                        match trimmed.get(i + 1).map(|t| &t.tok) {
                            Some(Tok::Ident(p)) if PSEUDO.contains(&p.to_ascii_lowercase().as_str()) => {
                                out.push_str(&format!(":{}", p.to_ascii_lowercase()));
                                i += 2;
                            }
                            Some(Tok::Delim(':')) => return refuse(t.line, "selector", "pseudo-elements (::before, ::after) aren't allowed"),
                            _ => return refuse(t.line, "selector", format!("only these pseudo-classes are allowed: {}", PSEUDO.join(", "))),
                        }
                    }
                    Tok::Ws | Tok::Delim('>') | Tok::Delim('+') | Tok::Delim('~') => {
                        if !compound_has_part {
                            return refuse(t.line, "selector", "every compound selector starts with [data-part=\"…\"]");
                        }
                        let mut comb = ' ';
                        while let Some(t) = trimmed.get(i) {
                            match t.tok {
                                Tok::Ws => {}
                                Tok::Delim(c @ ('>' | '+' | '~')) if comb == ' ' => comb = c,
                                _ => break,
                            }
                            i += 1;
                        }
                        out.push_str(if comb == ' ' { " " } else { match comb { '>' => " > ", '+' => " + ", _ => " ~ " } });
                        compound_has_part = false;
                    }
                    _ => return refuse(t.line, "selector", "only [data-part=\"…\"] selectors are allowed (no tags, classes, ids or *)"),
                }
            }
            if !compound_has_part {
                return refuse(line, "selector", "a selector ends with a combinator");
            }
            // Never anything inside a protected part (a dialog's buttons are buttons too).
            list.push(format!("[data-plugin-theme=\"{}\"] {out}:not(:is({protected}) *)", self.scope));
            subjects.push(subject.unwrap_or_default());
        }
        Ok((list.join(",\n"), subjects))
    }

    fn decls(&mut self, subjects: &[String]) -> Result<String, Refusal> {
        let mut out = String::new();
        loop {
            self.skip_ws();
            match self.peek() {
                Some(Tok::Delim('}')) | None => return Ok(out),
                Some(Tok::Delim(';')) => {
                    self.i += 1;
                    continue;
                }
                _ => {}
            }
            let line = self.line();
            let Some(Tok::Ident(name)) = self.peek().cloned() else {
                return refuse(line, "syntax", "expected a property name");
            };
            self.i += 1;
            self.expect(':')?;
            let value = self.until(&[';', '}'])?;
            let name = name.to_ascii_lowercase();
            let v = self.value(&name, &value, subjects, line)?;
            out.push_str(&format!("  {name}: {v};\n"));
        }
    }

    fn value(&mut self, name: &str, toks: &[Lexed], subjects: &[String], line: u32) -> Result<String, Refusal> {
        if name.starts_with("--") {
            return refuse(line, "property", "custom properties aren't allowed in theme.css; set tokens in theme.toml");
        }
        if name == "content" {
            return refuse(line, "content", "content: isn't allowed: a theme can't add or change text");
        }
        if !(PROPS.contains(&name) || PROP_PREFIXES.iter().any(|p| name.starts_with(p))) {
            return refuse(line, "property", format!("{name} isn't a property themes may set"));
        }
        if let Some(s) = subjects.iter().find(|s| CONTAINERS.contains(&s.as_str())) {
            if CONTAINER_REFUSED.contains(&name) {
                return refuse(line, "container", format!("{name} isn't allowed on \"{s}\": it holds the window controls"));
            }
        }
        let words: Vec<String> = toks.iter().filter_map(|t| match &t.tok { Tok::Ident(w) => Some(w.to_ascii_lowercase()), _ => None }).collect();
        let has = |w: &str| words.iter().any(|x| x == w);
        if toks.iter().any(|t| t.tok == Tok::Delim('!')) {
            return refuse(line, "important", "!important isn't allowed");
        }
        if name == "display" && has("none") {
            return refuse(line, "hiding", "display: none isn't allowed: a theme moves things, it never hides them");
        }
        if name == "position" && has("fixed") {
            return refuse(line, "fixed", "position: fixed isn't allowed: it could cover the title bar");
        }
        let mut out = String::new();
        let mut prev_ws = false;
        let mut i = 0;
        let mut funcs: Vec<String> = Vec::new();
        while i < toks.len() {
            let t = &toks[i];
            let line = t.line;
            let piece = match &t.tok {
                Tok::Ws => {
                    prev_ws = true;
                    i += 1;
                    continue;
                }
                Tok::Ident(w) => {
                    let w = w.to_ascii_lowercase();
                    if w == "expression" || w == "javascript" {
                        return refuse(line, "function", format!("{w} isn't allowed"));
                    }
                    if name.starts_with("animation") && self.keyframes.contains(&w) {
                        format!("{}-{w}", self.prefix)
                    } else {
                        w
                    }
                }
                Tok::Hash(h) => format!("#{h}"),
                Tok::Num(n) => {
                    self.check_number(name, n, funcs.last().map(String::as_str), line)?;
                    n.clone()
                }
                Tok::Str(s) => {
                    if name == "font-family" {
                        if !self.families.contains(s) {
                            return refuse(line, "font", format!("the font \"{s}\" isn't one this theme declares with @font-face"));
                        }
                        format!("\"{}-{}\"", self.prefix, s)
                    } else if name.starts_with("grid-template") || name == "grid-area" {
                        if !s.chars().all(|c| c.is_ascii_alphanumeric() || c == ' ' || c == '-' || c == '.') {
                            return refuse(line, "syntax", "grid area names are letters, digits, - and .");
                        }
                        format!("\"{s}\"")
                    } else {
                        return refuse(line, "content", "text in a value isn't allowed here");
                    }
                }
                Tok::Url(u) => self.url(u, name, line)?,
                Tok::Func(f) if f == "url" => {
                    let (Some(Lexed { tok: Tok::Str(u), .. }), Some(Lexed { tok: Tok::Delim(')'), .. })) = (toks.get(i + 1), toks.get(i + 2)) else {
                        return refuse(line, "url", "url(\"…\") takes one path");
                    };
                    let u = u.clone();
                    i += 2;
                    self.url(&u, name, line)?
                }
                Tok::Func(f) => {
                    if !FUNCS.contains(&f.as_str()) {
                        return refuse(line, "function", format!("{f}() isn't allowed"));
                    }
                    if f == "var" {
                        let Some(Lexed { tok: Tok::Ident(v), .. }) = toks[i + 1..].iter().find(|t| t.tok != Tok::Ws) else {
                            return refuse(line, "var", "var() names a Lumen token");
                        };
                        let known = v.strip_prefix("--").and_then(crate::theme::token_type).is_some();
                        if !known {
                            return refuse(line, "var", format!("var({v}) isn't one of Lumen's tokens"));
                        }
                    }
                    funcs.push(f.clone());
                    format!("{f}(")
                }
                Tok::Delim(c @ (',' | '/' | '(' | '%' | '+' | '-' | '*')) => {
                    if *c == '(' {
                        funcs.push(String::new());
                    }
                    c.to_string()
                }
                Tok::Delim(')') => {
                    funcs.pop();
                    ")".into()
                }
                Tok::Delim('.') if matches!(toks.get(i + 1).map(|t| &t.tok), Some(Tok::Ident(_))) => {
                    return refuse(line, "syntax", "a value can't hold a selector");
                }
                Tok::At(_) | Tok::Delim(_) => return refuse(line, "syntax", "this isn't a value"),
            };
            if prev_ws && !out.is_empty() && !out.ends_with('(') && piece != ")" && piece != "," {
                out.push(' ');
            }
            out.push_str(&piece);
            prev_ws = false;
            i += 1;
        }
        if out.is_empty() {
            return refuse(line, "syntax", format!("{name} has no value"));
        }
        Ok(out)
    }

    fn check_number(&self, name: &str, n: &str, func: Option<&str>, line: u32) -> Result<(), Refusal> {
        let digits = n.trim_end_matches(|c: char| c.is_ascii_alphabetic() || c == '%');
        let unit = &n[digits.len()..];
        let Ok(x) = digits.parse::<f32>() else {
            return refuse(line, "syntax", format!("{n} isn't a number"));
        };
        if name == "opacity" && (if unit == "%" { x / 100.0 } else { x }) < 0.5 {
            return refuse(line, "hiding", "opacity below 0.5 isn't allowed");
        }
        if name == "font-size" && ((unit == "px" && x < 9.0) || ((unit == "em" || unit == "rem") && x < 0.6) || (unit == "%" && x < 60.0)) {
            return refuse(line, "hiding", "text smaller than 9 px isn't allowed");
        }
        let length = matches!(unit, "px" | "%" | "em" | "rem" | "vw" | "vh" | "vmin" | "vmax");
        let offset = OFFSET_PROPS.contains(&name) || name.starts_with("inset") || name.starts_with("margin");
        if offset && length && x < 0.0 && !(unit == "px" && x >= -64.0) {
            return refuse(line, "off-screen", format!("{n} could move a part off-screen (at most -64px)"));
        }
        if (name == "scale" || func.is_some_and(|f| f.starts_with("scale"))) && unit.is_empty() && x < 0.5 {
            return refuse(line, "hiding", "a scale below 0.5 isn't allowed");
        }
        Ok(())
    }

    fn url(&mut self, u: &str, prop: &str, line: u32) -> Result<String, Refusal> {
        if u.contains(':') || u.starts_with("//") {
            let rule = if u.trim_start().to_ascii_lowercase().starts_with("data:") { "data-url" } else { "remote-url" };
            return refuse(line, rule, "url() may name only a file in the theme's package (no web or data: addresses)");
        }
        if !crate::theme::safe_rel_path(u) {
            return refuse(line, "url", format!("{u} isn't a path inside the package"));
        }
        let lower = u.to_ascii_lowercase();
        let font = prop == "src";
        let (dir_ok, max, magic): (bool, u64, &[&[u8]]) = if font {
            (lower.starts_with("fonts/") && lower.ends_with(".woff2"), MAX_FONT_BYTES, &[b"wOF2"])
        } else {
            let image = lower.starts_with("images/") && [".png", ".jpg", ".jpeg", ".webp"].iter().any(|e| lower.ends_with(e));
            (image, MAX_IMAGE_BYTES, &[b"\x89PNG", b"\xff\xd8\xff", b"RIFF"])
        };
        if !dir_ok {
            return refuse(line, "url", if font { "a font is a fonts/*.woff2 file" } else { "an image is an images/*.png, .jpg or .webp file" });
        }
        let Some((head, size)) = self.files.head(u) else {
            return refuse(line, "url", format!("{u} isn't in the package"));
        };
        if size > max {
            return refuse(line, "url", format!("{u} is over {} KB", max / 1024));
        }
        if !magic.iter().any(|m| head.starts_with(m)) {
            return refuse(line, "url", format!("{u} isn't the file its name says"));
        }
        self.assets.insert(u.to_string());
        Ok(format!("url(\"{}\")", (self.asset_url)(u)))
    }

    fn media(&mut self) -> Result<(), Refusal> {
        let line = self.line();
        let prelude = self.until(&['{'])?;
        let mut out = String::new();
        let mut i = 0;
        while i < prelude.len() {
            let piece = match &prelude[i].tok {
                Tok::Ws => " ".to_string(),
                Tok::Ident(w) if ["and", "screen", "not", "only"].contains(&w.as_str()) => w.clone(),
                Tok::Delim(c @ ('(' | ')' | ':' | ',')) => c.to_string(),
                Tok::Ident(w) if MEDIA_FEATURES.contains(&w.as_str()) => w.clone(),
                Tok::Ident(w) if ["reduce", "no-preference", "light", "dark", "more", "less", "portrait", "landscape"].contains(&w.as_str()) => w.clone(),
                Tok::Num(n) if n.ends_with("px") || n.ends_with("em") => n.clone(),
                _ => return refuse(prelude[i].line, "media", "only width, height, orientation and prefers-* media queries are allowed"),
            };
            out.push_str(&piece);
            i += 1;
        }
        self.expect('{')?;
        let outer = std::mem::take(&mut self.out);
        self.sheet(true)?;
        let inner = std::mem::replace(&mut self.out, outer);
        self.expect('}')?;
        if out.trim().is_empty() {
            return refuse(line, "media", "@media needs a query");
        }
        self.out.push_str(&format!("@media {} {{\n{inner}}}\n", out.split_whitespace().collect::<Vec<_>>().join(" ")));
        Ok(())
    }

    fn keyframes_rule(&mut self) -> Result<(), Refusal> {
        self.skip_ws();
        let line = self.line();
        let Some(Tok::Ident(name)) = self.peek().cloned() else {
            return refuse(line, "syntax", "@keyframes needs a name");
        };
        self.i += 1;
        self.expect('{')?;
        let mut body = String::new();
        loop {
            self.skip_ws();
            if self.peek() == Some(&Tok::Delim('}')) {
                self.i += 1;
                break;
            }
            let at = self.line();
            let stop = self.until(&['{'])?;
            let mut stops = Vec::new();
            for t in &stop {
                match &t.tok {
                    Tok::Ident(w) if w == "from" || w == "to" => stops.push(w.clone()),
                    Tok::Num(n) if n.ends_with('%') => stops.push(n.clone()),
                    Tok::Ws | Tok::Delim(',') => {}
                    _ => return refuse(t.line, "syntax", "a keyframe is from, to or a percentage"),
                }
            }
            if stops.is_empty() {
                return refuse(at, "syntax", "a keyframe is from, to or a percentage");
            }
            self.expect('{')?;
            let decls = self.decls(&[])?;
            self.expect('}')?;
            body.push_str(&format!("  {} {{\n{decls}  }}\n", stops.join(", ")));
        }
        self.out.push_str(&format!("@keyframes {}-{name} {{\n{body}}}\n", self.prefix));
        Ok(())
    }

    fn font_face(&mut self) -> Result<(), Refusal> {
        let line = self.line();
        self.expect('{')?;
        let mut family = None;
        let mut src = None;
        let mut rest = String::new();
        loop {
            self.skip_ws();
            match self.peek() {
                Some(Tok::Delim('}')) => {
                    self.i += 1;
                    break;
                }
                Some(Tok::Delim(';')) => {
                    self.i += 1;
                    continue;
                }
                None => return refuse(line, "syntax", "@font-face is never closed"),
                _ => {}
            }
            let at = self.line();
            let Some(Tok::Ident(name)) = self.peek().cloned() else {
                return refuse(at, "syntax", "expected a descriptor");
            };
            self.i += 1;
            self.expect(':')?;
            let v = self.until(&[';', '}'])?;
            let v: Vec<&Lexed> = v.iter().filter(|t| t.tok != Tok::Ws).collect();
            match (name.to_ascii_lowercase().as_str(), v.as_slice()) {
                ("font-family", [Lexed { tok: Tok::Str(f), .. }]) if !f.trim().is_empty() => family = Some(f.clone()),
                ("src", [Lexed { tok: Tok::Url(u), .. }, rest @ ..]) | ("src", [Lexed { tok: Tok::Func(_), .. }, Lexed { tok: Tok::Str(u), .. }, Lexed { tok: Tok::Delim(')'), .. }, rest @ ..]) => {
                    let ok_rest = rest.is_empty()
                        || matches!(rest, [Lexed { tok: Tok::Func(f), .. }, Lexed { tok: Tok::Str(s), .. }, Lexed { tok: Tok::Delim(')'), .. }] if f == "format" && s == "woff2");
                    if !ok_rest {
                        return refuse(at, "font", "src is one url(fonts/….woff2), optionally with format(\"woff2\")");
                    }
                    src = Some(self.url(&u.clone(), "src", at)?);
                }
                ("font-weight", [Lexed { tok: Tok::Num(n), .. }]) if n.parse::<u16>().is_ok_and(|w| (100..=900).contains(&w)) => {
                    rest.push_str(&format!("  font-weight: {n};\n"))
                }
                ("font-style", [Lexed { tok: Tok::Ident(s), .. }]) if s == "normal" || s == "italic" => rest.push_str(&format!("  font-style: {s};\n")),
                ("font-display", [Lexed { tok: Tok::Ident(s), .. }]) if ["swap", "block", "fallback", "optional", "auto"].contains(&s.as_str()) => {
                    rest.push_str(&format!("  font-display: {s};\n"))
                }
                (n, _) => return refuse(at, "font", format!("@font-face {n} isn't allowed or isn't written as expected")),
            }
        }
        let (Some(family), Some(src)) = (family, src) else {
            return refuse(line, "font", "@font-face needs font-family and src");
        };
        if !family.chars().all(|c| c.is_ascii_alphanumeric() || c == ' ' || c == '-') {
            return refuse(line, "font", "a font family name is letters, digits, spaces and -");
        }
        self.families.insert(family.clone());
        if self.families.len() > MAX_FAMILIES {
            return refuse(line, "font", "a theme can bring at most 2 font families");
        }
        self.out.push_str(&format!("@font-face {{\n  font-family: \"{}-{family}\";\n  src: {src} format(\"woff2\");\n{rest}}}\n", self.prefix));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    struct Pkg(BTreeMap<&'static str, Vec<u8>>);
    impl Files for Pkg {
        fn head(&self, rel: &str) -> Option<(Vec<u8>, u64)> {
            self.0.get(rel).map(|b| (b.iter().take(8).copied().collect(), b.len() as u64))
        }
    }
    fn pkg() -> Pkg {
        let mut m = BTreeMap::new();
        m.insert("fonts/serif.woff2", b"wOF2....".to_vec());
        m.insert("fonts/sans.woff2", b"wOF2....".to_vec());
        m.insert("fonts/third.woff2", b"wOF2....".to_vec());
        m.insert("fonts/big.woff2", [b"wOF2".to_vec(), vec![0; 600 * 1024]].concat());
        m.insert("fonts/fake.woff2", b"PK\x03\x04".to_vec());
        m.insert("images/grain.png", b"\x89PNG\r\n\x1a\n".to_vec());
        Pkg(m)
    }
    fn run(css: &str) -> Result<Checked, Refusal> {
        check(css, "studio.test.t/theme", "studio.test.t", &pkg(), &|p| format!("http://photos.localhost/theme-asset/studio.test.t/{p}"))
    }
    fn refused(css: &str) -> (u32, &'static str) {
        let r = run(css).expect_err(css);
        (r.line, r.rule)
    }

    #[test]
    fn a_layout_theme_is_written_back_scoped() {
        let css = "/* top bar */\n[data-part=\"shell\"] {\n  grid-template-areas: \"titlebar\" \"sidebar\" \"main\";\n  grid-template-columns: 1fr;\n}\n\
                   [data-part=\"sidebar\"] > [data-part=\"sidebar-item\"]:hover { background: url(images/grain.png), var(--bg-raised); border-radius: 8px }\n\
                   @font-face { font-family: \"Serif\"; src: url(fonts/serif.woff2) format(\"woff2\"); font-weight: 600 }\n\
                   [data-part=\"panel\"] { font-family: \"Serif\", serif; animation: rise 200ms ease-out }\n\
                   @keyframes rise { from { transform: translateY(8px) } to { transform: none } }\n\
                   @media (prefers-reduced-motion: reduce) { [data-part=\"panel\"] { animation: none } }\n";
        let out = run(css).unwrap();
        let scope = "[data-plugin-theme=\"studio.test.t/theme\"] ";
        assert!(out.css.contains(&format!("{scope}[data-part=\"shell\"]:not(:is(")), "{}", out.css);
        assert!(out.css.contains("grid-template-areas: \"titlebar\" \"sidebar\" \"main\";"), "{}", out.css);
        assert!(out.css.contains("[data-part=\"sidebar\"] > [data-part=\"sidebar-item\"]:hover"), "{}", out.css);
        assert!(out.css.contains("url(\"http://photos.localhost/theme-asset/studio.test.t/images/grain.png\"), var(--bg-raised)"), "{}", out.css);
        assert!(out.css.contains("[data-part=\"dialog\"]") && out.css.contains("[data-part=\"window-controls\"]"));
        let prefix = format!("lt{}", &blake3::hash(b"studio.test.t").to_hex().as_str()[..8]);
        assert!(out.css.contains(&format!("font-family: \"{prefix}-Serif\", serif")), "{}", out.css);
        assert!(out.css.contains(&format!("@keyframes {prefix}-rise")) && out.css.contains(&format!("animation: {prefix}-rise 200ms ease-out")), "{}", out.css);
        assert!(out.css.contains("@media (prefers-reduced-motion: reduce) {"), "{}", out.css);
        assert_eq!(out.assets, vec!["fonts/serif.woff2".to_string(), "images/grain.png".to_string()]);
    }

    #[test]
    fn every_refusal_names_its_line_and_rule() {
        let p = "[data-part=\"panel\"]";
        let cases: &[(String, (u32, &str))] = &[
            (format!("\n\n{p} {{ background: url(https://x.example/a.png) }}"), (3, "remote-url")),
            (format!("{p} {{ background: url(//x.example/a.png) }}"), (1, "remote-url")),
            (format!("{p} {{\n background: url(\"data:image/png;base64,AAAA\") }}"), (2, "data-url")),
            ("@import \"other.css\";".into(), (1, "import")),
            ("\n@namespace x;".into(), (2, "at-rule")),
            (format!("{p} {{ content: \"Delete\" }}"), (1, "content")),
            ("[data-part=\"dialog\"] { color: red }".into(), (1, "protected")),
            ("[data-part=\"panel\"] [data-part=\"confirm\"] { color: red }".into(), (1, "protected")),
            ("[data-part=\"toast\"] { color: red }".into(), (1, "protected")),
            ("[data-part=\"window-controls\"] { color: red }".into(), (1, "protected")),
            ("[data-part=\"nope\"] { color: red }".into(), (1, "unknown-part")),
            ("button { color: red }".into(), (1, "selector")),
            (".title { color: red }".into(), (1, "selector")),
            ("* { color: red }".into(), (1, "selector")),
            ("[data-part=\"panel\"] div { color: red }".into(), (1, "selector")),
            ("[aria-label=\"Delete\"] { color: red }".into(), (1, "selector")),
            ("[data-part=\"panel\"]::before { color: red }".into(), (1, "selector")),
            (format!("{p} {{\n  display: none\n}}"), (2, "hiding")),
            (format!("{p} {{ visibility: hidden }}"), (1, "property")),
            (format!("{p} {{ opacity: 0 }}"), (1, "hiding")),
            (format!("{p} {{ opacity: 20% }}"), (1, "hiding")),
            (format!("{p} {{ transform: scale(0.1) }}"), (1, "hiding")),
            (format!("{p} {{ font-size: 2px }}"), (1, "hiding")),
            (format!("{p} {{ left: -9999px }}"), (1, "off-screen")),
            (format!("{p} {{ transform: translateX(-100vw) }}"), (1, "off-screen")),
            (format!("{p} {{ margin-top: -500px }}"), (1, "off-screen")),
            (format!("{p} {{ pointer-events: none }}"), (1, "property")),
            (format!("{p} {{ z-index: 9999 }}"), (1, "property")),
            (format!("{p} {{ position: fixed }}"), (1, "fixed")),
            ("[data-part=\"shell\"] { opacity: 0.9 }".into(), (1, "container")),
            ("[data-part=\"titlebar\"] { transform: rotate(1deg) }".into(), (1, "container")),
            (format!("{p} {{ color: red !important }}"), (1, "important")),
            (format!("{p} {{ color: r\\65 d }}"), (1, "escapes")),
            (format!("{p} {{ --accent: red }}"), (1, "property")),
            (format!("{p} {{ color: var(--evil) }}"), (1, "var")),
            (format!("{p} {{ width: expression(alert(1)) }}"), (1, "function")),
            (format!("{p} {{ background: image-set(\"a.png\" 1x) }}"), (1, "function")),
            (format!("{p} {{ background: url(images/missing.png) }}"), (1, "url")),
            (format!("{p} {{ background: url(../secret.png) }}"), (1, "url")),
            (format!("{p} {{ background: url(fonts/serif.woff2) }}"), (1, "url")),
            ("@font-face { font-family: \"A\"; src: url(fonts/big.woff2) }".into(), (1, "url")),
            ("@font-face { font-family: \"A\"; src: url(fonts/fake.woff2) }".into(), (1, "url")),
            ("@font-face { font-family: \"A\"; src: url(fonts/serif.woff2) }\n@font-face { font-family: \"B\"; src: url(fonts/sans.woff2) }\n@font-face { font-family: \"C\"; src: url(fonts/third.woff2) }".into(), (3, "font")),
            (format!("{p} {{ font-family: \"Inter Variable\" }}"), (1, "font")),
            ("@media print { [data-part=\"panel\"] { color: red } }".into(), (1, "media")),
            (format!("{p} {{ color: red"), (1, "syntax")),
            (format!("{p} {{ color: <b> }}"), (1, "syntax")),
        ];
        for (css, want) in cases {
            assert_eq!(refused(css), *want, "{css}");
        }
    }

    #[test]
    fn a_rule_never_reaches_inside_a_protected_part() {
        let out = run("[data-part=\"button\"] { border-radius: 4px }").unwrap();
        for p in PROTECTED {
            assert!(out.css.contains(&format!("[data-part=\"{p}\"]")), "{p}: {}", out.css);
        }
        assert!(out.css.contains(") *)"), "{}", out.css);
    }

    #[test]
    fn the_part_lists_match_the_docs_and_the_golden() {
        let repo = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../..");
        let golden = serde_json::json!({ "parts": PARTS, "protected": PROTECTED });
        let path = repo.join("src/assets/theme-parts.json");
        let text = format!("{}\n", serde_json::to_string_pretty(&golden).unwrap());
        // Regenerated here, never by hand (like the other goldens); B2's test reads it.
        if std::fs::read_to_string(&path).ok().as_deref() != Some(text.as_str()) {
            std::fs::write(&path, &text).unwrap();
        }
        let docs = std::fs::read_to_string(repo.join("docs/THEMES.md")).unwrap();
        for p in PARTS.iter().chain(PROTECTED) {
            assert!(docs.contains(&format!("`{p}`")), "docs/THEMES.md doesn't list the part {p}");
        }
    }
}
