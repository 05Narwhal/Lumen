//! Music: thin adapters over `crate::music`. The ones that reach the network are `async`,
//! so the main thread never waits on Jamendo, and none of them ever returns the client id.

use std::path::{Path, PathBuf};

use tauri::{AppHandle, Manager, State};

use crate::error::{AppError, AppResult};
use crate::music::{self, MusicQuery, MusicTrack, SavedTrack};
use crate::state::AppState;

fn no_key() -> AppError {
    AppError {
        code: "music_no_key".into(),
        message: "Add a free Jamendo client id in Settings → Music to browse for music.".into(),
        detail: None,
    }
}

/// Searches Jamendo. Returns nothing at all until a client id has been entered.
#[tauri::command]
pub async fn music_search(app: AppHandle, query: MusicQuery) -> AppResult<Vec<MusicTrack>> {
    // Everything the request needs is taken before the await, so no lock is held across it.
    let (client, client_id) = {
        let state = app.state::<AppState>();
        let id = state.settings.read().expect("settings lock").music.jamendo_client_id.clone();
        (state.http.clone(), id.filter(|k| !k.trim().is_empty()).ok_or_else(no_key)?)
    };
    let tracks = music::search(&client, &client_id, &query).await?;
    app.state::<AppState>().music_previews.remember(&tracks);
    Ok(tracks)
}

/// Saves a track into `<library>/music`, with the sidecar that credits it.
#[tauri::command]
pub async fn music_download(app: AppHandle, track: MusicTrack) -> AppResult<SavedTrack> {
    let (client, root) = {
        let state = app.state::<AppState>();
        (state.http.clone(), state.library()?.root.clone())
    };
    let saved = music::download(&client, &track, &root).await?;
    Ok(saved)
}

/// Everything in `<library>/music`: saved tracks and any audio the user put there.
#[tauri::command(async)]
pub fn list_library_music(state: State<'_, AppState>) -> AppResult<Vec<SavedTrack>> {
    let lib = state.library()?;
    Ok(music::list_saved(&lib.root))
}

/// Removes a saved track and its sidecar. Only ever inside `<library>/music`.
#[tauri::command(async)]
pub fn delete_library_music(state: State<'_, AppState>, path: String) -> AppResult<()> {
    let lib = state.library()?;
    let path = PathBuf::from(path);
    if !path.starts_with(music::music_dir(&lib.root)) {
        return Err(AppError { code: "not_found".into(), message: "That track is not in this library.".into(), detail: None });
    }
    std::fs::remove_file(&path)?;
    let _ = std::fs::remove_file(Path::new(&path).with_extension("json"));
    Ok(())
}
