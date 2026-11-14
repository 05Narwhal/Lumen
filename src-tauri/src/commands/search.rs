//! Search (Round 4K): thin adapters over `crate::search`.

use photos_core::search_overview::SearchOverview;
use photos_core::tag_search::TagSuggestion;
use photos_core::tag_sentence::SentenceReading;
use tauri::{AppHandle, State};

use crate::error::AppResult;
use crate::state::AppState;

/// The results page's top: what `q` was read as, and the people, places, albums and memories
/// it names, with the number of photos it finds.
#[tauri::command(async)]
pub fn search_overview(state: State<'_, AppState>, q: String) -> AppResult<SearchOverview> {
    crate::search::overview(&state, q.trim())
}

/// The tags a grid page's search field offers for `input` (Round 7), best first, the text
/// itself last.
#[tauri::command(async)]
pub fn suggest_tags(app: AppHandle, state: State<'_, AppState>, input: String) -> AppResult<Vec<TagSuggestion>> {
    crate::search::suggest_tags(&app, &state, &input)
}

/// A sentence typed in a grid page's search field read into tags (Round 20 §2): `live and hdr
/// not screenshot` → `[Live Photos] and [HDR] and not [Screenshots]`. `None` when the text is one
/// plain term, and the field then behaves as before.
#[tauri::command(async)]
pub fn read_search_sentence(app: AppHandle, state: State<'_, AppState>, input: String) -> AppResult<Option<SentenceReading>> {
    crate::search::read_search_sentence(&app, &state, &input)
}
