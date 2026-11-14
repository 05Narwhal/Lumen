//! Round 12-6: cancel for any interactive AI action started with a task id.

use crate::ai_task;
use crate::error::AppResult;

/// `false` when the task is unknown or already finished (a late click is harmless).
#[tauri::command(async)]
pub fn ai_cancel(task: String) -> AppResult<bool> {
    Ok(ai_task::cancel(&task))
}
