//! An Apple Photos library as an import source: every visible original on disk, its Live
//! Photo video and its RAW alternate, carrying Photos' favourite, albums, text, date and
//! place as extras.

use std::collections::HashMap;
use std::io::Read;

use photos_core::extras::ItemExtras;
use serde::Serialize;
use ts_rs::TS;

use super::db::{read_library, AppleAsset};
use super::files::LibraryFiles;
use crate::macdisk::source::TempRead;
use crate::source::{ImportSource, SourceItem};
use crate::ImportError;

/// Photos' hidden items land here rather than in the timeline's plain view of everything.
pub const HIDDEN_ALBUM: &str = "Hidden in Apple Photos";

/// What an import of this library brings, and what it leaves behind and why.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct LibrarySummary {
    pub name: String,
    /// Photos and videos that will be imported; Live Photo videos and RAW alternates ride
    /// along and are not counted.
    pub items: u32,
    pub live_photos: u32,
    /// Originals in iCloud only (*Optimize Mac Storage*).
    pub not_downloaded: u32,
    /// Files Photos kept outside the library, at paths on the Mac they were imported on.
    pub referenced: u32,
    /// Burst frames the user did not pick.
    pub burst_frames: u32,
    /// In Photos' *Recently Deleted*.
    pub trashed: u32,
}

pub struct PhotosLibrarySource {
    files: Box<dyn LibraryFiles>,
    summary: LibrarySummary,
    items: Vec<SourceItem>,
    /// Primary items only: rel path → index into `assets`.
    primary: HashMap<String, usize>,
    assets: Vec<AppleAsset>,
}

/// Companion files are `<UUID>_<n>.<ext>`: 3 is a Live Photo's video, 4 a RAW alternate.
const LIVE_VIDEO: &str = "3";
const RAW_ALTERNATE: &str = "4";

impl PhotosLibrarySource {
    /// Copies `database/Photos.sqlite` (with its write-ahead log, which holds the latest
    /// changes) into a temporary folder, reads it, and lists the `originals` folders once
    /// to learn which files are really on disk.
    pub fn open(files: Box<dyn LibraryFiles>, name: &str) -> Result<Self, ImportError> {
        let tmp = tempfile::tempdir()?;
        let db_path = tmp.path().join("Photos.sqlite");
        {
            let mut out = std::io::BufWriter::with_capacity(1 << 20, std::fs::File::create(&db_path)?);
            files.copy_to("database/Photos.sqlite", &mut out).map_err(|e| ImportError::Source(format!("not a Photos library ({e})")))?;
        }
        if let Ok(mut wal) = std::fs::File::create(tmp.path().join("Photos.sqlite-wal")) {
            if files.copy_to("database/Photos.sqlite-wal", &mut wal).is_err() {
                drop(wal);
                let _ = std::fs::remove_file(tmp.path().join("Photos.sqlite-wal"));
            }
        }
        let assets = {
            let conn = rusqlite::Connection::open(&db_path).map_err(|e| ImportError::Source(format!("Photos.sqlite: {e}")))?;
            read_library(&conn)?
        };
        drop(tmp);

        // Which files exist: one listing per originals folder, not one lookup per photo.
        let mut dirs: Vec<&str> = assets.iter().filter(|a| !a.referenced()).map(|a| a.directory.as_str()).collect();
        dirs.sort_unstable();
        dirs.dedup();
        let mut on_disk: HashMap<String, u64> = HashMap::new();
        let mut companions: HashMap<String, Vec<(String, String, u64)>> = HashMap::new();
        for dir in dirs {
            for (file, size) in files.list(&format!("originals/{dir}")).unwrap_or_default() {
                if let Some((uuid, rest)) = file.split_once('_') {
                    if let Some((n, ext)) = rest.split_once('.') {
                        companions.entry(uuid.to_string()).or_default().push((n.to_string(), ext.to_string(), size));
                    }
                }
                on_disk.insert(format!("originals/{dir}/{file}"), size);
            }
        }

        let mut summary = LibrarySummary { name: name.to_string(), ..Default::default() };
        let mut items = Vec::new();
        let mut primary = HashMap::new();
        for (i, a) in assets.iter().enumerate() {
            if a.trashed {
                summary.trashed += 1;
                continue;
            }
            if !a.visible {
                summary.burst_frames += 1;
                continue;
            }
            if a.referenced() {
                summary.referenced += 1;
                continue;
            }
            let rel = format!("originals/{}/{}", a.directory, a.filename);
            let Some(&size) = on_disk.get(&rel) else {
                summary.not_downloaded += 1;
                continue;
            };
            let shown = a.original_filename.clone().unwrap_or_else(|| a.filename.clone());
            let stem = shown.rsplit_once('.').map_or(shown.as_str(), |(s, _)| s).to_string();
            let modified_at = a.created.map(|c| c + a.tz_offset_minutes.unwrap_or(0) as i64 * 60);
            summary.items += 1;
            primary.insert(rel.clone(), i);
            items.push(SourceItem { rel_path: rel, file_name: shown, size, modified_at, created_at: None });
            let mut extra = companions.get(&a.uuid).cloned().unwrap_or_default();
            extra.sort();
            for (n, ext, size) in extra {
                if n != LIVE_VIDEO && n != RAW_ALTERNATE {
                    continue;
                }
                if n == LIVE_VIDEO && !a.video {
                    summary.live_photos += 1;
                }
                // Named after the photo, so Live Photo pairing by stem finds the pair.
                items.push(SourceItem {
                    rel_path: format!("originals/{}/{}_{n}.{ext}", a.directory, a.uuid),
                    file_name: format!("{stem}.{}", ext.to_ascii_uppercase()),
                    size,
                    modified_at,
                    created_at: None,
                });
            }
        }
        Ok(Self { files, summary, items, primary, assets })
    }

