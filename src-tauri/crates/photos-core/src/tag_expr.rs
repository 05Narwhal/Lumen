//! Tag expressions (Round 20 §1): the chips of a grid page's search field joined by `and` / `or`,
//! negated by `not` and bracketed in groups. One [`TagSeq`] is what the token row shows and
//! what the query runs.
//!
//! A sequence that says exactly what Round 7's flat tag list said ([`TagSeq::as_plain`]) runs
//! through that list unchanged, so RAW pairing, month merging, *Best match* and speed stay as
//! they are; any other sequence becomes a [`QueryExpr`] tree, as a typed `or` / `and` / `not`
//! search does (Round 19 §6). `tests::golden` writes `src/lib/search-expr-golden.json`, which
//! holds the TypeScript copy of [`TagSeq::repaired`], [`TagSeq::as_plain`] and [`or_key`] to
//! these.

use std::fmt;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::query::DateField;
use crate::query_expr::QueryExpr;
use crate::tag_search::{SearchTag, TagValue};

/// The most tags one sequence keeps.
pub const MAX_TAGS: usize = 24;
/// The most groups inside one another.
pub const MAX_GROUP_DEPTH: usize = 4;

/// What sits between two items of a sequence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum Join {
    And,
    Or,
}

/// One item of a sequence: a tag, or a bracketed sub-sequence.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "camelCase")]
#[ts(export)]
pub enum TagItem {
    Tag {
        #[serde(default, skip_serializing_if = "is_false")]
        #[ts(optional, as = "Option<bool>")]
        not: bool,
        tag: SearchTag,
    },
    Group {
        #[serde(default, skip_serializing_if = "is_false")]
        #[ts(optional, as = "Option<bool>")]
        not: bool,
        seq: TagSeq,
    },
}

/// Items and the joins between them: `joins[i]` sits between `items[i]` and `items[i + 1]`.
/// Read with `not` binding tightest, then `and`, then `or`: `A or B and C` is `A or (B and C)`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct TagSeq {
    pub items: Vec<TagItem>,
    #[serde(default)]
    pub joins: Vec<Join>,
}

/// How Round 7's flat list treats two tags of one variant (`tag_search::to_query`).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum OrKey {
    /// Values of one field: either may hold.
    Field(String),
    /// One value at most: the last one wins.
    Single(String),
}

impl fmt::Display for OrKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            OrKey::Field(k) => write!(f, "field:{k}"),
            OrKey::Single(k) => write!(f, "single:{k}"),
        }
    }
}

fn is_false(b: &bool) -> bool {
    !*b
}

impl TagItem {
    fn not_mut(&mut self) -> &mut bool {
        match self {
            TagItem::Tag { not, .. } | TagItem::Group { not, .. } => not,
        }
    }
}

impl TagSeq {
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// The one normal form (spec §1): joins fitted to the items, empty groups dropped, groups of
    /// one unwrapped, groups whose brackets change nothing spliced into their parent, and the
    /// limits applied. Applied until nothing changes, so it is idempotent.
    pub fn repaired(&self) -> TagSeq {
        let mut s = self.clone();
        loop {
            let mut budget = MAX_TAGS;
            let next = pass(&s, 0, &mut budget);
            if next == s {
                return s;
            }
            s = next;
        }
    }

    /// The tree the sequence asks for, each tag read by `leaf`; `None` when it is empty.
    pub fn to_expr(&self, leaf: &mut impl FnMut(&SearchTag) -> QueryExpr) -> Option<QueryExpr> {
        // Runs of `and`, split at every `or`.
        let mut runs: Vec<Vec<QueryExpr>> = vec![Vec::new()];
        for (i, item) in self.items.iter().enumerate() {
            if i > 0 && self.joins.get(i - 1) == Some(&Join::Or) {
                runs.push(Vec::new());
            }
            let (not, e) = match item {
                TagItem::Tag { not, tag } => (*not, Some(leaf(tag))),
                TagItem::Group { not, seq } => (*not, seq.to_expr(leaf)),
            };
            if let Some(e) = e {
                runs.last_mut().expect("one run at least").push(if not { QueryExpr::Not(Box::new(e)) } else { e });
            }
        }
        let mut ors: Vec<QueryExpr> = runs
            .into_iter()
            .filter(|r| !r.is_empty())
            .map(|mut r| if r.len() == 1 { r.pop().expect("one") } else { QueryExpr::And(r) })
            .collect();
        match ors.len() {
            0 => None,
            1 => ors.pop(),
            _ => Some(QueryExpr::Or(ors)),
        }
    }

