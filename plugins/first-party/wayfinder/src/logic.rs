//! Wayfinder's decisions, with no WIT types so `cargo test` runs natively (spec §10.2).
//!
//! Three steps feed the Place review:
//! 1. [`rank`] turns what the model found (the gallery, the extra gallery points near the person's
//!    own places, the nearest of the person's own photos) into up to five candidates;
//! 2. [`Session::new`] adds the signals around a photo (camera anchors, time interpolation, bursts,
//!    moments, folder names) and decides state, group and flag;
//! 3. [`Session::apply`] answers each thing the person does and returns only what changed.
//!
//! Every threshold is a named constant; the values are W2's measured ones (plan, W2 *As built*).

use std::collections::{BTreeMap, HashMap, HashSet};

use serde::{Deserialize, Serialize};

/// A photo whose nearest own photo is at least this similar takes that photo's place (W2: the
/// curve is flat from 0.55 to 0.70; the spec's first guess, 0.85, was far too high).
pub const PERSONAL_THRESHOLD: f32 = 0.65;
/// GeoCLIP's softmax temperature (its logit scale, exp = 39.69).
pub const LOGIT_SCALE: f64 = 39.69;
/// Library prior: `p *= 1 + PRIOR_WEIGHT * exp(-d / PRIOR_SCALE_KM)`, d = km to the nearest place
/// the person has been (W2: the plateau starts at 300).
pub const PRIOR_WEIGHT: f64 = 300.0;
pub const PRIOR_SCALE_KM: f64 = 25.0;
/// A folder called like a place weighs a moment's guesses near it (a prior, never an anchor).
pub const FOLDER_WEIGHT: f64 = 3.0;
pub const FOLDER_SCALE_KM: f64 = 100.0;
/// Candidates closer than this are one candidate.
pub const MERGE_KM: f64 = 10.0;
pub const MAX_CANDIDATES: usize = 5;
/// The person's own places: greedy clusters of this radius holding at least this many photos.
pub const CLUSTER_KM: f64 = 10.0;
pub const CLUSTER_MIN_PHOTOS: usize = 3;
/// Extra gallery points: a grid within this radius of each cluster (W2: 1 km; Lumen's plugin
/// storage is 50 MB, so [`dense_step_km`] widens the grid until the points fit [`MAX_DENSE_POINTS`]).
pub const DENSE_RADIUS_KM: f64 = 25.0;
pub const MAX_DENSE_POINTS: usize = 16_000;
/// Moments: photos at most this far apart in time stay together, unless their places jump.
pub const MOMENT_GAP_SECS: i64 = 30 * 60;
pub const MOMENT_JUMP_KM: f64 = 50.0;
/// Time interpolation between two anchors at most this far apart in time.
pub const INTERP_WINDOW_SECS: i64 = 2 * 60 * 60;
pub const MAX_SPEED_KMH: f64 = 250.0;
/// One anchor this close in time gives its place.
pub const SINGLE_ANCHOR_SECS: i64 = 15 * 60;
/// Flags: a member this far from its moment's place; a guess under this confidence.
pub const FAR_FROM_GROUP_KM: f64 = 25.0;
pub const LOW_CONFIDENCE: f32 = 0.2;
const EARTH_KM: f64 = 6371.0088;
const KM_PER_DEGREE: f64 = 111.195;

// ------------------------------------------------------------------ geometry and time

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Point {
    pub lat: f64,
    pub lon: f64,
}

impl Point {
    pub fn new(lat: f64, lon: f64) -> Point {
        Point { lat, lon }
    }
}

/// Great-circle distance.
pub fn km(a: Point, b: Point) -> f64 {
    let (p1, p2) = (a.lat.to_radians(), b.lat.to_radians());
    let dl = (b.lon - a.lon).to_radians();
    let h = ((p2 - p1) / 2.0).sin().powi(2) + p1.cos() * p2.cos() * (dl / 2.0).sin().powi(2);
    2.0 * EARTH_KM * h.sqrt().min(1.0).asin()
}

fn wrap_lon(d: f64) -> f64 {
    (d + 180.0).rem_euclid(360.0) - 180.0
}

fn lerp(a: Point, b: Point, f: f64) -> Point {
    Point { lat: a.lat + (b.lat - a.lat) * f, lon: wrap_lon(a.lon + wrap_lon(b.lon - a.lon) * f) }
}

fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * ((m + 9) % 12) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

/// `YYYY-MM-DDTHH:MM:SS` (the library's wall-clock time) as seconds, so gaps can be taken.
pub fn parse_time(s: &str) -> Option<i64> {
    let n = |i: usize, len: usize| -> Option<i64> { s.get(i..i + len)?.parse().ok() };
    let (y, mo, d, h, mi, se) = (n(0, 4)?, n(5, 2)?, n(8, 2)?, n(11, 2)?, n(14, 2)?, n(17, 2)?);
    if !(1..=12).contains(&mo) || !(1..=31).contains(&d) || h > 23 || mi > 59 || se > 60 {
        return None;
    }
    Some(days_from_civil(y, mo, d) * 86_400 + h * 3600 + mi * 60 + se)
}

fn clock(t: i64) -> String {
    let s = t.rem_euclid(86_400);
    format!("{:02}:{:02}", s / 3600, s % 3600 / 60)
}

fn day(t: i64) -> String {
    const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    let (y, m, d) = civil_from_days(t.div_euclid(86_400));
    format!("{d} {} {y}", MONTHS[(m - 1) as usize])
}

