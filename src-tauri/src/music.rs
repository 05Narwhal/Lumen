//! Free music for slideshows, from Jamendo's Creative Commons catalogue.
//!
//! This is the only part of Lumen that touches the network, and it is off until a free
//! client id is entered in Settings → Music. **Rust owns the request**: the webview never
//! calls Jamendo, so the key cannot leave the backend — even the thirty-second preview is
//! proxied through `photos://music-preview/{id}`.
//!
//! Only music that may be used *under* a video is offered: a licence with `nd` (no
//! derivatives) in it is filtered out unless the user asks to see everything.

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::error::{AppError, AppResult};
use photos_core::music_profile::TrackProfile;

const API: &str = "https://api.jamendo.com/v3.0/tracks/";
/// Where a free client id comes from; shown in Settings, never requested automatically.
pub const DEVPORTAL: &str = "https://devportal.jamendo.com";

/// What the picker asks for. Every field is optional: an empty query is "what is popular".
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct MusicQuery {
    #[serde(default)]
    pub text: Option<String>,
    /// A Jamendo mood tag: happy, calm, epic, energetic, sad, romantic, dreamy.
    #[serde(default)]
    pub vibe: Option<String>,
    #[serde(default)]
    pub genre: Option<String>,
    #[serde(default)]
    pub instrumental_only: bool,
    /// `low`, `medium` or `high`.
    #[serde(default)]
    pub tempo: Option<String>,
    /// Off by default: a no-derivatives licence cannot be used under a slideshow.
    #[serde(default)]
    pub include_no_derivatives: bool,
    #[serde(default)]
    pub limit: u32,
    /// `popularity_total` or `relevance`. Absent: popular when nothing is typed (the picker).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub order: Option<String>,
    /// Only tracks this long, in seconds (Jamendo's `durationbetween`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub duration_between: Option<(u32, u32)>,
    /// Skip this many results (Round 6): different shows see different pages, so the same
    /// handful of tracks does not come back for every one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub offset: Option<u32>,
}

/// One track as the picker shows it. `preview_url` and `download_url` are Jamendo's; only
/// Rust ever fetches them.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct MusicTrack {
    pub id: String,
    pub title: String,
    pub artist: String,
    #[ts(type = "number")]
    pub duration_ms: u32,
    /// `CC BY 3.0`, `CC BY-NC-ND 2.0` — the badge a person understands.
    pub licence_name: String,
    pub licence_url: String,
    pub share_url: String,
    pub preview_url: String,
    pub download_url: String,
    pub tags: Vec<String>,
    /// False for a `-nd` licence: the track may not be used under a video.
    pub derivatives_allowed: bool,
}

/// A track saved into `<library>/music/`, ready to use offline. `credit` is empty for a
/// file the user brought themselves, which needs no attribution.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SavedTrack {
    pub id: String,
    pub title: String,
    pub artist: String,
    pub licence_name: String,
    pub licence_url: String,
    pub share_url: String,
    /// The line an export must carry.
    pub credit: String,
    pub path: String,
    #[ts(type = "number")]
    pub duration_ms: u32,
    /// The moods and genres it was found by, kept so a show can be given music that suits
    /// it without asking the network. Absent from anything saved before Phase 6d.
    #[serde(default)]
    pub tags: Vec<String>,
}

fn encode(value: &str) -> String {
    // Enough for a search box: anything outside the unreserved set becomes %XX, and a
    // space becomes `+` as Jamendo's API expects.
    let mut out = String::with_capacity(value.len());
    for byte in value.as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(*byte as char),
            b' ' => out.push('+'),
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
}

/// The request for `q`. The client id is in it, so this string is never logged or returned
/// in an error.
pub fn search_url(client_id: &str, q: &MusicQuery) -> String {
    let limit = q.limit.clamp(1, 100);
    let mut url = format!(
        "{API}?client_id={}&format=json&limit={limit}&include=musicinfo+licenses&audioformat=mp32",
        encode(client_id)
    );
    let text = q.text.as_deref().map(str::trim).filter(|t| !t.is_empty());
    if let Some(t) = text {
        url.push_str(&format!("&search={}", encode(t)));
    }
    match q.order.as_deref().filter(|o| !o.is_empty()) {
        Some(order) => url.push_str(&format!("&order={}", encode(order))),
        // Nothing typed yet: show what people actually listen to.
        None if text.is_none() => url.push_str("&order=popularity_total"),
        None => {}
    }
    if let Some((from, to)) = q.duration_between {
        url.push_str(&format!("&durationbetween={from}_{to}"));
    }
    let tags: Vec<&str> = [q.vibe.as_deref(), q.genre.as_deref()].into_iter().flatten().filter(|t| !t.is_empty()).collect();
    if !tags.is_empty() {
        url.push_str(&format!("&fuzzytags={}", tags.iter().map(|t| encode(t)).collect::<Vec<_>>().join("+")));
    }
    if q.instrumental_only {
        url.push_str("&vocalinstrumental=instrumental");
    }
    if let Some(speed) = q.tempo.as_deref().filter(|s| !s.is_empty()) {
        url.push_str(&format!("&speed={}", encode(speed)));
    }
    if let Some(offset) = q.offset.filter(|o| *o > 0) {
        url.push_str(&format!("&offset={offset}"));
    }
    url
}

#[derive(Deserialize)]
struct Response {
    #[serde(default)]
    results: Vec<Raw>,
}

#[derive(Deserialize)]
struct Raw {
    id: String,
    name: String,
    #[serde(default)]
    duration: u32,
    #[serde(default)]
    artist_name: String,
    #[serde(default)]
    audio: String,
    #[serde(default)]
    audiodownload: String,
    #[serde(default)]
    shareurl: String,
    #[serde(default)]
    license_ccurl: String,
    #[serde(default)]
    musicinfo: Option<MusicInfo>,
}

#[derive(Deserialize)]
struct MusicInfo {
    #[serde(default)]
    tags: Option<Tags>,
}

#[derive(Deserialize, Default)]
struct Tags {
    #[serde(default)]
    genres: Vec<String>,
    #[serde(default)]
    vartags: Vec<String>,
}

/// Jamendo's licence URLs → the badge a person understands.
fn licence_name(url: &str) -> String {
    let after = url.split("/licenses/").nth(1).unwrap_or("");
    let mut parts = after.trim_end_matches('/').split('/');
    let code = parts.next().unwrap_or("");
    let version = parts.next().unwrap_or("");
    if code.is_empty() {
        return "Creative Commons".into();
    }
    let label = code.split('-').map(str::to_uppercase).collect::<Vec<_>>().join("-");
    if version.is_empty() {
        format!("CC {label}")
    } else {
        format!("CC {label} {version}")
    }
}

/// Reads a `/tracks/` response. The error never carries the request, which holds the key.
pub fn parse_tracks(body: &str) -> AppResult<Vec<MusicTrack>> {
    let parsed: Response = serde_json::from_str(body).map_err(|e| AppError {
        code: "music_unreadable".into(),
        message: "The music service sent something Lumen could not read.".into(),
        detail: Some(e.to_string()),
    })?;
    Ok(parsed
        .results
        .into_iter()
        .map(|r| {
            let tags = r.musicinfo.and_then(|m| m.tags).unwrap_or_default();
            MusicTrack {
                derivatives_allowed: !r.license_ccurl.contains("-nd"),
                licence_name: licence_name(&r.license_ccurl),
                licence_url: r.license_ccurl,
                duration_ms: r.duration.saturating_mul(1000),
                id: r.id,
                title: r.name,
                artist: r.artist_name,
                share_url: r.shareurl,
                preview_url: r.audio,
                download_url: r.audiodownload,
                tags: tags.vartags.into_iter().chain(tags.genres).collect(),
            }
        })
        .collect())
}

/// A track may be used under a slideshow only if its licence allows a derivative work.
pub fn usable(track: &MusicTrack) -> bool {
    track.derivatives_allowed
}

/// What the export writes beside the MP4 and the editor shows under the track.
pub fn credit_line(track: &MusicTrack) -> String {
    format!("\u{201c}{}\u{201d} by {} \u{2014} {} \u{2014} {}", track.title, track.artist, track.licence_name, track.share_url)
}

/// `<library>/music`, which holds saved tracks and their sidecars.
pub fn music_dir(library_root: &Path) -> PathBuf {
    library_root.join("music")
}

