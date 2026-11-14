//! The `photos://` scheme: every image and video byte reaches the webview through here.
//! Metadata never does; that is `invoke`'s job.

use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use http::{header, Request, Response, StatusCode};
use photos_jobs::Lane;
use tauri::{AppHandle, Manager, UriSchemeContext, UriSchemeResponder, Wry};

use crate::derivatives;
use crate::state::AppState;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Route {
    Thumb { id: i64, size: u32 },
    Preview { id: i64 },
    Original(i64),
    /// Best full-resolution image for the viewer (RAW → rendered JPEG, else the original).
    Full(i64),
    /// The original video (Range-capable), or with `?proxy=1` the H.264 proxy — which
    /// answers `202 Accepted` while it is still being transcoded.
    Video { id: i64, proxy: bool },
    Probe(u64),
    /// A small preview of something still on a card or phone, by the token
    /// `get_connected_device` or the device browser handed out (see `connected.rs`), at
    /// `connected::PREVIEW_EDGE`, or `LARGE_EDGE` with `?s=1024`.
    DeviceThumb { token: u64, edge: u32 },
    /// The editor's live preview, by the token `render_edit_preview` returned.
    EditPreview(u64),
    /// The audio track of slideshow `id`, so the editor's preview can play it in time.
    SlideshowAudio(i64),
    /// A track from the music browser, fetched through Rust so the webview never has to
    /// talk to Jamendo itself.
    MusicPreview(String),
    /// A track already saved in `<library>/music`, by file name.
    LibraryMusic(String),
    /// A face, cropped square for the People screen (Phase 7b).
    Face(i64),
    /// A pet, cropped square for the Pets screen (Round 4C).
    Pet(i64),
    /// An AI mask's stored weights for Develop's overlay (Phase 8f): `width × height` bytes,
    /// the size `ai_masks_list` gave. A hash names one raster forever, so it is cached.
    AiMask { id: i64, hash: String },
    // Round 22 lane B: a staged recovery of photo `id` (`<library>/recovered/<id>.<jpg|mp4>`).
    Recovered(i64),
    /// An installed app's icon by the number `external_apps::icon_url` gave it this session
    /// (Round 22 lane C); a number nobody holds gets the generic icon.
    AppIcon(usize),
    /// A plugin's `icon.png` (Round 23 C1): only what `Store::icon_path` names.
    PluginIcon(String),
    /// A file a plugin's manifest declares under `[[assets]]`, percent-encoded as the page wrote
    /// it: `Store::asset_path` decides, anything else is 404.
    PluginAsset { id: String, path: String },
    Bad,
}

/// `thumb/12?s=256` → `Thumb { 12, 256 }`. Sizes other than 256/1024 (thumb) and 2560 (preview) are `Bad`.
pub fn resolve_route(path: &str) -> Route {
    let (route, query) = path.split_once('?').unwrap_or((path, ""));
    let mut parts = route.splitn(2, '/');
    let kind = parts.next().unwrap_or("");
    let rest = parts.next().unwrap_or("");
    if kind == "music-preview" {
        let id = rest.split('?').next().unwrap_or("");
        return if id.is_empty() { Route::Bad } else { Route::MusicPreview(id.to_string()) };
    }
    if kind == "library-music" {
        let name = rest.split('?').next().unwrap_or("");
        // A file name, never a path: nothing may point outside the music folder.
        let safe = !name.is_empty() && !name.contains(['/', '\\']) && !name.contains("..");
        return if safe { Route::LibraryMusic(name.to_string()) } else { Route::Bad };
    }
    // Round 23 C1: ids and paths are not numbers, so these two answer before the numeric match.
    if kind == "plugin-icon" {
        let id = rest.split('?').next().unwrap_or("");
        return if id.is_empty() || id.contains('/') { Route::Bad } else { Route::PluginIcon(id.to_string()) };
    }
    if kind == "plugin-asset" {
        return match rest.split_once('/') {
            Some((id, path)) if !id.is_empty() && !path.is_empty() => Route::PluginAsset { id: id.to_string(), path: path.to_string() },
            _ => Route::Bad,
        };
    }
    if kind == "ai-mask" {
        let (id, hash) = rest.split_once('/').unwrap_or((rest, ""));
        let hex = hash.len() == 32 && hash.bytes().all(|b| b.is_ascii_hexdigit());
        return match id.parse::<i64>() {
            Ok(id) if hex => Route::AiMask { id, hash: hash.to_string() },
            _ => Route::Bad,
        };
    }
    let Some(id) = rest.parse::<i64>().ok() else { return Route::Bad };
    let size = query
        .split('&')
        .find_map(|kv| kv.strip_prefix("s="))
        .and_then(|v| v.parse::<u32>().ok());
    match (kind, size) {
        ("thumb", Some(s)) if s == 256 || s == 1024 => Route::Thumb { id, size: s },
        ("thumb", None) => Route::Thumb { id, size: 256 },
        ("preview", None | Some(2560)) => Route::Preview { id },
        ("original", _) => Route::Original(id),
        ("full", _) => Route::Full(id),
        ("video", _) => Route::Video { id, proxy: query.split('&').any(|kv| kv == "proxy=1") },
        ("probe", _) if cfg!(debug_assertions) && id >= 0 => Route::Probe(id as u64),
        ("device-thumb", None) if id >= 0 => Route::DeviceThumb { token: id as u64, edge: crate::connected::PREVIEW_EDGE },
        ("device-thumb", Some(1024)) if id >= 0 => Route::DeviceThumb { token: id as u64, edge: crate::connected::LARGE_EDGE },
        ("edit-preview", _) if id >= 0 => Route::EditPreview(id as u64),
        ("slideshow-audio", _) => Route::SlideshowAudio(id),
        ("face", _) => Route::Face(id),
        ("pet", _) => Route::Pet(id),
        // Round 22 lane B
        ("recovered", _) => Route::Recovered(id),
        ("app-icon", _) if id >= 0 => Route::AppIcon(id as usize),
        _ => Route::Bad,
    }
}

