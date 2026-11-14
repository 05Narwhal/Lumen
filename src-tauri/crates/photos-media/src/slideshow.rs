//! A slideshow document as one FFmpeg run: every clip a still decoded once and repeated, its
//! Ken Burns drawn by `perspective` at sub-pixel precision (eased exactly as
//! `photos_core::slideshow::motion_at`), joined
//! by `xfade` where clips overlap and `concat` where they do not, with the audio trimmed,
//! faded and set to its gain. Timing comes from `photos_core::slideshow::timeline`, the same
//! function the preview is tested against.

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::Stdio;

use photos_core::slideshow::{frame_at, segment_bounds, timeline, Clip, FrameKind, MotionKind, SlideshowDoc, TransitionKind};

use crate::ffmpeg::{cmd, Ffmpeg};
use crate::MediaError;

fn secs(ms: u32) -> String {
    format!("{:.3}", ms as f64 / 1000.0)
}

/// `a + (b − a)·ease(u)` as an FFmpeg expression, `u = on / (frames − 1)`; `ease` is
/// smoothstep, as in `photos_core::slideshow::ease`. `on` is the frame number, `(on+skip)`
/// when an earlier piece of a parallel export already made `skip` of the clip's frames.
fn eased_at(a: f32, b: f32, frames: u32, on: &str) -> String {
    let u = format!("min({on}/{},1)", frames.saturating_sub(1).max(1));
    format!("{a:.6}+({:.6})*({u})*({u})*(3-2*({u}))", b - a)
}

/// Everything after the binary's name: inputs, `-filter_complex`, encoding, `out`.
/// Our transitions, one to one onto FFmpeg's. `None` never reaches here: it is a `concat`.
pub fn xfade_name(kind: TransitionKind) -> &'static str {
    match kind {
        TransitionKind::None | TransitionKind::Crossfade => "fade",
        TransitionKind::FadeBlack => "fadeblack",
        TransitionKind::FadeWhite => "fadewhite",
        TransitionKind::Slide => "slideleft",
        TransitionKind::SlideUp => "slideup",
        TransitionKind::Wipe => "wipeleft",
        TransitionKind::WipeUp => "wipeup",
        TransitionKind::Circle => "circleopen",
        TransitionKind::Zoom => "zoomin",
        TransitionKind::SlideDown => "slidedown",
        TransitionKind::Cover => "coverleft",
        TransitionKind::Reveal => "revealleft",
        TransitionKind::CircleClose => "circleclose",
        TransitionKind::VertOpen => "vertopen",
        TransitionKind::HorzOpen => "horzopen",
        TransitionKind::Corner => "wipetl",
        TransitionKind::Squeeze => "squeezeh",
        TransitionKind::Slices => "hlslice",
        TransitionKind::VSlices => "vuslice",
    }
}

/// Clip `i`'s stream: `frames − skip` frames of its motion from the `skip`-th on. The still
/// is covered to twice the frame, converted to 4:2:0 and decoded once, then repeated (`loop`).
/// Ken Burns is `perspective` stretching the view rectangle, at sub-pixel precision, over
/// the 2× frame, scaled down to the output: `zoompan` placed the view at whole pixels and
/// rounded its size every frame, so pans stepped and zooms wobbled (Round 6).
fn clip_filter(input: usize, label: &str, clip: &Clip, frames: u32, skip: u32, [w, h]: [u32; 2], fps: u32) -> String {
    if clip.video.is_some() {
        // A video clip arrives already framed at the output size (`video_clip_args`): its
        // frames are taken as they are, the last one held should the video end first.
        return format!(
            "[{input}:v]fps={fps},scale={w}:{h},setsar=1,tpad=stop_mode=clone:stop_duration={pad},trim=start_frame={skip}:end_frame={frames},setpts=PTS-STARTPTS,settb=1/{fps},format=yuv420p[{label}]",
            pad = secs(clip.duration_ms),
        );
    }
    let d = frames.saturating_sub(skip).max(1);
    let (sw, sh) = (w * 2, h * 2);
    let repeat = |size: String| format!("{size},loop=loop={}:size=1:start=0,settb=1/{fps},setpts=N,fps={fps}", d - 1);
    if clip.motion.kind == MotionKind::None {
        let still = repeat(format!("scale={w}:{h}:force_original_aspect_ratio=increase:flags=bicubic,crop={w}:{h},setsar=1,format=yuv420p"));
        return format!("[{input}:v]{still}[{label}]");
    }
    let (from, to) = (clip.motion.from, clip.motion.to);
    let on = if skip == 0 { "on".to_string() } else { format!("(on+{skip})") };
    let z = eased_at(from[2], to[2], frames, &on);
    // The view is `1/zoom` of the frame wide, centred at `0.5 + x` of it.
    let (cx, cy) = (format!("(0.5+{})", eased_at(from[0], to[0], frames, &on)), format!("(0.5+{})", eased_at(from[1], to[1], frames, &on)));
    let (x0, x1) = (format!("W*({cx}-0.5/({z}))"), format!("W*({cx}+0.5/({z}))"));
    let (y0, y1) = (format!("H*({cy}-0.5/({z}))"), format!("H*({cy}+0.5/({z}))"));
    let still = repeat(format!("scale={sw}:{sh}:force_original_aspect_ratio=increase,crop={sw}:{sh},setsar=1,format=yuv420p"));
    format!(
        "[{input}:v]{still},perspective=x0='{x0}':y0='{y0}':x1='{x1}':y1='{y0}':x2='{x0}':y2='{y1}':x3='{x1}':y3='{y1}':interpolation=linear:sense=source:eval=frame,scale={w}:{h}:flags=bicubic[{label}]"
    )
}

/// The track trimmed so its first beat lands on the first frame, padded with silence when it
/// is shorter than the show (never looped or cut short), faded and set to its gain.
fn audio_chain(doc: &SlideshowDoc, input: usize) -> Option<String> {
    let track = doc.audio.as_ref()?;
    let total = timeline(doc).total_ms;
    let start = track.start_ms;
    let mut chain = format!(
        "[{input}:a]atrim={}:{},asetpts=PTS-STARTPTS,apad=whole_dur={}",
        secs(start),
        secs(start.saturating_add(total)),
        secs(total)
    );
    if track.fade_in_ms > 0 {
        chain.push_str(&format!(",afade=t=in:st=0:d={}", secs(track.fade_in_ms)));
    }
    if track.fade_out_ms > 0 {
        chain.push_str(&format!(",afade=t=out:st={}:d={}", secs(total.saturating_sub(track.fade_out_ms)), secs(track.fade_out_ms)));
    }
    chain.push_str(&format!(",volume={:.1}dB[aout]", track.gain_db));
    Some(chain)
}

/// How a video clip is framed at `w × h`, as `photos_media::frame::compose` and the preview
/// frame a photograph: `fill` covers the frame; `blur` shows the whole picture over a
/// blurred, dimmed copy of itself (the preview blurs by a 40th of the width and dims by 0.18).
fn video_frame_filter(clip: &Clip, [w, h]: [u32; 2]) -> String {
    let cover = format!("scale={w}:{h}:force_original_aspect_ratio=increase,crop={w}:{h}");
    match clip.frame {
        FrameKind::Blur => format!(
            "split[a][b];[a]{cover},gblur=sigma={sigma},colorchannelmixer=rr=0.82:gg=0.82:bb=0.82[bg];[b]scale={w}:{h}:force_original_aspect_ratio=decrease[fg];[bg][fg]overlay=(W-w)/2:(H-h)/2,setsar=1",
            sigma = (w as f32 / 80.0).max(1.0),
        ),
        _ => format!("{cover},setsar=1"),
    }
}

