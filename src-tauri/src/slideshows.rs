//! Exporting a slideshow: every clip's edited photo rendered once, then one FFmpeg run.
//! Runs on the Import lane (a long job that must not hold up thumbnails) and reports with
//! `slideshow://progress` and `slideshow://finished`.

use std::path::{Path, PathBuf};

use photos_core::assets;
use photos_core::beatgrid::{self, TrackAnalysis};
use photos_core::slideshow::{self, SlideshowDoc};
use photos_jobs::Lane;
use photos_media::{resize_long_edge, DerivativeStore};
use tauri::{AppHandle, Emitter, Manager};

use crate::derivatives::RenderSpec;
use crate::error::{AppError, AppResult};
use crate::events::{SlideshowFinished, SlideshowProgress, SLIDESHOW_FINISHED, SLIDESHOW_PROGRESS};
use crate::state::AppState;

fn render_failed(e: impl std::fmt::Display) -> AppError {
    AppError { code: "derivative_failed".into(), message: "A photo could not be rendered.".into(), detail: Some(e.to_string()) }
}

/// Rendering the stills is about a tenth of the work since Round 4F; FFmpeg is the rest.
const STILLS_SHARE: f32 = 0.12;

/// How many stills are composed at once: half the cores less one (FFmpeg and the UI need the
/// rest), and never more than six, because each holds its photos decoded at full size.
pub fn still_jobs() -> usize {
    let cores = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(2);
    (cores / 2).saturating_sub(1).clamp(1, 6)
}