/// `origin` when it is one of Lumen's own pages (the bundled app on each OS, or the dev
/// server on localhost): the routes the page reads with `fetch` rather than `<img>` send it
/// back as `Access-Control-Allow-Origin`, since `photos.localhost` is another origin.
fn app_origin(origin: Option<&str>) -> Option<&str> {
    let o = origin?;
    let local_port = |rest: &str| rest.is_empty() || rest.strip_prefix(':').is_some_and(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()));
    let ok = matches!(o, "http://tauri.localhost" | "https://tauri.localhost" | "tauri://localhost")
        || o.strip_prefix("http://localhost").is_some_and(local_port)
        || o.strip_prefix("http://127.0.0.1").is_some_and(local_port);
    ok.then_some(o)
}

pub fn handle(ctx: UriSchemeContext<'_, Wry>, req: Request<Vec<u8>>, responder: UriSchemeResponder) {
    let app = ctx.app_handle().clone();
    let path = req.uri().path().trim_start_matches('/').to_string();
    let query = req.uri().query().unwrap_or("").to_string();
    let full = if query.is_empty() { path } else { format!("{path}?{query}") };
    let range = req
        .headers()
        .get(header::RANGE)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    let origin = app_origin(req.headers().get(header::ORIGIN).and_then(|v| v.to_str().ok())).map(str::to_owned);

    std::thread::spawn(move || {
        let response = match resolve_route(&full) {
            Route::Bad => plain(StatusCode::BAD_REQUEST, "bad route"),
            // Round 22 lane B: the next attempt is served at the same URL, so never kept.
            Route::Recovered(id) => {
                let state = app.state::<AppState>();
                match state.library().ok().and_then(|lib| crate::repair::staged_file(&lib.root, id)) {
                    _ if locked(&state, id) => plain(StatusCode::FORBIDDEN, "locked"),
                    Some(p) => {
                        let mut r = build_response(&p, range.as_deref());
                        r.headers_mut().insert(header::CACHE_CONTROL, header::HeaderValue::from_static("no-store"));
                        r
                    }
                    None => plain(StatusCode::NOT_FOUND, "not found"),
                }
            }
            // Rendered in memory for the editor; a new one per slider change, never cached.
            Route::EditPreview(token) => match app.state::<AppState>().edit_previews.get(token) {
                Some(bytes) => Response::builder()
                    .status(StatusCode::OK)
                    .header(header::CONTENT_TYPE, "image/jpeg")
                    .header(header::CACHE_CONTROL, "no-store")
                    .body(bytes.as_ref().clone())
                    .unwrap(),
                None => plain(StatusCode::NOT_FOUND, "preview expired"),
            },
            // Rendered in memory from the device, never written to disk.
            // Fetched once through Rust and kept in memory for a moment, like a device
            // preview: nothing from the music browser is written to disk until it is saved.
            Route::MusicPreview(id) => {
                let state = app.state::<AppState>();
                let bytes = state.music_previews.cached(&id).or_else(|| {
                    let url = state.music_previews.url_for(&id)?;
                    let fetched = tauri::async_runtime::block_on(crate::music::fetch(&state.http, &url)).ok()?;
                    let bytes = std::sync::Arc::new(fetched);
                    state.music_previews.store(&id, bytes.clone());
                    Some(bytes)
                });
                match bytes {
                    Some(bytes) => build_bytes(&bytes, "audio/mpeg", range.as_deref()),
                    None => plain(StatusCode::NOT_FOUND, "track unavailable"),
                }
            }
            Route::AiMask { id, hash } => {
                let state = app.state::<AppState>();
                let bytes = if locked(&state, id) {
                    Err(plain(StatusCode::FORBIDDEN, "locked"))
                } else {
                    state.library().ok().and_then(|lib| crate::facet::raster_bytes(&lib.db, id, &hash)).ok_or_else(|| plain(StatusCode::NOT_FOUND, "not found"))
                };
                let mut r = match bytes {
                    Ok(bytes) => Response::builder()
                        .status(StatusCode::OK)
                        .header(header::CONTENT_TYPE, "application/octet-stream")
                        .header(header::CACHE_CONTROL, "private, max-age=31536000, immutable")
                        .body(bytes)
                        .unwrap(),
                    Err(r) => r,
                };
                // The overlay `fetch`es these (a 404 too: it means *not on this photo*).
                if let Some(o) = origin.as_deref().and_then(|o| header::HeaderValue::from_str(o).ok()) {
                    r.headers_mut().insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, o);
                    r.headers_mut().insert(header::VARY, header::HeaderValue::from_static("Origin"));
                }
                r
            }
            Route::DeviceThumb { token, edge } => {
                use crate::connected::Thumb;
                let state = app.state::<AppState>();
                match state.device_previews.render(token, edge, state.ffmpeg().as_ref()) {
                    Thumb::Jpeg(bytes) => Response::builder()
                        .status(StatusCode::OK)
                        .header(header::CONTENT_TYPE, "image/jpeg")
                        .header(header::CACHE_CONTROL, "private, max-age=3600")
                        .body(bytes.as_ref().clone())
                        .unwrap(),
                    Thumb::Missing => plain(StatusCode::NOT_FOUND, "not found"),
                    Thumb::Gone => plain(StatusCode::GONE, "device gone"),
                    // Scrolled past before its turn: not an answer to remember.
                    Thumb::Dropped => Response::builder()
                        .status(StatusCode::SERVICE_UNAVAILABLE)
                        .header(header::CACHE_CONTROL, "no-store")
                        .body(b"dropped".to_vec())
                        .unwrap(),
                }
            }
            Route::AppIcon(n) => app_icon_response(&app, n),
            Route::PluginIcon(id) => plugin_file_response(app.state::<std::sync::Arc<crate::plugins::Plugins>>().serve_icon(&id), None),
            // Fonts are fetched across origins by the page's CSS, so its origin is let in.
            Route::PluginAsset { id, path } => plugin_file_response(app.state::<std::sync::Arc<crate::plugins::Plugins>>().serve_asset(&id, &path), origin.as_deref()),
            route => match (matches!(route, Route::Face(_) | Route::Pet(_)), resolve_file(&app, route)) {
                // A face's URL is its id, but its crop turns with the photo: asked again each
                // time rather than kept forever (a few KB from disk).
                (true, Ok(p)) => {
                    let mut r = build_response(&p, range.as_deref());
                    r.headers_mut().insert(header::CACHE_CONTROL, header::HeaderValue::from_static("no-cache"));
                    r
                }
                (_, Ok(p)) => build_response(&p, range.as_deref()),
                (_, Err(NotServed::NotFound)) => plain(StatusCode::NOT_FOUND, "not found"),
                (_, Err(NotServed::Locked)) => plain(StatusCode::FORBIDDEN, "locked"),
                (_, Err(NotServed::NotRenderable)) => plain(StatusCode::NOT_FOUND, "not renderable"),
                (_, Err(NotServed::Preparing)) => Response::builder()
                    .status(StatusCode::ACCEPTED)
                    .header(header::RETRY_AFTER, "2")
                    .header(header::CACHE_CONTROL, "no-store")
                    .body(b"preparing".to_vec())
                    .unwrap(),
                (_, Err(NotServed::StillRendering(small))) => busy_thumb_response(small.as_deref()),
                (_, Err(NotServed::Failed(msg))) => plain(StatusCode::INTERNAL_SERVER_ERROR, &msg),
            },
        };
        responder.respond(response);
    });
}

