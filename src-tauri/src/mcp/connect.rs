//! One-click *Connect* (Round 16 follow-up): point an assistant at `Lumen --mcp`, the stdio
//! bridge. The entry holds no key (the bridge reads it from Lumen's settings), so *New key*
//! never breaks it, and it starts even while Lumen is closed. Only the `lumen` entry is touched.

use std::path::PathBuf;

use serde_json::{json, Value};

/// Claude Desktop's config file.
fn desktop_path() -> Option<PathBuf> {
    #[cfg(target_os = "macos")]
    let base = dirs::home_dir()?.join("Library/Application Support");
    #[cfg(not(target_os = "macos"))]
    let base = dirs::config_dir()?;
    Some(base.join("Claude").join("claude_desktop_config.json"))
}

/// Codex's config file (`CODEX_HOME` when set).
fn codex_path() -> Option<PathBuf> {
    let home = std::env::var_os("CODEX_HOME").map(PathBuf::from).or_else(|| dirs::home_dir().map(|h| h.join(".codex")))?;
    Some(home.join("config.toml"))
}

/// `text` (Claude Desktop's JSON, maybe empty) with `mcpServers.lumen` set; every other key kept.
pub fn merge_desktop(text: &str, exe: &str) -> Result<String, String> {
    let mut root: Value = if text.trim().is_empty() {
        json!({})
    } else {
        serde_json::from_str(text).map_err(|e| format!("Claude Desktop's config isn't valid JSON ({e}); fix or remove it first."))?
    };
    let obj = root.as_object_mut().ok_or("Claude Desktop's config isn't a JSON object.")?;
    let servers = obj.entry("mcpServers").or_insert_with(|| json!({}));
    let servers = servers.as_object_mut().ok_or("Claude Desktop's mcpServers isn't an object.")?;
    servers.insert("lumen".into(), json!({ "command": exe, "args": ["--mcp"] }));
    serde_json::to_string_pretty(&root).map_err(|e| e.to_string())
}

fn toml_str(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

/// `text` (Codex's TOML) with a fresh `[mcp_servers.lumen]` table: the old one and its
/// sub-tables are dropped, nothing else is.
pub fn merge_codex(text: &str, exe: &str) -> String {
    let mut kept: Vec<&str> = Vec::new();
    let mut skipping = false;
    for line in text.lines() {
        let t = line.trim();
        if t.starts_with('[') {
            let name = t.trim_start_matches('[').split(']').next().unwrap_or("").trim();
            skipping = name == "mcp_servers.lumen" || name.starts_with("mcp_servers.lumen.");
        }
        if !skipping {
            kept.push(line);
        }
    }
    while kept.last().is_some_and(|l| l.trim().is_empty()) {
        kept.pop();
    }
    let mut out = kept.join("\n");
    if !out.is_empty() {
        out.push_str("\n\n");
    }
    out.push_str(&format!("[mcp_servers.lumen]\ncommand = {}\nargs = [\"--mcp\"]\n", toml_str(exe)));
    out
}

/// The arguments of `claude mcp add` for a user-wide `lumen` server.
pub fn claude_code_args(exe: &str) -> Vec<String> {
    ["mcp", "add", "--scope", "user", "lumen", "--", exe, "--mcp"].map(String::from).to_vec()
}

/// Write `new` to `path` through a temp file, keeping the first version as `.lumen-backup`.
fn write_with_backup(path: &PathBuf, new: &str) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    if path.exists() {
        let mut bak = path.clone().into_os_string();
        bak.push(".lumen-backup");
        let bak = PathBuf::from(bak);
        if !bak.exists() {
            std::fs::copy(path, bak).map_err(|e| e.to_string())?;
        }
    }
    let mut tmp = path.clone().into_os_string();
    tmp.push(".lumen-tmp");
    let tmp = PathBuf::from(tmp);
    std::fs::write(&tmp, new).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, path).map_err(|e| e.to_string())
}

