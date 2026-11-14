//! Duplicates (Round 4D): find sets, keep one and trash the rest, or say they are not
//! duplicates. Every change returns what Undo needs.

use std::cell::RefCell;
use std::collections::HashMap;

use photos_core::duplicates::{self, DuplicateSet, ResolveUndo};
use photos_media::compare;
use serde::Deserialize;
use tauri::{AppHandle, State};
use ts_rs::TS;

use crate::error::AppResult;
use crate::state::AppState;

/// One set to resolve: the photo to keep and the ones to move to the Trash.
#[derive(Debug, Clone, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DuplicateResolution {
    #[ts(type = "number")]
    pub keep: i64,
    #[ts(type = "Array<number>")]
    pub remove: Vec<i64>,
}

/// The finder's look at the files: editors' markers in the originals, and pixel signatures
/// cut from the 256 px thumbnails (each read once per call, only for exact candidates).
pub(crate) fn with_look<T>(root: &std::path::Path, f: impl FnOnce(&duplicates::FileLook) -> T) -> T {
    let originals = root.join("originals");
    let store = photos_media::store::DerivativeStore::new(root);
    // A thumbnail's signature is kept for the session once read (Round 7), so only the first
    // search after launch reads them; a missing thumbnail is asked again next time.
    static SIGNATURES: std::sync::LazyLock<std::sync::Mutex<HashMap<std::path::PathBuf, Vec<u8>>>> = std::sync::LazyLock::new(Default::default);
    let cache: RefCell<HashMap<String, Option<Vec<u8>>>> = RefCell::new(HashMap::new());
    let sig = |hash: &str| -> Option<Vec<u8>> {
        cache
            .borrow_mut()
            .entry(hash.to_string())
            .or_insert_with(|| {
                let path = store.thumb_path(hash, 256);
                if let Some(s) = SIGNATURES.lock().unwrap_or_else(|p| p.into_inner()).get(&path) {
                    return Some(s.clone());
                }
                let s = compare::signature(&path)?;
                SIGNATURES.lock().unwrap_or_else(|p| p.into_inner()).insert(path, s.clone());
                Some(s)
            })
            .clone()
    };
    let pixels = |a: &str, b: &str| Some(compare::difference(&sig(a)?, &sig(b)?));
    f(&duplicates::FileLook { originals: &originals, pixels: &pixels })
}

/// The last sets found, by library and version (Round 5A). Finding them reads thumbnails and
/// took 5–20 s on Julian's library, and Home asks for the count while the Duplicates screen
/// asks for the list: both are answered by one search until the library changes. The lock is
/// held while searching, so a second caller waits for the first answer instead of searching
/// beside it.
#[derive(Default)]
struct SetsCache(std::sync::Mutex<Option<Found>>);

/// (library root, version, its sets)
type Found = (std::path::PathBuf, u64, std::sync::Arc<Vec<DuplicateSet>>);

impl SetsCache {
    fn get(&self, library: &std::path::Path, version: u64, find: impl FnOnce() -> photos_core::Result<Vec<DuplicateSet>>) -> photos_core::Result<std::sync::Arc<Vec<DuplicateSet>>> {
        let mut held = self.0.lock().unwrap_or_else(|p| p.into_inner());
        if let Some((lib, v, sets)) = held.as_ref() {
            if lib == library && *v == version {
                return Ok(sets.clone());
            }
        }
        let sets = std::sync::Arc::new(find()?);
        *held = Some((library.to_path_buf(), version, sets.clone()));
        Ok(sets)
    }

    /// Dismissing changes the answer without changing the library's version.
    fn clear(&self) {
        *self.0.lock().unwrap_or_else(|p| p.into_inner()) = None;
    }
}

static SETS: std::sync::LazyLock<SetsCache> = std::sync::LazyLock::new(SetsCache::default);

