//! Round 25 P3 step 5: the first-party manifests use API 1.2's `[[ui.section]]` and keep the
//! setting keys their code and providers read.

use std::path::Path;

use photos_plugins::manifest::{self, ControlKind, Manifest};
use photos_plugins::ApiVersion;

fn load(name: &str) -> Manifest {
    let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../plugins/first-party").join(name).join("plugin.toml");
    manifest::parse(&std::fs::read_to_string(&p).unwrap()).unwrap_or_else(|e| panic!("{name}: {e:?}"))
}

fn keys(m: &Manifest) -> Vec<&str> {
    m.settings.iter().map(|s| s.key.as_str()).collect()
}

#[test]
fn disk_atlas_declares_its_page_as_a_ui_section() {
    let m = load("disk-atlas");
    assert_eq!(m.api, ApiVersion { major: 1, minor: 2 });
    assert_eq!(keys(&m), ["use_picker", "use_reveal"]);
    assert_eq!(m.sections.len(), 1);
    let s = &m.sections[0];
    assert!(s.title.is_some());
    assert!(s.controls.iter().all(|c| c.kind == ControlKind::Toggle && c.app.as_deref() == Some("disk-atlas") && c.help.is_some()));
    // The providers still find their switches.
    assert!(m.settings.iter().all(|s| s.default == serde_json::json!(true)));
}

#[test]
fn wayfinder_declares_its_page_as_ui_sections() {
    let m = load("wayfinder");
    assert_eq!(m.api, ApiVersion { major: 1, minor: 2 });
    assert_eq!(keys(&m), ["write_originals", "include_estimated"]);
    assert!(m.sections.iter().all(|s| s.title.is_some()));
    let controls: Vec<_> = m.sections.iter().flat_map(|s| &s.controls).collect();
    assert_eq!(controls.len(), 2);
    assert!(controls.iter().all(|c| c.kind == ControlKind::Toggle && c.help.is_some()));
    assert_eq!(m.settings[1].default, serde_json::json!(false));
    // Lumen has no Wayfinder code: its entry points are contributions.
    let c: Vec<_> = m.contributions.iter().map(|c| (c.label.as_str(), c.action.as_str())).collect();
    assert_eq!(c, [("Find locations", "tool:find_locations"), ("Review guessed places", "open-review:places")]);
}
