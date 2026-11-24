//! AI assistants in Develop (Round 16): an opt-in MCP server on `127.0.0.1:<port>/mcp`
//! (Streamable HTTP, `rmcp`). Off, no port is open. Every request needs the bearer token from
//! Settings; any request with an `Origin` header is refused (a browser page cannot reach it).
//! It sees only Develop's filmstrip (`scope`) and writes drafts only (P2).

pub mod bridge;
pub mod connect;
pub mod fit;
pub mod looks;
pub mod render;
pub mod scope;
pub mod stdio;
pub mod tools;
pub mod write;

use std::sync::{Arc, Mutex};
use std::time::Instant;

use axum::extract::Request;
use axum::http::{header, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use base64::Engine as _;
use photos_core::Library;
use photos_media::Ffmpeg;
use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, GetPromptRequestParams, GetPromptResponse, GetPromptResult, ListPromptsResult, ListToolsResult,
    Implementation, PaginatedRequestParams, Prompt, PromptArgument, PromptMessage, ListResourcesResult, ReadResourceRequestParams, ReadResourceResponse, ReadResourceResult, Resource, ResourceContents, Role, ServerCapabilities, ServerConfig, Tool,
};
use rmcp::service::{NotificationContext, Peer, RequestContext};
use rmcp::transport::streamable_http_server::{session::local::LocalSessionManager, StreamableHttpServerConfig, StreamableHttpService};
use rmcp::{ErrorData, RoleServer, ServerHandler};
use serde_json::{json, Value};
use tauri::{AppHandle, Manager};
use tokio_util::sync::CancellationToken;

use crate::error::AppResult;
use crate::plugins::mcp_tools;
use crate::state::AppState;

/// The playbook, sent at `initialize` and as the `retouch` prompt.
pub const INSTRUCTIONS: &str = "You are editing photos in Lumen's Develop to make them look good and distinctive, not only correct. Work on one photo at a time; every photo gets its own looks, since each has its own composition. \
1) Read it: `view` and `analyze` (light, palette, faces, and the regions Lumen Facet found: sky, subject, background, people). Say in one sentence what the photo is about and what its light and colour do. \
2) Choose looks. If the user gave a brief, build that one look. Otherwise call `suggest_looks`: Lumen scores every look in its catalogue for this photo by colour theory. Build every look it offers (Natural and those scoring 60 or more, at most 4); drop an offered look only when it fights the content, and say why. \
3) Build each look from the clean photo (`reset` between looks): start from the look's patch, then adapt it to this photo — white balance and exposure, then tone and the curve, then colour (HSL, grading) — and shape the light locally with masks: AI masks for sky, subject, background and people when `analyze` lists regions, else linear and radial masks. A look must be visible at a glance in a before/after split; one pass of global sliders is not a look. \
4) Check with `view compare:\"split\"`, `view overlay` for masks, and `analyze`: faces never clip, highlights clip < 0.5 %, skin hue stays ~20–50°. Revise until it is better, not merely not wrong. \
5) Keep each look with `draft_variant`; the user flips through them as snapshots in Develop. When all are saved, `draft_variant op:\"use\"` puts the top-scoring one on the photo. \
End with one plain line per look: what it is and why it fits. You see only the photos in Develop's filmstrip (`list_photos`); every edit is a draft the user reviews. `mark` sets a photo's rating, colour label or flag, and only when the user asks you to cull or mark photos. Use `apply_to` only when asked to match a set.";

const MAX_BODY: usize = 1 << 20;

/// Where the tools get the library from: the running app, or a test's temp library.
pub trait Host: Send + Sync + 'static {
    fn library(&self) -> AppResult<Arc<Library>>;
    fn ffmpeg(&self) -> Option<Ffmpeg>;
    /// Ask the page to apply a change to the photo open in Develop (`bridge`).
    fn emit(&self, req: &bridge::McpRequest) -> AppResult<()>;
    /// Drafts changed behind the page's back: it refetches them.
    fn changed(&self);
    /// A tool started or ended, for Develop's AI chip (`mcp://status`).
    fn activity(&self, _: &McpActivity) {}
    /// `auto` with `model` ("standard", "ray", "prism") on `recipe`; `no_model` when it is off.
    fn auto(&self, id: i64, recipe: &photos_core::edits::Recipe, model: &str) -> AppResult<photos_core::edits::Recipe>;
    /// What Lumen Facet found on the photo, running it first when the current model hasn't
    /// looked; `None` when AI masks are switched off or the model is missing.
    fn ai_masks(&self, _id: i64) -> AppResult<Option<Vec<photos_core::ai_masks::AiMaskInfo>>> {
        Ok(None)
    }
    /// The looks enabled plugins add to the catalogue (Round 23 B2).
    fn plugin_looks(&self) -> Vec<looks::Look> {
        Vec::new()
    }
    /// Settings → AI assistants is on now. Plugin hosts (Round 23 A4) ask on every call.
    fn assistants_on(&self) -> bool;
    /// The plugin store, for plugin tools and prompts (Round 23 A4); `None` lists none.
    fn plugins(&self) -> Option<Arc<crate::plugins::Plugins>> {
        None
    }
}

pub struct AppHost(pub AppHandle);

impl Host for AppHost {
    fn library(&self) -> AppResult<Arc<Library>> {
        self.0.state::<AppState>().library()
    }
    fn ffmpeg(&self) -> Option<Ffmpeg> {
        self.0.state::<AppState>().ffmpeg()
    }
    fn emit(&self, req: &bridge::McpRequest) -> AppResult<()> {
        use tauri::Emitter;
        self.0.emit(bridge::EVENT, req).map_err(|e| crate::error::AppError { code: "develop_busy".into(), message: e.to_string(), detail: None })
    }
    fn changed(&self) {
        self.0.state::<AppState>().changes.changed(&self.0, true);
    }
    fn activity(&self, a: &McpActivity) {
        use tauri::Emitter;
        let _ = self.0.emit(STATUS_EVENT, a);
    }
    fn auto(&self, id: i64, recipe: &photos_core::edits::Recipe, model: &str) -> AppResult<photos_core::edits::Recipe> {
        let state = self.0.state::<AppState>();
        let smart = state.settings.read().map(|s| s.smart.clone()).unwrap_or_default();
        let app = &self.0;
        match model {
            "standard" | "ray" => {
                if model == "ray" && !crate::develop_auto::use_model(&smart) {
                    return Err(write::no_model(model));
                }
                let task = crate::ai_task::AiTask::start(app.clone(), None, "auto");
                let out = photos_jobs::interactive(|| crate::develop_auto::run(app, &state, id, recipe, &task));
                task.finish(out.is_ok());
                let r = out?;
                if model == "ray" && r.source != "model" {
                    return Err(write::no_model(model));
                }
                Ok(write::with_auto(recipe, &r))
            }
            "prism" => {
                if !(smart.enabled && smart.experimental.retouch) {
                    return Err(write::no_model(model));
                }
                let task = crate::ai_task::AiTask::start(app.clone(), None, "retouch");
                let out = photos_jobs::interactive(|| crate::develop_retouch::run(app, &state, id, recipe, &task));
                task.finish(out.is_ok());
                let r = out?;
                if !r.used_model {
                    return Err(write::no_model(model));
                }
                Ok(write::with_retouch(recipe, &r))
            }
            other => Err(tools::bad(format!("model must be standard, ray or prism, not {other}."))),
        }
    }
    fn ai_masks(&self, id: i64) -> AppResult<Option<Vec<photos_core::ai_masks::AiMaskInfo>>> {
        let state = self.0.state::<AppState>();
        if !crate::facet::masks_on(&state) {
            return Ok(None);
        }
        // The panel's own path: stored masks when Facet has looked, else one run, on the
        // interactive lane with the AI progress the New mask button shows.
        let task = crate::ai_task::AiTask::start(self.0.clone(), None, "masks");
        let out = photos_jobs::interactive(|| crate::facet::detect(&state, id, false, &task));
        task.finish(out.is_ok());
        let list = out?;
        Ok(list.available.then_some(list.masks))
    }
    fn plugin_looks(&self) -> Vec<looks::Look> {
        self.0.try_state::<std::sync::Arc<crate::plugins::Plugins>>().map(|p| crate::plugins::data::plugin_looks(&p.read())).unwrap_or_default()
    }
    fn assistants_on(&self) -> bool {
        self.0.state::<AppState>().settings.read().is_ok_and(|s| s.ai_assistants.enabled)
    }
    fn plugins(&self) -> Option<Arc<crate::plugins::Plugins>> {
        self.0.try_state::<Arc<crate::plugins::Plugins>>().map(|p| p.inner().clone())
    }
}

