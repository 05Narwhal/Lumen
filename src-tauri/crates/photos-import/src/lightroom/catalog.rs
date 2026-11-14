//! A Lightroom Classic catalog as an import source.

use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};

use photos_core::edits::Recipe;
use photos_core::extras::ItemExtras;
use rusqlite::{Connection, OpenFlags};
use serde::Serialize;
use ts_rs::TS;

use crate::source::{ImportSource, SourceItem};
use crate::ImportError;

/// What an import of a catalog brings, and what it leaves behind.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct CatalogSummary {
    pub name: String,
    /// Photos and videos whose originals are there (a RAW's JPEG rides along, uncounted).
    pub items: u32,
    /// Originals the catalog lists that are not on this computer (moved, or on a drive that
    /// is not plugged in), by path.
    pub missing: Vec<String>,
    /// Lightroom's virtual copies: a second set of settings on the same original.
    pub virtual_copies: u32,
    /// Smart collections, by name: rules in Lightroom, not lists of photos.
    pub smart_collections: Vec<String>,
}

/// One photo of the catalog: what Lightroom says about it.
#[derive(Debug, Default)]
struct Photo {
    rating: Option<i64>,
    flag: i64,
    color_label: i64,
    keywords: Vec<String>,
    albums: Vec<String>,
    caption: Option<String>,
    recipe: Option<Recipe>,
}

pub struct LightroomCatalogSource {
    summary: CatalogSummary,
    path: PathBuf,
    items: Vec<SourceItem>,
    /// Item path → (photo, whether it is the photo itself rather than the JPEG beside a RAW).
    owner: HashMap<String, (usize, bool)>,
    photos: Vec<Photo>,
}

/// Lightroom's colour label names, in Lumen's order (1 red … 5 purple).
pub(crate) fn color_label(name: &str) -> i64 {
    match name.trim() {
        "Red" => 1,
        "Yellow" => 2,
        "Green" => 3,
        "Blue" => 4,
        "Purple" => 5,
        _ => 0,
    }
}

/// SQLite's lock-byte page starts at 1 GiB: Windows refuses to read bytes another process has
/// locked, and SQLite never keeps data there, so the copy writes zeros in its place.
const LOCK_BYTE: u64 = 0x4000_0000;
const LOCK_BYTES: u64 = 512;

fn copy_skipping_lock_page(from: &Path, to: &Path) -> std::io::Result<()> {
    use std::io::{Seek, SeekFrom, Write};
    let mut src = std::fs::File::open(from)?;
    let len = src.metadata()?.len();
    let mut dst = std::io::BufWriter::with_capacity(1 << 20, std::fs::File::create(to)?);
    let mut buf = vec![0u8; 1 << 20];
    let mut at = 0u64;
    while at < len {
        if at == LOCK_BYTE {
            let n = LOCK_BYTES.min(len - at);
            dst.write_all(&vec![0u8; n as usize])?;
            at += n;
            src.seek(SeekFrom::Start(at))?;
            continue;
        }
        let limit = if at < LOCK_BYTE { (LOCK_BYTE - at).min(buf.len() as u64) } else { buf.len() as u64 };
        let n = src.read(&mut buf[..limit as usize])?;
        if n == 0 {
            break;
        }
        dst.write_all(&buf[..n])?;
        at += n as u64;
    }
    dst.flush()
}

/// The catalog and its write-ahead log or journal, copied into `dir`: Lightroom holds the
/// catalog open, and Lumen never opens it itself. The `-shm` index is left behind: its bytes
/// are locked while Lightroom runs, and SQLite rebuilds it from the log.
pub(crate) fn copy_catalog(path: &Path, dir: &Path) -> std::io::Result<PathBuf> {
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "catalog.lrcat".into());
    let copy = dir.join(&name);
    copy_skipping_lock_page(path, &copy)?;
    for suffix in ["-wal", "-journal"] {
        let side = PathBuf::from(format!("{}{suffix}", path.display()));
        if side.is_file() {
            copy_skipping_lock_page(&side, &dir.join(format!("{name}{suffix}")))?;
        }
    }
    Ok(copy)
}

