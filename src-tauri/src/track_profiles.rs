//! What a track sounds like (Round 6): `photos_core::music_profile` over audio decoded by
//! FFmpeg, cached per saved track in `track_profiles`. The automatic music choice listens to
//! its candidates through here, and a show's window (the part of the song it plays) is chosen
//! from the profile.

use std::path::Path;

use photos_core::music_profile::{self, TrackProfile};
use tauri::AppHandle;

use crate::error::AppResult;

/// A file's size and modification time, which say whether a cached profile is still its own.
fn stamp(path: &Path) -> Option<(i64, i64)> {
    let meta = std::fs::metadata(path).ok()?;
    let modified = meta.modified().ok()?.duration_since(std::time::UNIX_EPOCH).ok()?.as_secs() as i64;
    Some((meta.len() as i64, modified))
}

fn file_name(path: &Path) -> Option<String> {
    path.file_name().and_then(|n| n.to_str()).map(str::to_string)
}

/// Measures the file at `path`: one decode for both the beats and the sound.
pub fn measure(app: &AppHandle, ff: &photos_media::Ffmpeg, path: &Path) -> AppResult<TrackProfile> {
    let (grid, energy, samples) = crate::beat_analysis::analyse_keeping_samples(app, ff, path)?;
    Ok(music_profile::profile(&samples, photos_media::beats::DSP_SAMPLE_RATE, &grid, energy))
}

/// The cached profile of a track in the library's music folder, when the file is unchanged.
pub fn cached(lib: &photos_core::Library, path: &Path) -> Option<TrackProfile> {
    if !path.starts_with(crate::music::music_dir(&lib.root)) {
        return None;
    }
    let (size, modified) = stamp(path)?;
    let name = file_name(path)?;
    lib.db.with_conn(|c| music_profile::get_cached(c, &name, size, modified)).ok().flatten()
}

/// Keeps a profile for a track in the library's music folder.
pub fn remember(lib: &photos_core::Library, path: &Path, profile: &TrackProfile) {
    if !path.starts_with(crate::music::music_dir(&lib.root)) {
        return;
    }
    if let (Some((size, modified)), Some(name)) = (stamp(path), file_name(path)) {
        if let Err(e) = lib.db.with_conn(|c| music_profile::put_cached(c, &name, size, modified, profile)) {
            tracing::warn!("track profile not kept: {e}");
        }
    }
}

/// The profile of `path`: the cached one, or measured now and kept. `None` when the file
/// cannot be read, which only costs the show its chosen window.
pub fn profile_of(app: &AppHandle, lib: &photos_core::Library, ff: &photos_media::Ffmpeg, path: &Path) -> Option<TrackProfile> {
    if let Some(p) = cached(lib, path) {
        return Some(p);
    }
    match measure(app, ff, path) {
        Ok(p) => {
            remember(lib, path, &p);
            Some(p)
        }
        Err(e) => {
            tracing::info!("track not measured: {}", e.detail.unwrap_or(e.message));
            None
        }
    }
}

/// Measures audio that is not in the library yet — a candidate being listened to. It goes
/// through a temporary file beside the other derivatives, removed afterwards.
pub fn measure_bytes(app: &AppHandle, lib: &photos_core::Library, ff: &photos_media::Ffmpeg, bytes: &[u8]) -> Option<TrackProfile> {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let dir = photos_media::DerivativeStore::new(&lib.root).root().join("tmp");
    std::fs::create_dir_all(&dir).ok()?;
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let path = dir.join(format!("listen-{}-{n}.mp3", std::process::id()));
    std::fs::write(&path, bytes).ok()?;
    let result = measure(app, ff, &path);
    let _ = std::fs::remove_file(&path);
    result.map_err(|e| tracing::info!("candidate not measured: {}", e.detail.unwrap_or(e.message))).ok()
}
