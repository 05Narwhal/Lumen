//! What a track sounds like (Round 6), measured rather than read from its tags, and which
//! part of it a show should use.
//!
//! The automatic music choice used to match a mood tag and nothing else. Every result of a
//! *calm* search carries *calm*, so the candidates tied and a hash decided — it sounded
//! random because it was. A profile gives each candidate numbers that differ: how fast, how
//! busy, how bright, how loud and how much that loudness moves. Its loudness curve also finds
//! the song's sections, so a show can start where the song does something rather than on a
//! slow intro.

use rusqlite::{params, Connection, OptionalExtension};
use rustfft::{num_complex::Complex, FftPlanner};
use serde::{Deserialize, Serialize};

use crate::beatgrid::BeatGrid;
use crate::Result;

/// Bump when what a profile measures changes; older cached profiles are measured again.
pub const PROFILE_VERSION: u32 = 1;
/// One point of the loudness and brightness curves.
pub const CURVE_MS: u32 = 500;
/// Quieter than this is silence: an intro's breath, a gap between songs on a recording.
pub const SILENCE_DB: f32 = -35.0;
/// A section is at least this long; anything shorter is a fill, not a part of the song.
const MIN_SECTION_MS: u32 = 8_000;
/// How far either side of a point the novelty compares (4 s).
const NOVELTY_HALF: usize = 8;
/// How far a boundary may move to land on a downbeat (or a beat).
const SNAP_MS: u32 = 1_500;
/// The analysis window for brightness, at 22 050 Hz about 93 ms.
const FFT_SIZE: usize = 2048;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Section {
    pub start_ms: u32,
    pub end_ms: u32,
    pub loudness_db: f32,
}

/// A track's measured sound. Every 0..1 value means the same for every track, so two tracks
/// compare and a mood's target can be written down.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TrackProfile {
    pub version: u32,
    pub duration_ms: u32,
    /// Beats per minute, 0 when there is no steady beat.
    pub tempo: f32,
    /// Onsets per second (`beatgrid::energy`).
    pub energy: f32,
    /// 0..1: the spectral centroid on a log scale from 250 Hz to 4 kHz, over the sound only.
    pub brightness: f32,
    /// The mean level in dBFS of the parts that are not silence.
    pub loudness_db: f32,
    /// The spread of the loudness curve in dB: a pad sits still, a song with a chorus moves.
    pub dynamics: f32,
    /// dBFS every `CURVE_MS`.
    pub curve: Vec<f32>,
    /// 0..1 brightness every `CURVE_MS`.
    pub brightness_curve: Vec<f32>,
    pub sections: Vec<Section>,
}

impl TrackProfile {
    /// 0..1 tempo, 60 → 0 and 180 → 1; unknown is the middle.
    pub fn tempo_norm(&self) -> f32 {
        if self.tempo <= 0.0 {
            0.5
        } else {
            ((self.tempo - 60.0) / 120.0).clamp(0.0, 1.0)
        }
    }

    /// 0..1 energy: the beat analysis's own scale (onsets per second over 6).
    pub fn energy_norm(&self) -> f32 {
        (self.energy / 6.0).clamp(0.0, 1.0)
    }

    /// 0..1 dynamics: 12 dB of spread is a lot for mastered music.
    pub fn dynamics_norm(&self) -> f32 {
        (self.dynamics / 12.0).clamp(0.0, 1.0)
    }
}

fn db(rms: f32) -> f32 {
    20.0 * rms.max(1e-6).log10()
}

fn brightness_of(centroid_hz: f32) -> f32 {
    ((centroid_hz.max(1.0) / 250.0).log2() / (4000.0f32 / 250.0).log2()).clamp(0.0, 1.0)
}

fn mean(v: &[f32]) -> f32 {
    if v.is_empty() {
        0.0
    } else {
        v.iter().sum::<f32>() / v.len() as f32
    }
}

