//! Google Takeout file-name rules.
//!
//! Takeout shortens names: a sidecar's full file name (media name plus
//! `.supplemental-metadata.json`) is cut to 51 characters including `.json`,
//! so the part before `.json` is at most 46 characters. A duplicate counter
//! `IMG_1234(1).jpg` moves in the sidecar to `IMG_1234.jpg(1).json`.

const STEM_LIMIT: usize = 46;
const SUPPLEMENTAL: &str = ".supplemental-metadata";

/// Year-folder prefixes (lower case, with the trailing space) by language:
/// English, German, French, Spanish and Portuguese ("Fotos de"), Dutch,
/// Italian ("Foto dal"), Italian also "Foto del".
const YEAR_PREFIXES: &[&str] = &[
    "photos from ",
    "fotos von ",
    "photos de ",
    "fotos de ",
    "foto's uit ",
    "foto dal ",
    "foto del ",
];

/// Suffixes Google adds to an edited copy, lower case: English, German,
/// French, Spanish/Portuguese, Dutch, Italian.
const EDITED_SUFFIXES: &[&str] = &[
    "-edited",
    "-bearbeitet",
    "-modifié",
    "-editado",
    "-bewerkt",
    "-modificato",
];

fn cut(s: &str, n: usize) -> String {
    s.chars().take(n).collect()
}

/// Splits `IMG_1234(1).jpg` into (`IMG_1234`, `(1)`, `.jpg`).
fn split_counter(name: &str) -> Option<(String, String, String)> {
    let (stem, ext) = match name.rfind('.') {
        Some(i) if i > 0 => (&name[..i], &name[i..]),
        _ => (name, ""),
    };
    let open = stem.rfind('(')?;
    let tail = &stem[open..];
    let digits = tail.strip_prefix('(')?.strip_suffix(')')?;
    if digits.is_empty() || !digits.chars().all(|c| c.is_ascii_digit()) || open == 0 {
        return None;
    }
    Some((stem[..open].to_string(), tail.to_string(), ext.to_string()))
}

fn push(out: &mut Vec<String>, s: String) {
    if !out.contains(&s) {
        out.push(s);
    }
}

/// `name` is the media name, `tail` a duplicate counter that goes before `.json`.
fn variants(out: &mut Vec<String>, name: &str, tail: &str) {
    let room = STEM_LIMIT.saturating_sub(tail.chars().count());
    for k in (1..=SUPPLEMENTAL.len()).rev() {
        let stem = format!("{name}{}", &SUPPLEMENTAL[..k]);
        push(out, format!("{}{tail}.json", cut(&stem, room)));
    }
    push(out, format!("{name}{tail}.json"));
    push(out, format!("{}{tail}.json", cut(name, room)));
}

/// Sidecar names to try for the media file `media_name`, best first.
pub fn sidecar_candidates(media_name: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut fallbacks = Vec::new();
    let (stem, _ext) = match media_name.rfind('.') {
        Some(i) if i > 0 => (&media_name[..i], &media_name[i..]),
        _ => (media_name, ""),
    };
    if let Some((base, tail, ext)) = split_counter(media_name) {
        variants(&mut out, &format!("{base}{ext}"), &tail);
        variants(&mut out, media_name, "");
        push(&mut fallbacks, format!("{base}{tail}.json"));
        push(&mut fallbacks, format!("{base}.json"));
    } else {
        variants(&mut out, media_name, "");
    }
    push(&mut fallbacks, format!("{stem}.json"));
    for f in fallbacks {
        push(&mut out, f);
    }
    out
}

/// `Photos from 2019` (or a localised form) gives the year.
pub fn is_year_folder(name: &str) -> Option<i32> {
    let lower = name.trim().to_lowercase();
    YEAR_PREFIXES.iter().find_map(|p| {
        let rest = lower.strip_prefix(p)?;
        if rest.len() == 4 && rest.chars().all(|c| c.is_ascii_digit()) {
            rest.parse().ok()
        } else {
            None
        }
    })
}

