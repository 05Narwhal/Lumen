//! The opt-in Discord status (Round 17): *Competing in Lumen* plus one coarse activity.
//!
//! Talks only to the Discord client on this machine, over its local IPC pipe. Ported from CPM's
//! `discord_presence.rs`, in layers that need no Discord to test:
//!
//! 1. [`presence_for`] / [`wanted`]: pure, from the settings and a fixed [`PresenceScreen`] to
//!    the activity to show (or `None` to clear). The only text that can leave is in this file.
//! 2. [`plan`]: a pure scheduler (15 s rate limit).
//! 3. [`Engine`]: connect, send, clear over an injectable [`Connector`], tested with a fake
//!    Discord. A worker thread drives it.
//!
//! Nothing here knows a file, album, person, place or date: the page sends the enum and nothing
//! else. Discord not running is the normal case and is only ever logged at `debug`.

use std::io::{self, Read, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use photos_core::settings::DiscordStatus;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

// *========== PURE: WHAT TO SHOW ==========*

/// Rich Presence art key for the Lumen mark (uploaded in Discord's developer portal).
pub const LARGE_IMAGE_KEY: &str = "lumen_logo";
const LARGE_IMAGE_TEXT: &str = "Lumen";
/// Discord activity type 5 (Competing): *Competing in Lumen*. Games use Playing (0) and CPM
/// Watching (3), so Lumen can sit beside both.
pub const ACTIVITY_TYPE_COMPETING: u8 = 5;
/// No input for this long clears the status.
pub const IDLE_CLEAR: Duration = Duration::from_secs(15 * 60);

/// What Lumen is doing, as coarse as it gets. The page sends one of these and nothing else.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum PresenceScreen {
    Browsing,
    Viewing,
    Developing,
    Slideshow,
}

impl PresenceScreen {
    fn line(self) -> &'static str {
        match self {
            Self::Browsing => "Browsing the library",
            Self::Viewing => "Looking through photos",
            Self::Developing => "Developing a photo",
            Self::Slideshow => "Watching a slideshow",
        }
    }
}

/// One Rich Presence activity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Activity {
    pub details: Option<String>,
    /// Elapsed-timer start, Unix seconds.
    pub start: Option<i64>,
}

impl Activity {
    /// The `activity` object of a `SET_ACTIVITY` command.
    pub fn to_json(&self) -> Value {
        let mut a = json!({
            "type": ACTIVITY_TYPE_COMPETING,
            "assets": { "large_image": LARGE_IMAGE_KEY, "large_text": LARGE_IMAGE_TEXT },
            "instance": false,
        });
        if let Some(d) = &self.details {
            a["details"] = json!(d);
        }
        if let Some(t) = self.start {
            a["timestamps"] = json!({ "start": t });
        }
        a
    }
}

/// The activity for `screen`, or `None` when the status is off. `started_at` is when Lumen
/// started (Unix seconds): the same for every screen, so the timer says how long Lumen has been
/// open, not what was looked at.
pub fn presence_for(s: &DiscordStatus, screen: PresenceScreen, started_at: i64) -> Option<Activity> {
    if !s.enabled {
        return None;
    }
    Some(Activity {
        details: s.show_activity.then(|| screen.line().to_string()),
        start: s.show_elapsed.then_some(started_at),
    })
}

/// What should be on Discord now: off, nothing reported yet, or idle for [`IDLE_CLEAR`] → `None`.
pub fn wanted(s: &DiscordStatus, live: Option<(PresenceScreen, Instant)>, started_at: i64, now: Instant) -> Option<Activity> {
    let (screen, seen) = live?;
    if now.saturating_duration_since(seen) >= IDLE_CLEAR {
        return None;
    }
    presence_for(s, screen, started_at)
}

/// The line Settings shows for what others see; built from the same pieces as the wire.
pub fn preview_line(s: &DiscordStatus, screen: PresenceScreen) -> String {
    let mut parts = vec!["Competing in Lumen"];
    if s.show_activity {
        parts.push(screen.line());
    }
    if s.show_elapsed {
        parts.push("with elapsed time");
    }
    parts.join(" \u{b7} ")
}

// *========== PURE: WIRE FRAMING ==========*

pub const OP_HANDSHAKE: u32 = 0;
pub const OP_FRAME: u32 = 1;
pub const OP_CLOSE: u32 = 2;
pub const OP_PING: u32 = 3;
pub const OP_PONG: u32 = 4;
/// Frames Discord sends are tiny; anything bigger is a corrupt stream, not a message.
const MAX_FRAME_LEN: u32 = 64 * 1024;

