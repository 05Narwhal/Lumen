use photos_core::edits::Recipe;
use photos_core::Settings;
use tauri::{AppHandle, State};

use crate::error::AppResult;
use crate::mcp::bridge::{McpReply, Op};
use crate::mcp::McpStatus;
use crate::state::AppState;

/// What AI assistants may see (Round 16): Develop's filmstrip in order and the open photo.
/// The page calls it whenever either changes, from anywhere in the app.
#[tauri::command(async)]
pub fn mcp_set_scope(state: State<'_, AppState>, ids: Vec<i64>, open: Option<i64>) -> AppResult<()> {
    state.mcp.shared.scope.set(ids, open);
    Ok(())
}

/// Develop's workspace is on screen (`true`) or left (`false`): edits to the open photo go
/// through the page while it is.
#[tauri::command(async)]
pub fn mcp_set_mounted(state: State<'_, AppState>, mounted: bool) -> AppResult<()> {
    state.mcp.shared.scope.set_mounted(mounted);
    Ok(())
}

/// An AI change on the recipe on screen, for the page to apply as one history step.
#[tauri::command(async)]
pub fn mcp_transform(app: AppHandle, state: State<'_, AppState>, asset_id: i64, recipe: Recipe, op: Op) -> AppResult<Recipe> {
    let host = crate::mcp::AppHost(app);
    let lib = state.library()?;
    crate::mcp::write::transform(&host, &lib, asset_id, &recipe, &op)
}

/// The page's answer to an `mcp://request`.
#[tauri::command(async)]
pub fn mcp_reply(state: State<'_, AppState>, req: u64, reply: McpReply) -> AppResult<()> {
    state.mcp.shared.bridge.reply(req, reply);
    Ok(())
}

/// Settings' line on the server: off, waiting, connected (and to whom), or why it can't run.
#[tauri::command(async)]
pub fn mcp_status(state: State<'_, AppState>) -> AppResult<McpStatus> {
    let enabled = state.settings.read().expect("settings lock").ai_assistants.enabled;
    Ok(state.mcp.status(enabled))
}

/// *New key*: clients holding the old one are refused from now on.
#[tauri::command(async)]
pub fn mcp_regenerate_token(app: AppHandle, state: State<'_, AppState>) -> AppResult<Settings> {
    let settings = {
        let mut s = state.settings.write().expect("settings lock");
        s.ai_assistants.token = crate::mcp::new_token();
        s.clone()
    };
    state.save_settings()?;
    crate::mcp::apply(&app);
    Ok(settings)
}

/// *Connect* for one assistant ("claude-code", "codex", "claude-desktop"): writes the entry
/// that runs `Lumen --mcp`. Returns the sentence to show.
#[tauri::command(async)]
pub fn mcp_connect(client: String) -> AppResult<String> {
    let exe = std::env::current_exe()?.to_string_lossy().into_owned();
    crate::mcp::connect::connect(&client, &exe).map_err(|message| crate::error::AppError { code: "mcp_connect".into(), message, detail: None })
}
