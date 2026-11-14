//! Slideshows: thin adapters over `photos_core::slideshow` and the export job.

use photos_core::now_unix;
use photos_core::slideshow::{self, AudioTrack, LayoutStyle, SlideshowDoc, SlideshowSummary};
use photos_core::music_profile::{self, MusicWindow, TrackProfile};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};
use ts_rs::TS;

use crate::error::{AppError, AppResult};
use crate::state::AppState;

/// How long after a slideshow is made or saved the music step looks again; the editor autosaves
/// often, and a burst of saves shares one pass.
const PREPARE_AFTER_SAVE: std::time::Duration = std::time::Duration::from_secs(15);

/// A slideshow as the editor loads it: the name beside the document.
#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SlideshowDetail {
    #[ts(type = "number")]
    pub id: i64,
    pub name: String,
    pub doc: SlideshowDoc,
    /// Only `sync_slideshow_to_music` fills this: the tempo it found, so the editor can say
    /// what it cut to.
    #[serde(default)]
    pub bpm: Option<u32>,
}

#[tauri::command(async)]
pub fn list_slideshows(state: State<'_, AppState>) -> AppResult<Vec<SlideshowSummary>> {
    let lib = state.library()?;
    Ok(lib.db.with_conn(slideshow::list)?)
}

#[tauri::command(async)]
pub fn get_slideshow(state: State<'_, AppState>, id: i64) -> AppResult<SlideshowDetail> {
    let lib = state.library()?;
    let (name, doc) = lib.db.with_conn(|c| slideshow::get(c, id))?;
    Ok(SlideshowDetail { id, name, doc, bpm: None })
}

/// A new show from the photos the user picked. Their shapes decide the frames, so portraits
/// are never cropped to a landscape video.
///
/// `arrange` defaults to true: the picker hands over a *set* of photographs rather than an
/// order the user authored, so the show is put in the order that reads best — the day kept
/// at the large scale, taste within each moment. Undo passes false, because there the order
/// is a document being restored.
#[tauri::command(async)]
pub fn create_slideshow(
    app: AppHandle,
    state: State<'_, AppState>,
    name: String,
    asset_ids: Vec<i64>,
    style: Option<LayoutStyle>,
    arrange: Option<bool>,
) -> AppResult<i64> {
    let lib = state.library()?;
    let style = style.unwrap_or_default();
    // Videos picked by hand are looked at now rather than waiting for the backfill (Round 6).
    crate::video_highlights::ensure(&app, &asset_ids)?;
    let id = lib.db.with_conn(|c| {
        let doc = if arrange.unwrap_or(true) {
            slideshow::build_arranged(&slideshow::orderables_for(c, &asset_ids)?, style)
        } else {
            slideshow::build(&slideshow::shapes_for(c, &asset_ids)?, style)
        };
        slideshow::create(c, &name, &doc, now_unix())
    })?;
    state.changes.changed(&app, false);
    crate::memory_prep::schedule(&app, PREPARE_AFTER_SAVE);
    Ok(id)
}

/// Re-frames an existing show: the same photos, laid out and joined afresh. This is the
/// editor's *Shuffle*, and how a show made before frames existed gets them.
#[tauri::command(async)]
pub fn rebuild_slideshow(app: AppHandle, state: State<'_, AppState>, id: i64, style: LayoutStyle) -> AppResult<SlideshowDetail> {
    let lib = state.library()?;
    let detail = lib.db.with_conn(|c| {
        let (name, doc) = slideshow::get(c, id)?;
        let ids: Vec<i64> = doc.clips.iter().flat_map(|clip| clip.assets()).collect();
        // A video keeps the part the user chose for it.
        let chosen: std::collections::HashMap<i64, slideshow::VideoSpan> = doc.clips.iter().filter_map(|c| c.video.map(|v| (c.asset_id, v))).collect();
        let mut shapes = slideshow::shapes_for(c, &ids)?;
        for s in &mut shapes {
            if let (Some(v), true) = (chosen.get(&s.id), s.video.is_some()) {
                s.video = Some(*v);
            }
        }
        let rebuilt = SlideshowDoc { audio: doc.audio.clone(), resolution: doc.resolution, fps: doc.fps, ..slideshow::build(&shapes, style) };
        slideshow::save(c, id, &name, &rebuilt, now_unix())?;
        Ok(SlideshowDetail { id, name, doc: rebuilt, bpm: None })
    })?;
    state.changes.changed(&app, false);
    Ok(detail)
}

