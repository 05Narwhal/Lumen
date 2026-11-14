//! A sentence typed in the tag field read into tags (Round 20 §2): `live and hdr not screenshot`
//! becomes `[Live Photos] and [HDR] and not [Screenshots]`. Cut by Round 19's lexer and descent
//! ([`crate::query_expr`]); each term resolved to the tag it surely means, else kept as text.

use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::query_expr::{self, Node};
use crate::settings::DateOrder;
use crate::tag_expr::{Join, TagItem, TagSeq};
use crate::tag_search::{self, Facets, SearchTag, TagValue};

/// What a typed sentence reads as.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SentenceReading {
    /// `or hdr`: how the sentence joins what is already chosen.
    pub lead: Option<Join>,
    /// Repaired; may be empty while only an operator is typed.
    pub seq: TagSeq,
    /// The last term as typed, `""` when the text ends in an operator.
    pub term: String,
}

/// `input` read as a sentence of tags; `None` when it is one plain term (no lead, no trailing
/// operator, no operator at all): the field then behaves as before.
pub fn read(facets: &Facets, input: &str, today: NaiveDate, order: DateOrder) -> Option<SentenceReading> {
    let mut body = input.trim();
    let join = |w: &str| match w.to_lowercase().as_str() {
        "and" => Some(Join::And),
        "or" => Some(Join::Or),
        _ => None,
    };
    // A first `and` / `or` with more after it joins the sentence to what is already chosen.
    let mut lead = None;
    if let Some((first, rest)) = body.split_once(char::is_whitespace) {
        if let Some(j) = join(first).filter(|_| !rest.trim().is_empty()) {
            lead = Some(j);
            body = rest.trim_start();
        }
    }
    // Trailing operators: the person is still typing.
    let mut tail = false;
    loop {
        let (rest, last) = body.rsplit_once(char::is_whitespace).unwrap_or(("", body));
        if last.is_empty() || (join(last).is_none() && !last.eq_ignore_ascii_case("not")) {
            break;
        }
        tail = true;
        body = rest.trim_end();
    }

    let node = match query_expr::parse_nodes(body) {
        Some(node) => node,
        None if lead.is_none() && !tail => return None,
        None if body.is_empty() => return Some(SentenceReading { lead, seq: TagSeq::default(), term: String::new() }),
        None => Node::Term(0, body.len()),
    };
    let term = if tail { String::new() } else { node.last_term().map(|(s, e)| body[s..e].to_string()).unwrap_or_default() };
    let mut resolve = |s: usize, e: usize| resolve(facets, &body[s..e], today, order);
    let seq = sequence(node, &mut resolve).repaired();
    Some(SentenceReading { lead, seq, term })
}

/// The tag `term` surely means: a quoted phrase is its text, else the first sure
/// [`tag_search::candidates`] entry, else the term as text (§1's app leaf then reads it as a
/// sentence, so nothing typed is lost).
pub fn resolve(facets: &Facets, term: &str, today: NaiveDate, order: DateOrder) -> SearchTag {
    let term = term.trim();
    let quote = |c: char| matches!(c, '"' | '“' | '”' | '„');
    let mut cs = term.chars();
    if let (Some(a), Some(b)) = (cs.next(), cs.next_back()) {
        let inside = cs.as_str();
        if quote(a) && quote(b) && !inside.contains(quote) && !inside.trim().is_empty() {
            return text(inside.trim());
        }
    }
    tag_search::candidates(facets, term, today, order).into_iter().find(|c| c.sure).map(|c| c.suggestion.tag).unwrap_or_else(|| text(term))
}

fn text(text: &str) -> SearchTag {
    SearchTag { label: text.into(), value: TagValue::Text { text: text.into() } }
}

/// The tree as a sequence: `and` children joined by `and` (an `or` child in a group), `or`
/// children joined by `or` (an `and` child spliced in, since `and` binds tighter).
fn sequence(node: Node, read: &mut impl FnMut(usize, usize) -> SearchTag) -> TagSeq {
    let mut seq = TagSeq::default();
    match node {
        Node::And(v) => {
            for (i, child) in v.into_iter().enumerate() {
                if i > 0 {
                    seq.joins.push(Join::And);
                }
                seq.items.push(item(child, read));
            }
        }
        Node::Or(v) => {
            for (i, child) in v.into_iter().enumerate() {
                if i > 0 {
                    seq.joins.push(Join::Or);
                }
                match child {
                    Node::And(_) => {
                        let inner = sequence(child, read);
                        seq.items.extend(inner.items);
                        seq.joins.extend(inner.joins);
                    }
                    child => seq.items.push(item(child, read)),
                }
            }
        }
        node => seq.items.push(item(node, read)),
    }
    seq
}

fn item(node: Node, read: &mut impl FnMut(usize, usize) -> SearchTag) -> TagItem {
    match node {
        Node::Term(s, e) => TagItem::Tag { not: false, tag: read(s, e) },
        Node::Not(n) => match item(*n, read) {
            TagItem::Tag { not, tag } => TagItem::Tag { not: !not, tag },
            TagItem::Group { not, seq } => TagItem::Group { not: !not, seq },
        },
        group => TagItem::Group { not: false, seq: sequence(group, read) },
    }
}

#[cfg(test)]
mod tests {
    use rusqlite::Connection;

    use super::*;
    use crate::db::Db;
    use crate::query::AssetKind;

