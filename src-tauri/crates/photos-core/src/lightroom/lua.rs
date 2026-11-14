//! The Lua tables Lightroom writes: a `.lrtemplate` preset (`s = { … value = { settings = { … } } }`)
//! and a catalog's `Adobe_imageDevelopSettings.text` (`s = { … }`). Not a Lua interpreter:
//! tables, strings, numbers and booleans.

use super::{Settings, Value};
use crate::{CoreError, Result};

/// `s = { … }`, `return { … }` or a bare `{ … }`, as settings.
pub fn read_lua_table(text: &str) -> Result<Settings> {
    let mut p = Parser { b: text.as_bytes(), at: 0 };
    p.space();
    // `s =` or `return` before the table.
    if p.peek() != Some(b'{') {
        let word = p.name().ok_or_else(|| p.error("a table"))?;
        if word != "return" {
            p.expect(b'=')?;
        }
        p.space();
    }
    let value = p.table()?;
    p.space();
    if p.at != p.b.len() {
        return Err(p.error("the end"));
    }
    match value {
        Value::Table(t) => Ok(t),
        _ => Err(p.error("named settings")),
    }
}

/// A `.lrtemplate`'s develop settings (`value.settings`), with its `title` as `Name`.
pub fn read_lrtemplate(text: &str) -> Result<Settings> {
    let mut top = read_lua_table(text)?;
    let Some(Value::Table(mut value)) = top.remove("value") else {
        return Err(CoreError::Invalid("lrtemplate: no value.settings".into()));
    };
    let Some(Value::Table(mut settings)) = value.remove("settings") else {
        return Err(CoreError::Invalid("lrtemplate: no value.settings".into()));
    };
    if !settings.contains_key("Name") {
        if let Some(Value::Str(title)) = top.remove("title").or_else(|| top.remove("internalName")) {
            settings.insert("Name".into(), Value::Str(title));
        }
    }
    Ok(settings)
}

struct Parser<'a> {
    b: &'a [u8],
    at: usize,
}