    pub fn summary(&self) -> &LibrarySummary {
        &self.summary
    }

    /// The `import_seen` key: re-importing a library brings only what is new.
    pub fn key(&self) -> String {
        format!("photos:{}", self.summary.name)
    }

    /// Newest photos and videos (not Live Photo videos or RAW alternates), newest first by
    /// the moment they were taken — UTC, so a photo from a later time zone does not jump
    /// ahead of one taken after it.
    pub fn newest(&self, n: usize) -> Vec<&SourceItem> {
        let taken = |i: &SourceItem| self.primary.get(&i.rel_path).and_then(|&a| self.assets[a].created);
        let mut shown: Vec<&SourceItem> = self.items.iter().filter(|i| self.primary.contains_key(&i.rel_path)).collect();
        shown.sort_by(|a, b| taken(b).cmp(&taken(a)).then(b.rel_path.cmp(&a.rel_path)));
        shown.truncate(n);
        shown
    }
}

impl ImportSource for PhotosLibrarySource {
    fn label(&self) -> String {
        self.summary.name.clone()
    }

    fn kind(&self) -> i64 {
        photos_core::batches::SOURCE_PHOTOS_LIBRARY
    }

    fn enumerate(&self) -> Result<Vec<SourceItem>, ImportError> {
        Ok(self.items.clone())
    }

    fn open(&self, item: &SourceItem) -> Result<Box<dyn Read + Send>, ImportError> {
        Ok(Box::new(TempRead::copy(|out| self.files.copy_to(&item.rel_path, out))?))
    }

    /// Only for the photo itself: its Live Photo video and RAW alternate ride along.
    fn extras(&self, item: &SourceItem) -> Option<ItemExtras> {
        let a = &self.assets[*self.primary.get(&item.rel_path)?];
        let mut albums = a.albums.clone();
        if a.hidden {
            albums.push(HIDDEN_ALBUM.to_string());
        }
        Some(ItemExtras {
            favorite: a.favorite,
            albums,
            title: a.title.clone(),
            caption: a.caption.clone(),
            keywords: a.keywords.clone(),
            captured: a.created.map(|c| (c, a.tz_offset_minutes)),
            location: a.location,
            ..Default::default()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::apple::db::tests::FIXTURE;
    use crate::apple::files::DirFiles;

    /// A library folder: the fixture catalogue, and originals for AAA (with its Live Photo
    /// video), BBB and the unpicked burst frame DDD. CCC is trashed, EEE referenced, and
    /// FFF's original is in iCloud only.
    fn library() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join("database")).unwrap();
        rusqlite::Connection::open(root.join("database/Photos.sqlite")).unwrap().execute_batch(FIXTURE).unwrap();
        for (path, bytes) in [
            ("originals/A/AAA.heic", &b"still"[..]),
            ("originals/A/AAA_3.mov", b"motion"),
            ("originals/A/AAA_9.plist", b"ignored"),
            ("originals/B/BBB.mov", b"video"),
            ("originals/D/DDD.jpeg", b"burst"),
        ] {
            std::fs::create_dir_all(root.join(path).parent().unwrap()).unwrap();
            std::fs::write(root.join(path), bytes).unwrap();
        }
        dir
    }

    #[test]
    fn imports_originals_with_their_live_video_and_counts_what_it_skips() {
        let lib = library();
        let src = PhotosLibrarySource::open(Box::new(DirFiles(lib.path().to_path_buf())), "Test Library").unwrap();
        let items = src.enumerate().unwrap();
        let names: Vec<(&str, &str)> = items.iter().map(|i| (i.rel_path.as_str(), i.file_name.as_str())).collect();
        assert_eq!(
            names,
            [
                ("originals/A/AAA.heic", "IMG_0001.HEIC"),
                ("originals/A/AAA_3.mov", "IMG_0001.MOV"),
                ("originals/B/BBB.mov", "IMG_0002.MOV"),
            ]
        );
        let s = src.summary();
        assert_eq!((s.items, s.live_photos, s.trashed, s.burst_frames, s.referenced, s.not_downloaded), (2, 1, 1, 1, 1, 1));

        let x = src.extras(&items[0]).unwrap();
        assert!(x.favorite);
        assert_eq!(x.albums, ["Iceland"]);
        assert_eq!(x.captured, Some((721000000 + 978_307_200, Some(120))));
        assert_eq!(x.location, Some((64.04, -16.18)));
        assert_eq!(x.keywords, ["iceland", "waterfall"]);
        assert!(src.extras(&items[1]).is_none(), "the Live Photo video rides on its still");
        assert_eq!(src.extras(&items[2]).unwrap().albums, ["Iceland", HIDDEN_ALBUM]);

        let mut bytes = Vec::new();
        src.open(&items[1]).unwrap().read_to_end(&mut bytes).unwrap();
        assert_eq!(bytes, b"motion");
        assert_eq!(src.newest(1)[0].rel_path, "originals/B/BBB.mov");
        assert_eq!(src.key(), "photos:Test Library");
        assert_eq!(src.kind(), photos_core::batches::SOURCE_PHOTOS_LIBRARY);
    }

    #[test]
    fn a_folder_that_is_not_a_library_says_so() {
        let dir = tempfile::tempdir().unwrap();
        let err = PhotosLibrarySource::open(Box::new(DirFiles(dir.path().to_path_buf())), "Nope").err().unwrap();
        assert!(err.to_string().contains("not a Photos library"), "{err}");
    }
}