/// `opcode u32 LE`, `length u32 LE`, then the JSON payload.
pub fn encode_frame(opcode: u32, payload: &Value) -> Vec<u8> {
    let body = serde_json::to_vec(payload).unwrap_or_default();
    let mut out = Vec::with_capacity(8 + body.len());
    out.extend_from_slice(&opcode.to_le_bytes());
    out.extend_from_slice(&(body.len() as u32).to_le_bytes());
    out.extend_from_slice(&body);
    out
}

/// Split the 8-byte header into `(opcode, length)`.
pub fn decode_header(header: &[u8; 8]) -> (u32, u32) {
    (
        u32::from_le_bytes([header[0], header[1], header[2], header[3]]),
        u32::from_le_bytes([header[4], header[5], header[6], header[7]]),
    )
}

/// Read one frame. An oversized length or non-JSON body is an error, not a panic.
pub fn read_frame<R: Read>(r: &mut R) -> io::Result<(u32, Value)> {
    let mut header = [0u8; 8];
    r.read_exact(&mut header)?;
    let (op, len) = decode_header(&header);
    if len > MAX_FRAME_LEN {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "oversized Discord frame"));
    }
    let mut body = vec![0u8; len as usize];
    r.read_exact(&mut body)?;
    let value = if body.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&body).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))?
    };
    Ok((op, value))
}

pub fn handshake_payload(client_id: &str) -> Value {
    json!({ "v": 1, "client_id": client_id })
}

/// `SET_ACTIVITY`. `None` sends `activity: null`, which clears the status.
pub fn set_activity_payload(pid: u32, activity: &Option<Activity>, nonce: &str) -> Value {
    json!({
        "cmd": "SET_ACTIVITY",
        "args": { "pid": pid, "activity": activity.as_ref().map(Activity::to_json) },
        "nonce": nonce,
    })
}

// *========== PURE: SCHEDULING ==========*

/// Discord allows roughly one presence update per 15 s; stay inside it.
pub const MIN_UPDATE_GAP: Duration = Duration::from_secs(15);
/// How often to look for Discord when it is not running.
pub const RETRY_GAP: Duration = Duration::from_secs(15);

#[derive(Debug, Default, Clone)]
pub struct SchedState {
    pub connected: bool,
    /// What Discord shows, as last sent on this connection (`Some(None)` = cleared).
    pub sent: Option<Option<Activity>>,
    pub last_send: Option<Instant>,
    pub next_connect: Option<Instant>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    Wait,
    Connect,
    Send(Option<Activity>),
    Disconnect,
}

/// The next move. A send that waits out the rate limit always sends the latest `desired`,
/// because the caller recomputes it every tick. Clearing is exempt from the limit.
pub fn plan(state: &SchedState, desired: &Option<Activity>, now: Instant) -> Step {
    match desired {
        None => {
            if !state.connected {
                Step::Wait
            } else if matches!(state.sent, Some(Some(_))) {
                Step::Send(None)
            } else {
                // Cleared (or never set): drop the pipe so nothing stays open while off.
                Step::Disconnect
            }
        }
        Some(_) => {
            if !state.connected {
                match state.next_connect {
                    Some(t) if now < t => Step::Wait,
                    _ => Step::Connect,
                }
            } else if state.sent.as_ref() == Some(desired)
                || state.last_send.is_some_and(|t| now.duration_since(t) < MIN_UPDATE_GAP)
            {
                Step::Wait
            } else {
                Step::Send(desired.clone())
            }
        }
    }
}

// *========== TRANSPORT ==========*

/// Where Discord's IPC sockets can be on Unix: each runtime directory, plus the Flatpak and
/// Snap subfolders, ids 0–9, most likely first.
#[cfg_attr(windows, allow(dead_code))]
pub fn unix_pipe_candidates(env: &dyn Fn(&str) -> Option<String>) -> Vec<PathBuf> {
    let mut bases: Vec<String> = ["XDG_RUNTIME_DIR", "TMPDIR", "TMP", "TEMP"]
        .iter()
        .filter_map(|k| env(k))
        .map(|d| d.trim_end_matches('/').to_string())
        .filter(|d| !d.is_empty())
        .collect();
    bases.push("/tmp".to_string());
    let mut seen = std::collections::HashSet::new();
    bases.retain(|b| seen.insert(b.clone()));
    let mut out = Vec::new();
    for base in &bases {
        for sub in ["", "app/com.discordapp.Discord", "snap.discord"] {
            for n in 0..10u8 {
                let dir = if sub.is_empty() { base.clone() } else { format!("{base}/{sub}") };
                out.push(PathBuf::from(format!("{dir}/discord-ipc-{n}")));
            }
        }
    }
    out
}

pub trait Pipe: Read + Write + Send {}
impl<T: Read + Write + Send> Pipe for T {}

