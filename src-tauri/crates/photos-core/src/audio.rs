//! Short-time spectra for the beat tracker. One implementation, used by the DSP front end
//! (spectral flux) and, through `photos-ml`, by the learned one (its log-mel input), so the
//! two never disagree about what a frame is. It follows torchaudio's `MelSpectrogram`:
//! centred frames with reflect padding, a periodic Hann window, Slaney or HTK mel scale.

use rustfft::{num_complex::Complex, FftPlanner};

#[derive(Debug, Clone, PartialEq)]
pub struct MelSpec {
    pub sample_rate: u32,
    pub n_fft: usize,
    pub hop: usize,
    pub n_mels: usize,
    pub fmin: f32,
    pub fmax: f32,
    /// Slaney's mel scale (linear below 1 kHz, logarithmic above); HTK's otherwise.
    pub slaney_scale: bool,
    /// Slaney area normalisation of each triangle; none otherwise.
    pub slaney_norm: bool,
    /// torchaudio's `normalized="frame_length"`, which is `torch.stft(normalized=True)`:
    /// magnitudes divided by √n_fft. (Not by the window's L2 norm — that is `"window"`;
    /// the two differ by √(8/3) for a Hann window, measured against torchaudio.)
    pub window_norm: bool,
    pub power: f32,
}

/// What the DSP front end uses: 22 050 Hz, ~10 ms frames, 64 bands.
pub const DSP_SPEC: MelSpec = MelSpec {
    sample_rate: 22_050,
    n_fft: 1024,
    hop: 220,
    n_mels: 64,
    fmin: 30.0,
    fmax: 8000.0,
    slaney_scale: true,
    slaney_norm: true,
    window_norm: false,
    power: 1.0,
};

const SLANEY_F_SP: f32 = 200.0 / 3.0;
const SLANEY_MIN_LOG_HZ: f32 = 1000.0;

fn slaney_logstep() -> f32 {
    6.4f32.ln() / 27.0
}

fn hz_to_mel(f: f32, slaney: bool) -> f32 {
    if !slaney {
        return 2595.0 * (1.0 + f / 700.0).log10();
    }
    let min_log_mel = SLANEY_MIN_LOG_HZ / SLANEY_F_SP;
    if f >= SLANEY_MIN_LOG_HZ {
        min_log_mel + (f / SLANEY_MIN_LOG_HZ).ln() / slaney_logstep()
    } else {
        f / SLANEY_F_SP
    }
}

fn mel_to_hz(m: f32, slaney: bool) -> f32 {
    if !slaney {
        return 700.0 * (10f32.powf(m / 2595.0) - 1.0);
    }
    let min_log_mel = SLANEY_MIN_LOG_HZ / SLANEY_F_SP;
    if m >= min_log_mel {
        SLANEY_MIN_LOG_HZ * (slaney_logstep() * (m - min_log_mel)).exp()
    } else {
        SLANEY_F_SP * m
    }
}

/// `n_mels` triangles over the `n_fft / 2 + 1` FFT bins.
pub fn mel_filterbank(spec: &MelSpec) -> Vec<Vec<f32>> {
    let bins = spec.n_fft / 2 + 1;
    let nyquist = spec.sample_rate as f32 / 2.0;
    let fft_hz: Vec<f32> = (0..bins).map(|i| nyquist * i as f32 / (bins - 1) as f32).collect();
    let (lo, hi) = (hz_to_mel(spec.fmin, spec.slaney_scale), hz_to_mel(spec.fmax, spec.slaney_scale));
    let pts: Vec<f32> = (0..spec.n_mels + 2)
        .map(|i| mel_to_hz(lo + (hi - lo) * i as f32 / (spec.n_mels + 1) as f32, spec.slaney_scale))
        .collect();
    (0..spec.n_mels)
        .map(|m| {
            let (l, c, r) = (pts[m], pts[m + 1], pts[m + 2]);
            let norm = if spec.slaney_norm { 2.0 / (r - l) } else { 1.0 };
            fft_hz
                .iter()
                .map(|f| {
                    let up = (f - l) / (c - l);
                    let down = (r - f) / (r - c);
                    up.min(down).max(0.0) * norm
                })
                .collect()
        })
        .collect()
}

