//! A slideshow exported with music from the library: the film has a soundtrack, and the
//! credit its licence demands lands beside it. Needs FFmpeg, nothing else — the photos and
//! the song are made here, so this runs anywhere the sidecar does:
//!
//! ```sh
//! LUMEN_FFMPEG_DIR=<repo>/src-tauri/binaries \
//!   cargo test -p lumen --test slideshow_music -- --ignored --nocapture
//! ```

use std::path::{Path, PathBuf};

use lumen_lib::music::{self, MusicTrack};
use lumen_lib::slideshows::write_attribution;
use photos_core::slideshow::{build, timeline, AudioTrack, FrameKind, LayoutStyle, PhotoShape, SlideshowDoc};
use photos_media::Ffmpeg;

/// A flat colour of the given size, which is all the exporter needs of a still.
fn still(dir: &Path, name: &str, w: u32, h: u32, rgb: [u8; 3]) -> PathBuf {
    let img = image::RgbImage::from_pixel(w, h, image::Rgb(rgb));
    let path = dir.join(name);
    photos_media::encode_jpeg(&image::DynamicImage::ImageRgb8(img), 92, &path).unwrap();
    path
}

/// The Creative Commons track from the fixture, saved into `<library>/music` with real
/// audio bytes, exactly as the picker's *Use* does it.
fn saved_song(ff: &Ffmpeg, library: &Path) -> (music::SavedTrack, MusicTrack) {
    let raw = library.join("song-source.mp3");
    let made = std::process::Command::new(&ff.ffmpeg)
        .args(["-v", "error", "-y", "-f", "lavfi", "-i", "sine=frequency=330:duration=30"])
        .arg(&raw)
        .status()
        .unwrap();
    assert!(made.success(), "FFmpeg could not make the test song");
    let bytes = std::fs::read(&raw).unwrap();

    let track = music::parse_tracks(include_str!("jamendo_fixture.json")).unwrap().remove(0);
    let saved = music::write_saved(library, &track, &bytes).unwrap();
    (saved, track)
}

#[test]
#[ignore = "renders a real MP4; needs FFmpeg"]
fn an_exported_show_carries_its_music_and_its_credit() {
    let ff = match Ffmpeg::locate() {
        Some(ff) => ff,
        None => {
            println!("no FFmpeg found; set LUMEN_FFMPEG_DIR");
            return;
        }
    };
    let dir = tempfile::tempdir().unwrap();
    let library = dir.path();

    let (saved, track) = saved_song(&ff, library);
    // The path is the one the picker hands the editor: inside the library's music folder.
    assert!(Path::new(&saved.path).starts_with(music::music_dir(library)), "{}", saved.path);

    // Four portraits, then a landscape with two more: asked to group, that is a quad and
    // then a feature — the layouts added in 6d, reaching a real FFmpeg run. (The quad takes
    // the first four it can, so the landscape has to come after them to lead a feature.)
    let shapes = [(900, 1600), (900, 1600), (900, 1600), (900, 1600), (1600, 1200), (1200, 1600), (1200, 1600)];
    let photos: Vec<(PathBuf, PhotoShape)> = shapes
        .iter()
        .enumerate()
        .map(|(i, (w, h))| {
            let tint = [(40 + i * 30) as u8, (90 + i * 20) as u8, (180 - i * 15) as u8];
            (still(library, &format!("p{i}.jpg"), *w, *h, tint), PhotoShape { id: i as i64 + 1, aspect: *w as f32 / *h as f32, video: None })
        })
        .collect();
    let doc = SlideshowDoc {
        audio: Some(AudioTrack { path: saved.path.clone(), fade_in_ms: 1000, fade_out_ms: 1500, gain_db: -4.0, start_ms: 0 }),
        ..build(&photos.iter().map(|(_, s)| *s).collect::<Vec<_>>(), LayoutStyle::Grid)
    };
    let frames: Vec<FrameKind> = doc.clips.iter().map(|c| c.frame).collect();
    assert!(frames.contains(&FrameKind::Feature), "the grouping style should make a feature: {frames:?}");
    assert!(frames.contains(&FrameKind::Quad), "and a quad: {frames:?}");
    println!("frames: {frames:?}");

    // One composed still per clip, exactly as the export job does it.
    let frame_aspect = doc.resolution[0] as f32 / doc.resolution[1] as f32;
    let stills: Vec<PathBuf> = doc
        .clips
        .iter()
        .enumerate()
        .map(|(i, clip)| {
            let images: Vec<image::DynamicImage> =
                clip.assets().iter().map(|id| image::open(&photos[*id as usize - 1].0).unwrap()).collect();
            let aspects: Vec<f32> = clip.assets().iter().map(|id| photos[*id as usize - 1].1.aspect).collect();
            let placements = photos_core::slideshow::frame_layout(clip.frame, &aspects, frame_aspect);
            let composed = photos_media::frame::compose(&placements, &images, doc.resolution[0] * 2, doc.resolution[1] * 2);
            let out = library.join(format!("clip{i}.jpg"));
            photos_media::encode_jpeg(&composed, 92, &out).unwrap();
            out
        })
        .collect();

    let out = library.join("exports").join("Show.mp4");
    std::fs::create_dir_all(out.parent().unwrap()).unwrap();
    photos_media::slideshow::render_slideshow(&ff, &doc, &stills, Some(Path::new(&saved.path)), &out, &mut |_| {}, &|| false)
        .unwrap();

    // The film exists, it has sound, and it lasts exactly as long as the timeline promised
    // — which is the check that composed frames do not disturb the graph's arithmetic.
    let probe = ff.probe(&out).unwrap();
    let expected = timeline(&doc).total_ms as i64;
    let got = probe.duration_ms.unwrap_or(0);
    assert!((got - expected).abs() <= 100, "exported {got} ms, timeline {expected} ms");
    assert!(probe.audio_codec.is_some(), "the export is silent: {probe:?}");
    println!("exported {} ms, {} bytes", probe.duration_ms.unwrap_or(0), std::fs::metadata(&out).unwrap().len());

    // And the credit the licence asks for sits beside it.
    let credit = write_attribution(&out, &doc, library).expect("an attribution file");
    assert_eq!(credit, library.join("exports").join("Show.mp4.attribution.txt"));
    let text = std::fs::read_to_string(&credit).unwrap();
    println!("--- {} ---\n{text}", credit.display());
    assert!(text.contains(&track.title), "names the track: {text}");
    assert!(text.contains(&track.artist), "names the artist: {text}");
    assert!(text.contains(&track.licence_name), "names the licence: {text}");
    assert!(text.contains(&track.share_url), "links back to Jamendo: {text}");

    // A song the user brought themselves is nobody's to credit.
    let own = SlideshowDoc {
        audio: Some(AudioTrack { path: library.join("song-source.mp3").display().to_string(), ..doc.audio.clone().unwrap() }),
        ..doc
    };
    assert!(write_attribution(&library.join("exports").join("Own.mp4"), &own, library).is_none());
}
