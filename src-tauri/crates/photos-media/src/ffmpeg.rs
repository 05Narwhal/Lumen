//! FFmpeg / ffprobe as subprocesses (architecture spec §5.3, round-2 plan Part B).
//!
//! Missing FFmpeg is a *state*, never an error: `Ffmpeg::locate()` returns `None` and the
//! callers catalogue videos and HEIC without derivatives until it appears.

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde::{Deserialize, Serialize};

use crate::MediaError;

/// Environment variable naming a directory that holds `ffmpeg` and `ffprobe`.
pub const ENV_DIR: &str = "LUMEN_FFMPEG_DIR";

/// The Rust target triple this binary was built for (matches Tauri's sidecar naming).
pub const TARGET_TRIPLE: &str = {
    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    {
        "x86_64-pc-windows-msvc"
    }
    #[cfg(all(target_os = "windows", target_arch = "aarch64"))]
    {
        "aarch64-pc-windows-msvc"
    }
    #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
    {
        "x86_64-apple-darwin"
    }
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    {
        "aarch64-apple-darwin"
    }
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    {
        "x86_64-unknown-linux-gnu"
    }
    #[cfg(all(target_os = "linux", target_arch = "aarch64"))]
    {
        "aarch64-unknown-linux-gnu"
    }
    #[cfg(not(any(
        all(target_os = "windows", any(target_arch = "x86_64", target_arch = "aarch64")),
        all(target_os = "macos", any(target_arch = "x86_64", target_arch = "aarch64")),
        all(target_os = "linux", any(target_arch = "x86_64", target_arch = "aarch64"))
    )))]
    {
        "unknown"
    }
};

const EXE: &str = if cfg!(windows) { ".exe" } else { "" };

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ffmpeg {
    pub ffmpeg: PathBuf,
    pub ffprobe: PathBuf,
    /// First line of `ffmpeg -version`, e.g. `ffmpeg version 7.1-essentials_build-www.gyan.dev`.
    pub version: String,
}

/// What `ffprobe` tells us about a container. All fields optional: a corrupt file
/// still gets catalogued.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Probe {
    pub duration_ms: Option<i64>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub video_codec: Option<String>,
    pub profile: Option<String>,
    pub pix_fmt: Option<String>,
    pub audio_codec: Option<String>,
    /// ffprobe `format_name`, e.g. `mov,mp4,m4a,3gp,3g2,mj2`.
    pub container: Option<String>,
    /// Display rotation in degrees (0, 90, 180, 270) from the stream side data.
    pub rotation: i32,
    /// `creation_time` tag as unix seconds.
    pub creation_time: Option<i64>,
    /// Frames per second when known.
    pub fps: Option<f64>,
    pub bit_rate: Option<i64>,
    /// Local wall-clock capture time and its offset, from QuickTime's
    /// `com.apple.quicktime.creationdate` (phones) when present.
    pub creation_local: Option<i64>,
    pub tz_offset_minutes: Option<i32>,
    /// `com.apple.quicktime.make` / `.model` (iPhone, iPad) or plain `make` / `model` tags.
    pub make: Option<String>,
    pub model: Option<String>,
    /// Colour characteristics (ITU-T H.273). `smpte2084` and `arib-std-b67` mean HDR.
    pub color_transfer: Option<String>,
    pub color_primaries: Option<String>,
    /// A Dolby Vision configuration record rides with the video (Round 9A): an iPhone's HDR.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub dolby_vision: bool,
    /// Where it was filmed, from QuickTime's ISO 6709 location (`com.apple.quicktime.location.ISO6709`
    /// on phones, `location` in MP4s). EXIF never carries it in a video.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gps_lat: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gps_lon: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gps_altitude: Option<f64>,
}

impl Probe {
    /// Width/height as displayed (rotation applied), for aspect ratios and layout.
    pub fn display_size(&self) -> (Option<u32>, Option<u32>) {
        if self.rotation == 90 || self.rotation == 270 {
            (self.height, self.width)
        } else {
            (self.width, self.height)
        }
    }
}

/// Whether the WebView can play the file as-is or needs the H.264 proxy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Playback {
    Native,
    Proxy,
}

/// What the WebView says it can play (Round 8A), asked once per run with
/// `MediaSource.isTypeSupported`: HEVC needs Windows' HEVC Video Extensions, and 10-bit HEVC
/// (an iPhone's HDR video) a decoder that does Main 10.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VideoSupport {
    pub hevc: bool,
    pub hevc_main10: bool,
}

/// The capability table from the plan's Global Constraints:
/// H.264 8-bit + AAC/MP3/none in MP4/MOV → native everywhere; HEVC 8-bit in MP4/MOV →
/// native on macOS and Windows, proxy on Linux; everything else → proxy.
pub fn playback_for(p: &Probe, os: &str) -> Playback {
    playback_with(p, os, None)
}