/// `12 Mar 2024 14:02–14:40` (or `…14:02` for one instant, both days when it crosses midnight).
pub fn time_range(a: i64, b: i64) -> String {
    if a.div_euclid(86_400) != b.div_euclid(86_400) {
        format!("{} {}–{} {}", day(a), clock(a), day(b), clock(b))
    } else if clock(a) == clock(b) {
        format!("{} {}", day(a), clock(a))
    } else {
        format!("{} {}–{}", day(a), clock(a), clock(b))
    }
}

// ------------------------------------------------------------------ the person's own places

/// Greedy clusters of photos taken within [`CLUSTER_KM`] of a cluster's centre, keeping those with
/// at least [`CLUSTER_MIN_PHOTOS`]; the centre is the mean of its photos. Biggest first.
pub fn clusters(points: &[Point]) -> Vec<Point> {
    struct C {
        sum_lat: f64,
        sum_lon: f64,
        n: usize,
        centre: Point,
    }
    let mut cs: Vec<C> = Vec::new();
    for &p in points {
        match cs.iter_mut().find(|c| km(c.centre, p) <= CLUSTER_KM) {
            Some(c) => {
                c.n += 1;
                c.sum_lat += p.lat;
                // lon relative to the first photo so the antimeridian doesn't average to zero
                c.sum_lon += wrap_lon(p.lon - c.centre.lon) + c.centre.lon;
                c.centre = Point { lat: c.sum_lat / c.n as f64, lon: wrap_lon(c.sum_lon / c.n as f64) };
            }
            None => cs.push(C { sum_lat: p.lat, sum_lon: p.lon, n: 1, centre: p }),
        }
    }
    cs.retain(|c| c.n >= CLUSTER_MIN_PHOTOS);
    cs.sort_by_key(|c| std::cmp::Reverse(c.n));
    cs.into_iter().map(|c| c.centre).collect()
}

/// A grid of `step_km` within [`DENSE_RADIUS_KM`] of one centre.
fn grid_around(c: Point, step_km: f64) -> Vec<Point> {
    let per_lon = KM_PER_DEGREE * c.lat.to_radians().cos().max(0.05);
    let n = (DENSE_RADIUS_KM / step_km).floor() as i64;
    let mut out = Vec::new();
    for iy in -n..=n {
        for ix in -n..=n {
            let (x, y) = (ix as f64 * step_km, iy as f64 * step_km);
            if x * x + y * y <= DENSE_RADIUS_KM * DENSE_RADIUS_KM {
                let lat = c.lat + y / KM_PER_DEGREE;
                if lat.abs() <= 90.0 {
                    out.push(Point { lat, lon: wrap_lon(c.lon + x / per_lon) });
                }
            }
        }
    }
    out
}

/// The extra gallery points: a `step_km` grid within 25 km of each centre.
pub fn dense_points(centres: &[Point], step_km: f64) -> Vec<Point> {
    centres.iter().flat_map(|&c| grid_around(c, step_km)).collect()
}

/// The finest grid (1 km, then 2, 3, …) whose points for these centres fit [`MAX_DENSE_POINTS`].
pub fn dense_step_km(centres: &[Point]) -> f64 {
    let per_centre = |step: f64| (std::f64::consts::PI * DENSE_RADIUS_KM * DENSE_RADIUS_KM / (step * step)).ceil() as usize;
    (1..=25).map(f64::from).find(|&s| per_centre(s) * centres.len() <= MAX_DENSE_POINTS).unwrap_or(25.0)
}

// ------------------------------------------------------------------ the model's candidates

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Cand {
    pub at: Point,
    pub conf: f32,
    pub label: Option<String>,
}

/// One row the host found for the photo's embedding: where it is and its dot product with it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hit {
    pub at: Point,
    pub score: f32,
}

fn personal_confidence(similarity: f32) -> f32 {
    (0.4 + (similarity - PERSONAL_THRESHOLD) * 1.5).clamp(0.4, 0.95)
}

/// Up to five distinct candidates, best first, for one photo:
/// - the nearest of the person's own photos if it is at least [`PERSONAL_THRESHOLD`] similar;
/// - GeoCLIP's top rows and the extra rows near the person's places, `softmax(39.69·(s − s_max))`
///   times the library prior (places the person has been), nearby rows merged into one.
pub fn rank(gallery: &[Hit], dense: &[Hit], personal: Option<Hit>, centres: &[Point]) -> Vec<Cand> {
    let mut out: Vec<Cand> = Vec::new();
    if let Some(p) = personal.filter(|p| p.score >= PERSONAL_THRESHOLD) {
        out.push(Cand { at: p.at, conf: personal_confidence(p.score), label: None });
    }
    let hits: Vec<Hit> = gallery.iter().chain(dense).copied().collect();
    let Some(top) = hits.iter().map(|h| h.score).reduce(f32::max) else { return out };
    let mut weighed: Vec<(Point, f64)> = hits
        .iter()
        .map(|h| {
            let mut p = (LOGIT_SCALE * f64::from(h.score - top)).exp();
            if !centres.is_empty() {
                let d = centres.iter().map(|&c| km(c, h.at)).fold(f64::MAX, f64::min);
                p *= 1.0 + PRIOR_WEIGHT * (-d / PRIOR_SCALE_KM).exp();
            }
            (h.at, p)
        })
        .collect();
    let total: f64 = weighed.iter().map(|w| w.1).sum();
    weighed.sort_by(|a, b| b.1.total_cmp(&a.1));
    let mut picked: Vec<(Point, f64)> = Vec::new();
    for (at, p) in weighed {
        if let Some(q) = picked.iter_mut().find(|q| km(q.0, at) <= MERGE_KM) {
            q.1 += p;
        } else if picked.len() < MAX_CANDIDATES {
            picked.push((at, p));
        }
    }
    picked.sort_by(|a, b| b.1.total_cmp(&a.1));
    for (at, p) in picked {
        if !out.iter().any(|c| km(c.at, at) <= MERGE_KM) {
            out.push(Cand { at, conf: (p / total) as f32, label: None });
        }
    }
    out.truncate(MAX_CANDIDATES);
    out
}

