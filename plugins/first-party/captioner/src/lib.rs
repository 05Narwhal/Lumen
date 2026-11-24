//! Captioner: proposes titles and captions from the people, places, scene words and text Lumen
//! already has for each photo (no model, nothing leaves the computer).
//!
//! `suggest_captions` is one tool that may take several calls: a call has 30 s, so each works for
//! about [`BUDGET_SECS`] and answers `{"status":"working",…}` until every page of the library has
//! been read. Where it stopped is kept in the plugin's storage. Nothing is written until the
//! person accepts on the review pages. Only the wording builds natively (`cargo test`).

pub mod logic;

/// What a call may spend before it answers `working` (the host stops a call at 30 s).
pub const BUDGET_SECS: u64 = 15;

#[cfg(target_arch = "wasm32")]
mod plugin {
    use std::time::{Duration, Instant};

    use lumen_plugin::proposals::{propose, submit_all};
    use lumen_plugin::{library, log, setting, storage, Asset, AssetFilter, GpsSource, Proposal, Proposed, Tool, Tools};
    use serde_json::json;

    use crate::logic::{self, Facts, Options};
    use crate::BUDGET_SECS;

    const PROGRESS: &str = "progress.json";
    const REVIEW: &str = "/review/title?plugin=jl.captioner";

    fn options() -> Options {
        let on = |key: &str, default: bool| setting::<bool>(key).unwrap_or(default);
        Options { titles: on("titles", true), captions: on("captions", true), text: on("include_text", true), album: on("include_album", false) }
    }

    fn min_confidence() -> f32 {
        setting::<f64>("min_confidence").unwrap_or(0.5).clamp(0.3, 0.95) as f32
    }

    /// Where a run stopped: the key of what it was asked, the next page's cursor and the totals.
    #[derive(Default)]
    struct Progress {
        key: String,
        cursor: Option<String>,
        read: u64,
        proposed: u64,
    }

    fn load() -> Option<Progress> {
        let bytes = storage::read(PROGRESS).ok().flatten()?;
        let v: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
        Some(Progress {
            key: v.get("key")?.as_str()?.to_string(),
            cursor: v.get("cursor").and_then(|c| c.as_str()).map(str::to_string),
            read: v.get("read")?.as_u64()?,
            proposed: v.get("proposed")?.as_u64()?,
        })
    }

    fn save(p: &Progress) -> Result<(), String> {
        let v = json!({ "key": p.key, "cursor": p.cursor, "read": p.read, "proposed": p.proposed });
        storage::write(PROGRESS, v.to_string().as_bytes())
    }

    fn facts(a: &Asset, d: &lumen_plugin::AssetDetails) -> Facts {
        Facts {
            wall: a.captured_at.clone(),
            place: a.place.clone(),
            estimated_place: matches!(a.gps.as_ref().map(|g| g.source), Some(GpsSource::Estimated)),
            people: d.people.clone(),
            scene: d.scene.clone(),
            text: d.text.clone(),
            albums: d.albums.clone(),
            has_title: d.title.is_some(),
            has_caption: d.caption.is_some(),
        }
    }

    fn suggest(args_json: &str) -> Result<String, String> {
        let args: serde_json::Value = if args_json.trim().is_empty() { json!({}) } else { serde_json::from_str(args_json).map_err(|e| format!("the arguments aren't JSON: {e}"))? };
        let scope = args.get("scope").and_then(|s| s.as_str()).unwrap_or("library");
        let album = args.get("album").and_then(|a| a.as_i64());
        let filter = match (scope, album) {
            ("library", _) => AssetFilter { has_gps: None, since: None, until: None, album: None, selection: false },
            ("selection", _) => AssetFilter { has_gps: None, since: None, until: None, album: None, selection: true },
            ("album", Some(id)) => AssetFilter { has_gps: None, since: None, until: None, album: Some(id), selection: false },
            ("album", None) => return Err("scope album needs an album id".into()),
            (other, _) => return Err(format!("scope is selection, album or library, not {other}")),
        };
        let (opts, floor) = (options(), min_confidence());
        if !opts.titles && !opts.captions {
            return Ok(json!({ "status": "done", "suggested": 0, "message": "Titles and captions are both turned off in Captioner's settings." }).to_string());
        }
        // The key says what a stored cursor belongs to: another scope or other settings start over.
        let key = format!("{scope}|{}|{}{}{}|{floor}", album.unwrap_or(0), opts.titles as u8, opts.captions as u8, opts.text as u8 * 2 + opts.album as u8);
        let mut progress = load().filter(|p| p.key == key && !args.get("restart").and_then(|r| r.as_bool()).unwrap_or(false)).unwrap_or(Progress { key, ..Default::default() });
        let started = Instant::now();

        loop {
            let page = library::assets(&filter, progress.cursor.as_deref())?;
            let ids: Vec<u64> = page.items.iter().map(|a| a.id).collect();
            let details = library::details(&ids)?;
            let mut batch: Vec<Proposal> = Vec::new();
            for a in &page.items {
                let Some(d) = details.iter().find(|d| d.id == a.id) else { continue };
                let Some(draft) = logic::compose(&facts(a, d), &opts) else { continue };
                if draft.confidence < floor {
                    continue;
                }
                if let Some(t) = &draft.title {
                    batch.push(propose(a.id, Proposed::Title(t.clone()), draft.confidence, Some(&draft.reason)));
                }
                if let Some(c) = &draft.caption {
                    batch.push(propose(a.id, Proposed::Caption(c.clone()), draft.confidence, Some(&draft.reason)));
                }
            }
            progress.read += page.items.len() as u64;
            progress.proposed += batch.len() as u64;
            submit_all(batch)?;
            progress.cursor = page.next_cursor;
            if progress.cursor.is_none() {
                break;
            }
            if started.elapsed() > Duration::from_secs(BUDGET_SECS) {
                save(&progress)?;
                return Ok(json!({ "status": "working", "read": progress.read, "proposed": progress.proposed }).to_string());
            }
        }
        let _ = storage::remove(PROGRESS);
        log(&format!("{} photos read, {} proposals", progress.read, progress.proposed));
        let message = if progress.proposed == 0 {
            "Nothing to propose: the photos here have no people, place or scene to build a title from, or already have their titles.".to_string()
        } else {
            format!("{} proposals to review.", progress.proposed)
        };
        Ok(json!({ "status": "done", "suggested": progress.proposed, "review": REVIEW, "message": message }).to_string())
    }

    struct Captioner;

    impl Tools for Captioner {
        fn tools() -> Vec<Tool> {
            let schema = json!({
                "type": "object",
                "properties": {
                    "scope": { "type": "string", "enum": ["selection", "album", "library"], "description": "Which photos (default library). Photos that already have a title or caption keep it." },
                    "album": { "type": "integer", "description": "The album's id, for scope album." },
                    "restart": { "type": "boolean", "description": "Start again from the first photo instead of where the last run stopped." }
                }
            });
            vec![Tool { name: "suggest_captions".into(), input_schema: schema.to_string() }]
        }

        fn call(name: String, args_json: String) -> Result<String, String> {
            if name != "suggest_captions" {
                return Err(format!("no tool {name}"));
            }
            suggest(&args_json)
        }
    }

    lumen_plugin::export_proposals!(Captioner);
}
