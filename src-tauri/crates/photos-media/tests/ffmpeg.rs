//! Integration tests that need real FFmpeg binaries. They print a note and pass when
//! `Ffmpeg::locate()` finds nothing, so CI without FFmpeg stays green.

use std::path::Path;
use std::process::Command;

use photos_media::ffmpeg::{poster_time, Ffmpeg};

fn ffmpeg() -> Option<Ffmpeg> {
    let f = Ffmpeg::locate();
    if f.is_none() {
        eprintln!("note: FFmpeg not found (see scripts/fetch-ffmpeg.*); skipping");
    }
    f
}

/// A 2 s, 320×240, 10 fps H.264 test clip with a 440 Hz tone.
fn make_clip(f: &Ffmpeg, out: &Path) {
    let st = Command::new(&f.ffmpeg)
        .args(["-v", "error", "-y", "-f", "lavfi", "-i", "testsrc=size=320x240:rate=10", "-f", "lavfi", "-i", "sine=frequency=440"])
        .args(["-t", "2", "-c:v", "libx264", "-pix_fmt", "yuv420p", "-c:a", "aac", "-shortest"])
        .arg(out)
        .status()
        .unwrap();
    assert!(st.success());
}

#[test]
fn probe_poster_and_proxy() {
    let Some(f) = ffmpeg() else { return };
    assert!(f.version.starts_with("ffmpeg version"), "{}", f.version);
    let dir = tempfile::tempdir().unwrap();
    let clip = dir.path().join("test.mp4");
    make_clip(&f, &clip);

    let p = f.probe(&clip).unwrap();
    assert!((1900..=2100).contains(&p.duration_ms.unwrap()), "{:?}", p.duration_ms);
    assert_eq!(p.video_codec.as_deref(), Some("h264"));
    assert_eq!(p.audio_codec.as_deref(), Some("aac"));
    assert_eq!((p.width, p.height), (Some(320), Some(240)));
    assert_eq!(p.rotation, 0);
    assert!(p.container.as_deref().unwrap().contains("mp4"));

    let poster = dir.path().join("poster.jpg");
    f.poster_frame(&clip, poster_time(p.duration_ms), &poster).unwrap();
    let img = image::open(&poster).unwrap();
    assert_eq!((img.width(), img.height()), (320, 240));

    // Seeking past the end falls back to the first frame instead of failing.
    let late = dir.path().join("late.jpg");
    f.poster_frame(&clip, 99.0, &late).unwrap();
    assert!(late.is_file());

    let proxy = dir.path().join("proxy.mp4");
    let mut last = 0.0f32;
    f.transcode_proxy(&clip, &proxy, &mut |p| last = last.max(p), &|| false).unwrap();
    assert!(last >= 0.9, "progress reached {last}");
    let q = f.probe(&proxy).unwrap();
    assert_eq!(q.video_codec.as_deref(), Some("h264"));
    assert_eq!(q.profile.as_deref(), Some("High"));
    assert_eq!(q.pix_fmt.as_deref(), Some("yuv420p"));
    assert!(!proxy.with_extension("part.mp4").exists());
}

#[test]
fn proxy_transcode_can_be_cancelled() {
    let Some(f) = ffmpeg() else { return };
    let dir = tempfile::tempdir().unwrap();
    let clip = dir.path().join("test.mp4");
    make_clip(&f, &clip);
    let proxy = dir.path().join("proxy.mp4");
    let err = f.transcode_proxy(&clip, &proxy, &mut |_| {}, &|| true).unwrap_err();
    assert!(err.to_string().contains("cancelled"), "{err}");
    assert!(!proxy.exists());
}

