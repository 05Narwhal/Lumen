//! Clockwork's maths, all native and testable: wall-clock arithmetic, which cameras to trust, and
//! the offset search.
//!
//! The idea: when two cameras were used at the same events, their photos land on the same
//! minutes once the clock that is off is shifted by the right amount. For one stretch of a
//! camera's photos (an *episode*) every shift from -15 h to +15 h is tried, and a shift scores
//! the number of the camera's distinct minutes that have a trusted photo within a few minutes of
//! them after the shift. A real offset is a sharp peak far above what chance gives at the other
//! shifts, and nothing else comes close; anything less is left alone.

/// Minutes searched each way (a camera left on the wrong zone is within 15 h of the right one in
/// all but the far corners of the world).
pub const SHIFT_MAX: i64 = 15 * 60;
/// Two photos are "at the same time" when they are within this many minutes after the shift.
pub const TOLERANCE: i64 = 3;
/// Photos of a camera more than this many minutes apart are different episodes.
pub const EPISODE_GAP: i64 = 2 * 24 * 60;
/// An episode needs at least this many distinct minutes with a photo, and a peak this many hits.
pub const MIN_MINUTES: usize = 6;
pub const MIN_HITS: usize = 4;
/// A shift smaller than this (minutes) isn't worth a proposal.
pub const MIN_SHIFT: i64 = 2;
/// A second peak at least this fraction as far above chance as the first, further than [`NEAR`]
/// from it, makes the answer ambiguous (a camera that is right and wrong on different days, say).
const RIVAL: f64 = 0.6;
/// Chance deviations a peak must stand above the shifts around it (the best of a hundred or so
/// independent chance shifts sits near 2.5).
const MIN_Z: f64 = 5.0;
/// The peak must beat chance by at least this share of the episode's minutes.
const MIN_EXCESS: f64 = 0.15;
const NEAR: i64 = 30;

// --- wall-clock arithmetic (days from civil: Howard Hinnant's algorithms) -----------------------

fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// `YYYY-MM-DDTHH:MM:SS` (a wall clock, no zone) as seconds since 1970-01-01T00:00:00 on that
/// same wall clock.
pub fn parse_wall(s: &str) -> Option<i64> {
    let b = s.as_bytes();
    if b.len() != 19 || b[4] != b'-' || b[7] != b'-' || b[10] != b'T' || b[13] != b':' || b[16] != b':' {
        return None;
    }
    let n = |a: usize, z: usize| s.get(a..z)?.parse::<i64>().ok();
    let (y, mo, d, h, mi, se) = (n(0, 4)?, n(5, 7)?, n(8, 10)?, n(11, 13)?, n(14, 16)?, n(17, 19)?);
    if !(1..=12).contains(&mo) || !(1..=31).contains(&d) || h > 23 || mi > 59 || se > 59 {
        return None;
    }
    Some(days_from_civil(y, mo, d) * 86_400 + h * 3600 + mi * 60 + se)
}

pub fn format_wall(secs: i64) -> String {
    let (days, rest) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    let (y, m, d) = civil_from_days(days);
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}", rest / 3600, rest % 3600 / 60, rest % 60)
}

/// The wall clock `delta` seconds earlier (a camera that is `delta` ahead, put right).
pub fn shifted(wall: &str, delta_secs: i64) -> Option<String> {
    parse_wall(wall).map(|s| format_wall(s - delta_secs))
}

/// `2 h 14 min`, `45 min`, `1 day 2 h`.
pub fn duration_words(minutes: i64) -> String {
    let m = minutes.abs();
    let (d, h, mi) = (m / 1440, m % 1440 / 60, m % 60);
    let mut parts = Vec::new();
    if d > 0 {
        parts.push(format!("{d} day{}", if d == 1 { "" } else { "s" }));
    }
    if h > 0 {
        parts.push(format!("{h} h"));
    }
    if mi > 0 || parts.is_empty() {
        parts.push(format!("{mi} min"));
    }
    parts.join(" ")
}

// --- whom to trust ---------------------------------------------------------------------------

/// A camera's clock can be trusted when it knows its own time zone (its photos record a UTC
/// offset, as phones do) or fixes its time from GPS (most of its photos carry the camera's own
/// GPS). A handful of photos prove nothing.
pub fn is_trusted(total: usize, with_offset: usize, with_camera_gps: usize) -> bool {
    total >= 10 && (with_offset * 10 >= total * 6 || with_camera_gps * 10 >= total * 6)
}

