//! Search with `or`, `and`, `not` and brackets (Round 19 §6). "anna or ben -dog" is a tree of
//! terms; each term between the operators is read by [`query_parse::parse`] unchanged, so a leaf
//! is today's reading of its words. A text with no operator gets no tree and takes today's path.
//!
//! Grammar, `not` binding tightest and juxtaposition meaning `and`:
//! ```text
//! expr := and ("or" and)*      and := unary (("and")? unary)*
//! unary := ("not" | "-")? atom atom := "(" expr ")" | segment
//! ```
//! `or`, `and` and `not` are operators only as whole words, in any case, outside quotes and
//! with a term to act on; at the start or end, doubled, or next to a bracket that leaves them
//! nothing, they are plain words.

use chrono::NaiveDate;
use rusqlite::types::Value;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::query::AssetQuery;
use crate::query_parse::{self, Chip, ChipKind, Vocabulary};
use crate::settings::DateOrder;

/// The most operators on one path from the top to a term, and the most brackets inside one another.
pub const MAX_DEPTH: usize = 8;
/// The most terms one search may have.
pub const MAX_LEAVES: usize = 16;

/// A search read as `and` / `or` / `not` of terms. A leaf is one term's reading, its `text`
/// what was left of its words for full-text search and search by meaning.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum QueryExpr {
    And(Vec<QueryExpr>),
    Or(Vec<QueryExpr>),
    Not(Box<QueryExpr>),
    Leaf(Box<AssetQuery>),
}

/// A text read as an expression: the tree, and every chip in reading order, operators and
/// brackets included (`ChipKind::Op`).
#[derive(Debug, Clone, PartialEq)]
pub struct ParsedExpr {
    pub expr: QueryExpr,
    pub chips: Vec<Chip>,
}

impl QueryExpr {
    /// How many terms the tree has.
    pub fn leaves(&self) -> usize {
        match self {
            QueryExpr::And(v) | QueryExpr::Or(v) => v.iter().map(QueryExpr::leaves).sum(),
            QueryExpr::Not(e) => e.leaves(),
            QueryExpr::Leaf(_) => 1,
        }
    }

    /// The most operators on one path from the top to a term.
    pub fn depth(&self) -> usize {
        match self {
            QueryExpr::And(v) | QueryExpr::Or(v) => 1 + v.iter().map(QueryExpr::depth).max().unwrap_or(0),
            QueryExpr::Not(e) => 1 + e.depth(),
            QueryExpr::Leaf(_) => 0,
        }
    }

    /// Every term in reading order, with whether it counts for the photos it finds (`true`) or
    /// against them (under an odd number of `not`s). The app fills in each term's meaning hits.
    pub fn leaves_mut(&mut self, f: &mut impl FnMut(&mut AssetQuery, bool)) {
        fn walk(e: &mut QueryExpr, positive: bool, f: &mut impl FnMut(&mut AssetQuery, bool)) {
            match e {
                QueryExpr::And(v) | QueryExpr::Or(v) => v.iter_mut().for_each(|e| walk(e, positive, f)),
                QueryExpr::Not(e) => walk(e, !positive, f),
                QueryExpr::Leaf(q) => f(q, positive),
            }
        }
        walk(self, true, f)
    }

    /// [`QueryExpr::leaves_mut`] without changing anything.
    pub fn each_leaf(&self, f: &mut impl FnMut(&AssetQuery, bool)) {
        fn walk(e: &QueryExpr, positive: bool, f: &mut impl FnMut(&AssetQuery, bool)) {
            match e {
                QueryExpr::And(v) | QueryExpr::Or(v) => v.iter().for_each(|e| walk(e, positive, f)),
                QueryExpr::Not(e) => walk(e, !positive, f),
                QueryExpr::Leaf(q) => f(q, positive),
            }
        }
        walk(self, true, f)
    }