pub const STATUS_EVENT: &str = "mcp://status";

/// A tool call starting (`running`) or ending; the chip shows `client` is editing.
#[derive(Debug, Clone, serde::Serialize, ts_rs::TS)]
#[ts(export)]
pub struct McpActivity {
    pub client: Option<String>,
    pub tool: String,
    pub running: bool,
}

/// What the server and the app share, whether or not the server runs.
#[derive(Default)]
pub struct Shared {
    pub scope: scope::Scope,
    pub renders: render::Renders,
    pub bridge: bridge::Bridge,
    /// The last client's name (`clientInfo.name`), for Settings' status and the chip.
    pub client: Mutex<Option<String>>,
    /// Each plugin's checked tools, per (id, version); cleared when the plugins change.
    pub plugin_tools: mcp_tools::ToolCache,
    /// The host of the server that runs now. Plugin tools instances outlive a server start, so
    /// their `photos` / `drafts` hosts look it up here on every call (none: the server is off).
    pub host: mcp_tools::HostSlot,
    /// The connected sessions, told when the tools or prompts change (Round 23 A4).
    pub peers: Peers,
    /// The photos the tool call from the app that runs now was given (a grid's or the viewer's selection).
    pub tool_selection: crate::plugins::tool_selection::ToolSelection,
}

/// How long one list-changed notification may take to send to one session.
const NOTIFY_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// The sessions that finished `initialize`, so a change of the plugins' tools or prompts reaches
/// them as `notifications/{tools,prompts}/list_changed`.
#[derive(Default)]
pub struct Peers(Mutex<Vec<Peer<RoleServer>>>);

impl Peers {
    fn lock(&self) -> std::sync::MutexGuard<'_, Vec<Peer<RoleServer>>> {
        self.0.lock().unwrap_or_else(|p| p.into_inner())
    }

    fn add(&self, peer: Peer<RoleServer>) {
        let mut all = self.lock();
        all.retain(|p| !p.is_transport_closed());
        all.push(peer);
    }

    fn clear(&self) {
        self.lock().clear();
    }

    /// Open sessions, after dropping the ended ones (tests only).
    #[cfg(test)]
    pub fn len(&self) -> usize {
        let mut all = self.lock();
        all.retain(|p| !p.is_transport_closed());
        all.len()
    }

    /// Tells every open session that the tools and the prompts may have changed. The sends run
    /// on the async runtime, one task per session, with no lock held: a session that has gone
    /// away or stalls costs the others nothing.
    pub fn notify_list_changed(&self) {
        let live: Vec<Peer<RoleServer>> = {
            let mut all = self.lock();
            all.retain(|p| !p.is_transport_closed());
            all.clone()
        };
        for peer in live {
            tauri::async_runtime::spawn(async move {
                // A session that doesn't read its stream gives up after a while, never forever.
                let _ = tokio::time::timeout(NOTIFY_TIMEOUT, peer.notify_tool_list_changed()).await;
                let _ = tokio::time::timeout(NOTIFY_TIMEOUT, peer.notify_prompt_list_changed()).await;
            });
        }
    }
}

pub(crate) struct Running {
    pub(crate) port: u16,
    token: String,
    pub(crate) cancel: CancellationToken,
}

#[derive(Default)]
pub struct McpState {
    pub shared: Arc<Shared>,
    running: Mutex<Option<Running>>,
    /// Why the server is not running although it is on (e.g. the port is taken).
    pub error: Mutex<Option<String>>,
}

/// A new bearer token: 256 random bits as hex.
pub fn new_token() -> String {
    format!("{}{}", uuid::Uuid::new_v4().simple(), uuid::Uuid::new_v4().simple())
}

/// The name shown for the connected assistant. Stateless clients (protocol 2026-07-28) send it in
/// each request's `_meta`, or not at all; rmcp then fills in its own placeholder, which is skipped.
fn client_name(info: Option<Implementation>, known: Option<String>) -> String {
    info.map(|i| i.name)
        .filter(|n| !n.trim().is_empty() && *n != Implementation::default().name)
        .or(known)
        .unwrap_or_else(|| "An AI assistant".into())
}

/// Constant-time, so the token cannot be guessed a byte at a time.
fn same(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

async fn guard(token: Arc<str>, req: Request, next: Next) -> Response {
    if req.headers().contains_key(header::ORIGIN) {
        return (StatusCode::FORBIDDEN, "Browser requests are refused.").into_response();
    }
    let sent = req.headers().get(header::AUTHORIZATION).and_then(|v| v.to_str().ok()).and_then(|v| v.strip_prefix("Bearer "));
    match sent {
        Some(t) if same(t.as_bytes(), token.as_bytes()) => next.run(req).await,
        _ => (StatusCode::UNAUTHORIZED, [(header::WWW_AUTHENTICATE, "Bearer")], "A valid Lumen key is required.").into_response(),
    }
}

/// Start serving on `127.0.0.1:port` (0 picks a free port). Binding happens now, so a taken
/// port is an error here rather than a silent failure later.
pub(crate) fn start(host: Arc<dyn Host>, shared: Arc<Shared>, port: u16, token: String) -> std::io::Result<Running> {
    let listener = std::net::TcpListener::bind(("127.0.0.1", port))?;
    listener.set_nonblocking(true)?;
    let port = listener.local_addr()?.port();
    let cancel = CancellationToken::new();
    let config = StreamableHttpServerConfig::default()
        .enforce_origin_validation()
        .with_max_request_body_bytes(MAX_BODY)
        .with_cancellation_token(cancel.child_token());
    let service: StreamableHttpService<Lumen, LocalSessionManager> =
        StreamableHttpService::new(move || Ok(Lumen { host: host.clone(), shared: shared.clone() }), Default::default(), config);
    let key: Arc<str> = Arc::from(token.as_str());
    let router = axum::Router::new()
        .nest_service("/mcp", service)
        .layer(axum::middleware::from_fn(move |req: Request, next: Next| guard(key.clone(), req, next)));
    let stop = cancel.clone();
    tauri::async_runtime::spawn(async move {
        let served = match tokio::net::TcpListener::from_std(listener) {
            Ok(l) => axum::serve(l, router).with_graceful_shutdown(stop.cancelled_owned()).await,
            Err(e) => Err(e),
        };
        if let Err(e) = served {
            tracing::warn!("mcp server ended: {e}");
        }
    });
    tracing::info!(port, "mcp server started");
    Ok(Running { port, token, cancel })
}

/// What Settings shows about the server (Round 16).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, ts_rs::TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum McpServerState {
    Off,
    Waiting,
    Connected,
    PortInUse,
    Failed,
}

#[derive(Debug, Clone, serde::Serialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct McpStatus {
    pub state: McpServerState,
    /// The last client seen since the server started (`clientInfo.name`).
    pub client: Option<String>,
    /// Why it failed, when `state` is `failed`.
    pub message: Option<String>,
    /// This program, for Claude Desktop's `--mcp` command.
    pub exe: Option<String>,
}

impl McpState {
    pub fn status(&self, enabled: bool) -> McpStatus {
        let client = self.shared.client.lock().unwrap_or_else(|p| p.into_inner()).clone();
        let error = self.error.lock().unwrap_or_else(|p| p.into_inner()).clone();
        let running = self.running.lock().unwrap_or_else(|p| p.into_inner()).is_some();
        let (state, message) = match (enabled, running, error) {
            (false, _, _) => (McpServerState::Off, None),
            (true, true, _) if client.is_some() => (McpServerState::Connected, None),
            (true, true, _) => (McpServerState::Waiting, None),
            (true, false, Some(e)) if e == "port_in_use" => (McpServerState::PortInUse, None),
            (true, false, e) => (McpServerState::Failed, e),
        };
        let exe = std::env::current_exe().ok().map(|p| p.to_string_lossy().into_owned());
        McpStatus { state, client: if running { client } else { None }, message, exe }
    }

