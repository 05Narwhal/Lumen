//! The face radar in the app (Round 9A, reworked in Round 10): every correction seeds it, *Not
//! Ruta* included, and half a second after the last one each person it touched gets one sweep
//! (`photos_core::people::radar`) over the photos the user is looking at: the grid the viewer
//! was opened from, filter and all, nearest to the open photo first. Without one (a correction
//! on the People page) it checks the photos taken around the correction.
//!
//! Sweeps run on their own thread, not a job lane: Round 9A's waited on the Maintenance lane
//! behind background video prep, sometimes for minutes, so the user never saw one.
//! What moved is announced on `faces://radar` with its Undo: *Fixed 7 more photos of Anna*.

use std::collections::BTreeMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use photos_core::layout::GroupBy;
use photos_core::people::{radar, Thresholds};
use photos_core::scope::LayoutScope;
use tauri::{AppHandle, Emitter, Manager, State};

use crate::error::AppResult;
use crate::state::{AppState, LayoutKey};

/// How long after the last correction the sweeps start: corrections in a row make one.
const QUIET: Duration = Duration::from_millis(500);

/// Corrections waiting for their sweep: person → capture times.
#[derive(Debug, Default)]
pub struct Seeds {
    by: BTreeMap<i64, Vec<i64>>,
    last: Option<Instant>,
}

impl Seeds {
    /// `at` is the corrected face's capture time, when it has one.
    pub fn add(&mut self, person: i64, at: Option<i64>, now: Instant) {
        let times = self.by.entry(person).or_default();
        if let Some(at) = at.filter(|at| !times.contains(at)) {
            times.push(at);
            times.sort();
        }
        self.last = Some(now);
    }

    /// Every person's seeds once `QUIET` has passed since the last; taken.
    pub fn due(&mut self, now: Instant) -> Option<Vec<(i64, Vec<i64>)>> {
        let last = self.last?;
        if now.duration_since(last) < QUIET {
            return None;
        }
        self.last = None;
        Some(std::mem::take(&mut self.by).into_iter().collect())
    }

    fn waiting(&self) -> bool {
        self.last.is_some()
    }
}

static SEEDS: Mutex<Option<Seeds>> = Mutex::new(None);

/// What the user is looking at: the grid's layout and the open photo.
static VIEW: Mutex<Option<(LayoutKey, Option<i64>)>> = Mutex::new(None);

/// The viewer says which grid it pages through and which photo is open; `None` when it closes.
#[tauri::command(async)]
#[allow(clippy::too_many_arguments)]
pub fn set_face_radar_view(width: Option<u32>, row_height: Option<u32>, gap: Option<u32>, scope: Option<LayoutScope>, group_by: Option<GroupBy>, at: Option<i64>, open: bool) -> AppResult<()> {
    let view = match (open, width, row_height, gap) {
        (true, Some(width), Some(row_height), Some(gap)) => Some((LayoutKey { width, row_height, gap, scope: scope.unwrap_or_default(), group_by: group_by.unwrap_or_default() }, at)),
        _ => None,
    };
    *VIEW.lock().unwrap_or_else(|e| e.into_inner()) = view;
    Ok(())
}

/// The user corrected these faces, touching `people` (who a face went to, and who it was
/// taken from): check the photos around them again for each.
pub fn seed(app: &AppHandle, people: &[Option<i64>], faces: &[i64]) {
    let Ok(lib) = app.state::<AppState>().library() else { return };
    let mut people: Vec<i64> = people.iter().flatten().copied().collect();
    people.sort();
    people.dedup();
    if people.is_empty() {
        return;
    }
    let times: Vec<Option<i64>> = faces
        .iter()
        .map(|f| {
            lib.db
                .with_conn(|c| Ok(c.query_row("SELECT a.captured_at FROM faces f JOIN assets a ON a.id = f.asset_id WHERE f.id = ?1", [f], |r| r.get::<_, Option<i64>>(0))?))
                .ok()
                .flatten()
        })
        .collect();
    let mut guard = SEEDS.lock().unwrap_or_else(|e| e.into_inner());
    let seeds = guard.get_or_insert_with(Seeds::default);
    let start = !seeds.waiting();
    let now = Instant::now();
    for &p in &people {
        seeds.add(p, None, now);
        for &at in &times {
            seeds.add(p, at, now);
        }
    }
    drop(guard);
    if start {
        let app = app.clone();
        let _ = std::thread::Builder::new().name("lumen-face-radar".into()).spawn(move || loop {
            std::thread::sleep(Duration::from_millis(100));
            let due = SEEDS.lock().unwrap_or_else(|e| e.into_inner()).as_mut().and_then(|s| s.due(Instant::now()));
            if let Some(due) = due {
                for (person, times) in due {
                    run(&app, person, &times);
                }
                break;
            }
        });
    }
}

/// The photos of the view, in its order, when one is open.
fn view_ids(state: &State<'_, AppState>) -> Option<(Vec<i64>, Option<i64>)> {
    let (key, at) = VIEW.lock().unwrap_or_else(|e| e.into_inner()).clone()?;
    let layout = state.layout_for(key).ok()?;
    Some((layout.ids(), at))
}

/// One sweep for `person`, announced when it moved anything.
fn run(app: &AppHandle, person: i64, times: &[i64]) {
    let state = app.state::<AppState>();
    let Ok(lib) = state.library() else { return };
    let started = Instant::now();
    let view = view_ids(&state);
    let scope = match &view {
        Some((ids, at)) => radar::Scope::View { ids, at: *at },
        None if !times.is_empty() => radar::Scope::Around(times),
        None => return,
    };
    let undo = match radar::sweep(&lib.db, person, scope, &Thresholds::MEASURED) {
        Ok(u) => u,
        Err(e) => {
            tracing::warn!(person, "face radar: {e}");
            return;
        }
    };
    tracing::info!(person, added = undo.added, removed = undo.removed, in_view = view.is_some(), ms = started.elapsed().as_millis() as u64, "face radar");
    if undo.moves.is_empty() {
        return;
    }
    let name = lib.db.with_conn(|c| Ok(c.query_row("SELECT name FROM people WHERE id = ?1", [person], |r| r.get::<_, Option<String>>(0))?)).ok().flatten();
    state.changes.changed(app, true);
    let _ = app.emit(crate::events::FACES_RADAR, crate::events::FacesRadar { person, name, photos: undo.photos() as u32, undo });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    #[test]
    fn corrections_in_a_row_make_one_sweep_per_person() {
        let t0 = Instant::now();
        let ms = Duration::from_millis;
        let mut s = Seeds::default();
        s.add(1, Some(100), t0);
        s.add(2, Some(500), t0 + ms(200));
        s.add(1, Some(900), t0 + ms(400));
        s.add(1, Some(100), t0 + ms(400));
        s.add(3, None, t0 + ms(400));
        assert_eq!(s.due(t0 + ms(800)), None, "half a second after the last, not the first");
        let due = s.due(t0 + ms(950)).unwrap();
        assert_eq!(due, vec![(1, vec![100, 900]), (2, vec![500]), (3, vec![])]);
        assert_eq!(s.due(t0 + ms(2000)), None, "taken once");
    }
}
