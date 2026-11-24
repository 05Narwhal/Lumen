//! The API 1.3 fixture (Round 25 P4b): a tools plugin that submits proposals.
//! - `call("all")` submits one proposal of every kind for asset 1 (face 7 for the person);
//! - `call("title")` submits only a title for asset 1;
//! - `call("bad")` submits a label that isn't a colour;
//! each answers `ok` or the host's error.

wit_bindgen::generate!({ world: "proposals-plugin", path: "../../../src-tauri/crates/photos-plugins/wit" });

use exports::lumen::plugin::tools::Guest;
use lumen::plugin::proposals;
use lumen::plugin::types::{FaceName, LatLon, Proposal, Proposed, Tool};

struct Fixture;

fn one(value: Proposed) -> Proposal {
    Proposal { asset: 1, value, confidence: 0.75, reason: Some("because".into()) }
}

impl Guest for Fixture {
    fn tools() -> Vec<Tool> {
        Vec::new()
    }
    fn call(name: String, _args_json: String) -> Result<String, String> {
        let batch = match name.as_str() {
            "all" => vec![
                one(Proposed::Location(LatLon { lat: 46.0, lon: 14.5 })),
                one(Proposed::Date("2021-05-04T10:00:00".into())),
                one(Proposed::Tags(vec!["beach".into(), " sea ".into()])),
                one(Proposed::Label("green".into())),
                one(Proposed::Person(FaceName { face: 7, name: "Ana".into() })),
                one(Proposed::Title("Sunset".into())),
                one(Proposed::Caption("A long day".into())),
            ],
            "title" => vec![one(Proposed::Title("Sunset".into()))],
            "bad" => vec![one(Proposed::Title("Fine".into())), one(Proposed::Label("Orange".into()))],
            other => return Err(format!("no tool {other}")),
        };
        proposals::submit(&batch).map(|()| "ok".into())
    }
}

export!(Fixture);