/// A new show from a memory's highlights, named after it.
#[tauri::command]
pub async fn create_slideshow_from_memory(app: AppHandle, memory_id: i64, style: Option<LayoutStyle>) -> AppResult<i64> {
    let (name, doc) = build_for_memory(&app, memory_id, style.unwrap_or_default()).await?;
    let lib = app.state::<AppState>().library()?;
    let id = lib.db.with_conn(|c| slideshow::create(c, &name, &doc, now_unix()))?;
    state_changed(&app);
    crate::memory_prep::schedule(&app, PREPARE_AFTER_SAVE);
    Ok(id)
}

/// A memory's show: its highlights arranged, with music that suits it (Round 4F: from the
/// whole Jamendo catalogue when a key is set). Shared by *Make slideshow* and the film.
pub async fn build_for_memory(app: &AppHandle, memory_id: i64, style: LayoutStyle) -> AppResult<(String, SlideshowDoc)> {
    // The show is built first, because how long it runs is half of what makes a track suit
    // it. Nothing is written until the music has been decided.
    let (lib, client, client_id, ffmpeg) = {
        let state = app.state::<AppState>();
        let id = state.settings.read().expect("settings lock").music.jamendo_client_id.clone();
        (state.library()?, state.http.clone(), id, state.ffmpeg())
    };
    let t = std::time::Instant::now();
    let (kind, name, doc) = lib.db.with_conn(|c| {
        let memory = photos_core::memories::get_memory(c, memory_id)?;
        // Videos join once their best part is known; a memory never waits for one (Round 6).
        let ids = slideshow::with_ready_videos(c, &photos_core::memories::curated_ids(c, memory_id)?)?;
        Ok((memory.kind, memory.title, slideshow::build_arranged(&slideshow::orderables_for(c, &ids)?, style)))
    })?;
    let arrange_ms = t.elapsed().as_millis() as u64;
    let t = std::time::Instant::now();

    // The photographs' own mood when they have embeddings; the memory's kind otherwise.
    let ids: Vec<i64> = doc.clips.iter().flat_map(|c| c.assets()).collect();
    let (moods, energy) = lib
        .db
        .with_conn(|c| {
            let features = photos_core::embedding::load(c, &ids)?;
            let vecs: Vec<&[i8]> = features.values().map(|f| f.vec.as_slice()).collect();
            Ok(crate::music::mood_from_photos(&vecs, &photos_ml::prototypes()))
        })?
        .map(|(m, e)| (m, Some(e)))
        .unwrap_or_else(|| (crate::music::mood_for_memory(kind), None));
    let want = crate::music::MusicWant {
        moods,
        energy,
        target_ms: slideshow::timeline(&doc).total_ms,
        // The memory's own id, so remaking the show gives the same tune.
        seed: memory_id,
    };
    // Never a tune another show already plays (Round 6) — except this memory's own film.
    let in_use = lib.db.with_conn(|c| slideshow::music_in_use(c, Some(memory_id)))?;
    let saved_profile = |t: &crate::music::SavedTrack| crate::track_profiles::cached(&lib, std::path::Path::new(&t.path));
    let analyse = |bytes: Vec<u8>| {
        let (app, lib, ffmpeg) = (app.clone(), lib.clone(), ffmpeg.clone());
        async move {
            let ff = ffmpeg?;
            tauri::async_runtime::spawn_blocking(move || crate::track_profiles::measure_bytes(&app, &lib, &ff, &bytes)).await.ok().flatten()
        }
    };
    let mood_ms = t.elapsed().as_millis() as u64;
    let t = std::time::Instant::now();
    let key = client_id.as_deref().map(str::trim).filter(|k| !k.is_empty());
    let jamendo = key.map(|key| crate::music::Jamendo { client: &client, key });
    let pick = crate::music::music_for(jamendo.as_ref(), &lib.root, &want, &in_use, saved_profile, analyse).await;
    let music_ms = t.elapsed().as_millis() as u64;
    let t = std::time::Instant::now();
    let audio = match pick {
        Some(pick) => {
            let path = std::path::PathBuf::from(&pick.track.path);
            if let Some(p) = &pick.profile {
                crate::track_profiles::remember(&lib, &path, p);
            }
            let window = music_window_of(app, pick.profile, &path, want.target_ms).await;
            tracing::info!(memory_id, arrange_ms, mood_ms, music_ms, window_ms = t.elapsed().as_millis() as u64, "memory film: show built");
            Some(AudioTrack {
                path: library_relative(&lib.root, &pick.track.path),
                fade_in_ms: window.fade_in_ms,
                fade_out_ms: window.fade_out_ms,
                gain_db: -4.0,
                start_ms: window.start_ms,
            })
        }
        None => None,
    };
    Ok((name, SlideshowDoc { audio, ..doc }))
}

