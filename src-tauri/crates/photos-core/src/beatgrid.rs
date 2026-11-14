//! Beats as a list of times (ML design §6). The Phase 6d detector read one tempo and one
//! phase for a whole track, so real music — which drifts, breathes and changes — read as no
//! beat at all (confidence 0.16 on a real Jamendo track). This tracks every beat with
//! dynamic programming over an onset strength, so the tempo may move and no error builds up.
//!
//! The onset strength comes from either front end — spectral flux (`audio::spectral_flux`)
//! or a learned model's beat activation (`photos-ml`) — at any frame rate.

use rusqlite::{params, Connection, OptionalExtension};

use crate::Result;

/// Below this a track has no beat worth cutting to. Midway between the corpus's beatless
/// cases and its weakest real rhythm (2026-09-22, `tests/beat_sync.rs` and the unit tests):
/// drone 0.03, xorshift noise 0.10 · tempo ramp 0.37, *What Is Love* (Jamendo 1204669,
/// refused at 0.16 by the Phase 6d detector) 0.60, click tracks 0.97–0.99.
pub const MIN_CONFIDENCE: f32 = 0.24;
const MIN_BPM: f32 = 60.0;
const MAX_BPM: f32 = 200.0;
/// How strongly the tracker holds to the prior period (Ellis 2007's "tightness").
const TIGHTNESS: f32 = 100.0;

#[derive(Debug, Clone, PartialEq)]
pub struct BeatGrid {
    pub beats_ms: Vec<u32>,
    /// Bar starts, when known; empty otherwise.
    pub downbeats_ms: Vec<u32>,
    pub bpm: f32,
    /// 0..1: how periodic the onsets are and how firmly they sit on the tracked beats.
    pub confidence: f32,
}

impl BeatGrid {
    pub fn none() -> Self {
        Self { beats_ms: Vec::new(), downbeats_ms: Vec::new(), bpm: 0.0, confidence: 0.0 }
    }

    pub fn is_steady(&self) -> bool {
        self.confidence >= MIN_CONFIDENCE && self.beats_ms.len() >= 8
    }

    /// A metronome: beats every `60 000 / bpm` ms from `first_ms` up to `until_ms`.
    pub fn even(bpm: f32, first_ms: u32, until_ms: u32) -> Self {
        let p = 60_000.0 / bpm as f64;
        let beats_ms = (0..).map(|k| (first_ms as f64 + k as f64 * p).round() as u32).take_while(|t| *t <= until_ms).collect();
        Self { beats_ms, downbeats_ms: Vec::new(), bpm, confidence: 1.0 }
    }

    /// The typical beat length in ms (the median interval).
    pub fn beat_ms(&self) -> f32 {
        let mut d: Vec<u32> = self.beats_ms.windows(2).map(|w| w[1] - w[0]).collect();
        if d.is_empty() {
            return if self.bpm > 0.0 { 60_000.0 / self.bpm } else { 0.0 };
        }
        d.sort_unstable();
        d[d.len() / 2] as f32
    }
}

fn mean_sd(v: &[f32]) -> (f32, f32) {
    let mean = v.iter().sum::<f32>() / v.len() as f32;
    (mean, (v.iter().map(|x| (x - mean).powi(2)).sum::<f32>() / v.len() as f32).sqrt())
}

/// Autocorrelation of the mean-removed signal at `lag`, as a fraction of its energy.
fn autocorrelation(c: &[f32], lag: usize) -> f32 {
    let energy: f32 = c.iter().map(|v| v * v).sum();
    if energy <= f32::EPSILON || lag >= c.len() {
        return 0.0;
    }
    c[lag..].iter().zip(c).map(|(a, b)| a * b).sum::<f32>() / energy
}