/// [`playback_for`], told what this WebView plays (Round 8A): HEVC, 10-bit included, plays
/// as it is when the WebView says so, whatever the system.
pub fn playback_with(p: &Probe, os: &str, support: Option<VideoSupport>) -> Playback {
    let audio_ok = matches!(p.audio_codec.as_deref(), None | Some("aac") | Some("mp3"));
    if container_ok(p) && audio_ok && video_plays(p, os, support) {
        Playback::Native
    } else {
        Playback::Proxy
    }
}

/// An MP4 or QuickTime file: the containers the WebView opens.
fn container_ok(p: &Probe) -> bool {
    p.container
        .as_deref()
        .map(|c| c.split(',').any(|f| matches!(f.trim(), "mov" | "mp4" | "m4a" | "3gp" | "3g2" | "mj2")))
        .unwrap_or(false)
}

/// Whether the WebView decodes the video stream itself, whatever holds it.
fn video_plays(p: &Probe, os: &str, support: Option<VideoSupport>) -> bool {
    let eight_bit = matches!(p.pix_fmt.as_deref(), None | Some("yuv420p") | Some("yuvj420p") | Some("nv12"));
    let ten_bit = matches!(p.pix_fmt.as_deref(), Some("yuv420p10le") | Some("p010le"));
    match (p.video_codec.as_deref(), support) {
        (Some("h264"), _) => eight_bit,
        (Some("hevc"), Some(s)) => (eight_bit && s.hevc) || (ten_bit && s.hevc_main10),
        (Some("hevc"), None) => eight_bit && (os == "macos" || os == "windows"),
        _ => false,
    }
}

/// How a proxy is made (Round 8A).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProxyPlan {
    /// The video plays; only its container or its sound does not (a Live Photo's PCM audio,
    /// an MKV, an AVCHD stream): the video is copied and the sound made AAC, in about a second.
    Remux { copy_audio: bool },
    /// The video itself is re-encoded to H.264.
    Transcode,
}

/// Files this size or bigger get a 1080p proxy even when their picture could be copied
/// (Round 9A's smart size): a copy is as big as the original, and a library's few huge
/// videos would double their space.
pub const REMUX_MAX_BYTES: u64 = 1_000_000_000;

/// A transcoded proxy's video bit rate ceiling (Round 9A), bits per second: measured on real
/// videos, NVENC at its proxy quality otherwise ran to 20 Mbit/s on noisy footage.
pub const PROXY_MAX_BITRATE: i64 = 8_000_000;

/// The fastest proxy that plays, at a sensible size: `size` is the original's bytes, when known.
/// A big file is copied only when its bit rate is already below a transcode's.
pub fn proxy_plan(p: &Probe, size: Option<u64>, os: &str, support: Option<VideoSupport>) -> ProxyPlan {
    let small = size.is_none_or(|s| s < REMUX_MAX_BYTES) || p.bit_rate.is_some_and(|b| b <= PROXY_MAX_BITRATE);
    if small && video_plays(p, os, support) {
        ProxyPlan::Remux { copy_audio: matches!(p.audio_codec.as_deref(), None | Some("aac") | Some("mp3")) }
    } else {
        ProxyPlan::Transcode
    }
}

/// FFmpeg's arguments for a proxy of `path` at `out`: the first video stream and the first
/// sound, MP4 with its index first, so playback starts before the whole file has loaded.
/// `video` is the encoder's arguments when transcoding (`Encoder::proxy_args`).
pub fn proxy_args(path: &Path, out: &Path, plan: ProxyPlan, video: &[String], hevc: bool) -> Vec<std::ffi::OsString> {
    let mut a: Vec<std::ffi::OsString> = ["-v", "error", "-y", "-nostdin", "-i"].map(Into::into).to_vec();
    a.push(path.into());
    a.extend(["-map", "0:v:0", "-map", "0:a:0?", "-sn", "-dn"].map(Into::into));
    match plan {
        ProxyPlan::Remux { copy_audio } => {
            a.extend(["-c:v", "copy"].map(Into::into));
            // Browsers take HEVC in MP4 tagged `hvc1`.
            if hevc {
                a.extend(["-tag:v", "hvc1"].map(Into::into));
            }
            a.extend(if copy_audio { vec!["-c:a", "copy"] } else { vec!["-c:a", "aac", "-b:a", "160k"] }.into_iter().map(Into::into));
        }
        ProxyPlan::Transcode => {
            a.extend(video.iter().map(Into::into));
            let (max, buf) = (PROXY_MAX_BITRATE.to_string(), (2 * PROXY_MAX_BITRATE).to_string());
            a.extend(["-maxrate", max.as_str(), "-bufsize", buf.as_str()].map(Into::into));
            a.extend(["-vf", "scale=w='min(1920,iw)':h='min(1920,ih)':force_original_aspect_ratio=decrease:force_divisible_by=2", "-c:a", "aac", "-b:a", "160k"].map(Into::into));
        }
    }
    a.extend(["-movflags", "+faststart", "-progress", "pipe:1"].map(Into::into));
    a.push(out.into());
    a
}

