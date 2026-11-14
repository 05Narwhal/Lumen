//! The best part of each video, for slideshows (Round 6). `photos_media::video_frames`
//! samples the video and measures each frame; here the ML adds what the frames show — faces
//! (YuNet) and how attractive they are (the CLIP aesthetic head) — when the models are ready,
//! and `photos_core::video_highlight::best_window` picks the part, which is kept in
//! `video_highlights`. The analyser's name says which models took part, so a model that
//! arrives later re-scores the videos already seen.

use std::collections::HashSet;

use photos_core::video_highlight::{self, Highlight, HIGHLIGHT_MAX_MS, HIGHLIGHT_MIN_MS};
use photos_media::video_frames;
use photos_ml::manifest::manifest;
use tauri::{AppHandle, Manager};

use crate::error::{AppError, AppResult};
use crate::state::AppState;

const BASE_ANALYZER: &str = "frames@1";
/// Videos per pass of the backfill, so a cancel is heard between them.
const BATCH: usize = 8;
/// The aesthetic head runs on every second sample: neighbouring samples look alike.
const AESTHETIC_EVERY: usize = 2;

fn ready(app: &AppHandle, role: &str) -> bool {
    let Some(e) = manifest().by_role(role) else { return false };
    crate::models::enabled(app) && crate::models::store(app).is_ready(e)
}

/// What would score a video now: the frame measures, plus each model that is ready.
pub fn analyzer(app: &AppHandle) -> String {
    let mut name = BASE_ANALYZER.to_string();
    if ready(app, "face_detection") && ready(app, "face_embedding") {
        name.push_str("+faces");
    }
    if ready(app, "image_embedding") {
        name.push_str("+clip");
    }
    name
}

fn unreadable(e: impl std::fmt::Display) -> AppError {
    AppError { code: "video_unreadable".into(), message: "The video could not be read.".into(), detail: Some(e.to_string()) }
}

/// Finds, keeps and returns the best part of video `asset_id`.
pub fn analyse(app: &AppHandle, lib: &photos_core::Library, ff: &photos_media::Ffmpeg, asset_id: i64) -> AppResult<Highlight> {
    let analyzer = analyzer(app);
    let (relative, duration): (String, Option<i64>) =
        lib.db.with_conn(|c| Ok(c.query_row("SELECT relative_path, duration_ms FROM assets WHERE id = ?1", [asset_id], |r| Ok((r.get(0)?, r.get(1)?)))?))?;
    let path = lib.originals_dir().join(&relative);
    let probe = ff.probe(&path).map_err(unreadable)?;
    let duration = probe.duration_ms.or(duration).unwrap_or(0).clamp(0, u32::MAX as i64) as u32;
    let keep: Vec<(u32, u32)> = lib
        .db
        .with_conn(|c| photos_core::edits::get_trim(c, asset_id))?
        .unwrap_or_default()
        .iter()
        .map(|s| (s.start_ms.clamp(0, u32::MAX as i64) as u32, s.end_ms.clamp(0, u32::MAX as i64) as u32))
        .collect();
    let hdr = matches!(probe.color_transfer.as_deref(), Some("smpte2084" | "arib-std-b67"));
    let samples = video_frames::sample(ff, &path, &probe, hdr).map_err(unreadable)?;
    let mut signals = video_frames::signals(&samples);
    let times: Vec<u32> = signals.iter().map(|s| s.t_ms).collect();
    if let Some(levels) = video_frames::loudness_at(ff, &path, &times, video_frames::sample_every(duration)) {
        for (s, l) in signals.iter_mut().zip(levels) {
            s.loudness = Some(l);
        }
    }
    if analyzer.contains("+faces") {
        crate::models::with_faces(app, |det, _| {
            for (s, sample) in signals.iter_mut().zip(&samples) {
                s.faces = det.detect(&sample.rgb, sample.width, sample.height).ok().map(|f| f.len() as u32);
            }
        });
    }
    if analyzer.contains("+clip") {
        let picked: Vec<usize> = (0..samples.len()).step_by(AESTHETIC_EVERY).collect();
        for chunk in picked.chunks(16) {
            let batch: Vec<photos_ml::clip::Rgb<'_>> =
                chunk.iter().map(|&i| photos_ml::clip::Rgb { data: &samples[i].rgb, width: samples[i].width, height: samples[i].height }).collect();
            if let Some(Ok(out)) = crate::models::with_encoder(app, |enc| enc.embed_batch(&batch)) {
                for (&i, (_, aesthetic)) in chunk.iter().zip(out) {
                    signals[i].aesthetic = aesthetic;
                }
            }
        }
    }
    let highlight = video_highlight::best_window(&signals, duration, &keep, HIGHLIGHT_MIN_MS, HIGHLIGHT_MAX_MS);
    lib.db.with_conn(|c| video_highlight::put(c, asset_id, &analyzer, &highlight, photos_core::now_unix()))?;
    Ok(highlight)
}

