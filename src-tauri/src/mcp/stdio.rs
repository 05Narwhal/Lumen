//! `Lumen --mcp` (Round 16): a stdio MCP server for clients that only speak stdio (Claude
//! Desktop). It is a pipe, not a second server: each JSON-RPC line from stdin is posted to the
//! running app's `/mcp` with the key from Lumen's settings file, and the answers (a JSON body or
//! an SSE stream) go to stdout one line each. The client's config holds no key.

use std::io::{BufRead, Write};

use serde_json::{json, Value};

/// Where the running app listens, and the key it wants.
pub struct Target {
    pub url: String,
    pub token: String,
}

const NOT_RUNNING: &str = "Lumen isn't running. Open Lumen, then try again.";

/// How long the probe waits for Lumen's port. Windows retries a refused connection to localhost
/// for ~2 s before reporting it; a listening app on the same machine accepts well within this.
const PROBE: std::time::Duration = std::time::Duration::from_millis(300);

/// Whether anything listens at `url`'s address, without waiting out Windows' retries. An address
/// this can't read counts as reachable, and the request then says what is wrong.
fn reachable(url: &str) -> bool {
    use std::net::ToSocketAddrs;
    let Ok(u) = reqwest::Url::parse(url) else { return true };
    let (Some(host), Some(port)) = (u.host_str(), u.port_or_known_default()) else { return true };
    let Ok(mut addrs) = (host, port).to_socket_addrs() else { return true };
    addrs.any(|a| std::net::TcpStream::connect_timeout(&a, PROBE).is_ok())
}

const OFF: &str = "AI assistants are off in Lumen. Turn them on in Lumen's Settings → AI assistants.";

/// Read the port and key from Lumen's settings; `None` when AI assistants are off.
fn target() -> Option<Target> {
    let paths = photos_core::paths::AppPaths::resolve().ok()?;
    let s = photos_core::Settings::load(&paths.settings_file()).ok()?.ai_assistants;
    (s.enabled && !s.token.is_empty()).then(|| Target { url: format!("http://127.0.0.1:{}/mcp", s.port), token: s.token })
}

/// The process's whole life in `--mcp` mode; the exit code.
pub fn main() -> i32 {
    let stdin = std::io::stdin();
    match bridge(stdin.lock(), std::io::stdout(), target) {
        Ok(()) => 0,
        Err(_) => 1,
    }
}

/// A JSON-RPC error for `msg` when it is a request (it has an `id`); notifications get none.
fn error_for(msg: &Value, message: &str) -> Option<Value> {
    let id = msg.get("id").filter(|_| msg.get("method").is_some())?;
    Some(json!({ "jsonrpc": "2.0", "id": id, "error": { "code": -32000, "message": message } }))
}

/// The JSON-RPC messages in an SSE body: each event's `data:` lines, joined.
fn sse_messages(body: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut data = String::new();
    for line in body.lines().chain(std::iter::once("")) {
        if line.is_empty() {
            if !data.trim().is_empty() && serde_json::from_str::<Value>(&data).is_ok() {
                out.push(std::mem::take(&mut data));
            }
            data.clear();
        } else if let Some(d) = line.strip_prefix("data:") {
            if !data.is_empty() {
                data.push('\n');
            }
            data.push_str(d.strip_prefix(' ').unwrap_or(d));
        }
    }
    out
}

/// What the bridge can say itself while the app is off or closed, so a client's handshake
/// still completes (Codex drops a server whose `initialize` fails). Requests that need the app
/// get `None` and an error instead.
fn offline_result(msg: &Value) -> Option<Value> {
    let result = match msg.get("method")?.as_str()? {
        "initialize" => {
            let version = msg.pointer("/params/protocolVersion").and_then(Value::as_str).unwrap_or("2025-06-18");
            json!({
                "protocolVersion": version,
                "capabilities": { "tools": { "listChanged": true } },
                "serverInfo": { "name": "lumen", "version": env!("CARGO_PKG_VERSION") },
                "instructions": "Lumen isn't running or has AI assistants off, so there are no tools yet. Open Lumen and turn on Settings → AI assistants."
            })
        }
        "ping" => json!({}),
        "tools/list" => json!({ "tools": [] }),
        "prompts/list" => json!({ "prompts": [] }),
        "resources/list" => json!({ "resources": [] }),
        "resources/templates/list" => json!({ "resourceTemplates": [] }),
        _ => return None,
    };
    let id = msg.get("id")?;
    Some(json!({ "jsonrpc": "2.0", "id": id, "result": result }))
}

