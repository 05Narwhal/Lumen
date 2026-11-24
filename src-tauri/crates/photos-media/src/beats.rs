//! Finding the beat of a file on disk. FFmpeg decodes it to mono PCM; everything after
//! that — the spectral flux and the beat tracker — is `photos_core::audio` and
//! `photos_core::beatgrid`, shared with the learned front end in `photos-ml`.

use std::path::Path;
use std::process::Stdio;

use photos_core::beatgrid::{energy, track, BeatGrid};

use crate::ffmpeg::Ffmpeg;
use crate::MediaError;

/// The rate the DSP tracker works at. Rhythm lives far below 11 kHz, and a low rate keeps a
/// five-minute track to a few megabytes.
pub const DSP_SAMPLE_RATE: u32 = 22_050;

/// The file as mono samples in −1..1 at `sample_rate`.
pub fn decode_mono(ff: &Ffmpeg, path: &Path, sample_rate: u32) -> Result<Vec<f32>, MediaError> {
    // `ffmpeg::cmd`, never a bare `Command`: on Windows a bare spawn flashes a console window
    // for every track memory prep listens to.
    let out = crate::ffmpeg::cmd(&ff.ffmpeg)
        .args(["-v", "error", "-i"])
        .arg(path)
        .args(["-f", "s16le", "-acodec", "pcm_s16le", "-ac", "1", "-ar"])
        .arg(sample_rate.to_string())
        .arg("-")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| MediaError::Decode(e.to_string()))?;
    if !out.status.success() {
        return Err(MediaError::Decode(String::from_utf8_lossy(&out.stderr).trim().to_string()));
    }
    Ok(out.stdout.as_chunks::<2>().0.iter().map(|b| i16::from_le_bytes(*b) as f32 / i16::MAX as f32).collect())
}

/// The DSP beat tracker on one file, and how busy the track is (onsets per second).
pub fn detect(ff: &Ffmpeg, path: &Path) -> Result<(BeatGrid, f32), MediaError> {
    let samples = decode_mono(ff, path, DSP_SAMPLE_RATE)?;
    let (onset, low, frame_ms) = photos_core::audio::spectral_flux(&samples, DSP_SAMPLE_RATE);
    Ok((track(&onset, frame_ms, None, Some(&low)), energy(&onset, frame_ms)))
}
