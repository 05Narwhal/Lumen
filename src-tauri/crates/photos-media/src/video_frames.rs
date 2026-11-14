//! A video seen a couple of times a second (Round 6), for choosing its best part: FFmpeg
//! decodes small RGB frames into a pipe, and each one is measured here — sharpness, exposure,
//! motion against the one before, and whether a new shot starts. What the frames *show*
//! (faces, how attractive) is the ML's part, in the binary; the choice itself is
//! `photos_core::video_highlight`.

use std::path::Path;
use std::process::Stdio;

use photos_core::video_highlight::{Signal, CUT_DISTANCE};

use crate::ffmpeg::{cmd, Ffmpeg, Probe};
use crate::MediaError;

/// Frames are this wide: plenty to judge focus and exposure, cheap to decode and to hold.
pub const SAMPLE_WIDTH: u32 = 192;
/// At most this many samples, however long the video: a long one is sampled more sparsely.
pub const MAX_SAMPLES: u32 = 600;

/// One decoded sample.
#[derive(Debug, Clone)]
pub struct Sample {
    pub t_ms: u32,
    pub width: u32,
    pub height: u32,
    /// Packed RGB, `width × height × 3`.
    pub rgb: Vec<u8>,
}

/// How often a video `duration_ms` long is sampled: twice a second, sparser for long videos.
pub fn sample_every(duration_ms: u32) -> u32 {
    (duration_ms / MAX_SAMPLES).max(500)
}

/// A frame size of `SAMPLE_WIDTH` across with the video's displayed aspect, both even.
fn sample_size(probe: &Probe) -> (u32, u32) {
    let (w, h) = probe.display_size();
    let (w, h) = (w.unwrap_or(16).max(1), h.unwrap_or(9).max(1));
    let height = ((SAMPLE_WIDTH as f64 * h as f64 / w as f64 / 2.0).round() as u32 * 2).clamp(2, SAMPLE_WIDTH * 4);
    (SAMPLE_WIDTH, height)
}

