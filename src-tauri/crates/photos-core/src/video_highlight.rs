//! The best few seconds of a video, for a slideshow (Round 6).
//!
//! A video is sampled about twice a second (`photos_media::video_frames`); each sample gets a
//! score from what the frame shows — sharp, well exposed, attractive, with people in it,
//! moving the right amount — and the window of `HIGHLIGHT_MIN_MS`–`HIGHLIGHT_MAX_MS` with the
//! best average is the part a show uses. A scene cut is a wall: a highlight never spans two
//! shots, which would read as a mistake. The ML terms (faces, aesthetic) are optional; without
//! them the remaining weights are renormalised, so a library with no models still gets a
//! sensible highlight.

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::Result;

/// A highlight is at least this long: shorter is a flicker in a slideshow.
pub const HIGHLIGHT_MIN_MS: u32 = 3_000;
/// And at most this long: longer and the video outstays the photographs around it.
pub const HIGHLIGHT_MAX_MS: u32 = 6_000;
/// The first and last moments of a clip are the phone being raised and lowered.
pub const EDGE_MS: u32 = 300;
/// A sample's histogram this far from the one before is a new shot.
pub const CUT_DISTANCE: f32 = 0.5;

/// What one sample of a video shows.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Signal {
    pub t_ms: u32,
    /// Variance of the Laplacian on the sample's luma: higher is sharper. Only comparable
    /// within one video, so it is used as a rank.
    pub sharpness: f32,
    /// Mean luma, 0..1.
    pub luma: f32,
    /// The share of pixels crushed to black or blown to white, 0..1.
    pub clipped: f32,
    /// Mean absolute difference from the previous sample, 0..1.
    pub motion: f32,
    /// A new shot starts at this sample.
    pub cut: bool,
    /// The sound's level around this sample in dBFS, when the video has sound.
    pub loudness: Option<f32>,
    /// Faces found, when the face model ran.
    pub faces: Option<u32>,
    /// The CLIP aesthetic score, 0..1, when the model ran on this sample.
    pub aesthetic: Option<f32>,
}

/// The part of a video a slideshow plays, and how good it is (0..1).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Highlight {
    pub in_ms: u32,
    pub out_ms: u32,
    pub score: f32,
}

/// Ranks in 0..1 where equal values share a rank; `None`s stay `None`.
fn ranks(values: &[Option<f32>]) -> Vec<Option<f32>> {
    let mut known: Vec<f32> = values.iter().flatten().copied().collect();
    known.sort_by(f32::total_cmp);
    known.dedup();
    let top = (known.len().max(2) - 1) as f32;
    values
        .iter()
        .map(|v| v.map(|v| if known.len() < 2 { 0.5 } else { known.partition_point(|k| *k < v) as f32 / top }))
        .collect()
}

/// Some motion is life — a wave, a turn, the sea. None is a photograph; a lot is shake.
fn motion_fit(m: f32) -> f32 {
    if m < 0.004 {
        return 0.5;
    }
    (1.0 - (m / 0.04).log10().abs()).clamp(0.0, 1.0)
}

fn exposure_fit(luma: f32, clipped: f32) -> f32 {
    ((1.0 - (luma - 0.45).abs() * 2.0) - clipped * 2.0).clamp(0.0, 1.0)
}

fn faces_fit(n: u32) -> f32 {
    match n {
        0 => 0.0,
        1 => 0.7,
        _ => 1.0,
    }
}

/// Every sample's score, 0..1: a weighted mean of the terms that sample has.
pub fn scores(signals: &[Signal]) -> Vec<f32> {
    let sharp = ranks(&signals.iter().map(|s| Some(s.sharpness)).collect::<Vec<_>>());
    let loud = ranks(&signals.iter().map(|s| s.loudness).collect::<Vec<_>>());
    signals
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let terms = [
                (0.30, sharp[i]),
                (0.20, Some(exposure_fit(s.luma, s.clipped))),
                (0.20, s.aesthetic),
                (0.15, s.faces.map(faces_fit)),
                (0.10, Some(motion_fit(s.motion))),
                (0.05, loud[i]),
            ];
            let (sum, weight) = terms.iter().filter_map(|(w, v)| v.map(|v| (w * v, *w))).fold((0.0, 0.0), |a, b| (a.0 + b.0, a.1 + b.1));
            if weight > 0.0 {
                sum / weight
            } else {
                0.0
            }
        })
        .collect()
}