/// The FFmpeg arguments that render a video clip on its own (Round 6): `duration_ms` of the
/// video from the clip's `in_ms` — past its `out_ms` when the clip has been lengthened, which
/// shows more of the moment rather than a frozen frame — framed at the output size and rate,
/// silent, and nearly lossless, since it is encoded again when the show is put together.
/// `tone_map` rolls an HDR video down to SDR first, as its poster frame is.
pub fn video_clip_args(source: &Path, clip: &Clip, size: [u32; 2], fps: u32, tone_map: bool, out: &Path) -> Vec<String> {
    let start = clip.video.map_or(0, |v| v.in_ms);
    let mut graph = String::new();
    if tone_map {
        graph.push_str(crate::ffmpeg::Ffmpeg::tone_map_filter());
        graph.push(',');
    }
    graph.push_str(&video_frame_filter(clip, size));
    graph.push_str(&format!(",fps={fps},format=yuv420p"));
    let mut args: Vec<String> = ["-v", "error", "-y", "-nostdin", "-ss"].map(String::from).to_vec();
    args.push(secs(start));
    args.push("-t".into());
    args.push(secs(clip.duration_ms));
    args.push("-i".into());
    args.push(source.display().to_string());
    args.extend(["-map", "0:v:0", "-an", "-filter_complex"].map(String::from));
    args.push(format!("[0:v]{graph}[v]"));
    args.extend(["-map", "[v]", "-c:v", "libx264", "-preset", "ultrafast", "-crf", "14", "-pix_fmt", "yuv420p"].map(String::from));
    args.push(out.display().to_string());
    args
}

/// Renders a video clip for the export (see `video_clip_args`). Without zimg FFmpeg cannot
/// tone map; the clip is then rendered as it comes rather than lost.
pub fn render_video_clip(ffmpeg: &Ffmpeg, source: &Path, clip: &Clip, size: [u32; 2], fps: u32, tone_map: bool, out: &Path) -> Result<(), MediaError> {
    let result = cmd(&ffmpeg.ffmpeg).args(video_clip_args(source, clip, size, fps, tone_map, out)).stdout(Stdio::null()).output()?;
    if result.status.success() && out.is_file() {
        return Ok(());
    }
    if tone_map {
        return render_video_clip(ffmpeg, source, clip, size, fps, false, out);
    }
    Err(MediaError::Decode(format!("video clip: {}", String::from_utf8_lossy(&result.stderr).trim())))
}

/// The H.264 encoder an export uses (Round 4F, Task 4). x264 is always there; a hardware one
/// is used when FFmpeg has it and it passes a probe, because x264 `medium` was ~95 % of an
/// export's CPU time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Encoder {
    X264,
    Nvenc,
    Qsv,
    Amf,
    VideoToolbox,
}

impl Encoder {
    /// Hardware encoders, best first.
    pub const HARDWARE: [Encoder; 4] = [Encoder::Nvenc, Encoder::Qsv, Encoder::Amf, Encoder::VideoToolbox];

    pub fn ffmpeg_name(self) -> &'static str {
        match self {
            Encoder::X264 => "libx264",
            Encoder::Nvenc => "h264_nvenc",
            Encoder::Qsv => "h264_qsv",
            Encoder::Amf => "h264_amf",
            Encoder::VideoToolbox => "h264_videotoolbox",
        }
    }

    /// The quality each encoder is held to, matched to x264 CRF 20 by PSNR on real photos
    /// (`encoders_on_this_machine`).
    pub fn default_quality(self) -> u32 {
        match self {
            Encoder::X264 => 20,
            Encoder::Nvenc => NVENC_CQ,
            Encoder::Qsv => 21,
            Encoder::Amf => 21,
            Encoder::VideoToolbox => 65,
        }
    }

    /// The codec and its quality settings, at `quality` (CRF, CQ, QP or VideoToolbox's 1–100).
    pub fn args(self, quality: u32) -> Vec<String> {
        let q = quality.to_string();
        let a: Vec<&str> = match self {
            Encoder::X264 => vec!["-c:v", "libx264", "-preset", "medium", "-crf", &q, "-pix_fmt", "yuv420p"],
            Encoder::Nvenc => vec!["-c:v", "h264_nvenc", "-preset", "p5", "-tune", "hq", "-rc", "vbr", "-cq", &q, "-b:v", "0", "-pix_fmt", "yuv420p"],
            Encoder::Qsv => vec!["-c:v", "h264_qsv", "-preset", "slow", "-global_quality", &q, "-pix_fmt", "nv12"],
            Encoder::Amf => vec!["-c:v", "h264_amf", "-quality", "quality", "-rc", "cqp", "-qp_i", &q, "-qp_p", &q, "-pix_fmt", "yuv420p"],
            Encoder::VideoToolbox => vec!["-c:v", "h264_videotoolbox", "-q:v", &q, "-pix_fmt", "yuv420p"],
        };
        a.into_iter().map(String::from).collect()
    }

    /// A playback proxy's codec settings (Round 8A): fast over small, as it is made while
    /// someone waits to watch. H.264 High at 8 bits, which every WebView plays.
    pub fn proxy_args(self) -> Vec<String> {
        let a: &[&str] = match self {
            Encoder::X264 => &["-c:v", "libx264", "-preset", "veryfast", "-crf", "23", "-profile:v", "high", "-pix_fmt", "yuv420p"],
            Encoder::Nvenc => &["-c:v", "h264_nvenc", "-preset", "p2", "-rc", "vbr", "-cq", "25", "-b:v", "0", "-profile:v", "high", "-pix_fmt", "yuv420p"],
            Encoder::Qsv => &["-c:v", "h264_qsv", "-preset", "veryfast", "-global_quality", "25", "-profile:v", "high", "-pix_fmt", "nv12"],
            Encoder::Amf => &["-c:v", "h264_amf", "-quality", "speed", "-rc", "cqp", "-qp_i", "24", "-qp_p", "26", "-profile:v", "high", "-pix_fmt", "yuv420p"],
            Encoder::VideoToolbox => &["-c:v", "h264_videotoolbox", "-q:v", "55", "-profile:v", "high", "-pix_fmt", "yuv420p"],
        };
        a.iter().map(|s| s.to_string()).collect()
    }

    /// How many FFmpeg processes may use it at once: a consumer GPU allows only a few
    /// encoding sessions, and one GPU encoder is faster than the pieces' filters anyway.
    pub fn max_pieces(self) -> usize {
        match self {
            Encoder::X264 => usize::MAX,
            // Measured with NVENC on 60 photos: 2 pieces 34.2 s, 5 pieces 33.4 s, 8 pieces 35.6 s.
            _ => 3,
        }
    }
}

/// NVENC's constant quality, matched to x264 CRF 20 (45.9 dB on 12 real photos: CQ 23 gave
/// 46.5 dB at 22.6 MB, CQ 25 45.3 dB at 16.7 MB, x264 45.9 dB at 15.6 MB).
pub const NVENC_CQ: u32 = 24;

/// The encoder names in `ffmpeg -hide_banner -encoders` output.
pub fn parse_encoders(text: &str) -> Vec<String> {
    text.lines()
        .filter_map(|l| {
            let mut parts = l.split_whitespace();
            let flags = parts.next()?;
            // Flag columns look like `V....D`; the header's legend lines do not.
            (flags.len() == 6 && flags.starts_with('V') && !l.contains('=')).then(|| parts.next().map(String::from)).flatten()
        })
        .collect()
}

/// The encoder to use: the first hardware encoder FFmpeg lists that `works` (a real encode of
/// two small frames), else x264. `allow_hardware` is the user's setting.
pub fn pick_encoder(listed: &[String], allow_hardware: bool, works: impl Fn(Encoder) -> bool) -> Encoder {
    if !allow_hardware {
        return Encoder::X264;
    }
    Encoder::HARDWARE.into_iter().find(|e| listed.iter().any(|l| l == e.ffmpeg_name()) && works(*e)).unwrap_or(Encoder::X264)
}