/// Every `sample_every` of the video at `path`, upright (FFmpeg applies the rotation) and
/// tone-mapped first when it is HDR. Falls back to no tone mapping when FFmpeg lacks zimg,
/// as `poster_frame_with` does.
pub fn sample(ff: &Ffmpeg, path: &Path, probe: &Probe, tone_map: bool) -> Result<Vec<Sample>, MediaError> {
    let every = sample_every(probe.duration_ms.unwrap_or(0).clamp(0, u32::MAX as i64) as u32);
    let (w, h) = sample_size(probe);
    let mut chain = Vec::new();
    if tone_map {
        chain.push(Ffmpeg::tone_map_filter().to_string());
    }
    chain.push(format!("fps={:.6}", 1000.0 / every as f64));
    chain.push(format!("scale={w}:{h}:flags=area"));
    let out = cmd(&ff.ffmpeg)
        .args(["-v", "error", "-nostdin", "-i"])
        .arg(path)
        .args(["-map", "0:v:0", "-an", "-vf"])
        .arg(chain.join(","))
        .args(["-f", "rawvideo", "-pix_fmt", "rgb24", "-"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()?;
    if !out.status.success() {
        if tone_map {
            return sample(ff, path, probe, false);
        }
        return Err(MediaError::Decode(format!("video samples: {}", String::from_utf8_lossy(&out.stderr).trim())));
    }
    let frame = (w * h * 3) as usize;
    Ok(out
        .stdout
        .chunks_exact(frame)
        .enumerate()
        .map(|(i, rgb)| Sample { t_ms: i as u32 * every, width: w, height: h, rgb: rgb.to_vec() })
        .collect())
}

fn luma_of(s: &Sample) -> Vec<f32> {
    s.rgb.as_chunks::<3>().0.iter().map(|p| (0.299 * p[0] as f32 + 0.587 * p[1] as f32 + 0.114 * p[2] as f32) / 255.0).collect()
}

/// 8 bins per channel, normalised: what a shot looks like, roughly enough to tell two apart.
fn histogram(s: &Sample) -> [f32; 24] {
    let mut h = [0f32; 24];
    for p in s.rgb.as_chunks::<3>().0 {
        for c in 0..3 {
            h[c * 8 + (p[c] as usize >> 5)] += 1.0;
        }
    }
    let n = (s.rgb.len() / 3).max(1) as f32;
    h.iter_mut().for_each(|v| *v /= n);
    h
}

/// What one sample shows, against the sample before it.
pub fn signal(prev: Option<&Sample>, s: &Sample) -> Signal {
    let y = luma_of(s);
    let (w, h) = (s.width as usize, s.height as usize);
    let luma = y.iter().sum::<f32>() / y.len().max(1) as f32;
    let clipped = y.iter().filter(|v| **v < 0.02 || **v > 0.98).count() as f32 / y.len().max(1) as f32;
    // Variance of the 4-neighbour Laplacian, in 0..255² units like `thumbs::sharpness`.
    let mut lap = Vec::with_capacity(w * h);
    for r in 1..h.saturating_sub(1) {
        for c in 1..w.saturating_sub(1) {
            let i = r * w + c;
            lap.push(255.0 * (4.0 * y[i] - y[i - 1] - y[i + 1] - y[i - w] - y[i + w]));
        }
    }
    let mean = lap.iter().sum::<f32>() / lap.len().max(1) as f32;
    let sharpness = lap.iter().map(|v| (v - mean).powi(2)).sum::<f32>() / lap.len().max(1) as f32;
    let (motion, cut) = match prev.filter(|p| p.rgb.len() == s.rgb.len()) {
        Some(p) => {
            let before = luma_of(p);
            let motion = y.iter().zip(&before).map(|(a, b)| (a - b).abs()).sum::<f32>() / y.len().max(1) as f32;
            let (ha, hb) = (histogram(p), histogram(s));
            // Total variation over three channels' worth of histograms, so 0..1.
            let distance = ha.iter().zip(&hb).map(|(a, b)| (a - b).abs()).sum::<f32>() / 6.0;
            (motion, distance > CUT_DISTANCE)
        }
        None => (0.0, false),
    };
    Signal { t_ms: s.t_ms, sharpness, luma, clipped, motion, cut, ..Default::default() }
}

/// Every sample's signal, in order.
pub fn signals(samples: &[Sample]) -> Vec<Signal> {
    samples.iter().enumerate().map(|(i, s)| signal(i.checked_sub(1).map(|j| &samples[j]), s)).collect()
}

/// The sound's level in dBFS around each of `times`, from the video's audio; `None` when it
/// has none. Decoded at 8 kHz, which is plenty for loudness.
pub fn loudness_at(ff: &Ffmpeg, path: &Path, times: &[u32], every_ms: u32) -> Option<Vec<f32>> {
    const RATE: u32 = 8_000;
    let samples = crate::beats::decode_mono(ff, path, RATE).ok().filter(|s| !s.is_empty())?;
    let half = (RATE as u64 * every_ms as u64 / 2000) as usize;
    Some(
        times
            .iter()
            .map(|t| {
                let at = (*t as u64 * RATE as u64 / 1000) as usize;
                let (a, b) = (at.saturating_sub(half).min(samples.len()), (at + half).min(samples.len()));
                let rms = if b > a { (samples[a..b].iter().map(|v| v * v).sum::<f32>() / (b - a) as f32).sqrt() } else { 0.0 };
                20.0 * rms.max(1e-6).log10()
            })
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(t_ms: u32, f: impl Fn(usize, usize) -> [u8; 3]) -> Sample {
        let (w, h) = (64usize, 36usize);
        let rgb = (0..h).flat_map(|r| (0..w).map(move |c| (r, c))).flat_map(|(r, c)| f(r, c)).collect();
        Sample { t_ms, width: w as u32, height: h as u32, rgb }
    }

    fn checker(t: u32) -> Sample {
        frame(t, |r, c| if (r / 2 + c / 2) % 2 == 0 { [230, 230, 230] } else { [30, 30, 30] })
    }

    fn soft(t: u32) -> Sample {
        frame(t, |_, c| {
            let v = (60 + c * 2) as u8;
            [v, v, v]
        })
    }

    #[test]
    fn a_sharp_frame_is_sharper_than_a_soft_one() {
        assert!(signal(None, &checker(0)).sharpness > 10.0 * signal(None, &soft(0)).sharpness.max(1.0));
    }

    #[test]
    fn a_new_shot_is_a_cut_and_the_same_shot_is_not() {
        let red = frame(500, |_, _| [220, 20, 20]);
        assert!(signal(Some(&checker(0)), &red).cut);
        assert!(!signal(Some(&checker(0)), &checker(500)).cut);
        assert_eq!(signal(Some(&checker(0)), &checker(500)).motion, 0.0);
    }

    #[test]
    fn motion_is_how_much_changed() {
        let moved = frame(500, |r, c| if ((r / 2) + c.div_ceil(2)) % 2 == 0 { [230, 230, 230] } else { [30, 30, 30] });
        assert!(signal(Some(&checker(0)), &moved).motion > 0.1);
    }

    #[test]
    fn exposure_is_measured() {
        let black = signal(None, &frame(0, |_, _| [0, 0, 0]));
        assert!(black.luma < 0.01 && black.clipped > 0.99);
    }

    #[test]
    fn long_videos_are_sampled_more_sparsely() {
        assert_eq!(sample_every(10_000), 500);
        assert_eq!(sample_every(600_000), 1_000);
    }

    #[test]
    #[ignore = "needs FFmpeg"]
    fn a_generated_video_is_sampled_and_its_cut_found() {
        let ff = Ffmpeg::locate().expect("FFmpeg");
        let dir = tempfile::tempdir().unwrap();
        let clip = dir.path().join("cut.mp4");
        let status = cmd(&ff.ffmpeg)
            .args(["-v", "error", "-y", "-f", "lavfi", "-i", "testsrc=duration=4:size=320x180:rate=30", "-f", "lavfi", "-i", "color=c=red:duration=4:size=320x180:rate=30"])
            .args(["-filter_complex", "[0:v][1:v]concat=n=2:v=1:a=0", "-pix_fmt", "yuv420p"])
            .arg(&clip)
            .status()
            .unwrap();
        assert!(status.success());
        let probe = ff.probe(&clip).unwrap();
        let samples = sample(&ff, &clip, &probe, false).unwrap();
        assert!((15..=17).contains(&samples.len()), "{}", samples.len());
        assert_eq!(samples[0].width, SAMPLE_WIDTH);
        let sig = signals(&samples);
        let cuts: Vec<u32> = sig.iter().filter(|s| s.cut).map(|s| s.t_ms).collect();
        assert_eq!(cuts.len(), 1, "{cuts:?}");
        assert!(cuts[0].abs_diff(4_000) <= 500, "{cuts:?}");
        assert!(loudness_at(&ff, &clip, &[0], 500).is_none(), "no sound is not an error");
    }
}