    /// A Live Photo, an HDR photo, a screenshot, Paris and Rome, and two Annas.
    fn library(c: &Connection) -> crate::Result<()> {
        c.execute_batch(
            "INSERT INTO assets (id, content_hash, relative_path, filename, extension, media_type, byte_size, orientation, imported_at, place_name, hdr)
               VALUES (1, 'h1', 'p', 'IMG_1.heic', 'heic', 2, 1, 1, 10, NULL, 0),
                      (2, 'h2', 'p', 'IMG_2.heic', 'heic', 0, 1, 1, 10, NULL, 1),
                      (3, 'h3', 'p', 'Screenshot 1.png', 'png', 0, 1, 1, 10, NULL, 0),
                      (4, 'h4', 'p', 'IMG_4.jpg', 'jpg', 0, 1, 1, 10, 'Paris, France', 0),
                      (5, 'h5', 'p', 'IMG_5.jpg', 'jpg', 0, 1, 1, 10, 'Rome, Italy', 0);
             INSERT INTO people (id, name, created_at) VALUES (1, 'Anna Berg', 0), (2, 'Anna Kos', 0);
             INSERT INTO faces (asset_id, x, y, w, h, score, vec, model, person_id)
               VALUES (4, 0, 0, .1, .1, .9, x'00', 'm', 1), (5, 0, 0, .1, .1, .9, x'00', 'm', 2);",
        )?;
        Ok(())
    }

    fn reading(input: &str) -> Option<SentenceReading> {
        let db = Db::open_in_memory().unwrap();
        db.with_conn(|c| {
            library(c)?;
            let f = Facets::load(c)?;
            Ok(read(&f, input, NaiveDate::from_ymd_opt(2026, 11, 12).unwrap(), DateOrder::Dmy))
        })
        .unwrap()
    }

    fn tag(item: &TagItem) -> (&TagValue, bool) {
        match item {
            TagItem::Tag { not, tag } => (&tag.value, *not),
            TagItem::Group { .. } => panic!("a group: {item:?}"),
        }
    }

    fn text(item: &TagItem) -> &str {
        match tag(item).0 {
            TagValue::Text { text } => text,
            other => panic!("not text: {other:?}"),
        }
    }

    #[test]
    fn kinds_with_and_and_not() {
        let r = reading("live and hdr not screenshot").expect("a sentence");
        assert_eq!(r.lead, None);
        assert_eq!(r.seq.joins, vec![Join::And, Join::And]);
        let kinds: Vec<_> = r.seq.items.iter().map(tag).collect();
        assert_eq!(
            kinds,
            vec![
                (&TagValue::Kind { kind: AssetKind::Live }, false),
                (&TagValue::Kind { kind: AssetKind::Hdr }, false),
                (&TagValue::Kind { kind: AssetKind::Screenshot }, true),
            ]
        );
        assert_eq!(r.term, "screenshot");
    }

    #[test]
    fn a_bracketed_or_becomes_a_group() {
        let r = reading("(paris or rome) 2026").expect("a sentence");
        assert_eq!(r.seq.joins, vec![Join::And]);
        let TagItem::Group { not: false, seq } = &r.seq.items[0] else { panic!("a group first: {:?}", r.seq) };
        assert_eq!(seq.joins, vec![Join::Or]);
        assert!(seq.items.iter().all(|i| matches!(tag(i).0, TagValue::Place { .. })), "{seq:?}");
        assert!(matches!(tag(&r.seq.items[1]).0, TagValue::Dates { .. }), "{:?}", r.seq);
        assert_eq!(r.term, "2026");
    }

    #[test]
    fn a_leading_operator_is_the_lead() {
        let r = reading("or hdr").expect("a lead");
        assert_eq!(r.lead, Some(Join::Or));
        assert_eq!(r.seq.items.len(), 1);
        assert_eq!(tag(&r.seq.items[0]), (&TagValue::Kind { kind: AssetKind::Hdr }, false));
        assert_eq!(r.term, "hdr");
    }

    #[test]
    fn a_trailing_operator_is_cut_and_empties_the_term() {
        let r = reading("live and ").expect("a tail");
        assert_eq!(r.lead, None);
        assert_eq!(r.seq.items.iter().map(tag).collect::<Vec<_>>(), vec![(&TagValue::Kind { kind: AssetKind::Live }, false)]);
        assert_eq!(r.term, "");
    }

    #[test]
    fn not_alone_is_no_lead() {
        let r = reading("not screenshot").expect("a sentence");
        assert_eq!(r.lead, None);
        assert_eq!(r.seq.items.iter().map(tag).collect::<Vec<_>>(), vec![(&TagValue::Kind { kind: AssetKind::Screenshot }, true)]);
    }

    #[test]
    fn plain_text_is_no_sentence() {
        assert_eq!(reading("\"rock and roll\""), None, "no operator outside the quotes");
        assert_eq!(reading("live hdr"), None, "no operator");
    }

    #[test]
    fn words_with_no_sure_tag_stay_text() {
        let r = reading("beach or mountains").expect("a sentence");
        assert_eq!(r.seq.joins, vec![Join::Or]);
        assert_eq!(r.seq.items.iter().map(text).collect::<Vec<_>>(), vec!["beach", "mountains"]);
        let r = reading("anna and paris").expect("a sentence");
        assert_eq!(text(&r.seq.items[0]), "anna", "two Annas: not sure which");
        assert!(matches!(tag(&r.seq.items[1]).0, TagValue::Place { .. }));
    }

    #[test]
    fn a_quoted_phrase_is_its_text() {
        let r = reading("\"rock and roll\" or live").expect("a sentence");
        assert_eq!(text(&r.seq.items[0]), "rock and roll");
        assert_eq!(tag(&r.seq.items[1]).0, &TagValue::Kind { kind: AssetKind::Live });
    }
}
