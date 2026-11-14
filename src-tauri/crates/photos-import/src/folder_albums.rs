//! A folder imported as albums (2026-11-08): each folder with photos becomes an album and each
//! folder with subfolders a collection, nested as on disk. A folder with both photos and
//! subfolders is a collection holding an album of its own name for its own photos. Folders
//! with nothing to import, however deep, make nothing.
//!
//! The albums are made as files land (`ItemExtras::folder_album`), so a cancelled import leaves
//! no empty album, and importing the folder again fills the same albums.

use std::collections::HashSet;
use std::io::Read;
use std::sync::OnceLock;

use photos_core::extras::ItemExtras;

use crate::source::{ImportSource, SourceItem};
use crate::ImportError;

/// Which folders of an import hold another folder with something to import.
#[derive(Debug, Clone)]
pub struct FolderTree {
    root: String,
    /// Relative folder paths (`""` is the chosen folder), `/`-separated as `SourceItem::rel_path`.
    parents: HashSet<String>,
}

fn folder_of(rel_path: &str) -> &str {
    rel_path.rsplit_once('/').map_or("", |(dir, _)| dir)
}

impl FolderTree {
    /// `root` names the chosen folder; `rel_paths` are the files that will be imported.
    pub fn new<'a>(root: &str, rel_paths: impl IntoIterator<Item = &'a str>) -> Self {
        let mut parents = HashSet::new();
        for rel in rel_paths {
            let mut dir = folder_of(rel);
            while !dir.is_empty() {
                let up = folder_of(dir);
                if !parents.insert(up.to_string()) {
                    break;
                }
                dir = up;
            }
        }
        Self { root: root.to_string(), parents }
    }

    /// The collections a file's album sits in, outermost first, then the album's name.
    pub fn album_for(&self, rel_path: &str) -> Vec<String> {
        let dir = folder_of(rel_path);
        let mut path = vec![self.root.clone()];
        path.extend(dir.split('/').filter(|s| !s.is_empty()).map(str::to_string));
        if self.parents.contains(dir) {
            // Its own photos, beside its subfolders' albums.
            let own = path.last().cloned().unwrap_or_default();
            path.push(own);
        }
        path
    }
}

/// `inner` with each file's folder album added to what it knows about the file.
pub struct FolderAlbumsSource<S: ImportSource> {
    inner: S,
    tree: OnceLock<FolderTree>,
}

impl<S: ImportSource> FolderAlbumsSource<S> {
    pub fn new(inner: S) -> Self {
        Self { inner, tree: OnceLock::new() }
    }
}

impl<S: ImportSource> ImportSource for FolderAlbumsSource<S> {
    fn label(&self) -> String {
        self.inner.label()
    }

    fn kind(&self) -> i64 {
        self.inner.kind()
    }

    fn enumerate(&self) -> Result<Vec<SourceItem>, ImportError> {
        let items = self.inner.enumerate()?;
        self.tree.get_or_init(|| FolderTree::new(&self.inner.label(), items.iter().map(|i| i.rel_path.as_str())));
        Ok(items)
    }

    fn open(&self, item: &SourceItem) -> Result<Box<dyn Read + Send>, ImportError> {
        self.inner.open(item)
    }

    fn extras(&self, item: &SourceItem) -> Option<ItemExtras> {
        let Some(tree) = self.tree.get() else { return self.inner.extras(item) };
        Some(ItemExtras { folder_album: Some(tree.album_for(&item.rel_path)), ..self.inner.extras(item).unwrap_or_default() })
    }

    fn is_connected(&self) -> bool {
        self.inner.is_connected()
    }

    fn sidecars(&self, item: &SourceItem) -> Vec<SourceItem> {
        self.inner.sidecars(item)
    }

    fn leftovers(&self) -> Vec<(String, &'static str)> {
        self.inner.leftovers()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn albums(files: &[&str]) -> Vec<Vec<String>> {
        let tree = FolderTree::new("Trips", files.iter().copied());
        files.iter().map(|f| tree.album_for(f)).collect()
    }

    fn path(p: &[&str]) -> Vec<String> {
        p.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn a_folder_of_photos_is_one_album() {
        assert_eq!(albums(&["a.jpg", "b.heic"]), [path(&["Trips"]), path(&["Trips"])]);
    }

    #[test]
    fn subfolders_make_the_folder_a_collection_of_their_albums() {
        let got = albums(&["2019/Summer/x.jpg", "2019/Winter/y.jpg", "2020/z.jpg", "A/B/C/deep.jpg"]);
        assert_eq!(got, [path(&["Trips", "2019", "Summer"]), path(&["Trips", "2019", "Winter"]), path(&["Trips", "2020"]), path(&["Trips", "A", "B", "C"])]);
    }

    #[test]
    fn a_folder_with_photos_and_subfolders_keeps_its_own_photos_in_an_album_of_its_name() {
        let got = albums(&["top.jpg", "2019/y.jpg", "2019/Summer/x.jpg"]);
        assert_eq!(got, [path(&["Trips", "Trips"]), path(&["Trips", "2019", "2019"]), path(&["Trips", "2019", "Summer"])]);
    }

    #[test]
    fn folders_with_nothing_to_import_make_nothing() {
        // Only the files that will be imported are listed, so an empty or a junk-only folder
        // never shows up, and a folder whose only subfolder is empty stays an album.
        let tree = FolderTree::new("Trips", ["Kept/a.jpg"]);
        assert_eq!(tree.album_for("Kept/a.jpg"), path(&["Trips", "Kept"]));
        assert!(!tree.parents.contains("Kept"));
    }

    struct Listed(Vec<&'static str>);

    impl ImportSource for Listed {
        fn label(&self) -> String {
            "Trips".into()
        }
        fn kind(&self) -> i64 {
            photos_core::batches::SOURCE_FOLDER
        }
        fn enumerate(&self) -> Result<Vec<SourceItem>, ImportError> {
            Ok(self.0.iter().map(|p| SourceItem { rel_path: p.to_string(), file_name: p.rsplit('/').next().unwrap().into(), size: 1, modified_at: None, created_at: None }).collect())
        }
        fn open(&self, _: &SourceItem) -> Result<Box<dyn Read + Send>, ImportError> {
            Ok(Box::new(std::io::empty()))
        }
        fn extras(&self, _: &SourceItem) -> Option<ItemExtras> {
            Some(ItemExtras { favorite: true, ..Default::default() })
        }
    }

    #[test]
    fn the_source_adds_each_files_album_to_what_the_inner_source_knows() {
        let source = FolderAlbumsSource::new(Listed(vec!["2019/x.jpg", "y.jpg"]));
        let items = source.enumerate().unwrap();
        let x = source.extras(&items[0]).unwrap();
        assert_eq!((x.favorite, x.folder_album), (true, Some(path(&["Trips", "2019"]))));
        assert_eq!(source.extras(&items[1]).unwrap().folder_album, Some(path(&["Trips", "Trips"])));
    }
}
