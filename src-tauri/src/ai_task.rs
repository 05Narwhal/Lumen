//! One mechanism for every interactive AI action (Round 12-6): the frontend makes a task id
//! and passes it to the command; the command wraps its work in an [`AiTask`], which emits
//! `ai://progress` stages and `ai://finished`, and registers a [`CancelToken`] that
//! [`cancel`] trips. Background ML stays on the activity ring.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use photos_jobs::CancelToken;
use serde::Serialize;
use tauri::{AppHandle, Emitter};
use ts_rs::TS;

use crate::events::{AI_FINISHED, AI_PROGRESS};

/// `total` 0 means unknown: the button shows a shimmer instead of a ring.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AiProgress {
    pub task: String,
    pub feature: String,
    pub stage: String,
    pub done: u32,
    pub total: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AiFinished {
    pub task: String,
    pub feature: String,
    pub ok: bool,
    pub cancelled: bool,
}

pub trait Emit {
    fn progress(&self, p: AiProgress);
    fn finished(&self, f: AiFinished);
}

impl Emit for AppHandle {
    fn progress(&self, p: AiProgress) {
        let _ = self.emit(AI_PROGRESS, p);
    }
    fn finished(&self, f: AiFinished) {
        let _ = self.emit(AI_FINISHED, f);
    }
}

fn registry() -> &'static Mutex<HashMap<String, CancelToken>> {
    static TASKS: OnceLock<Mutex<HashMap<String, CancelToken>>> = OnceLock::new();
    TASKS.get_or_init(Default::default)
}

pub struct AiTask<E: Emit> {
    id: Option<String>,
    feature: String,
    emit: E,
    cancel: CancelToken,
    finished: bool,
}

impl<E: Emit> AiTask<E> {
    /// `None` (an old caller that sends no id) means no events and no cancel.
    pub fn start(emit: E, id: Option<String>, feature: &str) -> Self {
        let cancel = CancelToken::new();
        if let Some(id) = &id {
            registry().lock().unwrap().insert(id.clone(), cancel.clone());
        }
        Self { id, feature: feature.to_owned(), emit, cancel, finished: false }
    }

    pub fn stage(&self, stage: &str, done: u32, total: u32) {
        if let Some(task) = &self.id {
            self.emit.progress(AiProgress {
                task: task.clone(),
                feature: self.feature.clone(),
                stage: stage.to_owned(),
                done,
                total,
            });
        }
    }

    pub fn cancelled(&self) -> bool {
        self.cancel.is_cancelled()
    }

    /// The token, for work that polls a `CancelToken` itself.
    pub fn token(&self) -> &CancelToken {
        &self.cancel
    }

    pub fn finish(mut self, ok: bool) {
        self.end(ok);
    }

    fn end(&mut self, ok: bool) {
        if self.finished {
            return;
        }
        self.finished = true;
        if let Some(task) = &self.id {
            registry().lock().unwrap().remove(task);
            self.emit.finished(AiFinished {
                task: task.clone(),
                feature: self.feature.clone(),
                ok,
                cancelled: self.cancel.is_cancelled(),
            });
        }
    }
}

impl<E: Emit> Drop for AiTask<E> {
    /// A command that returns early or panics still lets the button go back to idle.
    fn drop(&mut self) {
        self.end(false);
    }
}

/// Trips the task's token; `false` when the id is unknown or already finished.
pub fn cancel(id: &str) -> bool {
    match registry().lock().unwrap().get(id) {
        Some(token) => {
            token.cancel();
            true
        }
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[derive(Debug, Clone, PartialEq)]
    enum Rec {
        Progress(AiProgress),
        Finished(AiFinished),
    }

    #[derive(Clone, Default)]
    struct Recorder(Arc<Mutex<Vec<Rec>>>);

    impl Recorder {
        fn all(&self) -> Vec<Rec> {
            self.0.lock().unwrap().clone()
        }
    }

    impl Emit for Recorder {
        fn progress(&self, p: AiProgress) {
            self.0.lock().unwrap().push(Rec::Progress(p));
        }
        fn finished(&self, f: AiFinished) {
            self.0.lock().unwrap().push(Rec::Finished(f));
        }
    }

    fn finished(id: &str, ok: bool, cancelled: bool) -> Rec {
        Rec::Finished(AiFinished { task: id.into(), feature: "auto".into(), ok, cancelled })
    }

    #[test]
    fn stages_then_finish_are_recorded_in_order() {
        let rec = Recorder::default();
        let task = AiTask::start(rec.clone(), Some("t-order".into()), "auto");
        task.stage("reading", 0, 0);
        task.stage("trying", 2, 3);
        task.finish(true);
        let all = rec.all();
        assert_eq!(all.len(), 3);
        assert_eq!(
            all[1],
            Rec::Progress(AiProgress {
                task: "t-order".into(),
                feature: "auto".into(),
                stage: "trying".into(),
                done: 2,
                total: 3,
            })
        );
        assert_eq!(all[2], finished("t-order", true, false));
    }

    #[test]
    fn cancel_trips_the_token_and_finish_reports_it() {
        let rec = Recorder::default();
        let task = AiTask::start(rec.clone(), Some("t-cancel".into()), "auto");
        assert!(!task.cancelled());
        assert!(cancel("t-cancel"));
        assert!(task.cancelled());
        task.finish(false);
        assert_eq!(rec.all(), vec![finished("t-cancel", false, true)]);
    }

    #[test]
    fn dropping_without_finish_records_a_failure_once() {
        let rec = Recorder::default();
        drop(AiTask::start(rec.clone(), Some("t-drop".into()), "auto"));
        assert_eq!(rec.all(), vec![finished("t-drop", false, false)]);
    }

    #[test]
    fn finish_unregisters_the_id() {
        let task = AiTask::start(Recorder::default(), Some("t-unreg".into()), "auto");
        task.finish(true);
        assert!(!cancel("t-unreg"));
    }

    #[test]
    fn a_task_without_an_id_is_silent_and_never_cancelled() {
        let rec = Recorder::default();
        let task = AiTask::start(rec.clone(), None, "auto");
        task.stage("reading", 0, 0);
        assert!(!task.cancelled());
        task.finish(true);
        assert!(rec.all().is_empty());
    }

    #[test]
    fn two_tasks_cancel_independently() {
        let a = AiTask::start(Recorder::default(), Some("t-a".into()), "auto");
        let b = AiTask::start(Recorder::default(), Some("t-b".into()), "retouch");
        assert!(cancel("t-a"));
        assert!(a.cancelled());
        assert!(!b.cancelled());
    }
}