    fn stop(&self) {
        *self.shared.client.lock().unwrap_or_else(|p| p.into_inner()) = None;
        self.shared.host.clear();
        self.shared.peers.clear();
        if let Some(r) = self.running.lock().unwrap_or_else(|p| p.into_inner()).take() {
            r.cancel.cancel();
            tracing::info!(port = r.port, "mcp server stopped");
        }
    }

    /// Run the server as `settings` say: started, restarted on a new port or key, or stopped.
    fn follow(&self, host: impl FnOnce() -> Arc<dyn Host>, s: &photos_core::settings::AiAssistants) {
        {
            let running = self.running.lock().unwrap_or_else(|p| p.into_inner());
            let same_setup = running.as_ref().is_some_and(|r| r.port == s.port && r.token == s.token);
            if s.enabled && same_setup {
                return;
            }
        }
        self.stop();
        *self.error.lock().unwrap_or_else(|p| p.into_inner()) = None;
        if !s.enabled || s.token.is_empty() {
            return;
        }
        let host = host();
        self.shared.host.set(&host);
        match start(host, self.shared.clone(), s.port, s.token.clone()) {
            Ok(r) => *self.running.lock().unwrap_or_else(|p| p.into_inner()) = Some(r),
            Err(e) => {
                tracing::warn!(port = s.port, "mcp server could not start: {e}");
                *self.error.lock().unwrap_or_else(|p| p.into_inner()) = Some(if e.kind() == std::io::ErrorKind::AddrInUse { "port_in_use".into() } else { e.to_string() });
            }
        }
    }
}

/// At launch and after every settings change: start, restart or stop the server.
pub fn apply(app: &AppHandle) {
    let state = app.state::<AppState>();
    let s = state.settings.read().expect("settings lock").ai_assistants.clone();
    state.mcp.follow(|| Arc::new(AppHost(app.clone())), &s);
}

#[derive(Clone)]
pub struct Lumen {
    host: Arc<dyn Host>,
    shared: Arc<Shared>,
}

/// The prompts a client can pick: `edit_photo`, and `retouch` kept as its older name.
pub(crate) fn prompts() -> Vec<Prompt> {
    let brief = |name: &str| PromptArgument::new(name).with_description("The look the user wants, e.g. \"moody film\" (optional). Leave it out to be offered looks.").with_required(false);
    vec![
        Prompt::new("edit_photo", Some("Edit the photos in Lumen's Develop filmstrip into distinctive looks, scored for each photo."), Some(vec![brief("brief")])),
        Prompt::new("retouch", Some("Same as edit_photo (the older name); its argument is `style`."), Some(vec![brief("style")])),
    ]
}

/// The built-in prompts, then the plugins' while AI assistants are on (Round 23 A4).
pub(crate) fn listed_prompts(host: &dyn Host, shared: &Shared) -> Vec<Prompt> {
    let mut all = prompts();
    all.extend(mcp_tools::plugin_prompts(host, shared).into_iter().map(|p| Prompt::new(p.name, Some(p.description), None)));
    all
}

/// The text of the built-in or plugin prompt `name`; `None` for a prompt that isn't listed.
pub(crate) fn prompt_get(host: &dyn Host, shared: &Shared, name: &str, brief: Option<&str>) -> Option<String> {
    if mcp_tools::parse_mcp_name(name).is_none() {
        return prompt_text(name, brief);
    }
    mcp_tools::plugin_prompts(host, shared).into_iter().find(|p| p.name == name).map(|p| p.text)
}

/// The built-in tools, then the plugins' while AI assistants are on (Round 23 A4).
pub(crate) fn listed_tools(host: &Arc<dyn Host>, shared: &Arc<Shared>) -> Vec<Tool> {
    let schema = |v: Value| Arc::new(v.as_object().cloned().unwrap_or_default());
    let mut all: Vec<Tool> = tools::list().into_iter().map(|(name, description, v)| Tool::new(name, description, schema(v))).collect();
    all.extend(mcp_tools::plugin_tools(host, shared).into_iter().map(|t| Tool::new(t.name, t.description, schema(t.schema))));
    all
}

/// One `tools/call`, built-in or plugin: the same guard (AI assistants on) and the same log line.
pub(crate) fn run_call(host: &Arc<dyn Host>, shared: &Arc<Shared>, name: &str, args: &serde_json::Map<String, Value>) -> AppResult<tools::Output> {
    let id = args.get("id").and_then(Value::as_i64);
    let started = Instant::now();
    let out = if !host.assistants_on() {
        Err(mcp_tools::assistants_off())
    } else if mcp_tools::parse_mcp_name(name).is_some() {
        mcp_tools::call_plugin_tool(host, shared, name, args).map(tools::Output::Text)
    } else {
        tools::call(host.as_ref(), shared, name, args)
    };
    tracing::info!(tool = %name, id, ms = started.elapsed().as_millis() as u64, ok = out.is_ok(), "mcp call");
    out
}

/// The text of prompt `name` given its optional brief; `None` for a prompt that isn't one.
fn prompt_text(name: &str, brief: Option<&str>) -> Option<String> {
    if !matches!(name, "edit_photo" | "retouch") {
        return None;
    }
    Some(match brief.map(str::trim).filter(|s| !s.is_empty()) {
        Some(brief) => format!("{INSTRUCTIONS}\n\nThe brief: {brief}. Start with list_photos."),
        None => format!("{INSTRUCTIONS}\n\nStart with list_photos."),
    })
}

/// The resources an assistant can read: the looks catalogue.
fn resources() -> Vec<Resource> {
    vec![Resource::new(looks::LOOKS_URI, "looks")
        .with_description("Lumen's looks catalogue: each look's intent and its starting patch (the shape of get_recipe).")
        .with_mime_type("application/json")]
}

fn read_resource_at(host: &dyn Host, uri: &str) -> Result<ReadResourceResult, ErrorData> {
    if uri != looks::LOOKS_URI {
        return Err(ErrorData::resource_not_found(format!("There is no resource at {uri}; there is {}.", looks::LOOKS_URI), None));
    }
    // The built-ins and every enabled plugin's looks, as `suggest_looks` scores them.
    let text = looks::resource_of(&looks::with_plugins(host.plugin_looks())).to_string();
    Ok(ReadResourceResult::new(vec![ResourceContents::text(text, uri).with_mime_type("application/json")]))
}

