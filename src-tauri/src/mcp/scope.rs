//! What AI assistants may see (Round 16): the photos in Develop's filmstrip, nothing else.
//! The page reports the strip with `mcp_set_scope` whenever it or the open photo changes.

use std::collections::HashSet;
use std::sync::RwLock;

use crate::error::AppError;

#[derive(Debug, Default, Clone, PartialEq)]
pub struct McpScope {
    pub ids: HashSet<i64>,
    /// The strip in its order, for `list_photos`.
    pub order: Vec<i64>,
    /// The photo open in Develop.
    pub open: Option<i64>,
    /// Develop is on screen (its bridge answers edits to the open photo; Round 16 P2).
    pub develop_mounted: bool,
}

impl McpScope {
    pub fn new(order: Vec<i64>, open: Option<i64>) -> Self {
        let mut seen = HashSet::new();
        let order: Vec<i64> = order.into_iter().filter(|id| seen.insert(*id)).collect();
        // The open photo is always in the strip; one that is not is not visible either.
        let open = open.filter(|id| seen.contains(id));
        Self { ids: seen, order, open, develop_mounted: false }
    }
}

pub fn not_in_develop(id: i64) -> AppError {
    AppError {
        code: "not_in_develop".into(),
        message: format!("Photo {id} is not in Develop's filmstrip. Ask the user to open it in Develop."),
        detail: None,
    }
}

#[derive(Default)]
pub struct Scope(RwLock<McpScope>);

impl Scope {
    pub fn set(&self, order: Vec<i64>, open: Option<i64>) {
        let mut s = self.0.write().unwrap_or_else(|p| p.into_inner());
        let mounted = s.develop_mounted;
        *s = McpScope { develop_mounted: mounted, ..McpScope::new(order, open) };
    }

    /// Develop's workspace mounted (`true`) or left (`false`).
    pub fn set_mounted(&self, mounted: bool) {
        self.0.write().unwrap_or_else(|p| p.into_inner()).develop_mounted = mounted;
    }

    pub fn get(&self) -> McpScope {
        self.0.read().unwrap_or_else(|p| p.into_inner()).clone()
    }

    /// `Ok` when `id` is in the strip, else `not_in_develop`.
    pub fn check(&self, id: i64) -> Result<(), AppError> {
        if self.0.read().unwrap_or_else(|p| p.into_inner()).ids.contains(&id) {
            Ok(())
        } else {
            Err(not_in_develop(id))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_scope_is_the_strip_in_order_without_repeats() {
        let s = Scope::default();
        assert_eq!(s.check(1).unwrap_err().code, "not_in_develop", "empty strip, nothing visible");
        s.set(vec![3, 1, 3, 2], Some(1));
        let got = s.get();
        assert_eq!((got.order, got.open), (vec![3, 1, 2], Some(1)));
        assert!(s.check(2).is_ok());
        assert_eq!(s.check(4).unwrap_err().code, "not_in_develop");
        s.set(vec![5], Some(1));
        assert_eq!(s.get().open, None, "an open photo outside the strip is not visible");
    }
}