/// Weighs candidates near `centre` up (a prior), keeps their total confidence, best first.
fn reweigh(cands: &mut [Cand], centre: Point, weight: f64, scale_km: f64) {
    let before: f32 = cands.iter().map(|c| c.conf).sum();
    for c in cands.iter_mut() {
        c.conf *= (1.0 + weight * (-km(c.at, centre) / scale_km).exp()) as f32;
    }
    let after: f32 = cands.iter().map(|c| c.conf).sum();
    if after > 0.0 {
        for c in cands.iter_mut() {
            c.conf *= before / after;
        }
    }
    cands.sort_by(|a, b| b.conf.total_cmp(&a.conf));
}

// ------------------------------------------------------------------ anchors and time

/// A place that never changes: camera GPS, or a place the person confirmed.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Anchor {
    pub time: i64,
    pub at: Point,
}

/// Where a photo taken at `t` was, from anchors sorted by time: between two anchors at most
/// [`INTERP_WINDOW_SECS`] apart, time-weighted, if getting from one to the other takes no more than
/// [`MAX_SPEED_KMH`]; else the nearest anchor within [`SINGLE_ANCHOR_SECS`]. Returns the place and
/// how sure the inference is.
pub fn infer(anchors: &[Anchor], t: i64) -> Option<(Point, f32)> {
    let i = anchors.partition_point(|a| a.time <= t);
    let before = i.checked_sub(1).map(|k| anchors[k]);
    let after = anchors.get(i).copied();
    if let (Some(b), Some(a)) = (before, after) {
        let dt = a.time - b.time;
        if dt <= INTERP_WINDOW_SECS && km(b.at, a.at) / (dt as f64 / 3600.0) <= MAX_SPEED_KMH {
            let f = (t - b.time) as f64 / dt as f64;
            return Some((lerp(b.at, a.at, f), 0.9));
        }
    }
    [before, after].into_iter().flatten().filter(|a| (a.time - t).abs() <= SINGLE_ANCHOR_SECS).min_by_key(|a| (a.time - t).abs()).map(|a| (a.at, 0.8))
}