    /// One WHERE clause for the tree: each term is today's clause for its query (its meaning
    /// hits included), combined with AND / OR / NOT. A `not` treats a term SQL cannot decide
    /// (NULL) as not matching, so it shows that photo.
    pub fn sql(&self, params: &mut Vec<Value>) -> String {
        let join = |v: &[QueryExpr], op: &str, params: &mut Vec<Value>| {
            let parts: Vec<String> = v.iter().map(|e| e.sql(params)).collect();
            if parts.is_empty() {
                // An empty `and` holds for every photo, an empty `or` for none.
                return if op == "AND" { "1".to_string() } else { "0".to_string() };
            }
            format!("({})", parts.join(&format!(" {op} ")))
        };
        match self {
            QueryExpr::And(v) => join(v, "AND", params),
            QueryExpr::Or(v) => join(v, "OR", params),
            QueryExpr::Not(e) => format!("NOT COALESCE({}, 0)", e.sql(params)),
            QueryExpr::Leaf(q) => q.clause(params),
        }
    }
}

/// The *Best match* order of a tree's terms searched by meaning: each photo at its best score
/// among them, best first (ties by id). Only terms that count for photos are passed in.
pub fn best_first(scored: &[&[(i64, f32)]]) -> Vec<i64> {
    let mut best: std::collections::HashMap<i64, f32> = std::collections::HashMap::new();
    for (id, score) in scored.iter().flat_map(|s| s.iter()) {
        let slot = best.entry(*id).or_insert(f32::NEG_INFINITY);
        *slot = slot.max(*score);
    }
    let mut out: Vec<(i64, f32)> = best.into_iter().collect();
    out.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
    out.into_iter().map(|(id, _)| id).collect()
}

/// Reads `text` as an expression, each term with [`query_parse::parse`] (same arguments).
/// `None` when the text has no operator, or more terms or depth than the limits allow: the
/// caller then reads it as plain text, as before.
pub fn parse_expr(text: &str, vocab: &Vocabulary, today: NaiveDate, offset_secs: i32, order: DateOrder, count: impl Fn(&AssetQuery) -> u64) -> Option<ParsedExpr> {
    let (node, toks) = parse_nodes_with(text)?;
    let mut chips: Vec<Chip> = toks
        .iter()
        .filter_map(|t| {
            let label = match t.lex {
                Lex::Word => return None,
                Lex::Or => "or",
                Lex::And => "and",
                Lex::Not | Lex::Minus => "not",
                Lex::Open => "(",
                Lex::Close => ")",
            };
            Some(Chip { kind: ChipKind::Op, label: label.into(), span: (t.start, t.end), token: text[t.start..t.end].to_string(), path: Vec::new(), alternatives: Vec::new() })
        })
        .collect();
    let expr = node.read(&mut |start, end| {
        let parsed = query_parse::parse(&text[start..end], vocab, today, offset_secs, order, &count);
        let shift = |c: &mut Chip| c.span = (c.span.0 + start, c.span.1 + start);
        for mut chip in parsed.chips {
            shift(&mut chip);
            chip.alternatives.iter_mut().for_each(shift);
            chips.push(chip);
        }
        parsed.query
    });
    chips.sort_by_key(|c| c.span.0);
    Some(ParsedExpr { expr, chips })
}

/// The tree of `text` before its terms are read: `None` when the text has no operator, or more
/// terms or depth than the limits allow.
pub(crate) fn parse_nodes(text: &str) -> Option<Node> {
    parse_nodes_with(text).map(|(node, _)| node)
}

