use tauri::State;

use crate::discord_presence::{self, DiscordInfo, PresenceScreen};
use crate::error::AppResult;
use crate::state::AppState;

/// The page's current screen, as the fixed enum and nothing else. Also the idle heartbeat.
#[tauri::command(async)]
pub fn discord_set_screen(screen: PresenceScreen) -> AppResult<()> {
    discord_presence::set_screen(screen);
    Ok(())
}

/// For Settings: whether this build can show a status, whether it does, and the preview line.
#[tauri::command(async)]
pub fn discord_info(state: State<'_, AppState>) -> AppResult<DiscordInfo> {
    let status = state.settings.read().expect("settings lock").discord.clone();
    Ok(DiscordInfo {
        available: discord_presence::client_id().is_some(),
        state: discord_presence::state_of(status.enabled, discord_presence::showing()),
        preview: discord_presence::preview_line(&status, discord_presence::last_screen()),
    })
}