fn sets(state: &AppState) -> AppResult<std::sync::Arc<Vec<DuplicateSet>>> {
    let lib = state.library()?;
    let version = state.changes.version.load(std::sync::atomic::Ordering::SeqCst);
    Ok(SETS.get(&lib.root, version, || with_look(&lib.root, |look| duplicates::find_sets_on(&lib.db, Some(look))))?)
}

/// Every set, newest first.
#[tauri::command(async)]
pub fn list_duplicate_sets(state: State<'_, AppState>) -> AppResult<Vec<DuplicateSet>> {
    Ok(sets(&state)?.as_ref().clone())
}

/// `[sets, photos that could go]`, for the Home card.
#[tauri::command(async)]
pub fn count_duplicate_sets(state: State<'_, AppState>) -> AppResult<(usize, usize)> {
    let sets = sets(&state)?;
    Ok((sets.len(), sets.iter().map(|s| s.members.len() - 1).sum()))
}

/// Keeps one photo of each set and trashes the others, after merging what they carried.
#[tauri::command(async)]
pub fn resolve_duplicates(app: AppHandle, state: State<'_, AppState>, sets: Vec<DuplicateResolution>) -> AppResult<Vec<ResolveUndo>> {
    let lib = state.library()?;
    let now = photos_core::now_unix();
    let sets: Vec<(i64, Vec<i64>)> = sets.into_iter().map(|s| (s.keep, s.remove)).collect();
    let undos = lib.db.with_conn(|c| duplicates::resolve_all(c, &sets, now))?;
    state.changes.changed(&app, true);
    Ok(undos)
}

#[tauri::command(async)]
pub fn undo_resolve_duplicates(app: AppHandle, state: State<'_, AppState>, undos: Vec<ResolveUndo>) -> AppResult<()> {
    state.library()?.db.with_conn(|c| duplicates::undo_resolve(c, &undos))?;
    state.changes.changed(&app, true);
    Ok(())
}

/// *Not duplicates*: the photos of a set stay apart from now on.
#[tauri::command(async)]
pub fn dismiss_duplicates(state: State<'_, AppState>, ids: Vec<i64>) -> AppResult<()> {
    state.library()?.db.with_conn(|c| duplicates::dismiss(c, &ids, photos_core::now_unix()))?;
    SETS.clear();
    Ok(())
}