/// Whether `enc` really encodes here: a listed encoder may still fail (NVENC with no NVIDIA
/// GPU, QSV with no driver).
pub fn probe_encoder(ffmpeg: &Ffmpeg, enc: Encoder) -> bool {
    let mut args: Vec<String> = ["-v", "error", "-nostdin", "-f", "lavfi", "-i", "testsrc2=size=320x240:rate=30:duration=0.2", "-frames:v", "2"].map(String::from).to_vec();
    args.extend(enc.args(enc.default_quality()));
    args.extend(["-f", "null", "-"].map(String::from));
    cmd(&ffmpeg.ffmpeg).args(args).output().map(|o| o.status.success()).unwrap_or(false)
}

/// `pick_encoder` for this FFmpeg, decided once per session.
pub fn best_encoder(ffmpeg: &Ffmpeg, allow_hardware: bool) -> Encoder {
    use std::sync::{Mutex, OnceLock};
    static CHOSEN: OnceLock<Mutex<std::collections::HashMap<(PathBuf, bool), Encoder>>> = OnceLock::new();
    let key = (ffmpeg.ffmpeg.clone(), allow_hardware);
    if let Some(e) = CHOSEN.get_or_init(Default::default).lock().ok().and_then(|m| m.get(&key).copied()) {
        return e;
    }
    let listed = cmd(&ffmpeg.ffmpeg)
        .args(["-hide_banner", "-encoders"])
        .output()
        .map(|o| parse_encoders(&String::from_utf8_lossy(&o.stdout)))
        .unwrap_or_default();
    let chosen = pick_encoder(&listed, allow_hardware, |e| probe_encoder(ffmpeg, e));
    tracing::info!(encoder = chosen.ffmpeg_name(), "slideshow encoder");
    if let Ok(mut m) = CHOSEN.get_or_init(Default::default).lock() {
        m.insert(key, chosen);
    }
    chosen
}

/// Lumen's mark over a generated film (Round 8A): input `input` (a PNG with alpha) scaled to
/// 4 % of the frame's short side, at 80 % opacity, 2.5 % of the short side in from the bottom
/// right corner, for the whole film. `[from]` in, `[to]` out.
pub fn watermark_filter(input: usize, from: &str, to: &str, [w, h]: [u32; 2]) -> String {
    let short = w.min(h) as f64;
    // Even, so a chroma-subsampled frame never splits the mark's edge.
    let height = ((short * 0.04 / 2.0).round() as u32 * 2).max(8);
    let margin = (short * 0.025).round() as u32;
    format!(
        "[{input}:v]format=rgba,scale=-2:{height}:flags=lanczos,colorchannelmixer=aa=0.8[wm];[{from}][wm]overlay=x=W-w-{margin}:y=H-h-{margin}:eof_action=repeat,format=yuv420p[{to}]"
    )
}

pub fn ffmpeg_args(doc: &SlideshowDoc, stills: &[PathBuf], audio: Option<&Path>, out: &Path) -> Vec<String> {
    ffmpeg_args_with(doc, stills, audio, out, Encoder::X264, Encoder::X264.default_quality(), None)
}

/// `ffmpeg_args` with another encoder and quality, and `mark` (a PNG with alpha) laid over the
/// whole film at the bottom right ([`watermark_filter`]).
pub fn ffmpeg_args_with(doc: &SlideshowDoc, stills: &[PathBuf], audio: Option<&Path>, out: &Path, encoder: Encoder, quality: u32, mark: Option<&Path>) -> Vec<String> {
    let [w, h] = doc.resolution;
    let fps = doc.fps;
    let t = timeline(doc);
    let mut args: Vec<String> = ["-v", "error", "-y", "-nostdin"].map(String::from).to_vec();
    // Each still is read and decoded once: `loop` repeats the scaled picture for every frame
    // of the clip. Looping the input instead decoded, scaled and cropped the 4K picture again
    // for every output frame (Round 4F, Task 2).
    for still in stills.iter().take(doc.clips.len()) {
        args.push("-i".into());
        args.push(still.display().to_string());
    }
    let audio_input = doc.clips.len();
    if let Some(a) = audio {
        args.push("-i".into());
        args.push(a.display().to_string());
    }
    let mark_input = audio_input + usize::from(audio.is_some());
    if let Some(m) = mark {
        args.push("-i".into());
        args.push(m.display().to_string());
    }

    let mut graph: Vec<String> = Vec::new();
    for (i, clip) in doc.clips.iter().enumerate() {
        graph.push(clip_filter(i, &format!("v{i}"), clip, frame_at(clip.duration_ms, fps), 0, [w, h], fps));
    }
    let mut acc = "v0".to_string();
    for i in 1..doc.clips.len() {
        let overlap = t.overlaps[i - 1];
        let next = format!("x{i}");
        let join = if overlap == 0 {
            // `concat` hands on a microsecond timebase; an `xfade` after it needs the clips'.
            format!("concat=n=2:v=1:a=0,settb=1/{fps}")
        } else {
            format!(
                "xfade=transition={}:duration={}:offset={}",
                xfade_name(doc.clips[i - 1].transition.kind),
                secs(overlap),
                secs(t.starts[i])
            )
        };
        graph.push(format!("[{acc}][v{i}]{join}[{next}]"));
        acc = next;
    }
    if mark.is_some() {
        graph.push(watermark_filter(mark_input, &acc, "marked", [w, h]));
        acc = "marked".into();
    }
    let mut maps = vec!["-map".to_string(), format!("[{acc}]")];
    if let (Some(chain), Some(_)) = (audio_chain(doc, audio_input), audio) {
        graph.push(chain);
        maps.extend(["-map".to_string(), "[aout]".to_string()]);
    }
    args.push("-filter_complex".into());
    args.push(graph.join(";"));
    args.extend(maps);
    args.extend(encoder.args(quality));
    args.push("-r".into());
    args.push(fps.to_string());
    if audio.is_some() && doc.audio.is_some() {
        args.extend(["-c:a", "aac", "-b:a", "192k"].map(String::from));
    }
    args.extend(["-movflags", "+faststart", "-t"].map(String::from));
    args.push(secs(t.total_ms));
    args.extend(["-progress", "pipe:1", "-nostats"].map(String::from));
    args.push(out.display().to_string());
    args
}

/// Renders `doc` to `out` (through `*.part.mp4`, renamed when complete). `stills[i]` is clip
/// `i`'s picture, already rendered with its edits. `on_progress` gets 0..=1; `cancel` is
/// polled on every progress line and stops FFmpeg.
pub fn render_slideshow(
    ffmpeg: &Ffmpeg,
    doc: &SlideshowDoc,
    stills: &[PathBuf],
    audio: Option<&Path>,
    out: &Path,
    on_progress: &mut dyn FnMut(f32),
    cancel: &dyn Fn() -> bool,
) -> Result<(), MediaError> {
    render_slideshow_with(ffmpeg, doc, stills, audio, out, Encoder::X264, Encoder::X264.default_quality(), None, on_progress, cancel)
}

