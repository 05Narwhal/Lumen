use std::sync::atomic::{AtomicU64, Ordering};

use tauri::State;

use crate::state::AppState;

static NEXT: AtomicU64 = AtomicU64::new(1);

/// Debug only: expose an arbitrary local file at `photos://probe/{id}` to verify Range playback.
#[tauri::command(async)]
pub fn register_probe(state: State<'_, AppState>, path: String) -> u64 {
    let id = NEXT.fetch_add(1, Ordering::Relaxed);
    state.probes.lock().expect("probes lock").insert(id, path.into());
    id
}