/// A track's file name: the id, with anything that is not plainly a name removed, so a
/// value from the network can never point outside `<library>/music`.
fn safe_id(id: &str) -> String {
    let cleaned: String = id.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_').collect();
    if cleaned.is_empty() {
        "track".into()
    } else {
        cleaned
    }
}

/// Writes the audio and the sidecar that credits it. Pure file work: the bytes are already
/// in hand, so this is testable without the network.
pub fn write_saved(library_root: &Path, track: &MusicTrack, bytes: &[u8]) -> AppResult<SavedTrack> {
    let dir = music_dir(library_root);
    std::fs::create_dir_all(&dir)?;
    let id = safe_id(&track.id);
    let audio = dir.join(format!("{id}.mp3"));
    std::fs::write(&audio, bytes)?;
    let saved = SavedTrack {
        id: id.clone(),
        title: track.title.clone(),
        artist: track.artist.clone(),
        licence_name: track.licence_name.clone(),
        licence_url: track.licence_url.clone(),
        share_url: track.share_url.clone(),
        credit: credit_line(track),
        path: audio.display().to_string(),
        duration_ms: track.duration_ms,
        tags: track.tags.clone(),
    };
    let sidecar = dir.join(format!("{id}.json"));
    std::fs::write(&sidecar, serde_json::to_string_pretty(&saved).unwrap_or_default())?;
    Ok(saved)
}

/// The sidecar beside an audio file, when there is one.
pub fn sidecar_of(audio: &Path) -> Option<SavedTrack> {
    let text = std::fs::read_to_string(audio.with_extension("json")).ok()?;
    serde_json::from_str(&text).ok()
}

/// Every track in `<library>/music`: saved from Jamendo with its credit, or dropped in by
/// the user, in which case it stands for itself and needs no attribution.
pub fn list_saved(library_root: &Path) -> Vec<SavedTrack> {
    let Ok(entries) = std::fs::read_dir(music_dir(library_root)) else { return Vec::new() };
    let mut out: Vec<SavedTrack> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.extension().and_then(|e| e.to_str()).map(|e| AUDIO_EXTENSIONS.contains(&e.to_ascii_lowercase().as_str())) == Some(true)
        })
        .map(|path| {
            sidecar_of(&path).unwrap_or_else(|| SavedTrack {
                id: path.file_stem().unwrap_or_default().to_string_lossy().to_string(),
                title: path.file_stem().unwrap_or_default().to_string_lossy().to_string(),
                artist: String::new(),
                licence_name: String::new(),
                licence_url: String::new(),
                share_url: String::new(),
                credit: String::new(),
                path: path.display().to_string(),
                duration_ms: 0,
                tags: Vec::new(),
            })
        })
        .collect();
    out.sort_by_key(|t| t.title.to_lowercase());
    out
}

/// What the picker and the editor accept, mirroring `src/lib/media-extensions.ts`.
pub const AUDIO_EXTENSIONS: &[&str] = &["mp3", "m4a", "aac", "wav", "flac", "ogg"];

fn offline(e: impl std::fmt::Display) -> AppError {
    // Deliberately terse: the request carries the client id and must never be quoted.
    tracing::warn!("music request failed: {e}");
    AppError {
        code: "music_unavailable".into(),
        message: "Could not reach the music service. Check your connection and your client id in Settings.".into(),
        detail: None,
    }
}

/// One search against Jamendo. `include_no_derivatives` decides whether tracks that may
/// not be used under a video are shown at all.
pub async fn search(client: &reqwest::Client, client_id: &str, q: &MusicQuery) -> AppResult<Vec<MusicTrack>> {
    let body = client.get(search_url(client_id, q)).send().await.map_err(offline)?.text().await.map_err(offline)?;
    let mut tracks = parse_tracks(&body)?;
    if !q.include_no_derivatives {
        tracks.retain(usable);
    }
    Ok(tracks)
}

/// Fetches a track's audio. Used both to save one and to preview it.
pub async fn fetch(client: &reqwest::Client, url: &str) -> AppResult<Vec<u8>> {
    let response = client.get(url).send().await.map_err(offline)?;
    if !response.status().is_success() {
        return Err(offline(format!("status {}", response.status())));
    }
    Ok(response.bytes().await.map_err(offline)?.to_vec())
}

/// Saves `track` into the library, audio and sidecar together.
pub async fn download(client: &reqwest::Client, track: &MusicTrack, library_root: &Path) -> AppResult<SavedTrack> {
    if !usable(track) {
        return Err(AppError {
            code: "music_licence".into(),
            message: "This track's licence does not allow it under a video.".into(),
            detail: None,
        });
    }
    let url = if track.download_url.is_empty() { &track.preview_url } else { &track.download_url };
    let bytes = fetch(client, url).await?;
    write_saved(library_root, track, &bytes)
}

/// How many previews are kept in memory. A preview is a whole track, so a handful is
/// plenty: enough to flick back and forth between results without fetching twice.
const PREVIEW_KEEP: usize = 4;

/// Where the previews live. Search results register their URL here, and the protocol route
/// fetches through Rust, so the webview still never talks to the network.
#[derive(Default)]
pub struct PreviewCache {
    urls: Mutex<HashMap<String, String>>,
    bytes: Mutex<VecDeque<(String, Arc<Vec<u8>>)>>,
}

impl PreviewCache {
    /// Remembers where each of these tracks can be heard.
    pub fn remember(&self, tracks: &[MusicTrack]) {
        let mut urls = self.urls.lock().expect("preview urls");
        for t in tracks {
            if !t.preview_url.is_empty() {
                urls.insert(t.id.clone(), t.preview_url.clone());
            }
        }
    }

    pub fn url_for(&self, id: &str) -> Option<String> {
        self.urls.lock().expect("preview urls").get(id).cloned()
    }

    pub fn cached(&self, id: &str) -> Option<Arc<Vec<u8>>> {
        self.bytes.lock().expect("preview bytes").iter().find(|(t, _)| t == id).map(|(_, b)| b.clone())
    }

    pub fn store(&self, id: &str, bytes: Arc<Vec<u8>>) {
        let mut cache = self.bytes.lock().expect("preview bytes");
        cache.push_back((id.to_string(), bytes));
        while cache.len() > PREVIEW_KEEP {
            cache.pop_front();
        }
    }
}


/// Below this a track does not suit the show well enough to use at all.
pub const PICK_FLOOR: f64 = 0.35;

/// What a show wants of its music.
#[derive(Debug, Clone, PartialEq)]
pub struct MusicWant {
    /// Jamendo mood tags with how much each is wanted, heaviest first.
    pub moods: Vec<(String, f64)>,
    /// How busy the pictures are, 0..1, when the photographs have embeddings.
    pub energy: Option<f64>,
    pub target_ms: u32,
    /// Makes the choice reproducible for one show, and different between shows.
    pub seed: i64,
}

/// An ordered list of moods as weights, `1 − i/2n`: the first counts fully, the last a
/// little over half — which is exactly how the ordered list used to score.
pub fn weighted(moods: &[&str]) -> Vec<(String, f64)> {
    let n = moods.len().max(1) as f64;
    moods.iter().enumerate().map(|(i, m)| (m.to_string(), 1.0 - i as f64 / (n * 2.0))).collect()
}

/// Tags that make a photo slideshow feel like a trailer (Round 5). Never asked for, and a
/// track carrying one loses `DRAMATIC_PENALTY`.
pub const DRAMATIC: [&str; 6] = ["epic", "energetic", "dramatic", "action", "aggressive", "powerful"];
pub const DRAMATIC_PENALTY: f64 = 0.25;
/// The busiest pace a show asks for: lively pictures get a medium tempo, never a fast one.
pub const MAX_ENERGY: f64 = 0.55;

/// Moods made gentler for a photo slideshow (Round 5): anything dramatic becomes *happy* —
/// still lively, not a trailer — merging weights, heaviest first.
pub fn soften(moods: Vec<(String, f64)>) -> Vec<(String, f64)> {
    let mut out: Vec<(String, f64)> = Vec::with_capacity(moods.len());
    for (m, w) in moods {
        let m = if DRAMATIC.contains(&m.as_str()) { "happy".to_string() } else { m };
        match out.iter_mut().find(|(o, _)| *o == m) {
            Some(slot) => slot.1 += w,
            None => out.push((m, w)),
        }
    }
    out.sort_by(|a, b| b.1.total_cmp(&a.1));
    out
}