/// The prior period in frames: autocorrelation over 60–200 BPM, weighted towards 120 BPM by
/// a log-Gaussian one octave wide, with the peak refined between lags so the tempo is not
/// quantised to whole frames.
fn prior_period(centred: &[f32], frame_ms: f32) -> Option<f32> {
    let lag_of = |bpm: f32| 60_000.0 / bpm / frame_ms;
    let lo = (lag_of(MAX_BPM).floor() as usize).max(2);
    let hi = (lag_of(MIN_BPM).ceil() as usize).min(centred.len() / 2);
    if lo + 2 >= hi {
        return None;
    }
    let weighted = |lag: usize| {
        let bpm = 60_000.0 / (lag as f32 * frame_ms);
        autocorrelation(centred, lag) * (-0.5 * (bpm / 120.0).log2().powi(2)).exp()
    };
    let best = (lo..=hi).max_by(|a, b| weighted(*a).total_cmp(&weighted(*b)))?;
    let (y0, y1, y2) = (weighted(best - 1), weighted(best), weighted(best + 1));
    let denom = y0 - 2.0 * y1 + y2;
    let delta = if denom.abs() > f32::EPSILON { (0.5 * (y0 - y2) / denom).clamp(-0.5, 0.5) } else { 0.0 };
    Some(best as f32 + delta)
}

/// Every beat in `onset` (one value per `frame_ms`). `downbeat` is a model's downbeat
/// activation on the same frames; without one, `low_band` (bass onsets) lets the tracker
/// guess 4/4 bar starts, and only when one phase clearly wins.
pub fn track(onset: &[f32], frame_ms: f32, downbeat: Option<&[f32]>, low_band: Option<&[f32]>) -> BeatGrid {
    if onset.len() < 64 || frame_ms <= 0.0 {
        return BeatGrid::none();
    }
    let (mean, sd) = mean_sd(onset);
    if sd <= f32::EPSILON {
        return BeatGrid::none();
    }
    let centred: Vec<f32> = onset.iter().map(|v| v - mean).collect();
    let Some(period) = prior_period(&centred, frame_ms) else { return BeatGrid::none() };

    // The local score: onsets over their spread, lightly smoothed so a beat a frame early
    // or late still scores (a Gaussian of a thirty-second of the period).
    let sigma = (period / 32.0).max(0.5);
    let radius = (sigma * 3.0).ceil() as isize;
    let kernel: Vec<f32> = (-radius..=radius).map(|k| (-0.5 * (k as f32 / sigma).powi(2)).exp()).collect();
    let n = onset.len();
    let local: Vec<f32> = (0..n as isize)
        .map(|t| {
            (-radius..=radius)
                .map(|k| {
                    let i = t + k;
                    if i < 0 || i >= n as isize { 0.0 } else { onset[i as usize] / sd * kernel[(k + radius) as usize] }
                })
                .sum()
        })
        .collect();

    // C(t) = L(t) + max over τ ∈ [t − 2P, t − P/2] of C(τ) − α·ln((t − τ)/P)²
    let mut score = vec![0f32; n];
    let mut back = vec![usize::MAX; n];
    let near = (period / 2.0).round().max(1.0) as usize;
    let far = (period * 2.0).round() as usize;
    for t in 0..n {
        let mut best = (f32::MIN, usize::MAX);
        if t >= near {
            let from = t.saturating_sub(far);
            for (k, prev) in score[from..=t - near].iter().enumerate() {
                let tau = from + k;
                let v = prev - TIGHTNESS * ((t - tau) as f32 / period).ln().powi(2);
                if v > best.0 {
                    best = (v, tau);
                }
            }
        }
        if best.1 == usize::MAX {
            score[t] = local[t];
        } else {
            score[t] = local[t] + best.0;
            back[t] = best.1;
        }
    }
    let tail = n.saturating_sub(period.round() as usize);
    let mut t = (tail..n).max_by(|a, b| score[*a].total_cmp(&score[*b])).unwrap_or(n - 1);
    let mut frames = vec![t];
    while back[t] != usize::MAX {
        t = back[t];
        frames.push(t);
    }
    frames.reverse();
    // The chain starts wherever the first period allowed; drop leading beats that sit on
    // nothing, so a quiet intro does not start the show early.
    let floor = mean;
    while frames.len() > 1 && onset[frames[0]] < floor {
        frames.remove(0);
    }

    let ms = |f: usize| (f as f32 * frame_ms).round() as u32;
    let beats_ms: Vec<u32> = frames.iter().map(|f| ms(*f)).collect();

    // Confidence needs both: onsets that repeat at the tracked period, and beats that stand
    // out from the track. The geometric mean, because either alone is fooled — noise has
    // local maxima to call beats, and a pure drone's near-zero flux makes any "beat" look
    // infinitely louder than the average.
    let periodicity = autocorrelation(&centred, period.round() as usize).clamp(0.0, 1.0);
    let on_beats = frames.iter().map(|f| onset[*f]).sum::<f32>() / frames.len() as f32;
    let ratio = if mean > f32::EPSILON { on_beats / mean } else { 0.0 };
    let confidence = (periodicity * ((ratio - 1.0) / 1.5).clamp(0.0, 1.0)).sqrt();

    let near_max = |act: &[f32], f: usize| act[f.saturating_sub(2)..(f + 3).min(act.len())].iter().copied().fold(0.0, f32::max);
    let downbeats_ms = match (downbeat, low_band) {
        (Some(act), _) => frames.iter().filter(|f| near_max(act, **f) >= 0.5).map(|f| ms(*f)).collect(),
        (None, Some(low)) => {
            let phase_score = |k: usize| frames.iter().skip(k).step_by(4).map(|f| low.get(*f).copied().unwrap_or(0.0)).sum::<f32>();
            let mut s: Vec<(usize, f32)> = (0..4).map(|k| (k, phase_score(k))).collect();
            s.sort_by(|a, b| b.1.total_cmp(&a.1));
            if s[0].1 > 1.2 * s[1].1 {
                frames.iter().skip(s[0].0).step_by(4).map(|f| ms(*f)).collect()
            } else {
                Vec::new()
            }
        }
        _ => Vec::new(),
    };

    BeatGrid { bpm: fitted_bpm(&beats_ms), beats_ms, downbeats_ms, confidence }
}