    /// The flat tag list that says exactly what this sequence says, when there is one: no `not`,
    /// only `and` at the top, each `or` group made of values of one field, and no field in two
    /// top-level items (`Live and HDR` is not plain: the flat list would read it as `or`).
    pub fn as_plain(&self) -> Option<Vec<SearchTag>> {
        if self.joins.iter().any(|j| *j != Join::And) {
            return None;
        }
        let mut tags = Vec::new();
        let mut keys: Vec<OrKey> = Vec::new();
        for item in &self.items {
            let key = match item {
                TagItem::Tag { not: false, tag } => {
                    tags.push(tag.clone());
                    or_key(&tag.value)
                }
                TagItem::Group { not: false, seq } => {
                    if seq.joins.len() + 1 != seq.items.len() || seq.joins.iter().any(|j| *j != Join::Or) {
                        return None;
                    }
                    let mut key = None;
                    for inner in &seq.items {
                        let TagItem::Tag { not: false, tag } = inner else { return None };
                        let k @ Some(OrKey::Field(_)) = or_key(&tag.value) else { return None };
                        if key.is_some() && key != k {
                            return None;
                        }
                        key = k;
                        tags.push(tag.clone());
                    }
                    key
                }
                _ => return None,
            };
            if let Some(k) = key {
                if keys.contains(&k) {
                    return None;
                }
                keys.push(k);
            }
        }
        Some(tags)
    }

    /// Every tag in reading order, `true` when it counts for the photos it finds (under an even
    /// number of `not`s).
    pub fn leaves(&self) -> Vec<(&SearchTag, bool)> {
        fn walk<'a>(s: &'a TagSeq, positive: bool, out: &mut Vec<(&'a SearchTag, bool)>) {
            for item in &s.items {
                match item {
                    TagItem::Tag { not, tag } => out.push((tag, positive != *not)),
                    TagItem::Group { not, seq } => walk(seq, positive != *not, out),
                }
            }
        }
        let mut out = Vec::new();
        walk(self, true, &mut out);
        out
    }
}

/// One pass of [`TagSeq::repaired`] over a sequence held by `depth` groups; `budget` is how many
/// tags may still be kept, in reading order.
fn pass(seq: &TagSeq, depth: usize, budget: &mut usize) -> TagSeq {
    // Rules 1 and 5 (the count): each item with the join before it (the first one's unused),
    // a missing join read as `and`, items past the last tag allowed dropped.
    let mut items: Vec<(Join, TagItem)> = Vec::new();
    for (i, item) in seq.items.iter().enumerate() {
        if *budget == 0 {
            break;
        }
        let join = if i == 0 { Join::And } else { seq.joins.get(i - 1).copied().unwrap_or(Join::And) };
        let item = match item {
            TagItem::Tag { .. } => {
                *budget -= 1;
                item.clone()
            }
            TagItem::Group { not, seq } => TagItem::Group { not: *not, seq: pass(seq, depth + 1, budget) },
        };
        items.push((join, item));
    }
    // Rule 2: an empty group goes with the join before it; the first one takes the join after it
    // (the next item becomes first).
    items.retain(|(_, item)| !matches!(item, TagItem::Group { seq, .. } if seq.items.is_empty()));
    // Rule 3: a group of one is that item, negated by either but not both.
    for (_, item) in &mut items {
        if let TagItem::Group { not, seq } = item {
            if seq.items.len() == 1 {
                let flip = *not;
                let mut inner = seq.items[0].clone();
                *inner.not_mut() ^= flip;
                *item = inner;
            }
        }
    }
    // Rules 4 and 5 (the depth): a group is spliced into its parent when its brackets change
    // nothing, and always when it is deeper than the limit (its `not` then goes).
    fn push(out: &mut TagSeq, join: Join, item: TagItem) {
        if !out.items.is_empty() {
            out.joins.push(join);
        }
        out.items.push(item);
    }
    let mut out = TagSeq::default();
    for k in 0..items.len() {
        let (join, item) = items[k].clone();
        let before = (k > 0).then_some(join);
        let after = items.get(k + 1).map(|(j, _)| *j);
        match item {
            TagItem::Group { not, seq }
                if depth + 1 > MAX_GROUP_DEPTH
                    || (!not
                        && (seq.joins.iter().all(|j| *j == Join::And)
                            || (before != Some(Join::And) && after != Some(Join::And)))) =>
            {
                for (m, inner) in seq.items.into_iter().enumerate() {
                    let j = if m == 0 { join } else { seq.joins.get(m - 1).copied().unwrap_or(Join::And) };
                    push(&mut out, j, inner);
                }
            }
            item => push(&mut out, join, item),
        }
    }
    out
}

