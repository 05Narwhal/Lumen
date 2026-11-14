//! Room on the library's drive (round 3): an import leaves at least [`IMPORT_FREE_BUFFER`]
//! free. Before it starts, the newest files that fit are chosen; while it runs, the ingest
//! loop stops before a file would cross the buffer (another program may be writing too).

use std::sync::Arc;

use crate::source::SourceItem;

/// How much room is free on the library's drive, asked as the import goes. The binary
/// supplies it (it knows the disks); tests supply a number.
#[derive(Clone)]
pub struct FreeSpace(Arc<dyn Fn() -> Option<u64> + Send + Sync>);

impl FreeSpace {
    pub fn new(f: impl Fn() -> Option<u64> + Send + Sync + 'static) -> Self {
        Self(Arc::new(f))
    }

    pub fn free(&self) -> Option<u64> {
        (self.0)()
    }
}

impl std::fmt::Debug for FreeSpace {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("FreeSpace")
    }
}

/// What the history says when an import stops for room.
pub const OUT_OF_ROOM: &str = "Stopped to keep 20 GB free on the library's drive. Free up space and import again to finish.";

/// 20 GB, counted in thousands as drives are sold.
pub const IMPORT_FREE_BUFFER: u64 = 20_000_000_000;

/// The newest items (by modified time, then name) whose sizes add up to at most `budget`
/// bytes. Returns their indexes in the original order and the bytes they take.
pub fn fit_newest_first(items: &[SourceItem], budget: u64) -> (Vec<usize>, u64) {
    let mut order: Vec<usize> = (0..items.len()).collect();
    order.sort_by(|&a, &b| {
        let (x, y) = (&items[a], &items[b]);
        y.modified_at.unwrap_or(i64::MIN).cmp(&x.modified_at.unwrap_or(i64::MIN)).then_with(|| y.rel_path.cmp(&x.rel_path))
    });
    let (mut kept, mut bytes) = (Vec::new(), 0u64);
    for i in order {
        // A file that does not fit is passed over; smaller, older ones may still.
        if bytes + items[i].size <= budget {
            bytes += items[i].size;
            kept.push(i);
        }
    }
    kept.sort_unstable();
    (kept, bytes)
}

/// How many bytes an import may bring in when `free` bytes are free now.
pub fn budget_for(free: u64) -> u64 {
    free.saturating_sub(IMPORT_FREE_BUFFER)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(name: &str, gb: u64, at: i64) -> SourceItem {
        SourceItem { rel_path: name.into(), file_name: name.into(), size: gb * 1_000_000_000, modified_at: Some(at), created_at: None }
    }

    #[test]
    fn the_newest_that_fit_come_first() {
        let items = [item("a", 10, 1), item("b", 10, 5), item("c", 10, 3), item("d", 10, 4), item("e", 10, 2)];
        // 55 GB free keeps 20: 35 GB → the three newest (b, d, c), in their original order.
        let (kept, bytes) = fit_newest_first(&items, budget_for(55_000_000_000));
        assert_eq!(kept, vec![1, 2, 3]);
        assert_eq!(bytes, 30_000_000_000);
        // 15 GB free is already under the buffer: nothing.
        assert_eq!(fit_newest_first(&items, budget_for(15_000_000_000)), (vec![], 0));
        // Everything fits.
        assert_eq!(fit_newest_first(&items, budget_for(500_000_000_000)).0, vec![0, 1, 2, 3, 4]);
    }

    #[test]
    fn a_file_too_big_for_what_is_left_is_skipped_not_the_rest() {
        // Newest first: a 30 GB video does not fit in 25, the next photos still do.
        let items = [item("old", 1, 1), item("video", 30, 9), item("new", 2, 8)];
        let (kept, bytes) = fit_newest_first(&items, 25_000_000_000);
        assert_eq!(kept, vec![0, 2]);
        assert_eq!(bytes, 3_000_000_000);
    }
}