/// The moods that suit a memory of this kind — what a show gets when its photographs have
/// no embeddings yet. Calm by default (Round 5): trips and events may be cheerful, never epic.
pub fn mood_for_memory(kind: i64) -> Vec<(String, f64)> {
    weighted(match kind {
        photos_core::memories::KIND_TRIP => &["happy", "dreamy", "calm"],
        photos_core::memories::KIND_EVENT => &["happy", "calm"],
        photos_core::memories::KIND_ON_THIS_DAY => &["dreamy", "romantic", "calm"],
        _ => &["calm", "dreamy"],
    })
}

/// The moods the photographs themselves suggest (ML design §7): their mean embedding against
/// the `mood:*` prototypes, as a softmax — and how energetic they are, the weight on
/// *energetic*, *epic* and *playful*. `None` with no embeddings or no prototypes.
pub fn mood_from_photos(vecs: &[&[i8]], protos: &photos_core::embedding::Prototypes) -> Option<(Vec<(String, f64)>, f64)> {
    let mean = photos_core::embedding::mean(vecs)?;
    let weights = photos_core::embedding::softmax(&mean, protos, "mood:", 0.05);
    if weights.is_empty() {
        return None;
    }
    let energy = weights.iter().filter(|(m, _)| matches!(m.as_str(), "energetic" | "epic" | "playful")).map(|(_, w)| *w as f64).sum();
    Some((soften(weights.into_iter().map(|(m, w)| (m, w as f64)).collect()), energy))
}

/// How well a track's tags carry the wanted moods, 0..1. The best match counts most, and a
/// track carrying several of the wanted moods a little more (Round 6): matching *calm* alone
/// tied every result of a *calm* search.
fn mood_fit(tags: &[String], want: &MusicWant) -> f64 {
    let heaviest = want.moods.iter().map(|m| m.1).fold(0.0, f64::max);
    if want.moods.is_empty() || heaviest <= 0.0 {
        return 0.5;
    }
    let total: f64 = want.moods.iter().map(|m| m.1.max(0.0)).sum();
    let matched: Vec<f64> = want.moods.iter().filter(|(m, _)| tags.iter().any(|t| t.eq_ignore_ascii_case(m))).map(|(_, w)| *w).collect();
    let best = matched.iter().map(|w| w / heaviest).fold(0.0, f64::max);
    let share = if total > 0.0 { matched.iter().sum::<f64>() / total } else { 0.0 };
    0.8 * best + 0.2 * share
}

/// How well a track's length suits a show of `target_ms`, 0..1.
fn length_fit(duration_ms: u32, target_ms: u32) -> f64 {
    let (have, need) = (duration_ms as f64, target_ms.max(1) as f64);
    if have >= need {
        // Long enough. Twice the show is still fine; ten times is a track we are using
        // a tenth of, and something closer would suit better.
        (1.0 - (have - need) / (need * 4.0)).clamp(0.2, 1.0)
    } else {
        // Short: it will have to fade early, which is worse the shorter it is.
        (have / need).clamp(0.0, 0.9)
    }
}

fn dramatic_penalty(tags: &[String]) -> f64 {
    if tags.iter().any(|t| DRAMATIC.iter().any(|d| t.eq_ignore_ascii_case(d))) {
        DRAMATIC_PENALTY
    } else {
        0.0
    }
}

/// How well a track suits, in 0..1, from its tags and length. Mood counts double: the wrong
/// feeling under a film is worse than a track that has to fade out early. Energy — how busy
/// the track is against how busy the pictures are — counts once, and only when both are known.
fn suits(tags: &[String], duration_ms: u32, track_energy: Option<f64>, want: &MusicWant) -> f64 {
    suits_unpenalised(tags, duration_ms, track_energy, want) - dramatic_penalty(tags)
}

fn suits_unpenalised(tags: &[String], duration_ms: u32, track_energy: Option<f64>, want: &MusicWant) -> f64 {
    let mood = mood_fit(tags, want);
    let length = length_fit(duration_ms, want.target_ms);
    match (want.energy.map(|e| e.min(MAX_ENERGY)), track_energy) {
        (Some(w), Some(t)) => (2.0 * mood + (1.0 - (w - t).abs()) + length) / 4.0,
        _ => (2.0 * mood + length) / 3.0,
    }
}

/// What a mood sounds like (Round 6), on `TrackProfile`'s 0..1 scales: tempo (60–180 BPM),
/// energy (onsets per second over 6), brightness and dynamics. Hand-set from what the words
/// mean; to be tuned by listening.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SoundTarget {
    pub tempo: f64,
    pub energy: f64,
    pub brightness: f64,
    pub dynamics: f64,
}

pub const MOOD_SOUND: [(&str, SoundTarget); 9] = [
    ("calm", SoundTarget { tempo: 0.2, energy: 0.15, brightness: 0.35, dynamics: 0.3 }),
    ("dreamy", SoundTarget { tempo: 0.25, energy: 0.2, brightness: 0.45, dynamics: 0.4 }),
    ("romantic", SoundTarget { tempo: 0.3, energy: 0.25, brightness: 0.45, dynamics: 0.4 }),
    ("sad", SoundTarget { tempo: 0.2, energy: 0.15, brightness: 0.3, dynamics: 0.35 }),
    ("happy", SoundTarget { tempo: 0.55, energy: 0.5, brightness: 0.65, dynamics: 0.4 }),
    ("playful", SoundTarget { tempo: 0.6, energy: 0.55, brightness: 0.7, dynamics: 0.45 }),
    ("energetic", SoundTarget { tempo: 0.75, energy: 0.75, brightness: 0.7, dynamics: 0.4 }),
    ("epic", SoundTarget { tempo: 0.55, energy: 0.6, brightness: 0.55, dynamics: 0.8 }),
    ("relaxed", SoundTarget { tempo: 0.25, energy: 0.2, brightness: 0.4, dynamics: 0.3 }),
];
const NEUTRAL_SOUND: SoundTarget = SoundTarget { tempo: 0.45, energy: 0.4, brightness: 0.5, dynamics: 0.4 };

/// The sound a show asks for: its moods' targets blended by weight, and its pictures' own
/// energy when known pulling the energy half the way towards it.
pub fn sound_target(want: &MusicWant) -> SoundTarget {
    let mut sum = SoundTarget { tempo: 0.0, energy: 0.0, brightness: 0.0, dynamics: 0.0 };
    let mut weight = 0.0;
    for (mood, w) in &want.moods {
        let t = MOOD_SOUND.iter().find(|(m, _)| m.eq_ignore_ascii_case(mood)).map(|(_, t)| *t).unwrap_or(NEUTRAL_SOUND);
        let w = w.max(0.0);
        sum.tempo += t.tempo * w;
        sum.energy += t.energy * w;
        sum.brightness += t.brightness * w;
        sum.dynamics += t.dynamics * w;
        weight += w;
    }
    let mut target = if weight > 0.0 {
        SoundTarget { tempo: sum.tempo / weight, energy: sum.energy / weight, brightness: sum.brightness / weight, dynamics: sum.dynamics / weight }
    } else {
        NEUTRAL_SOUND
    };
    if let Some(e) = want.energy {
        target.energy = 0.5 * target.energy + 0.5 * e.min(MAX_ENERGY);
    }
    target
}

/// How close a track sounds to `target`, 0..1. Energy and tempo are most of what makes music
/// feel calm or lively; brightness and dynamics shade it.
pub fn sound_fit(p: &TrackProfile, target: &SoundTarget) -> f64 {
    let d = 0.3 * (p.tempo_norm() as f64 - target.tempo).abs()
        + 0.35 * (p.energy_norm() as f64 - target.energy).abs()
        + 0.2 * (p.brightness as f64 - target.brightness).abs()
        + 0.15 * (p.dynamics_norm() as f64 - target.dynamics).abs();
    (1.0 - 2.0 * d).clamp(0.0, 1.0)
}

/// A track judged by its tags *and* its sound (Round 6).
pub fn suits_heard(tags: &[String], duration_ms: u32, profile: &TrackProfile, want: &MusicWant) -> f64 {
    0.5 * sound_fit(profile, &sound_target(want)) + 0.3 * mood_fit(tags, want) + 0.2 * length_fit(duration_ms, want.target_ms)
        - dramatic_penalty(tags)
}