impl Ffmpeg {
    /// Resolution order: `LUMEN_FFMPEG_DIR` → next to the executable (sidecar names, then
    /// plain names) → `src-tauri/binaries/` in debug builds → `PATH`. `None` when absent.
    pub fn locate() -> Option<Ffmpeg> {
        let mut dirs: Vec<PathBuf> = Vec::new();
        if let Some(d) = std::env::var_os(ENV_DIR) {
            dirs.push(PathBuf::from(d));
        }
        if let Ok(exe) = std::env::current_exe() {
            if let Some(d) = exe.parent() {
                dirs.push(d.to_path_buf());
            }
        }
        if cfg!(debug_assertions) {
            // crates/photos-media → src-tauri/binaries
            dirs.push(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../binaries"));
        }
        for dir in dirs {
            if let Some(f) = Self::in_dir(&dir) {
                return Some(f);
            }
        }
        Self::from_pair(PathBuf::from(format!("ffmpeg{EXE}")), PathBuf::from(format!("ffprobe{EXE}")))
    }

    fn in_dir(dir: &Path) -> Option<Ffmpeg> {
        for (f, p) in [
            (format!("ffmpeg-{TARGET_TRIPLE}{EXE}"), format!("ffprobe-{TARGET_TRIPLE}{EXE}")),
            (format!("ffmpeg{EXE}"), format!("ffprobe{EXE}")),
        ] {
            let (f, p) = (dir.join(f), dir.join(p));
            if f.is_file() && p.is_file() {
                if let Some(x) = Self::from_pair(f, p) {
                    return Some(x);
                }
            }
        }
        None
    }

    fn from_pair(ffmpeg: PathBuf, ffprobe: PathBuf) -> Option<Ffmpeg> {
        let out = cmd(&ffmpeg).arg("-version").output().ok()?;
        if !out.status.success() {
            return None;
        }
        let version = String::from_utf8_lossy(&out.stdout).lines().next().unwrap_or("").trim().to_string();
        if !version.starts_with("ffmpeg version") {
            return None;
        }
        if !cmd(&ffprobe).arg("-version").output().map(|o| o.status.success()).unwrap_or(false) {
            return None;
        }
        Some(Ffmpeg { ffmpeg, ffprobe, version })
    }

    pub fn probe(&self, path: &Path) -> Result<Probe, MediaError> {
        let out = cmd(&self.ffprobe)
            .args(["-v", "error", "-print_format", "json", "-show_format", "-show_streams"])
            .arg(path)
            .output()?;
        if !out.status.success() {
            return Err(MediaError::Decode(format!("ffprobe: {}", String::from_utf8_lossy(&out.stderr).trim())));
        }
        parse_probe(&out.stdout)
    }

    /// Roll a PQ or HLG source down to bt709 through linear light, so a thumbnail of an
    /// HDR clip is not the washed-out grey you get from reading the pixels as if they
    /// were SDR. Hable keeps highlight detail; `desat=0` keeps the colour.
    pub(crate) fn tone_map_filter() -> &'static str {
        "zscale=transfer=linear:npl=100,tonemap=tonemap=hable:desat=0,zscale=transfer=bt709:matrix=bt709:primaries=bt709:range=tv,format=yuv420p"
    }

    /// One JPEG frame at `at_secs` (q:v 3 ≈ 90 %). Seeks before the input for speed.
    pub fn poster_frame(&self, path: &Path, at_secs: f64, out_jpg: &Path) -> Result<(), MediaError> {
        self.poster_frame_with(path, at_secs, out_jpg, false)
    }

    /// [`Ffmpeg::poster_frame`], tone mapping first when the source is PQ or HLG.
    pub fn poster_frame_with(&self, path: &Path, at_secs: f64, out_jpg: &Path, tone_map: bool) -> Result<(), MediaError> {
        let mut c = cmd(&self.ffmpeg);
        c.args(["-v", "error", "-y", "-ss", &format!("{at_secs:.3}"), "-i"]).arg(path);
        if tone_map {
            c.args(["-vf", Self::tone_map_filter()]);
        }
        let status = c.args(["-frames:v", "1", "-q:v", "3", "-f", "image2"]).arg(out_jpg).output()?;
        if !status.status.success() || !out_jpg.is_file() {
            // Tone mapping needs libzimg; without it, take the frame as it comes rather
            // than failing the whole derivative.
            if tone_map {
                return self.poster_frame_with(path, at_secs, out_jpg, false);
            }
            // A seek past the end yields no frame: fall back to the first frame.
            if at_secs > 0.0 {
                return self.poster_frame(path, 0.0, out_jpg);
            }
            return Err(MediaError::Decode(format!("poster frame: {}", String::from_utf8_lossy(&status.stderr).trim())));
        }
        Ok(())
    }

    /// Full-size JPEG from a HEIC/HEIF/AVIF still (q:v 2 ≈ 93 %).
    pub fn decode_heic(&self, path: &Path, out_jpg: &Path) -> Result<(), MediaError> {
        self.decode_heic_with(path, out_jpg, false)
    }

