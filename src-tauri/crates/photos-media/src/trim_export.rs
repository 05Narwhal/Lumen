//! A trimmed video, written out (Phase 8 spec §5). The trim is a list of kept segments; the
//! original is only ever FFmpeg's input.
//!
//! **Precise** re-encodes the kept segments with `trim`/`atrim` and `concat`, so every cut is
//! on the frame asked for: H.264 (CRF 18), or HEVC when the source is HEVC, and AAC. Decoding
//! applies the source's rotation, so the result is upright with none.
//! **Fast** copies the streams through the concat demuxer with `inpoint`/`outpoint`: no
//! quality lost and nearly instant, but each segment starts at the key frame at or before its
//! start. The rotation survives the copy.
//!
//! Neither keeps the container's tags (the concat demuxer has none to give), so the caller
//! carries them afterwards with ExifTool, as for photos: measured with FFmpeg 9.0.1 and
//! ExifTool 13.59, `-TagsFromFile <original> -all:all` restores QuickTime's creation date and
//! the `Keys` location, make and model, and does not copy `Rotation`.

use std::ffi::OsString;
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::Stdio;

use photos_core::edits::Segment;
use photos_core::export::VideoCut;

use crate::ffmpeg::{cmd, Probe};
use crate::{Ffmpeg, MediaError};

/// Seconds as FFmpeg reads them, to the millisecond.
fn secs(ms: i64) -> String {
    format!("{}.{:03}", ms / 1000, ms % 1000)
}

/// Stream copy needs a container that can hold the source's streams and that ExifTool can
/// write the tags back into: QuickTime and MP4.
pub fn can_copy(probe: &Probe) -> bool {
    probe.container.as_deref().is_some_and(|c| c.split(',').any(|f| f == "mov" || f == "mp4"))
}

/// The cut actually made: fast only where it can be.
pub fn effective_cut(cut: VideoCut, probe: &Probe) -> VideoCut {
    if cut == VideoCut::Fast && can_copy(probe) {
        VideoCut::Fast
    } else {
        VideoCut::Precise
    }
}

/// The extension a trimmed export is written with: a QuickTime source stays `.mov` (its
/// `Keys` and `UserData` live there), everything else becomes `.mp4`; a fast copy keeps the
/// source's own.
pub fn trimmed_extension(source_ext: &str, cut: VideoCut) -> &'static str {
    let e = source_ext.to_ascii_lowercase();
    match (cut, e.as_str()) {
        (_, "mov") => "mov",
        (VideoCut::Fast, "m4v") => "m4v",
        _ => "mp4",
    }
}

/// The concat demuxer's script for a fast cut. Inside single quotes nothing is special but
/// the quote itself, which is written `'\''`.
pub fn concat_script(input: &Path, segments: &[Segment]) -> String {
    let path = input.to_string_lossy().replace('\'', r"'\''");
    let mut s = String::from("ffconcat version 1.0\n");
    for seg in segments {
        s.push_str(&format!("file '{path}'\ninpoint {}\noutpoint {}\n", secs(seg.start_ms), secs(seg.end_ms)));
    }
    s
}

/// The filter graph for a precise cut: one `trim` (and `atrim`) per segment, joined by
/// `concat`. Timestamps restart at zero per segment so they join without gaps.
pub fn precise_graph(segments: &[Segment], audio: bool) -> String {
    let mut g = String::new();
    let mut joins = String::new();
    for (i, s) in segments.iter().enumerate() {
        let (a, b) = (secs(s.start_ms), secs(s.end_ms));
        g.push_str(&format!("[0:v:0]trim=start={a}:end={b},setpts=PTS-STARTPTS[v{i}];"));
        joins.push_str(&format!("[v{i}]"));
        if audio {
            g.push_str(&format!("[0:a:0]atrim=start={a}:end={b},asetpts=PTS-STARTPTS[a{i}];"));
            joins.push_str(&format!("[a{i}]"));
        }
    }
    g.push_str(&format!("{joins}concat=n={}:v=1:a={}[v]{}", segments.len(), audio as u8, if audio { "[a]" } else { "" }));
    g
}