/// A tiny deterministic jitter, so two shows with the same shape do not always land on the
/// same track while one show always does. It is far too small to override a real difference.
fn jitter(id: &str, seed: i64) -> f64 {
    let hash = id.bytes().fold(seed as u64 ^ 0x9E37_79B9, |a, b| a.wrapping_mul(31).wrapping_add(b as u64));
    (hash % 1000) as f64 / 1_000_000.0
}

/// The track to put under a show, or `None` when nothing in the library suits it — silence
/// is better than the wrong music.
pub fn pick_track<'a>(tracks: &'a [SavedTrack], want: &MusicWant) -> Option<&'a SavedTrack> {
    pick_track_with_energy(tracks, want, |_| None)
}

/// `pick_track`, knowing how busy each track is (onsets per second over 6, 0..1).
pub fn pick_track_with_energy<'a>(
    tracks: &'a [SavedTrack],
    want: &MusicWant,
    energy_of: impl Fn(&SavedTrack) -> Option<f64>,
) -> Option<&'a SavedTrack> {
    tracks
        .iter()
        .map(|t| (t, suits(&t.tags, t.duration_ms, energy_of(t), want) + jitter(&t.id, want.seed)))
        .filter(|(_, score)| *score >= PICK_FLOOR)
        .max_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(t, _)| t)
}

/// The same judgement over a track that is still out on Jamendo.
pub fn suits_found(track: &MusicTrack, want: &MusicWant) -> f64 {
    suits(&track.tags, track.duration_ms, None, want) + jitter(&track.id, want.seed)
}

/// Where automatic music is searched and fetched: Jamendo in the app, a fake in tests.
pub trait Searcher {
    fn search(&self, q: &MusicQuery) -> impl std::future::Future<Output = AppResult<Vec<MusicTrack>>> + Send;
    /// The track's audio, to listen to and — if it wins — to save.
    fn fetch(&self, track: &MusicTrack) -> impl std::future::Future<Output = AppResult<Vec<u8>>> + Send;
}

/// Jamendo with the user's client id.
pub struct Jamendo<'a> {
    pub client: &'a reqwest::Client,
    pub key: &'a str,
}

impl Searcher for Jamendo<'_> {
    async fn search(&self, q: &MusicQuery) -> AppResult<Vec<MusicTrack>> {
        search(self.client, self.key, q).await
    }

    async fn fetch(&self, track: &MusicTrack) -> AppResult<Vec<u8>> {
        let url = if track.download_url.is_empty() { &track.preview_url } else { &track.download_url };
        fetch(self.client, url).await
    }
}

/// How many automatic picks are remembered, and how much a remembered one loses: the same
/// tune under two shows in a row sounds like the app has only one.
pub const RECENT_KEEP: usize = 5;
pub const RECENT_PENALTY: f64 = 0.1;

fn recent_path(library_root: &Path) -> PathBuf {
    music_dir(library_root).join("recent.json")
}

/// The last automatic picks, newest first (`<library>/music/recent.json`).
pub fn recently_used(library_root: &Path) -> Vec<String> {
    std::fs::read_to_string(recent_path(library_root)).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

/// Remembers an automatic pick. A failure to write only loses the memory of it.
pub fn note_used(library_root: &Path, id: &str) {
    let mut ids = recently_used(library_root);
    ids.retain(|i| i != id);
    ids.insert(0, id.to_string());
    ids.truncate(RECENT_KEEP);
    let _ = std::fs::create_dir_all(music_dir(library_root));
    let _ = std::fs::write(recent_path(library_root), serde_json::to_string(&ids).unwrap_or_default());
}

/// Jamendo's speed tag for how busy the pictures are.
fn speed_for(energy: f64) -> &'static str {
    if energy >= 0.6 {
        "high"
    } else if energy >= 0.3 {
        "medium"
    } else {
        "low"
    }
}

/// How far into the results a query starts, by show (Round 6): 0, 25 or 50, so one show
/// always sees the same pages and different shows see different ones.
fn offset_for(seed: i64, query: usize) -> u32 {
    ((seed.unsigned_abs() as usize + query) % 3) as u32 * 25
}

/// The searches that find music for a show (Round 4F): its two heaviest moods, each alone and
/// with the speed its pictures' energy asks for, instrumental, 50 results each, lasting from
/// 60 % to 140 % of the show. All by relevance (Round 6: the popularity order handed every
/// show the same few tracks), the first from the top and the rest from a page that depends on
/// the show.
pub fn queries_for(want: &MusicWant) -> Vec<MusicQuery> {
    let secs = (want.target_ms / 1000).max(1);
    let duration = ((secs * 6 / 10).max(30), (secs * 14 / 10).max(90));
    let base = MusicQuery {
        instrumental_only: true,
        limit: 50,
        duration_between: Some(duration),
        order: Some("relevance".into()),
        ..MusicQuery::default()
    };
    let moods: Vec<Option<String>> = match want.moods.iter().take(2).map(|m| Some(m.0.clone())).collect::<Vec<_>>() {
        m if m.is_empty() => vec![None],
        m => m,
    };
    let mut out: Vec<MusicQuery> = Vec::new();
    for vibe in moods {
        let q = MusicQuery { vibe: vibe.clone(), ..base.clone() };
        out.push(q.clone());
        if let Some(e) = want.energy {
            out.push(MusicQuery { tempo: Some(speed_for(e.min(MAX_ENERGY)).into()), ..q });
        }
    }
    out.truncate(4);
    for (i, q) in out.iter_mut().enumerate().skip(1) {
        q.offset = Some(offset_for(want.seed, i));
    }
    out
}

/// Every query at once. `None` when every one failed (offline, Jamendo down, a bad key);
/// otherwise the results, each track once.
pub async fn search_many<S: Searcher>(searcher: &S, queries: &[MusicQuery]) -> Option<Vec<MusicTrack>> {
    let results = futures_util::future::join_all(queries.iter().map(|q| searcher.search(q))).await;
    if results.iter().all(|r| r.is_err()) {
        return None;
    }
    let mut seen = std::collections::HashSet::new();
    Some(results.into_iter().flatten().flatten().filter(|t| seen.insert(t.id.clone())).collect())
}

/// How many of the best-tagged results are listened to before choosing (Round 6).
pub const SHORTLIST: usize = 6;
/// Fewer than this and the search is widened (`wider_queries_for`).
pub const MIN_SHORTLIST: usize = 3;

/// The second, wider search when the first found too little (Round 6): each of the show's
/// moods alone, any length from 40 % to three times the show's, from the top of the results.
/// Instrumental still — a voice competes with the pictures.
pub fn wider_queries_for(want: &MusicWant) -> Vec<MusicQuery> {
    let secs = (want.target_ms / 1000).max(1);
    let duration = ((secs * 4 / 10).max(20), (secs * 3).max(120));
    let base = MusicQuery {
        instrumental_only: true,
        limit: 50,
        duration_between: Some(duration),
        order: Some("relevance".into()),
        ..MusicQuery::default()
    };
    let mut out: Vec<MusicQuery> = want.moods.iter().take(3).map(|m| MusicQuery { vibe: Some(m.0.clone()), ..base.clone() }).collect();
    if out.is_empty() {
        out.push(base);
    }
    out
}
/// How many are fetched at once.
const FETCH_AT_ONCE: usize = 3;

/// The music a new show gets, and what it sounds like when that was measured.
#[derive(Debug, Clone)]
pub struct Pick {
    pub track: SavedTrack,
    pub profile: Option<TrackProfile>,
}

/// The file name a saved track has, which is how shows refer to it (`music/<file>`).
pub fn file_name_of(track: &SavedTrack) -> String {
    Path::new(&track.path).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()
}