enum NotServed {
    NotFound,
    /// A photo in the Hidden folder while it is locked.
    Locked,
    NotRenderable,
    /// The proxy is being transcoded; ask again shortly.
    Preparing,
    /// A thumbnail outlasted the wait; the 256 px one, if it is on disk, stands in.
    StillRendering(Option<PathBuf>),
    Failed(String),
}

/// The photo a route serves bytes of, if any.
fn asset_of(route: &Route) -> Option<i64> {
    match *route {
        Route::Thumb { id, .. } | Route::Preview { id } | Route::Original(id) | Route::Full(id) | Route::Video { id, .. } => Some(id),
        _ => None,
    }
}

/// The Hidden folder's photos are not served while it is locked, whoever asks.
fn locked(state: &AppState, id: i64) -> bool {
    if state.vault_lock.is_unlocked() {
        state.vault_lock.touch();
        return false;
    }
    state.library().ok().is_some_and(|lib| lib.db.with_conn(|c| photos_core::vault::contains(c, id)).unwrap_or(false))
}

fn resolve_file(app: &AppHandle, route: Route) -> Result<PathBuf, NotServed> {
    let state = app.state::<AppState>();
    if asset_of(&route).is_some_and(|id| locked(&state, id)) {
        return Err(NotServed::Locked);
    }
    match route {
        Route::Original(id) => {
            let lib = state.library().map_err(|_| NotServed::NotFound)?;
            lib.original_path(id).map_err(|_| NotServed::NotFound)
        }
        Route::Full(id) => derivatives::ensure_full(app, id).map_err(|e| match e.code.as_str() {
            "not_found" => NotServed::NotFound,
            "not_renderable" => NotServed::NotRenderable,
            _ => NotServed::Failed(e.detail.unwrap_or(e.message)),
        }),
        Route::Video { id, proxy: false } => {
            let lib = state.library().map_err(|_| NotServed::NotFound)?;
            lib.original_path(id).map_err(|_| NotServed::NotFound)
        }
        Route::Video { id, proxy: true } => {
            let (path, ready) = derivatives::proxy_status(app, id).map_err(|_| NotServed::NotFound)?;
            if ready {
                return Ok(path);
            }
            derivatives::start_proxy(app, id).map_err(|e| match e.code.as_str() {
                "not_renderable" => NotServed::NotRenderable,
                _ => NotServed::Failed(e.detail.unwrap_or(e.message)),
            })?;
            Err(NotServed::Preparing)
        }
        Route::Thumb { id, size } => ensure(app, id, size),
        Route::Preview { id } => ensure(app, id, 2560),
        Route::Probe(id) => state.probes.lock().ok().and_then(|p| p.get(&id).cloned()).ok_or(NotServed::NotFound),
        Route::SlideshowAudio(id) => {
            let lib = state.library().map_err(|_| NotServed::NotFound)?;
            let (_, doc) = lib.db.with_conn(|c| photos_core::slideshow::get(c, id)).map_err(|_| NotServed::NotFound)?;
            crate::slideshows::audio_path(&lib.root, &doc).ok_or(NotServed::NotFound)
        }
        Route::LibraryMusic(name) => {
            let lib = state.library().map_err(|_| NotServed::NotFound)?;
            let path = crate::music::music_dir(&lib.root).join(name);
            path.is_file().then_some(path).ok_or(NotServed::NotFound)
        }
        Route::Face(id) => crate::faces::ensure_crop(app, id).map_err(|e| match e.code.as_str() {
            "not_found" => NotServed::NotFound,
            "not_renderable" => NotServed::NotRenderable,
            _ => NotServed::Failed(e.detail.unwrap_or(e.message)),
        }),
        Route::Pet(id) => crate::pets::ensure_crop(app, id).map_err(|e| match e.code.as_str() {
            "not_found" => NotServed::NotFound,
            "not_renderable" => NotServed::NotRenderable,
            _ => NotServed::Failed(e.detail.unwrap_or(e.message)),
        }),
        Route::DeviceThumb { .. } | Route::EditPreview(_) | Route::MusicPreview(_) | Route::AiMask { .. } | Route::Recovered(_) | Route::AppIcon(_) | Route::PluginIcon(_) | Route::PluginAsset { .. } | Route::Bad => Err(NotServed::NotFound),
    }
}

