//! Beat detection against audio whose tempo we know, because a pulse train in a unit test
//! is not a recording. FFmpeg synthesises the click, so this needs nothing else:
//!
//! ```sh
//! LUMEN_FFMPEG_DIR=<repo>/src-tauri/binaries \
//!   cargo test -p lumen --test beat_sync -- --ignored --nocapture
//! ```

use std::path::Path;

use photos_core::slideshow::{snap_to_beats, timeline, SlideshowDoc};
use photos_media::Ffmpeg;

/// A decaying click every `beat` seconds: `mod(t, beat)` restarts a fast exponential decay
/// on every beat, which is what a metronome sounds like to an onset detector.
fn click_track(ff: &Ffmpeg, out: &Path, bpm: f32, seconds: u32) {
    let beat = 60.0 / bpm;
    // A comma separates filters in a graph, so the one inside `mod()` has to be escaped or
    // FFmpeg reads the rest of the expression as another filter.
    let expression = format!("0.6*sin(2*PI*880*t)*exp(-14*mod(t\\,{beat:.6}))");
    let made = std::process::Command::new(&ff.ffmpeg)
        .args(["-v", "error", "-y", "-f", "lavfi", "-i"])
        .arg(format!("aevalsrc={expression}:d={seconds}:s=44100"))
        .arg(out)
        .status()
        .unwrap();
    assert!(made.success(), "FFmpeg could not synthesise the click track");
}

#[test]
#[ignore = "synthesises audio; needs FFmpeg"]
fn a_click_track_is_read_at_the_tempo_it_was_made_at() {
    let Some(ff) = Ffmpeg::locate() else {
        println!("no FFmpeg found; set LUMEN_FFMPEG_DIR");
        return;
    };
    let dir = tempfile::tempdir().unwrap();

    for wanted in [90.0f32, 120.0, 140.0] {
        let track = dir.path().join(format!("click-{wanted}.wav"));
        click_track(&ff, &track, wanted, 30);

        let (beats, busy) = photos_media::beats::detect(&ff, &track).unwrap();
        let first = beats.beats_ms[0];
        println!("{wanted} BPM click read as {:.2} BPM, first beat {first} ms, confidence {:.2}, energy {busy:.2}/s", beats.bpm, beats.confidence);
        assert!(beats.is_steady(), "a metronome must read as steady: {beats:?}");
        // No longer quantised to whole frames: the Phase 6d detector read 90 as 90.7.
        assert!((beats.bpm - wanted).abs() < 0.3, "read {:.2} BPM, wanted {wanted}", beats.bpm);

        // And a show cut to it lands on the beat.
        let doc = SlideshowDoc::new_for(&[1, 2, 3, 4, 5]);
        let snapped = snap_to_beats(&doc, beats.bpm, first);
        let beat_ms = 60_000.0 / beats.bpm as f64;
        for (i, start) in timeline(&snapped).starts.iter().enumerate().skip(1) {
            let since = *start as f64;
            let off = ((since / beat_ms).round() * beat_ms - since).abs();
            assert!(off < 15.0, "at {wanted} BPM, cut {i} is {off:.1} ms off the beat");
        }
        // Every clip still reads as a photograph rather than a flash.
        for clip in &snapped.clips {
            assert!(clip.duration_ms >= 1000, "clip of {} ms", clip.duration_ms);
        }
    }
}

#[test]
#[ignore = "synthesises audio; needs FFmpeg"]
fn a_drone_has_no_beat_to_find() {
    let Some(ff) = Ffmpeg::locate() else {
        println!("no FFmpeg found; set LUMEN_FFMPEG_DIR");
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let track = dir.path().join("drone.wav");
    let made = std::process::Command::new(&ff.ffmpeg)
        .args(["-v", "error", "-y", "-f", "lavfi", "-i", "sine=frequency=220:duration=30"])
        .arg(&track)
        .status()
        .unwrap();
    assert!(made.success());

    let (beats, _) = photos_media::beats::detect(&ff, &track).unwrap();
    println!("drone: {:.1} BPM at confidence {:.2}", beats.bpm, beats.confidence);
    assert!(!beats.is_steady(), "an unbroken tone must not pass for a rhythm: {beats:?}");

    // And the show is left exactly as it was.
    let doc = SlideshowDoc::new_for(&[1, 2, 3]);
    assert_eq!(snap_to_beats(&doc, 0.0, 0), doc);
}

/// A 16-bit mono WAV of decaying 1 kHz clicks at `beats_s`, written by hand so the beat
/// times are exactly known.
fn click_wav(path: &Path, beats_s: &[f64], seconds: f64) {
    let sr = 22_050u32;
    let n = (seconds * sr as f64) as usize;
    let mut y = vec![0f32; n];
    for b in beats_s {
        let start = (b * sr as f64) as usize;
        for k in 0..2000.min(n.saturating_sub(start)) {
            let t = k as f32 / sr as f32;
            y[start + k] += 0.6 * (2.0 * std::f32::consts::PI * 1000.0 * t).sin() * (-60.0 * t).exp();
        }
    }
    let mut bytes = Vec::with_capacity(44 + n * 2);
    let data_len = (n * 2) as u32;
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&(36 + data_len).to_le_bytes());
    bytes.extend_from_slice(b"WAVEfmt ");
    bytes.extend_from_slice(&16u32.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes());
    bytes.extend_from_slice(&sr.to_le_bytes());
    bytes.extend_from_slice(&(sr * 2).to_le_bytes());
    bytes.extend_from_slice(&2u16.to_le_bytes());
    bytes.extend_from_slice(&16u16.to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&data_len.to_le_bytes());
    for v in y {
        bytes.extend_from_slice(&((v.clamp(-1.0, 1.0) * 32_767.0) as i16).to_le_bytes());
    }
    std::fs::write(path, bytes).unwrap();
}

