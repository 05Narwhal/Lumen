//! Captioner's wording: what a title and a caption say, built only from what Lumen already
//! knows about a photo (the people named on it, its place, the scene Lumen saw, the text it read,
//! when it was taken). No model, no guessing beyond those words: a photo that has none of them
//! gets nothing proposed.

/// What is known about one photo (all from `library.details` and `library.assets`).
#[derive(Default, Clone, Debug)]
pub struct Facts {
    /// `YYYY-MM-DDTHH:MM:SS`, wall clock.
    pub wall: Option<String>,
    /// A place name such as `Idrija, Slovenia`.
    pub place: Option<String>,
    /// The place was estimated (Wayfinder, a folder's name) and not recorded by the camera.
    pub estimated_place: bool,
    pub people: Vec<String>,
    /// Scene words from Lumen's own look at the photo (`sunset`, `group of people`).
    pub scene: Vec<String>,
    pub text: Vec<String>,
    pub albums: Vec<String>,
    pub has_title: bool,
    pub has_caption: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Options {
    pub titles: bool,
    pub captions: bool,
    pub text: bool,
    pub album: bool,
}

impl Default for Options {
    fn default() -> Self {
        Options { titles: true, captions: true, text: true, album: false }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Draft {
    pub title: Option<String>,
    pub caption: Option<String>,
    pub confidence: f32,
    /// What the wording was built from (shown beside the proposal; at most 200 characters).
    pub reason: String,
}

const MONTHS: [&str; 12] = ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];
const DAYS: [&str; 7] = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];

/// `(year, month 1-12, day, hour)` of a wall clock.
fn parts(wall: &str) -> Option<(i32, usize, u32, u32)> {
    let b = wall.as_bytes();
    if b.len() != 19 || b[4] != b'-' || b[7] != b'-' || b[10] != b'T' {
        return None;
    }
    let y = wall.get(0..4)?.parse().ok()?;
    let m: usize = wall.get(5..7)?.parse().ok()?;
    let d = wall.get(8..10)?.parse().ok()?;
    let h = wall.get(11..13)?.parse().ok()?;
    ((1..=12).contains(&m) && (1..=31).contains(&d) && h < 24 && y > 0).then_some((y, m, d, h))
}

/// Sunday = 0 (Sakamoto's method).
fn weekday(y: i32, m: usize, d: u32) -> usize {
    const T: [i32; 12] = [0, 3, 2, 5, 0, 3, 5, 1, 4, 6, 2, 4];
    let y = if m < 3 { y - 1 } else { y };
    ((y + y / 4 - y / 100 + y / 400 + T[m - 1] + d as i32).rem_euclid(7)) as usize
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Part {
    Morning,
    Afternoon,
    Evening,
    Night,
}

fn part_of_day(hour: u32) -> Part {
    match hour {
        5..=11 => Part::Morning,
        12..=16 => Part::Afternoon,
        17..=20 => Part::Evening,
        _ => Part::Night,
    }
}

impl Part {
    fn noun(self) -> &'static str {
        match self {
            Part::Morning => "Morning",
            Part::Afternoon => "Afternoon",
            Part::Evening => "Evening",
            Part::Night => "Night",
        }
    }
    fn phrase(self) -> &'static str {
        match self {
            Part::Morning => "in the morning",
            Part::Afternoon => "in the afternoon",
            Part::Evening => "in the evening",
            Part::Night => "at night",
        }
    }
}

/// `Anna`, `Anna and Ben`, `Anna, Ben and Carl`, `Anna, Ben and 2 others`.
pub fn names(people: &[String]) -> Option<String> {
    let p: Vec<&str> = people.iter().map(|s| s.trim()).filter(|s| !s.is_empty()).collect();
    Some(match p.len() {
        0 => return None,
        1 => p[0].to_string(),
        2 => format!("{} and {}", p[0], p[1]),
        3 => format!("{}, {} and {}", p[0], p[1], p[2]),
        n => format!("{}, {} and {} others", p[0], p[1], n - 2),
    })
}