fn db_error(e: rusqlite::Error) -> ImportError {
    ImportError::Source(format!("not a Lightroom catalog ({e})"))
}

/// Rows of a query, or none when an older catalog lacks the table.
fn rows<T>(conn: &Connection, sql: &str, f: impl Fn(&rusqlite::Row) -> rusqlite::Result<T>) -> Vec<T> {
    let Ok(mut stmt) = conn.prepare(sql) else { return Vec::new() };
    let Ok(found) = stmt.query_map([], f) else { return Vec::new() };
    found.flatten().collect()
}

/// A catalog's develop text, converted into a recipe; `None` when it changes nothing.
fn recipe_from(text: &str) -> Option<Recipe> {
    let settings = photos_core::lightroom::read_lua_table(text).ok()?;
    let c = photos_core::lightroom::convert(&settings, None);
    let geometry = c.geometry.unwrap_or(photos_core::lightroom::Geometry { crop: None, straighten: 0.0 });
    let recipe = Recipe { develop: Some(c.develop), crop: geometry.crop, straighten: geometry.straighten, ..Recipe::identity() }.normalized();
    (!recipe.is_identity()).then_some(recipe)
}

fn file_item(path: &Path) -> Option<SourceItem> {
    let meta = std::fs::metadata(path).ok().filter(|m| m.is_file())?;
    let modified_at = meta.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_secs() as i64);
    Some(SourceItem {
        rel_path: path.to_string_lossy().into_owned(),
        file_name: path.file_name()?.to_string_lossy().into_owned(),
        size: meta.len(),
        modified_at,
        created_at: None,
    })
}