// ------------------------------------------------------------------ the review session

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum State {
    Anchor,
    Inferred,
    #[default]
    Guess,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Flag {
    FarFromGroup,
    ImpossibleSpeed,
    LowConfidence,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Answer {
    #[default]
    Pending,
    Confirmed,
    Rejected,
}

/// A place a folder's name points to (`places.lookup`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Prior {
    pub at: Point,
    pub name: String,
}

/// One photo without a location. `model` is what [`rank`] found; the rest is the person's and
/// Wayfinder's decisions, rebuilt from them by [`Session::resolve`].
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Item {
    pub id: u64,
    pub time: Option<i64>,
    pub burst: Option<String>,
    pub folder: Option<String>,
    pub model: Vec<Cand>,
    #[serde(default)]
    pub answer: Answer,
    /// A place the person fixed (*Confirm* keeps the shown place, *Move pin* sets another).
    #[serde(default)]
    pub pinned: Option<Point>,
    /// *Split* starts a new moment at this photo.
    #[serde(default)]
    pub split_start: bool,
    #[serde(skip)]
    pub cands: Vec<Cand>,
    #[serde(skip)]
    pub state: State,
    #[serde(skip)]
    pub group: String,
    #[serde(skip)]
    pub flag: Option<Flag>,
}

impl Item {
    pub fn new(id: u64, time: Option<i64>, burst: Option<String>, folder: Option<String>, model: Vec<Cand>) -> Item {
        Item { id, time, burst, folder, model, answer: Answer::Pending, pinned: None, split_start: false, cands: Vec::new(), state: State::Guess, group: String::new(), flag: None }
    }

    fn top(&self) -> Option<Point> {
        self.cands.first().map(|c| c.at)
    }
}

/// What the Place review shows for one photo (the host's `suggestion`, without WIT types).
#[derive(Clone, Debug, PartialEq)]
pub struct Out {
    pub asset: u64,
    pub cands: Vec<Cand>,
    pub state: State,
    pub group: String,
    pub label: String,
    pub flag: Option<Flag>,
}

/// What the person did on the Place review screen.
#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    Confirm(Vec<u64>),
    Reject(Vec<u64>),
    Move { assets: Vec<u64>, at: Point },
    Split { group: String, at_asset: u64 },
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Session {
    pub anchors: Vec<Anchor>,
    pub items: Vec<Item>,
    #[serde(default)]
    pub priors: BTreeMap<String, Prior>,
    #[serde(skip)]
    labels: BTreeMap<String, String>,
}

fn lead(first: Cand, rest: &[Cand]) -> Vec<Cand> {
    let mut v = vec![first.clone()];
    v.extend(rest.iter().filter(|c| km(c.at, first.at) > MERGE_KM / 2.0).take(MAX_CANDIDATES - 1).cloned());
    v
}

impl Session {
    pub fn new(mut anchors: Vec<Anchor>, mut items: Vec<Item>, priors: BTreeMap<String, Prior>) -> Session {
        anchors.sort_by_key(|a| a.time);
        items.sort_by_key(|i| (i.time.is_none(), i.time, i.id));
        let mut s = Session { anchors, items, priors, labels: BTreeMap::new() };
        s.resolve();
        s
    }

    /// Recomputes everything derived (state, candidates, moments, flags) from the model's
    /// candidates, the anchors and the person's answers. Pure: the same inputs, the same result.
    pub fn resolve(&mut self) {
        let mut timeline = self.anchors.clone();
        for it in &self.items {
            if let (Some(at), Some(time), Answer::Confirmed) = (it.pinned, it.time, it.answer) {
                timeline.push(Anchor { time, at });
            }
        }
        timeline.sort_by_key(|a| a.time);

        // 1. each photo on its own: pinned, inferred from anchors, or the model's guess
        for it in self.items.iter_mut() {
            it.flag = None;
            it.group.clear();
            if it.answer == Answer::Rejected {
                it.cands.clear();
                it.state = State::Guess;
                continue;
            }
            let mut own = it.model.clone();
            if let Some(prior) = it.folder.as_ref().and_then(|f| self.priors.get(f)) {
                reweigh(&mut own, prior.at, FOLDER_WEIGHT, FOLDER_SCALE_KM);
            }
            if let Some(at) = it.pinned {
                it.state = State::Anchor;
                it.cands = lead(Cand { at, conf: 1.0, label: None }, &own);
            } else if let Some((at, conf)) = it.time.and_then(|t| infer(&timeline, t)) {
                it.state = State::Inferred;
                it.cands = lead(Cand { at, conf, label: None }, &own);
            } else {
                it.state = State::Guess;
                it.cands = own;
            }
        }

        // 2. a burst or Live pair shares one place: its best member's
        let mut best: BTreeMap<String, usize> = BTreeMap::new();
        let rank_of = |it: &Item| (it.state as u8 ^ 0xff, it.cands.first().map_or(0.0, |c| c.conf));
        for (i, it) in self.items.iter().enumerate() {
            let Some(b) = it.burst.clone().filter(|_| it.answer != Answer::Rejected && !it.cands.is_empty()) else { continue };
            let better = best.get(&b).is_none_or(|&j| {
                let (a, c) = (rank_of(it), rank_of(&self.items[j]));
                a.0 < c.0 || (a.0 == c.0 && a.1 > c.1)
            });
            if better {
                best.insert(b, i);
            }
        }
        for i in 0..self.items.len() {
            let Some(&j) = self.items[i].burst.as_ref().and_then(|b| best.get(b)) else { continue };
            if i == j || self.items[i].pinned.is_some() || self.items[i].answer == Answer::Rejected {
                continue;
            }
            let (cands, state) = (self.items[j].cands.clone(), self.items[j].state);
            let it = &mut self.items[i];
            it.cands = cands;
            it.state = if state == State::Guess { State::Guess } else { State::Inferred };
        }

        // 3. moments: time order, split on a gap, a jump of place, or the person's *Split*
        let mut order: Vec<usize> = (0..self.items.len()).filter(|&i| self.items[i].answer != Answer::Rejected).collect();
        order.sort_by_key(|&i| (self.items[i].time.is_none(), self.items[i].time, self.items[i].id));
        let mut groups: Vec<Vec<usize>> = Vec::new();
        let mut prev: Option<usize> = None;
        for &i in &order {
            let it = &self.items[i];
            let together = prev.is_some_and(|p| {
                let q = &self.items[p];
                !it.split_start
                    && matches!((q.time, it.time), (Some(a), Some(b)) if b - a <= MOMENT_GAP_SECS)
                    && !matches!((q.top(), it.top()), (Some(a), Some(b)) if km(a, b) > MOMENT_JUMP_KM)
            });
            if together {
                groups.last_mut().expect("a group is open").push(i);
            } else {
                groups.push(vec![i]);
            }
            prev = Some(i);
        }

        // 4. one place per moment for its guesses (weighted medoid), far members flagged
        self.labels.clear();
        let mut far = vec![false; self.items.len()];
        for g in &groups {
            let id = format!("m{}", self.items[g[0]].id);
            for &i in g {
                self.items[i].group = id.clone();
            }
            let guesses: Vec<(usize, Point, f32)> = g.iter().filter(|&&i| self.items[i].state == State::Guess).filter_map(|&i| self.items[i].cands.first().map(|c| (i, c.at, c.conf))).collect();
            if let Some(&(_, place, _)) = guesses.iter().min_by(|a, b| {
                let cost = |x: &(usize, Point, f32)| guesses.iter().map(|y| f64::from(y.2) * km(x.1, y.1)).sum::<f64>();
                cost(a).total_cmp(&cost(b))
            }) {
                let agree: Vec<f32> = guesses.iter().filter(|x| km(x.1, place) <= FAR_FROM_GROUP_KM).map(|x| x.2).collect();
                let moment_conf = agree.iter().sum::<f32>() / agree.len() as f32;
                for &(i, at, conf) in &guesses {
                    if km(at, place) <= FAR_FROM_GROUP_KM {
                        let own = self.items[i].cands.clone();
                        self.items[i].cands = lead(Cand { at: place, conf: moment_conf.max(conf), label: None }, &own);
                    } else {
                        far[i] = true;
                    }
                }
            }
            let times: Vec<i64> = g.iter().filter_map(|&i| self.items[i].time).collect();
            let mut label = match (times.iter().min(), times.iter().max()) {
                (Some(&a), Some(&b)) => time_range(a, b),
                _ => "No time".to_string(),
            };
            label.push_str(&format!(" · {} photo{}", g.len(), if g.len() == 1 { "" } else { "s" }));
            if let Some(prior) = g.iter().find_map(|&i| self.items[i].folder.as_ref().and_then(|f| self.priors.get(f))) {
                label.push_str(&format!(" · {}", prior.name));
            }
            self.labels.insert(id, label);
        }

        // 5. flags, for guesses only
        let mut line: Vec<(i64, Point, Option<&str>)> = self.anchors.iter().map(|a| (a.time, a.at, None)).collect();
        for &i in &order {
            if let (Some(t), Some(at)) = (self.items[i].time, self.items[i].top()) {
                line.push((t, at, Some(self.items[i].group.as_str())));
            }
        }
        line.sort_by_key(|e| e.0);
        let impossible = |i: usize| -> bool {
            let it = &self.items[i];
            let (Some(t), Some(at)) = (it.time, it.top()) else { return false };
            let own = Some(it.group.as_str());
            let fast = |e: &(i64, Point, Option<&str>)| km(at, e.1) / ((e.0 - t).abs().max(60) as f64 / 3600.0) > MAX_SPEED_KMH;
            let k = line.partition_point(|e| e.0 < t);
            let back = line[..k].iter().rev().find(|e| e.2 != own);
            let ahead = line[k..].iter().find(|e| e.2 != own && e.0 >= t);
            back.is_some_and(fast) || ahead.is_some_and(fast)
        };
        let flags: Vec<(usize, Flag)> = order
            .iter()
            .filter(|&&i| self.items[i].state == State::Guess && !self.items[i].cands.is_empty())
            .filter_map(|&i| {
                if impossible(i) {
                    Some((i, Flag::ImpossibleSpeed))
                } else if far[i] {
                    Some((i, Flag::FarFromGroup))
                } else if self.items[i].cands[0].conf < LOW_CONFIDENCE {
                    Some((i, Flag::LowConfidence))
                } else {
                    None
                }
            })
            .collect();
        for (i, f) in flags {
            self.items[i].flag = Some(f);
        }
    }

    /// Every photo that still has a suggestion (a rejected one is gone).
    pub fn outputs(&self) -> Vec<Out> {
        self.items
            .iter()
            .filter(|i| i.answer != Answer::Rejected && !i.cands.is_empty())
            .map(|i| Out { asset: i.id, cands: i.cands.clone(), state: i.state, group: i.group.clone(), label: self.labels.get(&i.group).cloned().unwrap_or_default(), flag: i.flag })
            .collect()
    }

    pub fn moment_count(&self) -> usize {
        self.labels.len()
    }

    /// Applies one action and returns only the suggestions it changed (spec §10.2, *Propagation*):
    /// confirming makes a photo an anchor and its time neighbours are inferred from it at once;
    /// rejecting clears it (and what was inferred from it); moving a pin re-anchors.
    pub fn apply(&mut self, event: &Event) -> Vec<Out> {
        let before: HashMap<u64, Out> = self.outputs().into_iter().map(|o| (o.asset, o)).collect();
        let set = |ids: &[u64]| ids.iter().copied().collect::<HashSet<u64>>();
        match event {
            Event::Confirm(ids) => {
                let ids = set(ids);
                for it in self.items.iter_mut().filter(|i| ids.contains(&i.id) && i.answer != Answer::Rejected) {
                    if let Some(at) = it.top() {
                        it.pinned = Some(at);
                        it.answer = Answer::Confirmed;
                    }
                }
            }
            Event::Reject(ids) => {
                let ids = set(ids);
                for it in self.items.iter_mut().filter(|i| ids.contains(&i.id)) {
                    it.answer = Answer::Rejected;
                    it.pinned = None;
                }
            }
            Event::Move { assets, at } => {
                let ids = set(assets);
                for it in self.items.iter_mut().filter(|i| ids.contains(&i.id)) {
                    it.pinned = Some(*at);
                    it.answer = Answer::Confirmed;
                }
            }
            Event::Split { group, at_asset } => {
                if let Some(it) = self.items.iter_mut().find(|i| i.id == *at_asset && i.group == *group) {
                    it.split_start = true;
                }
            }
        }
        self.resolve();
        self.outputs().into_iter().filter(|o| before.get(&o.asset) != Some(o)).collect()
    }
}

// ------------------------------------------------------------------ vectors files (LWVG)

/// IEEE half precision, round to nearest even.
pub fn f16_bits(x: f32) -> u16 {
    let b = x.to_bits();
    let sign = ((b >> 16) & 0x8000) as u16;
    let exp = ((b >> 23) & 0xff) as i32;
    let man = b & 0x7f_ffff;
    if exp == 0xff {
        return sign | 0x7c00 | if man != 0 { 0x200 } else { 0 };
    }
    let e = exp - 127 + 15;
    if e >= 0x1f {
        return sign | 0x7c00;
    }
    if e <= 0 {
        if e < -10 {
            return sign;
        }
        let m = man | 0x80_0000;
        let shift = (14 - e) as u32;
        let half = (m >> shift) as u16;
        let rem = m & ((1 << shift) - 1);
        let mid = 1 << (shift - 1);
        return sign | if rem > mid || (rem == mid && half & 1 == 1) { half + 1 } else { half };
    }
    let half = ((e as u32) << 10 | man >> 13) as u16;
    let rem = man & 0x1fff;
    sign | if rem > 0x1000 || (rem == 0x1000 && half & 1 == 1) { half + 1 } else { half }
}

/// Lumen's vectors file (`src-tauri/src/plugins/models.rs`): `LWVG`, u32 version 1, rows, dim,
/// dtype 1 (f16), the f16 matrix, then a (lat, lon) f32 payload per row.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Lwvg {
    pub dim: usize,
    pub matrix: Vec<u16>,
    pub payload: Vec<f32>,
}