/// Opens Discord's pipe number `n` (0–9). Injected so tests can stand in for Discord.
pub trait Connector: Send {
    fn open(&mut self, n: u8) -> io::Result<Box<dyn Pipe>>;
}

/// The real thing: the named pipe on Windows, the Unix sockets elsewhere.
pub struct SystemConnector;

impl Connector for SystemConnector {
    fn open(&mut self, n: u8) -> io::Result<Box<dyn Pipe>> {
        #[cfg(windows)]
        {
            let f = std::fs::OpenOptions::new().read(true).write(true).open(format!(r"\\.\pipe\discord-ipc-{n}"))?;
            Ok(Box::new(f))
        }
        #[cfg(unix)]
        {
            let suffix = format!("discord-ipc-{n}");
            let mut last = io::Error::new(io::ErrorKind::NotFound, "no Discord socket");
            for p in unix_pipe_candidates(&|k| std::env::var(k).ok()) {
                if !p.ends_with(&suffix) {
                    continue;
                }
                match std::os::unix::net::UnixStream::connect(&p) {
                    Ok(s) => return Ok(Box::new(s)),
                    Err(e) => last = e,
                }
            }
            Err(last)
        }
    }
}

/// Connects, sends and clears according to [`plan`]. Every failure is a quiet retry.
pub struct Engine {
    client_id: String,
    connector: Box<dyn Connector>,
    state: SchedState,
    io: Option<Box<dyn Pipe>>,
    nonce: u64,
}

impl Engine {
    pub fn new(client_id: String, connector: Box<dyn Connector>) -> Self {
        Self { client_id, connector, state: SchedState::default(), io: None, nonce: 0 }
    }

    pub fn connected(&self) -> bool {
        self.state.connected
    }

    /// Discord currently shows an activity from us.
    pub fn showing(&self) -> bool {
        matches!(self.state.sent, Some(Some(_)))
    }

    pub fn tick(&mut self, desired: &Option<Activity>, now: Instant) {
        match plan(&self.state, desired, now) {
            Step::Wait => {}
            Step::Connect => match self.connect() {
                Ok(io) => {
                    self.io = Some(io);
                    self.state.connected = true;
                    self.state.sent = None;
                    self.state.next_connect = None;
                }
                Err(e) => {
                    tracing::debug!("[discord] not connected: {e}");
                    self.state.next_connect = Some(now + RETRY_GAP);
                }
            },
            Step::Send(act) => match self.send(&act) {
                Ok(()) => {
                    self.state.sent = Some(act);
                    self.state.last_send = Some(now);
                }
                Err(e) => {
                    tracing::debug!("[discord] send failed, will reconnect: {e}");
                    self.drop_connection();
                    self.state.next_connect = Some(now + RETRY_GAP);
                }
            },
            Step::Disconnect => self.drop_connection(),
        }
    }

    /// App exit: clear the status if one is shown, then close the pipe.
    pub fn clear_and_close(&mut self) {
        if self.showing() {
            let _ = self.send(&None);
        }
        self.drop_connection();
    }

    fn drop_connection(&mut self) {
        self.io = None;
        self.state.connected = false;
        self.state.sent = None;
    }

    fn connect(&mut self) -> io::Result<Box<dyn Pipe>> {
        let mut last = io::Error::new(io::ErrorKind::NotFound, "Discord is not running");
        for n in 0..10u8 {
            let mut io = match self.connector.open(n) {
                Ok(p) => p,
                Err(e) => {
                    last = e;
                    continue;
                }
            };
            let ready = io
                .write_all(&encode_frame(OP_HANDSHAKE, &handshake_payload(&self.client_id)))
                .and_then(|()| read_frame(&mut io));
            match ready {
                Ok((OP_FRAME, v)) if v.get("evt").and_then(Value::as_str) == Some("READY") => return Ok(io),
                Ok(_) => last = io::Error::new(io::ErrorKind::InvalidData, "Discord refused the handshake"),
                Err(e) => last = e,
            }
        }
        Err(last)
    }

    fn send(&mut self, act: &Option<Activity>) -> io::Result<()> {
        self.nonce += 1;
        let payload = set_activity_payload(std::process::id(), act, &format!("lumen-{}", self.nonce));
        let io = self.io.as_mut().ok_or_else(|| io::Error::new(io::ErrorKind::NotConnected, "no connection"))?;
        io.write_all(&encode_frame(OP_FRAME, &payload))?;
        loop {
            let (op, v) = read_frame(io)?;
            match op {
                OP_PING => io.write_all(&encode_frame(OP_PONG, &v))?,
                OP_CLOSE => return Err(io::Error::new(io::ErrorKind::ConnectionAborted, "Discord closed")),
                _ if v.get("evt").and_then(Value::as_str) == Some("ERROR") => {
                    return Err(io::Error::new(io::ErrorKind::InvalidInput, v.to_string()));
                }
                _ => return Ok(()),
            }
        }
    }
}