/// A date that came from a camera's shutter or an editor's own field; never the file's own date
/// or an edit time, which say nothing about a clock.
pub fn is_shutter_date(source: Option<&str>) -> bool {
    !matches!(source, Some("file_time") | Some("modified"))
}

// --- the search ------------------------------------------------------------------------------

/// What the search found for one episode.
#[derive(Clone, Debug, PartialEq)]
pub struct Peak {
    /// Minutes the camera is **ahead** of the trusted clock (negative: behind).
    pub shift: i64,
    /// Distinct camera minutes that have a trusted photo within [`TOLERANCE`] at this shift.
    pub hits: usize,
    /// Distinct camera minutes in the episode.
    pub of: usize,
    /// How many chance standard deviations above the other shifts the peak stands.
    pub z: f64,
}

/// `camera` and `trusted` are sorted distinct minutes (wall clock). `None` unless one shift stands
/// clearly above all the others and isn't zero.
pub fn search(camera: &[i64], trusted: &[i64]) -> Option<Peak> {
    let n = camera.len();
    if n < MIN_MINUTES || trusted.is_empty() {
        return None;
    }
    let w = SHIFT_MAX;
    let bins = (2 * w + 1) as usize;
    let mut hits = vec![0u32; bins];
    let mut mark = vec![u32::MAX; bins];
    for (ti, &t) in camera.iter().enumerate() {
        let lo = trusted.partition_point(|&r| r < t - w - TOLERANCE);
        let hi = trusted.partition_point(|&r| r <= t + w + TOLERANCE);
        for &r in &trusted[lo..hi] {
            let d = t - r;
            for dd in (d - TOLERANCE)..=(d + TOLERANCE) {
                if dd.abs() <= w {
                    let i = (dd + w) as usize;
                    if mark[i] != ti as u32 {
                        mark[i] = ti as u32;
                        hits[i] += 1;
                    }
                }
            }
        }
    }
    let best = *hits.iter().max()?;
    if (best as usize) < MIN_HITS {
        return None;
    }
    // The middle of the first run of bins that reach the best score.
    let first = hits.iter().position(|&h| h == best)?;
    let mut last = first;
    while last + 1 < bins && hits[last + 1] == best {
        last += 1;
    }
    let peak = (first + last) / 2;
    let far = |i: usize| (i as i64 - peak as i64).abs();

    // What chance gives: the mean score over the shifts just beside the peak (a quarter to an hour
    // and a half away), where the same days overlap just as much. A far shift, where the days
    // overlap less, would understate it: two cameras used in the same daylight hours score high
    // across a hump hours wide, and only a spike a few minutes wide is a real offset.
    let (mut sum, mut count) = (0u64, 0u64);
    for i in (0..bins).filter(|&i| far(i) > 15 && far(i) <= 90) {
        sum += hits[i] as u64;
        count += 1;
    }
    let bg = if count == 0 { 0.0 } else { sum as f64 / count as f64 };
    let p = (bg / n as f64).clamp(0.0, 1.0);
    let sigma = (n as f64 * p * (1.0 - p) + 0.5).sqrt();
    let excess = best as f64 - bg;
    if excess / sigma < MIN_Z || excess < MIN_EXCESS * n as f64 {
        return None;
    }
    // A rival: a different shift that also stands clearly above chance and nearly as far as the
    // first (the camera is right on some days and wrong on others): no guess.
    let rival = (0..bins).filter(|&i| far(i) > NEAR).map(|i| hits[i]).max().unwrap_or(0);
    let rival_excess = rival as f64 - bg;
    if rival_excess / sigma >= MIN_Z && rival_excess >= RIVAL * excess {
        return None;
    }
    let z = excess / sigma;
    let shift = refine(camera, trusted, peak as i64 - w);
    if shift.abs() < MIN_SHIFT {
        return None;
    }
    Some(Peak { shift, hits: best as usize, of: n, z })
}

/// The median of the offsets of the matching pairs near `rough`, snapped to a quarter hour (a
/// time zone) when it is within two minutes of one.
fn refine(camera: &[i64], trusted: &[i64], rough: i64) -> i64 {
    let mut ds: Vec<i64> = Vec::new();
    for &t in camera {
        let want = t - rough;
        let i = trusted.partition_point(|&r| r < want);
        let near = [i.checked_sub(1), Some(i)].into_iter().flatten().filter_map(|j| trusted.get(j)).min_by_key(|&&r| (r - want).abs());
        if let Some(&r) = near {
            if (r - want).abs() <= TOLERANCE {
                ds.push(t - r);
            }
        }
    }
    if ds.is_empty() {
        return rough;
    }
    ds.sort_unstable();
    let m = ds[ds.len() / 2];
    let quarter = (m as f64 / 15.0).round() as i64 * 15;
    if quarter != 0 && (m - quarter).abs() <= 2 {
        quarter
    } else {
        m
    }
}