#[tauri::command(async)]
pub fn undo_dismiss_duplicates(state: State<'_, AppState>, ids: Vec<i64>) -> AppResult<()> {
    state.library()?.db.with_conn(|c| duplicates::undismiss(c, &ids))?;
    SETS.clear();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_sets_are_found_once_per_library_version() {
        let cache = SetsCache::default();
        let runs = std::cell::Cell::new(0);
        let find = || {
            runs.set(runs.get() + 1);
            Ok(Vec::new())
        };
        let lib = std::path::Path::new("/lib");
        cache.get(lib, 1, find).unwrap();
        cache.get(lib, 1, find).unwrap();
        assert_eq!(runs.get(), 1, "the list and the Home count share one search");
        cache.get(lib, 2, find).unwrap();
        assert_eq!(runs.get(), 2, "a change to the library finds them again");
        cache.get(std::path::Path::new("/other"), 2, find).unwrap();
        assert_eq!(runs.get(), 3, "another library is another answer");
        cache.clear();
        cache.get(std::path::Path::new("/other"), 2, find).unwrap();
        assert_eq!(runs.get(), 4, "a dismissal does not bump the version, so it clears");
        // A failure is not remembered.
        let failing = || Err(photos_core::CoreError::Invalid("no".into()));
        assert!(cache.get(lib, 9, failing).is_err());
        cache.get(lib, 9, find).unwrap();
        assert_eq!(runs.get(), 5);
    }

    /// Round 4D, Task 5: how far apart the pixels of every *Exact* candidate pair are, on a
    /// COPY of a library's database (`LUMEN_DB`), reading only the thumbnails under
    /// `LUMEN_LIBRARY`. Prints each pair's difference and the sets with and without the check,
    /// and writes a contact sheet of the pairs between 2 and 20 to `LUMEN_SHEET` (a PNG path).
    ///
    /// ```sh
    /// LUMEN_DB=<copy> LUMEN_LIBRARY=<library> LUMEN_SHEET=<png> cargo test --release -p lumen --lib exact_pixels_on_a_real_library -- --ignored --nocapture
    /// ```
    #[test]
    #[ignore = "needs a copy of a real library.db"]
    fn exact_pixels_on_a_real_library() {
        let env = |k: &str| std::env::var(k).unwrap_or_else(|_| panic!("{k}"));
        let db = photos_core::db::Db::open(std::path::Path::new(&env("LUMEN_DB"))).unwrap();
        let root = std::path::PathBuf::from(env("LUMEN_LIBRARY"));
        let store = photos_media::store::DerivativeStore::new(&root);
        let originals = root.join("originals");
        let asked: RefCell<Vec<(String, String, f32)>> = RefCell::new(Vec::new());
        let cache: RefCell<HashMap<String, Option<Vec<u8>>>> = RefCell::new(HashMap::new());
        let sig = |h: &str| cache.borrow_mut().entry(h.to_string()).or_insert_with(|| compare::signature(&store.thumb_path(h, 256))).clone();
        let pixels = |a: &str, b: &str| {
            let d = compare::difference(&sig(a)?, &sig(b)?);
            asked.borrow_mut().push((a.to_string(), b.to_string(), d));
            Some(d)
        };
        let blind = duplicates::FileLook { originals: &originals, pixels: &|_, _| None };
        let look = duplicates::FileLook { originals: &originals, pixels: &pixels };
        let before = db.with_conn(|c| duplicates::find_sets_in(c, Some(&blind))).unwrap();
        let after = db.with_conn(|c| duplicates::find_sets_in(c, Some(&look))).unwrap();
        let exact = |s: &[DuplicateSet]| s.iter().filter(|s| s.likeness == duplicates::Likeness::Exact).count();
        println!("sets {} → {}, exact {} → {}", before.len(), after.len(), exact(&before), exact(&after));
        let mut pairs = asked.into_inner();
        pairs.sort_by(|a, b| a.2.total_cmp(&b.2));
        pairs.dedup_by(|a, b| a.0 == b.0 && a.1 == b.1);
        let buckets = [1.0, 2.0, 3.0, 4.0, 6.0, 8.0, 12.0, 20.0, 40.0, 255.0];
        for w in buckets.windows(2) {
            println!("  {:>5.1}–{:<5.1} {}", w[0], w[1], pairs.iter().filter(|p| p.2 >= w[0] && p.2 < w[1]).count());
        }
        println!("  below 1: {}", pairs.iter().filter(|p| p.2 < 1.0).count());
        // A contact sheet: each pair side by side at 160 px, with its difference in the order shown.
        if let Ok(out) = std::env::var("LUMEN_SHEET") {
            let band: Vec<_> = pairs.iter().filter(|p| p.2 >= 2.0 && p.2 < 20.0).take(60).collect();
            let (cell, cols) = (160u32, 4u32);
            let rows = (band.len() as u32).div_ceil(cols).max(1);
            let mut sheet = image::RgbImage::from_pixel(cols * cell * 2 + cols * 8, rows * (cell + 8), image::Rgb([30, 30, 30]));
            for (n, (a, b, d)) in band.iter().enumerate() {
                println!("  sheet #{n}: {d:.2} ({a} / {b})");
                let (cx, cy) = ((n as u32 % cols) * (cell * 2 + 8), (n as u32 / cols) * (cell + 8));
                for (k, h) in [a, b].iter().enumerate() {
                    if let Ok(img) = image::open(store.thumb_path(h, 256)) {
                        let t = img.resize(cell, cell, image::imageops::FilterType::Triangle).to_rgb8();
                        image::imageops::overlay(&mut sheet, &t, (cx + k as u32 * cell) as i64, cy as i64);
                    }
                }
            }
            sheet.save(&out).unwrap();
            println!("contact sheet: {out}");
        }
    }
}
