//! The Lightroom converter's golden file (Round 11): every fixture in `tests/fixtures/lightroom/`
//! (written by hand from the formats, never copied from anyone's files), its settings as read
//! and what `convert` makes of them. Session 11B's training scripts read the same file.
//!
//! `LUMEN_REGEN_GOLDEN=1 cargo test -p photos-core --test lightroom` rewrites it; otherwise
//! the test fails when the conversion changes, so a change to the mapping is a visible diff.

use std::collections::BTreeMap;
use std::path::Path;

use photos_core::lightroom::{convert, read_settings, Converted, Settings};
use serde::{Deserialize, Serialize};

#[derive(Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Case {
    as_shot_kelvin: Option<f32>,
    settings: Settings,
    converted: Converted,
}

/// The photo's own white balance, for the fixtures that set one in Kelvin.
fn as_shot(file: &str) -> Option<f32> {
    (file == "sidecar-classic-13.xmp").then_some(5250.0)
}

#[test]
fn every_fixture_converts_as_the_golden_file_says() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests");
    let mut cases: BTreeMap<String, Case> = BTreeMap::new();
    for entry in std::fs::read_dir(dir.join("fixtures/lightroom")).unwrap() {
        let path = entry.unwrap().path();
        let file = path.file_name().unwrap().to_string_lossy().into_owned();
        let text = std::fs::read_to_string(&path).unwrap();
        let settings = read_settings(&file, &text).unwrap_or_else(|e| panic!("{file}: {e}"));
        let kelvin = as_shot(&file);
        cases.insert(file, Case { as_shot_kelvin: kelvin, converted: convert(&settings, kelvin), settings });
    }
    assert_eq!(cases.len(), 6);

    let golden = dir.join("lightroom-golden.json");
    let written = serde_json::to_string_pretty(&cases).unwrap() + "\n";
    if std::env::var_os("LUMEN_REGEN_GOLDEN").is_some() || !golden.exists() {
        std::fs::write(&golden, &written).unwrap();
        return;
    }
    let stored: BTreeMap<String, Case> = serde_json::from_str(&std::fs::read_to_string(&golden).unwrap()).unwrap();
    for (file, case) in &cases {
        assert_eq!(stored.get(file), Some(case), "{file} differs from lightroom-golden.json (LUMEN_REGEN_GOLDEN=1 rewrites it)");
    }
    assert_eq!(stored.len(), cases.len(), "lightroom-golden.json lists a fixture that is gone");
}

#[test]
fn the_fixtures_say_what_the_spec_says() {
    let read = |f: &str| {
        let text = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/lightroom").join(f)).unwrap();
        read_settings(f, &text).unwrap()
    };
    let only = convert(&read("preset-vibrance-only.xmp"), None);
    assert_eq!(only.panels, vec![photos_core::develop::Panel::Basic]);
    assert_eq!(only.name.as_deref(), Some("Vibrance +25"));

    let warm = convert(&read("preset-warm-portrait.xmp"), None);
    assert_eq!((warm.name.as_deref(), warm.group.as_deref()), (Some("Warm Portrait"), Some("Portrait")));
    assert!(warm.skipped.is_empty(), "{:?}", warm.skipped);

    let sidecar = convert(&read("sidecar-classic-13.xmp"), Some(5250.0));
    assert_eq!(sidecar.develop.masks.len(), 2, "the gradient and the radial; the brush is skipped");
    for s in ["Brush mask", "LensProfileEnable", "AutoLateralCA", "Temperature (approximated)"] {
        assert!(sidecar.skipped.contains(&s.to_string()), "{s} in {:?}", sidecar.skipped);
    }
    assert!(sidecar.geometry.is_some_and(|g| g.crop.is_some() && g.straighten == -1.35));

    let catalog = convert(&read("catalog-develop.lua"), None);
    assert_eq!(catalog.develop.masks.len(), 1);
    assert!(catalog.skipped.contains(&"AI mask (subject, sky…)".to_string()), "{:?}", catalog.skipped);
    assert!(catalog.geometry.is_none());

    let old = convert(&read("old-pv2010.lrtemplate"), None);
    assert!(old.skipped.contains(&"Process version 2010 (approximated)".to_string()));
    assert_eq!(old.develop.basic.saturation, -1.0, "black and white");
}