/// The part of the track at `path` a show of `target_ms` plays (Round 6): from its profile,
/// measured now when there is none. Without FFmpeg, or a track that cannot be read, the top
/// of the track as before.
async fn music_window_of(app: &AppHandle, known: Option<TrackProfile>, path: &std::path::Path, target_ms: u32) -> MusicWindow {
    let top = MusicWindow { start_ms: 0, fade_in_ms: music_profile::FADE_IN_TOP_MS, fade_out_ms: 1500 };
    let profile = match known {
        Some(p) => Some(p),
        None => {
            let state = app.state::<AppState>();
            let (Ok(lib), Some(ff)) = (state.library(), state.ffmpeg()) else { return top };
            let (app, path) = (app.clone(), path.to_path_buf());
            tauri::async_runtime::spawn_blocking(move || crate::track_profiles::profile_of(&app, &lib, &ff, &path)).await.ok().flatten()
        }
    };
    profile.map(|p| music_profile::choose_window(&p, target_ms)).unwrap_or(top)
}

/// The part of a track a show of `target_ms` should play (Round 6). The editor asks when
/// music is chosen and on *Best part*, and applies the answer to its own copy of the show, so
/// nothing here races its autosave. `path` is a library track (`music/<file>` or absolute) or
/// the user's own file.
#[tauri::command]
pub async fn music_window(app: AppHandle, path: String, target_ms: u32) -> AppResult<MusicWindow> {
    let lib = app.state::<AppState>().library()?;
    let p = std::path::Path::new(&path);
    let full = if p.is_absolute() { p.to_path_buf() } else { lib.root.join(p) };
    Ok(music_window_of(&app, None, &full, target_ms).await)
}

/// A track saved in the library as `music/<file>`, which the preview plays by name and the
/// export resolves against the library; anything else as it is.
fn library_relative(root: &std::path::Path, path: &str) -> String {
    let p = std::path::Path::new(path);
    match (p.strip_prefix(crate::music::music_dir(root)), p.file_name()) {
        (Ok(_), Some(name)) => format!("music/{}", name.to_string_lossy()),
        _ => path.to_string(),
    }
}

/// Cuts `doc` to the beat of its own music: the cuts on the beat, the automatic joins chosen
/// again now that the downbeats and the music's energy are known, then the cuts once more
/// (the new transition lengths change the overlaps the cuts were placed around). Errors the
/// user can act on: no music, no FFmpeg, no steady beat.
pub async fn beat_sync(app: &AppHandle, doc: &SlideshowDoc) -> AppResult<(SlideshowDoc, u32)> {
    let (lib, ffmpeg) = {
        let state = app.state::<AppState>();
        (state.library()?, state.ffmpeg())
    };
    let Some(path) = crate::slideshows::audio_path(&lib.root, doc) else {
        return Err(AppError {
            code: "no_music".into(),
            message: "Add music to the slideshow first, then it can be cut to the beat.".into(),
            detail: None,
        });
    };
    let Some(ffmpeg) = ffmpeg else {
        return Err(AppError {
            code: "sync_needs_ffmpeg".into(),
            message: "Finding the beat needs FFmpeg. Set it up in Settings → Playback & codecs.".into(),
            detail: None,
        });
    };
    let lib2 = lib.clone();
    let app2 = app.clone();
    let analysis = tauri::async_runtime::spawn_blocking(move || crate::slideshows::analyse_track(&app2, &lib2, &ffmpeg, &path))
        .await
        .map_err(crate::beat_analysis::unreadable)??;
    let grid = analysis.grid.clone();
    if !grid.is_steady() {
        return Err(AppError {
            code: "no_beat".into(),
            message: "This track has no steady beat to cut to.".into(),
            detail: Some(format!("confidence {:.2}", grid.confidence)),
        });
    }
    let first = slideshow::snap_to_beat_grid(doc, &grid);
    let ids: Vec<i64> = first.clips.iter().map(|c| c.asset_id).collect();
    let photos: std::collections::HashMap<i64, slideshow::Orderable> =
        lib.db.with_conn(|c| slideshow::orderables_for(c, &ids))?.into_iter().map(|o| (o.id, o)).collect();
    let energy = (analysis.energy / 6.0).clamp(0.0, 1.0);
    let rejoined = slideshow::retransition(&first, &photos, Some(&grid), energy);
    Ok((slideshow::snap_to_beat_grid(&rejoined, &grid).normalized(), grid.bpm.round() as u32))
}

