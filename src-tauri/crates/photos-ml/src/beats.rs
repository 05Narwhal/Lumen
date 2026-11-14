//! The learned beat front end: the track's log-mel spectrogram in, per-frame beat and
//! downbeat probabilities out (Beat This!, ISMIR 2024). The decoder that turns them into
//! beats is `photos_core::beatgrid::track`, shared with the DSP front end.

use std::path::Path;

use ort::{inputs, session::Session, value::Tensor};
use photos_core::audio::{mel_frames, MelSpec};

use crate::manifest::{AudioSpec, ModelEntry};
use crate::MlError;

fn mel_spec(a: &AudioSpec) -> MelSpec {
    MelSpec {
        sample_rate: a.sample_rate,
        n_fft: a.n_fft,
        hop: a.hop,
        n_mels: a.n_mels,
        fmin: a.fmin,
        fmax: a.fmax,
        slaney_scale: a.mel_scale == "slaney",
        slaney_norm: a.norm == "slaney",
        window_norm: a.frame_length_norm,
        power: a.power,
    }
}

/// The model's input: `ln(1 + log_multiplier · mel)`, frames × n_mels.
pub fn log_mel(samples: &[f32], a: &AudioSpec) -> Vec<Vec<f32>> {
    mel_frames(samples, &mel_spec(a)).into_iter().map(|f| f.into_iter().map(|v| (1.0 + a.log_multiplier * v).ln()).collect()).collect()
}

/// The beat and downbeat tracker. The track is run in chunks of `chunk_frames` that
/// overlap, and each chunk's `border_frames` at either end are dropped: the model never saw
/// the edges of its input in training (Beat This!'s `split_piece`).
pub struct BeatModel {
    session: Session,
    audio: AudioSpec,
}

impl BeatModel {
    pub fn open(path: &Path, entry: &ModelEntry) -> Result<Self, MlError> {
        let audio = entry.audio.clone().ok_or_else(|| MlError::Missing(format!("{} has no audio spec", entry.id)))?;
        Ok(Self { session: Session::builder()?.with_intra_threads(4)?.commit_from_file(path)?, audio })
    }

    /// Runs the model on chunks of at most `frames` (never more than the manifest's).
    pub fn with_chunk(mut self, frames: usize) -> Self {
        self.audio.chunk_frames = frames.clamp(4 * self.audio.border_frames, self.audio.chunk_frames);
        self
    }

    /// The rate the model wants its samples at.
    pub fn sample_rate(&self) -> u32 {
        self.audio.sample_rate
    }

    /// Per-frame beat and downbeat probabilities for mono `samples` at `sample_rate()`, and
    /// the frame length in ms.
    pub fn activations(&mut self, samples: &[f32]) -> Result<(Vec<f32>, Vec<f32>, f32), MlError> {
        let spect = log_mel(samples, &self.audio);
        let (n, m) = (spect.len(), self.audio.n_mels);
        let (chunk, border) = (self.audio.chunk_frames, self.audio.border_frames);
        let step = chunk - 2 * border;
        let mut beat = vec![0f32; n];
        let mut down = vec![0f32; n];
        let mut start = 0usize;
        while start < n {
            let end = (start + chunk).min(n);
            let flat: Vec<f32> = spect[start..end].iter().flatten().copied().collect();
            let t = Tensor::from_array(([1usize, end - start, m], flat))?;
            let out = self.session.run(inputs!["spect" => t])?;
            let (_, b) = out[self.audio.beat_output.as_str()].try_extract_tensor::<f32>()?;
            let (_, d) = out[self.audio.downbeat_output.as_str()].try_extract_tensor::<f32>()?;
            if b.len() != end - start || d.len() != end - start {
                return Err(MlError::Shape(format!("{} frames in, {} out", end - start, b.len())));
            }
            // Keep the middle of each chunk; its edges saw too little context.
            let keep_from = if start == 0 { 0 } else { border };
            let keep_to = if end == n { end - start } else { end - start - border };
            beat[start + keep_from..start + keep_to].copy_from_slice(&b[keep_from..keep_to]);
            down[start + keep_from..start + keep_to].copy_from_slice(&d[keep_from..keep_to]);
            if end == n {
                break;
            }
            start += step;
        }
        Ok((beat, down, self.audio.hop as f32 * 1000.0 / self.audio.sample_rate as f32))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::manifest;

    /// The export script's reference signal, regenerated from its formula.
    fn reference_signal(sr: usize) -> Vec<f32> {
        (0..sr * 3)
            .map(|n| {
                let mut y = 0.3 * (2.0 * std::f64::consts::PI * 440.0 * n as f64 / sr as f64).sin();
                if n % (sr / 2) < 64 {
                    y += 0.8;
                }
                y as f32
            })
            .collect()
    }

    /// Rust's log-mel must be torchaudio's, or the model sees a spectrogram it never learned.
    #[test]
    fn log_mel_matches_torchaudio() {
        let Some(entry) = manifest().by_role("beat_activation") else { return };
        let a = entry.audio.as_ref().unwrap();
        let reference: serde_json::Value = serde_json::from_str(include_str!("../tests/fixtures/mel_reference.json")).unwrap();
        let want: Vec<Vec<f64>> = reference["log_mel"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r.as_array().unwrap().iter().map(|v| v.as_f64().unwrap()).collect())
            .collect();
        let got = log_mel(&reference_signal(a.sample_rate as usize), a);
        assert_eq!(got.len(), want.len(), "frame count");
        let mut worst = (0.0f64, 0, 0);
        for (t, (g, w)) in got.iter().zip(&want).enumerate() {
            assert_eq!(g.len(), w.len());
            for (m, (gv, wv)) in g.iter().zip(w).enumerate() {
                let d = (*gv as f64 - wv).abs();
                if d > worst.0 {
                    worst = (d, t, m);
                }
            }
        }
        assert!(worst.0 < 1e-3, "worst difference {:.2e} at frame {}, band {}", worst.0, worst.1, worst.2);
    }
}
