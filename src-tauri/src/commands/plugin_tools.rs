//! Round 23 A4: what plugins add to the AI assistants' server, for Settings → AI assistants.

use std::sync::Arc;

use tauri::AppHandle;

use crate::error::AppResult;
use crate::mcp::{AppHost, Host};
use crate::state::AppState;

/// How many MCP tools enabled plugins add; 0 while AI assistants are off. Reading a plugin's
/// tools the first time runs its wasm, so this is an `async` command (a worker thread).
#[tauri::command(async)]
pub fn plugin_tool_count(app: AppHandle, state: tauri::State<'_, AppState>) -> AppResult<u32> {
    let host: Arc<dyn Host> = Arc::new(AppHost(app));
    Ok(crate::plugins::mcp_tools::plugin_tool_count(&host, &state.mcp.shared))
}