fn parse_nodes_with(text: &str) -> Option<(Node, Vec<Tok>)> {
    let toks = operators(text);
    if !toks.iter().any(|t| matches!(t.lex, Lex::Or | Lex::And | Lex::Not | Lex::Minus)) {
        return None;
    }
    let mut p = Descent { toks: &toks, i: 0 };
    let node = p.expr(0)?;
    if p.i != toks.len() || node.leaves() > MAX_LEAVES || node.depth() > MAX_DEPTH {
        return None;
    }
    Some((node, toks))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Lex {
    Word,
    Open,
    Close,
    Or,
    And,
    Not,
    /// `-` written against the next word: `not`.
    Minus,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct Tok {
    pub(crate) lex: Lex,
    pub(crate) start: usize,
    pub(crate) end: usize,
    /// Has a letter or digit: something a term can be. A stray `(` or `""` is not.
    pub(crate) solid: bool,
}

fn is_quote(c: char) -> bool {
    matches!(c, '"' | '“' | '”' | '„')
}

/// `text` cut into words, brackets and operators, every operator that has nothing to act on
/// turned back into a word.
pub(crate) fn operators(text: &str) -> Vec<Tok> {
    let mut toks = Vec::new();
    // Words are what whitespace outside quotes separates, so `"rock and roll"` and
    // `place:"New York"` are one word each.
    let mut quoted = false;
    let mut start: Option<usize> = None;
    for (i, c) in text.char_indices() {
        if is_quote(c) {
            quoted = !quoted;
        }
        if c.is_whitespace() && !quoted {
            if let Some(s) = start.take() {
                chunk(text, s, i, &mut toks);
            }
        } else if start.is_none() {
            start = Some(i);
        }
    }
    if let Some(s) = start {
        chunk(text, s, text.len(), &mut toks);
    }

    // Brackets that pair up around something; any other bracket is text.
    let mut open: Vec<usize> = Vec::new();
    let mut paired = vec![false; toks.len()];
    for i in 0..toks.len() {
        match toks[i].lex {
            Lex::Open => open.push(i),
            Lex::Close => {
                if let Some(j) = open.pop() {
                    if j + 1 < i {
                        paired[i] = true;
                        paired[j] = true;
                    }
                }
            }
            _ => {}
        }
    }
    for (t, paired) in toks.iter_mut().zip(paired) {
        if matches!(t.lex, Lex::Open | Lex::Close) && !paired {
            t.lex = Lex::Word;
        }
    }

    // A doubled operator ("or or") is text, every copy of it.
    let doubled: Vec<usize> = (1..toks.len())
        .filter(|&i| matches!(toks[i].lex, Lex::Or | Lex::And | Lex::Not) && toks[i].lex == toks[i - 1].lex)
        .flat_map(|i| [i - 1, i])
        .collect();
    for i in doubled {
        toks[i].lex = Lex::Word;
    }

    // Only ever turning operators into words, so this settles.
    let term = |t: &Tok| t.lex == Lex::Word && t.solid;
    loop {
        let mut changed = false;
        for i in 0..toks.len() {
            let next = toks.get(i + 1);
            let ok = match toks[i].lex {
                Lex::Or | Lex::And => {
                    let before = i > 0 && (term(&toks[i - 1]) || toks[i - 1].lex == Lex::Close);
                    let after = next.is_some_and(|t| term(t) || matches!(t.lex, Lex::Open | Lex::Not | Lex::Minus));
                    before && after
                }
                Lex::Not | Lex::Minus => next.is_some_and(|t| term(t) || t.lex == Lex::Open),
                _ => true,
            };
            if !ok {
                toks[i].lex = Lex::Word;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    toks
}

/// One whitespace-separated word of `text[s..e]`: its leading `(` and `-`, the word, its
/// trailing `)`.
fn chunk(text: &str, mut s: usize, mut e: usize, toks: &mut Vec<Tok>) {
    let tok = |lex, start, end| Tok { lex, start, end, solid: text[start..end].chars().any(char::is_alphanumeric) };
    while s < e {
        let mut cs = text[s..e].chars();
        match cs.next() {
            Some('(') => toks.push(tok(Lex::Open, s, s + 1)),
            Some('-') if cs.next().is_some_and(|c| c.is_alphanumeric() || c == '(' || is_quote(c)) => toks.push(tok(Lex::Minus, s, s + 1)),
            _ => break,
        }
        s += 1;
    }
    let mut closes = 0;
    while e > s && text[s..e].ends_with(')') {
        e -= 1;
        closes += 1;
    }
    if s < e {
        let lex = match text[s..e].to_lowercase().as_str() {
            "or" => Lex::Or,
            "and" => Lex::And,
            "not" => Lex::Not,
            _ => Lex::Word,
        };
        toks.push(tok(lex, s, e));
    }
    for k in 0..closes {
        toks.push(tok(Lex::Close, e + k, e + k + 1));
    }
}

/// The tree before its terms are read: a term is a byte range of the text.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Node {
    And(Vec<Node>),
    Or(Vec<Node>),
    Not(Box<Node>),
    Term(usize, usize),
}

impl Node {
    fn leaves(&self) -> usize {
        match self {
            Node::And(v) | Node::Or(v) => v.iter().map(Node::leaves).sum(),
            Node::Not(n) => n.leaves(),
            Node::Term(..) => 1,
        }
    }

    fn depth(&self) -> usize {
        match self {
            Node::And(v) | Node::Or(v) => 1 + v.iter().map(Node::depth).max().unwrap_or(0),
            Node::Not(n) => 1 + n.depth(),
            Node::Term(..) => 0,
        }
    }

    /// The last term's byte range, if any.
    pub(crate) fn last_term(&self) -> Option<(usize, usize)> {
        match self {
            Node::And(v) | Node::Or(v) => v.iter().rev().find_map(Node::last_term),
            Node::Not(n) => n.last_term(),
            Node::Term(s, e) => Some((*s, *e)),
        }
    }

    /// The tree with each term read by `read`, left to right.
    fn read(self, read: &mut impl FnMut(usize, usize) -> AssetQuery) -> QueryExpr {
        match self {
            Node::And(v) => QueryExpr::And(v.into_iter().map(|n| n.read(read)).collect()),
            Node::Or(v) => QueryExpr::Or(v.into_iter().map(|n| n.read(read)).collect()),
            Node::Not(n) => QueryExpr::Not(Box::new(n.read(read))),
            Node::Term(s, e) => QueryExpr::Leaf(Box::new(read(s, e))),
        }
    }

    /// `parts` joined by `or` (else `and`), a part that is the same operator spliced in.
    fn join(mut parts: Vec<Node>, or: bool) -> Node {
        if parts.len() == 1 {
            return parts.pop().expect("one part");
        }
        let mut out = Vec::with_capacity(parts.len());
        for part in parts {
            match (part, or) {
                (Node::Or(v), true) | (Node::And(v), false) => out.extend(v),
                (part, _) => out.push(part),
            }
        }
        if or { Node::Or(out) } else { Node::And(out) }
    }
}

/// Recursive descent over the grammar in the module's docs; `None` on anything it does not
/// expect, and on brackets nested deeper than [`MAX_DEPTH`].
pub(crate) struct Descent<'a> {
    pub(crate) toks: &'a [Tok],
    pub(crate) i: usize,
}

impl Descent<'_> {
    fn peek(&self) -> Option<Lex> {
        self.toks.get(self.i).map(|t| t.lex)
    }

    pub(crate) fn expr(&mut self, nest: usize) -> Option<Node> {
        let mut parts = vec![self.and(nest)?];
        while self.peek() == Some(Lex::Or) {
            self.i += 1;
            parts.push(self.and(nest)?);
        }
        Some(Node::join(parts, true))
    }

    fn and(&mut self, nest: usize) -> Option<Node> {
        let mut parts = vec![self.unary(nest)?];
        loop {
            match self.peek() {
                Some(Lex::And) => {
                    self.i += 1;
                    parts.push(self.unary(nest)?);
                }
                Some(Lex::Word | Lex::Open | Lex::Not | Lex::Minus) => parts.push(self.unary(nest)?),
                _ => break,
            }
        }
        Some(Node::join(parts, false))
    }

    fn unary(&mut self, nest: usize) -> Option<Node> {
        if matches!(self.peek(), Some(Lex::Not | Lex::Minus)) {
            self.i += 1;
            return Some(Node::Not(Box::new(self.atom(nest)?)));
        }
        self.atom(nest)
    }

    fn atom(&mut self, nest: usize) -> Option<Node> {
        match self.peek()? {
            Lex::Open if nest < MAX_DEPTH => {
                self.i += 1;
                let inner = self.expr(nest + 1)?;
                if self.peek() != Some(Lex::Close) {
                    return None;
                }
                self.i += 1;
                Some(inner)
            }
            Lex::Word => {
                let start = self.toks[self.i].start;
                let mut end = start;
                while self.peek() == Some(Lex::Word) {
                    end = self.toks[self.i].end;
                    self.i += 1;
                }
                Some(Node::Term(start, end))
            }
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vocab() -> Vocabulary {
        let mut v = Vocabulary::default();
        v.people = vec![(1, "Anna".into()), (2, "Ben".into()), (3, "Cleo".into())];
        v.tags = crate::embedding::SCENE_TAGS.iter().map(|t| t.to_string()).collect();
        v
    }

    fn run(text: &str) -> Option<ParsedExpr> {
        parse_expr(text, &vocab(), NaiveDate::from_ymd_opt(2026, 11, 11).unwrap(), 0, DateOrder::Dmy, |_| 1)
    }

    /// A tree as text: `or(@1, and(#beach, not(rock and roll)))`; a leaf lists its people,
    /// tags and leftover text.
    fn show(e: &QueryExpr) -> String {
        let list = |v: &[QueryExpr]| v.iter().map(show).collect::<Vec<_>>().join(", ");
        match e {
            QueryExpr::And(v) => format!("and({})", list(v)),
            QueryExpr::Or(v) => format!("or({})", list(v)),
            QueryExpr::Not(e) => format!("not({})", show(e)),
            QueryExpr::Leaf(q) => {
                let mut parts: Vec<String> = q.person_ids.iter().map(|p| format!("@{p}")).collect();
                parts.extend(q.tags.iter().map(|t| format!("#{t}")));
                parts.extend(q.text.clone());
                parts.join(" ")
            }
        }
    }

    fn tree(text: &str) -> String {
        show(&run(text).unwrap_or_else(|| panic!("no tree for {text:?}")).expr)
    }

    #[test]
    fn precedence_not_then_and_then_or() {
        assert_eq!(tree("anna or ben"), "or(@1, @2)");
        assert_eq!(tree("anna or ben beach"), "or(@1, @2 #beach)");
        assert_eq!(tree("anna or ben and beach"), "or(@1, and(@2, #beach))");
        assert_eq!(tree("anna and ben or cleo"), "or(and(@1, @2), @3)");
        assert_eq!(tree("anna or ben or cleo"), "or(@1, @2, @3)");
        assert_eq!(tree("beach not dog"), "and(#beach, not(#dog))");
        assert_eq!(tree("beach and not dog"), "and(#beach, not(#dog))");
        assert_eq!(tree("not dog or cat"), "or(not(#dog), #cat)");
    }

    #[test]
    fn brackets_group_and_juxtaposition_is_and() {
        assert_eq!(tree("(anna or ben) and beach"), "and(or(@1, @2), #beach)");
        assert_eq!(tree("(anna or ben) beach"), "and(or(@1, @2), #beach)");
        assert_eq!(tree("beach (anna or ben)"), "and(#beach, or(@1, @2))");
        assert_eq!(tree("not (anna or ben)"), "not(or(@1, @2))");
        assert_eq!(tree("((anna or ben)) cat"), "and(or(@1, @2), #cat)");
    }

    #[test]
    fn minus_word_is_not() {
        assert_eq!(tree("beach -dog"), "and(#beach, not(#dog))");
        assert_eq!(tree("-dog"), "not(#dog)");
        assert_eq!(tree("-(anna or ben) beach"), "and(not(or(@1, @2)), #beach)");
        // A dash inside a word, or standing alone, is no operator.
        assert!(run("2023-05").is_none());
        assert!(run("2020 - 2023").is_none());
        assert!(run("anna-or-ben").is_none());
    }

    #[test]
    fn quoted_phrases_stay_one_term() {
        assert!(run("\"black and white\"").is_none());
        assert!(run("\"anna or ben\"").is_none());
        assert_eq!(tree("beach or \"rock and roll\""), "or(#beach, rock and roll)");
        let p = run("\"black and white\" or beach").unwrap();
        assert_eq!(p.expr.leaves(), 2);
        // A forced reading is one term too.
        assert_eq!(tree("tag:beach or anna"), "or(#beach, @1)");
    }

    #[test]
    fn operators_with_nothing_to_join_are_plain_words() {
        for t in ["or beach", "beach or", "and", "beach and", "beach not", "not", "beach or or dog", "beach and and dog", "not not dog", "(or beach)", "(beach or) dog"] {
            assert!(run(t).is_none(), "{t:?} should read as plain text");
        }
        // Whole words only.
        assert!(run("orange notebook android").is_none());
        // Any case.
        assert_eq!(tree("Anna OR Ben"), "or(@1, @2)");
        assert_eq!(tree("beach Not dog"), "and(#beach, not(#dog))");
    }

    #[test]
    fn text_without_an_operator_has_no_tree() {
        for t in ["anna beach 2023", "(beach)", "", "   ", "sunset over the sea"] {
            assert!(run(t).is_none(), "{t:?}");
        }
    }

    #[test]
    fn unmatched_brackets_are_plain_text() {
        assert_eq!(tree("(anna or ben"), "or(@1, @2)");
        assert_eq!(tree("anna or ben)"), "or(@1, @2)");
        assert!(run("photo(1) beach").is_none());
    }

    #[test]
    fn limits_degrade_to_plain_text() {
        let terms = |n: usize| (1..=n).map(|i| format!("w{i}")).collect::<Vec<_>>().join(" or ");
        assert_eq!(run(&terms(MAX_LEAVES)).unwrap().expr.leaves(), MAX_LEAVES);
        assert!(run(&terms(MAX_LEAVES + 1)).is_none());

        // `w0 or (w1 and (w2 or (…)))`: one operator per level.
        let nested = |levels: usize| {
            let mut s = format!("w{levels}");
            for i in (0..levels).rev() {
                let op = if i % 2 == 0 { "or" } else { "and" };
                s = if i == 0 { format!("w0 {op} {s}") } else { format!("w{i} {op} ({s})") };
            }
            s
        };
        let deep = run(&nested(MAX_DEPTH)).unwrap();
        assert_eq!(deep.expr.depth(), MAX_DEPTH);
        assert!(run(&nested(MAX_DEPTH + 1)).is_none());

        let brackets = |n: usize| format!("{}beach{} or dog", "(".repeat(n), ")".repeat(n));
        assert!(run(&brackets(MAX_DEPTH)).is_some());
        assert!(run(&brackets(MAX_DEPTH + 1)).is_none());
    }

    #[test]
    fn chips_in_reading_order_with_operators() {
        let p = run("(Anna or Ben) -dog").unwrap();
        let read: Vec<(ChipKind, &str, (usize, usize))> = p.chips.iter().map(|c| (c.kind, c.label.as_str(), c.span)).collect();
        assert_eq!(
            read,
            vec![
                (ChipKind::Op, "(", (0, 1)),
                (ChipKind::Person, "Anna", (1, 5)),
                (ChipKind::Op, "or", (6, 8)),
                (ChipKind::Person, "Ben", (9, 12)),
                (ChipKind::Op, ")", (12, 13)),
                (ChipKind::Op, "not", (14, 15)),
                (ChipKind::Tag, "Dog", (15, 18)),
            ]
        );
        // A term's chip still rewrites its own words.
        let dog = &p.chips[6];
        assert_eq!(&"(Anna or Ben) -dog"[dog.span.0..dog.span.1], "dog");
        assert_eq!(p.chips[5].token, "-");
        assert_eq!(p.chips[2].token, "or");
    }

    #[test]
    fn query_with_a_tree_round_trips_and_old_queries_read_the_same() {
        let p = run("anna or beach").unwrap();
        let q = AssetQuery { expr: Some(Box::new(p.expr.clone())), ..Default::default() };
        let json = serde_json::to_string(&q).unwrap();
        assert_eq!(serde_json::from_str::<AssetQuery>(&json).unwrap(), q);
        // A query without a tree is written exactly as before: no `expr` key.
        let plain = serde_json::to_string(&AssetQuery::default()).unwrap();
        assert!(!plain.contains("expr"), "{plain}");
        assert_eq!(serde_json::from_str::<AssetQuery>("{}").unwrap().expr, None);
    }

    // ---- Evaluation (B3) ----

    use crate::develop_strip::{strip_ids, strip_page, StripSort};
    use crate::scope::LayoutScope;
    use rusqlite::Connection;

    /// Tags per photo: 1 beach, 2 dog, 3 beach dog, 4 beach sunset, 5 dog sunset, 6 sunset;
    /// ratings 1–6 by id, captured a day apart.
    fn tagged(c: &Connection) {
        for (id, tags) in [(1, "beach"), (2, "dog"), (3, "beach dog"), (4, "beach sunset"), (5, "dog sunset"), (6, "sunset")] {
            c.execute(
                "INSERT INTO assets (id, content_hash, relative_path, filename, extension, media_type, byte_size, orientation, captured_at, imported_at, scene_tags, rating)
                 VALUES (?1, ?2, 'p', 'x.jpg', 'jpg', 0, 1, 1, ?3, ?3, ?4, ?1)",
                rusqlite::params![id, format!("h{id}"), 1_700_000_000 + id * 86_400, tags],
            )
            .unwrap();
        }
    }

    fn scope(expr: QueryExpr) -> LayoutScope {
        LayoutScope { query: Some(AssetQuery { expr: Some(Box::new(expr)), ..Default::default() }), ..Default::default() }
    }

    fn ids_of(c: &Connection, expr: QueryExpr) -> Vec<i64> {
        let mut ids = crate::layout::ids_scoped(c, &scope(expr), 100).unwrap();
        ids.sort_unstable();
        ids
    }

    fn typed(text: &str) -> QueryExpr {
        run(text).unwrap_or_else(|| panic!("no tree for {text:?}")).expr
    }

    fn leaf(q: AssetQuery) -> QueryExpr {
        QueryExpr::Leaf(Box::new(q))
    }

    fn with_db(f: impl FnOnce(&Connection)) {
        let db = crate::db::Db::open_in_memory().unwrap();
        db.with_conn(|c| {
            tagged(c);
            f(c);
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn or_and_not_and_brackets_select_the_right_photos() {
        with_db(|c| {
            assert_eq!(ids_of(c, typed("beach or dog")), vec![1, 2, 3, 4, 5]);
            assert_eq!(ids_of(c, typed("beach not dog")), vec![1, 4]);
            assert_eq!(ids_of(c, typed("beach -dog")), vec![1, 4]);
            assert_eq!(ids_of(c, typed("(beach or dog) and sunset")), vec![4, 5]);
            assert_eq!(ids_of(c, typed("beach or dog sunset")), vec![1, 3, 4, 5], "and binds tighter than or");
            let count = crate::layout::count_scoped(c, &scope(typed("beach or dog"))).unwrap();
            assert_eq!(count, 5, "the count sees the same tree");
        });
    }

    #[test]
    fn a_negated_structured_leaf_excludes_its_photos() {
        with_db(|c| {
            let rated = leaf(AssetQuery { min_rating: Some(4), ..Default::default() });
            assert_eq!(ids_of(c, QueryExpr::Not(Box::new(rated.clone()))), vec![1, 2, 3]);
            let sunset = leaf(AssetQuery { tags: vec!["sunset".into()], ..Default::default() });
            assert_eq!(ids_of(c, QueryExpr::And(vec![sunset, QueryExpr::Not(Box::new(rated))])), Vec::<i64>::new());
            // A photo with no camera in its metadata is "not a Nikon".
            let camera = leaf(AssetQuery { camera: Some("nikon".into()), ..Default::default() });
            assert_eq!(ids_of(c, QueryExpr::Not(Box::new(camera))).len(), 6);
        });
    }

    #[test]
    fn a_semantic_leaf_matches_its_hits_and_a_negated_one_excludes_them() {
        with_db(|c| {
            let meaning = |hits: Vec<i64>| leaf(AssetQuery { text: Some("seaside".into()), semantic: Some(hits), ..Default::default() });
            let beach = leaf(AssetQuery { tags: vec!["beach".into()], ..Default::default() });
            assert_eq!(ids_of(c, QueryExpr::Or(vec![meaning(vec![6]), beach.clone()])), vec![1, 3, 4, 6]);
            assert_eq!(ids_of(c, QueryExpr::And(vec![beach, QueryExpr::Not(Box::new(meaning(vec![3, 4])))])), vec![1]);
        });
    }

    #[test]
    fn keyset_pages_of_a_tree_neither_repeat_nor_skip() {
        with_db(|c| {
            let s = scope(typed("beach or sunset"));
            let mut paged = Vec::new();
            let mut cursor = None;
            loop {
                let page = strip_page(c, &s, None, StripSort::Captured, cursor.as_ref(), 1).unwrap();
                paged.extend(page.items.iter().map(|i| i.id));
                match page.next_cursor {
                    Some(n) => cursor = Some(n),
                    None => break,
                }
            }
            assert_eq!(paged, strip_ids(c, &s, None, StripSort::Captured).unwrap());
            assert_eq!(paged, vec![1, 3, 4, 5, 6]);
        });
    }

    #[test]
    fn a_query_without_a_tree_builds_the_same_sql_as_before() {
        let words = AssetQuery { text: Some("beach".into()), min_rating: Some(2), ..Default::default() };
        let (joins, wh, params) = words.sql_parts();
        assert_eq!(joins, " JOIN assets_fts f ON f.rowid = a.id");
        assert_eq!(wh, vec!["assets_fts MATCH ?".to_string(), "a.rating >= ?".to_string()]);
        assert_eq!(params.len(), 2);
        // Inside a tree the same words become a subquery, so `or` and `not` can hold them.
        let (joins, wh, _) = AssetQuery { expr: Some(Box::new(QueryExpr::Not(Box::new(leaf(words))))), ..Default::default() }.sql_parts();
        assert!(joins.is_empty(), "{joins}");
        assert_eq!(wh.len(), 1);
        assert!(wh[0].starts_with("NOT COALESCE((a.id IN (SELECT rowid FROM assets_fts WHERE assets_fts MATCH ?) AND a.rating >= ?)"), "{}", wh[0]);
    }

    #[test]
    fn leaves_know_whether_they_count_for_or_against() {
        let mut e = typed("beach or not (dog -sunset)");
        let mut seen = Vec::new();
        e.leaves_mut(&mut |q, positive| seen.push((q.tags.clone(), positive)));
        let tags = |t: &str| vec![t.to_string()];
        assert_eq!(seen, vec![(tags("beach"), true), (tags("dog"), false), (tags("sunset"), true)]);
    }

    #[test]
    fn best_first_orders_by_each_photos_best_score() {
        let a = [(1, 0.30), (2, 0.25)];
        let b = [(2, 0.40), (3, 0.20)];
        assert_eq!(best_first(&[&a[..], &b[..]]), vec![2, 1, 3]);
        assert!(best_first(&[]).is_empty());
    }

    /// `cargo test -p photos-core query_expr::tests::timing -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn timing_three_leaves_on_five_thousand_photos() {
        let db = crate::db::Db::open_in_memory().unwrap();
        db.with_conn(|c| {
            let tags = ["beach", "dog", "sunset", "beach dog", "city", "forest sunset"];
            let tx = c.unchecked_transaction()?;
            for id in 1..=5000i64 {
                tx.execute(
                    "INSERT INTO assets (id, content_hash, relative_path, filename, extension, media_type, byte_size, orientation, captured_at, imported_at, scene_tags, rating)
                     VALUES (?1, ?2, 'p', 'x.jpg', 'jpg', 0, 1, 1, ?3, ?3, ?4, ?5)",
                    rusqlite::params![id, format!("h{id}"), 1_600_000_000 + id * 3_600, tags[id as usize % tags.len()], id % 6],
                )?;
            }
            tx.commit()?;
            let s = scope(typed("(beach or dog) -sunset"));
            let started = std::time::Instant::now();
            let n = crate::layout::count_scoped(c, &s)?;
            let count_ms = started.elapsed().as_secs_f64() * 1000.0;
            let started = std::time::Instant::now();
            let index = crate::layout::LayoutIndex::load_scoped(c, &s, true)?;
            let load_ms = started.elapsed().as_secs_f64() * 1000.0;
            println!("three leaves on 5000 photos: {n} found, count {count_ms:.1} ms, layout index {load_ms:.1} ms ({} items)", index.items.len());
            Ok(())
        })
        .unwrap();
    }
}