/// Measures `samples` (mono, −1..1). The beats and how busy the track is come from the beat
/// analysis the show already runs, so a track is decoded once for both.
pub fn profile(samples: &[f32], sample_rate: u32, grid: &BeatGrid, energy: f32) -> TrackProfile {
    let block = (sample_rate as u64 * CURVE_MS as u64 / 1000).max(1) as usize;
    let duration_ms = (samples.len() as u64 * 1000 / sample_rate.max(1) as u64) as u32;
    let mut planner = FftPlanner::<f32>::new();
    let fft = planner.plan_fft_forward(FFT_SIZE);
    let window: Vec<f32> = (0..FFT_SIZE).map(|i| 0.5 - 0.5 * (2.0 * std::f32::consts::PI * i as f32 / FFT_SIZE as f32).cos()).collect();
    let bin_hz = sample_rate as f32 / FFT_SIZE as f32;
    let mut curve = Vec::new();
    let mut brightness_curve = Vec::new();
    let mut buf = vec![Complex::new(0.0f32, 0.0); FFT_SIZE];
    for chunk in samples.chunks(block) {
        if chunk.len() < block / 2 && !curve.is_empty() {
            break;
        }
        let rms = (chunk.iter().map(|s| s * s).sum::<f32>() / chunk.len() as f32).sqrt();
        curve.push(db(rms));
        // The centroid of the block's middle, which is enough for a 0.5 s average.
        let mid = chunk.len() / 2;
        let start = mid.saturating_sub(FFT_SIZE / 2);
        for (k, slot) in buf.iter_mut().enumerate() {
            let s = chunk.get(start + k).copied().unwrap_or(0.0);
            *slot = Complex::new(s * window[k], 0.0);
        }
        fft.process(&mut buf);
        let (mut num, mut den) = (0f32, 0f32);
        for (k, c) in buf[1..FFT_SIZE / 2].iter().enumerate() {
            let m = c.norm();
            num += (k + 1) as f32 * bin_hz * m;
            den += m;
        }
        brightness_curve.push(if den > 0.0 { brightness_of(num / den) } else { 0.0 });
    }
    let sounding: Vec<usize> = (0..curve.len()).filter(|&i| curve[i] > SILENCE_DB).collect();
    let loud: Vec<f32> = sounding.iter().map(|&i| curve[i]).collect();
    let loudness_db = if loud.is_empty() { SILENCE_DB * 2.0 } else { mean(&loud) };
    let dynamics = if loud.len() < 2 {
        0.0
    } else {
        (loud.iter().map(|v| (v - loudness_db).powi(2)).sum::<f32>() / loud.len() as f32).sqrt()
    };
    let brightness = mean(&sounding.iter().map(|&i| brightness_curve[i]).collect::<Vec<_>>());
    let sections = sections_of(&curve, &brightness_curve, grid, duration_ms);
    TrackProfile {
        version: PROFILE_VERSION,
        duration_ms,
        tempo: if grid.is_steady() { grid.bpm } else { 0.0 },
        energy,
        brightness,
        loudness_db,
        dynamics,
        curve,
        brightness_curve,
        sections,
    }
}