    /// [`Ffmpeg::decode_heic`], tone mapping first when the source is PQ or HLG. A
    /// gain-map HEIC needs no tone mapping: its base image is already SDR.
    pub fn decode_heic_with(&self, path: &Path, out_jpg: &Path, tone_map: bool) -> Result<(), MediaError> {
        let mut c = cmd(&self.ffmpeg);
        c.args(["-v", "error", "-y", "-i"]).arg(path);
        if tone_map {
            c.args(["-vf", Self::tone_map_filter()]);
        }
        let out = c.args(["-frames:v", "1", "-q:v", "2", "-f", "image2"]).arg(out_jpg).output()?;
        if !out.status.success() || !out_jpg.is_file() {
            if tone_map {
                return self.decode_heic_with(path, out_jpg, false);
            }
            return Err(MediaError::Decode(format!("heic decode: {}", String::from_utf8_lossy(&out.stderr).trim())));
        }
        Ok(())
    }

    /// H.264 High / AAC, ≤ 1080p, faststart, with x264. `on_progress` gets 0..=1 from
    /// `-progress pipe:1`; `cancel` is polled per progress line and kills the process when it
    /// returns true.
    pub fn transcode_proxy(
        &self,
        path: &Path,
        out_mp4: &Path,
        on_progress: &mut dyn FnMut(f32),
        cancel: &dyn Fn() -> bool,
    ) -> Result<(), MediaError> {
        let video = crate::slideshow::Encoder::X264.proxy_args();
        self.make_proxy(path, out_mp4, ProxyPlan::Transcode, &video, false, false, on_progress, cancel)
    }

    /// A proxy by `plan` (Round 8A): a remux when only the container or sound is the problem,
    /// else a transcode with `video` (an encoder's `proxy_args`). `hevc` tags a copied HEVC
    /// stream so the WebView takes it. `background` (Round 9A: nobody is waiting) runs FFmpeg
    /// below normal priority.
    #[allow(clippy::too_many_arguments)]
    pub fn make_proxy(
        &self,
        path: &Path,
        out_mp4: &Path,
        plan: ProxyPlan,
        video: &[String],
        hevc: bool,
        background: bool,
        on_progress: &mut dyn FnMut(f32),
        cancel: &dyn Fn() -> bool,
    ) -> Result<(), MediaError> {
        let duration = self.probe(path).ok().and_then(|p| p.duration_ms).unwrap_or(0).max(1) as f64;
        let tmp = out_mp4.with_extension("part.mp4");
        if let Some(p) = out_mp4.parent() {
            std::fs::create_dir_all(p)?;
        }
        let (program, prefix) = launcher(&self.ffmpeg, background, cfg!(unix));
        let mut command = cmd(&program);
        #[cfg(windows)]
        if background {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            const BELOW_NORMAL_PRIORITY_CLASS: u32 = 0x0000_4000;
            command.creation_flags(CREATE_NO_WINDOW | BELOW_NORMAL_PRIORITY_CLASS);
        }
        let mut child = command
            .args(prefix)
            .args(proxy_args(path, &tmp, plan, video, hevc))
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        photos_core::child_jobs::adopt(&child);
        let stdout = child.stdout.take().expect("piped");
        let mut cancelled = false;
        for line in BufReader::new(stdout).lines() {
            let line = line?;
            if let Some(us) = line.strip_prefix("out_time_us=") {
                if let Ok(us) = us.trim().parse::<f64>() {
                    on_progress((us / 1000.0 / duration).clamp(0.0, 1.0) as f32);
                }
            }
            if cancel() {
                cancelled = true;
                let _ = child.kill();
                break;
            }
        }
        let out = child.wait_with_output()?;
        if cancelled {
            let _ = std::fs::remove_file(&tmp);
            return Err(MediaError::Decode("cancelled".into()));
        }
        if !out.status.success() {
            let _ = std::fs::remove_file(&tmp);
            return Err(MediaError::Decode(format!("proxy transcode: {}", String::from_utf8_lossy(&out.stderr).trim())));
        }
        std::fs::rename(&tmp, out_mp4)?;
        on_progress(1.0);
        Ok(())
    }
}

/// The program and its leading arguments for running `bin`: through `nice` when `background`
/// on a `unix` system (Round 9A). Windows lowers the process by its creation flags instead.
pub(crate) fn launcher(bin: &Path, background: bool, unix: bool) -> (PathBuf, Vec<std::ffi::OsString>) {
    if background && unix {
        (PathBuf::from("nice"), vec!["-n".into(), "10".into(), bin.into()])
    } else {
        (bin.to_path_buf(), Vec::new())
    }
}

/// A `Command` that never flashes a console window on Windows.
pub(crate) fn cmd(bin: &Path) -> Command {
    let mut c = Command::new(bin);
    c.stdin(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        c.creation_flags(CREATE_NO_WINDOW);
    }
    c
}

#[derive(Deserialize)]
struct RawProbe {
    #[serde(default)]
    streams: Vec<RawStream>,
    #[serde(default)]
    format: RawFormat,
}

#[derive(Deserialize, Default)]
struct RawFormat {
    format_name: Option<String>,
    duration: Option<String>,
    bit_rate: Option<String>,
    #[serde(default)]
    tags: serde_json::Map<String, serde_json::Value>,
}