/// The neutral stand-in for an app whose icon cannot be read: an existing mark, no new art.
const GENERIC_APP_ICON: &[u8] = include_bytes!("../icons/64x64.png");

/// An app's 64 px icon, made on first ask. The URL carries the session, so a kept copy is only ever
/// asked for the same app; the generic icon is not kept, since the number may be given out later.
fn app_icon_response(app: &AppHandle, number: usize) -> Response<Vec<u8>> {
    let state = app.state::<AppState>();
    let settings = state.settings.read().expect("settings lock").external_apps.clone();
    let (bytes, cache) = app_icon_bytes(number, &settings, &state.paths.app_data.join("cache").join("app-icons"));
    Response::builder().status(StatusCode::OK).header(header::CONTENT_TYPE, "image/png").header(header::CACHE_CONTROL, cache).body(bytes).unwrap()
}

fn app_icon_bytes(number: usize, settings: &photos_core::settings::ExternalAppsSettings, cache_dir: &Path) -> (Vec<u8>, &'static str) {
    crate::external_apps::icon_target(number, settings)
        .and_then(|a| crate::external_apps::icon_png(&a, cache_dir))
        .and_then(|p| std::fs::read(p).ok())
        .map_or_else(|| (GENERIC_APP_ICON.to_vec(), "private, max-age=60"), |b| (b, "private, max-age=31536000, immutable"))
}