/// A memory played as a film (Round 4F, Task 5).
#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct MemoryFilm {
    pub name: String,
    pub doc: SlideshowDoc,
    /// The tempo the cuts follow, when the film is cut to the beat.
    pub bpm: Option<u32>,
}

/// The memory's film: from the cache when its photos are the ones it was made from, else
/// built (`build_for_memory`, then cut to the beat when the music has a steady one; a film
/// that cannot be synced stays as built) and kept. Progress goes out as `film://progress`.
#[tauri::command]
pub async fn memory_film(app: AppHandle, memory_id: i64, rebuild: Option<bool>) -> AppResult<MemoryFilm> {
    film_for(&app, memory_id, rebuild.unwrap_or(false), true).await
}

/// What Play runs, and what the background step (`memory_prep`) runs ahead of it (Round 21 §14):
/// the cached film, or one built now and kept. `announce` sends `film://progress` for a Play
/// that is waiting; the step stays quiet. Every part logs the milliseconds it took, so what Play
/// waits for is in the log (`memory film …`).
pub async fn film_for(app: &AppHandle, memory_id: i64, rebuild: bool, announce: bool) -> AppResult<MemoryFilm> {
    let started = std::time::Instant::now();
    let lib = app.state::<AppState>().library()?;
    let (name, key) = lib.db.with_conn(|c| {
        let memory = photos_core::memories::get_memory(c, memory_id)?;
        Ok((memory.title, photos_core::memory_films::source_key(&photos_core::memories::curated_ids(c, memory_id)?)))
    })?;
    if !rebuild {
        if let Some(film) = lib.db.with_conn(|c| photos_core::memory_films::get_fresh(c, memory_id, &key))? {
            tracing::info!(memory_id, announce, ms = started.elapsed().as_millis() as u64, "memory film: cache hit");
            return Ok(MemoryFilm { name, doc: film.doc, bpm: film.bpm });
        }
    }
    let lookup_ms = started.elapsed().as_millis() as u64;
    let stage = |stage: &str| {
        if announce {
            let _ = app.emit(crate::events::FILM_PROGRESS, crate::events::FilmProgress { memory_id, stage: stage.into() });
        }
    };
    stage("music");
    let t = std::time::Instant::now();
    let (name, doc) = build_for_memory(app, memory_id, LayoutStyle::default()).await?;
    let build_ms = t.elapsed().as_millis() as u64;
    let t = std::time::Instant::now();
    let (doc, bpm) = if doc.audio.is_some() {
        stage("beat");
        match beat_sync(app, &doc).await {
            Ok((synced, bpm)) => (synced, Some(bpm)),
            Err(e) => {
                tracing::info!("memory film {memory_id}: not cut to the beat ({})", e.message);
                (doc, None)
            }
        }
    } else {
        (doc, None)
    };
    let beat_ms = t.elapsed().as_millis() as u64;
    lib.db.with_conn(|c| photos_core::memory_films::put(c, memory_id, &key, &doc, bpm, now_unix()))?;
    stage("ready");
    tracing::info!(memory_id, announce, lookup_ms, build_ms, beat_ms, total_ms = started.elapsed().as_millis() as u64, "memory film: built");
    Ok(MemoryFilm { name, doc, bpm })
}

/// *Export video* on a memory (Round 6): its film, made if need be, exported to `dest_dir`
/// in the background without a Slideshows entry.
#[tauri::command]
pub async fn export_memory_film(app: AppHandle, memory_id: i64, dest_dir: String) -> AppResult<()> {
    let film = memory_film(app.clone(), memory_id, Some(false)).await?;
    crate::slideshows::start_memory_export(&app, memory_id, film.name, film.doc, Some(dest_dir))
}