#[derive(Deserialize)]
struct RawStream {
    codec_type: Option<String>,
    codec_name: Option<String>,
    profile: Option<String>,
    pix_fmt: Option<String>,
    color_transfer: Option<String>,
    color_primaries: Option<String>,
    width: Option<u32>,
    height: Option<u32>,
    duration: Option<String>,
    r_frame_rate: Option<String>,
    avg_frame_rate: Option<String>,
    #[serde(default)]
    side_data_list: Vec<serde_json::Map<String, serde_json::Value>>,
    #[serde(default)]
    tags: serde_json::Map<String, serde_json::Value>,
}

/// Pure: turn `ffprobe -print_format json` output into a [`Probe`].
pub fn parse_probe(json: &[u8]) -> Result<Probe, MediaError> {
    let raw: RawProbe = serde_json::from_slice(json).map_err(|e| MediaError::Decode(format!("ffprobe json: {e}")))?;
    let mut p = Probe::default();
    let secs = |s: &Option<String>| s.as_deref().and_then(|s| s.parse::<f64>().ok());
    p.duration_ms = secs(&raw.format.duration).map(|d| (d * 1000.0).round() as i64);
    p.container = raw.format.format_name.clone();
    p.bit_rate = raw.format.bit_rate.as_deref().and_then(|s| s.parse().ok());
    p.creation_time = raw.format.tags.get("creation_time").and_then(|v| v.as_str()).and_then(parse_creation_time);
    let tag = |k: &str| raw.format.tags.get(k).and_then(|v| v.as_str()).map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
    if let Some(local) = tag("com.apple.quicktime.creationdate").and_then(|s| parse_local_with_offset(&s)) {
        p.creation_local = Some(local.naive_local().and_utc().timestamp());
        p.tz_offset_minutes = Some(local.offset().local_minus_utc() / 60);
        if p.creation_time.is_none() {
            p.creation_time = Some(local.timestamp());
        }
    }
    p.make = tag("com.apple.quicktime.make").or_else(|| tag("make"));
    p.model = tag("com.apple.quicktime.model").or_else(|| tag("model"));
    if let Some((lat, lon, alt)) = ["com.apple.quicktime.location.ISO6709", "location", "location-eng"]
        .iter()
        .find_map(|k| tag(k).and_then(|v| parse_iso6709(&v)))
    {
        (p.gps_lat, p.gps_lon, p.gps_altitude) = (Some(lat), Some(lon), alt);
    }
    for s in &raw.streams {
        match s.codec_type.as_deref() {
            Some("video") if p.video_codec.is_none() => {
                p.video_codec = s.codec_name.clone();
                p.profile = s.profile.clone();
                p.pix_fmt = s.pix_fmt.clone();
                p.color_transfer = s.color_transfer.clone();
                p.color_primaries = s.color_primaries.clone();
                p.width = s.width;
                p.height = s.height;
                p.fps = s.avg_frame_rate.as_deref().or(s.r_frame_rate.as_deref()).and_then(parse_rate).filter(|f| *f > 0.0);
                if p.duration_ms.is_none() {
                    p.duration_ms = secs(&s.duration).map(|d| (d * 1000.0).round() as i64);
                }
                if p.creation_time.is_none() {
                    p.creation_time = s.tags.get("creation_time").and_then(|v| v.as_str()).and_then(parse_creation_time);
                }
                // Rotation: newer ffprobe puts it in side_data_list[].rotation; older in tags.rotate.
                let side = s
                    .side_data_list
                    .iter()
                    .find_map(|d| d.get("rotation").and_then(|r| r.as_i64().or_else(|| r.as_f64().map(|f| f as i64))));
                let tag = s.tags.get("rotate").and_then(|v| v.as_str()).and_then(|v| v.parse::<i64>().ok());
                p.rotation = side.or(tag).map(|r| r.rem_euclid(360) as i32).unwrap_or(0);
                p.dolby_vision = s.side_data_list.iter().any(|d| d.get("side_data_type").and_then(|t| t.as_str()).is_some_and(|t| t.starts_with("DOVI")));
            }
            Some("audio") if p.audio_codec.is_none() => p.audio_codec = s.codec_name.clone(),
            _ => {}
        }
    }
    Ok(p)
}

/// ISO 6709 decimal degrees: `+46.0010+014.0290+345.123/` or `-33.8688+151.2093/`.
/// `+00.0000+000.0000` is a phone that had no fix, not the Gulf of Guinea.
pub fn parse_iso6709(s: &str) -> Option<(f64, f64, Option<f64>)> {
    let s = s.trim().trim_end_matches('/');
    let mut parts = Vec::new();
    let mut start = 0;
    for (i, c) in s.char_indices().skip(1) {
        if c == '+' || c == '-' {
            parts.push(&s[start..i]);
            start = i;
        }
    }
    parts.push(&s[start..]);
    if parts.len() < 2 || !parts.iter().all(|p| p.starts_with('+') || p.starts_with('-')) {
        return None;
    }
    let lat = parts[0].parse::<f64>().ok()?;
    let lon = parts[1].parse::<f64>().ok()?;
    let alt = parts.get(2).and_then(|a| a.trim_end_matches("CRSWGS_84").parse::<f64>().ok());
    if !(-90.0..=90.0).contains(&lat) || !(-180.0..=180.0).contains(&lon) || (lat == 0.0 && lon == 0.0) {
        return None;
    }
    Some((lat, lon, alt))
}