/// Core's reading of one tag: exactly what the flat list does with it alone.
pub fn core_leaf(tag: &SearchTag) -> QueryExpr {
    QueryExpr::Leaf(Box::new(crate::tag_search::to_query(std::slice::from_ref(tag))))
}

/// How `tag_search::to_query` treats two tags of `value`'s variant (spec §1's table, the
/// contract): `None` both must hold, `Field` either may, `Single` the last one wins.
pub fn or_key(value: &TagValue) -> Option<OrKey> {
    use TagValue as V;
    let field = |k: &str| Some(OrKey::Field(k.to_string()));
    match value {
        V::Person { .. }
        | V::Pet { .. }
        | V::Album { .. }
        | V::Collection { .. }
        | V::Scene { .. }
        | V::Favorites
        | V::HasText
        | V::NoText
        | V::PhotoText { .. }
        | V::Text { .. }
        | V::Lumen { .. }
        | V::EditedIn { .. } => None,
        V::Media { .. } => field("media"),
        V::Kind { .. } => field("kind"),
        V::Place { .. } => field("place"),
        V::Extension { .. } => field("extension"),
        V::Shape { .. } => field("shape"),
        V::Rating { .. } => field("rating"),
        V::Months { .. } => field("months"),
        V::Dates { field: None | Some(DateField::Taken), .. } => field("dates"),
        V::Dates { field: Some(f), .. } => field(&format!("dates:{}", wire(f))),
        V::Exif { field: f, .. } => field(&format!("exif:{}", wire(f))),
        V::Range { measure, .. } => field(&format!("range:{}", wire(measure))),
        V::Flag { .. } => Some(OrKey::Single("flag".into())),
        V::Location { .. } => Some(OrKey::Single("location".into())),
    }
}