// *========== BUILD-TIME APPLICATION ID ==========*

/// The Discord Application ID, embedded at build time (`build.rs`) from the tracked
/// `src-tauri/discord_client_id.txt`; `LUMEN_DISCORD_CLIENT_ID` overrides it. A public
/// identifier. `None` (a build with neither) means Settings hides the section and no worker starts.
pub fn client_id() -> Option<&'static str> {
    resolve_client_id(option_env!("LUMEN_DISCORD_CLIENT_ID"))
}

fn resolve_client_id(env: Option<&'static str>) -> Option<&'static str> {
    env.map(str::trim).filter(|s| !s.is_empty())
}

// *========== STATE FOR SETTINGS ==========*

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ts_rs::TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum DiscordState {
    Off,
    /// On, but nothing published: Discord is not running, or the status was cleared after idling.
    Waiting,
    Showing,
}

pub fn state_of(enabled: bool, showing: bool) -> DiscordState {
    match (enabled, showing) {
        (false, _) => DiscordState::Off,
        (true, false) => DiscordState::Waiting,
        (true, true) => DiscordState::Showing,
    }
}

/// What Settings → Discord status shows.
#[derive(Debug, Clone, Serialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DiscordInfo {
    /// This build has an Application ID; without one the section is hidden.
    pub available: bool,
    pub state: DiscordState,
    pub preview: String,
}

// *========== WORKER ==========*

/// The last screen the page reported and when: any report counts as input for the idle clear.
#[derive(Debug, Default)]
pub struct Live(Option<(PresenceScreen, Instant)>);

impl Live {
    pub fn touch(&mut self, screen: PresenceScreen, now: Instant) {
        self.0 = Some((screen, now));
    }
    pub fn get(&self) -> Option<(PresenceScreen, Instant)> {
        self.0
    }
}

static LIVE: Mutex<Live> = Mutex::new(Live(None));
static SHOWING: AtomicBool = AtomicBool::new(false);
static STARTED: AtomicBool = AtomicBool::new(false);
static SHUTDOWN: AtomicBool = AtomicBool::new(false);
const TICK: Duration = Duration::from_secs(3);

/// The page reports the screen on a route change, and again as a heartbeat on input.
pub fn set_screen(screen: PresenceScreen) {
    LIVE.lock().unwrap_or_else(|e| e.into_inner()).touch(screen, Instant::now());
}

pub fn last_screen() -> PresenceScreen {
    LIVE.lock().unwrap_or_else(|e| e.into_inner()).get().map_or(PresenceScreen::Browsing, |(s, _)| s)
}

pub fn showing() -> bool {
    SHOWING.load(Ordering::SeqCst)
}

fn unix_now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64)
}

/// Start the worker. A build without an Application ID never starts it, so no pipe is ever
/// opened. Idempotent.
pub fn start(app: tauri::AppHandle) {
    use tauri::Manager;
    let Some(id) = client_id() else { return };
    if STARTED.swap(true, Ordering::SeqCst) {
        return;
    }
    let started_at = unix_now();
    let _ = std::thread::Builder::new().name("lumen-discord".into()).spawn(move || {
        let mut engine = Engine::new(id.to_string(), Box::new(SystemConnector));
        while !SHUTDOWN.load(Ordering::SeqCst) {
            std::thread::sleep(TICK);
            if SHUTDOWN.load(Ordering::SeqCst) {
                break;
            }
            // Never wait on the settings lock: skip the tick instead.
            let status = {
                let state = app.state::<crate::state::AppState>();
                let Ok(settings) = state.settings.try_read() else { continue };
                settings.discord.clone()
            };
            let live = LIVE.lock().unwrap_or_else(|e| e.into_inner()).get();
            let desired = wanted(&status, live, started_at, Instant::now());
            engine.tick(&desired, Instant::now());
            SHOWING.store(engine.showing(), Ordering::SeqCst);
        }
        engine.clear_and_close();
        SHOWING.store(false, Ordering::SeqCst);
    });
}