/// FFmpeg's arguments after the input for a precise cut.
pub fn precise_args(segments: &[Segment], probe: &Probe) -> Vec<OsString> {
    let audio = probe.audio_codec.is_some();
    let hevc = probe.video_codec.as_deref() == Some("hevc");
    let deep = probe.pix_fmt.as_deref().is_some_and(|p| p.contains("10"));
    let mut a: Vec<String> = vec!["-filter_complex".into(), precise_graph(segments, audio), "-map".into(), "[v]".into()];
    if audio {
        a.extend(["-map".into(), "[a]".into(), "-c:a".into(), "aac".into(), "-b:a".into(), "192k".into()]);
    }
    if hevc {
        // `hvc1` is the tag Apple's players require for HEVC in MP4 and QuickTime.
        let pix = if deep { "yuv420p10le" } else { "yuv420p" };
        a.extend(["-c:v", "libx265", "-crf", "20", "-preset", "medium", "-tag:v", "hvc1", "-pix_fmt", pix].map(String::from));
    } else {
        a.extend(["-c:v", "libx264", "-crf", "18", "-preset", "medium", "-profile:v", "high", "-pix_fmt", "yuv420p"].map(String::from));
    }
    // The source's colour, declared again (HDR stays HDR in its tags).
    for (flag, value) in [("-color_primaries", &probe.color_primaries), ("-color_trc", &probe.color_transfer)] {
        if let Some(v) = value {
            a.extend([flag.to_string(), v.clone()]);
        }
    }
    a.extend(["-map_metadata", "0", "-movflags", "use_metadata_tags+faststart"].map(String::from));
    a.into_iter().map(OsString::from).collect()
}

impl Ffmpeg {
    /// Write the kept `segments` of `input` to `out` (the caller's temporary file, whose
    /// extension picks the container). `cancel` is polled per progress line.
    pub fn export_trimmed(&self, input: &Path, segments: &[Segment], cut: VideoCut, out: &Path, cancel: &dyn Fn() -> bool) -> Result<VideoCut, MediaError> {
        if segments.is_empty() {
            return Err(MediaError::Unsupported("nothing is kept".into()));
        }
        let probe = self.probe(input)?;
        let cut = effective_cut(cut, &probe);
        let mut c = cmd(&self.ffmpeg);
        c.args(["-v", "error", "-y", "-nostdin"]);
        let mut script = None;
        match cut {
            VideoCut::Fast => {
                let list = out.with_extension("ffconcat");
                std::fs::File::create(&list)?.write_all(concat_script(input, segments).as_bytes())?;
                c.args(["-f", "concat", "-safe", "0", "-i"]).arg(&list);
                c.args(["-map", "0", "-c", "copy", "-movflags", "+faststart"]);
                script = Some(list);
            }
            VideoCut::Precise => {
                c.arg("-i").arg(input).args(precise_args(segments, &probe));
            }
        }
        let result = run(c.args(["-progress", "pipe:1"]).arg(out), cancel);
        if let Some(list) = script {
            let _ = std::fs::remove_file(list);
        }
        result.map(|()| cut)
    }
}