/// A unit enum's name on the wire (`fileModified`).
fn wire(v: &impl Serialize) -> String {
    serde_json::to_value(v).ok().and_then(|v| v.as_str().map(str::to_string)).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use rusqlite::Connection;

    use super::*;
    use crate::db::Db;
    use crate::query::{AssetKind, AssetQuery, ExifField, LumenMark, Measure, PlaceRef, Shape};

    fn st(label: &str, value: TagValue) -> SearchTag {
        SearchTag { label: label.into(), value }
    }

    fn city(name: &str, country: &str) -> TagValue {
        TagValue::Place { place: PlaceRef::City { name: name.into(), country: Some(country.into()) } }
    }

    /// The tag a test word names; any other word is a text tag.
    fn tag(label: &str) -> SearchTag {
        let value = match label {
            "Live" => TagValue::Kind { kind: AssetKind::Live },
            "HDR" => TagValue::Kind { kind: AssetKind::Hdr },
            "RAW" => TagValue::Kind { kind: AssetKind::Raw },
            "Paris" => city("Paris", "France"),
            "Lisbon" => city("Lisbon", "Portugal"),
            "Porto" => city("Porto", "Portugal"),
            "Anna" => TagValue::Person { id: 1 },
            "Ben" => TagValue::Person { id: 2 },
            "Picked" => TagValue::Flag { flag: 1 },
            "Rejected" => TagValue::Flag { flag: -1 },
            "Photos" => TagValue::Media { media: 0 },
            "Videos" => TagValue::Media { media: 1 },
            "Beach" => TagValue::Scene { tag: "beach".into() },
            "March" => TagValue::Months { months: vec![3] },
            "Summer" => TagValue::Months { months: vec![6, 7, 8] },
            other => TagValue::Text { text: other.to_lowercase() },
        };
        st(label, value)
    }

    /// `"Live or (HDR and not Paris)"` as a sequence; `()` is an empty group.
    fn p(text: &str) -> TagSeq {
        fn seq(t: &[&str], i: &mut usize) -> TagSeq {
            let mut s = TagSeq::default();
            while *i < t.len() && t[*i] != ")" {
                if !s.items.is_empty() {
                    s.joins.push(match t[*i] {
                        "or" => Join::Or,
                        "and" => Join::And,
                        x => panic!("a join, not {x:?}"),
                    });
                    *i += 1;
                }
                let not = t[*i] == "not";
                if not {
                    *i += 1;
                }
                let item = if t[*i] == "(" {
                    *i += 1;
                    let inner = seq(t, i);
                    assert_eq!(t[*i], ")");
                    *i += 1;
                    TagItem::Group { not, seq: inner }
                } else {
                    *i += 1;
                    TagItem::Tag { not, tag: tag(t[*i - 1]) }
                };
                s.items.push(item);
            }
            s
        }
        let spaced = text.replace('(', " ( ").replace(')', " ) ");
        let toks: Vec<&str> = spaced.split_whitespace().collect();
        let mut i = 0;
        let s = seq(&toks, &mut i);
        assert_eq!(i, toks.len(), "all of {text:?} read");
        s
    }

    /// A sequence written back, `and` for a missing join.
    fn show(s: &TagSeq) -> String {
        let mut out = String::new();
        for (i, item) in s.items.iter().enumerate() {
            if i > 0 {
                out += if s.joins.get(i - 1) == Some(&Join::Or) { " or " } else { " and " };
            }
            match item {
                TagItem::Tag { not, tag } => out += &format!("{}{}", if *not { "not " } else { "" }, tag.label),
                TagItem::Group { not, seq } => out += &format!("{}({})", if *not { "not " } else { "" }, show(seq)),
            }
        }
        out
    }

    fn fixed(text: &str) -> String {
        show(&p(text).repaired())
    }

    /// A tree written as `or(A, and(B, C))`, each leaf by its text.
    fn show_expr(e: &QueryExpr) -> String {
        let list = |v: &[QueryExpr]| v.iter().map(show_expr).collect::<Vec<_>>().join(", ");
        match e {
            QueryExpr::And(v) => format!("and({})", list(v)),
            QueryExpr::Or(v) => format!("or({})", list(v)),
            QueryExpr::Not(e) => format!("not({})", show_expr(e)),
            QueryExpr::Leaf(q) => q.text.clone().unwrap_or_default(),
        }
    }

    fn tree(text: &str) -> Option<String> {
        let mut leaf = |t: &SearchTag| QueryExpr::Leaf(Box::new(AssetQuery { text: Some(t.label.clone()), ..Default::default() }));
        p(text).repaired().to_expr(&mut leaf).map(|e| show_expr(&e))
    }

    fn plain(text: &str) -> Option<Vec<String>> {
        p(text).repaired().as_plain().map(|v| v.into_iter().map(|t| t.label).collect())
    }

    #[test]
    fn rule_1_joins_fit_the_items() {
        let mut s = p("A or B and C");
        s.joins = vec![Join::Or];
        assert_eq!(s.repaired().joins, [Join::Or, Join::And], "padded with and");
        s.joins = vec![Join::Or, Join::Or, Join::Or, Join::Or];
        assert_eq!(s.repaired().joins, [Join::Or, Join::Or], "cut");
        let empty = TagSeq { items: vec![], joins: vec![Join::Or] };
        assert_eq!(empty.repaired(), TagSeq::default());
    }

    #[test]
    fn rule_2_an_empty_group_goes_with_one_join() {
        assert_eq!(fixed("A or ()"), "A");
        assert_eq!(fixed("() or A and B"), "A and B", "the first takes the join after it");
        assert_eq!(fixed("A or () and B"), "A and B", "the join before it goes");
        assert_eq!(fixed("A and (() or ()) or B"), "A or B", "emptied by its own empties");
        assert_eq!(fixed("()"), "");
    }

    #[test]
    fn rule_3_a_group_of_one_is_its_item() {
        assert_eq!(fixed("A and not (not B)"), "A and B");
        assert_eq!(fixed("A and not (B)"), "A and not B");
        assert_eq!(fixed("A and (not B)"), "A and not B");
        assert_eq!(fixed("A and not ((B or C))"), "A and not (B or C)", "a group in a group of one");
    }

    #[test]
    fn rule_4_brackets_that_change_nothing_go() {
        assert_eq!(fixed("A or (B and C)"), "A or B and C");
        assert_eq!(fixed("(A and B) and C"), "A and B and C");
        assert_eq!(fixed("(A or B)"), "A or B", "alone");
        assert_eq!(fixed("X or (A or B and C) or Y"), "X or A or B and C or Y", "or on both sides");
        assert_eq!(fixed("A and (B or C)"), "A and (B or C)");
        assert_eq!(fixed("A or (B or C) and D"), "A or (B or C) and D", "and after it");
        assert_eq!(fixed("not (A and B) or C"), "not (A and B) or C", "negated");
        assert_eq!(fixed("A and (B and (C or D))"), "A and B and (C or D)");
    }

    #[test]
    fn rule_5_the_limits() {
        let many = (0..30).map(|i| format!("T{i}")).collect::<Vec<_>>().join(" or ");
        let r = p(&many).repaired();
        assert_eq!(r.items.len(), MAX_TAGS);
        assert_eq!(r.joins.len(), MAX_TAGS - 1);
        assert_eq!(r.leaves().last().unwrap().0.label, "T23", "the first 24 kept");

        // Six groups inside one another, none of whose brackets could go by rule 4.
        let mut text = "L6a or L6b".to_string();
        for d in (0..6).rev() {
            text = format!("L{d}a or L{d}b and ({text})");
        }
        fn depth(s: &TagSeq) -> usize {
            s.items.iter().map(|i| if let TagItem::Group { seq, .. } = i { 1 + depth(seq) } else { 0 }).max().unwrap_or(0)
        }
        let s = p(&text);
        assert_eq!(depth(&s), 6);
        let r = s.repaired();
        assert_eq!(depth(&r), MAX_GROUP_DEPTH);
        assert_eq!(r.leaves().len(), s.leaves().len(), "flattened, not dropped");
    }

    fn cases() -> Vec<(&'static str, TagSeq)> {
        let mut misjoined = p("Live or HDR and Paris");
        misjoined.joins = vec![Join::Or];
        [
            ("one tag", "Live"),
            ("and", "Anna and Ben"),
            ("or binds loosest", "Live or HDR and Paris"),
            ("not", "not Live"),
            ("group and not", "(Live or HDR) and not Paris"),
            ("plain field group", "(Live or HDR) and Paris"),
            ("same field anded", "Live and HDR"),
            ("or across fields", "Live or Paris"),
            ("two flags", "Picked or Rejected"),
            ("flag twice anded", "Picked and Rejected"),
            ("people group", "(Anna or Ben) and Paris"),
            ("months group", "(March or Summer) and Anna"),
            ("empty group", "Live and () or Paris"),
            ("empty first", "() or Live"),
            ("group of one", "Anna and not (not Ben)"),
            ("negated group of one", "Anna and not (Ben)"),
            ("splice and group", "Live or (HDR and Paris)"),
            ("splice between ors", "Anna or (Live or HDR and Paris) or Ben"),
            ("keep group before and", "Anna or (Live or HDR) and Paris"),
            ("keep negated group", "not (Live or HDR) and Paris"),
            ("nested", "Anna and (Ben and (Live or HDR))"),
            ("text tags", "beach and sunset"),
        ]
        .into_iter()
        .map(|(name, text)| (name, p(text)))
        .chain([("joins padded", misjoined)])
        .collect()
    }

    #[test]
    fn repaired_is_idempotent() {
        for (name, s) in cases() {
            let r = s.repaired();
            assert_eq!(r.repaired(), r, "{name}");
            assert_eq!(r.joins.len(), r.items.len().saturating_sub(1), "{name}");
        }
    }

    #[test]
    fn to_expr_shapes() {
        assert_eq!(tree("A and B").as_deref(), Some("and(A, B)"));
        assert_eq!(tree("A or B and C").as_deref(), Some("or(A, and(B, C))"));
        assert_eq!(tree("not A").as_deref(), Some("not(A)"));
        assert_eq!(tree("(A or B) and not C").as_deref(), Some("and(or(A, B), not(C))"));
        assert_eq!(tree("A").as_deref(), Some("A"));
        assert_eq!(tree("()"), None);
        assert_eq!(TagSeq::default().to_expr(&mut core_leaf), None);
    }

    #[test]
    fn as_plain_only_when_the_flat_list_says_the_same() {
        assert_eq!(plain("(Live or HDR) and Paris"), Some(vec!["Live".into(), "HDR".into(), "Paris".into()]));
        assert_eq!(plain("Anna and Ben"), Some(vec!["Anna".into(), "Ben".into()]), "people: and either way");
        assert_eq!(plain("(March or Summer) and Anna").map(|v| v.len()), Some(3));
        assert_eq!(plain("Live and HDR"), None, "the flat list would read or");
        assert_eq!(plain("Live or Paris"), None);
        assert_eq!(plain("not Live"), None);
        assert_eq!(plain("Picked or Rejected"), None);
        assert_eq!(plain("Picked and Rejected"), None, "a single key twice");
        assert_eq!(plain("(Anna or Ben) and Paris"), None, "people in an or group");
        assert_eq!(plain("(Live or Paris) and Anna"), None, "two fields in one group");
        assert_eq!(plain("(Live or HDR) and RAW"), None, "kind in two items");
        assert_eq!(plain(""), Some(vec![]));
    }

    #[test]
    fn leaves_count_for_or_against() {
        let s = p("A and not (B or not C)");
        let got: Vec<(String, bool)> = s.leaves().into_iter().map(|(t, pos)| (t.label.clone(), pos)).collect();
        assert_eq!(got, [("A".into(), true), ("B".into(), false), ("C".into(), true)]);
    }

    /// One value per variant (dates and ranges twice), with its key as the golden file writes it.
    fn samples() -> Vec<(TagValue, Option<&'static str>)> {
        use TagValue as V;
        vec![
            (V::Person { id: 1 }, None),
            (V::Pet { id: 1 }, None),
            (V::Album { id: 1 }, None),
            (V::Collection { id: 1 }, None),
            (V::Scene { tag: "beach".into() }, None),
            (V::Favorites, None),
            (V::HasText, None),
            (V::NoText, None),
            (V::PhotoText { text: "menu".into(), exact: false }, None),
            (V::Text { text: "sunset".into() }, None),
            (V::Lumen { mark: LumenMark::Edited }, None),
            (V::EditedIn { app: Some("photoshop".into()) }, None),
            (V::Media { media: 1 }, Some("field:media")),
            (V::Kind { kind: AssetKind::Live }, Some("field:kind")),
            (city("Paris", "France"), Some("field:place")),
            (V::Extension { ext: "nef".into() }, Some("field:extension")),
            (V::Shape { shape: Shape::Portrait }, Some("field:shape")),
            (V::Rating { min: 3 }, Some("field:rating")),
            (V::Months { months: vec![3] }, Some("field:months")),
            (V::Dates { from: Some(0), to: None, field: None }, Some("field:dates")),
            (V::Dates { from: Some(0), to: None, field: Some(DateField::Taken) }, Some("field:dates")),
            (V::Dates { from: None, to: Some(0), field: Some(DateField::FileModified) }, Some("field:dates:fileModified")),
            (V::Exif { field: ExifField::Model, value: "iPhone 12".into() }, Some("field:exif:model")),
            (V::Range { measure: Measure::FileSize, min: Some(1), max: None }, Some("field:range:fileSize")),
            (V::Range { measure: Measure::Duration, min: None, max: Some(10) }, Some("field:range:duration")),
            (V::Flag { flag: 1 }, Some("single:flag")),
            (V::Location { has: true }, Some("single:location")),
        ]
    }

    #[test]
    fn or_key_follows_the_table() {
        for (value, key) in samples() {
            assert_eq!(or_key(&value).map(|k| k.to_string()).as_deref(), key, "{value:?}");
        }
    }

    #[test]
    fn the_wire_omits_a_false_not() {
        let s = p("Live and not HDR");
        let json = serde_json::to_value(&s).unwrap();
        assert!(json["items"][0].get("not").is_none());
        assert_eq!(json["items"][1]["not"], true);
        assert_eq!(json["joins"][0], "and");
        assert_eq!(json["items"][0]["type"], "tag");
        assert_eq!(serde_json::from_value::<TagSeq>(json).unwrap(), s);
        let q: AssetQuery = serde_json::from_str("{}").unwrap();
        assert_eq!(q.tag_expr, None);
        assert!(!serde_json::to_string(&q).unwrap().contains("tagExpr"));
    }

    fn ids_of(c: &Connection, query: AssetQuery) -> Vec<i64> {
        let scope = crate::scope::LayoutScope { query: Some(query), ..Default::default() };
        let mut ids = crate::layout::ids_scoped(c, &scope, 100).unwrap();
        ids.sort_unstable();
        ids
    }

    fn tree_query(s: &TagSeq) -> AssetQuery {
        AssetQuery { expr: s.to_expr(&mut core_leaf).map(Box::new), ..Default::default() }
    }

    /// `tag_search`'s fixture with what the variants need: Anna on all but the trashed and the
    /// vault photo, Ben on 2 and 3, a second album, dates, sizes, shapes, Live Photos and HDR.
    fn library(c: &Connection) -> crate::Result<()> {
        crate::tag_search::tests::library(c)?;
        let day = |y, m, d| chrono::NaiveDate::from_ymd_opt(y, m, d).unwrap().and_hms_opt(12, 0, 0).unwrap().and_utc().timestamp();
        for (id, captured, w, h) in [
            (1, day(2024, 3, 2), 3000, 4000),
            (2, day(2024, 7, 9), 4000, 3000),
            (3, day(2023, 12, 24), 1000, 1000),
            (4, day(2025, 3, 15), 6000, 4000),
            (5, day(2024, 8, 1), 1920, 1080),
            (7, day(2025, 1, 6), 4000, 6000),
        ] {
            c.execute(
                "UPDATE assets SET captured_at = ?2, width = ?3, height = ?4, byte_size = ?1 * 1000000 WHERE id = ?1",
                rusqlite::params![id, captured, w, h],
            )?;
        }
        c.execute("UPDATE assets SET media_type = ?1 WHERE id IN (1, 2)", [crate::assets::MEDIA_TYPE_LIVE])?;
        c.execute("UPDATE assets SET edited_in = 'photoshop' WHERE id = 5", [])?;
        c.execute_batch(
            "UPDATE assets SET hdr = 1 WHERE id IN (1, 3);
             INSERT INTO people (id, name, created_at) VALUES (2, 'Ben', 0);
             INSERT INTO faces (asset_id, x, y, w, h, score, vec, model, person_id) VALUES
               (3, 0, 0, .1, .1, .9, x'00', 'm', 1), (4, 0, 0, .1, .1, .9, x'00', 'm', 1),
               (5, 0, 0, .1, .1, .9, x'00', 'm', 1), (7, 0, 0, .1, .1, .9, x'00', 'm', 1),
               (2, .5, .5, .1, .1, .9, x'00', 'm', 2), (3, .5, .5, .1, .1, .9, x'00', 'm', 2);
             INSERT INTO albums (id, name, kind, created_at, updated_at) VALUES (10, 'Lisbon', 0, 0, 0);
             INSERT INTO album_items (album_id, asset_id, position, added_at) VALUES (10, 1, 1, 0), (10, 2, 2, 0), (9, 2, 2, 0);",
        )?;
        Ok(())
    }

    #[test]
    fn plain_sequences_find_what_their_tree_finds() {
        use TagValue as V;
        let day = |y, m, d| chrono::NaiveDate::from_ymd_opt(y, m, d).unwrap().and_hms_opt(0, 0, 0).unwrap().and_utc().timestamp();
        let anna = TagItem::Tag { not: false, tag: tag("Anna") };
        let item = |v: TagValue| TagItem::Tag { not: false, tag: st("t", v) };
        // An `or` group of two values of one field, and Anna.
        let field = |a: TagValue, b: TagValue| TagSeq {
            items: vec![TagItem::Group { not: false, seq: TagSeq { items: vec![item(a), item(b)], joins: vec![Join::Or] } }, anna.clone()],
            joins: vec![Join::And],
        };
        // Two tags of a variant the flat list ANDs.
        let both = |a: TagValue, b: TagValue| TagSeq { items: vec![item(a), item(b)], joins: vec![Join::And] };
        let span = |from: Option<i64>, to: Option<i64>, field| V::Dates { from, to, field };
        let range = |min, max| V::Range { measure: Measure::FileSize, min, max };
        let exif = |v: &str| V::Exif { field: ExifField::Model, value: v.into() };
        let sequences = [
            ("media", field(V::Media { media: 0 }, V::Media { media: 1 }), true),
            ("kind", field(V::Kind { kind: AssetKind::Raw }, V::Kind { kind: AssetKind::Live }), true),
            ("place", field(city("Lisbon", "Portugal"), city("Porto", "Portugal")), true),
            ("extension", field(V::Extension { ext: "heic".into() }, V::Extension { ext: "nef".into() }), true),
            ("shape", field(V::Shape { shape: Shape::Portrait }, V::Shape { shape: Shape::Square }), true),
            ("rating", field(V::Rating { min: 5 }, V::Rating { min: 3 }), true),
            ("months", field(V::Months { months: vec![3] }, V::Months { months: vec![7, 8] }), true),
            ("dates", field(span(Some(day(2024, 1, 1)), Some(day(2024, 12, 31)), None), span(Some(day(2025, 1, 1)), None, None)), true),
            ("dates:imported", field(span(Some(-100_000), Some(100_000), Some(DateField::Imported)), span(Some(1_000_000_000), None, Some(DateField::Imported))), true),
            ("exif", field(exif("iPhone 12"), exif("NIKON D7100")), true),
            ("range", field(range(Some(0), Some(2_500_000)), range(Some(4_000_000), None)), true),
            ("person", both(V::Person { id: 1 }, V::Person { id: 2 }), true),
            ("pet", both(V::Pet { id: 1 }, V::Pet { id: 2 }), false),
            ("album", both(V::Album { id: 9 }, V::Album { id: 10 }), true),
            ("collection", both(V::Collection { id: 1 }, V::Collection { id: 2 }), false),
            ("scene", both(V::Scene { tag: "beach".into() }, V::Scene { tag: "sunset".into() }), true),
            ("favorites", both(V::Favorites, V::Favorites), true),
            ("hasText", both(V::HasText, V::HasText), false),
            ("noText", both(V::NoText, V::NoText), false),
            ("photoText", both(V::PhotoText { text: "menu".into(), exact: false }, V::PhotoText { text: "bar".into(), exact: true }), false),
            ("text", both(V::Text { text: "lisbon".into() }, V::Text { text: "portugal".into() }), false),
            ("lumen", both(V::Lumen { mark: LumenMark::Edited }, V::Lumen { mark: LumenMark::Exported }), false),
            ("editedIn", both(V::EditedIn { app: None }, V::EditedIn { app: Some("photoshop".into()) }), true),
        ];
        let db = Db::open_in_memory().unwrap();
        db.with_conn(|c| {
            library(c)?;
            for (name, s, finds) in &sequences {
                let tags = s.as_plain().unwrap_or_else(|| panic!("{name} is plain"));
                let flat = ids_of(c, AssetQuery { search_tags: tags, ..Default::default() });
                assert_eq!(ids_of(c, tree_query(s)), flat, "{name}: tree == flat list");
                assert_eq!(ids_of(c, AssetQuery { tag_expr: Some(s.clone()), ..Default::default() }), flat, "{name}: tag_expr");
                assert_eq!(!flat.is_empty(), *finds, "{name} finds {flat:?}");
            }

            let live_and_hdr = p("Live and HDR");
            assert_eq!(ids_of(c, AssetQuery { tag_expr: Some(live_and_hdr), ..Default::default() }), [1], "only both");
            let live_or_hdr = p("Live or HDR");
            assert_eq!(ids_of(c, AssetQuery { tag_expr: Some(live_or_hdr), ..Default::default() }), [1, 2, 3]);
            let flat = ids_of(c, AssetQuery { search_tags: vec![tag("Live"), tag("HDR")], ..Default::default() });
            assert_eq!(flat, [1, 2, 3], "the flat list reads them as or");
            assert_eq!(ids_of(c, AssetQuery { tag_expr: Some(p("Anna and not (Live or HDR)")), ..Default::default() }), [4, 5, 7]);
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn a_query_without_tag_expr_builds_the_same_sql() {
        let tags = vec![tag("Live"), tag("HDR"), tag("Paris"), tag("Anna")];
        let page = AssetQuery { favorites: true, album_ids: vec![9], ..Default::default() };
        let before = crate::tag_search::merge(&page, &crate::tag_search::to_query(&tags)).sql_parts();
        assert_eq!(AssetQuery { search_tags: tags.clone(), ..page.clone() }.sql_parts(), before);
        let seq = p("(Live or HDR) and Paris and Anna");
        assert_eq!(AssetQuery { tag_expr: Some(seq), ..page.clone() }.sql_parts(), before, "a plain sequence is the flat list");
        let empty = AssetQuery { tag_expr: Some(TagSeq::default()), ..page.clone() };
        assert_eq!(empty.sql_parts(), page.sql_parts());
        assert!(AssetQuery { tag_expr: Some(p("()")), ..Default::default() }.is_empty());
    }

    /// Writes `src/lib/search-expr-golden.json`, which holds the TypeScript copy to these.
    #[test]
    fn golden() {
        let sequences: Vec<_> = cases()
            .into_iter()
            .map(|(name, input)| {
                let repaired = input.repaired();
                serde_json::json!({ "name": name, "input": input, "plain": repaired.as_plain(), "repaired": repaired })
            })
            .collect();
        let keys: Vec<_> = samples()
            .into_iter()
            .map(|(value, key)| {
                assert_eq!(or_key(&value).map(|k| k.to_string()).as_deref(), key);
                serde_json::json!({ "value": value, "key": key })
            })
            .collect();
        let text = serde_json::to_string_pretty(&serde_json::json!({ "sequences": sequences, "keys": keys })).unwrap() + "\n";
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../src/lib/search-expr-golden.json");
        if std::fs::read_to_string(&path).ok().as_deref() != Some(text.as_str()) {
            std::fs::write(&path, text).unwrap();
        }
    }
}