/// Mel magnitudes per frame (frames × n_mels).
pub fn mel_frames(samples: &[f32], spec: &MelSpec) -> Vec<Vec<f32>> {
    let bank = mel_filterbank(spec);
    // Each triangle covers a few bins of the 513; multiplying only those is most of the
    // difference between seconds and a fraction of one on a four-minute track.
    let spans: Vec<(usize, usize)> = bank
        .iter()
        .map(|row| {
            let first = row.iter().position(|w| *w > 0.0).unwrap_or(0);
            let last = row.iter().rposition(|w| *w > 0.0).map(|i| i + 1).unwrap_or(first);
            (first, last)
        })
        .collect();
    let n = spec.n_fft;
    let half = n / 2;
    let window: Vec<f32> = (0..n).map(|i| 0.5 - 0.5 * (2.0 * std::f32::consts::PI * i as f32 / n as f32).cos()).collect();
    let scale = if spec.window_norm { 1.0 / (n as f32).sqrt() } else { 1.0 };
    let len = samples.len() as isize;
    // Reflect padding, as torch.stft(center=True, pad_mode="reflect").
    let at = |i: isize| -> f32 {
        if len == 0 {
            return 0.0;
        }
        let mut j = i;
        while j < 0 || j >= len {
            if j < 0 {
                j = -j;
            }
            if j >= len {
                j = 2 * (len - 1) - j;
            }
            if len == 1 {
                j = 0;
            }
        }
        samples[j as usize]
    };
    let frames = samples.len() / spec.hop + 1;
    let fft = FftPlanner::<f32>::new().plan_fft_forward(n);
    let mut buf = vec![Complex::new(0.0, 0.0); n];
    let mut out = Vec::with_capacity(frames);
    for f in 0..frames {
        let start = (f * spec.hop) as isize - half as isize;
        for (k, slot) in buf.iter_mut().enumerate() {
            *slot = Complex::new(at(start + k as isize) * window[k], 0.0);
        }
        fft.process(&mut buf);
        let mag: Vec<f32> = buf[..=half].iter().map(|c| (c.norm() * scale).powf(spec.power)).collect();
        out.push(bank.iter().zip(&spans).map(|(row, (a, b))| row[*a..*b].iter().zip(&mag[*a..*b]).map(|(w, m)| w * m).sum()).collect());
    }
    out
}

/// Onset strength (half-wave-rectified log-mel difference minus a ±0.5 s local mean) and
/// low-band (below 150 Hz) onset energy per frame, for the DSP front end. Returns the frame
/// length in ms as well.
pub fn spectral_flux(samples: &[f32], sample_rate: u32) -> (Vec<f32>, Vec<f32>, f32) {
    let spec = MelSpec { sample_rate, ..DSP_SPEC };
    let frame_ms = spec.hop as f32 * 1000.0 / sample_rate as f32;
    let mel: Vec<Vec<f32>> =
        mel_frames(samples, &spec).into_iter().map(|f| f.into_iter().map(|v| (1.0 + 1000.0 * v).ln()).collect()).collect();
    let bank = mel_filterbank(&spec);
    let bin_hz = sample_rate as f32 / spec.n_fft as f32;
    // A band counts as low when most of its weight sits below 150 Hz.
    let low: Vec<bool> = bank
        .iter()
        .map(|row| {
            let total: f32 = row.iter().sum();
            let below: f32 = row.iter().enumerate().filter(|(i, _)| (*i as f32) * bin_hz < 150.0).map(|(_, w)| w).sum();
            total > 0.0 && below / total > 0.5
        })
        .collect();
    let mut flux = vec![0f32; mel.len()];
    let mut low_band = vec![0f32; mel.len()];
    for t in 1..mel.len() {
        for (b, (now, before)) in mel[t].iter().zip(&mel[t - 1]).enumerate() {
            let rise = (now - before).max(0.0);
            flux[t] += rise;
            if low[b] {
                low_band[t] += rise;
            }
        }
    }
    let w = (500.0 / frame_ms) as usize;
    let mut prefix = vec![0f32; flux.len() + 1];
    for (i, v) in flux.iter().enumerate() {
        prefix[i + 1] = prefix[i] + v;
    }
    let onset = (0..flux.len())
        .map(|i| {
            let (a, b) = (i.saturating_sub(w), (i + w + 1).min(flux.len()));
            (flux[i] - (prefix[b] - prefix[a]) / (b - a) as f32).max(0.0)
        })
        .collect();
    (onset, low_band, frame_ms)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_band_is_a_proper_triangle() {
        let bank = mel_filterbank(&DSP_SPEC);
        assert_eq!(bank.len(), 64);
        for (m, row) in bank.iter().enumerate() {
            assert_eq!(row.len(), 513);
            assert!(row.iter().all(|w| *w >= 0.0));
            assert!(row.iter().any(|w| *w > 0.0), "band {m} is empty");
        }
    }

    #[test]
    fn a_click_every_half_second_is_an_onset_every_half_second() {
        let sr = 22_050;
        let mut y: Vec<f32> = (0..sr * 4).map(|i| 0.05 * (2.0 * std::f32::consts::PI * 220.0 * i as f32 / sr as f32).sin()).collect();
        for k in 0..8 {
            for s in 0..64 {
                y[k * sr / 2 + s] += 0.8;
            }
        }
        let (onset, _, frame_ms) = spectral_flux(&y, sr as u32);
        for k in 1..8 {
            let at = (k as f32 * 500.0 / frame_ms).round() as usize;
            let window = &onset[at.saturating_sub(3)..at + 4];
            let peak = window.iter().copied().fold(0.0, f32::max);
            let elsewhere = onset[at + 10..at + 30].iter().copied().fold(0.0, f32::max);
            assert!(peak > 3.0 * elsewhere.max(1e-6), "click {k}: {peak} vs {elsewhere}");
        }
    }
}