/// `IMG_1-edited.jpg` gives `IMG_1.jpg`; `IMG_1-edited(1).jpg` gives `IMG_1(1).jpg`;
/// `None` if the name is not an edited copy.
pub fn edited_original(name: &str) -> Option<String> {
    let (stem, ext) = match name.rfind('.') {
        Some(i) if i > 0 => (&name[..i], &name[i..]),
        _ => (name, ""),
    };
    // A duplicate counter after the suffix stays after the original's name.
    let (stem, counter) = match split_counter(name) {
        Some((base, tail, _)) => (&stem[..base.len()], tail),
        None => (stem, String::new()),
    };
    let lower = stem.to_lowercase();
    for suf in EDITED_SUFFIXES {
        if lower.ends_with(suf) && lower.len() > suf.len() {
            // Lower-casing may change byte lengths for non-ASCII; cut by chars.
            let keep = stem.chars().count() - suf.chars().count();
            return Some(format!("{}{counter}{ext}", cut(stem, keep)));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sidecar_name_rules() {
        let c = sidecar_candidates("IMG_1.jpg");
        assert_eq!(c[0], "IMG_1.jpg.supplemental-metadata.json");
        assert!(c.contains(&"IMG_1.jpg.supplemental-metadat.json".to_string()));
        assert!(c.contains(&"IMG_1.jpg.s.json".to_string()));
        assert!(c.contains(&"IMG_1.jpg..json".to_string()));
        assert!(c.contains(&"IMG_1.jpg.json".to_string()));
        assert_eq!(c.last().unwrap(), "IMG_1.json");
        let p = |s: &str| c.iter().position(|x| x == s).unwrap();
        assert!(p("IMG_1.jpg.supplemental-metadata.json") < p("IMG_1.jpg.json"));
        assert!(p("IMG_1.jpg.json") < p("IMG_1.json"));

        // Full sidecar name cut to 51 characters including ".json".
        let long = "IMG_20190101_123456789_BURST001_COVER.jpg";
        let c = sidecar_candidates(long);
        assert!(c.iter().all(|x| x.chars().count() <= 51 || x == &format!("{long}.json")));
        let cut = format!("{long}.supplemental-metadata").chars().take(46).collect::<String>() + ".json";
        assert!(c.contains(&cut), "{c:?}");
        // 46-character media-name limit.
        let limit: String = long.chars().take(46).collect();
        assert!(c.contains(&format!("{limit}.json")));

        // Duplicate counter moves before .json.
        let c = sidecar_candidates("IMG_1234(1).jpg");
        assert_eq!(c[0], "IMG_1234.jpg.supplemental-metadata(1).json");
        assert!(c.contains(&"IMG_1234.jpg(1).json".to_string()));
        assert!(c.contains(&"IMG_1234(1).jpg.json".to_string()));
        let mut d = c.clone();
        d.dedup();
        assert_eq!(d.len(), c.len());
    }

    #[test]
    fn localised_names() {
        assert_eq!(is_year_folder("Photos from 2019"), Some(2019));
        assert_eq!(is_year_folder("Fotos von 2020"), Some(2020));
        assert_eq!(is_year_folder("Photos de 2018"), Some(2018));
        assert_eq!(is_year_folder("Fotos de 2017"), Some(2017));
        assert_eq!(is_year_folder("Foto's uit 2016"), Some(2016));
        assert_eq!(is_year_folder("Foto dal 2015"), Some(2015));
        assert_eq!(is_year_folder("photos from 2019"), Some(2019));
        assert_eq!(is_year_folder("Holiday 2019"), None);
        assert_eq!(is_year_folder("Photos from 20x9"), None);
        assert_eq!(is_year_folder("Photos from 2019 trip"), None);
        assert_eq!(is_year_folder("Photos from 99999"), None);
    }

    #[test]
    fn edited_suffixes() {
        assert_eq!(edited_original("IMG_1-edited.jpg").as_deref(), Some("IMG_1.jpg"));
        assert_eq!(edited_original("IMG_1-bearbeitet.jpg").as_deref(), Some("IMG_1.jpg"));
        assert_eq!(edited_original("IMG_1-modifié.JPG").as_deref(), Some("IMG_1.JPG"));
        assert_eq!(edited_original("a-editado.png").as_deref(), Some("a.png"));
        assert_eq!(edited_original("a-bewerkt.png").as_deref(), Some("a.png"));
        assert_eq!(edited_original("a-modificato.png").as_deref(), Some("a.png"));
        assert_eq!(edited_original("IMG_1.jpg"), None);
        assert_eq!(edited_original("-edited.jpg"), None);
        assert_eq!(edited_original("IMG_1-edited"), Some("IMG_1".to_string()));
        assert_eq!(edited_original("IMG_1-edited(1).jpg").as_deref(), Some("IMG_1(1).jpg"));
        assert_eq!(edited_original("IMG_1(1).jpg"), None);
    }
}