fn ensure(app: &AppHandle, id: i64, size: u32) -> Result<PathBuf, NotServed> {
    derivatives::ensure(app, id, size, Lane::Interactive).map_err(|e| match e.code.as_str() {
        "not_found" => NotServed::NotFound,
        "not_renderable" => NotServed::NotRenderable,
        "derivative_timeout" => {
            tracing::info!(asset = id, size, "thumbnail still rendering after the wait");
            NotServed::StillRendering(derivatives::small_thumb(app, id))
        }
        _ => NotServed::Failed(e.detail.unwrap_or(e.message)),
    })
}

/// A thumbnail that outlasted the wait is still rendering. The 256 px one stands in if it is on
/// disk, and neither answer may be kept: the URL is the 1024's, and files there are `immutable`.
fn busy_thumb_response(small: Option<&Path>) -> Response<Vec<u8>> {
    let mut r = match small {
        Some(p) => build_response(p, None),
        None => plain(StatusCode::SERVICE_UNAVAILABLE, "still rendering"),
    };
    r.headers_mut().insert(header::CACHE_CONTROL, header::HeaderValue::from_static("no-store"));
    r
}

/// The most a single answer reads. A response body is one buffer, and the webview asks for a
/// video with `bytes=0-`: answered whole, a 96 GB film was allocated at once and the app
/// aborted. A shorter 206 is a valid answer to an open range; the player asks on from there.
const MAX_PIECE: u64 = 8 * 1024 * 1024;
/// A video or audio file up to this size is still answered whole when asked without a range.
const MAX_WHOLE: u64 = 32 * 1024 * 1024;

/// Full (200) or partial (206) file response honouring `Range`, never more than
/// [`MAX_PIECE`] of a ranged request or of a large video or audio file.
pub fn build_response(path: &Path, range: Option<&str>) -> Response<Vec<u8>> {
    let mut file = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(_) => return plain(StatusCode::NOT_FOUND, "file not found"),
    };
    let total = match file.metadata() {
        Ok(m) => m.len(),
        Err(_) => return plain(StatusCode::INTERNAL_SERVER_ERROR, "stat failed"),
    };
    let mime = mime_guess::from_path(path).first_or_octet_stream();
    // An image is never asked for by range, and `<img>` cannot take part of one.
    let streamed = matches!(mime.type_(), mime_guess::mime::VIDEO | mime_guess::mime::AUDIO);
    let mime = mime.to_string();

    let (status, start, end) = match range {
        None if streamed && total > MAX_WHOLE => (StatusCode::PARTIAL_CONTENT, 0, MAX_PIECE - 1),
        None => (StatusCode::OK, 0, total.saturating_sub(1)),
        Some(h) => match photos_core::range::parse(h, total) {
            Some((s, e)) => (StatusCode::PARTIAL_CONTENT, s, e.min(s + MAX_PIECE - 1)),
            None => {
                return Response::builder()
                    .status(StatusCode::RANGE_NOT_SATISFIABLE)
                    .header(header::CONTENT_RANGE, format!("bytes */{total}"))
                    .body(Vec::new())
                    .unwrap();
            }
        },
    };

    let len = if total == 0 { 0 } else { end - start + 1 };
    let mut body = vec![0u8; len as usize];
    if len > 0 && (file.seek(SeekFrom::Start(start)).is_err() || file.read_exact(&mut body).is_err()) {
        return plain(StatusCode::INTERNAL_SERVER_ERROR, "read failed");
    }

    let mut b = Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, mime)
        .header(header::ACCEPT_RANGES, "bytes")
        .header(header::CONTENT_LENGTH, len.to_string())
        .header(header::CACHE_CONTROL, "private, max-age=31536000, immutable");
    if status == StatusCode::PARTIAL_CONTENT {
        b = b.header(header::CONTENT_RANGE, format!("bytes {start}-{end}/{total}"));
    }
    b.body(body).unwrap()
}

/// The same `Range` answer as [`build_response`], for bytes that never touch the disk —
/// a proxied music preview.
pub fn build_bytes(bytes: &[u8], mime: &str, range: Option<&str>) -> Response<Vec<u8>> {
    let total = bytes.len() as u64;
    let (status, start, end) = match range {
        None => (StatusCode::OK, 0, total.saturating_sub(1)),
        Some(h) => match photos_core::range::parse(h, total) {
            Some((s, e)) => (StatusCode::PARTIAL_CONTENT, s, e),
            None => {
                return Response::builder()
                    .status(StatusCode::RANGE_NOT_SATISFIABLE)
                    .header(header::CONTENT_RANGE, format!("bytes */{total}"))
                    .body(Vec::new())
                    .unwrap();
            }
        },
    };
    let len = if total == 0 { 0 } else { end - start + 1 };
    let body = bytes[start as usize..(start + len) as usize].to_vec();
    let mut b = Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, mime)
        .header(header::ACCEPT_RANGES, "bytes")
        .header(header::CONTENT_LENGTH, len.to_string())
        // A preview is fetched again after a restart; it is not a library file.
        .header(header::CACHE_CONTROL, "no-store");
    if status == StatusCode::PARTIAL_CONTENT {
        b = b.header(header::CONTENT_RANGE, format!("bytes {start}-{end}/{total}"));
    }
    b.body(body).unwrap()
}

