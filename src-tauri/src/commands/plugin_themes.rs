//! Plugin themes for the page (Round 23 lane B1): the list the theme picker shows and the CSS
//! `<style id="plugin-themes">` is filled with. Thin adapters over `plugins::themes`.

use std::sync::Arc;

use photos_plugins::theme::PluginThemeMeta;
use tauri::State;

use crate::error::AppResult;
use crate::plugins::{themes, Plugins};

/// The themes of the enabled plugins, in plugin and file order.
#[tauri::command(async)]
pub fn list_plugin_themes(plugins: State<'_, Arc<Plugins>>) -> AppResult<Vec<PluginThemeMeta>> {
    Ok(themes::list(&plugins))
}

/// The CSS for every enabled plugin theme (empty when there are none).
#[tauri::command(async)]
pub fn plugin_theme_css(plugins: State<'_, Arc<Plugins>>) -> AppResult<String> {
    Ok(themes::css(&plugins))
}