/// Scene words in the order they make the better title: an occasion before a landscape before an
/// activity before a thing; the vague ones last. `(word, title phrase)`.
const SCENES: &[(&str, &str)] = &[
    ("wedding", "Wedding"), ("fireworks", "Fireworks"), ("christmas", "Christmas"), ("concert", "Concert"), ("party", "Party"), ("stage", "On stage"),
    ("sunset", "Sunset"), ("sunrise", "Sunrise"), ("night sky", "Night sky"), ("waterfall", "Waterfall"), ("beach", "Beach"), ("mountain", "Mountains"),
    ("lake", "Lake"), ("river", "River"), ("sea", "By the sea"), ("forest", "Forest"), ("desert", "Desert"), ("snow", "Snow"), ("autumn leaves", "Autumn leaves"),
    ("fog", "Fog"), ("rain", "Rain"), ("flowers", "Flowers"), ("skiing", "Skiing"), ("hiking", "Hiking"), ("swimming", "Swimming"), ("camping", "Camping"),
    ("cycling", "Cycling"), ("football", "Football"), ("sport", "Sport"), ("castle", "Castle"), ("church", "Church"), ("bridge", "Bridge"), ("museum", "Museum"),
    ("painting", "Painting"), ("restaurant", "Restaurant"), ("cake", "Cake"), ("coffee", "Coffee"), ("drinks", "Drinks"), ("food", "Food"), ("dog", "Dog"),
    ("cat", "Cat"), ("horse", "Horse"), ("bird", "Bird"), ("baby", "Baby"), ("boat", "Boat"), ("train", "Train"), ("airplane", "Airplane"), ("bicycle", "Bicycle"),
    ("garden", "Garden"), ("city", "City"), ("street", "Street"), ("building", "Building"),
];

fn best_scene(scene: &[String]) -> Option<(&'static str, &'static str)> {
    SCENES.iter().find(|(w, _)| scene.iter().any(|s| s == w)).copied()
}

fn lower_first(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_lowercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}

/// The part of a place name before its first comma: `Idrija, Slovenia` -> `Idrija`.
fn short_place(place: &str) -> &str {
    place.split(',').next().unwrap_or(place).trim()
}

/// The first line of text that reads like a sign or a heading and not like a number, a price or a
/// page of print; none when the photo is mostly text.
fn useful_text(lines: &[String]) -> Option<String> {
    if lines.len() >= 8 {
        return None;
    }
    lines.iter().map(|l| l.trim()).find(|l| {
        let letters = l.chars().filter(|c| c.is_alphabetic()).count();
        let n = l.chars().count();
        letters >= 3 && n <= 40 && letters * 10 >= n * 6
    }).map(str::to_string)
}

fn list_words(items: &[&str]) -> String {
    match items {
        [] => String::new(),
        [a] => a.to_string(),
        [a, b] => format!("{a} and {b}"),
        _ => format!("{} and {}", items[..items.len() - 1].join(", "), items[items.len() - 1]),
    }
}