/// A plugin's icon or declared asset (Round 23 C1), or 404 when the store named nothing. A dev
/// folder is read in place and changes under the same URL, so it is never kept; an installed
/// plugin's URL carries its version.
fn plugin_file_response(served: Option<crate::plugins::state::Served>, origin: Option<&str>) -> Response<Vec<u8>> {
    let Some(served) = served else { return plain(StatusCode::NOT_FOUND, "not found") };
    let mut r = build_response(&served.path, None);
    if r.status() != StatusCode::OK {
        return r;
    }
    if served.dev {
        r.headers_mut().insert(header::CACHE_CONTROL, header::HeaderValue::from_static("no-cache"));
    }
    if let Some(o) = app_origin(origin).and_then(|o| header::HeaderValue::from_str(o).ok()) {
        r.headers_mut().insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, o);
        r.headers_mut().insert(header::VARY, header::HeaderValue::from_static("Origin"));
    }
    r
}

fn plain(status: StatusCode, msg: &str) -> Response<Vec<u8>> {
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, "text/plain")
        .body(msg.as_bytes().to_vec())
        .unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> tempfile::NamedTempFile {
        let f = tempfile::Builder::new().suffix(".mp4").tempfile().unwrap();
        std::fs::write(f.path(), b"0123456789").unwrap();
        f
    }

    #[test]
    fn a_thumbnail_past_the_wait_stands_in_with_the_small_one_and_is_never_kept() {
        let f = fixture();
        let r = busy_thumb_response(Some(f.path()));
        assert_eq!(r.status(), 200);
        assert_eq!(r.headers()[header::CACHE_CONTROL], "no-store");
        let r = busy_thumb_response(None);
        assert_eq!(r.status(), 503);
        assert_eq!(r.headers()[header::CACHE_CONTROL], "no-store");
    }

    #[test]
    fn an_app_icon_route_parses_and_an_unknown_number_gets_the_generic_icon() {
        assert_eq!(resolve_route("app-icon/3?v=99"), Route::AppIcon(3));
        assert_eq!(resolve_route("app-icon/x"), Route::Bad);
        let dir = tempfile::tempdir().unwrap();
        let (bytes, cache) = app_icon_bytes(987_654, &Default::default(), dir.path());
        assert_eq!(bytes, GENERIC_APP_ICON);
        assert_eq!(cache, "private, max-age=60");
        assert!(image::load_from_memory(&bytes).is_ok());
    }

    #[test]
    fn routes_parse() {
        assert_eq!(resolve_route("face/42"), Route::Face(42));
        assert_eq!(resolve_route("pet/7"), Route::Pet(7));
        assert_eq!(resolve_route("thumb/12?s=256"), Route::Thumb { id: 12, size: 256 });
        assert_eq!(resolve_route("thumb/12?s=1024"), Route::Thumb { id: 12, size: 1024 });
        assert_eq!(resolve_route("thumb/12"), Route::Thumb { id: 12, size: 256 });
        assert_eq!(resolve_route("preview/3?s=2560"), Route::Preview { id: 3 });
        assert_eq!(resolve_route("thumb/1?s=999"), Route::Bad);
        assert_eq!(resolve_route("original/5"), Route::Original(5));
        assert_eq!(resolve_route("recovered/5"), Route::Recovered(5)); // Round 22 lane B
        assert_eq!(resolve_route("recovered/..%2Fx"), Route::Bad);
        assert_eq!(resolve_route("full/5"), Route::Full(5));
        assert_eq!(resolve_route("video/9"), Route::Video { id: 9, proxy: false });
        assert_eq!(resolve_route("device-thumb/7"), Route::DeviceThumb { token: 7, edge: 320 });
        assert_eq!(resolve_route("device-thumb/7?s=1024"), Route::DeviceThumb { token: 7, edge: 1024 });
        assert_eq!(resolve_route("device-thumb/7?s=99"), Route::Bad);
        assert_eq!(resolve_route("device-thumb/-1"), Route::Bad);
        assert_eq!(resolve_route("edit-preview/42"), Route::EditPreview(42));
        assert_eq!(resolve_route("slideshow-audio/8"), Route::SlideshowAudio(8));
        // The track's path rides along only to make a new track a new URL.
        assert_eq!(resolve_route("slideshow-audio/8?t=C%3A%2Fsong.mp3"), Route::SlideshowAudio(8));
        assert_eq!(resolve_route("music-preview/1886711"), Route::MusicPreview("1886711".into()));
        assert_eq!(resolve_route("music-preview/"), Route::Bad);
        assert_eq!(resolve_route("library-music/42.mp3"), Route::LibraryMusic("42.mp3".into()));
        assert_eq!(resolve_route("library-music/../secrets.txt"), Route::Bad);
        // `v` only busts the webview cache; it never changes what is served.
        assert_eq!(resolve_route("thumb/7?s=256&v=1789000000000"), Route::Thumb { id: 7, size: 256 });
        assert_eq!(resolve_route("preview/7?v=3"), Route::Preview { id: 7 });
        assert_eq!(resolve_route("video/9?proxy=1"), Route::Video { id: 9, proxy: true });
        let hash = "0123456789abcdef0123456789abcdef";
        assert_eq!(resolve_route(&format!("ai-mask/4/{hash}")), Route::AiMask { id: 4, hash: hash.into() });
        assert_eq!(resolve_route("ai-mask/4/../secrets"), Route::Bad);
        assert_eq!(resolve_route("ai-mask/4/0123"), Route::Bad);
        assert_eq!(resolve_route(&format!("ai-mask/x/{hash}")), Route::Bad);
        assert_eq!(resolve_route("ai-mask/4"), Route::Bad);
        assert_eq!(resolve_route("x/1"), Route::Bad);
        assert_eq!(resolve_route("thumb/abc"), Route::Bad);
    }

    #[test]
    fn only_the_apps_own_pages_may_read_ai_masks() {
        // The page is another origin than `photos.localhost`, so `fetch` needs the header.
        for ok in ["http://tauri.localhost", "https://tauri.localhost", "tauri://localhost", "http://localhost:1420", "http://localhost:1478", "http://127.0.0.1:1420"] {
            assert_eq!(app_origin(Some(ok)), Some(ok), "{ok}");
        }
        for no in ["https://example.com", "http://localhost.evil.com", "http://tauri.localhost.evil", "null", ""] {
            assert_eq!(app_origin(Some(no)), None, "{no}");
        }
        assert_eq!(app_origin(None), None);
    }

    #[test]
    fn full_response_advertises_ranges() {
        let f = fixture();
        let r = build_response(f.path(), None);
        assert_eq!(r.status(), 200);
        assert_eq!(r.headers()["accept-ranges"], "bytes");
        assert_eq!(r.headers()["content-length"], "10");
        assert_eq!(r.headers()["content-type"], "video/mp4");
        assert_eq!(r.body(), b"0123456789");
    }

    #[test]
    fn partial_response_is_206_with_content_range() {
        let f = fixture();
        let r = build_response(f.path(), Some("bytes=2-5"));
        assert_eq!(r.status(), 206);
        assert_eq!(r.headers()["content-range"], "bytes 2-5/10");
        assert_eq!(r.headers()["content-length"], "4");
        assert_eq!(r.body(), b"2345");
    }

    /// A sparse file as big as a long film: nothing is written, so it costs no disk.
    fn huge(len: u64) -> tempfile::NamedTempFile {
        let f = tempfile::Builder::new().suffix(".mov").tempfile().unwrap();
        f.as_file().set_len(len).unwrap();
        f
    }

    #[test]
    fn a_long_video_is_served_a_piece_at_a_time() {
        // The webview asks for `bytes=0-`; reading all of a 96 GB film into one buffer
        // aborted the app.
        let total = 3 * 1024 * 1024 * 1024u64;
        let f = huge(total);
        let r = build_response(f.path(), Some("bytes=0-"));
        assert_eq!(r.status(), 206);
        assert_eq!(r.body().len() as u64, MAX_PIECE);
        assert_eq!(r.headers()["content-range"], format!("bytes 0-{}/{total}", MAX_PIECE - 1));
        assert_eq!(r.headers()["content-length"], MAX_PIECE.to_string());

        // A seek far into the film is answered from there.
        let at = total - 1000;
        let r = build_response(f.path(), Some(&format!("bytes={at}-")));
        assert_eq!(r.headers()["content-range"], format!("bytes {at}-{}/{total}", total - 1));
        assert_eq!(r.body().len(), 1000);

        // Asked for without a range at all, it still starts with one piece.
        let r = build_response(f.path(), None);
        assert_eq!(r.status(), 206);
        assert_eq!(r.body().len() as u64, MAX_PIECE);
        assert_eq!(r.headers()["content-range"], format!("bytes 0-{}/{total}", MAX_PIECE - 1));
    }

    #[test]
    fn a_file_under_the_limit_is_served_whole() {
        let f = huge(MAX_WHOLE);
        let r = build_response(f.path(), None);
        assert_eq!(r.status(), 200);
        assert_eq!(r.body().len() as u64, MAX_WHOLE);
    }

    #[test]
    fn unsatisfiable_is_416_and_missing_is_404() {
        let f = fixture();
        assert_eq!(build_response(f.path(), Some("bytes=50-")).status(), 416);
        assert_eq!(build_response(Path::new("/nope/none.jpg"), None).status(), 404);
    }

    // Round 23 C1: the plugin routes.

    fn plugin_store() -> (tempfile::TempDir, std::sync::Arc<crate::plugins::Plugins>) {
        use crate::commands::plugins::{fixture, logic};
        let t = tempfile::tempdir().unwrap();
        let p = std::sync::Arc::new(crate::plugins::Plugins::load(t.path()));
        logic::set_developer_mode(&p, true).unwrap();
        let assets = "\n[[assets]]\npath = \"assets/fonts/Lora.woff2\"\nlicense = \"OFL-1.1\"\n";
        let icon = fixture::png(64, 64);
        let file = fixture::package_with(
            t.path(),
            "com.test.a",
            assets,
            &[("assets/fonts/Lora.woff2", b"wOF2"), ("icon.png", &icon), ("secret.txt", b"nope")],
        );
        logic::install(&p, &file).unwrap();
        (t, p)
    }

    #[test]
    fn plugin_routes_resolve() {
        assert!(matches!(resolve_route("plugin-icon/com.test.a?v=1"), Route::PluginIcon(id) if id == "com.test.a"));
        assert!(matches!(
            resolve_route("plugin-asset/com.test.a/assets/fonts/Lora.woff2?v=1"),
            Route::PluginAsset { id, path } if id == "com.test.a" && path == "assets/fonts/Lora.woff2"
        ));
        assert!(matches!(resolve_route("plugin-asset/com.test.a"), Route::Bad));
        assert!(matches!(resolve_route("plugin-asset/com.test.a/"), Route::Bad));
        assert!(matches!(resolve_route("plugin-icon/"), Route::Bad));
    }

    #[test]
    fn icon_served_as_png() {
        let (_t, p) = plugin_store();
        let r = plugin_file_response(p.serve_icon("com.test.a"), None);
        assert_eq!(r.status(), 200);
        assert_eq!(r.headers()[header::CONTENT_TYPE], "image/png");
        assert_eq!(plugin_file_response(p.serve_icon("com.none"), None).status(), 404);
    }

    #[test]
    fn asset_served_as_woff2() {
        let (_t, p) = plugin_store();
        let r = plugin_file_response(p.serve_asset("com.test.a", "assets/fonts/Lora.woff2"), Some("tauri://localhost"));
        assert_eq!(r.status(), 200);
        assert_eq!(r.headers()[header::CONTENT_TYPE], "font/woff2");
        assert!(r.headers()[header::CACHE_CONTROL].to_str().unwrap().contains("immutable"));
        // A font is fetched across origins, so the page's own origin is let in.
        assert_eq!(r.headers()[header::ACCESS_CONTROL_ALLOW_ORIGIN], "tauri://localhost");
        assert_eq!(r.body(), b"wOF2");
        // The same path as the webview may write it.
        assert_eq!(plugin_file_response(p.serve_asset("com.test.a", "assets%2Ffonts%2FLora.woff2"), None).status(), 200);
    }

    #[test]
    fn unlisted_file_404() {
        let (_t, p) = plugin_store();
        for f in ["plugin.toml", "plugin.wasm", "secret.txt", "icon.png", "SIGNATURE", ""] {
            assert_eq!(plugin_file_response(p.serve_asset("com.test.a", f), None).status(), 404, "{f}");
        }
    }

    #[test]
    fn encoded_traversal_404() {
        let (_t, p) = plugin_store();
        for f in [
            "assets%2F..%2F..%2Fplugin.toml",
            "%2e%2e%2fplugin.toml",
            "assets/%2e%2e/plugin.toml",
            "assets/fonts/../../plugin.toml",
            "assets%2Ffonts%2FLora.woff2%00",
            "assets%252Ffonts%252FLora.woff2",
            "assets%5Cfonts%5CLora.woff2",
            "%zz",
        ] {
            assert_eq!(plugin_file_response(p.serve_asset("com.test.a", f), None).status(), 404, "{f}");
        }
    }

    #[test]
    fn disabled_plugin_assets_404() {
        let (_t, p) = plugin_store();
        crate::commands::plugins::logic::set_enabled(&p, "com.test.a", false).unwrap();
        assert_eq!(plugin_file_response(p.serve_asset("com.test.a", "assets/fonts/Lora.woff2"), None).status(), 404);
        // The Settings list still shows an off plugin's icon.
        assert_eq!(plugin_file_response(p.serve_icon("com.test.a"), None).status(), 200);
    }
}
