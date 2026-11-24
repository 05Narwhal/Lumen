//! The photos a tool call from the app works on (Round 25 P4b-2b). A grid or the viewer runs a
//! plugin's `tool:` action on the photos its menu was opened on; for that one call the host's
//! `library` interface reads those as the *selection*. Without them the selection is what the app
//! reports (Develop's filmstrip), as before.

use std::sync::Mutex;

/// The selection of the tool call that runs now, if the app named one.
#[derive(Default)]
pub struct ToolSelection(Mutex<Option<Vec<i64>>>);

/// Clears the selection when the call ends, however it ends.
struct Clear<'a>(&'a ToolSelection);

impl Drop for Clear<'_> {
    fn drop(&mut self) {
        *self.0 .0.lock().unwrap_or_else(|p| p.into_inner()) = None;
    }
}

impl ToolSelection {
    /// The photos the running call was given, in the order they were given.
    pub fn current(&self) -> Option<Vec<i64>> {
        self.0.lock().unwrap_or_else(|p| p.into_inner()).clone()
    }

    /// Runs `f` with `ids` as the selection.
    pub fn scoped<T>(&self, ids: Vec<i64>, f: impl FnOnce() -> T) -> T {
        *self.0.lock().unwrap_or_else(|p| p.into_inner()) = Some(ids);
        let _clear = Clear(self);
        f()
    }
}

/// The photo ids a `{"scope":"selection","assets":[…]}` argument names; `None` for any other
/// scope or when there are none (the call then uses the app's own selection).
pub fn assets_of(args: &serde_json::Value) -> Option<Vec<i64>> {
    if args.get("scope")?.as_str()? != "selection" {
        return None;
    }
    let ids: Vec<i64> = args.get("assets")?.as_array()?.iter().filter_map(|v| v.as_i64()).collect();
    (!ids.is_empty()).then_some(ids)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_selection_lasts_for_the_call_and_not_after() {
        let s = ToolSelection::default();
        assert_eq!(s.current(), None);
        let inside = s.scoped(vec![3, 1, 2], || s.current());
        assert_eq!(inside, Some(vec![3, 1, 2]));
        assert_eq!(s.current(), None);
    }

    #[test]
    fn a_call_that_panics_leaves_no_selection_behind() {
        let s = ToolSelection::default();
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| s.scoped(vec![1], || panic!("trap"))));
        assert!(r.is_err());
        assert_eq!(s.current(), None);
    }

    #[test]
    fn only_a_selection_scope_with_ids_names_photos() {
        assert_eq!(assets_of(&json!({"scope":"selection","assets":[5,6]})), Some(vec![5, 6]));
        assert_eq!(assets_of(&json!({"scope":"selection"})), None);
        assert_eq!(assets_of(&json!({"scope":"selection","assets":[]})), None);
        assert_eq!(assets_of(&json!({"scope":"library","assets":[5]})), None);
        assert_eq!(assets_of(&json!({"scope":"album","album":3})), None);
        assert_eq!(assets_of(&json!("selection")), None);
        assert_eq!(assets_of(&json!({"scope":"selection","assets":["x",7]})), Some(vec![7]));
    }
}