/// The ranges a highlight may come from: the kept segments of the video's trim (the whole
/// video when it has none), less the edges, split at every cut.
fn ranges(signals: &[Signal], duration_ms: u32, keep: &[(u32, u32)]) -> Vec<(u32, u32)> {
    let kept: Vec<(u32, u32)> = if keep.is_empty() { vec![(0, duration_ms)] } else { keep.to_vec() };
    let cuts: Vec<u32> = signals.iter().filter(|s| s.cut).map(|s| s.t_ms).collect();
    let mut out = Vec::new();
    for (a, b) in kept {
        let (a, b) = (a.min(duration_ms), b.min(duration_ms));
        // Edges come off only when there is room for them to.
        let (a, b) = if b.saturating_sub(a) > 2 * EDGE_MS + HIGHLIGHT_MIN_MS { (a + EDGE_MS, b - EDGE_MS) } else { (a, b) };
        let mut start = a;
        for &c in cuts.iter().filter(|c| **c > a && **c < b) {
            out.push((start, c));
            start = c;
        }
        out.push((start, b));
    }
    out.retain(|(a, b)| b > a);
    out
}

/// The best `min_ms`–`max_ms` of a video `duration_ms` long, from its samples (spec §3). `keep`
/// is its trim's kept segments, empty when it is not trimmed. A video too short for a
/// highlight is used whole; a video with no samples (no FFmpeg) plays from the top.
pub fn best_window(signals: &[Signal], duration_ms: u32, keep: &[(u32, u32)], min_ms: u32, max_ms: u32) -> Highlight {
    let scores = scores(signals);
    let ranges = ranges(signals, duration_ms, keep);
    let mean_in = |a: u32, b: u32| {
        let inside: Vec<f32> = signals.iter().zip(&scores).filter(|(s, _)| s.t_ms >= a && s.t_ms < b).map(|(_, v)| *v).collect();
        if inside.is_empty() {
            0.0
        } else {
            inside.iter().sum::<f32>() / inside.len() as f32
        }
    };
    let step = 250;
    let mut best: Option<Highlight> = None;
    for &(a, b) in &ranges {
        if b - a < min_ms {
            continue;
        }
        let mut start = a;
        while start + min_ms <= b {
            let longest = max_ms.min(b - start);
            let mut len = longest;
            loop {
                // Longer is a little better when it is as good: more of the moment.
                let score = mean_in(start, start + len) + 0.02 * (len - min_ms) as f32 / (max_ms - min_ms).max(1) as f32;
                if best.is_none_or(|h| score > h.score + 1e-6) {
                    best = Some(Highlight { in_ms: start, out_ms: start + len, score });
                }
                if len <= min_ms {
                    break;
                }
                len = len.saturating_sub(500).max(min_ms);
            }
            start += step;
        }
    }
    if let Some(mut h) = best {
        h.score = mean_in(h.in_ms, h.out_ms).clamp(0.0, 1.0);
        return h;
    }
    // Nothing long enough: the longest stretch there is, or the top of the video.
    let (a, b) = ranges.iter().copied().max_by_key(|(a, b)| b - a).unwrap_or((0, duration_ms));
    let (a, b) = if b - a >= 1_000 { (a, b.min(a + max_ms)) } else { (0, duration_ms.min(max_ms)) };
    Highlight { in_ms: a, out_ms: b.max(a + 1), score: mean_in(a, b).clamp(0.0, 1.0) }
}

/// A highlight as the catalogue keeps it.
#[derive(Debug, Clone, PartialEq)]
pub struct Stored {
    pub analyzer: String,
    pub highlight: Highlight,
}

pub fn get(conn: &Connection, asset_id: i64) -> Result<Option<Stored>> {
    Ok(conn
        .query_row("SELECT analyzer, in_ms, out_ms, score FROM video_highlights WHERE asset_id = ?1", [asset_id], |r| {
            Ok(Stored {
                analyzer: r.get(0)?,
                highlight: Highlight { in_ms: r.get(1)?, out_ms: r.get(2)?, score: r.get::<_, f64>(3)? as f32 },
            })
        })
        .optional()?)
}

pub fn put(conn: &Connection, asset_id: i64, analyzer: &str, h: &Highlight, now: i64) -> Result<()> {
    conn.execute(
        "INSERT INTO video_highlights (asset_id, analyzer, in_ms, out_ms, score, computed_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT(asset_id) DO UPDATE SET analyzer = excluded.analyzer, in_ms = excluded.in_ms, out_ms = excluded.out_ms,
           score = excluded.score, computed_at = excluded.computed_at",
        params![asset_id, analyzer, h.in_ms, h.out_ms, h.score as f64, now],
    )?;
    Ok(())
}

/// Drops a video's highlight, so it is found again (its trim changed).
pub fn forget(conn: &Connection, asset_id: i64) -> Result<()> {
    conn.execute("DELETE FROM video_highlights WHERE asset_id = ?1", [asset_id])?;
    Ok(())
}

