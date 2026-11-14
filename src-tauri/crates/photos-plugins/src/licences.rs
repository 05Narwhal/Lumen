//! A plugin's SPDX licence expression: declared and well-formed, nothing more.
//!
//! Lumen is free software and a plugin's code runs in a sandbox behind a WIT interface, so the
//! app doesn't police *which* licence a plugin uses. It shows the expression on the install
//! sheet. A registry that lists plugins decides its own policy (OSI licences only, no ND) in its
//! own checks, not here.

#[derive(Debug, PartialEq)]
enum Expr {
    Id(String),
    And(Box<Expr>, Box<Expr>),
    Or(Box<Expr>, Box<Expr>),
}

fn tokens(expr: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut word = String::new();
    for c in expr.chars() {
        if c == '(' || c == ')' || c.is_whitespace() {
            if !word.is_empty() {
                out.push(std::mem::take(&mut word));
            }
            if !c.is_whitespace() {
                out.push(c.to_string());
            }
        } else {
            word.push(c);
        }
    }
    if !word.is_empty() {
        out.push(word);
    }
    out
}

struct Parser {
    toks: Vec<String>,
    at: usize,
}

impl Parser {
    fn peek(&self) -> Option<&str> {
        self.toks.get(self.at).map(String::as_str)
    }

    fn or(&mut self) -> Result<Expr, String> {
        let mut left = self.and()?;
        while self.peek() == Some("OR") {
            self.at += 1;
            left = Expr::Or(Box::new(left), Box::new(self.and()?));
        }
        Ok(left)
    }

    fn and(&mut self) -> Result<Expr, String> {
        let mut left = self.atom()?;
        while self.peek() == Some("AND") {
            self.at += 1;
            left = Expr::And(Box::new(left), Box::new(self.atom()?));
        }
        Ok(left)
    }

    fn atom(&mut self) -> Result<Expr, String> {
        match self.peek() {
            None => Err("The licence expression ends too early".into()),
            Some("(") => {
                self.at += 1;
                let e = self.or()?;
                if self.peek() != Some(")") {
                    return Err("The licence expression is missing a ')'".into());
                }
                self.at += 1;
                Ok(e)
            }
            Some(t @ (")" | "AND" | "OR" | "WITH")) => {
                Err(format!("Unexpected \"{t}\" in the licence expression"))
            }
            Some(id) => {
                let id = id.to_string();
                self.at += 1;
                if self.peek() == Some("WITH") {
                    self.at += 1;
                    let exception = self
                        .peek()
                        .filter(|t| !matches!(*t, "(" | ")" | "AND" | "OR" | "WITH"))
                        .ok_or_else(|| format!("{id} WITH needs an exception name"))?
                        .to_string();
                    self.at += 1;
                    return Ok(Expr::Id(format!("{id} WITH {exception}")));
                }
                Ok(Expr::Id(id))
            }
        }
    }
}

/// An SPDX-shaped identifier (`[A-Za-z0-9.+-]+`), or a `LicenseRef-<name>` whose text ships in
/// the package's `LICENSE` file.
fn accepted(id: &str, has_licence_file: bool) -> Result<(), String> {
    let (id, exception) = id.split_once(" WITH ").unwrap_or((id, ""));
    let shaped = |s: &str| {
        !s.is_empty()
            && s.chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '+' | '-'))
    };
    if !shaped(id) || (!exception.is_empty() && !shaped(exception)) {
        return Err(format!("{id} isn't a licence identifier"));
    }
    if let Some(name) = id.strip_prefix("LicenseRef-") {
        return if !name.is_empty() && has_licence_file {
            Ok(())
        } else {
            Err(format!("{id} needs its text in a LICENSE file"))
        };
    }
    Ok(())
}

fn eval(e: &Expr, has_file: bool) -> Result<(), String> {
    match e {
        Expr::Id(id) => accepted(id, has_file),
        Expr::And(a, b) => eval(a, has_file).and(eval(b, has_file)),
        Expr::Or(a, b) => {
            eval(a, has_file).or_else(|ea| eval(b, has_file).map_err(|eb| format!("{ea}; {eb}")))
        }
    }
}

/// `Ok` when the SPDX expression is declared and well-formed; the error names the identifier
/// that failed.
pub fn check(expr: &str, has_licence_file: bool) -> Result<(), String> {
    let toks = tokens(expr);
    if toks.is_empty() {
        return Err("No licence is given".into());
    }
    let mut p = Parser { toks, at: 0 };
    let e = p.or()?;
    if let Some(t) = p.peek() {
        return Err(format!("Unexpected \"{t}\" in the licence expression"));
    }
    eval(&e, has_licence_file)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn licence_table() {
        let table = [
            ("MIT", false, true),
            ("GPL-3.0-only", false, true),
            ("AGPL-3.0-or-later", false, true),
            ("CC-BY-NC-4.0", false, true),
            ("Foo-1.0", false, true),
            ("Apache-2.0 OR GPL-3.0-only", false, true),
            ("MIT WITH Classpath-exception-2.0", false, true),
            ("(MIT OR Zlib) AND ISC", false, true),
            ("LicenseRef-Acme", true, true),
            ("LicenseRef-Acme", false, false),
            ("LicenseRef-", true, false),
            ("", false, false),
            ("(MIT", false, false),
            ("MIT OR", false, false),
            ("MIT AND", false, false),
            ("MIT WITH", false, false),
            ("bad id!", false, false),
        ];
        for (expr, has_file, ok) in table {
            let got = check(expr, has_file);
            assert_eq!(got.is_ok(), ok, "{expr:?} {has_file}: {got:?}");
        }
    }

    #[test]
    fn error_names_the_failing_identifier() {
        let e = check("MIT AND LicenseRef-Acme", false).unwrap_err();
        assert!(e.contains("LicenseRef-Acme"), "{e}");
    }
}