/// A video that cannot be read still gets a highlight — its opening — so the backfill does
/// not try it again on every pass, and a show can still use it.
fn fallback(app: &AppHandle, lib: &photos_core::Library, asset_id: i64) {
    let duration: i64 = lib
        .db
        .with_conn(|c| Ok(c.query_row("SELECT COALESCE(duration_ms, 0) FROM assets WHERE id = ?1", [asset_id], |r| r.get(0))?))
        .unwrap_or(0);
    let h = video_highlight::best_window(&[], duration.clamp(0, u32::MAX as i64) as u32, &[], HIGHLIGHT_MIN_MS, HIGHLIGHT_MAX_MS);
    let _ = lib.db.with_conn(|c| video_highlight::put(c, asset_id, &analyzer(app), &h, photos_core::now_unix()));
}

/// Every video without a current highlight, on the Maintenance lane (coalesced). Without
/// FFmpeg there is nothing to look with, and nothing is done.
pub fn schedule(app: &AppHandle) {
    let state = app.state::<AppState>();
    if state.ffmpeg().is_none() {
        return;
    }
    let a = app.clone();
    state.jobs.submit("maintenance:video-highlights", photos_jobs::Lane::Maintenance, move |cancel| {
        let mut skip: HashSet<i64> = HashSet::new();
        let started = std::time::Instant::now();
        let mut done = 0usize;
        loop {
            let state = a.state::<AppState>();
            let (Ok(lib), Some(ff)) = (state.library(), state.ffmpeg()) else { break };
            let name = analyzer(&a);
            let Ok(ids) = lib.db.with_conn(|c| video_highlight::missing(c, &name, BATCH + skip.len())) else { break };
            let ids: Vec<i64> = ids.into_iter().filter(|id| !skip.contains(id)).take(BATCH).collect();
            if ids.is_empty() {
                break;
            }
            for id in ids {
                if cancel.is_cancelled() {
                    return;
                }
                if let Err(e) = analyse(&a, &lib, &ff, id) {
                    tracing::info!(asset = id, "video highlight: {}", e.detail.unwrap_or(e.message));
                    fallback(&a, &lib, id);
                }
                skip.insert(id);
                done += 1;
            }
        }
        if done > 0 {
            tracing::info!(videos = done, seconds = started.elapsed().as_secs_f32(), "video highlights found");
        }
    });
}

/// The highlight of each video among `ids` that has none yet, found now — for videos the user
/// adds to a show by hand, who should not wait for the backfill. Photos are ignored.
pub fn ensure(app: &AppHandle, ids: &[i64]) -> AppResult<()> {
    let state = app.state::<AppState>();
    let lib = state.library()?;
    let videos: Vec<i64> = lib.db.with_conn(|c| {
        let mut stmt = c.prepare_cached("SELECT 1 FROM assets WHERE id = ?1 AND media_type = 1")?;
        Ok(ids.iter().copied().filter(|id| stmt.exists([id]).unwrap_or(false)).collect())
    })?;
    for id in videos {
        if lib.db.with_conn(|c| video_highlight::get(c, id))?.is_some() {
            continue;
        }
        match state.ffmpeg() {
            Some(ff) => {
                if let Err(e) = analyse(app, &lib, &ff, id) {
                    tracing::info!(asset = id, "video highlight: {}", e.detail.unwrap_or(e.message));
                    fallback(app, &lib, id);
                }
            }
            None => fallback(app, &lib, id),
        }
    }
    Ok(())
}
