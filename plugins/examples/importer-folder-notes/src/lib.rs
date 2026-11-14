//! Imports a folder of photos. Items carry `path`, so Lumen opens the files itself (no bytes pass
//! through wasm). `IMG_1.txt` beside `IMG_1.jpg` supplies the title (first line) and the caption
//! (the rest); the setting `albums_from_folders` turns each parent folder into an album.
//!
//! Only the pure helpers build natively (`cargo test`); the plugin itself is wasm32-only.

#[cfg(target_arch = "wasm32")]
mod plugin {
    use lumen_plugin::{chosen, log, setting, Extras, Importer, Item};

    use super::{album_of, extension, join, note_path, split_note, MAX_DEPTH, MAX_NOTE, PHOTO_EXTENSIONS};

    struct FolderNotes;

    impl Importer for FolderNotes {
        fn enumerate() -> Result<Vec<Item>, String> {
            let mut items = Vec::new();
            let mut pending = vec![(String::new(), 0usize)];
            while let Some((dir, depth)) = pending.pop() {
                for entry in chosen::list(&dir)? {
                    let path = join(&dir, &entry.name);
                    if entry.is_dir {
                        if depth < MAX_DEPTH {
                            pending.push((path, depth + 1));
                        }
                    } else if extension(&entry.name).is_some_and(|e| PHOTO_EXTENSIONS.contains(&e.as_str())) {
                        items.push(Item { key: path.clone(), name: entry.name, size: entry.size, modified: entry.modified, path: Some(path) });
                    }
                }
                // The host asks the plugin to stop (cancelled, or out of time) by failing this call.
                chosen::progress(items.len() as u64, None)?;
            }
            items.sort_by(|a, b| a.key.cmp(&b.key));
            log(&format!("found {} photos", items.len()));
            Ok(items)
        }

        // Every item has a `path`, so Lumen never asks for its bytes.
        fn read(_key: String, _offset: u64, _len: u32) -> Result<Vec<u8>, String> {
            Err("this importer's items are opened by path".into())
        }

        fn extras(key: String) -> Option<Extras> {
            let note = note_path(&key);
            let (title, caption) = match chosen::stat(&note) {
                Ok(n) if !n.is_dir && n.size <= MAX_NOTE => {
                    let bytes = chosen::read_all(&note).ok()?;
                    split_note(&String::from_utf8_lossy(&bytes))
                }
                _ => (None, None),
            };
            let albums = if setting::<bool>("albums_from_folders").unwrap_or(true) { album_of(&key).into_iter().collect() } else { Vec::new() };
            if title.is_none() && caption.is_none() && albums.is_empty() {
                return None;
            }
            Some(Extras { favourite: false, albums, title, caption, keywords: Vec::new(), captured_at: None, lat: None, lon: None })
        }

        fn sidecars(_key: String) -> Vec<Item> {
            Vec::new()
        }
    }

    lumen_plugin::export_importer!(FolderNotes);
}

pub const PHOTO_EXTENSIONS: &[&str] = &["jpg", "jpeg", "png", "tif", "tiff", "heic", "webp", "dng", "nef", "cr2", "cr3", "arw"];
pub const MAX_DEPTH: usize = 8;
pub const MAX_NOTE: u64 = 64 * 1024;

pub fn extension(name: &str) -> Option<String> {
    name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase())
}

pub fn join(dir: &str, name: &str) -> String {
    if dir.is_empty() {
        name.to_string()
    } else {
        format!("{dir}/{name}")
    }
}

/// `a/b/IMG_1.jpg` -> `a/b/IMG_1.txt`.
pub fn note_path(key: &str) -> String {
    match key.rsplit_once('.') {
        Some((stem, _)) => format!("{stem}.txt"),
        None => format!("{key}.txt"),
    }
}

/// The last folder of `a/b/IMG_1.jpg` (`b`), if the photo is not in the root.
pub fn album_of(key: &str) -> Option<String> {
    let (dir, _) = key.rsplit_once('/')?;
    Some(dir.rsplit('/').next().unwrap_or(dir).to_string())
}

/// First non-empty line is the title; what follows is the caption.
pub fn split_note(text: &str) -> (Option<String>, Option<String>) {
    let mut lines = text.lines().map(str::trim_end).skip_while(|l| l.trim().is_empty());
    let title = lines.next().map(|l| l.trim().to_string()).filter(|t| !t.is_empty());
    let rest: Vec<&str> = lines.collect();
    let caption = rest.join("\n").trim().to_string();
    (title, (!caption.is_empty()).then_some(caption))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_and_albums() {
        assert_eq!(note_path("a/b/IMG_1.jpg"), "a/b/IMG_1.txt");
        assert_eq!(album_of("a/b/IMG_1.jpg").as_deref(), Some("b"));
        assert_eq!(album_of("IMG_1.jpg"), None);
        assert_eq!(join("", "x.jpg"), "x.jpg");
        assert_eq!(join("a", "x.jpg"), "a/x.jpg");
    }

    #[test]
    fn notes_split_into_title_and_caption() {
        assert_eq!(split_note("Harbour at dawn\n\nThe first boat out.\nCold."), (Some("Harbour at dawn".into()), Some("The first boat out.\nCold.".into())));
        assert_eq!(split_note("Only a title"), (Some("Only a title".into()), None));
        assert_eq!(split_note("  \n"), (None, None));
    }
}