/// How sure Clockwork is, 0-1: it grows with the peak's distance from chance (every peak that
/// gets this far already stands 5 chance deviations above its surroundings) and is held back when
/// few minutes back it.
pub fn confidence(p: &Peak) -> f32 {
    let base = 1.0 - (-p.z / 4.0).exp();
    let backed = if p.hits >= 8 { 1.0 } else { 0.9 };
    ((base * backed) as f32).clamp(0.0, 0.97)
}

/// Splits sorted shot times (seconds) into runs where neighbours are within [`EPISODE_GAP`].
pub fn episodes(sorted_secs: &[i64]) -> Vec<std::ops::Range<usize>> {
    let mut out = Vec::new();
    let mut start = 0;
    for i in 1..=sorted_secs.len() {
        if i == sorted_secs.len() || sorted_secs[i] - sorted_secs[i - 1] > EPISODE_GAP * 60 {
            if i > start {
                out.push(start..i);
            }
            start = i;
        }
    }
    out
}

/// Distinct minutes of sorted seconds.
pub fn minutes(sorted_secs: &[i64]) -> Vec<i64> {
    let mut m: Vec<i64> = sorted_secs.iter().map(|s| s.div_euclid(60)).collect();
    m.dedup();
    m
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A tiny deterministic generator, so the synthetic libraries are the same every run.
    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            self.0 >> 33
        }
        fn below(&mut self, n: u64) -> i64 {
            (self.next() % n) as i64
        }
    }

    const DAY: i64 = 1440;

    /// One trip: `days` days from `start` (minutes). The phone shoots at `phone_n` random minutes
    /// of each day between 09:00 and 20:00, and the camera shoots `cam_n` of its own plus
    /// `shared` at the same moments (within a minute) as the phone, on a clock `cam_clock`
    /// minutes ahead.
    fn trip(rng: &mut Rng, start: i64, days: i64, phone_n: u64, cam_n: u64, shared: usize, cam_clock: i64) -> (Vec<i64>, Vec<i64>) {
        let (mut phone, mut cam) = (Vec::new(), Vec::new());
        for d in 0..days {
            let day = start + d * DAY;
            let mut today: Vec<i64> = (0..phone_n).map(|_| day + 540 + rng.below(660)).collect();
            for k in 0..shared.min(today.len()) {
                cam.push(today[k] + rng.below(3) - 1 + cam_clock);
            }
            for _ in 0..cam_n {
                cam.push(day + 540 + rng.below(660) + cam_clock);
            }
            phone.append(&mut today);
        }
        phone.sort_unstable();
        phone.dedup();
        cam.sort_unstable();
        cam.dedup();
        (phone, cam)
    }

    #[test]
    fn wall_clock_round_trips_and_shifts() {
        assert_eq!(parse_wall("1970-01-01T00:00:00"), Some(0));
        assert_eq!(format_wall(parse_wall("2024-02-29T23:59:59").unwrap()), "2024-02-29T23:59:59");
        assert_eq!(shifted("2024-03-01T01:00:00", 2 * 3600 + 14 * 60).as_deref(), Some("2024-02-29T22:46:00"));
        assert_eq!(shifted("2024-03-01T01:00:00", -3600).as_deref(), Some("2024-03-01T02:00:00"));
        assert_eq!(shifted("1969-12-31T23:00:00", 0).as_deref(), Some("1969-12-31T23:00:00"));
        for bad in ["2024-13-01T00:00:00", "2024-01-01 00:00:00", "soon", "2024-01-01T24:00:00", ""] {
            assert_eq!(parse_wall(bad), None, "{bad}");
        }
    }

    #[test]
    fn durations_read_like_a_person_says_them() {
        assert_eq!(duration_words(134), "2 h 14 min");
        assert_eq!(duration_words(-45), "45 min");
        assert_eq!(duration_words(120), "2 h");
        assert_eq!(duration_words(1500), "1 day 1 h");
    }

    #[test]
    fn a_camera_with_no_clock_of_its_own_isnt_trusted() {
        assert!(is_trusted(100, 90, 0), "a phone records its offset");
        assert!(is_trusted(100, 0, 70), "or fixes its time from GPS");
        assert!(!is_trusted(100, 10, 20), "a camera set by hand");
        assert!(!is_trusted(8, 8, 8), "too few photos to say");
        assert!(is_shutter_date(Some("original")) && is_shutter_date(None));
        assert!(!is_shutter_date(Some("file_time")) && !is_shutter_date(Some("modified")));
    }

    #[test]
    fn a_camera_two_hours_ahead_is_found_to_the_minute() {
        let mut rng = Rng(7);
        // 2 h 14 min ahead, three days, a few shared moments a day.
        let (phone, cam) = trip(&mut rng, 480_000, 3, 30, 10, 12, 134);
        let p = search(&cam, &phone).expect("a clear offset");
        assert!((p.shift - 134).abs() <= 1, "found {}", p.shift);
        assert!(p.hits >= 20 && p.z > 5.0, "{p:?}");
        assert!(confidence(&p) >= 0.7, "{p:?}");
    }

    #[test]
    fn behind_is_negative_and_a_zone_snaps_to_the_quarter_hour() {
        let mut rng = Rng(11);
        let (phone, cam) = trip(&mut rng, 5_000_000, 4, 40, 10, 10, -3 * 60);
        let p = search(&cam, &phone).expect("three hours behind");
        assert_eq!(p.shift, -180, "a whole zone is exact");
        let mut rng = Rng(12);
        let (phone, cam) = trip(&mut rng, 5_000_000, 4, 40, 10, 10, 5 * 60 + 30);
        assert_eq!(search(&cam, &phone).unwrap().shift, 330, "India's half hour");
    }

    #[test]
    fn a_camera_whose_clock_is_right_is_left_alone() {
        let mut rng = Rng(3);
        let (phone, cam) = trip(&mut rng, 5_000_000, 3, 30, 14, 8, 0);
        assert_eq!(search(&cam, &phone), None, "the peak is at zero");
    }

    #[test]
    fn unrelated_cameras_never_make_a_finding() {
        // The camera was used a year after the phone: nothing to compare with.
        for seed in 1..=60 {
            let mut rng = Rng(seed);
            let (phone, _) = trip(&mut rng, 5_000_000, 5, 25, 0, 0, 0);
            let (_, cam) = trip(&mut rng, 5_000_000 + 400 * DAY, 3, 0, 20, 0, 0);
            assert_eq!(search(&cam, &phone), None, "seed {seed}");
        }
    }

    #[test]
    fn two_cameras_on_the_same_days_but_never_at_the_same_moments_are_not_matched() {
        // The same days, the same daylight hours, but each shoots at its own moments: the phone
        // is dense, so many shifts score about the same and none stands out.
        for seed in 1..=60 {
            let mut rng = Rng(100 + seed);
            let (phone, cam) = trip(&mut rng, 5_000_000, 3, 20 + (seed as u64 % 4) * 20, 12, 0, 77);
            assert_eq!(search(&cam, &phone), None, "seed {seed}");
        }
    }

    #[test]
    fn too_little_is_not_enough() {
        let mut rng = Rng(5);
        let (phone, cam) = trip(&mut rng, 5_000_000, 1, 10, 0, 4, 90);
        assert_eq!(search(&cam[..3.min(cam.len())], &phone), None);
        assert_eq!(search(&cam, &[]), None);
    }

    #[test]
    fn a_camera_right_on_one_trip_and_wrong_on_the_next_is_judged_a_trip_at_a_time() {
        let mut rng = Rng(9);
        let (phone_a, cam_a) = trip(&mut rng, 5_000_000, 3, 30, 10, 10, 0);
        let (phone_b, cam_b) = trip(&mut rng, 5_000_000 + 60 * DAY, 3, 30, 10, 10, 60);
        // A camera's trips are separate episodes, so one trip's clock never decides another's.
        let secs: Vec<i64> = cam_a.iter().chain(&cam_b).map(|m| m * 60).collect();
        assert_eq!(episodes(&secs).len(), 2);
        // Each trip alone:
        assert_eq!(search(&cam_a, &phone_a), None, "trip A: nothing to fix");
        assert_eq!(search(&cam_b, &phone_b).unwrap().shift, 60, "trip B: an hour ahead");
    }

    #[test]
    fn episodes_split_on_a_two_day_gap() {
        let t = |d: i64, m: i64| d * 86_400 + m * 60;
        let secs = [t(0, 600), t(0, 900), t(1, 100), t(5, 0), t(5, 30), t(30, 0)];
        assert_eq!(episodes(&secs), vec![0..3, 3..5, 5..6]);
        assert!(episodes(&[]).is_empty());
        assert_eq!(minutes(&[60, 61, 119, 120, 121]), vec![1, 2]);
    }
}