impl LightroomCatalogSource {
    /// Copies the catalog into a temporary folder, reads it there and lists which originals
    /// are on this computer.
    pub fn open(path: &Path) -> Result<Self, ImportError> {
        if !path.is_file() {
            return Err(ImportError::Source(format!("{} is not a Lightroom catalog", path.display())));
        }
        let tmp = tempfile::tempdir()?;
        let copy = copy_catalog(path, tmp.path())?;
        // The copy is Lumen's own: opened for writing so SQLite can replay the log into it.
        let conn = Connection::open_with_flags(&copy, OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX).map_err(db_error)?;
        let name = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "Lightroom catalog".into());
        let mut source = Self { summary: CatalogSummary { name, ..Default::default() }, path: path.to_path_buf(), items: Vec::new(), owner: HashMap::new(), photos: Vec::new() };
        source.read(&conn)?;
        Ok(source)
    }

    fn read(&mut self, conn: &Connection) -> Result<(), ImportError> {
        type Row = (i64, Option<f64>, Option<f64>, Option<String>, Option<i64>, String, String, String, String, Option<String>);
        let mut stmt = conn
            .prepare(
                "SELECT i.id_local, i.rating, i.pick, i.colorLabels, i.masterImage, r.absolutePath, fo.pathFromRoot, f.baseName, f.extension, f.sidecarExtensions
                 FROM Adobe_images i
                 JOIN AgLibraryFile f ON f.id_local = i.rootFile
                 JOIN AgLibraryFolder fo ON fo.id_local = f.folder
                 JOIN AgLibraryRootFolder r ON r.id_local = fo.rootFolder
                 ORDER BY i.id_local",
            )
            .map_err(db_error)?;
        let images: Vec<Row> = stmt
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?, r.get(7)?, r.get(8)?, r.get(9)?)))
            .map_err(db_error)?
            .collect::<rusqlite::Result<_>>()
            .map_err(db_error)?;

        let mut keywords: HashMap<i64, Vec<String>> = HashMap::new();
        for (image, name) in rows(
            conn,
            "SELECT ki.image, k.name FROM AgLibraryKeywordImage ki JOIN AgLibraryKeyword k ON k.id_local = ki.tag WHERE k.name IS NOT NULL AND k.name != '' ORDER BY ki.image, k.name",
            |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)),
        ) {
            keywords.entry(image).or_default().push(name);
        }
        let mut albums: HashMap<i64, Vec<String>> = HashMap::new();
        for (image, name) in rows(
            conn,
            "SELECT ci.image, c.name FROM AgLibraryCollectionImage ci JOIN AgLibraryCollection c ON c.id_local = ci.collection
             WHERE c.creationId = 'com.adobe.ag.library.collection' AND c.name IS NOT NULL ORDER BY ci.image, c.name",
            |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)),
        ) {
            albums.entry(image).or_default().push(name);
        }
        self.summary.smart_collections =
            rows(conn, "SELECT name FROM AgLibraryCollection WHERE creationId = 'com.adobe.ag.library.smart_collection' AND name IS NOT NULL ORDER BY name", |r| r.get::<_, String>(0));
        let captions: HashMap<i64, String> =
            rows(conn, "SELECT image, caption FROM AgLibraryIPTC WHERE caption IS NOT NULL AND caption != ''", |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))).into_iter().collect();
        let develop: HashMap<i64, String> =
            rows(conn, "SELECT image, text FROM Adobe_imageDevelopSettings WHERE text IS NOT NULL AND text != ''", |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))).into_iter().collect();

        for (id, rating, pick, label, master, root, folder, base, ext, sidecars) in images {
            if master.is_some() {
                self.summary.virtual_copies += 1;
                continue;
            }
            let dir = PathBuf::from(format!("{root}{folder}"));
            let original = dir.join(format!("{base}.{ext}"));
            let Some(item) = file_item(&original) else {
                self.summary.missing.push(original.to_string_lossy().into_owned());
                continue;
            };
            let index = self.photos.len();
            self.photos.push(Photo {
                rating: rating.map(|r| r.round() as i64).filter(|r| *r > 0),
                flag: pick.map_or(0, |p| p.signum() as i64),
                color_label: label.as_deref().map_or(0, color_label),
                keywords: keywords.remove(&id).unwrap_or_default(),
                albums: albums.remove(&id).unwrap_or_default(),
                caption: captions.get(&id).cloned(),
                recipe: develop.get(&id).and_then(|t| recipe_from(t)),
            });
            self.summary.items += 1;
            self.owner.insert(item.rel_path.clone(), (index, true));
            self.items.push(item);
            // The JPEG a camera saved beside the RAW: Lightroom lists it as a sidecar.
            for side in sidecars.as_deref().unwrap_or("").split(',').map(str::trim).filter(|s| !s.is_empty() && !s.eq_ignore_ascii_case("xmp")) {
                if let Some(item) = file_item(&dir.join(format!("{base}.{side}"))) {
                    self.owner.insert(item.rel_path.clone(), (index, false));
                    self.items.push(item);
                }
            }
        }
        Ok(())
    }

    pub fn summary(&self) -> &CatalogSummary {
        &self.summary
    }

    /// The `import_seen` key: importing a catalog again brings only what is new.
    pub fn key(&self) -> String {
        format!("lightroom:{}", self.path.display())
    }
}

impl ImportSource for LightroomCatalogSource {
    fn label(&self) -> String {
        self.summary.name.clone()
    }

    fn kind(&self) -> i64 {
        photos_core::batches::SOURCE_LIGHTROOM
    }

    fn enumerate(&self) -> Result<Vec<SourceItem>, ImportError> {
        Ok(self.items.clone())
    }

    /// Originals are read, never written.
    fn open(&self, item: &SourceItem) -> Result<Box<dyn Read + Send>, ImportError> {
        Ok(Box::new(std::fs::File::open(PathBuf::from(&item.rel_path))?))
    }