/// The tempo as the slope of beat time against beat index (least squares). Beat times are
/// quantised to frames; the fit averages that away, where a median interval cannot.
fn fitted_bpm(beats_ms: &[u32]) -> f32 {
    let n = beats_ms.len();
    if n < 2 {
        return 0.0;
    }
    let (mx, my) = ((n - 1) as f64 / 2.0, beats_ms.iter().map(|b| *b as f64).sum::<f64>() / n as f64);
    let (mut sxy, mut sxx) = (0f64, 0f64);
    for (i, b) in beats_ms.iter().enumerate() {
        let dx = i as f64 - mx;
        sxy += dx * (*b as f64 - my);
        sxx += dx * dx;
    }
    let slope = sxy / sxx;
    if slope > 0.0 { (60_000.0 / slope) as f32 } else { 0.0 }
}

/// How busy a track is: onset peaks above mean + 1σ, per second.
pub fn energy(onset: &[f32], frame_ms: f32) -> f32 {
    if onset.len() < 3 || frame_ms <= 0.0 {
        return 0.0;
    }
    let (mean, sd) = mean_sd(onset);
    let peaks = (1..onset.len() - 1).filter(|i| onset[*i] > mean + sd && onset[*i] >= onset[i - 1] && onset[*i] > onset[i + 1]).count();
    peaks as f32 / (onset.len() as f32 * frame_ms / 1000.0)
}

pub fn to_blob(ms: &[u32]) -> Vec<u8> {
    ms.iter().flat_map(|v| v.to_le_bytes()).collect()
}

pub fn from_blob(b: &[u8]) -> Vec<u32> {
    b.as_chunks::<4>().0.iter().map(|c| u32::from_le_bytes(*c)).collect()
}

