//! Clockwork: finds a camera whose clock was off and proposes corrected dates for its photos.
//!
//! `check_clocks` reads the library (never a photo's bytes), picks the cameras it can trust (a
//! phone records its own UTC offset, a camera with GPS fixes its time from it), and for every
//! other camera looks, trip by trip, for a single shift that lines its photos up with the trusted
//! ones. Where one is clear it proposes the shifted date for each photo of that trip. Nothing is
//! written until the person accepts on the review page, and then only Lumen's own record of the
//! capture time, never the file. Only the pure helpers build natively (`cargo test`).

pub mod logic;

#[cfg(target_arch = "wasm32")]
mod plugin {
    use std::collections::{BTreeMap, HashSet};

    use lumen_plugin::proposals::{propose, submit_all};
    use lumen_plugin::{library, log, setting, Asset, AssetFilter, GpsSource, Proposed, Tool, Tools};
    use serde_json::json;

    use crate::logic::{self, Peak};

    const REVIEW: &str = "/review/date?plugin=jl.clockwork";

    /// What the tool needs of one photo.
    struct Shot {
        id: u64,
        secs: i64,
        wall: String,
    }

    struct Camera {
        total: usize,
        with_offset: usize,
        with_gps: usize,
        shots: Vec<Shot>,
    }

    fn filter(selection: bool) -> AssetFilter {
        AssetFilter { has_gps: None, since: None, until: None, album: None, selection }
    }

    /// Every dated photo with a camera, grouped by camera; photos whose date isn't a shutter's
    /// (the file's own date, an edit time) are left out of the clock maths.
    fn read_cameras() -> Result<BTreeMap<String, Camera>, String> {
        let mut cams: BTreeMap<String, Camera> = BTreeMap::new();
        let mut failure: Option<String> = None;
        library::each_page(&filter(false), |page| {
            let page: Vec<Asset> = page.into_iter().filter(|a| a.camera.is_some() && a.captured_at.is_some()).collect();
            let ids: Vec<u64> = page.iter().map(|a| a.id).collect();
            let sources: BTreeMap<u64, Option<String>> = match library::details(&ids) {
                Ok(d) => d.into_iter().map(|d| (d.id, d.capture_source)).collect(),
                Err(e) => {
                    failure = Some(e);
                    return false;
                }
            };
            for a in page {
                let (Some(name), Some(wall)) = (a.camera.clone(), a.captured_at.clone()) else { continue };
                if !logic::is_shutter_date(sources.get(&a.id).and_then(|s| s.as_deref())) {
                    continue;
                }
                let Some(secs) = logic::parse_wall(&wall) else { continue };
                let c = cams.entry(name).or_insert(Camera { total: 0, with_offset: 0, with_gps: 0, shots: Vec::new() });
                c.total += 1;
                c.with_offset += a.offset_minutes.is_some() as usize;
                c.with_gps += matches!(a.gps.as_ref().map(|g| g.source), Some(GpsSource::Camera)) as usize;
                c.shots.push(Shot { id: a.id, secs, wall });
            }
            true
        })?;
        match failure {
            Some(e) => Err(e),
            None => Ok(cams),
        }
    }

    fn selection_ids() -> Result<HashSet<u64>, String> {
        let mut ids = HashSet::new();
        library::each_page(&filter(true), |page| {
            ids.extend(page.iter().map(|a| a.id));
            true
        })?;
        Ok(ids)
    }

    /// The trusted camera with the most photos near the shifted moments of `secs`.
    fn who_matches(secs: &[i64], trusted: &BTreeMap<String, Vec<i64>>, shift: i64) -> String {
        let mut best: (usize, &str) = (0, "");
        for (name, mins) in trusted {
            let n = secs
                .iter()
                .filter(|s| {
                    let want = s.div_euclid(60) - shift;
                    let i = mins.partition_point(|&r| r < want);
                    [i.checked_sub(1), Some(i)].into_iter().flatten().filter_map(|j| mins.get(j)).any(|&r| (r - want).abs() <= logic::TOLERANCE)
                })
                .count();
            if n > best.0 {
                best = (n, name);
            }
        }
        best.1.to_string()
    }

