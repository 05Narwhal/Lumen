//! What the app is doing in the background, in words a person understands (Round 9B): the
//! activity ring lists these, never lanes, queues or workers.
//!
//! A task calls [`start`] with a stable id and a label ("Preparing videos"), says how much
//! there is ([`Activity::add_total`]) and how far it got ([`Activity::advance`]), and drops
//! the handle when it is done. `done` and `total` only grow while it runs, so its progress
//! never jumps back to zero; new work adds to the total. Starting an id that is already
//! running joins it rather than starting over.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{LazyLock, Mutex};

/// One line of the activity ring.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActivityItem {
    pub id: String,
    pub label: String,
    pub done: u64,
    pub total: u64,
    /// A short second line ("Not enough space to prepare videos").
    pub detail: Option<String>,
}

struct Entry {
    item: ActivityItem,
    /// Handles holding it: the line ends when the last one is dropped.
    holders: usize,
}

static REGISTRY: LazyLock<Mutex<BTreeMap<String, Entry>>> = LazyLock::new(Default::default);
/// Bumped on every change, so the ticker sends only when something moved.
static VERSION: AtomicU64 = AtomicU64::new(0);

/// A running activity. Dropping the last handle for its id ends it.
#[derive(Debug)]
pub struct Activity {
    id: String,
}

/// Starts `id` labelled `label` with `total` to do, or joins it if it is already running
/// (its total grows by `total`; its label is kept).
pub fn start(id: &str, label: &str, total: u64) -> Activity {
    with(|reg| {
        reg.entry(id.to_string())
            .and_modify(|e| {
                e.holders += 1;
                e.item.total += total;
            })
            .or_insert_with(|| Entry { item: ActivityItem { id: id.into(), label: label.into(), done: 0, total, detail: None }, holders: 1 });
    });
    Activity { id: id.to_string() }
}

impl Activity {
    /// More to do (newly imported photos for a scan already running).
    pub fn add_total(&self, n: u64) {
        self.update(|i| i.total += n);
    }

    /// `n` more done. A task that does more than it said raises its total with it, so the
    /// line never reads past 100 %.
    pub fn advance(&self, n: u64) {
        self.update(|i| {
            i.done += n;
            i.total = i.total.max(i.done);
        });
    }

    /// Sets the second line, or clears it.
    pub fn set_detail(&self, detail: Option<&str>) {
        self.update(|i| i.detail = detail.map(str::to_string));
    }

    /// Renames the line ("Preparing videos" → "Preparing videos · paused for space").
    pub fn set_label(&self, label: &str) {
        self.update(|i| i.label = label.to_string());
    }

    fn update(&self, f: impl FnOnce(&mut ActivityItem)) {
        with(|reg| {
            if let Some(e) = reg.get_mut(&self.id) {
                f(&mut e.item);
            }
        });
    }
}

impl Drop for Activity {
    fn drop(&mut self) {
        with(|reg| {
            if let Some(e) = reg.get_mut(&self.id) {
                e.holders -= 1;
                if e.holders == 0 {
                    reg.remove(&self.id);
                }
            }
        });
    }
}

fn with(f: impl FnOnce(&mut BTreeMap<String, Entry>)) {
    let mut reg = REGISTRY.lock().unwrap_or_else(|p| p.into_inner());
    f(&mut reg);
    VERSION.fetch_add(1, Ordering::Relaxed);
}

/// Every running activity, by id, and a version that changes whenever any of them does.
pub fn snapshot() -> (u64, Vec<ActivityItem>) {
    let reg = REGISTRY.lock().unwrap_or_else(|p| p.into_inner());
    (VERSION.load(Ordering::Relaxed), reg.values().map(|e| e.item.clone()).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn find(id: &str) -> Option<ActivityItem> {
        snapshot().1.into_iter().find(|i| i.id == id)
    }

    #[test]
    fn an_activity_counts_up_and_ends_when_dropped() {
        let a = start("test:videos", "Preparing videos", 10);
        a.advance(3);
        assert_eq!(find("test:videos").map(|i| (i.done, i.total)), Some((3, 10)));
        // More work arrives: the total grows, what is done stays done.
        a.add_total(5);
        a.advance(1);
        assert_eq!(find("test:videos").map(|i| (i.done, i.total)), Some((4, 15)));
        drop(a);
        assert_eq!(find("test:videos"), None);
    }

    #[test]
    fn starting_a_running_id_joins_it() {
        let a = start("test:scan", "Looking for faces", 4);
        a.advance(2);
        let b = start("test:scan", "ignored", 6);
        let item = find("test:scan").unwrap();
        assert_eq!((item.label.as_str(), item.done, item.total), ("Looking for faces", 2, 10));
        drop(a);
        assert!(find("test:scan").is_some(), "still held by the second");
        drop(b);
        assert!(find("test:scan").is_none());
    }

    #[test]
    fn the_version_moves_with_every_change() {
        let (v0, _) = snapshot();
        let a = start("test:version", "x", 1);
        let (v1, _) = snapshot();
        a.advance(1);
        let (v2, _) = snapshot();
        assert!(v1 > v0 && v2 > v1);
    }
}
