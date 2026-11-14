//! Google Takeout importer. The logic is plain Rust over [`fs::Fs`] (tested natively on synthetic
//! Takeouts); only the entry points below are wasm32-only.

pub mod fs;
pub mod names;
pub mod scan;
pub mod zip;

#[cfg(target_arch = "wasm32")]
mod chosen_fs;

#[cfg(target_arch = "wasm32")]
mod plugin {
    use crate::chosen_fs::{ticking, ChosenFs};
    use crate::scan::{scan, ScanExtras};
    use crate::zip::{read_index_ticking, OverlayFs, Streams, ZipFs, ZipSet};
    use std::rc::Rc;
    use lumen_plugin::{chosen, log, setting, Extras, Importer, Item};
    use std::cell::RefCell;
    use std::collections::HashMap;

    thread_local! {
        /// What the last `enumerate` found, so `extras` costs a lookup, not a second walk.
        static FOUND: RefCell<Option<HashMap<String, ScanExtras>>> = const { RefCell::new(None) };
    }

    thread_local! {
        /// Range reads over the zips of the last `enumerate`.
        static STREAMS: RefCell<Option<Streams>> = const { RefCell::new(None) };
    }

    /// Indexes every `*.zip` in the chosen folder (its root only), in name order.
    fn index_zips() -> Rc<ZipSet> {
        let mut names: Vec<(String, u64)> = chosen::list("")
            .unwrap_or_default()
            .into_iter()
            .filter(|e| !e.is_dir && e.name.to_lowercase().ends_with(".zip"))
            .map(|e| (e.name, e.size))
            .collect();
        names.sort();
        let mut zips = Vec::new();
        for (i, (name, size)) in names.iter().enumerate() {
            let _ = chosen::progress(i as u64, Some(names.len() as u64));
            let n = name.clone();
            // A cut-off zip is walked header by header: report it so the host can show and stop it.
            let index = read_index_ticking(&|o, l| chosen::read(&n, o, l).unwrap_or_default(), *size, &|w| chosen::progress(w, None));
            zips.push(index.unwrap_or_else(|e| {
                log(&format!("{name}: not read, {e}"));
                Vec::new()
            }));
        }
        let paths: Vec<String> = names.into_iter().map(|(n, _)| n).collect();
        let by_name = paths.clone();
        Rc::new(ZipSet::new(paths, zips, Box::new(move |zi, o, l| chosen::read(&by_name[zi], o, l).unwrap_or_default())))
    }

    fn run() -> Result<Vec<(Item, ScanExtras)>, String> {
        let albums = setting::<bool>("albums_from_folders").unwrap_or(true);
        let set = index_zips();
        let zfs = ZipFs::new(set.clone());
        for (name, why) in zfs.skipped() {
            log(&format!("skipped {name}: {why}"));
        }
        let ticks = ticking(&zfs);
        let real = ChosenFs::default();
        let found = scan(&OverlayFs::new(&real, &ticks), albums);
        STREAMS.with(|s| *s.borrow_mut() = Some(Streams::new(set)));
        Ok(found
            .into_iter()
            .map(|(i, x)| (Item { key: i.key, name: i.name, size: i.size, modified: i.modified, path: i.path }, x))
            .collect())
    }

    struct Takeout;

    impl Importer for Takeout {
        fn enumerate() -> Result<Vec<Item>, String> {
            let found = run()?;
            let _ = chosen::progress(found.len() as u64, Some(found.len() as u64));
            log(&format!("found {} photos and videos", found.len()));
            let map = found.iter().map(|(i, x)| (i.key.clone(), x.clone())).collect();
            FOUND.with(|f| *f.borrow_mut() = Some(map));
            Ok(found.into_iter().map(|(i, _)| i).collect())
        }

        // Items in the chosen tree have a `path`; only zip entries come here.
        fn read(key: String, offset: u64, len: u32) -> Result<Vec<u8>, String> {
            // The host keeps one instance for a whole import, so this only runs when `read`
            // comes first; it needs the zips' index, not a scan.
            if STREAMS.with(|s| s.borrow().is_none()) {
                let set = index_zips();
                STREAMS.with(|s| *s.borrow_mut() = Some(Streams::new(set)));
            }
            STREAMS.with(|s| s.borrow().as_ref().ok_or_else(|| "no zips".to_string())?.read(&key, offset, len))
        }

        fn extras(key: String) -> Option<Extras> {
            if FOUND.with(|f| f.borrow().is_none()) {
                let map = run().ok()?.into_iter().map(|(i, x)| (i.key, x)).collect();
                FOUND.with(|f| *f.borrow_mut() = Some(map));
            }
            let x = FOUND.with(|f| f.borrow().as_ref().and_then(|m| m.get(&key).cloned()))?;
            Some(Extras {
                favourite: x.favourite,
                albums: x.albums,
                title: x.title,
                caption: x.caption,
                keywords: x.keywords,
                captured_at: x.captured_at,
                lat: x.lat,
                lon: x.lon,
            })
        }

        fn sidecars(_key: String) -> Vec<Item> {
            Vec::new()
        }
    }

    lumen_plugin::export_importer!(Takeout);
}