    /// The photo's marks, keywords, collections and caption; its develop only on the photo
    /// itself, not on the JPEG beside a RAW.
    fn extras(&self, item: &SourceItem) -> Option<ItemExtras> {
        let &(index, primary) = self.owner.get(&item.rel_path)?;
        let p = &self.photos[index];
        Some(ItemExtras {
            albums: p.albums.clone(),
            caption: p.caption.clone(),
            keywords: p.keywords.clone(),
            rating: p.rating,
            flag: (p.flag != 0).then_some(p.flag),
            color_label: (p.color_label != 0).then_some(p.color_label),
            recipe: if primary { p.recipe.clone() } else { None },
            ..Default::default()
        })
    }

    /// An original on a drive that has been unplugged since the import began.
    fn is_connected(&self) -> bool {
        self.items.first().is_none_or(|i| Path::new(&i.rel_path).exists())
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use rusqlite::Connection;

    /// A catalog with the tables and columns Lumen reads, as Lightroom Classic writes them.
    pub(crate) fn create_catalog(path: &Path) -> Connection {
        let c = Connection::open(path).unwrap();
        c.execute_batch(
            "CREATE TABLE AgLibraryRootFolder (id_local INTEGER PRIMARY KEY, id_global TEXT, absolutePath TEXT, name TEXT, relativePathFromCatalog TEXT);
             CREATE TABLE AgLibraryFolder (id_local INTEGER PRIMARY KEY, id_global TEXT, pathFromRoot TEXT, rootFolder INTEGER);
             CREATE TABLE AgLibraryFile (id_local INTEGER PRIMARY KEY, id_global TEXT, baseName TEXT, extension TEXT, folder INTEGER, idx_filename TEXT, originalFilename TEXT, sidecarExtensions TEXT);
             CREATE TABLE Adobe_images (id_local INTEGER PRIMARY KEY, id_global TEXT, captureTime TEXT, colorLabels TEXT, fileFormat TEXT, masterImage INTEGER, orientation TEXT, pick REAL, rating REAL, rootFile INTEGER, copyName TEXT);
             CREATE TABLE Adobe_imageDevelopSettings (id_local INTEGER PRIMARY KEY, image INTEGER, text TEXT, digest TEXT);
             CREATE TABLE AgLibraryKeyword (id_local INTEGER PRIMARY KEY, id_global TEXT, name TEXT, lc_name TEXT, parent INTEGER);
             CREATE TABLE AgLibraryKeywordImage (id_local INTEGER PRIMARY KEY, image INTEGER, tag INTEGER);
             CREATE TABLE AgLibraryCollection (id_local INTEGER PRIMARY KEY, creationId TEXT, name TEXT, parent INTEGER);
             CREATE TABLE AgLibraryCollectionImage (id_local INTEGER PRIMARY KEY, collection INTEGER, image INTEGER, pick REAL, positionInCollection TEXT);
             CREATE TABLE AgLibraryIPTC (id_local INTEGER PRIMARY KEY, image INTEGER, caption TEXT, copyright TEXT);",
        )
        .unwrap();
        c
    }

    /// Three photos in `originals`: IMG_0001.CR3 (with its JPEG beside it, rated 4, red,
    /// picked, edited), a virtual copy of it, and missing.jpg, which is not on disk.
    pub(crate) fn sample(dir: &Path) -> PathBuf {
        let originals = dir.join("originals").join("2024").join("06");
        std::fs::create_dir_all(&originals).unwrap();
        std::fs::write(originals.join("IMG_0001.CR3"), b"raw bytes").unwrap();
        std::fs::write(originals.join("IMG_0001.JPG"), b"jpeg bytes").unwrap();
        let catalog = dir.join("Lightroom Catalog.lrcat");
        let c = create_catalog(&catalog);
        let root = format!("{}/", dir.join("originals").to_string_lossy().replace('\\', "/"));
        c.execute("INSERT INTO AgLibraryRootFolder (id_local, absolutePath, name) VALUES (1, ?1, 'originals')", [root]).unwrap();
        c.execute_batch(
            "INSERT INTO AgLibraryFolder (id_local, pathFromRoot, rootFolder) VALUES (2, '2024/06/', 1);
             INSERT INTO AgLibraryFile (id_local, baseName, extension, folder, idx_filename, sidecarExtensions) VALUES (100, 'IMG_0001', 'CR3', 2, 'IMG_0001.CR3', 'JPG,xmp');
             INSERT INTO AgLibraryFile (id_local, baseName, extension, folder, idx_filename, sidecarExtensions) VALUES (102, 'missing', 'jpg', 2, 'missing.jpg', '');
             INSERT INTO Adobe_images (id_local, captureTime, colorLabels, fileFormat, masterImage, orientation, pick, rating, rootFile) VALUES (10, '2024-06-01T10:00:00', 'Red', 'RAW', NULL, 'AB', 1, 4, 100);
             INSERT INTO Adobe_images (id_local, captureTime, colorLabels, fileFormat, masterImage, orientation, pick, rating, rootFile, copyName) VALUES (11, '2024-06-01T10:00:00', '', 'RAW', 10, 'AB', 0, 2, 100, 'Copy 1');
             INSERT INTO Adobe_images (id_local, captureTime, colorLabels, fileFormat, masterImage, orientation, pick, rating, rootFile) VALUES (12, '2024-06-02T09:00:00', 'Blue', 'JPG', NULL, 'AB', 0, NULL, 102);
             INSERT INTO Adobe_imageDevelopSettings (image, text) VALUES (10, 's = { Exposure2012 = 0.5, Vibrance = 10, HasCrop = true, CropLeft = 0.1, CropTop = 0, CropRight = 0.9, CropBottom = 1, CropAngle = 0, ProcessVersion = \"11.0\", WhiteBalance = \"As Shot\" }');
             INSERT INTO Adobe_imageDevelopSettings (image, text) VALUES (11, 's = { Exposure2012 = -1 }');
             INSERT INTO AgLibraryKeyword (id_local, name) VALUES (1, NULL);
             INSERT INTO AgLibraryKeyword (id_local, name, parent) VALUES (20, 'beach', 1);
             INSERT INTO AgLibraryKeyword (id_local, name, parent) VALUES (21, 'family', 1);
             INSERT INTO AgLibraryKeywordImage (image, tag) VALUES (10, 20);
             INSERT INTO AgLibraryKeywordImage (image, tag) VALUES (10, 21);
             INSERT INTO AgLibraryCollection (id_local, creationId, name) VALUES (30, 'com.adobe.ag.library.collection', 'Trip to Porto');
             INSERT INTO AgLibraryCollection (id_local, creationId, name) VALUES (31, 'com.adobe.ag.library.smart_collection', 'Five stars');
             INSERT INTO AgLibraryCollection (id_local, creationId, name) VALUES (32, 'com.adobe.ag.library.group', 'Travel');
             INSERT INTO AgLibraryCollectionImage (collection, image) VALUES (30, 10);
             INSERT INTO AgLibraryIPTC (image, caption) VALUES (10, 'Sunset over the river');",
        )
        .unwrap();
        drop(c);
        catalog
    }

    #[test]
    fn a_catalog_brings_its_originals_with_their_marks_and_edits() {
        let dir = tempfile::tempdir().unwrap();
        let catalog = sample(dir.path());
        let source = LightroomCatalogSource::open(&catalog).unwrap();
        let s = source.summary();
        assert_eq!(s.name, "Lightroom Catalog");
        assert_eq!(s.items, 1);
        assert_eq!(s.virtual_copies, 1);
        assert_eq!(s.smart_collections, vec!["Five stars".to_string()]);
        assert_eq!(s.missing.len(), 1);
        assert!(s.missing[0].ends_with("missing.jpg"), "{:?}", s.missing);

        let items = source.enumerate().unwrap();
        let names: Vec<&str> = items.iter().map(|i| i.file_name.as_str()).collect();
        assert_eq!(names, vec!["IMG_0001.CR3", "IMG_0001.JPG"], "the RAW and the JPEG Lightroom kept beside it");
        let mut bytes = Vec::new();
        source.open(&items[0]).unwrap().read_to_end(&mut bytes).unwrap();
        assert_eq!(bytes, b"raw bytes");

        let raw = source.extras(&items[0]).unwrap();
        assert_eq!((raw.rating, raw.flag, raw.color_label), (Some(4), Some(1), Some(1)));
        assert_eq!(raw.keywords, vec!["beach".to_string(), "family".into()]);
        assert_eq!(raw.albums, vec!["Trip to Porto".to_string()]);
        assert_eq!(raw.caption.as_deref(), Some("Sunset over the river"));
        let recipe = raw.recipe.expect("the develop settings, converted");
        assert_eq!(recipe.develop.as_ref().map(|d| (d.basic.exposure, d.basic.vibrance)), Some((0.5, 0.1)));
        assert!(recipe.crop.is_some_and(|k| (k.x - 0.1).abs() < 1e-6 && (k.w - 0.8).abs() < 1e-6));
        // The JPEG shares the photo's marks, not its RAW develop.
        let jpeg = source.extras(&items[1]).unwrap();
        assert_eq!((jpeg.rating, jpeg.albums.len(), jpeg.recipe.is_none()), (Some(4), 1, true));
    }

    #[test]
    fn colour_labels_read_in_lumens_order() {
        for (name, n) in [("Red", 1), ("Yellow", 2), ("Green", 3), ("Blue", 4), ("Purple", 5), ("", 0), ("Custom", 0)] {
            assert_eq!(color_label(name), n, "{name}");
        }
    }

    #[test]
    fn a_catalog_open_in_lightroom_still_reads_from_its_copy() {
        let dir = tempfile::tempdir().unwrap();
        let catalog = sample(dir.path());
        // Lightroom keeps the catalog in WAL mode and holds a write lock while it is open.
        let lr = Connection::open(&catalog).unwrap();
        lr.pragma_update(None, "journal_mode", "WAL").unwrap();
        lr.execute("INSERT INTO AgLibraryKeyword (id_local, name, parent) VALUES (22, 'committed', 1)", []).unwrap();
        lr.execute("INSERT INTO AgLibraryKeywordImage (image, tag) VALUES (10, 22)", []).unwrap();
        lr.execute_batch("BEGIN IMMEDIATE; INSERT INTO AgLibraryKeyword (id_local, name) VALUES (23, 'uncommitted');").unwrap();
        assert!(catalog.with_file_name("Lightroom Catalog.lrcat-wal").exists());
        let before = std::fs::read(&catalog).unwrap();

        let source = LightroomCatalogSource::open(&catalog).unwrap();
        let items = source.enumerate().unwrap();
        let x = source.extras(&items[0]).unwrap();
        assert!(x.keywords.contains(&"committed".to_string()), "what the WAL holds is read: {:?}", x.keywords);
        assert!(!x.keywords.contains(&"uncommitted".to_string()));
        assert_eq!(std::fs::read(&catalog).unwrap(), before, "the catalog itself is untouched");
        lr.execute_batch("ROLLBACK").unwrap();
    }

    #[test]
    fn what_is_no_catalog_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let not = dir.path().join("x.lrcat");
        std::fs::write(&not, b"not a database").unwrap();
        assert!(LightroomCatalogSource::open(&not).is_err());
        assert!(LightroomCatalogSource::open(&dir.path().join("gone.lrcat")).is_err());
    }
}