/// What the single-tempo detector could never do: follow a tempo that moves. The beat
/// period shrinks from 600 to 480 ms (100 → 125 BPM) over a minute; every beat must be found.
#[test]
#[ignore = "decodes audio; needs FFmpeg"]
fn a_tempo_ramp_is_followed_beat_by_beat() {
    let Some(ff) = Ffmpeg::locate() else {
        println!("no FFmpeg found; set LUMEN_FFMPEG_DIR");
        return;
    };
    let mut beats = vec![0.5f64];
    while *beats.last().unwrap() < 60.0 {
        let t = *beats.last().unwrap();
        beats.push(t + 0.6 - 0.12 * (t / 60.0));
    }
    let dir = tempfile::tempdir().unwrap();
    let track = dir.path().join("ramp.wav");
    click_wav(&track, &beats, 61.0);
    let (grid, _) = photos_media::beats::detect(&ff, &track).unwrap();
    let found: Vec<f64> = grid.beats_ms.iter().map(|b| *b as f64 / 1000.0).collect();
    let inner = &beats[4..beats.len() - 4];
    let worst = inner.iter().map(|b| found.iter().map(|f| (f - b).abs()).fold(f64::MAX, f64::min)).fold(0.0, f64::max);
    println!("ramp: {} beats tracked of {}, worst miss {:.1} ms, confidence {:.2}", found.len(), beats.len(), worst * 1000.0, grid.confidence);
    assert!(worst <= 0.030, "a beat was missed by {:.1} ms", worst * 1000.0);
}

/// F-measure of tracked beats against reference beats, both in seconds, within ±`tol`.
fn f_measure(found: &[f64], reference: &[f64], tol: f64) -> f64 {
    let mut used = vec![false; found.len()];
    let mut hits = 0usize;
    for r in reference {
        if let Some(i) = (0..found.len()).filter(|i| !used[*i] && (found[*i] - r).abs() <= tol).min_by(|a, b| {
            (found[*a] - r).abs().total_cmp(&(found[*b] - r).abs())
        }) {
            used[i] = true;
            hits += 1;
        }
    }
    if found.is_empty() || reference.is_empty() {
        return 0.0;
    }
    let (p, r) = (hits as f64 / found.len() as f64, hits as f64 / reference.len() as f64);
    if p + r == 0.0 { 0.0 } else { 2.0 * p * r / (p + r) }
}

/// A real recording. `LUMEN_BEAT_TRACK=<file>`; with `LUMEN_BEAT_LABELS=<Audacity label
/// file of tapped beats>` it is also scored. The Phase 6d detector refused *What Is Love*
/// (Jamendo 1204669) at confidence 0.16.
#[test]
#[ignore = "needs a real track and FFmpeg"]
fn a_real_track_is_steady() {
    let (Ok(track), Some(ff)) = (std::env::var("LUMEN_BEAT_TRACK"), Ffmpeg::locate()) else {
        println!("set LUMEN_BEAT_TRACK and LUMEN_FFMPEG_DIR");
        return;
    };
    let (grid, busy) = photos_media::beats::detect(&ff, Path::new(&track)).unwrap();
    println!(
        "{track}: {:.2} BPM, {} beats, {} downbeats, confidence {:.3}, energy {busy:.2}/s",
        grid.bpm,
        grid.beats_ms.len(),
        grid.downbeats_ms.len(),
        grid.confidence
    );
    if let Ok(labels) = std::env::var("LUMEN_BEAT_LABELS") {
        let reference: Vec<f64> = std::fs::read_to_string(labels)
            .unwrap()
            .lines()
            .filter_map(|l| l.split_whitespace().next()?.parse().ok())
            .collect();
        let found: Vec<f64> = grid.beats_ms.iter().map(|b| *b as f64 / 1000.0).collect();
        let f = f_measure(&found, &reference, 0.070);
        println!("F-measure at ±70 ms: {f:.3}");
        assert!(f >= 0.8, "F-measure {f:.3}");
    }
    assert!(grid.is_steady(), "confidence {:.3}", grid.confidence);
}