/// *Edit slideshow* on a film: the film kept as an ordinary slideshow for the editor.
#[tauri::command]
pub async fn save_memory_film(app: AppHandle, memory_id: i64) -> AppResult<i64> {
    let film = memory_film(app.clone(), memory_id, Some(false)).await?;
    let lib = app.state::<AppState>().library()?;
    let id = lib.db.with_conn(|c| slideshow::create(c, &film.name, &film.doc, now_unix()))?;
    state_changed(&app);
    crate::memory_prep::schedule(&app, PREPARE_AFTER_SAVE);
    Ok(id)
}

fn state_changed(app: &AppHandle) {
    app.state::<AppState>().changes.changed(app, false);
}

/// Re-cuts a show so its cuts land on the beat of its own music.
///
/// The decode runs off the main thread: reading a five-minute track is a second of work,
/// and the UI must stay answerable. Returns an error the user can act on when there is no
/// music, or when the track has no steady beat to cut to.
#[tauri::command]
pub async fn sync_slideshow_to_music(app: AppHandle, id: i64) -> AppResult<SlideshowDetail> {
    let lib = app.state::<AppState>().library()?;
    let (name, doc) = lib.db.with_conn(|c| slideshow::get(c, id))?;
    let (snapped, bpm) = beat_sync(&app, &doc).await?;
    lib.db.with_conn(|c| slideshow::save(c, id, &name, &snapped, now_unix()))?;
    state_changed(&app);
    Ok(SlideshowDetail { id, name, doc: snapped, bpm: Some(bpm) })
}

/// The editor autosaves through here; the document is normalized on the way in.
#[tauri::command(async)]
pub fn save_slideshow(app: AppHandle, state: State<'_, AppState>, id: i64, name: String, doc: SlideshowDoc) -> AppResult<()> {
    let lib = state.library()?;
    lib.db.with_conn(|c| slideshow::save(c, id, &name, &doc.normalized(), now_unix()))?;
    state.changes.changed(&app, false);
    crate::memory_prep::schedule(&app, PREPARE_AFTER_SAVE);
    Ok(())
}

/// Deleting returns the document, so Undo can put it back.
#[tauri::command(async)]
pub fn delete_slideshow(app: AppHandle, state: State<'_, AppState>, id: i64) -> AppResult<Option<SlideshowDetail>> {
    let lib = state.library()?;
    let removed = lib.db.with_conn(|c| slideshow::delete(c, id))?;
    state.changes.changed(&app, false);
    Ok(removed.map(|(name, doc)| SlideshowDetail { id, name, doc, bpm: None }))
}

/// Queues the export on the Import lane; progress arrives as `slideshow://progress`.
#[tauri::command(async)]
pub fn export_slideshow(app: AppHandle, id: i64, dest_dir: Option<String>) -> AppResult<()> {
    crate::slideshows::start_export(&app, id, dest_dir)
}

/// Clips for photos and videos added to a show in the editor (Round 6), framed as a new show
/// would frame them and each video at its best part. The editor appends them to its own copy
/// of the show, so nothing here races its autosave.
#[tauri::command(async)]
pub fn slideshow_clips(app: AppHandle, state: State<'_, AppState>, asset_ids: Vec<i64>) -> AppResult<Vec<slideshow::Clip>> {
    crate::video_highlights::ensure(&app, &asset_ids)?;
    let lib = state.library()?;
    Ok(lib.db.with_conn(|c| Ok(slideshow::build(&slideshow::shapes_for(c, &asset_ids)?, LayoutStyle::Smart).normalized().clips))?)
}

/// A video's best part, looked for again now (Round 6): the inspector's *Best part*.
#[tauri::command(async)]
pub fn video_best_part(app: AppHandle, state: State<'_, AppState>, asset_id: i64) -> AppResult<slideshow::VideoSpan> {
    let lib = state.library()?;
    let Some(ff) = state.ffmpeg() else {
        return Err(AppError {
            code: "video_needs_ffmpeg".into(),
            message: "Finding a video's best part needs FFmpeg. Set it up in Settings → Playback & codecs.".into(),
            detail: None,
        });
    };
    let h = crate::video_highlights::analyse(&app, &lib, &ff, asset_id)?;
    Ok(slideshow::VideoSpan { in_ms: h.in_ms, out_ms: h.out_ms })
}