impl Lwvg {
    pub fn new(dim: usize) -> Lwvg {
        Lwvg { dim, matrix: Vec::new(), payload: Vec::new() }
    }

    pub fn rows(&self) -> usize {
        self.payload.len() / 2
    }

    pub fn push(&mut self, embedding: &[f32], at: Point) {
        debug_assert_eq!(embedding.len(), self.dim);
        self.matrix.extend(embedding.iter().map(|&x| f16_bits(x)));
        self.payload.extend([at.lat as f32, at.lon as f32]);
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let mut b = Vec::with_capacity(20 + self.matrix.len() * 2 + self.payload.len() * 4);
        b.extend(b"LWVG");
        for w in [1u32, self.rows() as u32, self.dim as u32, 1] {
            b.extend(w.to_le_bytes());
        }
        b.extend(self.matrix.iter().flat_map(|h| h.to_le_bytes()));
        b.extend(self.payload.iter().flat_map(|f| f.to_le_bytes()));
        b
    }

    /// `None` when the bytes aren't a file this plugin wrote (two payload floats per row).
    pub fn parse(b: &[u8]) -> Option<Lwvg> {
        let word = |i: usize| Some(u32::from_le_bytes(b.get(i..i + 4)?.try_into().ok()?) as usize);
        if b.get(..4)? != b"LWVG" || word(4)? != 1 || word(16)? != 1 {
            return None;
        }
        let (rows, dim) = (word(8)?, word(12)?);
        let matrix_end = 20 + rows.checked_mul(dim)?.checked_mul(2)?;
        if b.len() != matrix_end + rows * 8 {
            return None;
        }
        let matrix = b[20..matrix_end].chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
        let payload = b[matrix_end..].chunks_exact(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect();
        Some(Lwvg { dim, matrix, payload })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PARIS: Point = Point { lat: 48.8566, lon: 2.3522 };
    const ROME: Point = Point { lat: 41.9028, lon: 12.4964 };
    const BLED: Point = Point { lat: 46.3683, lon: 14.1146 };

    fn cand(at: Point, conf: f32) -> Cand {
        Cand { at, conf, label: None }
    }
    fn item(id: u64, t: i64, at: Point, conf: f32) -> Item {
        Item::new(id, Some(t), None, None, vec![cand(at, conf)])
    }
    fn north(p: Point, km: f64) -> Point {
        Point { lat: p.lat + km / KM_PER_DEGREE, lon: p.lon }
    }
    fn session(items: Vec<Item>) -> Session {
        Session::new(Vec::new(), items, BTreeMap::new())
    }
    fn out(s: &Session, id: u64) -> Out {
        s.outputs().into_iter().find(|o| o.asset == id).unwrap_or_else(|| panic!("no suggestion for {id}"))
    }

    #[test]
    fn moments_split_on_30_min_gap() {
        let s = session(vec![item(1, 0, PARIS, 0.5), item(2, 600, PARIS, 0.5), item(3, 600 + MOMENT_GAP_SECS + 60, PARIS, 0.5)]);
        assert_eq!(out(&s, 1).group, out(&s, 2).group);
        assert_ne!(out(&s, 2).group, out(&s, 3).group);
        assert_eq!(s.moment_count(), 2);
    }

    #[test]
    fn moments_split_on_50_km_jump() {
        let s = session(vec![item(1, 0, PARIS, 0.5), item(2, 60, north(PARIS, 100.0), 0.5)]);
        assert_ne!(out(&s, 1).group, out(&s, 2).group);
        let near = session(vec![item(1, 0, PARIS, 0.5), item(2, 60, north(PARIS, 10.0), 0.5)]);
        assert_eq!(out(&near, 1).group, out(&near, 2).group);
    }

    #[test]
    fn interpolates_between_anchors() {
        let a = [Anchor { time: 0, at: Point::new(46.0, 14.0) }, Anchor { time: 3600, at: Point::new(46.0, 15.0) }];
        let (p, conf) = infer(&a, 900).expect("between two anchors");
        assert!((p.lat - 46.0).abs() < 1e-9 && (p.lon - 14.25).abs() < 1e-9, "{p:?}");
        assert!(conf > 0.5);
        // anchors further apart than two hours don't interpolate
        let far = [Anchor { time: 0, at: Point::new(46.0, 14.0) }, Anchor { time: 3 * 3600, at: Point::new(46.0, 14.5) }];
        assert_eq!(infer(&far, 5400), None);
    }

    #[test]
    fn no_inference_over_250_kmh() {
        let a = [Anchor { time: 0, at: Point::new(46.0, 14.0) }, Anchor { time: 3600, at: Point::new(50.0, 25.0) }];
        assert_eq!(infer(&a, 1800), None, "900 km in an hour is not a trip");
    }

    #[test]
    fn single_anchor_within_15_min() {
        let a = [Anchor { time: 0, at: BLED }];
        assert_eq!(infer(&a, 600).map(|x| x.0), Some(BLED));
        assert_eq!(infer(&a, -600).map(|x| x.0), Some(BLED));
        assert_eq!(infer(&a, SINGLE_ANCHOR_SECS + 1), None);
    }

    #[test]
    fn bursts_share_place() {
        let mut a = item(1, 0, PARIS, 0.6);
        let mut b = item(2, 5, ROME, 0.3);
        a.burst = Some("b".into());
        b.burst = Some("b".into());
        let s = session(vec![a, b]);
        assert_eq!(out(&s, 1).cands[0].at, PARIS);
        assert_eq!(out(&s, 2).cands[0].at, PARIS, "the burst takes its best member's place");
    }

    #[test]
    fn flags_far_from_group_25_km() {
        // three photos in five minutes; one guessed 40 km away (under the 50 km that splits a moment)
        let s = session(vec![item(1, 0, PARIS, 0.5), item(2, 60, PARIS, 0.5), item(3, 120, north(PARIS, 40.0), 0.5)]);
        assert_eq!(out(&s, 3).flag, Some(Flag::FarFromGroup));
        assert_eq!(out(&s, 1).flag, None);
        assert_eq!(out(&s, 2).flag, None);
        assert_eq!(out(&s, 3).group, out(&s, 1).group);
    }

    #[test]
    fn flags_low_confidence_0_2() {
        let s = session(vec![item(1, 0, PARIS, 0.15), item(2, 7200, north(PARIS, 100.0), 0.3)]);
        assert_eq!(out(&s, 1).flag, Some(Flag::LowConfidence));
        assert_eq!(out(&s, 2).flag, None);
    }

    #[test]
    fn flags_impossible_speed_between_moments() {
        // Paris to Rome (1,100 km) in 40 minutes
        let s = session(vec![item(1, 0, PARIS, 0.5), item(2, 2400, ROME, 0.5)]);
        assert_eq!(out(&s, 1).flag, Some(Flag::ImpossibleSpeed));
        assert_eq!(out(&s, 2).flag, Some(Flag::ImpossibleSpeed));
    }

    #[test]
    fn confirm_propagates_to_neighbours() {
        // A and C are a trip's two ends, B was guessed in Tokyo; confirming A and C places B between
        let (tokyo, c_at) = (Point::new(35.68, 139.69), north(PARIS, 20.0));
        let mut s = session(vec![item(1, 0, PARIS, 0.5), item(2, 1800, tokyo, 0.5), item(3, 3600, c_at, 0.5)]);
        assert_eq!(out(&s, 2).state, State::Guess);
        let changed = s.apply(&Event::Confirm(vec![1, 3]));
        let b = changed.iter().find(|o| o.asset == 2).expect("B was re-inferred");
        assert_eq!(b.state, State::Inferred);
        assert!(km(b.cands[0].at, lerp_for_test(PARIS, c_at)) < 0.5, "{:?}", b.cands[0].at);
        assert_eq!(changed.iter().find(|o| o.asset == 1).map(|o| o.state), Some(State::Anchor));
    }

    fn lerp_for_test(a: Point, b: Point) -> Point {
        lerp(a, b, 0.5)
    }

    #[test]
    fn reject_clears() {
        let c_at = north(PARIS, 20.0);
        let mut s = session(vec![item(1, 0, PARIS, 0.5), item(2, 1800, ROME, 0.5), item(3, 3600, c_at, 0.5)]);
        s.apply(&Event::Confirm(vec![1, 3]));
        assert_eq!(out(&s, 2).state, State::Inferred);
        let changed = s.apply(&Event::Reject(vec![1]));
        assert!(s.outputs().iter().all(|o| o.asset != 1), "a rejected photo has no suggestion");
        assert_eq!(changed.iter().find(|o| o.asset == 2).map(|o| o.state), Some(State::Guess), "what was inferred from it is a guess again");
    }

    #[test]
    fn move_reanchors() {
        let mut s = session(vec![item(1, 0, ROME, 0.4), item(2, 300, ROME, 0.4)]);
        let changed = s.apply(&Event::Move { assets: vec![1], at: BLED });
        assert_eq!(out(&s, 1).state, State::Anchor);
        assert_eq!(out(&s, 1).cands[0].at, BLED);
        let two = changed.iter().find(|o| o.asset == 2).expect("the neighbour follows the pin");
        assert_eq!((two.state, two.cands[0].at), (State::Inferred, BLED));
    }

    #[test]
    fn split_starts_a_new_moment() {
        let mut s = session(vec![item(1, 0, PARIS, 0.5), item(2, 60, PARIS, 0.5), item(3, 120, PARIS, 0.5)]);
        let group = out(&s, 1).group;
        s.apply(&Event::Split { group, at_asset: 3 });
        assert_eq!(out(&s, 1).group, out(&s, 2).group);
        assert_ne!(out(&s, 2).group, out(&s, 3).group);
    }

    fn hit(at: Point, score: f32) -> Hit {
        Hit { at, score }
    }

    #[test]
    fn library_prior_reranks_but_never_beats_anchor() {
        let gallery = [hit(ROME, 0.30), hit(BLED, 0.29)];
        assert_eq!(rank(&gallery, &[], None, &[])[0].at, ROME, "raw order");
        let ranked = rank(&gallery, &[], None, &[BLED]);
        assert_eq!(ranked[0].at, BLED, "the person has been to Bled");
        // the same photo, a minute from a camera photo taken in Paris: the anchor decides
        let mut it = Item::new(1, Some(60), None, None, ranked);
        it.model[0].conf = 0.9;
        let s = Session::new(vec![Anchor { time: 0, at: PARIS }], vec![it], BTreeMap::new());
        let o = out(&s, 1);
        assert_eq!((o.state, o.cands[0].at), (State::Inferred, PARIS));
    }

    #[test]
    fn personal_match_above_threshold_wins() {
        let gallery = [hit(ROME, 0.30), hit(PARIS, 0.28)];
        assert_eq!(rank(&gallery, &[], Some(hit(BLED, 0.70)), &[])[0].at, BLED);
        assert_eq!(rank(&gallery, &[], Some(hit(BLED, 0.60)), &[])[0].at, ROME, "below 0.65 it only counts as a hint");
        let ranked = rank(&gallery, &[], Some(hit(BLED, 0.70)), &[]);
        assert!(ranked.len() <= MAX_CANDIDATES && ranked.iter().any(|c| c.at == ROME));
    }

    #[test]
    fn dense_gallery_points_within_25_km_of_clusters() {
        let photos: Vec<Point> = (0..5).map(|i| north(BLED, f64::from(i) * 0.5)).chain((0..3).map(|_| PARIS)).chain([ROME]).collect();
        let centres = clusters(&photos);
        assert_eq!(centres.len(), 2, "Rome has one photo, under the three a cluster needs");
        let points = dense_points(&centres[..1], 1.0);
        assert!((1900..2050).contains(&points.len()), "{} points on a 1 km grid", points.len());
        assert!(points.iter().all(|&p| km(p, centres[0]) <= DENSE_RADIUS_KM + 0.3));
        assert!(points.iter().any(|&p| km(p, centres[0]) < 1.0), "one at the centre");
        // seventy-six clusters at 1 km would be 149k points: the grid widens until it fits the quota
        let many = vec![PARIS; 76];
        assert!(dense_step_km(&many) > 1.0 && dense_points(&many, dense_step_km(&many)).len() <= MAX_DENSE_POINTS);
        assert_eq!(dense_step_km(&centres), 1.0);
    }

    #[test]
    fn folder_name_prior() {
        let mut it = Item::new(1, Some(0), None, Some("Paris 2024".into()), vec![cand(ROME, 0.5), cand(PARIS, 0.4)]);
        it.folder = Some("Paris 2024".into());
        let priors = BTreeMap::from([("Paris 2024".to_string(), Prior { at: PARIS, name: "Paris".into() })]);
        let s = Session::new(Vec::new(), vec![it], priors);
        assert_eq!(out(&s, 1).cands[0].at, PARIS);
        assert!(out(&s, 1).label.ends_with("Paris"), "{}", out(&s, 1).label);
        // without the folder's hint the guess stands
        let plain = session(vec![Item::new(1, Some(0), None, Some("Paris 2024".into()), vec![cand(ROME, 0.5), cand(PARIS, 0.4)])]);
        assert_eq!(out(&plain, 1).cands[0].at, ROME);
    }

    #[test]
    fn rank_merges_nearby_rows_and_sums_to_one() {
        let g = [hit(PARIS, 0.3), hit(north(PARIS, 2.0), 0.3), hit(ROME, 0.29)];
        let r = rank(&g, &[], None, &[]);
        assert_eq!(r.len(), 2, "the two Paris rows are one place");
        assert!((r.iter().map(|c| c.conf).sum::<f32>() - 1.0).abs() < 1e-4);
    }

    #[test]
    fn time_parses_and_formats() {
        let t = parse_time("2024-03-12T14:02:33").unwrap();
        assert_eq!(parse_time("2024-03-12T14:12:33").unwrap() - t, 600);
        assert_eq!(time_range(t, t + 38 * 60), "12 Mar 2024 14:02–14:40");
        assert_eq!(parse_time("2024-13-12T14:02:33"), None);
        assert_eq!(parse_time("not a time at all!!"), None);
        assert_eq!(clock(parse_time("1970-01-01T00:00:00").unwrap()), "00:00");
    }

    #[test]
    fn half_floats_and_the_vectors_file_round_trip() {
        assert_eq!(f16_bits(1.0), 0x3c00);
        assert_eq!(f16_bits(-2.0), 0xc000);
        assert_eq!(f16_bits(0.5), 0x3800);
        assert_eq!(f16_bits(0.0), 0);
        assert_eq!(f16_bits(1e9), 0x7c00);
        let mut v = Lwvg::new(2);
        v.push(&[1.0, 0.5], BLED);
        v.push(&[0.0, -2.0], PARIS);
        let again = Lwvg::parse(&v.to_bytes()).expect("parses");
        assert_eq!(again, v);
        assert_eq!(again.rows(), 2);
        assert_eq!(Lwvg::parse(b"nope"), None);
        let mut cut = v.to_bytes();
        cut.pop();
        assert_eq!(Lwvg::parse(&cut), None);
    }

    #[test]
    fn a_session_survives_storage() {
        let s = session(vec![item(1, 0, PARIS, 0.5), item(2, 60, PARIS, 0.5)]);
        let json = serde_json::to_string(&s).unwrap();
        let mut back: Session = serde_json::from_str(&json).unwrap();
        back.resolve();
        assert_eq!(back.outputs(), s.outputs());
    }
}
