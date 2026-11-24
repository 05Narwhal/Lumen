//! The review screen's proposals (Round 25 P4b, migration 0041): what plugins proposed for
//! photos, a page at a time, and the person's *Reject* and its undo. *Accept* writes through the
//! app's own paths, to Lumen's records only; nothing here touches an original.

use photos_core::proposals::{self, ProposalCount, ProposalStatus};
use photos_core::proposals_accept::{self, AcceptItem, AcceptReport, AcceptReversal, AcceptUndo, ProposalReviewPage};
use tauri::{AppHandle, State};

use crate::error::{AppError, AppResult};
use crate::state::AppState;

const PAGE: usize = 200;

fn known_kind(kind: &str) -> AppResult<()> {
    if proposals::KINDS.contains(&kind) {
        Ok(())
    } else {
        Err(AppError { code: "unknown_kind".into(), message: format!("Lumen has no review for \"{kind}\""), detail: None })
    }
}

/// One kind's proposals in `status` (pending by default), oldest first, optionally from one plugin,
/// each with the photo's file name and what it has now.
#[tauri::command(async)]
pub fn proposals_list(
    state: State<'_, AppState>,
    kind: String,
    status: Option<ProposalStatus>,
    plugin_id: Option<String>,
    cursor: Option<i64>,
) -> AppResult<ProposalReviewPage> {
    known_kind(&kind)?;
    let lib = state.library()?;
    let status = status.unwrap_or(ProposalStatus::Pending);
    Ok(lib.db.with_conn(|c| proposals_accept::review_page(c, &kind, status, plugin_id.as_deref(), cursor, PAGE))?)
}

/// How many proposals of each kind wait for review.
#[tauri::command(async)]
pub fn proposals_counts(state: State<'_, AppState>) -> AppResult<Vec<ProposalCount>> {
    let lib = state.library()?;
    Ok(lib.db.with_conn(proposals::pending_counts)?)
}

/// *Reject*: the proposals leave the review; a plugin proposing the same value again doesn't
/// bring them back. Returns how many changed.
#[tauri::command(async)]
pub fn proposals_reject(state: State<'_, AppState>, ids: Vec<i64>) -> AppResult<usize> {
    let lib = state.library()?;
    let now = chrono::Utc::now().timestamp();
    Ok(lib.db.with_conn(|c| proposals::set_status(c, &ids, ProposalStatus::Rejected, now))?)
}

/// AcceptUndo of *Reject*: the proposals are pending again.
#[tauri::command(async)]
pub fn proposals_restore(state: State<'_, AppState>, ids: Vec<i64>) -> AppResult<usize> {
    let lib = state.library()?;
    let now = chrono::Utc::now().timestamp();
    Ok(lib.db.with_conn(|c| proposals::set_status(c, &ids, ProposalStatus::Pending, now))?)
}

/// *Accept*: each proposal (or the value the person edited it to) is written through the app's
/// own paths, to Lumen's records only (`photos-core::proposals_accept`). The answer carries what
/// was accepted, what was skipped and why, and the undo for the 6 s toast.
#[tauri::command(async)]
pub fn proposals_accept(app: AppHandle, state: State<'_, AppState>, items: Vec<AcceptItem>) -> AppResult<AcceptReport> {
    let lib = state.library()?;
    let now = chrono::Utc::now().timestamp();
    let report = lib.db.with_conn(|c| proposals_accept::accept(c, &items, now))?;
    if !report.accepted.is_empty() {
        state.changes.changed(&app, true);
        if report.undo.reversals.iter().any(|r| matches!(r, AcceptReversal::Face { .. })) {
            crate::faces::schedule_recluster(&app);
        }
    }
    Ok(report)
}

/// AcceptUndo of *Accept*: the photos are as they were and the proposals are pending again.
#[tauri::command(async)]
pub fn proposals_undo_accept(app: AppHandle, state: State<'_, AppState>, undo: AcceptUndo) -> AppResult<()> {
    let lib = state.library()?;
    let now = chrono::Utc::now().timestamp();
    lib.db.with_conn(|c| proposals_accept::undo(c, &undo, now))?;
    if undo.reversals.iter().any(|r| matches!(r, AcceptReversal::Face { .. })) {
        crate::faces::schedule_recluster(&app);
    }
    state.changes.changed(&app, true);
    Ok(())
}