/// Where the song changes: points whose following 4 s differ most from the 4 s before them
/// in loudness and brightness, at least `MIN_SECTION_MS` apart, each moved onto the nearest
/// downbeat (a beat when there are no downbeats) within `SNAP_MS`.
fn sections_of(curve: &[f32], bright: &[f32], grid: &BeatGrid, duration_ms: u32) -> Vec<Section> {
    let n = curve.len();
    // Loudness on a 0..1 scale (−60..0 dB) beside brightness, so neither drowns the other.
    let feat: Vec<(f32, f32)> = (0..n).map(|i| (((curve[i] + 60.0) / 60.0).clamp(0.0, 1.0), bright[i])).collect();
    let novelty: Vec<f32> = (0..n)
        .map(|i| {
            if i < NOVELTY_HALF || i + NOVELTY_HALF > n {
                return 0.0;
            }
            let before = &feat[i - NOVELTY_HALF..i];
            let after = &feat[i..i + NOVELTY_HALF];
            let m = |s: &[(f32, f32)], f: fn(&(f32, f32)) -> f32| s.iter().map(f).sum::<f32>() / s.len() as f32;
            (m(after, |p| p.0) - m(before, |p| p.0)).abs() + 0.5 * (m(after, |p| p.1) - m(before, |p| p.1)).abs()
        })
        .collect();
    let mut sorted: Vec<f32> = novelty.iter().copied().filter(|v| *v > 0.0).collect();
    sorted.sort_by(f32::total_cmp);
    let median = sorted.get(sorted.len() / 2).copied().unwrap_or(0.0);
    let floor = (median * 1.5).max(0.05);
    let min_gap = (MIN_SECTION_MS / CURVE_MS) as usize;
    // Strongest first, so a weak wobble never blocks the real change beside it.
    let mut peaks: Vec<usize> = (0..n)
        .filter(|&i| novelty[i] >= floor && (i.saturating_sub(4)..(i + 5).min(n)).all(|j| novelty[j] <= novelty[i]))
        .collect();
    peaks.sort_by(|a, b| novelty[*b].total_cmp(&novelty[*a]));
    let mut chosen: Vec<usize> = Vec::new();
    for p in peaks {
        if p >= min_gap && n - p >= min_gap && chosen.iter().all(|c| c.abs_diff(p) >= min_gap) {
            chosen.push(p);
        }
    }
    chosen.sort_unstable();
    let marks = if grid.downbeats_ms.len() >= 2 { &grid.downbeats_ms } else { &grid.beats_ms };
    let snap = |ms: u32| marks.iter().copied().filter(|m| m.abs_diff(ms) <= SNAP_MS).min_by_key(|m| m.abs_diff(ms)).unwrap_or(ms);
    let mut bounds: Vec<u32> = vec![0];
    bounds.extend(chosen.iter().map(|&i| snap(i as u32 * CURVE_MS)));
    bounds.push(duration_ms);
    bounds.dedup();
    bounds
        .windows(2)
        .filter(|w| w[1] > w[0])
        .map(|w| {
            let (a, b) = ((w[0] / CURVE_MS) as usize, ((w[1] / CURVE_MS) as usize).min(n));
            Section { start_ms: w[0], end_ms: w[1], loudness_db: if a < b { mean(&curve[a..b]) } else { SILENCE_DB * 2.0 } }
        })
        .collect()
}

/// Where in the track a show plays from, and how it fades.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export)]
pub struct MusicWindow {
    pub start_ms: u32,
    pub fade_in_ms: u32,
    pub fade_out_ms: u32,
}

/// A track that plays from the top fades in over a second, as before Round 6.
pub const FADE_IN_TOP_MS: u32 = 1_000;
/// Starting mid-song on a section: a short fade, only so the first sample is not a click.
pub const FADE_IN_MID_MS: u32 = 400;
/// The show ends where the song does (a boundary or its end): a short fade.
pub const FADE_OUT_ON_BOUNDARY_MS: u32 = 1_000;
/// The show ends mid-phrase: a long fade, so the music leaves rather than stops.
pub const FADE_OUT_MID_MS: u32 = 2_500;
/// How near a boundary the show's end must be to count as ending with the song.
const END_NEAR_MS: u32 = 2_000;