fn parse_rate(s: &str) -> Option<f64> {
    let (n, d) = s.split_once('/')?;
    let (n, d) = (n.parse::<f64>().ok()?, d.parse::<f64>().ok()?);
    if d == 0.0 {
        None
    } else {
        Some(n / d)
    }
}

/// `2024-05-01T12:20:30+0200` (Apple, no colon in the offset) or RFC 3339.
fn parse_local_with_offset(s: &str) -> Option<chrono::DateTime<chrono::FixedOffset>> {
    chrono::DateTime::parse_from_rfc3339(s)
        .or_else(|_| chrono::DateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S%z"))
        .or_else(|_| chrono::DateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S%.f%z"))
        .ok()
}

/// `2024-05-01T10:20:30.000000Z` (QuickTime) or `2024-05-01 10:20:30` → unix seconds.
fn parse_creation_time(s: &str) -> Option<i64> {
    if let Ok(t) = chrono::DateTime::parse_from_rfc3339(s) {
        return Some(t.timestamp());
    }
    let s = s.trim_end_matches('Z');
    chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S%.f")
        .or_else(|_| chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S"))
        .ok()
        .map(|t| t.and_utc().timestamp())
}

/// Where the poster frame is taken: 10 % into the clip, at least 0 s, at most 5 s in.
pub fn poster_time(duration_ms: Option<i64>) -> f64 {
    match duration_ms {
        Some(d) if d > 0 => ((d as f64 / 1000.0) * 0.1).min(5.0),
        _ => 0.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_iso6709_locations() {
        assert_eq!(parse_iso6709("+46.0010+014.0290+345.123/"), Some((46.001, 14.029, Some(345.123))));
        assert_eq!(parse_iso6709("-33.8688+151.2093/"), Some((-33.8688, 151.2093, None)));
        assert_eq!(parse_iso6709("+00.0000+000.0000/"), None);
        assert_eq!(parse_iso6709("garbage"), None);
        let json = br#"{"format":{"tags":{"com.apple.quicktime.location.ISO6709":"+45.9990+014.0280+330.000/"}},"streams":[]}"#;
        let p = parse_probe(json).unwrap();
        assert_eq!((p.gps_lat, p.gps_lon), (Some(45.999), Some(14.028)));
    }

    fn probe(video: &str, pix: &str, audio: Option<&str>, container: &str) -> Probe {
        Probe {
            video_codec: Some(video.into()),
            pix_fmt: Some(pix.into()),
            audio_codec: audio.map(Into::into),
            container: Some(container.into()),
            ..Default::default()
        }
    }

    #[test]
    fn playback_table() {
        let h264 = probe("h264", "yuv420p", Some("aac"), "mov,mp4,m4a,3gp,3g2,mj2");
        for os in ["windows", "macos", "linux"] {
            assert_eq!(playback_for(&h264, os), Playback::Native, "{os}");
        }
        let hevc = probe("hevc", "yuv420p", Some("aac"), "mov,mp4,m4a,3gp,3g2,mj2");
        assert_eq!(playback_for(&hevc, "windows"), Playback::Native);
        assert_eq!(playback_for(&hevc, "macos"), Playback::Native);
        assert_eq!(playback_for(&hevc, "linux"), Playback::Proxy);
        let hevc10 = probe("hevc", "yuv420p10le", Some("aac"), "mov,mp4,m4a,3gp,3g2,mj2");
        assert_eq!(playback_for(&hevc10, "windows"), Playback::Proxy);
        let prores = probe("prores", "yuv422p10le", Some("pcm_s16le"), "mov,mp4,m4a,3gp,3g2,mj2");
        assert_eq!(playback_for(&prores, "macos"), Playback::Proxy);
        let mkv = probe("h264", "yuv420p", Some("aac"), "matroska,webm");
        assert_eq!(playback_for(&mkv, "windows"), Playback::Proxy);
        let silent = probe("h264", "yuv420p", None, "mov,mp4,m4a,3gp,3g2,mj2");
        assert_eq!(playback_for(&silent, "linux"), Playback::Native);
    }

    #[test]
    fn the_webview_says_what_plays() {
        let hevc = probe("hevc", "yuvj420p", Some("aac"), "mov,mp4,m4a,3gp,3g2,mj2");
        let hdr = probe("hevc", "yuv420p10le", Some("aac"), "mov,mp4,m4a,3gp,3g2,mj2");
        let none = VideoSupport::default();
        let eight = VideoSupport { hevc: true, hevc_main10: false };
        let all = VideoSupport { hevc: true, hevc_main10: true };
        assert_eq!(playback_with(&hevc, "windows", Some(none)), Playback::Proxy, "no HEVC Video Extensions");
        assert_eq!(playback_with(&hevc, "linux", Some(eight)), Playback::Native, "the WebView knows better than the OS");
        assert_eq!(playback_with(&hdr, "windows", Some(eight)), Playback::Proxy);
        assert_eq!(playback_with(&hdr, "windows", Some(all)), Playback::Native, "an iPhone's HDR video plays as it is");
        assert_eq!(playback_with(&hevc, "windows", None), playback_for(&hevc, "windows"), "unasked, as before");
    }

    #[test]
    fn a_big_file_gets_a_smaller_proxy_not_a_copy() {
        // Round 9A: a copy is as big as the original, so above ~1 GB the proxy is a 1080p
        // version instead, even when the picture could be copied.
        let all = Some(VideoSupport { hevc: true, hevc_main10: true });
        let live = probe("hevc", "yuvj420p", Some("pcm_s16le"), "mov,mp4,m4a,3gp,3g2,mj2");
        assert_eq!(proxy_plan(&live, Some(REMUX_MAX_BYTES - 1), "windows", all), ProxyPlan::Remux { copy_audio: false });
        assert_eq!(proxy_plan(&live, Some(REMUX_MAX_BYTES), "windows", all), ProxyPlan::Transcode);
        assert_eq!(proxy_plan(&live, None, "windows", all), ProxyPlan::Remux { copy_audio: false }, "size unknown: as before");
        // The 101 GB MOV in Julian's library: 10-bit HEVC with 24-bit PCM.
        let huge = probe("hevc", "yuv420p10le", Some("pcm_s24le"), "mov,mp4,m4a,3gp,3g2,mj2");
        assert_eq!(proxy_plan(&huge, Some(101_000_000_000), "windows", all), ProxyPlan::Transcode);
        // An MKV of 4 GB that plays once remuxed.
        let mkv = probe("h264", "yuv420p", Some("aac"), "matroska,webm");
        assert_eq!(proxy_plan(&mkv, Some(4_000_000_000), "windows", all), ProxyPlan::Transcode);
        // A long film already below the proxy's bit rate: a 1080p version would be no
        // smaller, only slower and worse. Copied.
        let film = Probe { bit_rate: Some(2_500_000), ..mkv.clone() };
        assert_eq!(proxy_plan(&film, Some(2_500_000_000), "windows", all), ProxyPlan::Remux { copy_audio: true });
        let busy = Probe { bit_rate: Some(PROXY_MAX_BITRATE + 1), ..mkv };
        assert_eq!(proxy_plan(&busy, Some(2_500_000_000), "windows", all), ProxyPlan::Transcode);
    }

    #[test]
    fn a_background_proxy_runs_ffmpeg_below_normal_priority() {
        // Round 9A: the video prep can convert for an hour; it must not take the cores the
        // app is using. On macOS and Linux through `nice`; Windows sets a priority class.
        let bin = Path::new("/opt/ffmpeg");
        let (prog, pre) = launcher(bin, true, true);
        assert_eq!(prog, Path::new("nice"));
        assert_eq!(pre, vec![std::ffi::OsString::from("-n"), "10".into(), bin.into()]);
        assert_eq!(launcher(bin, false, true), (bin.to_path_buf(), vec![]), "someone waiting: as fast as it goes");
        assert_eq!(launcher(bin, true, false), (bin.to_path_buf(), vec![]), "Windows: by creation flag instead");
    }

    #[test]
    fn a_transcode_fits_1080p_whichever_way_up() {
        // Round 9A: a portrait 4K video's long edge is its height; the box is 1920 × 1920 so
        // both orientations end at most 1080p, keeping their shape and even sizes.
        let join = |a: Vec<std::ffi::OsString>| a.iter().map(|s| s.to_string_lossy().into_owned()).collect::<Vec<_>>().join(" ");
        let video = crate::slideshow::Encoder::X264.proxy_args();
        let encode = join(proxy_args(Path::new("in.mov"), Path::new("out.mp4"), ProxyPlan::Transcode, &video, false));
        assert!(
            encode.contains("scale=w='min(1920,iw)':h='min(1920,ih)':force_original_aspect_ratio=decrease:force_divisible_by=2"),
            "{encode}"
        );
        // Measured on Julian's library: NVENC at CQ 25 ran to 20 Mbit/s on noisy footage.
        assert!(encode.contains("-maxrate 8000000 -bufsize 16000000"), "a ceiling, whatever the encoder: {encode}");
    }

    #[test]
    fn a_proxy_copies_what_plays_and_encodes_only_what_does_not() {
        let all = Some(VideoSupport { hevc: true, hevc_main10: true });
        // A Live Photo: HEVC with PCM sound.
        let live = probe("hevc", "yuvj420p", Some("pcm_s16le"), "mov,mp4,m4a,3gp,3g2,mj2");
        assert_eq!(playback_with(&live, "windows", all), Playback::Proxy);
        assert_eq!(proxy_plan(&live, None, "windows", all), ProxyPlan::Remux { copy_audio: false });
        assert_eq!(proxy_plan(&live, None, "windows", Some(VideoSupport::default())), ProxyPlan::Transcode, "no HEVC: encode");
        // An MKV that plays once it is in MP4.
        let mkv = probe("hevc", "yuv420p", Some("aac"), "matroska,webm");
        assert_eq!(proxy_plan(&mkv, None, "windows", all), ProxyPlan::Remux { copy_audio: true });
        // AVCHD: H.264 in a transport stream, Blu-ray PCM.
        let mts = probe("h264", "yuv420p", Some("pcm_bluray"), "mpegts");
        assert_eq!(proxy_plan(&mts, None, "windows", None), ProxyPlan::Remux { copy_audio: false });
        let prores = probe("prores", "yuv422p10le", Some("pcm_s24le"), "mov,mp4,m4a,3gp,3g2,mj2");
        assert_eq!(proxy_plan(&prores, None, "macos", all), ProxyPlan::Transcode);

        let join = |a: Vec<std::ffi::OsString>| a.iter().map(|s| s.to_string_lossy().into_owned()).collect::<Vec<_>>().join(" ");
        let remux = join(proxy_args(Path::new("in.mov"), Path::new("out.mp4"), ProxyPlan::Remux { copy_audio: false }, &[], true));
        assert!(remux.contains("-c:v copy -tag:v hvc1 -c:a aac"), "{remux}");
        assert!(!remux.contains("scale"), "a copied stream keeps its size: {remux}");
        let video = crate::slideshow::Encoder::Nvenc.proxy_args();
        let encode = join(proxy_args(Path::new("in.mov"), Path::new("out.mp4"), ProxyPlan::Transcode, &video, false));
        assert!(encode.contains("-c:v h264_nvenc") && encode.contains("scale=w='min(1920,iw)'"), "{encode}");
        assert!(encode.contains("-movflags +faststart"), "{encode}");
    }

    #[test]
    fn parses_ffprobe_json() {
        let json = br#"{
          "streams": [
            {"codec_type":"video","codec_name":"hevc","profile":"Main","pix_fmt":"yuv420p","width":1920,"height":1080,
             "color_transfer":"smpte2084","color_primaries":"bt2020",
             "avg_frame_rate":"30000/1001","side_data_list":[{"side_data_type":"Display Matrix","rotation":-90}],
             "tags":{"creation_time":"2024-05-01T10:20:30.000000Z"}},
            {"codec_type":"audio","codec_name":"aac"}
          ],
          "format": {"format_name":"mov,mp4,m4a,3gp,3g2,mj2","duration":"12.345","bit_rate":"8000000",
                     "tags":{"creation_time":"2024-05-01T10:20:30.000000Z","com.apple.quicktime.creationdate":"2024-05-01T12:20:30+0200",
                             "com.apple.quicktime.make":"Apple","com.apple.quicktime.model":"iPhone 15 Pro"}}
        }"#;
        let p = parse_probe(json).unwrap();
        assert_eq!(p.duration_ms, Some(12345));
        assert_eq!(p.video_codec.as_deref(), Some("hevc"));
        assert_eq!((p.width, p.height), (Some(1920), Some(1080)));
        assert_eq!(p.rotation, 270);
        assert_eq!(p.audio_codec.as_deref(), Some("aac"));
        assert_eq!(p.creation_time, Some(1714558830));
        assert!((p.fps.unwrap() - 29.97).abs() < 0.01);
        assert_eq!(p.bit_rate, Some(8_000_000));
        assert_eq!(p.creation_local, Some(1714558830 + 7200), "local wall time");
        assert_eq!(p.tz_offset_minutes, Some(120));
        assert_eq!(p.make.as_deref(), Some("Apple"));
        assert_eq!(p.model.as_deref(), Some("iPhone 15 Pro"));
        assert_eq!(p.display_size(), (Some(1080), Some(1920)), "rotated 270 -> portrait");
        assert_eq!(p.color_transfer.as_deref(), Some("smpte2084"));
        assert_eq!(p.color_primaries.as_deref(), Some("bt2020"));
        assert!(!p.dolby_vision);
    }

    #[test]
    fn an_iphones_dolby_vision_video_says_so() {
        // Round 9A: an iPhone's HDR video is HLG with a Dolby Vision record beside it.
        let json = br#"{"streams":[{"codec_type":"video","codec_name":"hevc","pix_fmt":"yuv420p10le","color_transfer":"arib-std-b67",
          "side_data_list":[{"side_data_type":"DOVI configuration record","dv_profile":8},{"side_data_type":"Display Matrix","rotation":0}]}],
          "format":{"format_name":"mov,mp4,m4a,3gp,3g2,mj2"}}"#;
        let p = parse_probe(json).unwrap();
        assert!(p.dolby_vision);
        assert_eq!(p.color_transfer.as_deref(), Some("arib-std-b67"));
    }

    #[test]
    fn poster_time_is_ten_percent_capped() {
        assert_eq!(poster_time(Some(2000)), 0.2);
        assert_eq!(poster_time(Some(600_000)), 5.0);
        assert_eq!(poster_time(None), 0.0);
    }
}
