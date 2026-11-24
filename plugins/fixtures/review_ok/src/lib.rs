//! The API 1.1 fixture (Round 24 D1): one tool per new interface and a `review` export.
//! - `library` returns the ids of the first page of `library::assets` joined by `,`;
//! - `suggest` sends one guess for asset 7 through `places::suggest` and returns `sent`;
//! - `lookup` returns the first hit's name of `places::lookup("Bled")`;
//! - `run` returns the first output's data of `models::run("m", [x = [1, 2]])`;
//! - `nearest` returns the rows of `models::nearest(storage("own"), [1, 0], 2)`.
//! `on-review` answers `confirm(ids)` with one anchor per id and anything else with nothing.

wit_bindgen::generate!({ world: "places-plugin", path: "../../../src-tauri/crates/photos-plugins/wit" });

use exports::lumen::plugin::review;
use exports::lumen::plugin::tools::Guest;
use lumen::plugin::types::{AssetFilter, Candidate, PlaceState, ReviewEvent, Suggestion, Tensor, Tool, VectorSource};
use lumen::plugin::{library, models, places};

struct Fixture;

fn guess(asset: u64, state: PlaceState) -> Suggestion {
    Suggestion {
        asset,
        candidates: vec![Candidate { lat: 46.36, lon: 14.09, confidence: 0.5, label: Some("Bled".into()) }],
        state,
        group: "g1".into(),
        group_label: "Bled".into(),
        flag: None,
    }
}

impl Guest for Fixture {
    fn tools() -> Vec<Tool> {
        ["library", "suggest", "lookup", "run", "nearest"]
            .into_iter()
            .map(|n| Tool { name: n.into(), input_schema: "{\"type\":\"object\"}".into() })
            .collect()
    }

    fn call(name: String, _args_json: String) -> Result<String, String> {
        match name.as_str() {
            "library" => {
                let filter = AssetFilter { has_gps: Some(false), since: None, until: None, album: None, selection: false };
                let page = library::assets(&filter, None)?;
                Ok(page.items.iter().map(|a| a.id.to_string()).collect::<Vec<_>>().join(","))
            }
            "suggest" => {
                places::suggest(&[guess(7, PlaceState::Guess)])?;
                Ok("sent".into())
            }
            "lookup" => Ok(places::lookup("Bled").first().map(|h| h.name.clone()).unwrap_or_default()),
            "run" => {
                let out = models::run("m", &[Tensor { name: "x".into(), shape: vec![2], data: vec![1.0, 2.0] }])?;
                Ok(format!("{:?}", out.first().map(|t| t.data.clone()).unwrap_or_default()))
            }
            "nearest" => {
                let rows = models::nearest(&VectorSource::Storage("own".into()), &[1.0, 0.0], 2)?;
                Ok(rows.iter().map(|r| r.row.to_string()).collect::<Vec<_>>().join(","))
            }
            other => Err(format!("no tool {other}")),
        }
    }
}

impl review::Guest for Fixture {
    fn on_review(event: ReviewEvent) -> Vec<Suggestion> {
        match event {
            ReviewEvent::Confirm(ids) => ids.into_iter().map(|id| guess(id, PlaceState::Anchor)).collect(),
            _ => Vec::new(),
        }
    }
}

export!(Fixture);