/// The music for a new show (Round 4F, Round 6). With a searcher (the user entered a client
/// id) it searches Jamendo (`queries_for`), leaves out anything another show already uses
/// (`in_use`, file names in the music folder), shortlists the `SHORTLIST` best by tags and
/// length, **listens** to each (`analyse` the fetched audio into a `TrackProfile`) and takes
/// the one that sounds most like the show's mood (`suits_heard`); only the winner is saved.
/// The saved library is the fallback when every search or fetch fails, judged by the profiles
/// already measured (`saved_profile`). A recent pick loses `RECENT_PENALTY`. Nothing that suits
/// the show (`PICK_FLOOR`) means silence, which is better than the wrong music.
///
/// Failure is never fatal. A show with no music is still a show, and a network error here
/// must not lose the slideshow the user just asked for: every path returns `None` instead.
pub async fn music_for<S, A, F>(
    searcher: Option<&S>,
    library_root: &Path,
    want: &MusicWant,
    in_use: &std::collections::HashSet<String>,
    saved_profile: impl Fn(&SavedTrack) -> Option<TrackProfile>,
    analyse: A,
) -> Option<Pick>
where
    S: Searcher,
    A: Fn(Vec<u8>) -> F,
    F: std::future::Future<Output = Option<TrackProfile>>,
{
    let saved = list_saved(library_root);
    let recent = recently_used(library_root);
    let penalty = |id: &str| if recent.iter().any(|r| r == id) { RECENT_PENALTY } else { 0.0 };
    if let Some(searcher) = searcher {
        if let Some(mut found) = search_many(searcher, &queries_for(want)).await {
            let eligible = |found: &[MusicTrack]| -> Vec<(f64, MusicTrack)> {
                found
                    .iter()
                    .filter(|t| usable(t))
                    .filter(|t| !in_use.contains(&format!("{}.mp3", safe_id(&t.id))))
                    .map(|t| (suits_found(t, want) - penalty(&safe_id(&t.id)), t.clone()))
                    .filter(|(score, _)| *score >= PICK_FLOOR)
                    .collect()
            };
            let mut shortlist = eligible(&found);
            // A narrow mood finds few tracks, and once the library's shows use those there is
            // nothing left: search again, wider, before settling for less (Round 6).
            if shortlist.len() < MIN_SHORTLIST {
                if let Some(more) = search_many(searcher, &wider_queries_for(want)).await {
                    let known: std::collections::HashSet<String> = found.iter().map(|t| t.id.clone()).collect();
                    found.extend(more.into_iter().filter(|t| !known.contains(&t.id)));
                    shortlist = eligible(&found);
                }
            }
            shortlist.sort_by(|a, b| b.0.total_cmp(&a.0));
            shortlist.truncate(SHORTLIST);
            // Listen to each: a track already saved is read from disk, the rest fetched.
            let mut heard: Vec<(f64, MusicTrack, Vec<u8>, Option<TrackProfile>)> = Vec::new();
            for batch in shortlist.chunks(FETCH_AT_ONCE) {
                let fetched = futures_util::future::join_all(batch.iter().map(|(_, t)| async {
                    let local = saved.iter().find(|s| s.id == safe_id(&t.id) && !s.credit.is_empty()).and_then(|s| std::fs::read(&s.path).ok());
                    match local {
                        Some(bytes) => Ok(bytes),
                        None => searcher.fetch(t).await,
                    }
                }))
                .await;
                for ((tag_score, t), bytes) in batch.iter().zip(fetched) {
                    let Ok(bytes) = bytes else { continue };
                    let profile = analyse(bytes.clone()).await;
                    let score = match &profile {
                        Some(p) => suits_heard(&t.tags, t.duration_ms, p, want) + jitter(&t.id, want.seed) - penalty(&safe_id(&t.id)),
                        // Not heard (no FFmpeg, an unreadable file): its tags, a little behind
                        // the ones that were.
                        None => tag_score - 0.05,
                    };
                    heard.push((score, t.clone(), bytes, profile));
                }
            }
            if let Some((_, best, bytes, profile)) =
                heard.into_iter().filter(|h| h.0 >= PICK_FLOOR).max_by(|a, b| a.0.total_cmp(&b.0))
            {
                let id = safe_id(&best.id);
                let chosen = match saved.iter().find(|t| t.id == id && !t.credit.is_empty()) {
                    Some(t) => Some(t.clone()),
                    None => write_saved(library_root, &best, &bytes).ok(),
                };
                if let Some(chosen) = chosen {
                    note_used(library_root, &chosen.id);
                    return Some(Pick { track: chosen, profile });
                }
            }
            // Nothing could be heard or saved: the library is all there is.
        }
    }
    let picked = saved
        .iter()
        .filter(|t| !in_use.contains(&file_name_of(t)))
        .map(|t| {
            let profile = saved_profile(t);
            let score = match &profile {
                Some(p) => suits_heard(&t.tags, t.duration_ms, p, want),
                None => suits(&t.tags, t.duration_ms, None, want),
            } + jitter(&t.id, want.seed);
            (score - penalty(&t.id), t, profile)
        })
        .filter(|(score, _, _)| *score >= PICK_FLOOR)
        .max_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, t, profile)| Pick { track: t.clone(), profile })?;
    note_used(library_root, &picked.track.id);
    Some(picked)
}