impl ServerHandler for Lumen {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().enable_tool_list_changed().enable_prompts().enable_prompts_list_changed().enable_resources().build())
            .with_server_info(Implementation::new("Lumen", env!("CARGO_PKG_VERSION")))
            .with_instructions(INSTRUCTIONS)
    }

    async fn on_initialized(&self, ctx: NotificationContext<RoleServer>) {
        self.shared.peers.add(ctx.peer.clone());
        if let Some(info) = ctx.peer.peer_info() {
            let mut known = self.shared.client.lock().unwrap_or_else(|p| p.into_inner());
            let name = client_name(Some(info.client_info.clone()), known.take());
            tracing::info!(client = %name, "mcp client connected");
            *known = Some(name);
        }
    }

    async fn list_tools(&self, _: Option<PaginatedRequestParams>, _: RequestContext<RoleServer>) -> Result<ListToolsResult, ErrorData> {
        // A plugin's `tools()` runs wasm the first time: off the async runtime.
        let (host, shared) = (self.host.clone(), self.shared.clone());
        let tools = tokio::task::spawn_blocking(move || listed_tools(&host, &shared)).await.map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
        Ok(ListToolsResult::with_all_items(tools))
    }

    async fn call_tool(&self, req: CallToolRequestParams, ctx: RequestContext<RoleServer>) -> Result<CallToolResponse, ErrorData> {
        let client = {
            let mut known = self.shared.client.lock().unwrap_or_else(|p| p.into_inner());
            *known = Some(client_name(ctx.client_info(), known.take()));
            known.clone()
        };
        let name = req.name.to_string();
        let activity = |running| McpActivity { client: client.clone(), tool: name.clone(), running };
        self.host.activity(&activity(true));
        let args = req.arguments.unwrap_or_default();
        let (host, shared, tool) = (self.host.clone(), self.shared.clone(), name.clone());
        let out = tokio::task::spawn_blocking(move || run_call(&host, &shared, &tool, &args))
            .await
            .map_err(|e| ErrorData::internal_error(e.to_string(), None));
        self.host.activity(&activity(false));
        let result = match out? {
            Ok(tools::Output::Json(v)) => CallToolResult::success(vec![ContentBlock::text(v.to_string())]),
            Ok(tools::Output::Text(text)) => CallToolResult::success(vec![ContentBlock::text(text)]),
            Ok(tools::Output::Image(jpeg, line)) => {
                let b64 = base64::engine::general_purpose::STANDARD.encode(&jpeg.bytes);
                CallToolResult::success(vec![ContentBlock::image(b64, "image/jpeg"), ContentBlock::text(line)])
            }
            Err(e) => CallToolResult::error(vec![ContentBlock::text(json!({ "error": e.code, "message": e.message }).to_string())]),
        };
        Ok(result.into())
    }

    async fn list_resources(&self, _: Option<PaginatedRequestParams>, _: RequestContext<RoleServer>) -> Result<ListResourcesResult, ErrorData> {
        Ok(ListResourcesResult::with_all_items(resources()))
    }

    async fn read_resource(&self, req: ReadResourceRequestParams, _: RequestContext<RoleServer>) -> Result<ReadResourceResponse, ErrorData> {
        read_resource_at(self.host.as_ref(), &req.uri).map(Into::into)
    }

    async fn list_prompts(&self, _: Option<PaginatedRequestParams>, _: RequestContext<RoleServer>) -> Result<ListPromptsResult, ErrorData> {
        // Reading the plugins' prompts touches files and the store: off the async runtime.
        let (host, shared) = (self.host.clone(), self.shared.clone());
        let prompts = tokio::task::spawn_blocking(move || listed_prompts(host.as_ref(), &shared)).await.map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
        Ok(ListPromptsResult::with_all_items(prompts))
    }

    async fn get_prompt(&self, req: GetPromptRequestParams, _: RequestContext<RoleServer>) -> Result<GetPromptResponse, ErrorData> {
        let arg = |key: &str| req.arguments.as_ref().and_then(|a| a.get(key)).and_then(Value::as_str);
        let brief = arg("brief").or_else(|| arg("style")).map(str::to_string);
        let (host, shared, name) = (self.host.clone(), self.shared.clone(), req.name.clone());
        let text = tokio::task::spawn_blocking(move || prompt_get(host.as_ref(), &shared, &name, brief.as_deref())).await.map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
        let Some(text) = text else {
            return Err(ErrorData::invalid_params(format!("There is no prompt called {}.", req.name), None));
        };
        Ok(GetPromptResult::new(vec![PromptMessage::new(Role::User, ContentBlock::text(text))]).into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use photos_core::assets::{insert_assets, NewAsset};

    #[test]
    fn client_name_skips_rmcp_placeholder_and_keeps_the_last_real_one() {
        let named = |n: &str| Some(Implementation::new(n, "1"));
        assert_eq!(client_name(named("claude-code"), None), "claude-code");
        assert_eq!(client_name(Some(Implementation::default()), Some("codex".into())), "codex");
        assert_eq!(client_name(None, Some("codex".into())), "codex");
        assert_eq!(client_name(named(" "), None), "An AI assistant");
        assert_eq!(client_name(Some(Implementation::default()), None), "An AI assistant");
    }

    /// A temp library; the page, when `page` is set, answers each request through it.
    type Page = Arc<dyn Fn(&bridge::McpRequest) + Send + Sync>;
    struct TestHost(Arc<Library>, Option<Page>);

    impl Host for TestHost {
        fn library(&self) -> AppResult<Arc<Library>> {
            Ok(self.0.clone())
        }
        fn ffmpeg(&self) -> Option<Ffmpeg> {
            None
        }
        fn emit(&self, req: &bridge::McpRequest) -> AppResult<()> {
            if let Some(page) = &self.1 {
                page(req);
            }
            Ok(())
        }
        fn changed(&self) {}
        fn auto(&self, _: i64, recipe: &photos_core::edits::Recipe, model: &str) -> AppResult<photos_core::edits::Recipe> {
            if model != "standard" {
                return Err(write::no_model(model));
            }
            let mut d = recipe.develop.clone().unwrap_or_default();
            (d.basic.exposure, d.auto) = (0.5, true);
            Ok(photos_core::edits::Recipe { develop: Some(d), ..recipe.clone() })
        }
        /// Facet is "on" for a photo once a test has stored a detection for it.
        fn ai_masks(&self, id: i64) -> AppResult<Option<Vec<photos_core::ai_masks::AiMaskInfo>>> {
            use photos_core::ai_masks as am;
            Ok(self.0.db.with_conn(|c| -> photos_core::Result<_> { Ok(if am::scanned_by(c, id)?.is_some() { Some(am::list_for_asset(c, id)?) } else { None }) })?)
        }
        fn assistants_on(&self) -> bool {
            true
        }
    }

    #[test]
    fn the_mask_tool_adds_facets_masks_by_name_and_keeps_them_through_updates() {
        use photos_core::ai_masks::{self as am, AiMaskKind, NewAiMask};
        use photos_core::edits::MaskShape;
        let (_tmp, lib, id) = fixture();
        let host = TestHost(lib.clone(), None);
        let shared = Shared::default();
        shared.scope.set(vec![id], Some(id));
        let sky = json!({ "id": id, "name": "Sky", "shapes": [{ "kind": "ai", "target": "sky" }], "adjust": { "exposure": -0.3 } });
        let err = tools::call(&host, &shared, "mask", &args(sky.clone())).err().unwrap();
        assert_eq!(err.code, "no_model", "Facet off or missing: {}", err.message);

        let raster = |kind, instance| NewAiMask { kind, instance, score: 0.99, width: 4, height: 4, weights: vec![255; 16] };
        let found = [raster(AiMaskKind::Sky, 0), raster(AiMaskKind::Person, 0), raster(AiMaskKind::Person, 1)];
        lib.db.with_conn(|c| am::put_detection(c, id, "lumen-facet-test", &found)).unwrap();
        let hash = |kind, instance| lib.db.with_conn(|c| am::list_for_asset(c, id)).unwrap().into_iter().find(|m| m.kind == kind && m.instance == instance).unwrap().hash;

        let Ok(tools::Output::Json(out)) = tools::call(&host, &shared, "mask", &args(sky)) else { panic!() };
        assert_eq!(out["masks"][0]["ai"], json!(["sky"]), "{out}");
        let people = json!({ "id": id, "name": "Second person", "shapes": [{ "kind": "ai", "target": "person", "instance": 1 }, { "kind": "ai", "target": "person", "instance": 0, "add": false }], "adjust": { "shadows": 0.2 } });
        assert!(tools::call(&host, &shared, "mask", &args(people)).is_ok());
        let none = json!({ "id": id, "shapes": [{ "kind": "ai", "target": "subject" }] });
        let err = tools::call(&host, &shared, "mask", &args(none)).err().unwrap();
        assert_eq!(err.code, "invalid_arguments");
        assert!(err.message.contains("sky, person 0, person 1"), "{}", err.message);

        // An update that leaves shapes out keeps the AI shapes exactly, crop or not.
        assert!(tools::call(&host, &shared, "edit", &args(json!({ "id": id, "patch": { "crop": { "x": 0.1, "y": 0.1, "w": 0.5, "h": 0.5 } }, "label": "crop" }))).is_ok());
        assert!(tools::call(&host, &shared, "mask", &args(json!({ "id": id, "op": "update", "mask_id": 1, "adjust": { "exposure": -0.5 } })) ).is_ok());
        let masks = tools::recipe(&lib, id).unwrap().develop.unwrap().masks;
        assert_eq!(masks[0].shapes, [MaskShape::Ai { add: true, target: AiMaskKind::Sky, instance: 0, raster: hash(AiMaskKind::Sky, 0) }]);
        assert_eq!(masks[0].adjust.exposure, -0.5);
        assert_eq!(
            masks[1].shapes,
            [
                MaskShape::Ai { add: true, target: AiMaskKind::Person, instance: 1, raster: hash(AiMaskKind::Person, 1) },
                MaskShape::Ai { add: false, target: AiMaskKind::Person, instance: 0, raster: hash(AiMaskKind::Person, 0) },
            ]
        );
        assert_eq!(history(&lib, id), ["AI: mask Sky", "AI: mask Second person", "AI: crop", "AI: mask mask"]);
    }

    fn args(v: Value) -> serde_json::Map<String, Value> {
        v.as_object().cloned().unwrap()
    }

    fn history(lib: &Library, id: i64) -> Vec<String> {
        lib.db.with_conn(|c| photos_core::develop_history::list_history(c, id)).unwrap().into_iter().map(|s| s.label).collect()
    }

    #[test]
    fn edits_land_in_the_draft_with_ai_steps_and_undo_only_takes_back_its_own() {
        let (_tmp, lib, id) = fixture();
        let host = TestHost(lib.clone(), None);
        let shared = Shared::default();
        shared.scope.set(vec![id], Some(id));
        let patch = json!({ "id": id, "patch": { "develop": { "basic": { "exposure": 9, "shadows": 0.2 } } }, "label": "brighter" });
        let Ok(tools::Output::Json(out)) = tools::call(&host, &shared, "edit", &args(patch)) else { panic!() };
        assert_eq!(out["recipe"], json!({ "develop": { "basic": { "exposure": 5.0, "shadows": 0.2 } } }), "clamped to the slider");
        assert!(out["luma_mean"].as_f64().is_some() && out["clip"].is_object(), "{out}");
        let err = tools::call(&host, &shared, "edit", &args(json!({ "id": id, "patch": { "develop": { "masks": [] } }, "label": "x" }))).err().unwrap();
        assert_eq!(err.code, "invalid_patch");

        let mask = json!({ "id": id, "name": "face", "shapes": [{ "kind": "radial", "center": [0.5, 0.5], "radius": [0.1, 0.2] }], "adjust": { "exposure": 0.3 } });
        let Ok(tools::Output::Json(masks)) = tools::call(&host, &shared, "mask", &args(mask)) else { panic!() };
        assert_eq!(masks["masks"][0]["adjust"], json!({ "exposure": 0.3 }));
        let r = tools::recipe(&lib, id).unwrap();
        let photos_core::edits::MaskShape::Radial { center, radius, .. } = r.develop.as_ref().unwrap().masks[0].shapes[0].clone() else { panic!() };
        assert!((center[0] - 0.5).abs() < 1e-3 && (radius[0] - 0.1).abs() < 1e-3 && (radius[1] - 0.2).abs() < 1e-3, "{center:?} {radius:?}");
        // Every local tool, merged into the mask and read back in the photo's shape.
        let more = json!({ "id": id, "op": "update", "mask_id": 1, "adjust": { "curve": { "master": [[0.0, 0.1], [1.0, 0.9]] }, "hsl": { "blue": { "s": -0.4 } }, "sharpen": { "radius": 9 }, "glow": { "amount": 0.3 } } });
        let Ok(tools::Output::Json(masks)) = tools::call(&host, &shared, "mask", &args(more)) else { panic!() };
        let a = &masks["masks"][0]["adjust"];
        assert_eq!((&a["exposure"], &a["hsl"], &a["glow"]["amount"], &a["sharpen"]["radius"]), (&json!(0.3), &json!({ "blue": { "s": -0.4 } }), &json!(0.3), &json!(3.0)), "{a}");
        let m = tools::recipe(&lib, id).unwrap().develop.unwrap().masks.remove(0);
        assert_eq!((m.adjust.curve.master.len(), m.adjust.glow.amount), (2, 0.3));
        let unknown = json!({ "id": id, "op": "update", "mask_id": 1, "adjust": { "warmth": 1 } });
        assert_eq!(tools::call(&host, &shared, "mask", &args(unknown)).err().unwrap().code, "invalid_patch");
        let brush = json!({ "id": id, "shapes": [{ "kind": "brush" }] });
        assert_eq!(tools::call(&host, &shared, "mask", &args(brush)).err().unwrap().code, "invalid_arguments");

        assert_eq!(tools::call(&host, &shared, "auto", &args(json!({ "id": id, "model": "prism" }))).err().unwrap().code, "no_model");
        assert!(tools::call(&host, &shared, "auto", &args(json!({ "id": id }))).is_ok());
        assert!(tools::call(&host, &shared, "reset", &args(json!({ "id": id, "panels": ["masks"] }))).is_ok());
        assert_eq!(history(&lib, id), ["AI: brighter", "AI: mask face", "AI: mask mask", "AI: Auto (standard)", "AI: Reset"]);
        assert!(tools::recipe(&lib, id).unwrap().develop.unwrap().masks.is_empty());

        assert!(tools::call(&host, &shared, "undo", &args(json!({ "id": id }))).is_ok());
        assert_eq!(tools::recipe(&lib, id).unwrap().develop.unwrap().masks.len(), 1, "back to before the reset");
        lib.db.with_conn(|c| photos_core::develop_history::push_step(c, id, &photos_core::edits::Recipe::identity(), "Exposure +0.30", 1)).unwrap();
        assert_eq!(tools::call(&host, &shared, "undo", &args(json!({ "id": id }))).err().unwrap().code, "last_step_not_ai");
    }

    #[test]
    fn the_open_photo_changes_through_the_page_and_silence_is_busy() {
        let (_tmp, lib, id) = fixture();
        let shared = Arc::new(Shared::default());
        shared.scope.set(vec![id], Some(id));
        shared.scope.set_mounted(true);
        let (s, l) = (shared.clone(), lib.clone());
        // The page: runs the op on its draft (identity here) and answers, as mcp-bridge.ts does.
        let page = move |req: &bridge::McpRequest| {
            let host = TestHost(l.clone(), None);
            let r = write::transform(&host, &l, req.asset_id, &photos_core::edits::Recipe::identity(), &req.op).unwrap();
            assert_eq!(req.label, "AI: warm");
            s.bridge.reply(req.req, bridge::McpReply { recipe: Some(r), error: None, message: None });
        };
        let host = TestHost(lib.clone(), Some(Arc::new(page)));
        let edit = args(json!({ "id": id, "patch": { "develop": { "basic": { "temperature": 10 } } }, "label": "warm" }));
        let Ok(tools::Output::Json(out)) = tools::call(&host, &shared, "edit", &edit) else { panic!() };
        assert_eq!(out["recipe"], json!({ "develop": { "basic": { "temperature": 10.0 } } }));
        assert!(history(&lib, id).is_empty(), "the page records the step, not Rust");
    }

    /// A temp library with one 600×400 JPEG.
    fn fixture() -> (tempfile::TempDir, Arc<Library>, i64) {
        fixture_of(|x, y| image::Rgb([(x % 256) as u8, (y % 256) as u8, 128]))
    }

    /// The same, painted by `paint(x, y)`.
    fn fixture_of(paint: impl Fn(u32, u32) -> image::Rgb<u8>) -> (tempfile::TempDir, Arc<Library>, i64) {
        let tmp = tempfile::tempdir().unwrap();
        let lib = Library::create(&tmp.path().join("lib")).unwrap();
        let img = image::RgbImage::from_fn(600, 400, paint);
        let bytes = photos_media::encode_jpeg_bytes(&image::DynamicImage::ImageRgb8(img), 90).unwrap();
        std::fs::write(lib.originals_dir().join("a.jpg"), bytes).unwrap();
        let id = lib
            .db
            .with_conn(|c| {
                insert_assets(
                    c,
                    &[NewAsset {
                        content_hash: "cafe".into(),
                        relative_path: "a.jpg".into(),
                        filename: "a.jpg".into(),
                        extension: "jpg".into(),
                        width: Some(600),
                        height: Some(400),
                        orientation: 1,
                        ..Default::default()
                    }],
                )
            })
            .unwrap()[0];
        (tmp, Arc::new(lib), id)
    }

    #[test]
    fn mark_sets_rating_label_and_flag_on_strip_photos_only() {
        let (_tmp, lib, id) = fixture();
        let host = TestHost(lib.clone(), None);
        let shared = Shared::default();
        let args = |v: Value| v.as_object().cloned().unwrap();
        let err = tools::call(&host, &shared, "mark", &args(json!({ "id": id, "rating": 4 }))).err().unwrap();
        assert_eq!(err.code, "not_in_develop");
        shared.scope.set(vec![id], Some(id));
        let Ok(tools::Output::Json(out)) = tools::call(&host, &shared, "mark", &args(json!({ "id": id, "rating": 4, "label": "green", "flag": "pick" }))) else { panic!() };
        assert_eq!(out, json!({ "id": id, "rating": 4, "label": "green", "flag": "pick" }));
        let Ok(tools::Output::Json(listed)) = tools::call(&host, &shared, "list_photos", &args(json!({}))) else { panic!() };
        assert_eq!((listed["photos"][0]["rating"].clone(), listed["photos"][0]["label"].clone(), listed["photos"][0]["flag"].clone()), (json!(4), json!("green"), json!("pick")));
        // Fields left out stay; a bad value is refused and changes nothing.
        let Ok(tools::Output::Json(out)) = tools::call(&host, &shared, "mark", &args(json!({ "id": id, "flag": "reject" }))) else { panic!() };
        assert_eq!(out, json!({ "id": id, "rating": 4, "label": "green", "flag": "reject" }));
        assert!(tools::call(&host, &shared, "mark", &args(json!({ "id": id, "rating": 9, "label": "pink" }))).is_err());
        let Ok(tools::Output::Json(out)) = tools::call(&host, &shared, "mark", &args(json!({ "id": id, "rating": 0, "label": "none", "flag": "none" }))) else { panic!() };
        assert_eq!(out, json!({ "id": id, "rating": 0, "label": "none", "flag": "none" }));
    }

    #[test]
    fn tools_refuse_photos_outside_the_strip_and_view_what_is_in_it() {
        let (_tmp, lib, id) = fixture();
        let host = TestHost(lib, None);
        let shared = Shared::default();
        let args = |v: Value| v.as_object().cloned().unwrap();
        let err = tools::call(&host, &shared, "get_recipe", &args(json!({ "id": id }))).err().unwrap();
        assert_eq!(err.code, "not_in_develop");
        let Ok(tools::Output::Json(listed)) = tools::call(&host, &shared, "list_photos", &args(json!({}))) else { panic!() };
        assert_eq!(listed["photos"], json!([]), "an empty strip shows nothing");

        shared.scope.set(vec![id], Some(id));
        let Ok(tools::Output::Json(listed)) = tools::call(&host, &shared, "list_photos", &args(json!({}))) else { panic!() };
        assert_eq!(listed["photos"][0], json!({ "id": id, "name": "a.jpg", "w": 600, "h": 400, "raw": false, "edited": false, "open": true, "rating": 0, "label": "none", "flag": "none" }));
        let Ok(tools::Output::Json(summary)) = tools::call(&host, &shared, "get_recipe", &args(json!({ "id": id }))) else { panic!() };
        assert_eq!(summary, json!({}));
        let Ok(tools::Output::Image(jpeg, _)) = tools::call(&host, &shared, "view", &args(json!({ "id": id, "size": 300, "compare": "split" }))) else { panic!() };
        assert_eq!((jpeg.width, jpeg.height), (300, 200));
        let decoded = image::load_from_memory(&jpeg.bytes).unwrap();
        assert_eq!((decoded.width(), decoded.height()), (300, 200));
        let Ok(tools::Output::Image(part, _)) = tools::call(&host, &shared, "view", &args(json!({ "id": id, "region": { "x": 0.0, "y": 0.0, "w": 0.5, "h": 0.5 } }))) else { panic!() };
        assert_eq!((part.width, part.height), (300, 200), "a region is at 100 %");
        let Ok(tools::Output::Json(a)) = tools::call(&host, &shared, "analyze", &args(json!({ "id": id }))) else { panic!() };
        assert_eq!((a["size"].as_str(), a["raw"].as_bool()), (Some("600×400"), Some(false)));
        assert!(a["luma"]["mean"].as_f64().is_some(), "{a}");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn the_server_wants_the_key_and_refuses_browsers() {
        let (_tmp, lib, _) = fixture();
        let running = start(Arc::new(TestHost(lib, None)), Arc::new(Shared::default()), 0, "secret".into()).unwrap();
        let url = format!("http://127.0.0.1:{}/mcp", running.port);
        let init = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"test","version":"1"}}}"#;
        let client = reqwest::Client::new();
        let post = |auth: Option<&str>, origin: Option<&str>| {
            let mut r = client.post(&url).header("content-type", "application/json").header("accept", "application/json, text/event-stream").body(init);
            if let Some(a) = auth {
                r = r.header("authorization", a);
            }
            if let Some(o) = origin {
                r = r.header("origin", o);
            }
            r.send()
        };
        assert_eq!(post(None, None).await.unwrap().status(), 401);
        assert_eq!(post(Some("Bearer wrong"), None).await.unwrap().status(), 401);
        assert_eq!(post(Some("Bearer secret"), Some("http://evil.example")).await.unwrap().status(), 403);
        let ok = post(Some("Bearer secret"), None).await.unwrap();
        assert_eq!(ok.status(), 200);
        assert!(ok.text().await.unwrap().contains("Lumen's Develop"), "initialize carries the playbook");
        running.cancel.cancel();
    }

    #[test]
    fn status_follows_the_setting_the_port_and_the_client() {
        let (_tmp, lib, _) = fixture();
        let host = || -> Arc<dyn Host> { Arc::new(TestHost(lib.clone(), None)) };
        let mcp = McpState::default();
        assert_eq!(mcp.status(false).state, McpServerState::Off);
        let taken = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let mut s = photos_core::settings::AiAssistants { enabled: true, port: taken.local_addr().unwrap().port(), token: "k".into(), ..Default::default() };
        mcp.follow(host, &s);
        assert_eq!(mcp.status(true).state, McpServerState::PortInUse);
        s.port = 0;
        mcp.follow(host, &s);
        assert_eq!(mcp.status(true).state, McpServerState::Waiting);
        *mcp.shared.client.lock().unwrap() = Some("claude-code".into());
        let st = mcp.status(true);
        assert_eq!((st.state, st.client.as_deref()), (McpServerState::Connected, Some("claude-code")));
        s.enabled = false;
        mcp.follow(host, &s);
        let st = mcp.status(false);
        assert_eq!((st.state, st.client), (McpServerState::Off, None), "stopping forgets the client");
    }

    #[test]
    fn the_stdio_bridge_pipes_a_session_through_the_server() {
        let (_tmp, lib, _) = fixture();
        let running = start(Arc::new(TestHost(lib, None)), Arc::new(Shared::default()), 0, "secret".into()).unwrap();
        let url = format!("http://127.0.0.1:{}/mcp", running.port);
        let input = [
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"desk","version":"1"}}}"#,
            r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
            r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#,
        ]
        .join("\n");
        let mut out = Vec::new();
        stdio::bridge(input.as_bytes(), &mut out, || Some(stdio::Target { url: url.clone(), token: "secret".into() })).unwrap();
        let lines: Vec<Value> = String::from_utf8(out).unwrap().lines().map(|l| serde_json::from_str(l).unwrap()).collect();
        assert_eq!(lines.len(), 2, "{lines:?}");
        assert!(lines[0]["result"]["instructions"].as_str().unwrap().contains("Lumen's Develop"));
        assert!(lines[1]["result"]["tools"].as_array().unwrap().iter().any(|t| t["name"] == "list_photos"), "the session id was kept");

        let mut out = Vec::new();
        stdio::bridge(input.as_bytes(), &mut out, || Some(stdio::Target { url: url.clone(), token: "old".into() })).unwrap();
        let first: Value = serde_json::from_str(String::from_utf8(out).unwrap().lines().next().unwrap()).unwrap();
        assert!(first["error"]["message"].as_str().unwrap().contains("refused the key"));
        running.cancel.cancel();
    }

    #[test]
    fn tokens_are_long_and_compared_whole() {
        let t = new_token();
        assert_eq!(t.len(), 64);
        assert_ne!(t, new_token());
        assert!(same(b"abc", b"abc") && !same(b"abc", b"abd") && !same(b"abc", b"ab"));
    }

    #[test]
    fn the_looks_resource_is_listed_and_read() {
        let (_tmp, lib, _) = fixture();
        let host = TestHost(lib.clone(), None);
        let lumen = Lumen { host: Arc::new(TestHost(lib, None)), shared: Arc::new(Shared::default()) };
        assert!(lumen.get_info().capabilities.resources.is_some());
        assert_eq!(resources().iter().map(|r| r.uri.as_str()).collect::<Vec<_>>(), [looks::LOOKS_URI]);
        let read = read_resource_at(&host, looks::LOOKS_URI).unwrap();
        let rmcp::model::ResourceContents::TextResourceContents { text, mime_type, .. } = &read.contents[0] else { panic!() };
        assert_eq!(mime_type.as_deref(), Some("application/json"));
        let v: Value = serde_json::from_str(text).unwrap();
        assert_eq!(v["looks"].as_array().unwrap().len(), 12);
        assert!(read_resource_at(&host, "lumen://nothing").is_err());
    }

    /// [`TestHost`] whose plugins add one look (Round 23 B2).
    struct LooksHost(TestHost);

    impl Host for LooksHost {
        fn library(&self) -> AppResult<Arc<Library>> {
            self.0.library()
        }
        fn ffmpeg(&self) -> Option<Ffmpeg> {
            None
        }
        fn emit(&self, req: &bridge::McpRequest) -> AppResult<()> {
            self.0.emit(req)
        }
        fn changed(&self) {}
        fn auto(&self, id: i64, recipe: &photos_core::edits::Recipe, model: &str) -> AppResult<photos_core::edits::Recipe> {
            self.0.auto(id, recipe, model)
        }
        fn plugin_looks(&self) -> Vec<looks::Look> {
            let base = looks::catalogue()[1].clone();
            vec![looks::Look { id: "plugin:com.test.film/warm.json".into(), name: "Warm film".into(), safe: false, ..base }]
        }
        fn assistants_on(&self) -> bool {
            true
        }
    }

    #[test]
    fn the_looks_resource_has_the_plugins_looks() {
        let (_tmp, lib, _) = fixture();
        let read = read_resource_at(&LooksHost(TestHost(lib, None)), looks::LOOKS_URI).unwrap();
        let rmcp::model::ResourceContents::TextResourceContents { text, .. } = &read.contents[0] else { panic!() };
        let v: Value = serde_json::from_str(text).unwrap();
        let all = v["looks"].as_array().unwrap();
        assert_eq!(all.len(), 13, "the 12 built-ins and the plugin's");
        assert_eq!((&all[12]["id"], &all[12]["name"]), (&json!("plugin:com.test.film/warm.json"), &json!("Warm film")));
    }

    #[test]
    fn analyze_lists_facets_regions_in_the_edited_frame() {
        use photos_core::ai_masks::{self as am, AiMaskKind, NewAiMask};
        let (_tmp, lib, id) = fixture();
        let host = TestHost(lib.clone(), None);
        let shared = Shared::default();
        shared.scope.set(vec![id], Some(id));
        let analyze = || {
            let Ok(tools::Output::Json(a)) = tools::call(&host, &shared, "analyze", &args(json!({ "id": id }))) else { panic!() };
            a
        };
        assert!(analyze().get("regions").is_none(), "no detection, no regions");

        // Sky over the top half of the unedited photo.
        let sky = NewAiMask { kind: AiMaskKind::Sky, instance: 0, score: 0.99, width: 16, height: 16, weights: (0..256).map(|i| if i < 128 { 255 } else { 0 }).collect() };
        lib.db.with_conn(|c| am::put_detection(c, id, "lumen-facet-test", &[sky])).unwrap();
        let a = analyze();
        let regions = a["regions"].as_array().unwrap_or_else(|| panic!("{a}"));
        assert_eq!(regions[0]["name"], "sky", "{a}");
        let share = regions[0]["share"].as_f64().unwrap();
        assert!((share - 50.0).abs() <= 5.0, "uncropped: {share}");
        assert!(regions[0]["luma"].as_f64().is_some() && regions[0]["sat"].as_f64().is_some(), "{a}");

        let crop = json!({ "id": id, "patch": { "crop": { "x": 0.0, "y": 0.5, "w": 1.0, "h": 0.5 } }, "label": "crop" });
        assert!(tools::call(&host, &shared, "edit", &args(crop)).is_ok());
        let a = analyze();
        let share = a["regions"][0]["share"].as_f64().unwrap_or_else(|| panic!("{a}"));
        assert!(share < 5.0, "cropped to the bottom half: {share}");
    }

    #[test]
    fn view_overlays_a_mask_where_it_lands() {
        let (_tmp, lib, id) = fixture();
        let host = TestHost(lib.clone(), None);
        let shared = Shared::default();
        shared.scope.set(vec![id], Some(id));
        let mask = json!({ "id": id, "name": "fox", "shapes": [{ "kind": "radial", "center": [0.25, 0.5], "radius": [0.1, 0.1] }], "adjust": { "exposure": 0.1 } });
        assert!(tools::call(&host, &shared, "mask", &args(mask)).is_ok());
        // Red minus green at (x, 0.5) of the frame the overlay shows.
        let red_at = |xs: [f32; 2]| {
            let Ok(tools::Output::Image(jpeg, line)) = tools::call(&host, &shared, "view", &args(json!({ "id": id, "size": 600, "overlay": 1 }))) else { panic!() };
            assert!(line.contains("mask fox in red"), "{line}");
            let img = image::load_from_memory(&jpeg.bytes).unwrap().to_rgb8();
            xs.map(|x| {
                let p = img.get_pixel(((x * img.width() as f32) as u32).min(img.width() - 1), img.height() / 2);
                p[0] as i32 - p[1] as i32
            })
        };
        let [on, off] = red_at([0.25, 0.75]);
        assert!(on - off > 60, "on the mask {on}, off it {off}");

        let crop = json!({ "id": id, "patch": { "crop": { "x": 0.0, "y": 0.0, "w": 0.5, "h": 1.0 } }, "label": "crop" });
        assert!(tools::call(&host, &shared, "edit", &args(crop)).is_ok());
        let [on, off] = red_at([0.5, 0.9]);
        assert!(on - off > 60, "after the crop: on the mask {on}, off it {off}");

        let err = tools::call(&host, &shared, "view", &args(json!({ "id": id, "overlay": 7 }))).err().unwrap();
        assert_eq!(err.code, "invalid_arguments");
        assert!(err.message.contains("1 (fox)"), "{}", err.message);
    }

    fn json_of(out: AppResult<tools::Output>) -> Value {
        match out {
            Ok(tools::Output::Json(v)) => v,
            Ok(_) => panic!("an image"),
            Err(e) => panic!("{}: {}", e.code, e.message),
        }
    }

    /// `AI: …` snapshots of a photo: (name, develop temperature).
    fn snapshots(lib: &Library, id: i64) -> Vec<(String, f32)> {
        let all = lib.db.with_conn(|c| photos_core::develop_history::list_history(c, id)).unwrap();
        all.into_iter()
            .filter_map(|s| Some((s.snapshot_name?, s.recipe.develop.map_or(0.0, |d| d.basic.temperature))))
            .collect()
    }

    #[test]
    fn looks_are_scored_per_photo_and_kept_as_snapshots() {
        let scores = |v: &Value| -> Vec<(String, u64)> { v["looks"].as_array().unwrap().iter().map(|l| (l["id"].as_str().unwrap().to_string(), l["score"].as_u64().unwrap())).collect() };
        let (_t1, warm, wid) = fixture_of(|x, y| image::Rgb([200, (130 + x / 20) as u8, (80 + y / 20) as u8]));
        let (_t2, grey, gid) = fixture_of(|x, _| image::Rgb([(60 + x / 4) as u8; 3]));
        let (host_w, host_g) = (TestHost(warm.clone(), None), TestHost(grey.clone(), None));
        let (shared_w, shared_g) = (Shared::default(), Shared::default());
        shared_w.scope.set(vec![wid], Some(wid));
        shared_g.scope.set(vec![gid], Some(gid));

        let first = json_of(tools::call(&host_w, &shared_w, "suggest_looks", &args(json!({ "id": wid }))));
        let grey_first = json_of(tools::call(&host_g, &shared_g, "suggest_looks", &args(json!({ "id": gid }))));
        assert_ne!(scores(&first), scores(&grey_first), "each photo is scored on its own");
        for v in [&first, &grey_first] {
            let offer = v["offer"].as_array().unwrap();
            assert_eq!(offer[0], "natural", "{v}");
            assert!(offer.len() <= 4, "{v}");
            assert_eq!(v["threshold"], 60);
            let l = &v["looks"][0];
            assert!(l["name"].is_string() && l["reasons"].is_array() && l["intent"].is_string() && l["patch"].is_object(), "{l}");
        }
        let bw = grey_first["looks"].as_array().unwrap().iter().find(|l| l["id"] == "bw-soft").unwrap();
        assert!(bw["score"].as_u64().unwrap() >= 60, "a grey photo suits B&W: {bw}");

        // Asked again, and after an edit: the same numbers, since they come from the before render.
        assert!(tools::call(&host_w, &shared_w, "edit", &args(json!({ "id": wid, "patch": { "develop": { "basic": { "exposure": 1.5, "saturation": 0.8 } } }, "label": "bold" }))).is_ok());
        let again = json_of(tools::call(&host_w, &shared_w, "suggest_looks", &args(json!({ "id": wid }))));
        assert_eq!(scores(&first), scores(&again));
        assert_eq!(tools::call(&host_w, &shared_w, "suggest_looks", &args(json!({ "id": wid + 99 }))).err().unwrap().code, "not_in_develop");

        // Two variants with a reset between them become two AI snapshots.
        assert!(tools::call(&host_w, &shared_w, "reset", &args(json!({ "id": wid }))).is_ok());
        let warm_patch = json!({ "id": wid, "patch": { "develop": { "basic": { "temperature": 20 } } }, "label": "a" });
        assert!(tools::call(&host_w, &shared_w, "edit", &args(warm_patch)).is_ok());
        let out = json_of(tools::call(&host_w, &shared_w, "draft_variant", &args(json!({ "id": wid, "name": "Natural", "score": 70 }))));
        assert_eq!(out["snapshots"], json!(["AI: Natural · 70"]));
        assert!(tools::call(&host_w, &shared_w, "reset", &args(json!({ "id": wid }))).is_ok());
        let cool_patch = json!({ "id": wid, "patch": { "develop": { "basic": { "temperature": -30 } } }, "label": "b" });
        assert!(tools::call(&host_w, &shared_w, "edit", &args(cool_patch)).is_ok());
        let out = json_of(tools::call(&host_w, &shared_w, "draft_variant", &args(json!({ "id": wid, "name": "Cool moody", "score": 64 }))));
        assert_eq!(out["snapshots"], json!(["AI: Natural · 70", "AI: Cool moody · 64"]));
        assert_eq!(snapshots(&warm, wid), [("AI: Natural · 70".to_string(), 20.0), ("AI: Cool moody · 64".to_string(), -30.0)]);

        // `use` puts the first one's recipe back on the photo.
        let out = json_of(tools::call(&host_w, &shared_w, "draft_variant", &args(json!({ "id": wid, "name": "Natural", "op": "use" }))));
        assert_eq!(out["snapshots"].as_array().unwrap().len(), 2, "{out}");
        assert_eq!(tools::recipe(&warm, wid).unwrap().develop.unwrap().basic.temperature, 20.0);
        let err = tools::call(&host_w, &shared_w, "draft_variant", &args(json!({ "id": wid, "name": "Nothing", "op": "use" }))).err().unwrap();
        assert_eq!(err.code, "invalid_arguments");
        assert!(err.message.contains("Natural"), "{}", err.message);
        assert_eq!(tools::call(&host_w, &shared_w, "draft_variant", &args(json!({ "id": wid + 99, "name": "x" }))).err().unwrap().code, "not_in_develop");
    }

    #[test]
    fn a_variant_saves_the_pages_live_recipe_not_the_stale_draft() {
        let (_tmp, lib, id) = fixture();
        // The draft in the database says exposure 1; the page, which is ahead of it, says temperature 10.
        lib.db.with_conn(|c| photos_core::edits::save_draft(c, id, &photos_core::edits::Recipe::identity(), 1)).unwrap();
        let shared = Arc::new(Shared::default());
        shared.scope.set(vec![id], Some(id));
        shared.scope.set_mounted(true);
        let (s, l) = (shared.clone(), lib.clone());
        let page = move |req: &bridge::McpRequest| {
            let host = TestHost(l.clone(), None);
            let mut develop = photos_core::edits::Develop::default();
            develop.basic.temperature = 10.0;
            let live = photos_core::edits::Recipe { develop: Some(develop), ..photos_core::edits::Recipe::identity() };
            let r = write::transform(&host, &l, req.asset_id, &live, &req.op).unwrap();
            s.bridge.reply(req.req, bridge::McpReply { recipe: Some(r), error: None, message: None });
        };
        let host = TestHost(lib.clone(), Some(Arc::new(page)));
        let out = json_of(tools::call(&host, &shared, "draft_variant", &args(json!({ "id": id, "name": "Live", "score": 61 }))));
        assert_eq!(out["snapshots"], json!(["AI: Live · 61"]));
        assert_eq!(snapshots(&lib, id), [("AI: Live · 61".to_string(), 10.0)]);
    }

    #[test]
    fn reads_see_the_pages_live_recipe_not_the_stale_draft() {
        let (_tmp, lib, id) = fixture();
        // The page has just taken a `use` the database draft has not caught up with yet.
        lib.db.with_conn(|c| photos_core::edits::save_draft(c, id, &photos_core::edits::Recipe::identity(), 1)).unwrap();
        let shared = Arc::new(Shared::default());
        shared.scope.set(vec![id], Some(id));
        shared.scope.set_mounted(true);
        let (s, l) = (shared.clone(), lib.clone());
        let page = move |req: &bridge::McpRequest| {
            let host = TestHost(l.clone(), None);
            let mut develop = photos_core::edits::Develop::default();
            develop.basic.temperature = 10.0;
            let live = photos_core::edits::Recipe { develop: Some(develop), ..photos_core::edits::Recipe::identity() };
            let r = write::transform(&host, &l, req.asset_id, &live, &req.op).unwrap();
            s.bridge.reply(req.req, bridge::McpReply { recipe: Some(r), error: None, message: None });
        };
        let host = TestHost(lib.clone(), Some(Arc::new(page)));
        let out = json_of(tools::call(&host, &shared, "get_recipe", &args(json!({ "id": id }))));
        assert!(out.to_string().contains("10"), "get_recipe should show the page's temperature: {out}");
        assert!(tools::call(&host, &shared, "view", &args(json!({ "id": id }))).is_ok());
    }

    #[test]
    fn the_playbook_asks_for_looks_not_restraint() {
        for needle in ["suggest_looks", "draft_variant", "60", "reset", "build that one look"] {
            assert!(INSTRUCTIONS.contains(needle), "the playbook lacks {needle}");
        }
        for banned in ["restraint", "subtle"] {
            assert!(!INSTRUCTIONS.to_lowercase().contains(banned), "the playbook still says {banned}");
        }
        for (name, description, schema) in tools::list() {
            for text in [description, schema.to_string()] {
                assert!(!text.to_lowercase().contains("keep it subtle"), "{name} still asks for subtlety");
            }
        }
        let adjust = tools::list().into_iter().find(|t| t.0 == "mask").unwrap().2.to_string();
        assert!(adjust.contains("one stop brighter"), "adjust says what the values do");
        let edit = tools::list().into_iter().find(|t| t.0 == "edit").unwrap().1;
        assert!(edit.contains("curve") && edit.contains("grading") && edit.contains("looks"), "edit says where looks are made");

        let with = prompt_text("edit_photo", Some("moody film")).unwrap();
        assert!(with.contains("moody film") && with.contains("build that one look"), "{with}");
        assert!(prompt_text("edit_photo", None).unwrap().contains("Start with list_photos"));
        assert!(!prompt_text("edit_photo", Some("  ")).unwrap().contains("The brief"));
        assert!(prompt_text("retouch", Some("moody film")).unwrap().contains("moody film"), "retouch stays an alias");
        assert!(prompt_text("nothing", None).is_none());
        let names: Vec<String> = prompts().into_iter().map(|p| p.name).collect();
        assert_eq!(names, ["edit_photo", "retouch"]);
    }
}
