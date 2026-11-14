//! Emits `jobs://update` while the scheduler is busy, `activity://update` when a named
//! activity moves (Round 9B), and flushes held-back `library://changed` events. One thread,
//! 250 ms period.

use std::time::Duration;

use tauri::{AppHandle, Emitter, Manager};

use crate::events::{ActivityLine, ActivityUpdate, JobsUpdate, ACTIVITY_UPDATE, JOBS_UPDATE};
use crate::state::AppState;

pub fn start(app: AppHandle) {
    std::thread::Builder::new()
        .name("lumen-jobs-ticker".into())
        .spawn(move || {
            let mut last: Option<JobsUpdate> = None;
            let mut activity_version = 0u64;
            loop {
                std::thread::sleep(Duration::from_millis(250));
                let state = app.state::<AppState>();
                let s = state.jobs.stats();
                let update = JobsUpdate {
                    running: s.running as u32,
                    queued: s.queued.iter().sum::<usize>() as u32,
                    interactive: s.queued[0] as u32,
                    prefetch: s.queued[1] as u32,
                    import: s.queued[2] as u32,
                    maintenance: s.queued[3] as u32,
                    workers: s.workers as u32,
                };
                let changed = last.as_ref().is_none_or(|l| {
                    l.running != update.running
                        || l.queued != update.queued
                        || l.interactive != update.interactive
                        || l.prefetch != update.prefetch
                        || l.import != update.import
                        || l.maintenance != update.maintenance
                });
                if changed {
                    let _ = app.emit(JOBS_UPDATE, &update);
                    last = Some(update);
                }
                let (version, items) = photos_jobs::activity::snapshot();
                if version != activity_version {
                    activity_version = version;
                    let _ = app.emit(ACTIVITY_UPDATE, &ActivityUpdate { items: items.into_iter().map(ActivityLine::from).collect() });
                }
                state.changes.flush(&app);
            }
        })
        .expect("spawn jobs ticker");
}