/// The credit an export must carry, when its music came from the library with a sidecar.
/// A file the user chose themselves returns `None`: there is nobody to credit.
pub fn attribution_for(doc: &photos_core::slideshow::SlideshowDoc, library_root: &Path) -> Option<String> {
    let track = doc.audio.as_ref()?;
    let path = Path::new(&track.path);
    let path = if path.is_absolute() { path.to_path_buf() } else { library_root.join(path) };
    if !path.starts_with(music_dir(library_root)) {
        return None;
    }
    let saved = sidecar_of(&path)?;
    if saved.credit.is_empty() {
        return None;
    }
    Some(format!(
        "Music in this slideshow\n\n{}\n\nLicence: {}\nFrom Jamendo: {}\n",
        saved.credit, saved.licence_url, saved.share_url
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn track_with(id: &str, secs: u32, tags: &[&str]) -> SavedTrack {
        SavedTrack {
            id: id.into(),
            title: format!("T{id}"),
            artist: "A".into(),
            licence_name: "CC BY 4.0".into(),
            licence_url: String::new(),
            share_url: String::new(),
            credit: "c".into(),
            path: format!("C:/lib/music/{id}.mp3"),
            duration_ms: secs * 1000,
            tags: tags.iter().map(|t| t.to_string()).collect(),
        }
    }

    #[test]
    fn the_mood_matters_more_than_the_length() {
        let tracks = vec![track_with("a", 60, &["epic"]), track_with("b", 120, &["sad"])];
        let want = MusicWant { energy: None, moods: weighted(&["epic"]), target_ms: 110_000, seed: 1 };
        assert_eq!(pick_track(&tracks, &want).unwrap().id, "a");
    }

    #[test]
    fn a_track_long_enough_beats_one_that_would_run_out() {
        let tracks = vec![track_with("short", 30, &["calm"]), track_with("long", 130, &["calm"])];
        let want = MusicWant { energy: None, moods: weighted(&["calm"]), target_ms: 120_000, seed: 1 };
        assert_eq!(pick_track(&tracks, &want).unwrap().id, "long");
    }

    #[test]
    fn but_not_one_far_longer_than_the_show() {
        let tracks = vec![track_with("fits", 130, &["calm"]), track_with("endless", 900, &["calm"])];
        let want = MusicWant { energy: None, moods: weighted(&["calm"]), target_ms: 120_000, seed: 1 };
        assert_eq!(pick_track(&tracks, &want).unwrap().id, "fits");
    }

    #[test]
    fn the_first_mood_asked_for_beats_the_second() {
        let tracks = vec![track_with("a", 120, &["energetic"]), track_with("b", 120, &["epic"])];
        let want = MusicWant { energy: None, moods: weighted(&["epic", "energetic"]), target_ms: 120_000, seed: 1 };
        assert_eq!(pick_track(&tracks, &want).unwrap().id, "b");
    }

    #[test]
    fn the_same_show_always_gets_the_same_track() {
        let tracks = vec![track_with("a", 120, &["calm"]), track_with("b", 120, &["calm"])];
        let want = MusicWant { energy: None, moods: weighted(&["calm"]), target_ms: 120_000, seed: 77 };
        let first = pick_track(&tracks, &want).unwrap().id.clone();
        assert_eq!(pick_track(&tracks, &want).unwrap().id, first, "the same show, the same tune");
        let other = MusicWant { seed: 78, ..want.clone() };
        assert!(pick_track(&tracks, &other).is_some());
    }

    #[test]
    fn nothing_suitable_means_no_music_rather_than_bad_music() {
        let want = MusicWant { energy: None, moods: weighted(&["epic"]), target_ms: 300_000, seed: 1 };
        assert!(pick_track(&[track_with("a", 10, &["sad"])], &want).is_none(), "a ten-second sad track is not a trip film");
        assert!(pick_track(&[], &want).is_none());
    }

    #[test]
    fn a_memory_asks_for_music_that_suits_its_kind() {
        assert!(mood_for_memory(photos_core::memories::KIND_TRIP).iter().any(|m| m.0 == "happy"));
        assert!(mood_for_memory(photos_core::memories::KIND_EVENT).iter().any(|m| m.0 == "happy"));
        assert!(mood_for_memory(photos_core::memories::KIND_ON_THIS_DAY).iter().any(|m| m.0 == "dreamy"));
        assert!(!mood_for_memory(99).is_empty(), "an unknown kind still asks for something");
    }

    #[test]
    fn weighted_moods_from_a_kind_score_as_the_ordered_list_did() {
        let tracks = vec![track_with("a", 60, &["happy"]), track_with("b", 60, &["dreamy"])];
        let want = MusicWant { moods: mood_for_memory(photos_core::memories::KIND_TRIP), energy: None, target_ms: 60_000, seed: 1 };
        assert_eq!(pick_track(&tracks, &want).unwrap().id, "a", "happy leads the trip list");
    }

    /// Round 5: slideshows lean calm. No kind of memory asks for drama, the pictures' own
    /// mood is softened the same way, and a dramatic track loses to a gentle one.
    #[test]
    fn slideshows_lean_calm() {
        use photos_core::memories::{KIND_EVENT, KIND_ON_THIS_DAY, KIND_TRIP};
        for kind in [KIND_TRIP, KIND_EVENT, KIND_ON_THIS_DAY, 99] {
            assert!(mood_for_memory(kind).iter().all(|m| !DRAMATIC.contains(&m.0.as_str())), "kind {kind}");
        }
        let softened = soften(vec![("epic".into(), 0.6), ("energetic".into(), 0.3), ("calm".into(), 0.1)]);
        assert_eq!(softened[0].0, "happy", "{softened:?}");
        assert!((softened[0].1 - 0.9).abs() < 1e-9, "merged weights: {softened:?}");
        assert!(softened.iter().all(|m| !DRAMATIC.contains(&m.0.as_str())));

        let want = MusicWant { moods: weighted(&["happy"]), energy: None, target_ms: 60_000, seed: 1 };
        let tracks = vec![track_with("loud", 60, &["happy", "epic"]), track_with("gentle", 60, &["happy", "calm"])];
        assert_eq!(pick_track(&tracks, &want).unwrap().id, "gentle");
    }

    #[test]
    fn busy_pictures_ask_for_a_medium_pace_at_most() {
        let qs = queries_for(&MusicWant { moods: weighted(&["happy"]), energy: Some(0.95), target_ms: 60_000, seed: 1 });
        assert!(qs.iter().filter_map(|q| q.tempo.as_deref()).all(|t| t == "medium"), "{qs:?}");
    }

    #[test]
    fn the_photos_choose_the_mood() {
        use photos_core::embedding::{quantize, Prototypes};
        let axis = |k: usize| {
            let mut v = vec![0f32; 512];
            v[k] = 1.0;
            quantize(&v)
        };
        let protos = Prototypes { model: "m".into(), items: vec![("mood:calm".into(), axis(0)), ("mood:energetic".into(), axis(1))] };
        let calm_photo = axis(0);
        let (moods, energy) = mood_from_photos(&[&calm_photo], &protos).unwrap();
        assert_eq!(moods[0].0, "calm");
        assert!(energy < 0.1, "{energy}");
        assert!(mood_from_photos(&[], &protos).is_none());
        assert!(mood_from_photos(&[&calm_photo], &Prototypes::empty()).is_none());
    }

    #[test]
    fn a_busy_track_suits_busy_pictures() {
        let tracks = vec![track_with("slow", 60, &["happy"]), track_with("busy", 60, &["happy"])];
        let energies = [("slow", 0.1), ("busy", 0.9)];
        let want = MusicWant { moods: vec![("happy".into(), 1.0)], energy: Some(0.85), target_ms: 60_000, seed: 1 };
        let best = pick_track_with_energy(&tracks, &want, |t| energies.iter().find(|e| e.0 == t.id).map(|e| e.1)).unwrap();
        assert_eq!(best.id, "busy");
    }

    #[test]
    fn an_old_sidecar_without_tags_still_reads() {
        let json = r#"{"id":"1","title":"t","artist":"a","licenceName":"CC BY 4.0","licenceUrl":"","shareUrl":"","credit":"c","path":"p","durationMs":1000}"#;
        let t: SavedTrack = serde_json::from_str(json).unwrap();
        assert!(t.tags.is_empty(), "a track saved before 6d has no tags, and that is fine");
    }

    #[test]
    fn a_saved_track_remembers_the_tags_it_was_found_by() {
        let dir = tempfile::tempdir().unwrap();
        let track = parse_tracks(include_str!("../tests/jamendo_fixture.json")).unwrap().remove(0);
        assert!(!track.tags.is_empty(), "the fixture has tags to keep");
        let saved = write_saved(dir.path(), &track, b"x").unwrap();
        assert_eq!(saved.tags, track.tags);
        assert_eq!(list_saved(dir.path())[0].tags, track.tags, "and they survive the sidecar");
    }


    #[test]
    fn a_query_becomes_a_url_without_leaking_anything_extra() {
        let q = MusicQuery {
            text: Some("sunrise".into()),
            vibe: Some("happy".into()),
            genre: Some("ambient".into()),
            instrumental_only: true,
            tempo: Some("low".into()),
            include_no_derivatives: false,
            limit: 24,
            order: None,
            duration_between: Some((60, 180)),
            offset: Some(50),
        };
        let url = search_url("KEY123", &q);
        assert!(url.starts_with("https://api.jamendo.com/v3.0/tracks/?"), "{url}");
        for part in [
            "client_id=KEY123",
            "format=json",
            "limit=24",
            "include=musicinfo+licenses",
            "audioformat=mp32",
            "search=sunrise",
            "fuzzytags=happy+ambient",
            "vocalinstrumental=instrumental",
            "speed=low",
            "durationbetween=60_180",
            "offset=50",
        ] {
            assert!(url.contains(part), "missing {part} in {url}");
        }
    }

    #[test]
    fn an_empty_query_asks_for_what_is_popular() {
        let url = search_url("K", &MusicQuery::default());
        assert!(url.contains("order=popularity_total"), "{url}");
        assert!(!url.contains("search="), "{url}");
        assert!(!url.contains("vocalinstrumental="), "{url}");
    }

    #[test]
    fn a_search_term_is_encoded() {
        let url = search_url("K", &MusicQuery { text: Some("café piano".into()), ..Default::default() });
        assert!(url.contains("search=caf%C3%A9+piano"), "{url}");
    }

    #[test]
    fn the_key_is_never_in_an_error() {
        let err = parse_tracks("not json").unwrap_err();
        assert!(!format!("{err:?}").contains("client_id"));
    }

    #[test]
    fn a_real_response_parses() {
        let tracks = parse_tracks(include_str!("../tests/jamendo_fixture.json")).unwrap();
        assert_eq!(tracks.len(), 3);
        let first = &tracks[0];
        assert_eq!(first.title, "Sunny Afternoon");
        assert_eq!(first.artist, "Jasmine Jordan");
        assert_eq!(first.duration_ms, 184_000);
        assert_eq!(first.licence_name, "CC BY 3.0");
        assert!(first.derivatives_allowed);
        assert!(first.tags.contains(&"happy".to_string()));
        assert!(first.tags.contains(&"pop".to_string()));
    }

    #[test]
    fn a_no_derivatives_track_is_not_usable_in_a_video() {
        let tracks = parse_tracks(include_str!("../tests/jamendo_fixture.json")).unwrap();
        let nd = tracks.iter().find(|t| t.licence_url.contains("-nd")).expect("the fixture has an ND track");
        assert!(!nd.derivatives_allowed);
        assert!(!usable(nd));
        assert_eq!(nd.licence_name, "CC BY-NC-ND 2.0");
        assert!(usable(&tracks[1]), "share-alike is fine");
    }

    #[test]
    fn a_credit_line_names_the_artist_the_track_and_the_licence() {
        let tracks = parse_tracks(include_str!("../tests/jamendo_fixture.json")).unwrap();
        let credit = credit_line(&tracks[0]);
        assert!(credit.contains("Sunny Afternoon"));
        assert!(credit.contains("Jasmine Jordan"));
        assert!(credit.contains("CC BY 3.0"));
        assert!(credit.contains("https://www.jamendo.com/track/1886711"));
    }

    fn fixture_track() -> MusicTrack {
        parse_tracks(include_str!("../tests/jamendo_fixture.json")).unwrap().remove(0)
    }

    #[test]
    fn a_saved_track_writes_a_sidecar_that_can_be_read_back() {
        let dir = tempfile::tempdir().unwrap();
        let track = MusicTrack { id: "42".into(), ..fixture_track() };
        let saved = write_saved(dir.path(), &track, b"ID3 fake mp3 bytes").unwrap();
        assert_eq!(PathBuf::from(&saved.path), dir.path().join("music").join("42.mp3"));
        assert!(PathBuf::from(&saved.path).is_file());
        assert!(saved.credit.contains("Jasmine Jordan"));

        let listed = list_saved(dir.path());
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].title, "Sunny Afternoon");
        assert_eq!(sidecar_of(Path::new(&saved.path)).unwrap().artist, "Jasmine Jordan");
    }

    #[test]
    fn a_music_file_with_no_sidecar_is_listed_as_itself() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("music")).unwrap();
        std::fs::write(dir.path().join("music").join("mine.mp3"), b"x").unwrap();
        let listed = list_saved(dir.path());
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].title, "mine");
        assert!(listed[0].credit.is_empty(), "the user's own file needs no credit");
    }

    #[test]
    fn an_id_from_the_network_cannot_escape_the_music_folder() {
        let dir = tempfile::tempdir().unwrap();
        let track = MusicTrack { id: "../../evil".into(), ..fixture_track() };
        let saved = write_saved(dir.path(), &track, b"x").unwrap();
        assert!(PathBuf::from(&saved.path).starts_with(dir.path().join("music")), "{}", saved.path);
    }

    // Round 4F, Task 1: automatic music from the whole catalogue.

    fn found(id: &str, secs: u32, tags: &[&str]) -> MusicTrack {
        MusicTrack { id: id.into(), duration_ms: secs * 1000, tags: tags.iter().map(|t| t.to_string()).collect(), ..fixture_track() }
    }

    /// Answers every query with `tracks` (or fails them all); a track's audio is its id's
    /// bytes, and an id in `unreachable` cannot be fetched.
    struct Fake {
        tracks: Option<Vec<MusicTrack>>,
        searched: Mutex<Vec<MusicQuery>>,
        fetched: Mutex<Vec<String>>,
        unreachable: Vec<String>,
        /// What a wider search (a length range reaching past 300 s) finds instead, when set.
        wider: Option<Vec<MusicTrack>>,
    }

    impl Fake {
        fn new(tracks: Option<Vec<MusicTrack>>) -> Self {
            Self { tracks, searched: Mutex::new(Vec::new()), fetched: Mutex::new(Vec::new()), unreachable: Vec::new(), wider: None }
        }
    }

    impl Searcher for Fake {
        async fn search(&self, q: &MusicQuery) -> AppResult<Vec<MusicTrack>> {
            self.searched.lock().unwrap().push(q.clone());
            if let (Some(wider), Some((_, to))) = (&self.wider, q.duration_between) {
                if to >= 300 {
                    return Ok(wider.clone());
                }
            }
            self.tracks.clone().ok_or_else(|| offline("no network"))
        }

        async fn fetch(&self, track: &MusicTrack) -> AppResult<Vec<u8>> {
            if self.unreachable.contains(&track.id) {
                return Err(offline("gone"));
            }
            self.fetched.lock().unwrap().push(track.id.clone());
            Ok(track.id.as_bytes().to_vec())
        }
    }

    fn calm(secs: u32) -> MusicWant {
        MusicWant { moods: weighted(&["calm"]), energy: None, target_ms: secs * 1000, seed: 1 }
    }

    fn run<T>(f: impl std::future::Future<Output = T>) -> T {
        tokio::runtime::Builder::new_current_thread().build().unwrap().block_on(f)
    }

    /// A measured sound on the profile's own scales.
    fn sounding(tempo_bpm: f32, onsets: f32, brightness: f32, dynamics_db: f32) -> TrackProfile {
        TrackProfile {
            version: photos_core::music_profile::PROFILE_VERSION,
            duration_ms: 120_000,
            tempo: tempo_bpm,
            energy: onsets,
            brightness,
            loudness_db: -14.0,
            dynamics: dynamics_db,
            curve: Vec::new(),
            brightness_curve: Vec::new(),
            sections: Vec::new(),
        }
    }

    /// Picks with nothing in use and no track ever heard.
    fn pick_unheard<S: Searcher>(s: Option<&S>, root: &Path, want: &MusicWant) -> Option<SavedTrack> {
        run(music_for(s, root, want, &Default::default(), |_| None, |_| async { None })).map(|p| p.track)
    }

    #[test]
    fn online_a_better_track_beats_a_saved_one_and_is_saved() {
        let dir = tempfile::tempdir().unwrap();
        write_saved(dir.path(), &found("1", 30, &["calm"]), b"x").unwrap();
        let fake = Fake::new(Some(vec![found("2", 120, &["calm"])]));
        let got = pick_unheard(Some(&fake), dir.path(), &calm(120)).unwrap();
        assert_eq!(got.id, "2");
        assert_eq!(*fake.fetched.lock().unwrap(), vec!["2".to_string()]);
        assert_eq!(std::fs::read(&got.path).unwrap(), b"2", "the bytes listened to are the bytes saved");
    }

    #[test]
    fn online_a_saved_track_not_in_the_results_is_never_chosen() {
        let dir = tempfile::tempdir().unwrap();
        // A perfect saved track, but the search found something else that suits.
        write_saved(dir.path(), &found("1", 120, &["calm"]), b"x").unwrap();
        let fake = Fake::new(Some(vec![found("2", 100, &["calm"])]));
        assert_eq!(pick_unheard(Some(&fake), dir.path(), &calm(120)).unwrap().id, "2");
    }

    #[test]
    fn the_winner_already_saved_is_read_rather_than_fetched() {
        let dir = tempfile::tempdir().unwrap();
        write_saved(dir.path(), &found("2", 120, &["calm"]), b"x").unwrap();
        let fake = Fake::new(Some(vec![found("2", 120, &["calm"])]));
        assert_eq!(pick_unheard(Some(&fake), dir.path(), &calm(120)).unwrap().id, "2");
        assert!(fake.fetched.lock().unwrap().is_empty());
    }

    #[test]
    fn the_same_track_is_not_picked_for_two_shows_in_a_row() {
        let dir = tempfile::tempdir().unwrap();
        let fake = Fake::new(Some(vec![found("best", 120, &["calm"]), found("close", 110, &["calm"])]));
        let first = pick_unheard(Some(&fake), dir.path(), &calm(120)).unwrap();
        let second = pick_unheard(Some(&fake), dir.path(), &calm(120)).unwrap();
        assert_eq!(first.id, "best");
        assert_eq!(second.id, "close", "within the penalty, the other one");
        assert_eq!(recently_used(dir.path()), vec!["close".to_string(), "best".to_string()]);
    }

    #[test]
    fn offline_falls_back_to_the_library() {
        let dir = tempfile::tempdir().unwrap();
        write_saved(dir.path(), &found("1", 120, &["calm"]), b"x").unwrap();
        let fake = Fake::new(None);
        assert_eq!(pick_unheard(Some(&fake), dir.path(), &calm(120)).unwrap().id, "1");
        // And with no key at all.
        assert_eq!(pick_unheard(None::<&Fake>, dir.path(), &calm(120)).unwrap().id, "1");
    }

    #[test]
    fn a_no_derivatives_result_is_never_chosen() {
        let dir = tempfile::tempdir().unwrap();
        let nd = MusicTrack { licence_url: "https://creativecommons.org/licenses/by-nc-nd/3.0/".into(), derivatives_allowed: false, ..found("nd", 120, &["calm"]) };
        let fake = Fake::new(Some(vec![nd, found("ok", 60, &["calm"])]));
        assert_eq!(pick_unheard(Some(&fake), dir.path(), &calm(120)).unwrap().id, "ok");
    }

    #[test]
    fn nothing_that_suits_means_silence() {
        let dir = tempfile::tempdir().unwrap();
        let fake = Fake::new(Some(vec![found("sad", 10, &["sad"])]));
        assert!(pick_unheard(Some(&fake), dir.path(), &calm(300)).is_none());
    }

    // Round 6: unique music, chosen by listening.

    #[test]
    fn a_track_another_show_uses_is_never_chosen() {
        let dir = tempfile::tempdir().unwrap();
        let in_use: std::collections::HashSet<String> = ["best.mp3".to_string()].into();
        let fake = Fake::new(Some(vec![found("best", 120, &["calm"]), found("other", 100, &["calm"])]));
        let got = run(music_for(Some(&fake), dir.path(), &calm(120), &in_use, |_| None, |_| async { None })).unwrap();
        assert_eq!(got.track.id, "other");
        assert!(!fake.fetched.lock().unwrap().contains(&"best".to_string()), "not even listened to");
        // Nor from the library when offline.
        write_saved(dir.path(), &found("best", 120, &["calm"]), b"x").unwrap();
        let offline = Fake::new(None);
        let got = run(music_for(Some(&offline), dir.path(), &calm(120), &in_use, |_| None, |_| async { None })).unwrap();
        assert_eq!(got.track.id, "other");
    }

    #[test]
    fn a_calm_show_gets_the_track_that_sounds_calm() {
        let dir = tempfile::tempdir().unwrap();
        // Both are tagged calm and equally long: before Round 6 a hash decided between them.
        let fake = Fake::new(Some(vec![found("busy", 120, &["calm"]), found("quiet", 120, &["calm"])]));
        let analyse = |bytes: Vec<u8>| async move {
            Some(if bytes == b"quiet" { sounding(72.0, 0.9, 0.3, 3.0) } else { sounding(150.0, 4.5, 0.8, 4.0) })
        };
        let got = run(music_for(Some(&fake), dir.path(), &calm(120), &Default::default(), |_| None, analyse)).unwrap();
        assert_eq!(got.track.id, "quiet");
        assert!(got.profile.is_some(), "the profile comes back for the window and the cache");
        // And a lively show the lively one.
        let dir = tempfile::tempdir().unwrap();
        let happy = MusicWant { moods: weighted(&["happy"]), energy: Some(0.5), target_ms: 120_000, seed: 1 };
        let fake = Fake::new(Some(vec![found("busy", 120, &["happy"]), found("quiet", 120, &["happy"])]));
        let got = run(music_for(Some(&fake), dir.path(), &happy, &Default::default(), |_| None, analyse)).unwrap();
        assert_eq!(got.track.id, "busy");
    }

    #[test]
    fn a_track_that_cannot_be_fetched_is_passed_over() {
        let dir = tempfile::tempdir().unwrap();
        let mut fake = Fake::new(Some(vec![found("best", 120, &["calm"]), found("other", 100, &["calm"])]));
        fake.unreachable = vec!["best".into()];
        assert_eq!(pick_unheard(Some(&fake), dir.path(), &calm(120)).unwrap().id, "other");
    }

    #[test]
    fn only_the_shortlist_is_listened_to() {
        let dir = tempfile::tempdir().unwrap();
        let many: Vec<MusicTrack> = (0..20).map(|i| found(&format!("t{i}"), 120, &["calm"])).collect();
        let fake = Fake::new(Some(many));
        pick_unheard(Some(&fake), dir.path(), &calm(120)).unwrap();
        assert_eq!(fake.fetched.lock().unwrap().len(), SHORTLIST);
        assert_eq!(list_saved(dir.path()).len(), 1, "only the winner is kept");
    }

    #[test]
    fn a_saved_track_is_judged_by_its_measured_sound() {
        let dir = tempfile::tempdir().unwrap();
        write_saved(dir.path(), &found("busy", 120, &["calm"]), b"x").unwrap();
        write_saved(dir.path(), &found("quiet", 120, &["calm"]), b"x").unwrap();
        let profile = |t: &SavedTrack| Some(if t.id == "quiet" { sounding(72.0, 0.9, 0.3, 3.0) } else { sounding(150.0, 4.5, 0.8, 4.0) });
        let got = run(music_for(None::<&Fake>, dir.path(), &calm(120), &Default::default(), profile, |_| async { None })).unwrap();
        assert_eq!(got.track.id, "quiet");
    }

    #[test]
    fn a_mood_sounds_like_itself() {
        let calm = sound_target(&MusicWant { moods: weighted(&["calm"]), energy: None, target_ms: 1, seed: 1 });
        let happy = sound_target(&MusicWant { moods: weighted(&["happy"]), energy: None, target_ms: 1, seed: 1 });
        let slow = sounding(70.0, 0.8, 0.3, 3.0);
        let fast = sounding(140.0, 3.5, 0.7, 4.0);
        assert!(sound_fit(&slow, &calm) > sound_fit(&fast, &calm) + 0.3);
        assert!(sound_fit(&fast, &happy) > sound_fit(&slow, &happy) + 0.2);
    }

    #[test]
    fn a_track_carrying_more_of_the_wanted_moods_scores_higher() {
        let want = MusicWant { moods: weighted(&["calm", "dreamy"]), energy: None, target_ms: 1, seed: 1 };
        let one = mood_fit(&["calm".into()], &want);
        let both = mood_fit(&["calm".into(), "dreamy".into()], &want);
        assert!(both > one, "{both} vs {one}");
    }

    #[test]
    fn queries_cover_two_moods_with_and_without_the_speed() {
        let want = MusicWant { moods: weighted(&["epic", "happy", "calm"]), energy: Some(0.7), target_ms: 100_000, seed: 1 };
        let qs = queries_for(&want);
        assert_eq!(qs.len(), 4);
        let keys: std::collections::HashSet<_> = qs.iter().map(|q| (q.vibe.clone(), q.tempo.clone())).collect();
        assert_eq!(keys.len(), 4, "{qs:?}");
        assert!(qs.iter().all(|q| q.instrumental_only && q.limit == 50 && q.duration_between == Some((60, 140))));
        // Round 6: never by popularity, which handed every show the same few tracks.
        assert!(qs.iter().all(|q| q.order.as_deref() == Some("relevance")), "{qs:?}");
        // Busy pictures ask for a lively pace, but never a fast one (Round 5).
        assert!(qs.iter().any(|q| q.tempo.as_deref() == Some("medium")));
        assert!(qs.iter().all(|q| q.tempo.as_deref() != Some("high")));
        let one = queries_for(&MusicWant { moods: weighted(&["calm"]), energy: None, target_ms: 100_000, seed: 1 });
        assert_eq!(one.len(), 1);
    }

    #[test]
    fn search_many_merges_results_once_each() {
        let fake = Fake::new(Some(vec![found("1", 60, &[]), found("2", 60, &[])]));
        let qs = queries_for(&MusicWant { moods: weighted(&["a", "b"]), energy: Some(0.1), target_ms: 60_000, seed: 1 });
        let all = run(search_many(&fake, &qs)).unwrap();
        assert_eq!(all.len(), 2);
        assert_eq!(fake.searched.lock().unwrap().len(), 4);
        assert!(run(search_many(&Fake::new(None), &qs)).is_none());
    }

    #[test]
    fn different_shows_search_different_pages() {
        let q = |seed| queries_for(&MusicWant { moods: weighted(&["calm"]), energy: Some(0.2), target_ms: 60_000, seed });
        assert_eq!(q(1)[0].offset, None, "the first query always reads from the top");
        let pages: std::collections::HashSet<_> = (0..3).map(|s| q(s)[1].offset).collect();
        assert_eq!(pages.len(), 3, "{pages:?}");
        assert_eq!(q(5), q(5), "and one show always the same");
    }

    #[test]
    fn when_every_close_match_is_taken_the_search_widens() {
        let dir = tempfile::tempdir().unwrap();
        let in_use: std::collections::HashSet<String> = ["taken.mp3".to_string()].into();
        let mut fake = Fake::new(Some(vec![found("taken", 100, &["calm"])]));
        fake.wider = Some(vec![found("taken", 100, &["calm"]), found("further", 200, &["calm"])]);
        let got = run(music_for(Some(&fake), dir.path(), &calm(100), &in_use, |_| None, |_| async { None })).unwrap();
        assert_eq!(got.track.id, "further");
        // A first search that finds plenty never asks twice.
        let plenty = Fake::new(Some((0..5).map(|i| found(&format!("t{i}"), 100, &["calm"])).collect()));
        pick_unheard(Some(&plenty), dir.path(), &calm(100)).unwrap();
        assert_eq!(plenty.searched.lock().unwrap().len(), queries_for(&calm(100)).len());
    }
}