/// `render_slideshow` with another encoder and quality, and the `mark` laid over it.
#[allow(clippy::too_many_arguments)]
pub fn render_slideshow_with(
    ffmpeg: &Ffmpeg,
    doc: &SlideshowDoc,
    stills: &[PathBuf],
    audio: Option<&Path>,
    out: &Path,
    encoder: Encoder,
    quality: u32,
    mark: Option<&Path>,
    on_progress: &mut dyn FnMut(f32),
    cancel: &dyn Fn() -> bool,
) -> Result<(), MediaError> {
    if doc.clips.is_empty() || stills.len() != doc.clips.len() {
        return Err(MediaError::Decode("a slideshow needs one picture per clip".into()));
    }
    if let Some(p) = out.parent() {
        std::fs::create_dir_all(p)?;
    }
    let tmp = out.with_extension("part.mp4");
    let total = timeline(doc).total_ms.max(1) as f64;
    let mut child = cmd(&ffmpeg.ffmpeg).args(ffmpeg_args_with(doc, stills, audio, &tmp, encoder, quality, mark)).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn()?;
    photos_core::child_jobs::adopt(&child);
    let stdout = child.stdout.take().expect("piped");
    let mut cancelled = false;
    for line in BufReader::new(stdout).lines() {
        let line = line?;
        if let Some(us) = line.strip_prefix("out_time_us=") {
            if let Ok(us) = us.trim().parse::<f64>() {
                on_progress((us / 1000.0 / total).clamp(0.0, 1.0) as f32);
            }
        }
        if cancel() {
            cancelled = true;
            let _ = child.kill();
            break;
        }
    }
    let result = child.wait_with_output()?;
    if cancelled {
        let _ = std::fs::remove_file(&tmp);
        return Err(MediaError::Decode("cancelled".into()));
    }
    if !result.status.success() {
        let _ = std::fs::remove_file(&tmp);
        return Err(MediaError::Decode(format!("slideshow export: {}", String::from_utf8_lossy(&result.stderr).trim())));
    }
    std::fs::rename(&tmp, out)?;
    on_progress(1.0);
    Ok(())
}

/// Frames `[from, to)` of the show as their own silent video (Round 4F): the clips on screen
/// in that range, the first one continuing its motion from where the previous piece left it,
/// with the same encoder settings as a whole run. `threads` bounds the encoder. The `mark`
/// sits still, so each piece lays it where the whole run would.
#[allow(clippy::too_many_arguments)]
pub fn segment_args(doc: &SlideshowDoc, stills: &[PathBuf], (from, to): (u32, u32), threads: u32, encoder: Encoder, quality: u32, mark: Option<&Path>, out: &Path) -> Vec<String> {
    let [w, h] = doc.resolution;
    let fps = doc.fps.max(1);
    let t = timeline(doc);
    let from_s = from as f64 / fps as f64;
    let to_ms = (to as u64 * 1000 / fps as u64) as u32;
    let from_ms = (from as u64 * 1000 / fps as u64) as u32;
    // The clips that show at least one frame in the range.
    let shown: Vec<usize> = (0..doc.clips.len())
        .filter(|&i| t.starts[i] < to_ms.max(1) && t.starts[i] + doc.clips[i].duration_ms > from_ms)
        .collect();
    let mut args: Vec<String> = ["-v", "error", "-y", "-nostdin"].map(String::from).to_vec();
    for &i in &shown {
        args.push("-i".into());
        args.push(stills[i].display().to_string());
    }
    if let Some(m) = mark {
        args.push("-i".into());
        args.push(m.display().to_string());
    }
    let mut graph: Vec<String> = Vec::new();
    for (k, &i) in shown.iter().enumerate() {
        let skip = if k == 0 { from.saturating_sub(frame_at(t.starts[i], fps)) } else { 0 };
        graph.push(clip_filter(k, &format!("v{k}"), &doc.clips[i], frame_at(doc.clips[i].duration_ms, fps), skip, [w, h], fps));
    }
    let mut acc = "v0".to_string();
    for (k, &i) in shown.iter().enumerate().skip(1) {
        let overlap = t.overlaps[i - 1];
        let next = format!("x{k}");
        let join = if overlap == 0 {
            // `concat` hands on a microsecond timebase; an `xfade` after it needs the clips'.
            format!("concat=n=2:v=1:a=0,settb=1/{fps}")
        } else {
            format!(
                "xfade=transition={}:duration={}:offset={:.6}",
                xfade_name(doc.clips[i - 1].transition.kind),
                secs(overlap),
                t.starts[i] as f64 / 1000.0 - from_s
            )
        };
        graph.push(format!("[{acc}][v{k}]{join}[{next}]"));
        acc = next;
    }
    if mark.is_some() {
        graph.push(watermark_filter(shown.len(), &acc, "marked", [w, h]));
        acc = "marked".into();
    }
    args.push("-filter_complex".into());
    args.push(graph.join(";"));
    args.extend(["-map".to_string(), format!("[{acc}]")]);
    args.extend(encoder.args(quality));
    args.push("-r".into());
    args.push(fps.to_string());
    args.extend(["-threads".to_string(), threads.max(1).to_string()]);
    // Every piece starts on a key frame, so the pieces join without re-encoding.
    args.extend(["-frames:v".to_string(), to.saturating_sub(from).to_string()]);
    args.extend(["-progress", "pipe:1", "-nostats"].map(String::from));
    args.push(out.display().to_string());
    args
}

/// Runs one FFmpeg process to the end, reporting `out_time` (seconds) into `done` and stopping
/// when `stop` is set. Returns its error text on failure.
fn run_piece(ffmpeg: &Ffmpeg, args: Vec<String>, done: &std::sync::atomic::AtomicU64, stop: &std::sync::atomic::AtomicBool) -> Result<(), String> {
    use std::sync::atomic::Ordering;
    let mut child = cmd(&ffmpeg.ffmpeg).args(args).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().map_err(|e| e.to_string())?;
    photos_core::child_jobs::adopt(&child);
    let stdout = child.stdout.take().expect("piped");
    for line in BufReader::new(stdout).lines() {
        let Ok(line) = line else { break };
        if let Some(us) = line.strip_prefix("out_time_us=").and_then(|v| v.trim().parse::<u64>().ok()) {
            done.store(us, Ordering::Relaxed);
        }
        if stop.load(Ordering::Relaxed) {
            let _ = child.kill();
            break;
        }
    }
    let result = child.wait_with_output().map_err(|e| e.to_string())?;
    if stop.load(Ordering::Relaxed) {
        return Err("cancelled".into());
    }
    if !result.status.success() {
        return Err(String::from_utf8_lossy(&result.stderr).trim().to_string());
    }
    Ok(())
}

/// How many pieces a parallel export uses, and how many encoder threads each gets: half the
/// cores less one in pieces (the UI and the stills' last stragglers keep the rest), at least
/// two threads each.
pub fn parallel_plan(cores: usize) -> (usize, u32) {
    let pieces = (cores / 2).saturating_sub(1).max(1);
    let threads = (cores / pieces).max(2) as u32;
    (pieces, threads)
}