fn claude_cli(args: &[String]) -> std::io::Result<std::process::Output> {
    #[cfg(windows)]
    let mut c = {
        use std::os::windows::process::CommandExt;
        let mut c = std::process::Command::new("cmd");
        c.args(["/C", "claude"]).creation_flags(0x0800_0000);
        c
    };
    #[cfg(not(windows))]
    let mut c = std::process::Command::new("claude");
    c.args(args).stdin(std::process::Stdio::null()).output()
}

/// Connect `client` ("claude-code", "codex" or "claude-desktop"); the sentence to show.
pub fn connect(client: &str, exe: &str) -> Result<String, String> {
    match client {
        "claude-desktop" => {
            let path = desktop_path().ok_or("Couldn't find Claude Desktop's config folder.")?;
            let old = std::fs::read_to_string(&path).unwrap_or_default();
            write_with_backup(&path, &merge_desktop(&old, exe)?)?;
            Ok("Connected. Restart Claude Desktop to see Lumen.".into())
        }
        "codex" => {
            let path = codex_path().ok_or("Couldn't find Codex's config folder.")?;
            let old = std::fs::read_to_string(&path).unwrap_or_default();
            write_with_backup(&path, &merge_codex(&old, exe))?;
            Ok("Connected. Start a new Codex session to see Lumen.".into())
        }
        "claude-code" => {
            let _ = claude_cli(&["mcp".into(), "remove".into(), "--scope".into(), "user".into(), "lumen".into()]);
            match claude_cli(&claude_code_args(exe)) {
                Ok(o) if o.status.success() => Ok("Connected. Start a new Claude Code session to see Lumen.".into()),
                Ok(o) => Err(format!("Claude Code refused: {}", String::from_utf8_lossy(&o.stderr).trim())),
                Err(_) => Err("Couldn't run `claude`. Is Claude Code installed? Use the setup line below instead.".into()),
            }
        }
        _ => Err("Unknown assistant.".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn desktop_merge_keeps_other_servers_and_keys() {
        let old = r#"{"theme":"dark","mcpServers":{"other":{"command":"x"},"lumen":{"command":"old"}}}"#;
        let v: Value = serde_json::from_str(&merge_desktop(old, "C:\\Lumen\\Lumen.exe").unwrap()).unwrap();
        assert_eq!(v["theme"], "dark");
        assert_eq!(v["mcpServers"]["other"]["command"], "x");
        assert_eq!(v["mcpServers"]["lumen"], json!({"command":"C:\\Lumen\\Lumen.exe","args":["--mcp"]}));
    }

    #[test]
    fn desktop_merge_starts_from_nothing_and_refuses_broken_json() {
        assert!(merge_desktop("", "L").unwrap().contains("\"lumen\""));
        assert!(merge_desktop("{ nope", "L").is_err());
        assert!(merge_desktop("[]", "L").is_err());
    }

    #[test]
    fn codex_merge_replaces_only_the_lumen_table() {
        let old = "model = \"x\"\n\n[mcp_servers.lumen]\nurl = \"http://old\"\nhttp_headers = { A = \"b\" }\n\n[mcp_servers.lumen.env]\nK = \"v\"\n\n[mcp_servers.other]\ncommand = \"o\"\n";
        let new = merge_codex(old, "C:\\L\\Lumen.exe");
        assert!(new.contains("model = \"x\""));
        assert!(new.contains("[mcp_servers.other]\ncommand = \"o\""));
        assert!(!new.contains("http://old") && !new.contains("K = \"v\""));
        assert!(new.contains("command = \"C:\\\\L\\\\Lumen.exe\"\nargs = [\"--mcp\"]"));
        assert_eq!(new.matches("[mcp_servers.lumen]").count(), 1);
        // Running it twice changes nothing.
        assert_eq!(merge_codex(&new, "C:\\L\\Lumen.exe"), new);
    }

    #[test]
    fn codex_merge_on_an_empty_file() {
        assert_eq!(merge_codex("", "/a/Lumen"), "[mcp_servers.lumen]\ncommand = \"/a/Lumen\"\nargs = [\"--mcp\"]\n");
    }

    #[test]
    fn claude_code_gets_a_user_scoped_stdio_server() {
        assert_eq!(claude_code_args("/a/Lumen").join(" "), "mcp add --scope user lumen -- /a/Lumen --mcp");
    }
}