/// The part of the track a show of `target_ms` uses (spec §2). Every start considered is the
/// top of the track or the top of a section, never in silence; of those, the window that is
/// loudest on average and holds most of the song's strongest section wins, a little more if it
/// ends where the song does, and the top of the track is preferred when it is as good — a song
/// that works from its beginning plays from its beginning.
pub fn choose_window(p: &TrackProfile, target_ms: u32) -> MusicWindow {
    let top = MusicWindow { start_ms: 0, fade_in_ms: FADE_IN_TOP_MS, fade_out_ms: FADE_OUT_MID_MS };
    if p.duration_ms <= target_ms.saturating_add(END_NEAR_MS) || p.curve.is_empty() {
        let ends_with_song = p.duration_ms <= target_ms.saturating_add(END_NEAR_MS);
        return MusicWindow { fade_out_ms: if ends_with_song { FADE_OUT_ON_BOUNDARY_MS } else { FADE_OUT_MID_MS }, ..top };
    }
    let level = |ms: u32| p.curve.get((ms / CURVE_MS) as usize).copied().unwrap_or(SILENCE_DB * 2.0);
    let peak = p.sections.iter().max_by(|a, b| a.loudness_db.total_cmp(&b.loudness_db));
    let ends: Vec<u32> = p.sections.iter().map(|s| s.end_ms).collect();
    let mut starts: Vec<u32> = std::iter::once(0).chain(p.sections.iter().map(|s| s.start_ms)).collect();
    starts.dedup();
    let latest = p.duration_ms - target_ms;
    let mut best: Option<(f32, MusicWindow)> = None;
    for &start in starts.iter().filter(|s| **s <= latest) {
        // Never start in silence: a section that opens on a breath is not where to begin.
        if start > 0 && level(start) <= SILENCE_DB {
            continue;
        }
        let end = start + target_ms;
        let (a, b) = ((start / CURVE_MS) as usize, ((end / CURVE_MS) as usize).min(p.curve.len()));
        let loudness = ((mean(&p.curve[a..b.max(a + 1).min(p.curve.len())]) + 60.0) / 60.0).clamp(0.0, 1.0);
        let holds_peak = peak.map_or(0.0, |s| {
            let covered = end.min(s.end_ms).saturating_sub(start.max(s.start_ms)) as f32;
            covered / (s.end_ms - s.start_ms).max(1) as f32
        });
        let on_boundary = ends.iter().any(|e| e.abs_diff(end) <= END_NEAR_MS);
        let score = loudness + 0.5 * holds_peak + if on_boundary { 0.15 } else { 0.0 } + if start == 0 { 0.05 } else { 0.0 };
        let window = MusicWindow {
            start_ms: start,
            fade_in_ms: if start == 0 { FADE_IN_TOP_MS } else { FADE_IN_MID_MS },
            fade_out_ms: if on_boundary { FADE_OUT_ON_BOUNDARY_MS } else { FADE_OUT_MID_MS },
        };
        if best.as_ref().is_none_or(|(s, _)| score > *s + 1e-6) {
            best = Some((score, window));
        }
    }
    best.map(|(_, w)| w).unwrap_or(top)
}