/// A track's beats as the catalogue caches them, keyed by its file name in the library's
/// music folder and invalidated when the file's size or modification time changes.
#[derive(Debug, Clone, PartialEq)]
pub struct TrackAnalysis {
    pub file_name: String,
    pub byte_size: i64,
    pub modified: i64,
    pub analyzer: String,
    pub grid: BeatGrid,
    pub energy: f32,
}

pub fn get_analysis(conn: &Connection, file_name: &str) -> Result<Option<TrackAnalysis>> {
    Ok(conn
        .query_row(
            "SELECT byte_size, modified, analyzer, bpm, confidence, beats, downbeats, energy FROM track_analysis WHERE file_name = ?1",
            [file_name],
            |r| {
                Ok(TrackAnalysis {
                    file_name: file_name.to_string(),
                    byte_size: r.get(0)?,
                    modified: r.get(1)?,
                    analyzer: r.get(2)?,
                    grid: BeatGrid {
                        bpm: r.get::<_, f64>(3)? as f32,
                        confidence: r.get::<_, f64>(4)? as f32,
                        beats_ms: from_blob(&r.get::<_, Vec<u8>>(5)?),
                        downbeats_ms: from_blob(&r.get::<_, Vec<u8>>(6)?),
                    },
                    energy: r.get::<_, f64>(7)? as f32,
                })
            },
        )
        .optional()?)
}