/// A title and a caption for one photo, or `None` when nothing is known about it that would make
/// them more than a date.
pub fn compose(f: &Facts, o: &Options) -> Option<Draft> {
    let who = names(&f.people);
    let place = f.place.as_deref().map(str::trim).filter(|p| !p.is_empty());
    let scene = best_scene(&f.scene);
    let when = f.wall.as_deref().and_then(parts);
    if who.is_none() && place.is_none() && scene.is_none() {
        return None;
    }
    let part = when.map(|(_, _, _, h)| part_of_day(h));
    let month_year = when.map(|(y, m, _, _)| format!("{} {y}", MONTHS[m - 1]));

    let title = (o.titles && !f.has_title).then(|| match (&who, place, scene) {
        (Some(w), Some(p), _) => format!("{w} in {}", short_place(p)),
        (Some(w), None, Some((_, s))) => format!("{w}, {}", lower_first(s)),
        (Some(w), None, None) => match &month_year {
            Some(my) => format!("{w}, {my}"),
            None => w.clone(),
        },
        (None, Some(p), Some((_, s))) => format!("{s} in {}", short_place(p)),
        (None, None, Some((_, s))) => match &month_year {
            Some(my) => format!("{s}, {my}"),
            None => s.to_string(),
        },
        (None, Some(p), None) => match part {
            Some(t) => format!("{} in {}", t.noun(), short_place(p)),
            None => short_place(p).to_string(),
        },
        (None, None, None) => unreachable!("handled above"),
    });

    let caption = (o.captions && !f.has_caption).then(|| {
        let first = match (&who, place, scene) {
            (Some(w), Some(p), _) => format!("{w} in {p}"),
            (Some(w), None, _) => w.clone(),
            (None, Some(p), _) => p.to_string(),
            (None, None, Some((_, s))) => s.to_string(),
            (None, None, None) => unreachable!("handled above"),
        };
        let mut out = vec![format!("{first}.")];
        if let Some((y, m, d, _)) = when {
            let day = format!("{} {d} {} {y}", DAYS[weekday(y, m, d)], MONTHS[m - 1]);
            out.push(match part {
                Some(t) => format!("{day}, {}.", t.phrase()),
                None => format!("{day}."),
            });
        }
        // The scene is Lumen's own look at the photo, so it is said as a look.
        let looks: Vec<&str> = f.scene.iter().filter(|s| *s != "group of people" && *s != "selfie").take(3).map(String::as_str).collect();
        if !looks.is_empty() && (who.is_some() || place.is_some()) {
            out.push(format!("Looks like {}.", list_words(&looks)));
        }
        if o.text {
            if let Some(t) = useful_text(&f.text) {
                out.push(format!("Reads \u{201c}{t}\u{201d}."));
            }
        }
        if o.album {
            if let Some(a) = f.albums.first() {
                out.push(format!("From the album \u{201c}{a}\u{201d}."));
            }
        }
        out.join(" ")
    });
    if title.is_none() && caption.is_none() {
        return None;
    }

    let ingredients = who.is_some() as u8 + place.is_some() as u8 + scene.is_some() as u8;
    let mut confidence = match ingredients {
        3 => 0.8,
        2 => 0.65,
        _ => 0.5,
    };
    if f.estimated_place && place.is_some() {
        confidence -= 0.1;
    }
    let mut from: Vec<String> = Vec::new();
    if let Some(w) = &who {
        from.push(format!("people ({w})"));
    }
    if let Some(p) = place {
        from.push(if f.estimated_place { format!("an estimated place ({})", short_place(p)) } else { format!("place ({})", short_place(p)) });
    }
    if let Some((w, _)) = scene {
        from.push(format!("what Lumen saw ({w})"));
    }
    if when.is_some() {
        from.push("date".to_string());
    }
    if o.captions && o.text && useful_text(&f.text).is_some() {
        from.push("text in the photo".to_string());
    }
    let reason: String = format!("Built from {}.", from.join(", ")).chars().take(200).collect();
    Some(Draft { title, caption, confidence, reason })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn f() -> Facts {
        Facts { wall: Some("2026-03-12T18:40:00".into()), ..Default::default() }
    }
    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn people_in_a_place_make_the_best_title_and_say_when() {
        let d = compose(&Facts { people: s(&["Anna", "Ben"]), place: Some("Idrija, Slovenia".into()), scene: s(&["sunset", "sea"]), ..f() }, &Options::default()).unwrap();
        assert_eq!(d.title.as_deref(), Some("Anna and Ben in Idrija"));
        assert_eq!(d.caption.as_deref(), Some("Anna and Ben in Idrija, Slovenia. Thursday 12 March 2026, in the evening. Looks like sunset and sea."));
        assert_eq!(d.confidence, 0.8);
        assert_eq!(d.reason, "Built from people (Anna and Ben), place (Idrija), what Lumen saw (sunset), date.");
    }

    #[test]
    fn a_title_falls_back_one_ingredient_at_a_time() {
        let o = Options::default();
        let t = |facts: Facts| compose(&facts, &o).unwrap().title.unwrap();
        assert_eq!(t(Facts { people: s(&["Anna"]), scene: s(&["beach"]), ..f() }), "Anna, beach");
        assert_eq!(t(Facts { people: s(&["Anna"]), ..f() }), "Anna, March 2026");
        assert_eq!(t(Facts { place: Some("Piran, Slovenia".into()), scene: s(&["sunset"]), ..f() }), "Sunset in Piran");
        assert_eq!(t(Facts { place: Some("Piran, Slovenia".into()), ..f() }), "Evening in Piran");
        assert_eq!(t(Facts { place: Some("Piran".into()), wall: None, ..f() }), "Piran");
        assert_eq!(t(Facts { scene: s(&["waterfall", "forest"]), ..f() }), "Waterfall, March 2026", "the better word wins");
        assert_eq!(t(Facts { people: s(&["A", "B", "C", "D", "E"]), place: Some("Rome".into()), ..f() }), "A, B and 3 others in Rome");
    }

    #[test]
    fn nothing_known_means_nothing_proposed() {
        let o = Options::default();
        assert_eq!(compose(&f(), &o), None, "a date alone is no title");
        assert_eq!(compose(&Facts { scene: s(&["group of people", "selfie"]), ..f() }, &o), None, "vague scenes say nothing");
        assert_eq!(compose(&Facts { text: s(&["Hello there"]), ..f() }, &o), None, "text alone isn't enough either");
    }

    #[test]
    fn a_photo_that_has_a_title_or_caption_keeps_it() {
        let facts = Facts { place: Some("Bled, Slovenia".into()), has_title: true, ..f() };
        let d = compose(&facts, &Options::default()).unwrap();
        assert_eq!(d.title, None);
        assert!(d.caption.unwrap().starts_with("Bled, Slovenia."));
        let both = Facts { has_caption: true, ..facts };
        assert_eq!(compose(&both, &Options::default()), None);
        let off = Options { titles: false, captions: false, ..Options::default() };
        assert_eq!(compose(&Facts { place: Some("Bled".into()), ..f() }, &off), None);
    }

    #[test]
    fn text_is_quoted_only_when_it_reads_like_a_sign() {
        let o = Options::default();
        let cap = |lines: &[&str]| compose(&Facts { place: Some("Bled".into()), text: s(lines), ..f() }, &o).unwrap().caption.unwrap();
        assert!(cap(&["12.50", "OPEN 24h"]).ends_with("Reads \u{201c}OPEN 24h\u{201d}."));
        assert!(!cap(&["12.50", "3 x 4"]).contains("Reads"), "numbers aren't a sign");
        assert!(!cap(&["a"; 9]).contains("Reads"), "a page of print isn't one either");
        assert!(!cap(&["A very long line of text that goes on and on and on"]).contains("Reads"));
        let off = Options { text: false, ..o };
        let c = compose(&Facts { place: Some("Bled".into()), text: s(&["OPEN"]), ..f() }, &off).unwrap();
        assert!(!c.caption.unwrap().contains("Reads"));
    }

    #[test]
    fn an_estimated_place_is_trusted_less_and_said_so() {
        let d = compose(&Facts { place: Some("Bled, Slovenia".into()), estimated_place: true, ..f() }, &Options::default()).unwrap();
        assert!((d.confidence - 0.4).abs() < 1e-6, "{}", d.confidence);
        assert!(d.reason.contains("an estimated place (Bled)"), "{}", d.reason);
    }

    #[test]
    fn the_album_is_mentioned_only_when_asked() {
        let facts = Facts { place: Some("Bled".into()), albums: s(&["Slovenia 2026"]), ..f() };
        assert!(!compose(&facts, &Options::default()).unwrap().caption.unwrap().contains("album"));
        let with = Options { album: true, ..Options::default() };
        assert!(compose(&facts, &with).unwrap().caption.unwrap().ends_with("From the album \u{201c}Slovenia 2026\u{201d}."));
    }

    #[test]
    fn dates_and_parts_of_the_day() {
        assert_eq!(weekday(2026, 3, 12), 4, "a Thursday");
        assert_eq!(weekday(2000, 1, 1), 6, "a Saturday");
        assert_eq!(weekday(2024, 2, 29), 4);
        assert_eq!([4, 5, 11, 12, 16, 17, 20, 21, 23].map(|h| part_of_day(h).noun()), ["Night", "Morning", "Morning", "Afternoon", "Afternoon", "Evening", "Evening", "Night", "Night"]);
        assert_eq!(parts("2026-13-01T00:00:00"), None);
        assert_eq!(parts("yesterday"), None);
        let no_time = compose(&Facts { place: Some("Bled".into()), wall: None, ..Default::default() }, &Options::default()).unwrap();
        assert_eq!(no_time.caption.as_deref(), Some("Bled."));
    }

    #[test]
    fn a_reason_fits_the_review() {
        let long = Facts { people: vec!["N".repeat(150), "M".repeat(150)], place: Some("P".repeat(100)), scene: s(&["sunset"]), ..f() };
        assert!(compose(&long, &Options::default()).unwrap().reason.chars().count() <= 200);
    }
}