fn run(c: &mut std::process::Command, cancel: &dyn Fn() -> bool) -> Result<(), MediaError> {
    let mut child = c.stdout(Stdio::piped()).stderr(Stdio::piped()).spawn()?;
    photos_core::child_jobs::adopt(&child);
    let stdout = child.stdout.take().expect("piped");
    for line in BufReader::new(stdout).lines() {
        line?;
        if cancel() {
            let _ = child.kill();
            let _ = child.wait();
            return Err(MediaError::Decode("cancelled".into()));
        }
    }
    let output = child.wait_with_output()?;
    if !output.status.success() {
        return Err(MediaError::Decode(format!("trimmed export: {}", String::from_utf8_lossy(&output.stderr).trim())));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seg(a: i64, b: i64) -> Segment {
        Segment { start_ms: a, end_ms: b }
    }

    #[test]
    fn a_precise_graph_trims_each_segment_and_joins_them() {
        assert_eq!(
            precise_graph(&[seg(1000, 2500), seg(3500, 6000)], true),
            "[0:v:0]trim=start=1.000:end=2.500,setpts=PTS-STARTPTS[v0];[0:a:0]atrim=start=1.000:end=2.500,asetpts=PTS-STARTPTS[a0];\
             [0:v:0]trim=start=3.500:end=6.000,setpts=PTS-STARTPTS[v1];[0:a:0]atrim=start=3.500:end=6.000,asetpts=PTS-STARTPTS[a1];\
             [v0][a0][v1][a1]concat=n=2:v=1:a=1[v][a]"
        );
        assert_eq!(precise_graph(&[seg(0, 40)], false), "[0:v:0]trim=start=0.000:end=0.040,setpts=PTS-STARTPTS[v0];[v0]concat=n=1:v=1:a=0[v]");
    }

    #[test]
    fn hevc_stays_hevc_and_everything_else_becomes_h264() {
        let text = |p: &Probe| precise_args(&[seg(0, 1000)], p).iter().map(|s| s.to_string_lossy().into_owned()).collect::<Vec<_>>().join(" ");
        let hevc = Probe { video_codec: Some("hevc".into()), pix_fmt: Some("yuv420p10le".into()), color_transfer: Some("arib-std-b67".into()), ..Default::default() };
        let t = text(&hevc);
        assert!(t.contains("-c:v libx265") && t.contains("-tag:v hvc1") && t.contains("yuv420p10le") && t.contains("-color_trc arib-std-b67"), "{t}");
        assert!(!t.contains("[a]"), "no audio stream, no audio graph");
        let t = text(&Probe { video_codec: Some("vp9".into()), audio_codec: Some("opus".into()), ..Default::default() });
        assert!(t.contains("-c:v libx264") && t.contains("-c:a aac") && t.contains("-map_metadata 0"), "{t}");
    }

    #[test]
    fn fast_cuts_need_a_quicktime_family_container() {
        let mov = Probe { container: Some("mov,mp4,m4a,3gp,3g2,mj2".into()), ..Default::default() };
        let mkv = Probe { container: Some("matroska,webm".into()), ..Default::default() };
        assert_eq!(effective_cut(VideoCut::Fast, &mov), VideoCut::Fast);
        assert_eq!(effective_cut(VideoCut::Fast, &mkv), VideoCut::Precise);
        assert_eq!(trimmed_extension("MOV", VideoCut::Precise), "mov");
        assert_eq!(trimmed_extension("mkv", VideoCut::Precise), "mp4");
        assert_eq!(trimmed_extension("m4v", VideoCut::Fast), "m4v");
    }

    #[test]
    fn the_concat_script_quotes_paths_safely() {
        let s = concat_script(Path::new("D:/Clips/Anna's day.mov"), &[seg(1000, 2500)]);
        assert_eq!(s, "ffconcat version 1.0\nfile 'D:/Clips/Anna'\\''s day.mov'\ninpoint 1.000\noutpoint 2.500\n");
    }

    /// A 6 s clip with QuickTime tags an iPhone writes, turned 90°.
    fn clip(ff: &Ffmpeg, dir: &Path) -> std::path::PathBuf {
        let src = dir.join("clip.mov");
        let ok = cmd(&ff.ffmpeg)
            .args(["-v", "error", "-y", "-f", "lavfi", "-i", "testsrc=size=320x240:rate=30:duration=6", "-f", "lavfi", "-i", "sine=frequency=440:duration=6"])
            .args(["-c:v", "libx264", "-g", "30", "-pix_fmt", "yuv420p", "-c:a", "aac", "-shortest", "-metadata", "creation_time=2024-05-01T10:00:00Z"])
            .arg(&src)
            .status()
            .unwrap();
        assert!(ok.success());
        src
    }

    #[test]
    #[ignore = "needs FFmpeg and ExifTool"]
    fn both_cuts_keep_the_right_length_and_the_originals_tags() {
        use crate::metadata_transfer::{transfer, verify, Expect, Target};
        let ff = Ffmpeg::locate().expect("FFmpeg");
        let et = crate::ExifTool::locate().expect("ExifTool");
        let dir = tempfile::tempdir().unwrap();
        let src = clip(&ff, dir.path());
        let mut s = et.session().unwrap();
        let out = s.run(["-overwrite_original", "-Keys:GPSCoordinates=46.0512, 14.5058, 295", "-Keys:Make=Apple", "-Keys:Model=iPhone 15", "-Rotation=90", src.to_str().unwrap()]).unwrap();
        assert!(out.errors().is_empty(), "{out:?}");
        let before = std::fs::read(&src).unwrap();
        let turned = ff.probe(&src).unwrap().rotation;
        assert_ne!(turned, 0);
        let segments = [seg(1000, 2500), seg(3500, 6000)];
        for (cut, rotation, tolerance_ms) in [(VideoCut::Precise, 0, 34), (VideoCut::Fast, turned, 120)] {
            let out = dir.path().join(format!("{cut:?}.mov"));
            assert_eq!(ff.export_trimmed(&src, &segments, cut, &out, &|| false).unwrap(), cut);
            transfer(&mut s, &src, &out, Target::Video, &Default::default(), None).unwrap();
            let lost = verify(&mut s, &src, &out, &Expect { target: Some(Target::Video), ..Default::default() }).unwrap();
            assert!(lost.is_empty(), "{cut:?}: {lost:#?}");
            let p = ff.probe(&out).unwrap();
            let ms = p.duration_ms.unwrap();
            assert!((ms - 4000).abs() <= tolerance_ms, "{cut:?}: {ms} ms");
            assert_eq!(p.rotation, rotation, "{cut:?}");
            if cut == VideoCut::Precise {
                assert_eq!(p.display_size(), (Some(240), Some(320)), "upright pixels");
            }
            assert!(!out.with_extension("ffconcat").exists());
        }
        assert_eq!(std::fs::read(&src).unwrap(), before, "the original is only ever the input");
    }
}
