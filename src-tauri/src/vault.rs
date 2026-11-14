//! Whether the Hidden folder is open (round 3). The state lives here, in Rust, so a locked
//! folder's grid and its photos' bytes are refused whatever the webview asks for. It locks
//! again after five minutes without use, when the app starts, and when the user leaves it.

use std::sync::Mutex;
use std::time::{Duration, Instant};

/// How long the folder stays open without being used.
pub const IDLE: Duration = Duration::from_secs(5 * 60);

/// How long *Move to Hidden* can be undone from its toast without the password (the toast
/// shows for six seconds).
pub const UNDO_WINDOW: Duration = Duration::from_secs(10);

#[derive(Default)]
pub struct VaultLock {
    open_until: Mutex<Option<Instant>>,
    /// What was last moved in, and when, so its toast can undo it.
    just_moved: Mutex<Option<(Vec<i64>, Instant)>>,
}

impl VaultLock {
    pub fn unlock(&self) {
        self.unlock_at(Instant::now());
    }

    fn unlock_at(&self, now: Instant) {
        *self.open_until.lock().expect("vault lock") = Some(now + IDLE);
    }

    pub fn lock(&self) {
        *self.open_until.lock().expect("vault lock") = None;
    }

    pub fn is_unlocked(&self) -> bool {
        self.is_unlocked_at(Instant::now())
    }

    fn is_unlocked_at(&self, now: Instant) -> bool {
        self.open_until.lock().expect("vault lock").is_some_and(|t| now < t)
    }

    pub fn remember_moved(&self, ids: &[i64]) {
        self.remember_moved_at(ids, Instant::now());
    }

    fn remember_moved_at(&self, ids: &[i64], now: Instant) {
        *self.just_moved.lock().expect("vault lock") = Some((ids.to_vec(), now));
    }

    /// Whether moving `ids` out is the undo of the last move in, still within its window.
    pub fn may_undo(&self, ids: &[i64]) -> bool {
        self.may_undo_at(ids, Instant::now())
    }

    fn may_undo_at(&self, ids: &[i64], now: Instant) -> bool {
        let moved = self.just_moved.lock().expect("vault lock");
        !ids.is_empty()
            && moved.as_ref().is_some_and(|(list, at)| now.duration_since(*at) < UNDO_WINDOW && ids.iter().all(|i| list.contains(i)))
    }

    /// Being used keeps it open: every grid page and photo asked for moves the deadline.
    pub fn touch(&self) {
        self.touch_at(Instant::now());
    }

    fn touch_at(&self, now: Instant) {
        let mut until = self.open_until.lock().expect("vault lock");
        if until.is_some_and(|t| now < t) {
            *until = Some(now + IDLE);
        } else {
            *until = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_starts_locked_and_locks_itself_after_idling() {
        let v = VaultLock::default();
        let t0 = Instant::now();
        assert!(!v.is_unlocked_at(t0));
        v.unlock_at(t0);
        assert!(v.is_unlocked_at(t0 + Duration::from_secs(60)));
        // Use keeps it open past the first deadline…
        v.touch_at(t0 + Duration::from_secs(240));
        assert!(v.is_unlocked_at(t0 + Duration::from_secs(400)));
        // …and idling closes it.
        assert!(!v.is_unlocked_at(t0 + Duration::from_secs(240) + IDLE));
        // A touch after the deadline does not reopen it.
        v.touch_at(t0 + Duration::from_secs(10_000));
        assert!(!v.is_unlocked_at(t0 + Duration::from_secs(10_001)));
        v.unlock_at(t0);
        v.lock();
        assert!(!v.is_unlocked_at(t0));
    }

    /// The toast's Undo after *Move to Hidden* must work without the password, but only for
    /// exactly what was just moved.
    #[test]
    fn a_move_in_can_be_undone_briefly_without_the_password() {
        let v = VaultLock::default();
        let t0 = Instant::now();
        v.remember_moved_at(&[3, 4, 9], t0);
        assert!(v.may_undo_at(&[4, 9], t0 + Duration::from_secs(5)));
        assert!(!v.may_undo_at(&[4, 10], t0 + Duration::from_secs(5)), "only what was moved");
        assert!(!v.may_undo_at(&[4], t0 + UNDO_WINDOW), "only briefly");
        assert!(!v.may_undo_at(&[], t0), "nothing is not an undo");
    }
}