    fn plural(n: usize) -> &'static str {
        if n == 1 {
            ""
        } else {
            "s"
        }
    }

    fn reason(camera: &str, p: &Peak, against: &str, dir: &str) -> String {
        let text = format!("{camera}'s clock looks {} {dir} {against} on these days: {} of its {} moments line up once shifted.", logic::duration_words(p.shift), p.hits, p.of);
        text.chars().take(200).collect()
    }

    fn check(args_json: &str) -> Result<String, String> {
        let args: serde_json::Value = if args_json.trim().is_empty() { json!({}) } else { serde_json::from_str(args_json).map_err(|e| format!("the arguments aren't JSON: {e}"))? };
        let scope = args.get("scope").and_then(|s| s.as_str()).unwrap_or("library");
        if !matches!(scope, "library" | "selection") {
            return Err(format!("scope is library or selection, not {scope}"));
        }
        let min_confidence = setting::<f64>("min_confidence").unwrap_or(0.7).clamp(0.3, 0.95) as f32;

        let cams = read_cameras()?;
        let only = if scope == "selection" { Some(selection_ids()?) } else { None };

        let mut trusted: BTreeMap<String, Vec<i64>> = BTreeMap::new();
        let mut trusted_all: Vec<i64> = Vec::new();
        for (name, c) in &cams {
            if logic::is_trusted(c.total, c.with_offset, c.with_gps) {
                let mut secs: Vec<i64> = c.shots.iter().map(|s| s.secs).collect();
                secs.sort_unstable();
                let mins = logic::minutes(&secs);
                trusted_all.extend(&mins);
                trusted.insert(name.clone(), mins);
            }
        }
        trusted_all.sort_unstable();
        trusted_all.dedup();
        if trusted.is_empty() {
            return Ok(json!({ "status": "done", "suggested": 0, "cameras": [], "message":
                "Clockwork compares a camera with one whose clock it can trust, such as a phone, taken to the same places. None of the cameras in your library qualifies yet." })
            .to_string());
        }

        let mut proposals = Vec::new();
        let mut report = Vec::new();
        let mut checked = 0usize;
        for (name, cam) in &cams {
            if trusted.contains_key(name) {
                continue;
            }
            checked += 1;
            let mut shots: Vec<&Shot> = cam.shots.iter().collect();
            shots.sort_by_key(|s| s.secs);
            let secs: Vec<i64> = shots.iter().map(|s| s.secs).collect();
            for run in logic::episodes(&secs) {
                let ep = &shots[run.clone()];
                let ep_secs = &secs[run];
                let Some(peak) = logic::search(&logic::minutes(ep_secs), &trusted_all) else { continue };
                let conf = logic::confidence(&peak);
                if conf < min_confidence {
                    continue;
                }
                let against = who_matches(ep_secs, &trusted, peak.shift);
                let dir = if peak.shift > 0 { "ahead of" } else { "behind" };
                let why = reason(name, &peak, &against, dir);
                let mut n = 0;
                for s in ep.iter().filter(|s| only.as_ref().map_or(true, |o| o.contains(&s.id))) {
                    let Some(date) = logic::shifted(&s.wall, peak.shift * 60) else { continue };
                    proposals.push(propose(s.id, Proposed::Date(date), conf, Some(&why)));
                    n += 1;
                }
                if n > 0 {
                    report.push(json!({
                        "camera": name,
                        "photos": n,
                        "from": logic::format_wall(ep_secs[0]),
                        "to": logic::format_wall(ep_secs[ep_secs.len() - 1]),
                        "clock": format!("{} {dir} {against}", logic::duration_words(peak.shift)),
                        "confidence": ((conf * 100.0).round() / 100.0),
                    }));
                }
            }
        }
        let suggested = proposals.len();
        submit_all(proposals)?;
        log(&format!("{checked} camera{} checked against {} trusted, {suggested} dates proposed", plural(checked), trusted.len()));
        let message = if suggested == 0 {
            format!("Checked {checked} camera{} against {} trusted one{} and found no clock that is clearly off.", plural(checked), trusted.len(), plural(trusted.len()))
        } else {
            format!("{suggested} corrected date{} to review.", plural(suggested))
        };
        Ok(json!({ "status": "done", "suggested": suggested, "review": REVIEW, "cameras": report, "message": message }).to_string())
    }

    struct Clockwork;

    impl Tools for Clockwork {
        fn tools() -> Vec<Tool> {
            let schema = json!({
                "type": "object",
                "properties": {
                    "scope": { "type": "string", "enum": ["library", "selection"], "description": "Which photos may get a corrected date (default library). Every photo is still used as evidence." }
                }
            });
            vec![Tool { name: "check_clocks".into(), input_schema: schema.to_string() }]
        }

        fn call(name: String, args_json: String) -> Result<String, String> {
            if name != "check_clocks" {
                return Err(format!("no tool {name}"));
            }
            check(&args_json)
        }
    }

    lumen_plugin::export_proposals!(Clockwork);
}