/// Ask the worker to clear the status and stop (app exit). The worker clears on its own thread;
/// if the process ends first, the pipe closing clears the status anyway.
pub fn shutdown() {
    SHUTDOWN.store(true, Ordering::SeqCst);
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};
    use std::collections::VecDeque;
    use std::io;
    use std::sync::{mpsc, Arc, Mutex};
    use std::time::{Duration, Instant};

    #[test]
    fn client_id_is_baked_into_a_clean_build() {
        // The id is a tracked file (src-tauri/discord_client_id.txt), so a build with no
        // environment still has the Discord status.
        assert!(client_id().is_some(), "no Discord application id was baked in");
    }

    const ALL: [PresenceScreen; 4] =
        [PresenceScreen::Browsing, PresenceScreen::Viewing, PresenceScreen::Developing, PresenceScreen::Slideshow];

    fn on() -> DiscordStatus {
        DiscordStatus { enabled: true, ..DiscordStatus::default() }
    }

    #[test]
    fn disabled_shows_nothing() {
        let s = DiscordStatus { enabled: false, ..on() };
        for screen in ALL {
            assert!(presence_for(&s, screen, 1_700_000_000).is_none());
        }
    }

    #[test]
    fn each_screen_has_its_fixed_line() {
        let line = |screen| presence_for(&on(), screen, 1).unwrap().details;
        assert_eq!(line(PresenceScreen::Browsing).as_deref(), Some("Browsing the library"));
        assert_eq!(line(PresenceScreen::Viewing).as_deref(), Some("Looking through photos"));
        assert_eq!(line(PresenceScreen::Developing).as_deref(), Some("Developing a photo"));
        assert_eq!(line(PresenceScreen::Slideshow).as_deref(), Some("Watching a slideshow"));
    }

    #[test]
    fn activity_off_leaves_details_out() {
        let s = DiscordStatus { show_activity: false, ..on() };
        let a = presence_for(&s, PresenceScreen::Developing, 1).unwrap();
        assert_eq!(a.details, None);
        assert!(a.to_json().get("details").is_none(), "{}", a.to_json());
    }

    #[test]
    fn elapsed_toggle() {
        assert_eq!(presence_for(&on(), PresenceScreen::Browsing, 1_700_000_000).unwrap().start, Some(1_700_000_000));
        let s = DiscordStatus { show_elapsed: false, ..on() };
        let a = presence_for(&s, PresenceScreen::Browsing, 1_700_000_000).unwrap();
        assert_eq!(a.start, None);
        assert!(a.to_json().get("timestamps").is_none());
    }

    #[test]
    fn type_is_competing_never_playing_or_watching() {
        let a = presence_for(&on(), PresenceScreen::Browsing, 1_700_000_000);
        let v = set_activity_payload(42, &a, "n1");
        assert_eq!(v["cmd"], "SET_ACTIVITY");
        assert_eq!(v["nonce"], "n1");
        assert_eq!(v["args"]["pid"], 42);
        assert_eq!(v["args"]["activity"]["type"], 5);
        assert_eq!(v["args"]["activity"]["assets"]["large_image"], "lumen_logo");
        assert_eq!(v["args"]["activity"]["timestamps"]["start"], 1_700_000_000);
        assert!(set_activity_payload(42, &None, "n2")["args"]["activity"].is_null());
    }

    fn strings(v: &Value, out: &mut Vec<String>) {
        match v {
            Value::String(s) => out.push(s.clone()),
            Value::Array(a) => a.iter().for_each(|x| strings(x, out)),
            Value::Object(o) => o.values().for_each(|x| strings(x, out)),
            _ => {}
        }
    }

    #[test]
    fn nothing_but_the_fixed_strings_reaches_the_wire() {
        let allowed = [
            "SET_ACTIVITY",
            "n",
            "Lumen",
            "lumen_logo",
            "Browsing the library",
            "Looking through photos",
            "Developing a photo",
            "Watching a slideshow",
        ];
        for screen in ALL {
            for mask in 0..8u8 {
                let s = DiscordStatus { enabled: mask & 1 != 0, show_activity: mask & 2 != 0, show_elapsed: mask & 4 != 0, ..DiscordStatus::default() };
                let payload = set_activity_payload(7, &presence_for(&s, screen, 1_700_000_000), "n");
                let mut found = Vec::new();
                strings(&payload, &mut found);
                for f in found {
                    assert!(allowed.contains(&f.as_str()), "unexpected string on the wire: {f:?} in {payload}");
                }
            }
        }
    }

    #[test]
    fn idle_for_15_minutes_clears() {
        let t0 = Instant::now();
        let live = Some((PresenceScreen::Browsing, t0));
        let s = on();
        assert!(wanted(&s, live, 1, t0 + IDLE_CLEAR - Duration::from_secs(1)).is_some());
        assert!(wanted(&s, live, 1, t0 + IDLE_CLEAR).is_none());
        assert!(wanted(&s, None, 1, t0).is_none(), "no screen reported yet");
        assert!(wanted(&DiscordStatus { enabled: false, ..on() }, live, 1, t0).is_none());
    }

    #[test]
    fn preview_matches_what_is_sent() {
        assert_eq!(preview_line(&on(), PresenceScreen::Browsing), "Competing in Lumen \u{b7} Browsing the library \u{b7} with elapsed time");
        let no_time = DiscordStatus { show_elapsed: false, ..on() };
        assert_eq!(preview_line(&no_time, PresenceScreen::Developing), "Competing in Lumen \u{b7} Developing a photo");
        let bare = DiscordStatus { show_activity: false, show_elapsed: false, ..on() };
        assert_eq!(preview_line(&bare, PresenceScreen::Slideshow), "Competing in Lumen");
    }

    #[test]
    fn screen_names_are_snake_case_on_the_page() {
        assert_eq!(serde_json::to_value(PresenceScreen::Slideshow).unwrap(), json!("slideshow"));
        assert_eq!(serde_json::from_value::<PresenceScreen>(json!("developing")).unwrap(), PresenceScreen::Developing);
        assert!(serde_json::from_value::<PresenceScreen>(json!("/albums/Secret")).is_err());
    }

    #[test]
    fn frame_roundtrip_and_layout() {
        let payload = json!({ "v": 1, "client_id": "123" });
        let bytes = encode_frame(OP_HANDSHAKE, &payload);
        let (op, len) = decode_header(bytes[..8].try_into().unwrap());
        assert_eq!((op, len as usize), (0, bytes.len() - 8));
        let (op, v) = read_frame(&mut io::Cursor::new(bytes)).unwrap();
        assert_eq!(op, OP_HANDSHAKE);
        assert_eq!(v, payload);
    }

    #[test]
    fn frame_header_is_little_endian() {
        let bytes = encode_frame(1, &json!({}));
        assert_eq!(&bytes[..8], &[1, 0, 0, 0, 2, 0, 0, 0]);
        assert_eq!(&bytes[8..], b"{}");
    }

    #[test]
    fn oversized_and_truncated_frames_error() {
        let mut huge = Vec::new();
        huge.extend_from_slice(&1u32.to_le_bytes());
        huge.extend_from_slice(&10_000_000u32.to_le_bytes());
        assert!(read_frame(&mut io::Cursor::new(huge)).is_err());
        let mut short = encode_frame(1, &json!({"a": 1}));
        short.truncate(10);
        assert!(read_frame(&mut io::Cursor::new(short)).is_err());
    }

    // ---- scheduler ----

    fn act(d: &str) -> Option<Activity> {
        Some(Activity { details: Some(d.into()), start: None })
    }

    #[test]
    fn scheduler_never_connects_when_nothing_is_wanted() {
        assert_eq!(plan(&SchedState::default(), &None, Instant::now()), Step::Wait);
    }

    #[test]
    fn scheduler_retries_connect_every_15s() {
        let t = Instant::now();
        let mut s = SchedState::default();
        assert_eq!(plan(&s, &act("a"), t), Step::Connect);
        s.next_connect = Some(t + RETRY_GAP);
        assert_eq!(plan(&s, &act("a"), t + Duration::from_secs(14)), Step::Wait);
        assert_eq!(plan(&s, &act("a"), t + Duration::from_secs(15)), Step::Connect);
    }

    #[test]
    fn scheduler_rate_limits_and_coalesces() {
        let t = Instant::now();
        let mut s = SchedState { connected: true, ..Default::default() };
        assert_eq!(plan(&s, &act("a"), t), Step::Send(act("a")));
        s.sent = Some(act("a"));
        s.last_send = Some(t);
        assert_eq!(plan(&s, &act("a"), t + Duration::from_secs(60)), Step::Wait);
        assert_eq!(plan(&s, &act("b"), t + Duration::from_secs(5)), Step::Wait);
        assert_eq!(plan(&s, &act("c"), t + Duration::from_secs(15)), Step::Send(act("c")));
    }

    #[test]
    fn scheduler_clears_immediately_then_disconnects() {
        let t = Instant::now();
        let mut s = SchedState { connected: true, sent: Some(act("a")), last_send: Some(t), next_connect: None };
        assert_eq!(plan(&s, &None, t + Duration::from_secs(1)), Step::Send(None));
        s.sent = Some(None);
        assert_eq!(plan(&s, &None, t + Duration::from_secs(2)), Step::Disconnect);
    }

    // ---- pipe paths ----

    #[test]
    fn pipe_candidates_cover_flatpak_and_snap() {
        let env = |k: &str| (k == "XDG_RUNTIME_DIR").then(|| "/run/user/1000".to_string());
        let paths: Vec<String> = unix_pipe_candidates(&env).iter().map(|p| p.to_string_lossy().replace('\\', "/")).collect();
        let pos = |s: &str| paths.iter().position(|p| p == s).unwrap_or_else(|| panic!("missing {s} in {paths:?}"));
        assert!(pos("/run/user/1000/discord-ipc-0") < pos("/run/user/1000/app/com.discordapp.Discord/discord-ipc-0"));
        assert!(pos("/run/user/1000/app/com.discordapp.Discord/discord-ipc-0") < pos("/run/user/1000/snap.discord/discord-ipc-0"));
        assert!(paths.contains(&"/run/user/1000/discord-ipc-9".to_string()));
        assert!(paths.last().unwrap().starts_with("/tmp/"));
        let mut dedup = paths.clone();
        dedup.sort();
        dedup.dedup();
        assert_eq!(dedup.len(), paths.len(), "no duplicates");
        assert!(!paths.iter().any(|p| p.contains("//")), "unset variables are skipped: {paths:?}");
    }

    // ---- a fake Discord ----

    #[derive(Default)]
    struct Fake {
        refuse: bool,
        die_after_sends: Option<usize>,
        ping_first: bool,
        opens: usize,
        frames: Vec<(u32, Value)>,
    }
    type Shared = Arc<Mutex<Fake>>;

    struct FakePipe {
        rx: mpsc::Receiver<Vec<u8>>,
        tx: mpsc::Sender<Vec<u8>>,
        buf: VecDeque<u8>,
    }
    impl Read for FakePipe {
        fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
            if self.buf.is_empty() {
                match self.rx.recv() {
                    Ok(b) => self.buf.extend(b),
                    Err(_) => return Ok(0),
                }
            }
            let n = out.len().min(self.buf.len());
            for o in out.iter_mut().take(n) {
                *o = self.buf.pop_front().unwrap();
            }
            Ok(n)
        }
    }
    impl Write for FakePipe {
        fn write(&mut self, b: &[u8]) -> io::Result<usize> {
            self.tx.send(b.to_vec()).map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "Discord went away"))?;
            Ok(b.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    struct FakeConnector(Shared);
    impl Connector for FakeConnector {
        fn open(&mut self, n: u8) -> io::Result<Box<dyn Pipe>> {
            let (die_after, ping_first) = {
                let mut f = self.0.lock().unwrap();
                f.opens += 1;
                if f.refuse || n != 0 {
                    return Err(io::Error::new(io::ErrorKind::NotFound, "no pipe"));
                }
                (f.die_after_sends, f.ping_first)
            };
            let (c2s_tx, c2s_rx) = mpsc::channel();
            let (s2c_tx, s2c_rx) = mpsc::channel();
            let shared = self.0.clone();
            std::thread::spawn(move || {
                let mut server = FakePipe { rx: c2s_rx, tx: s2c_tx, buf: VecDeque::new() };
                let mut sends = 0usize;
                while let Ok((op, v)) = read_frame(&mut server) {
                    shared.lock().unwrap().frames.push((op, v.clone()));
                    if op == OP_HANDSHAKE {
                        let _ = server.write_all(&encode_frame(OP_FRAME, &json!({ "cmd": "DISPATCH", "evt": "READY" })));
                    } else if v["cmd"] == "SET_ACTIVITY" {
                        if ping_first {
                            let _ = server.write_all(&encode_frame(OP_PING, &json!({ "p": 1 })));
                            if let Ok((op, v)) = read_frame(&mut server) {
                                shared.lock().unwrap().frames.push((op, v));
                            }
                        }
                        let _ = server.write_all(&encode_frame(OP_FRAME, &json!({ "cmd": "SET_ACTIVITY", "evt": null })));
                        sends += 1;
                        if die_after.is_some_and(|n| sends >= n) {
                            return;
                        }
                    }
                }
            });
            Ok(Box::new(FakePipe { rx: s2c_rx, tx: c2s_tx, buf: VecDeque::new() }))
        }
    }

    fn engine() -> (Engine, Shared) {
        let shared: Shared = Arc::default();
        (Engine::new("123".into(), Box::new(FakeConnector(shared.clone()))), shared)
    }
    fn sets(shared: &Shared) -> Vec<Value> {
        shared.lock().unwrap().frames.iter().filter(|(_, v)| v["cmd"] == "SET_ACTIVITY").map(|(_, v)| v.clone()).collect()
    }
    fn secs(t: Instant, s: u64) -> Instant {
        t + Duration::from_secs(s)
    }

    #[test]
    fn engine_connects_sends_type_5_and_clears() {
        let (mut e, shared) = engine();
        let t = Instant::now();
        e.tick(&act("a"), t);
        assert!(e.connected() && !e.showing());
        e.tick(&act("a"), t);
        assert!(e.showing());
        e.tick(&None, secs(t, 1));
        e.tick(&None, secs(t, 2));
        assert!(!e.connected(), "the pipe is dropped once cleared");
        let f = shared.lock().unwrap().frames.clone();
        assert_eq!((f[0].0, f[0].1["client_id"].clone()), (OP_HANDSHAKE, json!("123")));
        let s = sets(&shared);
        assert_eq!(s.len(), 2);
        assert_eq!(s[0]["args"]["activity"]["type"], 5);
        assert_eq!(s[0]["args"]["activity"]["details"], "a");
        assert!(s[1]["args"]["activity"].is_null());
    }

    #[test]
    fn engine_waits_quietly_while_discord_is_absent_then_connects() {
        let (mut e, shared) = engine();
        shared.lock().unwrap().refuse = true;
        let t = Instant::now();
        e.tick(&act("a"), t);
        let opens = shared.lock().unwrap().opens;
        assert_eq!(opens, 10, "all ten pipes tried");
        e.tick(&act("a"), secs(t, 14));
        assert_eq!(shared.lock().unwrap().opens, opens, "no retry inside 15 s");
        assert!(!e.connected());
        shared.lock().unwrap().refuse = false;
        e.tick(&act("a"), secs(t, 15));
        e.tick(&act("a"), secs(t, 15));
        assert!(e.connected() && e.showing());
        assert_eq!(sets(&shared).len(), 1);
    }

    #[test]
    fn engine_recovers_after_discord_goes_away() {
        let (mut e, shared) = engine();
        shared.lock().unwrap().die_after_sends = Some(1);
        let t = Instant::now();
        e.tick(&act("a"), t);
        e.tick(&act("a"), t);
        e.tick(&act("b"), secs(t, 15));
        assert!(!e.connected(), "the failed send drops the connection");
        shared.lock().unwrap().die_after_sends = None;
        e.tick(&act("b"), secs(t, 30));
        e.tick(&act("b"), secs(t, 30));
        assert!(e.connected() && e.showing());
        assert_eq!(sets(&shared).last().unwrap()["args"]["activity"]["details"], "b");
    }

    #[test]
    fn engine_obeys_the_rate_limit_through_the_wire() {
        let (mut e, shared) = engine();
        let t = Instant::now();
        e.tick(&act("a"), t);
        e.tick(&act("a"), t);
        e.tick(&act("b"), secs(t, 5));
        assert_eq!(sets(&shared).len(), 1);
        e.tick(&act("c"), secs(t, 15));
        let s = sets(&shared);
        assert_eq!(s.len(), 2);
        assert_eq!(s[1]["args"]["activity"]["details"], "c", "the latest value is sent");
    }

    #[test]
    fn engine_answers_ping_with_pong() {
        let (mut e, shared) = engine();
        shared.lock().unwrap().ping_first = true;
        let t = Instant::now();
        e.tick(&act("a"), t);
        e.tick(&act("a"), t);
        assert!(e.showing());
        assert!(shared.lock().unwrap().frames.iter().any(|(op, _)| *op == OP_PONG));
    }

    #[test]
    fn client_id_is_trimmed_blank_means_none_and_nothing_is_compiled_in() {
        assert_eq!(resolve_client_id(Some(" 123 ")), Some("123"));
        assert_eq!(resolve_client_id(Some("   ")), None);
        assert_eq!(resolve_client_id(None), None, "no default Application ID: a fork must not publish as Lumen");
    }

    #[test]
    fn state_reports_off_waiting_showing() {
        assert_eq!(state_of(false, false), DiscordState::Off);
        assert_eq!(state_of(false, true), DiscordState::Off, "switched off wins over a status not yet cleared");
        assert_eq!(state_of(true, false), DiscordState::Waiting);
        assert_eq!(state_of(true, true), DiscordState::Showing);
    }

    #[test]
    fn a_report_refreshes_the_idle_clock() {
        let t = Instant::now();
        let mut live = Live::default();
        assert_eq!(live.get(), None);
        live.touch(PresenceScreen::Browsing, t);
        live.touch(PresenceScreen::Developing, secs(t, 600));
        assert_eq!(live.get(), Some((PresenceScreen::Developing, secs(t, 600))));
    }

    #[test]
    fn engine_never_opens_a_pipe_when_nothing_is_wanted() {
        let (mut e, shared) = engine();
        let t = Instant::now();
        for i in 0..50 {
            e.tick(&None, secs(t, i * 3));
        }
        assert_eq!(shared.lock().unwrap().opens, 0);
        assert!(!e.connected());
    }
}