#[test]
fn decode_heic_when_a_sample_is_available() {
    let Some(f) = ffmpeg() else { return };
    let Some(sample) = std::env::var_os("LUMEN_TEST_HEIC") else {
        eprintln!("note: set LUMEN_TEST_HEIC=<file.heic> to exercise HEIC decoding; skipping");
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("heic.jpg");
    f.decode_heic(Path::new(&sample), &out).unwrap();
    let img = image::open(&out).unwrap();
    assert!(img.width() > 0 && img.height() > 0);
}

/// A Live Photo's shape: HEVC with PCM sound in a QuickTime file, `secs` long at `size`.
fn make_live_photo(f: &Ffmpeg, out: &Path, size: &str, secs: &str) {
    let st = Command::new(&f.ffmpeg)
        .args(["-v", "error", "-y", "-f", "lavfi", "-i", &format!("testsrc2=size={size}:rate=30"), "-f", "lavfi", "-i", "sine=frequency=440"])
        .args(["-t", secs, "-c:v", "libx265", "-preset", "ultrafast", "-x265-params", "log-level=error", "-tag:v", "hvc1", "-pix_fmt", "yuvj420p", "-c:a", "pcm_s16le", "-shortest"])
        .arg(out)
        .status()
        .unwrap();
    assert!(st.success());
}

/// Round 8A: a video whose only problem is its sound is remuxed, its picture copied, so the
/// proxy is ready in a moment and is the same video.
#[test]
fn a_live_photo_proxy_copies_the_picture() {
    use photos_media::{proxy_plan, ProxyPlan, VideoSupport};
    let Some(f) = ffmpeg() else { return };
    let dir = tempfile::tempdir().unwrap();
    let clip = dir.path().join("IMG_0001.MOV");
    make_live_photo(&f, &clip, "480x640", "2");
    let p = f.probe(&clip).unwrap();
    let plan = proxy_plan(&p, Some(std::fs::metadata(&clip).unwrap().len()), "windows", Some(VideoSupport { hevc: true, hevc_main10: true }));
    assert_eq!(plan, ProxyPlan::Remux { copy_audio: false });
    let proxy = dir.path().join("proxy.mp4");
    f.make_proxy(&clip, &proxy, plan, &[], true, true, &mut |_| {}, &|| false).unwrap();
    let q = f.probe(&proxy).unwrap();
    assert_eq!(q.video_codec.as_deref(), Some("hevc"));
    assert_eq!(q.audio_codec.as_deref(), Some("aac"));
    assert_eq!((q.width, q.height), (Some(480), Some(640)), "the picture is untouched");
    assert!(q.container.as_deref().unwrap().contains("mp4"));
    assert_eq!(photos_media::playback_with(&q, "windows", Some(VideoSupport { hevc: true, hevc_main10: false })), photos_media::Playback::Native);
}

/// How long each kind of proxy takes here, for the plan's *As built*:
/// `cargo test --release -p photos-media --test ffmpeg proxy_speed -- --ignored --nocapture`
#[test]
#[ignore = "a measurement"]
fn proxy_speed() {
    use photos_media::slideshow::{best_encoder, Encoder};
    use photos_media::ProxyPlan;
    let Some(f) = ffmpeg() else { return };
    let dir = tempfile::tempdir().unwrap();
    let gpu = best_encoder(&f, true);
    for (name, size, secs) in [("live photo 3 s 1440x1920", "1440x1920", "3"), ("phone video 30 s 4K", "3840x2160", "30")] {
        let clip = dir.path().join(format!("{}.mov", name.replace(' ', "_")));
        make_live_photo(&f, &clip, size, secs);
        let time = |plan: ProxyPlan, enc: Option<Encoder>| {
            let out = dir.path().join("p.mp4");
            let _ = std::fs::remove_file(&out);
            let video = enc.map(|e| e.proxy_args()).unwrap_or_default();
            let t = std::time::Instant::now();
            f.make_proxy(&clip, &out, plan, &video, true, false, &mut |_| {}, &|| false).unwrap();
            t.elapsed().as_millis()
        };
        let remux = time(ProxyPlan::Remux { copy_audio: false }, None);
        let x264 = time(ProxyPlan::Transcode, Some(Encoder::X264));
        let hw = time(ProxyPlan::Transcode, Some(gpu));
        println!("{name}: remux {remux} ms, x264 veryfast {x264} ms, {} {hw} ms", gpu.ffmpeg_name());
    }
}
