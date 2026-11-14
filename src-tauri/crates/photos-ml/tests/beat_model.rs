//! The learned beat tracker against the real model: the Python export's activations, then a
//! click track and (optionally) a real recording through the shared decoder.
//!
//! ```sh
//! LUMEN_MODELS_DIR=<folder holding the model files> [LUMEN_BEAT_WAV=<22 050 Hz mono 16-bit wav>] \
//!   cargo test --release -p photos-ml --test beat_model -- --ignored --nocapture
//! ```

use std::path::Path;

use photos_core::beatgrid::track;
use photos_ml::beats::BeatModel;
use photos_ml::manifest::manifest;

fn model() -> BeatModel {
    let dir = std::env::var("LUMEN_MODELS_DIR").expect("LUMEN_MODELS_DIR");
    let entry = manifest().by_role("beat_activation").expect("a beat model in the manifest");
    BeatModel::open(&Path::new(&dir).join(&entry.file), entry).unwrap()
}

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

#[test]
#[ignore = "needs the downloaded model"]
fn matches_the_python_activations() {
    let mut m = model();
    let (beat, down, frame_ms) = m.activations(&reference_signal(m.sample_rate() as usize)).unwrap();
    let reference: serde_json::Value = serde_json::from_str(include_str!("fixtures/mel_reference.json")).unwrap();
    let want = |k: &str| -> Vec<f32> { reference[k].as_array().unwrap().iter().map(|v| v.as_f64().unwrap() as f32).collect() };
    let worst = |a: &[f32], b: &[f32]| a.iter().zip(b).map(|(x, y)| (x - y).abs()).fold(0.0, f32::max);
    println!("frame {frame_ms} ms; worst beat diff {:.2e}, downbeat {:.2e}", worst(&beat, &want("beat")), worst(&down, &want("downbeat")));
    assert!(worst(&beat, &want("beat")) < 1e-2);
    assert!(worst(&down, &want("downbeat")) < 1e-2);
}

/// A 120 BPM click, longer than one chunk, so the stitching is exercised.
#[test]
#[ignore = "needs the downloaded model"]
fn a_click_track_is_read_at_its_tempo() {
    let mut m = model();
    let sr = m.sample_rate() as usize;
    let mut y = vec![0f32; sr * 45];
    for k in 0..90 {
        let start = k * sr / 2;
        for i in 0..2000.min(y.len() - start) {
            let t = i as f32 / sr as f32;
            y[start + i] += 0.6 * (2.0 * std::f32::consts::PI * 1000.0 * t).sin() * (-60.0 * t).exp();
        }
    }
    let (beat, down, ms) = m.activations(&y).unwrap();
    let grid = track(&beat, ms, Some(&down), None);
    println!("click: {:.2} BPM, {} beats, {} downbeats, confidence {:.2}", grid.bpm, grid.beats_ms.len(), grid.downbeats_ms.len(), grid.confidence);
    assert!((grid.bpm - 120.0).abs() < 0.5, "{}", grid.bpm);
}

/// A real recording as a WAV (the test does not decode compressed audio):
/// `ffmpeg -i song.mp3 -ac 1 -ar 22050 -c:a pcm_s16le song.wav`.
#[test]
#[ignore = "needs the downloaded model and a WAV"]
fn a_real_recording() {
    let Ok(wav) = std::env::var("LUMEN_BEAT_WAV") else {
        println!("set LUMEN_BEAT_WAV");
        return;
    };
    let bytes = std::fs::read(wav).unwrap();
    let data = bytes.windows(4).position(|w| w == b"data").expect("a data chunk") + 8;
    let samples: Vec<f32> = bytes[data..].as_chunks::<2>().0.iter().map(|b| i16::from_le_bytes(*b) as f32 / 32_767.0).collect();
    let chunk: usize = std::env::var("LUMEN_BEAT_CHUNK").ok().and_then(|v| v.parse().ok()).unwrap_or(usize::MAX);
    let mut m = model().with_chunk(chunk);
    let t = std::time::Instant::now();
    let (beat, down, ms) = m.activations(&samples).unwrap();
    let grid = track(&beat, ms, Some(&down), None);
    println!(
        "{:.1} s of audio in {:?}: {:.2} BPM, {} beats, {} downbeats, confidence {:.3}",
        samples.len() as f32 / m.sample_rate() as f32,
        t.elapsed(),
        grid.bpm,
        grid.beats_ms.len(),
        grid.downbeats_ms.len(),
        grid.confidence
    );
    assert!(grid.is_steady());
}