/// Pipe `input` to the app and its answers to `output`, until `input` ends. `target` is asked
/// for every request, so turning the setting on or restarting Lumen needs no new client.
pub fn bridge(input: impl BufRead, mut output: impl Write, mut target: impl FnMut() -> Option<Target>) -> std::io::Result<()> {
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build()?;
    let client = reqwest::Client::new();
    let mut session: Option<String> = None;
    // The client was answered locally; tell it the tools exist once the app is reachable.
    let mut announce = false;
    for line in input.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let Ok(msg) = serde_json::from_str::<Value>(&line) else {
            writeln!(output, "{}", json!({ "jsonrpc": "2.0", "id": null, "error": { "code": -32700, "message": "Parse error" } }))?;
            output.flush()?;
            continue;
        };
        let Some(t) = target() else {
            announce = true;
            session = None;
            if let Some(e) = offline_result(&msg).or_else(|| error_for(&msg, OFF)) {
                writeln!(output, "{e}")?;
                output.flush()?;
            }
            continue;
        };
        let answer = if !reachable(&t.url) {
            Err(NOT_RUNNING.to_string())
        } else {
            rt.block_on(async {
                let mut req = client
                    .post(&t.url)
                    .header("content-type", "application/json")
                    .header("accept", "application/json, text/event-stream")
                    .header("authorization", format!("Bearer {}", t.token))
                    .body(line.clone());
                if let Some(s) = &session {
                    req = req.header("mcp-session-id", s);
                }
                let resp = req.send().await.map_err(|e| {
                    if e.is_connect() {
                        NOT_RUNNING.to_string()
                    } else {
                        e.to_string()
                    }
                })?;
                if let Some(s) = resp.headers().get("mcp-session-id").and_then(|v| v.to_str().ok()) {
                    session = Some(s.to_string());
                }
                let status = resp.status();
                let sse = resp.headers().get("content-type").and_then(|v| v.to_str().ok()).is_some_and(|c| c.starts_with("text/event-stream"));
                let body = resp.text().await.map_err(|e| e.to_string())?;
                match status.as_u16() {
                    200..=299 if sse => Ok(sse_messages(&body)),
                    200..=299 => Ok(if body.trim().is_empty() { vec![] } else { vec![body] }),
                    401 => Err("Lumen refused the key. Its key changed: in Lumen's Settings → AI assistants, copy the setup again.".to_string()),
                    404 if session.is_some() => {
                        session = None;
                        Err("Lumen restarted; reconnect the assistant.".to_string())
                    }
                    _ => Err(format!("Lumen answered {status}: {}", body.trim())),
                }
            })
        };
        // Not running: the handshake and listings are answered here; calls say why they can't be.
        let mut from_app = true;
        let answer = match answer {
            Err(e) if e == NOT_RUNNING => {
                announce = true;
                from_app = false;
                session = None;
                match offline_result(&msg) {
                    Some(r) => Ok(vec![r.to_string()]),
                    None => Err(e),
                }
            }
            other => other,
        };
        match answer {
            Ok(messages) => {
                // Only an answer from the app means the tools exist; one made here is still empty.
                if from_app && announce && messages.iter().all(|m| !m.contains("\"serverInfo\"")) && msg.get("method").and_then(Value::as_str) != Some("initialize") {
                    announce = false;
                    writeln!(output, "{}", json!({ "jsonrpc": "2.0", "method": "notifications/tools/list_changed" }))?;
                }
                for m in messages {
                    writeln!(output, "{}", m.replace('\n', ""))?;
                }
            }
            Err(e) => {
                if let Some(err) = error_for(&msg, &e) {
                    writeln!(output, "{err}")?;
                }
            }
        }
        output.flush()?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(input: &str, target: Option<Target>) -> Vec<Value> {
        let mut out = Vec::new();
        let mut t = Some(target);
        bridge(input.as_bytes(), &mut out, move || t.take().unwrap_or(None)).unwrap();
        String::from_utf8(out).unwrap().lines().map(|l| serde_json::from_str(l).unwrap()).collect()
    }

    fn dead_target() -> Target {
        // Nothing listens on a port just freed.
        let port = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap().local_addr().unwrap().port();
        Target { url: format!("http://127.0.0.1:{port}/mcp"), token: "k".into() }
    }

    const INIT: &str = "{\"jsonrpc\":\"2.0\",\"id\":7,\"method\":\"initialize\",\"params\":{\"protocolVersion\":\"2025-03-26\"}}
";
    const NOTE: &str = "{\"jsonrpc\":\"2.0\",\"method\":\"notifications/initialized\"}
";

    #[test]
    fn off_still_completes_the_handshake_so_the_client_can_start() {
        let lines = run(&format!("{INIT}{NOTE}{{\"jsonrpc\":\"2.0\",\"id\":8,\"method\":\"tools/list\"}}
"), None);
        assert_eq!(lines.len(), 2, "{lines:?}");
        assert_eq!(lines[0]["id"], 7);
        assert_eq!(lines[0]["result"]["protocolVersion"], "2025-03-26");
        assert!(lines[0]["result"]["capabilities"]["tools"].is_object());
        assert_eq!(lines[0]["result"]["serverInfo"]["name"], "lumen");
        assert_eq!(lines[1]["result"]["tools"], json!([]));
    }

    #[test]
    fn lumen_not_running_still_completes_the_handshake() {
        let lines = run(&format!("{INIT}{{\"jsonrpc\":\"2.0\",\"id\":9,\"method\":\"ping\"}}
"), Some(dead_target()));
        assert_eq!(lines.len(), 2, "{lines:?}");
        assert!(lines[0]["result"]["serverInfo"].is_object());
        assert_eq!(lines[1]["result"], json!({}));
    }

    #[test]
    fn a_tool_call_while_off_or_closed_says_why() {
        let call = "{\"jsonrpc\":\"2.0\",\"id\":3,\"method\":\"tools/call\",\"params\":{\"name\":\"list_photos\"}}
";
        let off = run(call, None);
        assert!(off[0]["error"]["message"].as_str().unwrap().contains("Settings → AI assistants"));
        let closed = run(call, Some(dead_target()));
        assert!(closed[0]["error"]["message"].as_str().unwrap().contains("isn't running"), "{closed:?}");
    }

    #[test]
    fn notifications_get_no_answer() {
        assert!(run(NOTE, None).is_empty());
    }

    #[test]
    fn sse_events_become_one_line_each() {
        let body = "id: 0
retry: 3000
data:

data: {\"jsonrpc\":\"2.0\",\"id\":1,
 data: \"result\":{}}

";
        let body = body.replace("
 data:", "
data:");
        assert_eq!(sse_messages(&body), ["{\"jsonrpc\":\"2.0\",\"id\":1,
\"result\":{}}"]);
    }

    #[test]
    fn the_key_is_read_again_for_every_request() {
        // Off at first, then the user turns it on: the second request reaches for the app.
        let input = "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/call\"}
{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/call\"}
";
        let mut out = Vec::new();
        let mut calls = 0;
        bridge(input.as_bytes(), &mut out, || {
            calls += 1;
            (calls > 1).then(dead_target)
        })
        .unwrap();
        let lines: Vec<Value> = String::from_utf8(out).unwrap().lines().map(|l| serde_json::from_str(l).unwrap()).collect();
        assert!(lines[0]["error"]["message"].as_str().unwrap().contains("Settings"));
        assert!(lines[1]["error"]["message"].as_str().unwrap().contains("isn't running"));
    }

    const LIST: &str = "{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/list\"}
";

    fn is_list_changed(l: &Value) -> bool {
        l["method"] == "notifications/tools/list_changed"
    }

    fn lines_of(out: Vec<u8>) -> Vec<Value> {
        String::from_utf8(out).unwrap().lines().map(|l| serde_json::from_str(l).unwrap()).collect()
    }

    // 2026-11-01 release check: an empty listing answered here was followed by `list_changed`,
    // so a client that lists again on it would loop while Lumen stays closed.
    #[test]
    fn a_listing_answered_here_never_announces_tools() {
        let url = dead_target().url;
        let mut out = Vec::new();
        bridge(format!("{INIT}{NOTE}{LIST}{LIST}").as_bytes(), &mut out, || Some(Target { url: url.clone(), token: "k".into() })).unwrap();
        let lines = lines_of(out);
        assert!(!lines.iter().any(is_list_changed), "{lines:?}");
        assert_eq!(lines.len(), 3, "{lines:?}");
    }

    // Windows retries a refused connection to localhost for ~2 s before reporting it, so every
    // request waited that long while Lumen was closed.
    #[test]
    fn a_closed_lumen_answers_without_waiting() {
        let url = dead_target().url;
        let start = std::time::Instant::now();
        let mut out = Vec::new();
        bridge(format!("{INIT}{LIST}").as_bytes(), &mut out, || Some(Target { url: url.clone(), token: "k".into() })).unwrap();
        assert!(start.elapsed() < std::time::Duration::from_millis(1500), "{:?}", start.elapsed());
    }

    /// A tiny HTTP server that answers `tools/list` with one tool (connections that send
    /// nothing, like the reachability probe, are skipped).
    fn live_target() -> Target {
        use std::io::Read;
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            for stream in listener.incoming().take(2) {
                let mut s = stream.unwrap();
                let mut buf = [0u8; 4096];
                if s.read(&mut buf).unwrap_or(0) == 0 {
                    continue;
                }
                let body = "{\"jsonrpc\":\"2.0\",\"id\":2,\"result\":{\"tools\":[{\"name\":\"view\"}]}}";
                let _ = write!(s, "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}", body.len());
            }
        });
        Target { url: format!("http://127.0.0.1:{port}/mcp"), token: "k".into() }
    }

    #[test]
    fn tools_are_announced_once_the_app_answers() {
        // Off for the handshake, then on: the listing comes from the app, after `list_changed`.
        let mut live = Some(live_target());
        let mut calls = 0;
        let mut out = Vec::new();
        bridge(format!("{INIT}{LIST}").as_bytes(), &mut out, || {
            calls += 1;
            if calls > 1 {
                live.take()
            } else {
                None
            }
        })
        .unwrap();
        let lines = lines_of(out);
        assert_eq!(lines.len(), 3, "{lines:?}");
        assert!(is_list_changed(&lines[1]), "{lines:?}");
        assert_eq!(lines[2]["result"]["tools"][0]["name"], "view");
    }
}
