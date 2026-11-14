//! Codec and playback status commands.

use photos_core::assets;
use photos_media::ffmpeg::ENV_DIR;
use photos_media::{kind_for_extension, playback_with, DerivativeStore, MediaKind, Playback, Probe, VideoSupport};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, State};
use ts_rs::TS;

use crate::error::{AppError, AppResult};
use crate::state::AppState;

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct FfmpegStatus {
    pub available: bool,
    pub version: Option<String>,
    pub path: Option<String>,
    /// Name of the environment variable that overrides the search path.
    pub env_var: String,
}

/// Looks for FFmpeg again (so a fresh install is picked up) and reports what was found.
#[tauri::command(async)]
pub fn get_ffmpeg_status(app: AppHandle, state: State<'_, AppState>) -> AppResult<FfmpegStatus> {
    let found = state.relocate_ffmpeg(&app);
    if found.is_some() {
        crate::derivatives::requeue_underived(&app);
    }
    Ok(FfmpegStatus {
        available: found.is_some(),
        version: found.as_ref().map(|f| f.version.clone()),
        path: found.as_ref().map(|f| f.ffmpeg.display().to_string()),
        env_var: ENV_DIR.into(),
    })
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ExifToolStatus {
    pub available: bool,
    pub version: Option<String>,
    pub path: Option<String>,
    pub env_var: String,
}

/// Looks for ExifTool again and reports what was found. Async: it starts Perl.
#[tauri::command(async)]
pub fn get_exiftool_status(state: State<'_, AppState>) -> AppResult<ExifToolStatus> {
    let found = state.relocate_exiftool();
    Ok(ExifToolStatus {
        available: found.is_some(),
        version: found.as_ref().map(|e| e.version.clone()),
        path: found.as_ref().map(|e| e.program.display().to_string()),
        env_var: photos_media::exiftool::ENV_DIR.into(),
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export)]
pub enum PlaybackMode {
    /// The WebView plays the original through `video/{id}`.
    Native,
    /// Needs the H.264 proxy (`video/{id}?proxy=1`), transcoded on first request.
    Proxy,
    /// No FFmpeg and a container the WebView will not open.
    Unavailable,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PlaybackInfo {
    pub mode: PlaybackMode,
    pub proxy_ready: bool,
    pub video_codec: Option<String>,
    pub audio_codec: Option<String>,
    pub container: Option<String>,
    #[ts(type = "number | null")]
    pub duration_ms: Option<i64>,
    #[ts(type = "number | null")]
    pub width: Option<i64>,
    #[ts(type = "number | null")]
    pub height: Option<i64>,
}

/// What the WebView can play (Round 8A), as `src/lib/video-support.ts` asks it.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct WebviewVideoSupport {
    pub hevc: bool,
    pub hevc_main10: bool,
}

/// Remembers what the WebView plays, once per run, and makes the newest videos' proxies
/// ahead of time, so opening one plays at once (Round 8A).
#[tauri::command(async)]
pub fn set_video_support(app: AppHandle, state: State<'_, AppState>, support: WebviewVideoSupport) -> AppResult<()> {
    let support = VideoSupport { hevc: support.hevc, hevc_main10: support.hevc_main10 };
    let first = state.video_support.write().expect("video support lock").replace(support).is_none();
    tracing::info!(hevc = support.hevc, hevc_main10 = support.hevc_main10, "the WebView's video support");
    if first {
        crate::derivatives::prefetch_newest_proxies(&app);
        crate::video_prep::schedule(&app);
    }
    Ok(())
}

/// What the background video prep is doing (Round 9A), `None` before its first walk.
#[tauri::command(async)]
pub fn get_video_prep_status() -> AppResult<Option<crate::video_prep::Progress>> {
    Ok(crate::video_prep::status())
}

/// Makes the proxies of the videos beside the one being watched (Round 8A).
#[tauri::command(async)]
pub fn prefetch_playback(app: AppHandle, ids: Vec<i64>) -> AppResult<()> {
    crate::derivatives::prefetch_proxies(&app, &ids, photos_jobs::Lane::Prefetch);
    Ok(())
}

/// Pure: the mode for a video given what we know. Without a probe (imported before FFmpeg
/// existed and still no FFmpeg) MP4/MOV get a native attempt; anything else is unavailable.
/// `support` is what the WebView said it plays, when it has.
pub fn playback_mode(probe: Option<&Probe>, extension: &str, ffmpeg_available: bool, os: &str, support: Option<VideoSupport>) -> PlaybackMode {
    match probe {
        Some(p) => match playback_with(p, os, support) {
            Playback::Native => PlaybackMode::Native,
            Playback::Proxy if ffmpeg_available => PlaybackMode::Proxy,
            Playback::Proxy => PlaybackMode::Unavailable,
        },
        None if matches!(extension.to_ascii_lowercase().as_str(), "mp4" | "m4v" | "mov") => PlaybackMode::Native,
        None => PlaybackMode::Unavailable,
    }
}

/// How the viewer should play `id`. Videos imported before FFmpeg was available are
/// probed now and the facts stored.
#[tauri::command(async)]
pub fn get_playback(state: State<'_, AppState>, id: i64) -> AppResult<PlaybackInfo> {
    let lib = state.library()?;
    let ffmpeg = state.ffmpeg();
    let mut detail = lib.db.with_conn(|c| assets::get_asset(c, id, ffmpeg.is_some()))?;
    if kind_for_extension(&detail.extension) != MediaKind::Video {
        return Err(AppError { code: "not_video".into(), message: "This item is not a video.".into(), detail: None });
    }
    let mut meta: photos_media::Metadata =
        detail.metadata.take().and_then(|v| serde_json::from_value(v).ok()).unwrap_or_default();
    if meta.probe.is_none() {
        if let Some(p) = ffmpeg.as_ref().and_then(|f| f.probe(&lib.original_path(id).ok()?).ok()) {
            photos_media::apply_probe(&mut meta, p);
            if let Ok(json) = serde_json::to_string(&meta) {
                let (w, h) = (meta.width.map(i64::from), meta.height.map(i64::from));
                let _ = lib.db.with_conn(|c| assets::set_video_facts(c, id, &json, meta.duration_ms, w, h));
            }
        }
    }
    let support = *state.video_support.read().expect("video support lock");
    let mode = playback_mode(meta.probe.as_ref(), &detail.extension, ffmpeg.is_some(), std::env::consts::OS, support);
    let proxy_ready = mode == PlaybackMode::Proxy
        && DerivativeStore::new(&lib.root).proxy_path(&detail.content_hash).is_file();
    let probe = meta.probe.as_ref();
    Ok(PlaybackInfo {
        mode,
        proxy_ready,
        video_codec: probe.and_then(|p| p.video_codec.clone()),
        audio_codec: probe.and_then(|p| p.audio_codec.clone()),
        container: probe.and_then(|p| p.container.clone()),
        duration_ms: meta.duration_ms.or(detail.duration_ms),
        width: meta.width.map(i64::from).or(detail.width),
        height: meta.height.map(i64::from).or(detail.height),
    })
}

/// Start transcoding the playback proxy for `id` now (idempotent; progress arrives as
/// `proxy://progress`, completion as `proxy://finished`).
#[tauri::command(async)]
pub fn prepare_proxy(app: AppHandle, id: i64) -> AppResult<()> {
    crate::derivatives::start_proxy(&app, id)
}

/// Bytes used by playback proxies in the current library.
#[tauri::command(async)]
pub fn get_proxy_cache_size(state: State<'_, AppState>) -> AppResult<u64> {
    let lib = state.library()?;
    Ok(DerivativeStore::new(&lib.root).proxy_bytes())
}

/// Delete every playback proxy; they are rebuilt on demand. Returns bytes freed.
#[tauri::command(async)]
pub fn clear_proxy_cache(state: State<'_, AppState>) -> AppResult<u64> {
    let lib = state.library()?;
    Ok(DerivativeStore::new(&lib.root).clear_proxies())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn probe(video: &str, pix: &str) -> Probe {
        Probe {
            video_codec: Some(video.into()),
            pix_fmt: Some(pix.into()),
            audio_codec: Some("aac".into()),
            container: Some("mov,mp4,m4a,3gp,3g2,mj2".into()),
            ..Default::default()
        }
    }

    #[test]
    fn mode_from_probe_extension_and_ffmpeg() {
        let h264 = probe("h264", "yuv420p");
        assert_eq!(playback_mode(Some(&h264), "mp4", false, "linux", None), PlaybackMode::Native);
        let hevc = probe("hevc", "yuv420p");
        assert_eq!(playback_mode(Some(&hevc), "mov", true, "windows", None), PlaybackMode::Native);
        assert_eq!(playback_mode(Some(&hevc), "mov", true, "linux", None), PlaybackMode::Proxy);
        assert_eq!(playback_mode(Some(&hevc), "mov", false, "linux", None), PlaybackMode::Unavailable);
        let prores = probe("prores", "yuv422p10le");
        assert_eq!(playback_mode(Some(&prores), "mov", true, "macos", None), PlaybackMode::Proxy);
        assert_eq!(playback_mode(None, "mp4", false, "windows", None), PlaybackMode::Native, "no probe: let the WebView try");
        assert_eq!(playback_mode(None, "mkv", false, "windows", None), PlaybackMode::Unavailable);
    }
}