/// A saved track's profile from the catalogue, when its file has not changed since.
pub fn get_cached(conn: &Connection, file_name: &str, byte_size: i64, modified: i64) -> Result<Option<TrackProfile>> {
    let row: Option<(i64, i64, u32, String)> = conn
        .query_row(
            "SELECT byte_size, modified, version, profile_json FROM track_profiles WHERE file_name = ?1",
            [file_name],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .optional()?;
    Ok(row
        .filter(|(size, m, v, _)| *size == byte_size && *m == modified && *v == PROFILE_VERSION)
        .and_then(|(_, _, _, json)| serde_json::from_str(&json).ok()))
}

pub fn put_cached(conn: &Connection, file_name: &str, byte_size: i64, modified: i64, p: &TrackProfile) -> Result<()> {
    conn.execute(
        "INSERT INTO track_profiles (file_name, byte_size, modified, version, profile_json) VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(file_name) DO UPDATE SET byte_size = excluded.byte_size, modified = excluded.modified,
           version = excluded.version, profile_json = excluded.profile_json",
        params![file_name, byte_size, modified, p.version, serde_json::to_string(p)?],
    )?;
    Ok(())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub const SR: u32 = 22_050;

    /// A tone at `hz` and `amp` for `secs`.
    pub fn tone(hz: f32, amp: f32, secs: f32) -> Vec<f32> {
        (0..(secs * SR as f32) as usize).map(|i| amp * (2.0 * std::f32::consts::PI * hz * i as f32 / SR as f32).sin()).collect()
    }

    /// A song: a quiet dark intro, a loud bright chorus, a quiet outro.
    fn song() -> Vec<f32> {
        let mut y = tone(220.0, 0.05, 20.0);
        y.extend(tone(1760.0, 0.6, 20.0));
        y.extend(tone(220.0, 0.08, 20.0));
        y
    }

    #[test]
    fn a_song_is_split_where_it_changes() {
        let p = profile(&song(), SR, &BeatGrid::none(), 2.0);
        assert_eq!(p.duration_ms, 60_000);
        assert_eq!(p.sections.len(), 3, "{:?}", p.sections);
        assert!(p.sections[1].start_ms.abs_diff(20_000) <= 1_000, "{:?}", p.sections);
        assert!(p.sections[2].start_ms.abs_diff(40_000) <= 1_000, "{:?}", p.sections);
        assert!(p.sections[1].loudness_db > p.sections[0].loudness_db + 10.0);
    }

    #[test]
    fn boundaries_land_on_downbeats() {
        let mut grid = BeatGrid::even(120.0, 0, 60_000);
        grid.downbeats_ms = (0..30).map(|k| 300 + k * 2_000).collect();
        let p = profile(&song(), SR, &grid, 2.0);
        for s in &p.sections[1..] {
            assert!(grid.downbeats_ms.contains(&s.start_ms), "{} not a downbeat", s.start_ms);
        }
    }

    #[test]
    fn a_bright_tone_is_brighter_than_a_dark_one() {
        let bright = profile(&tone(3000.0, 0.3, 5.0), SR, &BeatGrid::none(), 1.0);
        let dark = profile(&tone(200.0, 0.3, 5.0), SR, &BeatGrid::none(), 1.0);
        assert!(bright.brightness > dark.brightness + 0.5, "{} vs {}", bright.brightness, dark.brightness);
    }

    #[test]
    fn a_song_with_a_chorus_is_more_dynamic_than_a_drone() {
        let song = profile(&song(), SR, &BeatGrid::none(), 1.0);
        let drone = profile(&tone(440.0, 0.3, 60.0), SR, &BeatGrid::none(), 1.0);
        assert!(song.dynamics > drone.dynamics + 5.0, "{} vs {}", song.dynamics, drone.dynamics);
        assert!(drone.dynamics < 0.5);
    }

    #[test]
    fn a_short_show_plays_the_chorus() {
        let p = profile(&song(), SR, &BeatGrid::none(), 2.0);
        let w = choose_window(&p, 18_000);
        assert!(w.start_ms.abs_diff(20_000) <= 1_000, "{w:?}");
        assert_eq!(w.fade_in_ms, FADE_IN_MID_MS);
    }

    #[test]
    fn a_window_ending_with_its_section_fades_briefly() {
        let p = profile(&song(), SR, &BeatGrid::none(), 2.0);
        let w = choose_window(&p, 20_000);
        assert_eq!(w.fade_out_ms, FADE_OUT_ON_BOUNDARY_MS, "{w:?} {:?}", p.sections);
    }

    #[test]
    fn a_show_as_long_as_the_song_plays_it_from_the_top() {
        let p = profile(&song(), SR, &BeatGrid::none(), 2.0);
        assert_eq!(choose_window(&p, 59_000).start_ms, 0);
        assert_eq!(choose_window(&p, 90_000).start_ms, 0);
    }

    #[test]
    fn an_even_song_plays_from_the_top() {
        let p = profile(&tone(440.0, 0.3, 120.0), SR, &BeatGrid::none(), 1.0);
        let w = choose_window(&p, 30_000);
        assert_eq!(w.start_ms, 0);
        assert_eq!(w.fade_in_ms, FADE_IN_TOP_MS);
    }

    #[test]
    fn a_window_never_starts_in_silence() {
        let mut y = tone(440.0, 0.3, 20.0);
        y.extend(vec![0.0; 10 * SR as usize]);
        y.extend(tone(440.0, 0.3, 30.0));
        let p = profile(&y, SR, &BeatGrid::none(), 1.0);
        let w = choose_window(&p, 15_000);
        assert!(p.curve[(w.start_ms / CURVE_MS) as usize] > SILENCE_DB, "{w:?} {:?}", p.sections);
    }

    #[test]
    fn a_cached_profile_is_returned_only_for_the_same_file() {
        let db = crate::Db::open_in_memory().unwrap();
        let p = profile(&tone(440.0, 0.3, 3.0), SR, &BeatGrid::none(), 1.0);
        db.with_conn(|c| put_cached(c, "1.mp3", 100, 5, &p)).unwrap();
        assert_eq!(db.with_conn(|c| get_cached(c, "1.mp3", 100, 5)).unwrap(), Some(p.clone()));
        assert_eq!(db.with_conn(|c| get_cached(c, "1.mp3", 101, 5)).unwrap(), None);
        assert_eq!(db.with_conn(|c| get_cached(c, "2.mp3", 100, 5)).unwrap(), None);
    }
}