/// A slideshow's name as a file name: `Trip: Lisbon/2024` → `Trip Lisbon 2024.mp4`.
pub fn output_name(name: &str) -> String {
    let cleaned: String = name.chars().map(|c| if r#"\/:*?"<>|"#.contains(c) { ' ' } else { c }).collect();
    let cleaned = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
    let stem = if cleaned.is_empty() { "Slideshow".to_string() } else { cleaned };
    format!("{stem}.mp4")
}

/// Where the document's audio lives: an absolute path, or one inside the library.
pub fn audio_path(lib_root: &Path, doc: &SlideshowDoc) -> Option<PathBuf> {
    let track = doc.audio.as_ref()?;
    let path = Path::new(&track.path);
    let full = if path.is_absolute() { path.to_path_buf() } else { lib_root.join(path) };
    full.is_file().then_some(full)
}

/// A track's beats, from the catalogue when the file is unchanged since it was analysed,
/// otherwise analysed now and cached. Keyed by file name, so only tracks inside the
/// library's music folder are cached; one picked from elsewhere is analysed each time.
pub fn analyse_track(app: &AppHandle, lib: &photos_core::Library, ffmpeg: &photos_media::Ffmpeg, path: &Path) -> AppResult<TrackAnalysis> {
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or_default().to_string();
    let meta = std::fs::metadata(path).map_err(crate::beat_analysis::unreadable)?;
    let modified = meta
        .modified()
        .ok()
        .and_then(|m| m.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let analyzer = crate::beat_analysis::current_analyzer(app);
    let in_library = path.starts_with(crate::music::music_dir(&lib.root));
    if in_library {
        if let Some(a) = lib.db.with_conn(|c| beatgrid::get_analysis(c, &name))? {
            if a.byte_size == meta.len() as i64 && a.modified == modified && a.analyzer == analyzer {
                return Ok(a);
            }
        }
    }
    let (grid, energy) = crate::beat_analysis::analyse(app, ffmpeg, path)?;
    let a = TrackAnalysis { file_name: name, byte_size: meta.len() as i64, modified, analyzer, grid, energy };
    if in_library {
        lib.db.with_conn(|c| beatgrid::put_analysis(c, &a))?;
    }
    Ok(a)
}

fn needs_ffmpeg() -> AppError {
    AppError {
        code: "export_needs_ffmpeg".into(),
        message: "Exporting a slideshow needs FFmpeg. Set it up in Settings → Playback & codecs.".into(),
        detail: None,
    }
}

/// Starts the export of a saved slideshow and returns at once.
pub fn start_export(app: &AppHandle, id: i64, dest_dir: Option<String>) -> AppResult<()> {
    let (name, doc) = app.state::<AppState>().library()?.db.with_conn(|c| slideshow::get(c, id))?;
    export_doc(app, id, false, name, doc, dest_dir)
}

/// Starts the export of a memory's film (Round 6), straight from its document: nothing is
/// added to Slideshows.
pub fn start_memory_export(app: &AppHandle, memory_id: i64, name: String, doc: SlideshowDoc, dest_dir: Option<String>) -> AppResult<()> {
    export_doc(app, memory_id, true, name, doc, dest_dir)
}

/// The export job. `id` is the slideshow's, or the memory's when `memory`; progress and the
/// result go out under it.
fn export_doc(app: &AppHandle, id: i64, memory: bool, name: String, doc: SlideshowDoc, dest_dir: Option<String>) -> AppResult<()> {
    let state = app.state::<AppState>();
    let lib = state.library()?;
    if doc.clips.is_empty() {
        return Err(AppError { code: "empty_slideshow".into(), message: "This slideshow has no photos yet.".into(), detail: None });
    }
    let Some(ffmpeg) = state.ffmpeg() else { return Err(needs_ffmpeg()) };
    let dir = dest_dir.map(PathBuf::from).unwrap_or_else(|| lib.root.join("exports"));
    std::fs::create_dir_all(&dir)?;
    let out = assets::unique_export_name(&dir, &output_name(&name));
    let audio = audio_path(&lib.root, &doc);
    let store = DerivativeStore::new(&lib.root);
    let originals = lib.originals_dir();
    let (hardware, watermark) = {
        let settings = state.settings.read().expect("settings lock");
        (settings.playback.hardware_encoding.unwrap_or(true), watermark_on(&settings))
    };
    let app2 = app.clone();

    let key = if memory { format!("memory-film:{id}") } else { format!("slideshow:{id}") };
    state.jobs.submit(&key.clone(), Lane::Import, move |cancel| {
        let progress = |p: f32| {
            let _ = app2.emit(SLIDESHOW_PROGRESS, SlideshowProgress { id, memory, progress: p.clamp(0.0, 1.0) });
        };
        let finish = |ok: bool, error: Option<String>| {
            let _ = app2.emit(
                SLIDESHOW_FINISHED,
                SlideshowFinished { id, memory, ok, path: ok.then(|| out.display().to_string()), error },
            );
        };
        // The stills are large (twice the output's long edge) and short-lived, so they go
        // beside the other derivatives rather than in the system temp, which may be on a
        // smaller disk than the library.
        let work = store.root().join("tmp").join(key.replace(':', "-"));
        let _ = std::fs::remove_dir_all(&work);
        if let Err(e) = std::fs::create_dir_all(&work) {
            return finish(false, Some(e.to_string()));
        }
        // One still per clip — its whole frame, composed — at twice the output size, so the
        // Ken Burns zoom still has pixels to show.
        let (fw, fh) = (doc.resolution[0] * 2, doc.resolution[1] * 2);
        let frame_aspect = doc.resolution[0] as f32 / doc.resolution[1].max(1) as f32;
        // The stills are independent: compose them several at a time (Round 4F, Task 3),
        // below normal priority so the UI keeps its share.
        let done = std::sync::atomic::AtomicUsize::new(0);
        let render_one = |i: usize| -> AppResult<PathBuf> {
            if cancel.is_cancelled() {
                return Err(AppError { code: "cancelled".into(), message: "cancelled".into(), detail: Some("cancelled".into()) });
            }
            let clip = &doc.clips[i];
            if clip.video.is_some() {
                // A video clip is rendered as a short video of its own (Round 6), framed at the
                // output size, and joined like a still.
                let info = lib.db.with_conn(|c| assets::render_info(c, clip.asset_id))?;
                let source = originals.join(&info.relative_path);
                let hdr = ffmpeg.probe(&source).ok().is_some_and(|p| matches!(p.color_transfer.as_deref(), Some("smpte2084" | "arib-std-b67")));
                let out = work.join(format!("clip{i}.mp4"));
                photos_media::slideshow::render_video_clip(&ffmpeg, &source, clip, doc.resolution, doc.fps, hdr, &out).map_err(render_failed)?;
                let n = done.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
                progress(STILLS_SHARE * n as f32 / doc.clips.len() as f32);
                return Ok(out);
            }
            let still = work.join(format!("clip{i}.jpg"));
            // Every photo the clip shows, edited, in painting order.
            let mut photos = Vec::new();
            let mut aspects = Vec::new();
            for asset_id in clip.assets() {
                let info = lib.db.with_conn(|c| assets::render_info(c, asset_id))?;
                let source = originals.join(&info.relative_path);
                let spec = RenderSpec::from_info(&info, &source);
                // The photo as shown: upright and edited. A plain JPEG's full size is its
                // original file, whose orientation tag `image::open` ignores.
                let img = crate::derivatives::decode_display_full(&store, &spec, Some(&ffmpeg))?;
                aspects.push(img.width() as f32 / img.height().max(1) as f32);
                // Nothing is drawn larger than the frame, so the composite never resizes from
                // more pixels than it can use.
                photos.push(resize_long_edge(&img, fw.max(fh)));
            }
            let placements = photos_core::slideshow::frame_layout(clip.frame, &aspects, frame_aspect);
            let composed = photos_media::frame::compose(&placements, &photos, fw, fh);
            photos_media::encode_jpeg(&composed, 92, &still).map_err(render_failed)?;
            let n = done.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
            progress(STILLS_SHARE * n as f32 / doc.clips.len() as f32);
            Ok(still)
        };
        let rendered: AppResult<Vec<PathBuf>> = match rayon::ThreadPoolBuilder::new()
            .num_threads(still_jobs())
            .start_handler(|_| photos_jobs::background_priority())
            .build()
        {
            Ok(pool) => pool.install(|| {
                use rayon::prelude::*;
                (0..doc.clips.len()).into_par_iter().map(render_one).collect()
            }),
            Err(_) => (0..doc.clips.len()).map(render_one).collect(),
        };
        let mark = watermark.then(|| work.join("watermark.png")).filter(|m| std::fs::write(m, WATERMARK_PNG).is_ok());
        let stills = match rendered {
            Ok(stills) => stills,
            Err(e) => {
                let _ = std::fs::remove_dir_all(&work);
                return finish(false, Some(e.detail.unwrap_or(e.message)));
            }
        };
        // Several FFmpeg processes, each rendering a piece of the show (Round 4F).
        let cores = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(2);
        let (pieces, threads) = photos_media::slideshow::parallel_plan(cores);
        let encoder = photos_media::slideshow::best_encoder(&ffmpeg, hardware);
        let render = |encoder| {
            photos_media::slideshow::render_slideshow_parallel(
                &ffmpeg,
                &doc,
                &stills,
                audio.as_deref(),
                &out,
                &work.join("pieces"),
                pieces,
                threads,
                encoder,
                mark.as_deref(),
                &mut |p| progress(STILLS_SHARE + (1.0 - STILLS_SHARE) * p),
                &|| cancel.is_cancelled(),
            )
        };
        let mut result = render(encoder);
        // A graphics card can still refuse mid-export (too many encoding sessions, a driver
        // reset): the export is made again on the processor rather than lost.
        if let Err(e) = &result {
            if encoder != photos_media::slideshow::Encoder::X264 && !cancel.is_cancelled() {
                tracing::warn!("slideshow: {} failed ({e}); again with x264", encoder.ffmpeg_name());
                result = render(photos_media::slideshow::Encoder::X264);
            }
        }
        let _ = std::fs::remove_dir_all(&work);
        match result {
            Ok(()) => {
                write_attribution(&out, &doc, &lib.root);
                finish(true, None)
            }
            Err(e) => finish(false, Some(e.to_string())),
        }
    });
    Ok(())
}

/// Lumen's mark (`scripts/brand/make_watermark.py`): the logo and the word *Lumen*, drawn
/// opaque; FFmpeg sizes it to the film and sets its opacity.
const WATERMARK_PNG: &[u8] = include_bytes!("../resources/watermark.png");

/// Whether the films Lumen makes carry its mark (Round 8A, Task 4): Settings → Export.
/// The one place a later Pro licence will be checked, so a free copy always marks its films.
pub fn watermark_on(settings: &photos_core::settings::Settings) -> bool {
    settings.export.watermark
}

/// Music from the library carries a licence, so the credit goes beside the film as
/// `<export>.mp4.attribution.txt`. Returns where it was written, or `None` when the show is
/// silent or its audio is the user's own file, which needs no attribution.
pub fn write_attribution(out: &Path, doc: &SlideshowDoc, library_root: &Path) -> Option<PathBuf> {
    let text = crate::music::attribution_for(doc, library_root)?;
    let path = out.with_extension("mp4.attribution.txt");
    std::fs::write(&path, text).ok()?;
    Some(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_mark_follows_the_setting_and_has_alpha() {
        let mut settings = photos_core::settings::Settings::default();
        assert!(watermark_on(&settings), "on by default");
        settings.export.watermark = false;
        assert!(!watermark_on(&settings));
        let mark = image::load_from_memory(WATERMARK_PNG).unwrap();
        assert!(mark.color().has_alpha());
        assert!(mark.width() > mark.height() * 2, "the logo and the word, side by side");
    }

    use photos_core::slideshow::{AudioTrack, SlideshowDoc};

    fn with_audio(path: &str) -> SlideshowDoc {
        SlideshowDoc {
            audio: Some(AudioTrack { path: path.into(), fade_in_ms: 0, fade_out_ms: 0, gain_db: 0.0, start_ms: 0 }),
            ..SlideshowDoc::new_for(&[1])
        }
    }

    #[test]
    fn an_export_using_library_music_is_credited() {
        let dir = tempfile::tempdir().unwrap();
        let track = crate::music::parse_tracks(include_str!("../tests/jamendo_fixture.json")).unwrap().remove(0);
        let saved = crate::music::write_saved(dir.path(), &track, b"x").unwrap();
        let text = crate::music::attribution_for(&with_audio(&saved.path), dir.path()).unwrap();
        assert!(text.contains("Sunny Afternoon"));
        assert!(text.contains("Jasmine Jordan"));
        assert!(text.contains("CC BY 3.0"));
    }

    #[test]
    fn an_export_using_the_users_own_file_needs_no_credit() {
        let dir = tempfile::tempdir().unwrap();
        assert!(crate::music::attribution_for(&with_audio("C:/Users/me/song.mp3"), dir.path()).is_none());
    }

    #[test]
    fn stills_use_some_cores_but_never_too_many() {
        let n = still_jobs();
        assert!((1..=6).contains(&n), "{n}");
    }

    #[test]
    fn a_name_becomes_a_file_name() {
        assert_eq!(output_name("Trip: Lisbon/2024"), "Trip Lisbon 2024.mp4");
        assert_eq!(output_name("  "), "Slideshow.mp4");
        assert_eq!(output_name("On this day"), "On this day.mp4");
    }
}
