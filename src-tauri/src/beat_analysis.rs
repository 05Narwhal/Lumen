//! A track's beats for cut-to-the-beat (ML design §6): the learned tracker when its model is
//! here and smart features are on, the DSP tracker otherwise. Both feed the same decoder.

use photos_core::beatgrid::{energy, track, BeatGrid};
use tauri::AppHandle;

use crate::error::{AppError, AppResult};

pub fn unreadable(e: impl std::fmt::Display) -> AppError {
    AppError { code: "sync_failed".into(), message: "The track could not be read.".into(), detail: Some(e.to_string()) }
}

/// Beat This! is run on 15 s chunks rather than its trained 30 s. Its attention needs memory
/// in the square of the chunk: at 30 s ONNX Runtime's arena grew past 2 GB. Measured on What
/// Is Love (212 s): 30 s → 327 beats, 81 downbeats, 0.948, 4.7 s; 15 s → the same beats and
/// downbeats, 0.940, 2.7 s; 10 s → 88 downbeats, so the bars start to slip.
const BEAT_CHUNK_FRAMES: usize = 750;

fn learned_entry(app: &AppHandle) -> Option<&'static photos_ml::manifest::ModelEntry> {
    let e = photos_ml::manifest::manifest().by_role("beat_activation")?;
    (crate::models::enabled(app) && crate::models::store(app).is_ready(e)).then_some(e)
}

/// Which tracker would analyse a track now. A cached analysis from another one is redone,
/// so a model arriving later improves the tracks already analysed.
pub fn current_analyzer(app: &AppHandle) -> String {
    learned_entry(app).map(|e| e.id.clone()).unwrap_or_else(|| "dsp@1".into())
}


/// The beats of `path` and how busy it is (onsets per second). Energy always comes from
/// spectral flux, so it means the same whichever tracker found the beats.
pub fn analyse(app: &AppHandle, ff: &photos_media::Ffmpeg, path: &std::path::Path) -> AppResult<(BeatGrid, f32)> {
    analyse_keeping_samples(app, ff, path).map(|(grid, busy, _)| (grid, busy))
}

/// `analyse`, also handing back the mono samples at `photos_media::beats::DSP_SAMPLE_RATE`,
/// so a track's sound can be measured without decoding it a second time (Round 6).
pub fn analyse_keeping_samples(app: &AppHandle, ff: &photos_media::Ffmpeg, path: &std::path::Path) -> AppResult<(BeatGrid, f32, Vec<f32>)> {
    let rate = photos_media::beats::DSP_SAMPLE_RATE;
    let dsp = photos_media::beats::decode_mono(ff, path, rate).map_err(unreadable)?;
    let (onset, low, frame_ms) = photos_core::audio::spectral_flux(&dsp, rate);
    let busy = energy(&onset, frame_ms);
    if let Some(entry) = learned_entry(app) {
        let store = crate::models::store(app);
        match photos_ml::beats::BeatModel::open(&store.path_of(entry), entry) {
            Ok(model) => {
                let mut model = model.with_chunk(BEAT_CHUNK_FRAMES);
                let resampled =
                    if model.sample_rate() == rate { None } else { Some(photos_media::beats::decode_mono(ff, path, model.sample_rate()).map_err(unreadable)?) };
                match model.activations(resampled.as_deref().unwrap_or(&dsp)) {
                    Ok((beat, down, ms)) => return Ok((track(&beat, ms, Some(&down), None), busy, dsp)),
                    Err(e) => tracing::warn!("beat model: {e}; using the DSP tracker"),
                }
            }
            Err(e) => tracing::warn!("beat model: {e}; using the DSP tracker"),
        }
    }
    Ok((track(&onset, frame_ms, None, Some(&low)), busy, dsp))
}
