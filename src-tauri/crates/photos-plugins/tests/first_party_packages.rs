//! Round 26 C2b: the first-party themes pass install's checks (contrast, `theme.css`) and every
//! Film Lab preset reads through the same checks Lumen runs on a plugin's presets, with every
//! value already inside its range (nothing is quietly clamped).

use std::path::PathBuf;

use photos_core::develop_presets::parse_plugin_preset;
use photos_plugins::theme_pkg::{css, open_folder};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../..")
}

fn lumen() -> semver::Version {
    "26.2.0-alpha".parse().unwrap()
}

#[test]
fn first_party_themes_pass_install_checks() {
    let mut all = Vec::new();
    for name in ["nocturne", "coastline", "paper-mill"] {
        let p = open_folder(&root().join("themes/first-party").join(name), &lumen()).unwrap_or_else(|e| panic!("{name}: {e:?}"));
        assert_eq!(p.problem(false), None, "{name}");
        all.push(p);
    }
    // Paper Mill is the layout theme: its theme.css is checked on open and written back scoped.
    let mill = all.iter().find(|p| p.meta.id == "jl.paper-mill").unwrap();
    assert!(mill.layout_css("http://check.invalid").unwrap().contains("data-part=\"sidebar\""));
    // For the listing screenshots: `LUMEN_DUMP_THEME_CSS=<folder>` writes each theme's CSS there.
    if let Ok(dir) = std::env::var("LUMEN_DUMP_THEME_CSS") {
        std::fs::create_dir_all(&dir).unwrap();
        for p in &all {
            std::fs::write(PathBuf::from(&dir).join(format!("{}.css", p.meta.id)), css(std::slice::from_ref(p), "http://dump.invalid")).unwrap();
        }
    }
}

#[test]
fn film_lab_presets_read_and_stay_in_range() {
    let dir = root().join("plugins/first-party/film-lab/presets");
    let mut n = 0;
    let mut names = std::collections::HashSet::new();
    for e in std::fs::read_dir(&dir).unwrap() {
        let path = e.unwrap().path();
        let text = std::fs::read_to_string(&path).unwrap();
        let f = parse_plugin_preset(&text).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        assert!(names.insert(f.name.clone()), "duplicate name {}", f.name);
        // Reading brings values inside their ranges; the file must already be there.
        let raw: serde_json::Value = serde_json::from_str(&text).unwrap();
        let read = serde_json::to_value(&f.develop).unwrap();
        assert_subset(&raw["develop"], &read, &f.name);
        assert!(f.look.is_some(), "{} has a look", f.name);
        n += 1;
    }
    assert!((28..=32).contains(&n), "{n} presets");
}

/// Every number the file sets comes back unchanged from the normalised recipe.
fn assert_subset(want: &serde_json::Value, have: &serde_json::Value, what: &str) {
    use serde_json::Value::*;
    match (want, have) {
        (Object(w), Object(h)) => w.iter().for_each(|(k, v)| assert_subset(v, h.get(k).unwrap_or_else(|| panic!("{what}: {k} lost")), what)),
        (Array(w), Array(h)) => {
            assert_eq!(w.len(), h.len(), "{what}");
            w.iter().zip(h).for_each(|(a, b)| assert_subset(a, b, what));
        }
        (Number(w), Number(h)) => assert!((w.as_f64().unwrap() - h.as_f64().unwrap()).abs() < 1e-4, "{what}: {w} became {h}"),
        (a, b) => assert_eq!(a, b, "{what}"),
    }
}