pub fn put_analysis(conn: &Connection, a: &TrackAnalysis) -> Result<()> {
    conn.execute(
        "INSERT INTO track_analysis (file_name, byte_size, modified, analyzer, bpm, confidence, beats, downbeats, energy)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
         ON CONFLICT(file_name) DO UPDATE SET byte_size = excluded.byte_size, modified = excluded.modified,
           analyzer = excluded.analyzer, bpm = excluded.bpm, confidence = excluded.confidence, beats = excluded.beats,
           downbeats = excluded.downbeats, energy = excluded.energy",
        params![
            a.file_name,
            a.byte_size,
            a.modified,
            a.analyzer,
            a.grid.bpm as f64,
            a.grid.confidence as f64,
            to_blob(&a.grid.beats_ms),
            to_blob(&a.grid.downbeats_ms),
            a.energy as f64
        ],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An onset train: a spike at each of `beats_ms` on a faint noise floor, 10 ms frames.
    fn train(beats_ms: &[f32], len_ms: f32) -> Vec<f32> {
        let n = (len_ms / 10.0) as usize;
        let mut o: Vec<f32> = (0..n).map(|i| 0.05 * ((i * 7919 % 13) as f32 / 13.0)).collect();
        for b in beats_ms {
            let i = (b / 10.0).round() as usize;
            if i < n {
                o[i] += 1.0;
                if i + 1 < n {
                    o[i + 1] += 0.4;
                }
            }
        }
        o
    }

    fn steady(bpm: f32, from: f32, len_ms: f32) -> Vec<f32> {
        let p = 60_000.0 / bpm;
        (0..).map(|k| from + k as f32 * p).take_while(|t| *t < len_ms).collect()
    }

    #[test]
    fn reads_a_steady_tempo_to_a_fraction_of_a_bpm() {
        for bpm in [90.0f32, 120.0, 140.0] {
            let g = track(&train(&steady(bpm, 250.0, 60_000.0), 60_000.0), 10.0, None, None);
            assert!((g.bpm - bpm).abs() < 0.3, "{bpm}: read {}", g.bpm);
            assert!(g.is_steady(), "{bpm}: confidence {}", g.confidence);
        }
    }

    /// The failure of the Phase 6d detector: one tempo for the whole track. The DP tracker
    /// must follow a tempo that moves, so every beat of a ramp is found where it is.
    #[test]
    fn follows_a_tempo_ramp_beat_by_beat() {
        let mut beats = vec![500.0f32];
        while *beats.last().unwrap() < 60_000.0 {
            let t = *beats.last().unwrap();
            let bpm = 100.0 + 25.0 * (t / 60_000.0);
            beats.push(t + 60_000.0 / bpm);
        }
        let g = track(&train(&beats, 61_000.0), 10.0, None, None);
        let found: Vec<f32> = g.beats_ms.iter().map(|b| *b as f32).collect();
        let inner = &beats[4..beats.len() - 4];
        let worst = inner.iter().map(|b| found.iter().map(|f| (f - b).abs()).fold(f32::MAX, f32::min)).fold(0.0, f32::max);
        assert!(worst <= 30.0, "a beat was missed by {worst} ms");
    }

    #[test]
    fn noise_has_no_beat() {
        // xorshift: a multiplicative sequence (i·k mod m) is a lattice, and genuinely periodic.
        let mut s = 0x9E37_79B9_7F4A_7C15u64;
        let noise: Vec<f32> = (0..6000)
            .map(|_| {
                s ^= s << 13;
                s ^= s >> 7;
                s ^= s << 17;
                (s % 10_000) as f32 / 10_000.0
            })
            .collect();
        println!("noise confidence {}", track(&noise, 10.0, None, None).confidence);
        assert!(!track(&noise, 10.0, None, None).is_steady());
        assert!(!track(&[], 10.0, None, None).is_steady());
    }

    #[test]
    fn downbeats_come_from_the_model_when_it_gives_them() {
        let beats = steady(120.0, 0.0, 20_000.0);
        let onset = train(&beats, 20_000.0);
        let mut down = vec![0f32; onset.len()];
        for b in beats.iter().step_by(4) {
            down[(b / 10.0) as usize] = 0.9;
        }
        let g = track(&onset, 10.0, Some(&down), None);
        assert!(!g.downbeats_ms.is_empty());
        for d in &g.downbeats_ms {
            let bar = 2000.0;
            let off = (*d as f32 % bar).min(bar - *d as f32 % bar);
            assert!(off <= 20.0, "downbeat at {d} is not on a bar line");
        }
    }

    #[test]
    fn an_even_grid_and_the_blob_codec_round_trip() {
        let g = BeatGrid::even(120.0, 250, 2_300);
        assert_eq!(g.beats_ms, vec![250, 750, 1250, 1750, 2250]);
        assert_eq!(from_blob(&to_blob(&g.beats_ms)), g.beats_ms);
    }

    #[test]
    fn energy_counts_onsets_per_second() {
        let busy = train(&steady(240.0, 0.0, 10_000.0), 10_000.0);
        let calm = train(&steady(60.0, 0.0, 10_000.0), 10_000.0);
        assert!(energy(&busy, 10.0) > 3.0 * energy(&calm, 10.0));
        assert_eq!(energy(&[], 10.0), 0.0);
    }

    #[test]
    fn analyses_round_trip_through_the_catalogue() {
        let db = crate::Db::open_in_memory().unwrap();
        db.with_conn(|c| {
            let a = TrackAnalysis {
                file_name: "1204669.mp3".into(),
                byte_size: 5_376_911,
                modified: 1_789_000_000,
                analyzer: "dsp@1".into(),
                grid: BeatGrid { beats_ms: vec![100, 760, 1420], downbeats_ms: vec![100], bpm: 90.9, confidence: 0.6 },
                energy: 2.5,
            };
            put_analysis(c, &a)?;
            let back = get_analysis(c, "1204669.mp3")?.unwrap();
            assert_eq!(back.grid.beats_ms, a.grid.beats_ms);
            assert_eq!(back.grid.downbeats_ms, a.grid.downbeats_ms);
            assert!((back.grid.bpm - 90.9).abs() < 1e-4);
            assert!(get_analysis(c, "other.mp3")?.is_none());
            Ok(())
        })
        .unwrap();
    }
}