impl Parser<'_> {
    fn error(&self, wanted: &str) -> CoreError {
        CoreError::Invalid(format!("Lua table: expected {wanted} at byte {}", self.at))
    }

    fn peek(&self) -> Option<u8> {
        self.b.get(self.at).copied()
    }

    /// Spaces and `--` comments.
    fn space(&mut self) {
        loop {
            while self.peek().is_some_and(|c| c.is_ascii_whitespace()) {
                self.at += 1;
            }
            if self.b[self.at..].starts_with(b"--") {
                while self.peek().is_some_and(|c| c != b'\n') {
                    self.at += 1;
                }
            } else {
                return;
            }
        }
    }

    fn expect(&mut self, c: u8) -> Result<()> {
        self.space();
        if self.peek() == Some(c) {
            self.at += 1;
            Ok(())
        } else {
            Err(self.error(&format!("'{}'", c as char)))
        }
    }

    fn name(&mut self) -> Option<String> {
        let start = self.at;
        if !self.peek().is_some_and(|c| c.is_ascii_alphabetic() || c == b'_') {
            return None;
        }
        while self.peek().is_some_and(|c| c.is_ascii_alphanumeric() || c == b'_') {
            self.at += 1;
        }
        Some(String::from_utf8_lossy(&self.b[start..self.at]).into_owned())
    }

    fn value(&mut self) -> Result<Value> {
        self.space();
        match self.peek() {
            Some(b'{') => self.table(),
            Some(b'"' | b'\'') => self.string().map(Value::Str),
            Some(b'[') if self.b[self.at..].starts_with(b"[[") => self.long_string().map(Value::Str),
            Some(c) if c == b'-' || c == b'+' || c == b'.' || c.is_ascii_digit() => self.number(),
            Some(_) => match self.name().as_deref() {
                Some("true") => Ok(Value::Bool(true)),
                Some("false") => Ok(Value::Bool(false)),
                Some("nil") => Ok(Value::Str(String::new())),
                _ => Err(self.error("a value")),
            },
            None => Err(self.error("a value")),
        }
    }

    fn number(&mut self) -> Result<Value> {
        let start = self.at;
        if matches!(self.peek(), Some(b'-' | b'+')) {
            self.at += 1;
        }
        while self.peek().is_some_and(|c| c.is_ascii_digit() || c == b'.') {
            self.at += 1;
        }
        if matches!(self.peek(), Some(b'e' | b'E')) {
            self.at += 1;
            if matches!(self.peek(), Some(b'-' | b'+')) {
                self.at += 1;
            }
            while self.peek().is_some_and(|c| c.is_ascii_digit()) {
                self.at += 1;
            }
        }
        match std::str::from_utf8(&self.b[start..self.at]).ok().and_then(|t| t.parse::<f64>().ok()) {
            Some(n) => Ok(Value::Num(n)),
            None => {
                self.at = start;
                Err(self.error("a number"))
            }
        }
    }

    fn string(&mut self) -> Result<String> {
        let quote = self.b[self.at];
        self.at += 1;
        let mut out: Vec<u8> = Vec::new();
        loop {
            let c = self.peek().ok_or_else(|| self.error("the string's end"))?;
            self.at += 1;
            match c {
                c if c == quote => break,
                b'\\' => {
                    let e = self.peek().ok_or_else(|| self.error("an escape"))?;
                    self.at += 1;
                    match e {
                        b'n' => out.push(b'\n'),
                        b't' => out.push(b'\t'),
                        b'r' => out.push(b'\r'),
                        b'0'..=b'9' => {
                            // \ddd, up to three digits.
                            let mut v = (e - b'0') as u32;
                            for _ in 0..2 {
                                match self.peek() {
                                    Some(d @ b'0'..=b'9') => {
                                        v = v * 10 + (d - b'0') as u32;
                                        self.at += 1;
                                    }
                                    _ => break,
                                }
                            }
                            out.push(v.min(255) as u8);
                        }
                        other => out.push(other),
                    }
                }
                c => out.push(c),
            }
        }
        Ok(String::from_utf8_lossy(&out).into_owned())
    }

    /// `[[ … ]]`.
    fn long_string(&mut self) -> Result<String> {
        self.at += 2;
        let rest = &self.b[self.at..];
        let end = rest.windows(2).position(|w| w == b"]]").ok_or_else(|| self.error("']]'"))?;
        let s = String::from_utf8_lossy(&rest[..end]).into_owned();
        self.at += end + 2;
        Ok(s)
    }

    /// `{ … }`: named entries make a table, positional ones a list (an empty one a table).
    fn table(&mut self) -> Result<Value> {
        self.expect(b'{')?;
        let mut named = Settings::new();
        let mut list: Vec<Value> = Vec::new();
        loop {
            self.space();
            match self.peek() {
                Some(b'}') => {
                    self.at += 1;
                    break;
                }
                None => return Err(self.error("'}'")),
                _ => {}
            }
            // `[ "key" ] = v`, `name = v`, or `v`.
            let key = if self.peek() == Some(b'[') && !self.b[self.at..].starts_with(b"[[") {
                self.at += 1;
                let key = match self.value()? {
                    Value::Str(s) => s,
                    Value::Num(n) => n.to_string(),
                    _ => return Err(self.error("a key")),
                };
                self.expect(b']')?;
                self.expect(b'=')?;
                Some(key)
            } else {
                let save = self.at;
                match self.name() {
                    Some(n) if !matches!(n.as_str(), "true" | "false" | "nil") => {
                        self.expect(b'=')?;
                        Some(n)
                    }
                    _ => {
                        self.at = save;
                        None
                    }
                }
            };
            let value = self.value()?;
            match key {
                Some(k) => {
                    named.insert(k, value);
                }
                None => list.push(value),
            }
            self.space();
            match self.peek() {
                Some(b',' | b';') => self.at += 1,
                Some(b'}') => {}
                _ => return Err(self.error("',' or '}'")),
            }
        }
        if named.is_empty() && !list.is_empty() {
            return Ok(Value::List(list));
        }
        for (i, v) in list.into_iter().enumerate() {
            named.insert((i + 1).to_string(), v);
        }
        Ok(Value::Table(named))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_a_catalogs_develop_text() {
        let s = read_lua_table(
            r#"s = { AutoLateralCA = 0,
	Exposure2012 = -0.35,
	Name = "With \"quotes\" and \\ backslash",
	HasCrop = true, ConvertToGrayscale = false,
	Tiny = 1.5e-3, Neg = -2E2,
	ToneCurvePV2012 = { 0, 0, 64, 50, 255, 255, },
	[ "key with spaces" ] = 1;
	Nested = { Inner = { Deep = "x" }, },
}"#,
        )
        .unwrap();
        assert_eq!(s["AutoLateralCA"], Value::Num(0.0));
        assert_eq!(s["Exposure2012"], Value::Num(-0.35));
        assert_eq!(s["Name"], Value::Str(r#"With "quotes" and \ backslash"#.into()));
        assert_eq!(s["HasCrop"], Value::Bool(true));
        assert_eq!(s["ConvertToGrayscale"], Value::Bool(false));
        assert_eq!(s["Tiny"], Value::Num(0.0015));
        assert_eq!(s["Neg"], Value::Num(-200.0));
        assert_eq!(s["ToneCurvePV2012"], Value::List([0.0, 0.0, 64.0, 50.0, 255.0, 255.0].map(Value::Num).to_vec()));
        assert_eq!(s["key with spaces"], Value::Num(1.0));
        let Value::Table(nested) = &s["Nested"] else { panic!("{:?}", s["Nested"]) };
        let Value::Table(inner) = &nested["Inner"] else { panic!() };
        assert_eq!(inner["Deep"], Value::Str("x".into()));
        // A bare table and `return` read the same.
        assert_eq!(read_lua_table("{ A = 1 }").unwrap()["A"], Value::Num(1.0));
        assert_eq!(read_lua_table("return { A = 'one' }").unwrap()["A"], Value::Str("one".into()));
    }

    #[test]
    fn reads_an_lrtemplate_s_settings_and_title() {
        let s = read_lrtemplate(
            r#"s = {
	id = "8E0A5B1C-0000-4000-8000-000000000001",
	internalName = "Soft Portrait",
	title = "Soft Portrait",
	type = "Develop",
	value = {
		settings = {
			Vibrance = 15,
			ProcessVersion = "11.0",
		},
		uuid = "8E0A5B1C-0000-4000-8000-000000000001",
	},
	version = 0,
}
"#,
        )
        .unwrap();
        assert_eq!(s["Vibrance"], Value::Num(15.0));
        assert_eq!(s["Name"], Value::Str("Soft Portrait".into()));
        assert!(!s.contains_key("uuid"));
    }

    #[test]
    fn what_is_no_table_is_an_error_not_a_panic() {
        for bad in ["", "s = ", "s = { A = ", "{ A = \"open }", "{ = 1 }", "{ A = 1 } trailing", "{ [1 = 2 }", "x"] {
            let e = read_lua_table(bad).unwrap_err();
            assert_eq!(e.code(), "invalid", "{bad:?}");
        }
        assert_eq!(read_lrtemplate("s = { title = \"x\" }").unwrap_err().code(), "invalid", "no settings");
    }

    #[test]
    fn a_catalog_sized_table_reads_fast() {
        let mut text = String::from("s = {\n");
        let mut n = 0;
        while text.len() < 200_000 {
            text.push_str(&format!("\tSetting{n} = {},\n\tName{n} = \"value {n}\",\n\tCurve{n} = {{ 0, 0, 128, 140, 255, 255 }},\n", n as f64 * 0.5 - 30.0));
            n += 1;
        }
        text.push('}');
        let start = std::time::Instant::now();
        let s = read_lua_table(&text).unwrap();
        let took = start.elapsed();
        assert_eq!(s.len(), n * 3);
        // 5 ms in a release build; debug builds are several times slower.
        let budget = if cfg!(debug_assertions) { 40 } else { 5 };
        assert!(took < std::time::Duration::from_millis(budget), "{took:?}");
    }
}