/// `render_slideshow`, in pieces rendered side by side (Round 4F): the show is cut where one
/// photo is alone on screen (`segment_bounds`), each piece is encoded by its own FFmpeg, the
/// pieces are joined without re-encoding, and the audio is mixed once over the whole. `work`
/// holds the pieces and is emptied afterwards. One piece is exactly `render_slideshow`.
#[allow(clippy::too_many_arguments)]
pub fn render_slideshow_parallel(
    ffmpeg: &Ffmpeg,
    doc: &SlideshowDoc,
    stills: &[PathBuf],
    audio: Option<&Path>,
    out: &Path,
    work: &Path,
    pieces: usize,
    threads: u32,
    encoder: Encoder,
    mark: Option<&Path>,
    on_progress: &mut dyn FnMut(f32),
    cancel: &dyn Fn() -> bool,
) -> Result<(), MediaError> {
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
    let quality = encoder.default_quality();
    if doc.clips.is_empty() || stills.len() != doc.clips.len() {
        return Err(MediaError::Decode("a slideshow needs one picture per clip".into()));
    }
    let bounds = segment_bounds(doc, pieces.min(encoder.max_pieces()));
    if bounds.len() < 2 {
        return render_slideshow_with(ffmpeg, doc, stills, audio, out, encoder, quality, mark, on_progress, cancel);
    }
    std::fs::create_dir_all(work)?;
    if let Some(p) = out.parent() {
        std::fs::create_dir_all(p)?;
    }
    let total = timeline(doc).total_ms.max(1) as f64;
    let parts: Vec<PathBuf> = (0..bounds.len()).map(|k| work.join(format!("piece{k}.mp4"))).collect();
    let done: Vec<AtomicU64> = bounds.iter().map(|_| AtomicU64::new(0)).collect();
    let stop = AtomicBool::new(false);
    let cleanup = || {
        for p in &parts {
            let _ = std::fs::remove_file(p);
        }
    };
    let results: Vec<Result<(), String>> = std::thread::scope(|scope| {
        let handles: Vec<_> = bounds
            .iter()
            .zip(&parts)
            .zip(&done)
            .map(|((range, part), done)| {
                let args = segment_args(doc, stills, *range, threads, encoder, quality, mark, part);
                let stop = &stop;
                scope.spawn(move || run_piece(ffmpeg, args, done, stop))
            })
            .collect();
        // Rendering is 95 % of the work; the join and the audio are the rest.
        while handles.iter().any(|h| !h.is_finished()) {
            if cancel() {
                stop.store(true, Ordering::Relaxed);
            }
            let us: u64 = done.iter().map(|d| d.load(Ordering::Relaxed)).sum();
            on_progress((0.95 * us as f64 / 1000.0 / total).clamp(0.0, 0.95) as f32);
            std::thread::sleep(std::time::Duration::from_millis(150));
        }
        handles.into_iter().map(|h| h.join().unwrap_or_else(|_| Err("a piece panicked".into()))).collect()
    });
    if stop.load(Ordering::Relaxed) {
        cleanup();
        return Err(MediaError::Decode("cancelled".into()));
    }
    if let Some(e) = results.into_iter().find_map(Result::err) {
        cleanup();
        return Err(MediaError::Decode(format!("slideshow export: {e}")));
    }
    // Join the pieces as they are, then lay the music over the whole.
    let list = work.join("pieces.txt");
    let listing: String = parts.iter().map(|p| format!("file '{}'\n", p.display().to_string().replace('\\', "/").replace('\'', "'\\''"))).collect();
    std::fs::write(&list, listing)?;
    let tmp = out.with_extension("part.mp4");
    let joined = work.join("joined.mp4");
    let mut join: Vec<String> = ["-v", "error", "-y", "-nostdin", "-f", "concat", "-safe", "0", "-i"].map(String::from).to_vec();
    join.push(list.display().to_string());
    let with_audio = audio.is_some() && doc.audio.is_some();
    join.extend(["-c", "copy"].map(String::from));
    if !with_audio {
        join.extend(["-movflags", "+faststart"].map(String::from));
    }
    join.push(if with_audio { joined.display().to_string() } else { tmp.display().to_string() });
    let status = cmd(&ffmpeg.ffmpeg).args(&join).output()?;
    cleanup();
    let _ = std::fs::remove_file(&list);
    if !status.status.success() {
        let _ = std::fs::remove_file(&joined);
        return Err(MediaError::Decode(format!("slideshow join: {}", String::from_utf8_lossy(&status.stderr).trim())));
    }
    if let (true, Some(a), Some(chain)) = (with_audio, audio, audio_chain(doc, 1)) {
        let mut mux: Vec<String> = ["-v", "error", "-y", "-nostdin", "-i"].map(String::from).to_vec();
        mux.push(joined.display().to_string());
        mux.push("-i".into());
        mux.push(a.display().to_string());
        mux.extend(["-filter_complex".to_string(), chain]);
        mux.extend(["-map", "0:v", "-map", "[aout]", "-c:v", "copy", "-c:a", "aac", "-b:a", "192k", "-movflags", "+faststart", "-t"].map(String::from));
        mux.push(secs(timeline(doc).total_ms));
        mux.push(tmp.display().to_string());
        let status = cmd(&ffmpeg.ffmpeg).args(&mux).output()?;
        let _ = std::fs::remove_file(&joined);
        if !status.status.success() {
            let _ = std::fs::remove_file(&tmp);
            return Err(MediaError::Decode(format!("slideshow audio: {}", String::from_utf8_lossy(&status.stderr).trim())));
        }
    }
    std::fs::rename(&tmp, out)?;
    on_progress(1.0);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use photos_core::slideshow::AudioTrack;
    use photos_core::slideshow::{Clip, Motion, SlideshowDoc, Transition, TransitionKind};

    fn doc(kinds: &[TransitionKind], clip_ms: u32, transition_ms: u32, resolution: [u32; 2]) -> SlideshowDoc {
        let mut d = SlideshowDoc::new_for(&(1..=kinds.len() as i64).collect::<Vec<_>>());
        d.resolution = resolution;
        for (c, k) in d.clips.iter_mut().zip(kinds) {
            c.duration_ms = clip_ms;
            c.transition = Transition { kind: *k, duration_ms: transition_ms, auto: false };
        }
        d
    }

    fn stills(n: usize) -> Vec<PathBuf> {
        (0..n).map(|i| PathBuf::from(format!("clip{i}.jpg"))).collect()
    }

    #[test]
    fn clips_overlap_with_xfade_at_their_timeline_offsets() {
        let d = doc(&[TransitionKind::Crossfade, TransitionKind::Slide, TransitionKind::Zoom], 4000, 600, [1920, 1080]);
        let args = ffmpeg_args(&d, &stills(3), None, Path::new("out.mp4")).join(" ");
        assert!(args.contains("xfade=transition=fade:duration=0.600:offset=3.400"), "{args}");
        assert!(args.contains("xfade=transition=slideleft:duration=0.600:offset=6.800"), "{args}");
        assert_eq!(args.matches("perspective=").count(), 3);
        assert_eq!(args.matches("scale=1920:1080:flags=bicubic").count(), 3);
        assert!(args.contains("-t 10.800"), "the output is exactly the show's length: {args}");
        assert!(!args.contains("afade"), "no audio, no audio filters");
    }

    #[test]
    fn a_cut_is_a_concat_and_audio_is_trimmed_faded_and_gained() {
        let mut d = doc(&[TransitionKind::None, TransitionKind::None], 2000, 600, [1280, 720]);
        d.audio = Some(photos_core::slideshow::AudioTrack { path: "song.mp3".into(), fade_in_ms: 500, fade_out_ms: 1000, gain_db: -6.0, start_ms: 0 });
        let args = ffmpeg_args(&d, &stills(2), Some(Path::new("song.mp3")), Path::new("out.mp4")).join(" ");
        assert!(args.contains("concat=n=2:v=1:a=0"), "{args}");
        assert!(!args.contains("xfade"));
        assert!(args.contains("atrim=0.000:4.000"), "{args}");
        // A track shorter than the show is followed by silence, never looped or cut short.
        assert!(args.contains("asetpts=PTS-STARTPTS,apad=whole_dur=4.000"), "{args}");
        assert!(args.contains("afade=t=in:st=0:d=0.500"));
        assert!(args.contains("afade=t=out:st=3.000:d=1.000"));
        assert!(args.contains("volume=-6.0dB"));
    }

    #[test]
    fn every_transition_maps_to_an_xfade_name() {
        use photos_core::slideshow::TransitionKind as T;
        for (kind, name) in [
            (T::Crossfade, "fade"),
            (T::FadeBlack, "fadeblack"),
            (T::FadeWhite, "fadewhite"),
            (T::Slide, "slideleft"),
            (T::SlideUp, "slideup"),
            (T::Wipe, "wipeleft"),
            (T::WipeUp, "wipeup"),
            (T::Circle, "circleopen"),
            (T::Zoom, "zoomin"),
            (T::SlideDown, "slidedown"),
            (T::Cover, "coverleft"),
            (T::Reveal, "revealleft"),
            (T::CircleClose, "circleclose"),
            (T::VertOpen, "vertopen"),
            (T::HorzOpen, "horzopen"),
            (T::Corner, "wipetl"),
            (T::Squeeze, "squeezeh"),
            (T::Slices, "hlslice"),
            (T::VSlices, "vuslice"),
        ] {
            let d = doc(&[kind, TransitionKind::None], 4000, 600, [1280, 720]);
            let args = ffmpeg_args(&d, &stills(2), None, Path::new("out.mp4")).join(" ");
            assert!(args.contains(&format!("xfade=transition={name}:")), "{kind:?} should be {name}: {args}");
        }
    }

    /// A transition with no name of its own would silently render as a crossfade, and a
    /// variant missing from `ALL` would never reach the editor's list.
    #[test]
    fn every_variant_has_a_name_and_no_two_share_one() {
        use photos_core::slideshow::TransitionKind as T;
        let mut names: Vec<&str> = T::ALL.iter().copied().filter(|k| *k != T::None).map(xfade_name).collect();
        names.sort_unstable();
        let before = names.len();
        names.dedup();
        assert_eq!(names.len(), before, "two transitions share an xfade name");
        assert_eq!(T::ALL.len(), 20, "twenty transitions, and every one of them listed in ALL");
    }

    #[test]
    fn the_audio_starts_where_the_first_beat_is() {
        let mut d = doc(&[TransitionKind::Crossfade, TransitionKind::None], 4000, 600, [1280, 720]);
        d.audio = Some(AudioTrack { path: "m.mp3".into(), fade_in_ms: 0, fade_out_ms: 0, gain_db: 0.0, start_ms: 1500 });
        let args = ffmpeg_args(&d, &stills(2), Some(Path::new("m.mp3")), Path::new("out.mp4")).join(" ");
        assert!(args.contains("atrim=1.500:"), "{args}");
    }

    #[test]
    fn a_still_motion_is_a_plain_frame() {
        let mut d = doc(&[TransitionKind::None], 1000, 0, [640, 360]);
        d.clips[0] = Clip { motion: Motion::none(), ..d.clips[0].clone() };
        let args = ffmpeg_args(&d, &stills(1), None, Path::new("out.mp4")).join(" ");
        assert!(!args.contains("perspective"), "{args}");
        assert!(args.contains("crop=640:360,setsar=1,format=yuv420p,loop=loop=29:size=1:start=0"), "decoded and scaled once: {args}");
    }

    /// Round 6: `zoompan` put its view at whole pixels of the 2× still and rounded its size
    /// every frame, so pans stepped and zooms wobbled (frame-to-frame judder 0.58 px RMS,
    /// `scripts/slideshow/ken_burns_smoothness.py`). `perspective` places it at sub-pixel
    /// precision (0.03 px), from one decode of the still.
    #[test]
    fn ken_burns_moves_the_view_at_sub_pixel_precision_from_one_decode() {
        let mut d = doc(&[TransitionKind::None], 4000, 0, [1280, 720]);
        d.clips[0].motion = Motion { kind: MotionKind::KenBurns, from: [-0.05, 0.0, 1.0], to: [0.05, 0.02, 1.25] };
        let args = ffmpeg_args(&d, &stills(1), None, Path::new("out.mp4")).join(" ");
        assert!(!args.contains("zoompan"), "{args}");
        assert!(args.contains("scale=2560:1440:force_original_aspect_ratio=increase,crop=2560:1440,setsar=1,format=yuv420p,loop=loop=119:size=1:start=0"), "{args}");
        assert!(args.contains(":interpolation=linear:sense=source:eval=frame,scale=1280:720:flags=bicubic"), "{args}");
        // The view's left edge on the first frame: W·(0.5 − 0.05 − 0.5/1.0).
        assert!(args.contains("x0='W*((0.5+-0.050000+(0.100000)*"), "{args}");
    }

    #[test]
    fn a_piece_continues_the_motion_of_the_clip_it_starts_in() {
        let d = doc(&[TransitionKind::Crossfade, TransitionKind::None, TransitionKind::Slide], 4000, 600, [640, 360]);
        // Frames 60–200 (2.0–6.67 s): clip 0 from its 60th frame, then clip 1; clip 2 starts at
        // 7.4 s, after the piece.
        let args = segment_args(&d, &stills(3), (60, 200), 2, Encoder::X264, 20, None, Path::new("p.mp4")).join(" ");
        assert!(args.contains("(on+60)/119"), "{args}");
        assert!(args.contains("xfade=transition=fade:duration=0.600:offset=1.400000"), "{args}");
        assert!(args.contains("-frames:v 140"), "{args}");
        assert!(!args.contains("aout"), "pieces are silent");
        assert_eq!(args.matches("perspective=").count(), 2);
    }

    #[test]
    fn the_encoder_list_is_read_from_ffmpeg() {
        let text = "Encoders:\n V..... = Video\n A..... = Audio\n ------\n V....D libx264              libx264 H.264\n V....D h264_nvenc           NVIDIA NVENC H.264 encoder (codec h264)\n V..... h264_qsv             H.264 (Intel Quick Sync Video acceleration) (codec h264)\n A....D aac                  AAC (Advanced Audio Coding)\n";
        assert_eq!(parse_encoders(text), vec!["libx264", "h264_nvenc", "h264_qsv"]);
    }

    #[test]
    fn a_listed_encoder_that_fails_its_probe_is_skipped() {
        let listed: Vec<String> = ["libx264", "h264_nvenc", "h264_qsv"].map(String::from).to_vec();
        assert_eq!(pick_encoder(&listed, true, |e| e != Encoder::Nvenc), Encoder::Qsv);
        assert_eq!(pick_encoder(&listed, true, |_| false), Encoder::X264);
        assert_eq!(pick_encoder(&listed, true, |_| true), Encoder::Nvenc);
        assert_eq!(pick_encoder(&listed, false, |_| true), Encoder::X264, "the setting off");
        assert_eq!(pick_encoder(&["libx264".into()], true, |_| true), Encoder::X264, "nothing listed");
    }

    #[test]
    fn a_hardware_encoder_gets_few_pieces() {
        assert_eq!(Encoder::Nvenc.max_pieces(), 3);
        assert!(Encoder::Nvenc.args(21).join(" ").contains("-c:v h264_nvenc -preset p5 -tune hq -rc vbr -cq 21 -b:v 0"));
    }

    #[test]
    fn the_cores_are_shared_out() {
        assert_eq!(parallel_plan(24), (11, 2));
        assert_eq!(parallel_plan(8), (3, 2));
        assert_eq!(parallel_plan(2), (1, 2));
    }

    /// Round 4F, Task 3: the show in pieces is the show in one run: the same number of
    /// frames, the same length, and the same pictures either side of every seam.
    #[test]
    #[ignore = "runs FFmpeg several times"]
    fn pieces_make_the_same_film_as_one_run() {
        let Some(ff) = crate::Ffmpeg::locate() else {
            eprintln!("note: FFmpeg not found; skipping");
            return;
        };
        let dir = tempfile::tempdir().unwrap();
        let paths: Vec<PathBuf> = (0..6)
            .map(|i| {
                let p = dir.path().join(format!("{i}.jpg"));
                image::RgbImage::from_fn(1280, 720, |x, y| image::Rgb([((x + 40 * i) % 256) as u8, (y % 256) as u8, (60 * i) as u8])).save(&p).unwrap();
                p
            })
            .collect();
        let kinds = [TransitionKind::Crossfade, TransitionKind::None, TransitionKind::Slide, TransitionKind::Crossfade, TransitionKind::Wipe, TransitionKind::None];
        let mut d = doc(&kinds, 5000, 700, [640, 360]);
        for c in &mut d.clips {
            c.motion = photos_core::slideshow::Motion { kind: MotionKind::KenBurns, from: [-0.05, 0.0, 1.0], to: [0.05, 0.02, 1.25] };
        }
        let song = dir.path().join("song.wav");
        assert!(std::process::Command::new(&ff.ffmpeg).args(["-v", "error", "-y", "-f", "lavfi", "-i", "sine=frequency=440:duration=40"]).arg(&song).status().unwrap().success());
        d.audio = Some(AudioTrack { path: song.display().to_string(), fade_in_ms: 300, fade_out_ms: 600, gain_db: -3.0, start_ms: 0 });
        let one = dir.path().join("one.mp4");
        render_slideshow(&ff, &d, &paths, Some(&song), &one, &mut |_| {}, &|| false).unwrap();
        let many = dir.path().join("many.mp4");
        let work = dir.path().join("work");
        let mut last = 0.0;
        render_slideshow_parallel(&ff, &d, &paths, Some(&song), &many, &work, 3, 2, Encoder::X264, None, &mut |p| last = p, &|| false).unwrap();
        assert_eq!(last, 1.0);
        assert!(std::fs::read_dir(&work).unwrap().next().is_none(), "the pieces are gone");
        let bounds = segment_bounds(&d, 3);
        assert_eq!(bounds.len(), 3, "{bounds:?}");
        let frames = |p: &Path| -> u32 {
            let o = std::process::Command::new(&ff.ffprobe)
                .args(["-v", "error", "-count_frames", "-select_streams", "v:0", "-show_entries", "stream=nb_read_frames", "-of", "csv=p=0"])
                .arg(p)
                .output()
                .unwrap();
            String::from_utf8_lossy(&o.stdout).trim().parse().unwrap()
        };
        assert_eq!(frames(&many), frames(&one));
        let (a, b) = (ff.probe(&one).unwrap().duration_ms.unwrap() as i64, ff.probe(&many).unwrap().duration_ms.unwrap() as i64);
        assert!((a - b).abs() <= 40, "{a} vs {b} ms");
        // PSNR of the two films, frame by frame, around every seam.
        let o = std::process::Command::new(&ff.ffmpeg)
            .args(["-v", "info", "-i"])
            .arg(&one)
            .arg("-i")
            .arg(&many)
            .args(["-lavfi", "[0:v][1:v]psnr=stats_file=-", "-f", "null", "-"])
            .output()
            .unwrap();
        let stats = String::from_utf8_lossy(&o.stdout);
        let psnr: Vec<f64> = stats
            .lines()
            .filter_map(|l| l.split_whitespace().find_map(|w| w.strip_prefix("psnr_avg:")).map(|v| if v == "inf" { 99.0 } else { v.parse().unwrap() }))
            .collect();
        assert_eq!(psnr.len() as u32, frames(&one));
        for &(from, _) in &bounds[1..] {
            for f in from.saturating_sub(2)..(from + 3).min(psnr.len() as u32) {
                assert!(psnr[f as usize] >= 40.0, "frame {f} at a seam: {:.1} dB", psnr[f as usize]);
            }
        }
        let worst = psnr.iter().cloned().fold(f64::MAX, f64::min);
        println!("pieces {bounds:?}; worst frame {worst:.1} dB");
    }

    /// Cancelling a parallel export stops every piece and leaves nothing behind.
    #[test]
    #[ignore = "runs FFmpeg"]
    fn cancelling_pieces_leaves_nothing_behind() {
        let Some(ff) = crate::Ffmpeg::locate() else {
            eprintln!("note: FFmpeg not found; skipping");
            return;
        };
        let dir = tempfile::tempdir().unwrap();
        let paths: Vec<PathBuf> = (0..6)
            .map(|i| {
                let p = dir.path().join(format!("{i}.jpg"));
                image::RgbImage::from_fn(1280, 720, |x, y| image::Rgb([(x % 256) as u8, (y % 256) as u8, (40 * i) as u8])).save(&p).unwrap();
                p
            })
            .collect();
        let d = doc(&[TransitionKind::Crossfade; 6], 8000, 700, [1280, 720]);
        let (out, work) = (dir.path().join("show.mp4"), dir.path().join("work"));
        let started = std::time::Instant::now();
        let mut seen = 0.0;
        let err = render_slideshow_parallel(&ff, &d, &paths, None, &out, &work, 3, 2, Encoder::X264, None, &mut |p| seen = p, &|| started.elapsed().as_millis() > 800).unwrap_err();
        assert!(err.to_string().contains("cancelled"), "{err}");
        assert!(!out.exists() && !out.with_extension("part.mp4").exists());
        assert!(std::fs::read_dir(&work).unwrap().next().is_none(), "the pieces are gone");
        assert!(started.elapsed().as_secs() < 10, "every FFmpeg stopped promptly");
    }

    /// The preview promises the export lasts `timeline().total_ms`; FFmpeg has to agree.
    #[test]
    fn the_export_lasts_exactly_as_long_as_the_timeline_says() {
        let Some(ff) = crate::Ffmpeg::locate() else {
            eprintln!("note: FFmpeg not found; skipping");
            return;
        };
        let dir = tempfile::tempdir().unwrap();
        let paths: Vec<PathBuf> = (0..3)
            .map(|i| {
                let p = dir.path().join(format!("{i}.jpg"));
                image::RgbImage::from_fn(640, 360, |x, y| image::Rgb([(x % 256) as u8, (y % 256) as u8, 80 * i as u8])).save(&p).unwrap();
                p
            })
            .collect();
        let song = dir.path().join("song.wav");
        let st = std::process::Command::new(&ff.ffmpeg)
            .args(["-v", "error", "-y", "-f", "lavfi", "-i", "sine=frequency=440:duration=6"])
            .arg(&song)
            .status()
            .unwrap();
        assert!(st.success());
        let mut d = doc(&[TransitionKind::Crossfade, TransitionKind::None, TransitionKind::Slide], 1000, 300, [320, 180]);
        d.audio = Some(photos_core::slideshow::AudioTrack { path: song.display().to_string(), fade_in_ms: 200, fade_out_ms: 400, gain_db: -3.0, start_ms: 0 });
        let total = photos_core::slideshow::timeline(&d).total_ms;
        assert_eq!(total, 2700);
        let out = dir.path().join("show.mp4");
        let mut last = 0.0;
        render_slideshow(&ff, &d, &paths, Some(&song), &out, &mut |p| last = p, &|| false).unwrap();
        assert_eq!(last, 1.0);
        let probe = ff.probe(&out).unwrap();
        let got = probe.duration_ms.unwrap() as i64;
        assert!((got - total as i64).abs() <= 100, "exported {got} ms, timeline {total} ms");
        assert_eq!(probe.display_size(), (Some(320), Some(180)));
    }

    // Round 6: video clips.

    fn with_video(mut d: SlideshowDoc, i: usize, frame: FrameKind) -> SlideshowDoc {
        d.clips[i].video = Some(photos_core::slideshow::VideoSpan { in_ms: 7_000, out_ms: 11_000 });
        d.clips[i].frame = frame;
        d.clips[i].motion = Motion::none();
        d
    }

    #[test]
    fn a_video_clip_is_its_own_frames_rather_than_a_zoom() {
        let d = with_video(doc(&[TransitionKind::Crossfade; 3], 4000, 600, [1920, 1080]), 1, FrameKind::Fill);
        let args = ffmpeg_args(&d, &stills(3), None, Path::new("out.mp4")).join(" ");
        assert_eq!(args.matches("loop=loop=").count(), 2, "the stills, and not the video: {args}");
        assert!(args.contains("[1:v]fps=30,scale=1920:1080,setsar=1,tpad=stop_mode=clone:stop_duration=4.000,trim=start_frame=0:end_frame=120"), "{args}");
        assert!(args.contains("xfade=transition=fade:duration=0.600:offset=3.400"), "joins are unchanged: {args}");
    }

    #[test]
    fn a_piece_starting_inside_a_video_clip_skips_its_first_frames() {
        let d = with_video(doc(&[TransitionKind::None; 2], 4000, 0, [1280, 720]), 0, FrameKind::Fill);
        let args = segment_args(&d, &stills(2), (30, 240), 2, Encoder::X264, 23, None, Path::new("p.mp4")).join(" ");
        assert!(args.contains("trim=start_frame=30:end_frame=120"), "{args}");
    }

    #[test]
    fn a_video_clip_is_rendered_from_its_best_part_and_framed() {
        let d = with_video(doc(&[TransitionKind::Crossfade], 4000, 600, [1920, 1080]), 0, FrameKind::Fill);
        let fill = video_clip_args(Path::new("v.mov"), &d.clips[0], [1920, 1080], 30, false, Path::new("c.mp4")).join(" ");
        assert!(fill.contains("-ss 7.000 -t 4.000 -i v.mov"), "{fill}");
        assert!(fill.contains("-an"), "a video plays muted under the music");
        assert!(fill.contains("force_original_aspect_ratio=increase,crop=1920:1080"), "{fill}");
        assert!(!fill.contains("zscale"));
        let mut blur_clip = d.clips[0].clone();
        blur_clip.frame = FrameKind::Blur;
        let blur = video_clip_args(Path::new("v.mov"), &blur_clip, [1920, 1080], 30, true, Path::new("c.mp4")).join(" ");
        assert!(blur.contains("gblur") && blur.contains("overlay") && blur.contains("force_original_aspect_ratio=decrease"), "{blur}");
        assert!(blur.contains("zscale"), "an HDR video is tone mapped: {blur}");
    }

    #[test]
    #[ignore = "needs FFmpeg"]
    fn a_show_with_a_video_exports() {
        let ff = Ffmpeg::locate().expect("FFmpeg");
        let dir = tempfile::tempdir().unwrap();
        let video = dir.path().join("v.mp4");
        let still = dir.path().join("s.jpg");
        let gen = |args: &[&str], out: &Path| {
            let ok = cmd(&ff.ffmpeg).args(["-v", "error", "-y"]).args(args).arg(out).status().unwrap().success();
            assert!(ok, "generating {}", out.display());
        };
        gen(&["-f", "lavfi", "-i", "testsrc=duration=12:size=640x360:rate=30", "-pix_fmt", "yuv420p"], &video);
        gen(&["-f", "lavfi", "-i", "color=c=blue:size=1280x720", "-frames:v", "1"], &still);
        let mut d = with_video(doc(&[TransitionKind::Crossfade; 3], 3000, 500, [640, 360]), 1, FrameKind::Fill);
        d.clips[1].duration_ms = 4000;
        let clip_mp4 = dir.path().join("clip1.mp4");
        render_video_clip(&ff, &video, &d.clips[1], d.resolution, d.fps, false, &clip_mp4).unwrap();
        let inputs = vec![still.clone(), clip_mp4, still];
        let out = dir.path().join("show.mp4");
        render_slideshow(&ff, &d, &inputs, None, &out, &mut |_| {}, &|| false).unwrap();
        let probe = ff.probe(&out).unwrap();
        let expected = timeline(&d).total_ms as i64;
        assert!((probe.duration_ms.unwrap() - expected).abs() <= 100, "{:?} vs {expected}", probe.duration_ms);
        // And in pieces, side by side.
        let out2 = dir.path().join("show2.mp4");
        render_slideshow_parallel(&ff, &d, &inputs, None, &out2, &dir.path().join("work"), 3, 2, Encoder::X264, None, &mut |_| {}, &|| false).unwrap();
        assert!((ff.probe(&out2).unwrap().duration_ms.unwrap() - expected).abs() <= 100);
    }

    // Round 8A: Lumen's mark on the films it makes.

    #[test]
    fn the_mark_is_laid_over_the_whole_film_only_when_asked() {
        let d = doc(&[TransitionKind::Crossfade; 3], 4000, 600, [1920, 1080]);
        let plain = ffmpeg_args(&d, &stills(3), None, Path::new("out.mp4")).join(" ");
        assert!(!plain.contains("overlay"), "{plain}");
        let mark = Path::new("mark.png");
        let marked = ffmpeg_args_with(&d, &stills(3), None, Path::new("out.mp4"), Encoder::X264, 20, Some(mark)).join(" ");
        assert!(marked.contains("-i mark.png"), "{marked}");
        // 4 % of 1080 is 43 (44, even), the margin 2.5 % is 27.
        assert!(marked.contains("[3:v]format=rgba,scale=-2:44:flags=lanczos,colorchannelmixer=aa=0.8[wm];[x2][wm]overlay=x=W-w-27:y=H-h-27"), "{marked}");
        assert!(marked.contains("-map [marked]"), "the marked picture is the one written: {marked}");
        // With music, the mark is the input after the track.
        let mut d2 = d.clone();
        d2.audio = Some(AudioTrack { path: "song.mp3".into(), fade_in_ms: 0, fade_out_ms: 0, gain_db: 0.0, start_ms: 0 });
        let with_music = ffmpeg_args_with(&d2, &stills(3), Some(Path::new("song.mp3")), Path::new("out.mp4"), Encoder::X264, 20, Some(mark)).join(" ");
        assert!(with_music.contains("[4:v]format=rgba"), "{with_music}");
        // Every piece of a parallel export carries it.
        let piece = segment_args(&d, &stills(3), (60, 200), 2, Encoder::X264, 20, Some(mark), Path::new("p.mp4")).join(" ");
        assert!(piece.contains("overlay=x=W-w-27:y=H-h-27") && piece.contains("-map [marked]"), "{piece}");
    }

    #[test]
    fn the_mark_is_drawn_in_the_bottom_right_corner() {
        let Some(ff) = crate::Ffmpeg::locate() else {
            eprintln!("skipping: no FFmpeg");
            return;
        };
        let dir = tempfile::tempdir().unwrap();
        let still = dir.path().join("grey.png");
        image::RgbImage::from_pixel(640, 360, image::Rgb([128, 128, 128])).save(&still).unwrap();
        let mark = dir.path().join("mark.png");
        image::RgbaImage::from_pixel(120, 40, image::Rgba([255, 255, 255, 255])).save(&mark).unwrap();
        let d = doc(&[TransitionKind::None, TransitionKind::None], 1000, 0, [640, 360]);
        let stills = vec![still.clone(), still];
        let frame_of = |video: &Path| {
            let png = video.with_extension("png");
            let ok = cmd(&ff.ffmpeg).args(["-v", "error", "-y", "-ss", "0.5", "-i"]).arg(video).args(["-frames:v", "1"]).arg(&png).status().unwrap().success();
            assert!(ok);
            image::open(&png).unwrap().to_rgb8()
        };
        let plain = dir.path().join("plain.mp4");
        render_slideshow_with(&ff, &d, &stills, None, &plain, Encoder::X264, 20, None, &mut |_| {}, &|| false).unwrap();
        let marked = dir.path().join("marked.mp4");
        render_slideshow_parallel(&ff, &d, &stills, None, &marked, &dir.path().join("work"), 2, 2, Encoder::X264, Some(&mark), &mut |_| {}, &|| false).unwrap();
        let (a, b) = (frame_of(&plain), frame_of(&marked));
        // The mark is 14 px tall, 9 px in: (640 − 9 − 21, 360 − 9 − 7) is inside it.
        let corner = |f: &image::RgbImage| f.get_pixel(640 - 9 - 21, 360 - 9 - 7)[0];
        assert!(corner(&a) < 140, "no mark: grey ({})", corner(&a));
        assert!(corner(&b) > 200, "the mark at 80 % white over grey ({})", corner(&b));
        assert!(a.get_pixel(20, 20)[0].abs_diff(b.get_pixel(20, 20)[0]) < 4, "the rest of the frame is untouched");
    }
}