/// Videos in the library with no highlight from `analyzer` yet, newest first.
pub fn missing(conn: &Connection, analyzer: &str, limit: usize) -> Result<Vec<i64>> {
    let mut stmt = conn.prepare_cached(
        "SELECT a.id FROM assets a LEFT JOIN video_highlights h ON h.asset_id = a.id
         WHERE a.media_type = 1 AND a.state = 0 AND (h.asset_id IS NULL OR h.analyzer != ?1)
         ORDER BY COALESCE(a.captured_at, a.imported_at) DESC, a.id DESC LIMIT ?2",
    )?;
    let ids = stmt.query_map(params![analyzer, limit as i64], |r| r.get(0))?.collect::<std::result::Result<_, _>>()?;
    Ok(ids)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Samples every 500 ms for `secs`, all ordinary.
    fn plain(secs: u32) -> Vec<Signal> {
        (0..secs * 2).map(|i| Signal { t_ms: i * 500, sharpness: 10.0, luma: 0.45, motion: 0.04, ..Default::default() }).collect()
    }

    fn window(s: &[Signal], duration: u32) -> Highlight {
        best_window(s, duration, &[], HIGHLIGHT_MIN_MS, HIGHLIGHT_MAX_MS)
    }

    #[test]
    fn the_best_stretch_is_chosen() {
        let mut s = plain(20);
        // Seconds 10–14 are sharp and have two people in them.
        for x in s.iter_mut().filter(|x| (10_000..14_000).contains(&x.t_ms)) {
            x.sharpness = 50.0;
            x.faces = Some(2);
        }
        for x in s.iter_mut().filter(|x| x.faces.is_none()) {
            x.faces = Some(0);
        }
        let h = window(&s, 20_000);
        assert!(h.in_ms >= 9_500 && h.out_ms <= 14_500, "{h:?}");
        assert!(h.out_ms - h.in_ms >= HIGHLIGHT_MIN_MS);
    }

    #[test]
    fn a_highlight_never_crosses_a_cut() {
        let mut s = plain(12);
        for x in s.iter_mut().filter(|x| (4_000..8_000).contains(&x.t_ms)) {
            x.sharpness = 50.0;
        }
        s[12].cut = true; // 6 s: a new shot in the middle of the good part.
        let h = window(&s, 12_000);
        assert!(h.out_ms <= 6_000 || h.in_ms >= 6_000, "{h:?} spans the cut");
    }

    #[test]
    fn a_trimmed_video_is_highlighted_inside_what_it_keeps() {
        let mut s = plain(20);
        for x in s.iter_mut().filter(|x| x.t_ms < 5_000) {
            x.sharpness = 99.0; // The best part was trimmed away.
        }
        let h = best_window(&s, 20_000, &[(8_000, 16_000)], HIGHLIGHT_MIN_MS, HIGHLIGHT_MAX_MS);
        assert!(h.in_ms >= 8_000 && h.out_ms <= 16_000, "{h:?}");
    }

    #[test]
    fn a_short_video_is_used_whole() {
        let h = window(&plain(2), 2_000);
        assert_eq!((h.in_ms, h.out_ms), (0, 2_000));
    }

    #[test]
    fn a_video_with_no_samples_plays_from_the_top() {
        let h = window(&[], 30_000);
        assert_eq!(h.in_ms, EDGE_MS);
        assert_eq!(h.out_ms - h.in_ms, HIGHLIGHT_MAX_MS);
    }

    #[test]
    fn the_edges_are_left_out() {
        let mut s = plain(10);
        for x in s.iter_mut().filter(|x| x.t_ms < 4_000) {
            x.sharpness = 50.0;
        }
        assert!(window(&s, 10_000).in_ms >= EDGE_MS);
    }

    #[test]
    fn without_ml_the_other_terms_still_decide() {
        let mut s = plain(10);
        for x in s.iter_mut().filter(|x| x.t_ms >= 6_000) {
            x.luma = 0.02; // Dark.
            x.clipped = 0.6;
        }
        assert!(window(&s, 10_000).out_ms <= 6_500);
        let sc = scores(&s);
        assert!(sc.iter().all(|v| (0.0..=1.0).contains(v)));
    }

    #[test]
    fn shake_and_stillness_score_below_life() {
        assert!(motion_fit(0.04) > motion_fit(0.4) + 0.5);
        assert!(motion_fit(0.04) > motion_fit(0.0));
    }

    #[test]
    fn highlights_are_kept_and_videos_without_one_are_found() {
        let db = crate::Db::open_in_memory().unwrap();
        db.with_conn(|c| {
            for (id, media_type) in [(1, 1), (2, 1), (3, 0)] {
                c.execute(
                    "INSERT INTO assets (id, content_hash, relative_path, filename, extension, media_type, byte_size, orientation, imported_at)
                     VALUES (?1, ?2, ?3, 'f', 'mov', ?4, 1, 1, ?1)",
                    params![id, format!("h{id}"), format!("p{id}"), media_type],
                )?;
            }
            assert_eq!(missing(c, "v1", 10)?, vec![2, 1], "videos only, newest first");
            let h = Highlight { in_ms: 300, out_ms: 3_300, score: 0.7 };
            put(c, 1, "v1", &h, 5)?;
            assert_eq!(get(c, 1)?.unwrap().highlight, h);
            assert_eq!(missing(c, "v1", 10)?, vec![2]);
            assert_eq!(missing(c, "v2", 10)?, vec![2, 1], "a new analyser looks again");
            Ok(())
        })
        .unwrap();
    }
}
