//! Walks a Google Takeout export over [`Fs`] and returns every photo and video with the
//! metadata Google keeps in its JSON sidecars.
//!
//! Layout: `[Takeout/]Google Photos/<folder>/<media>` plus `<media>.supplemental-metadata.json`
//! (see `names`). Folders named `Photos from YYYY` are the year view of the whole library; every
//! other folder is an album, named by its `metadata.json` `title` when it has one.
//!
//! Only JSON files are ever read; media bytes go to the host by `path`.
//!
//! Duplicates: a photo in an album and in its year folder is one item. Identity is
//! `media name + size`, with photoTakenTime as a tiebreaker only when both copies have a
//! sidecar (never drops a same-name photo of another size or time). The item is
//! kept once, at its first album copy's path (folders in name order) when it has one, with every album it is in.
//!
//! `captured_at` format: RFC 3339 in UTC (`2019-07-04T12:00:00Z`). The host reads that as a
//! moment with offset 0 and applies it over the file's own date (`photos-core::extras::apply`
//! runs after ingest's EXIF choice), so it outranks EXIF; Takeout's value is Google's own
//! (UTC) idea of when the photo was taken. That would shift a photo whose EXIF holds the camera's
//! wall-clock time, so `captured_at` is sent only for formats without EXIF (video, PNG, GIF, WebP,
//! BMP); the rest keep the file's own date.

use crate::fs::{join, Entry, Fs};
use crate::names::{edited_original, is_year_folder, sidecar_candidates};
use serde_json::Value;
use std::collections::{HashMap, HashSet};

/// Mirrors the WIT `item` record (the SDK's type exists only on wasm32).
#[derive(Debug, Clone, PartialEq)]
pub struct ScanItem {
    pub key: String,
    pub name: String,
    pub size: u64,
    pub modified: Option<i64>,
    /// The real path in the chosen tree (host fast path).
    pub path: Option<String>,
}

/// Mirrors the WIT `extras` record.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ScanExtras {
    pub favourite: bool,
    pub albums: Vec<String>,
    pub title: Option<String>,
    pub caption: Option<String>,
    pub keywords: Vec<String>,
    pub captured_at: Option<String>,
    pub lat: Option<f64>,
    pub lon: Option<f64>,
}

const MEDIA_EXT: &[&str] = &[
    "jpg", "jpeg", "png", "gif", "heic", "heif", "webp", "tif", "tiff", "bmp", "dng", "raw", "nef", "cr2", "cr3",
    "arw", "orf", "rw2", "raf", "srw", "avif", "mp4", "mov", "m4v", "avi", "3gp", "3g2", "mkv", "webm", "mts",
    "m2ts", "mpg", "mpeg", "wmv",
];

fn is_media(name: &str) -> bool {
    match name.rfind('.') {
        Some(i) if i > 0 => MEDIA_EXT.contains(&name[i + 1..].to_lowercase().as_str()),
        _ => false,
    }
}

/// The folder holding the albums: `Google Photos` under the chosen folder or under `Takeout`,
/// or the chosen folder itself when it is Google Photos.
fn find_root(fs: &dyn Fs) -> String {
    let find = |dir: &str, want: &str| -> Option<String> {
        fs.list(dir)
            .ok()?
            .into_iter()
            .find(|e| e.is_dir && e.name.eq_ignore_ascii_case(want))
            .map(|e| join(dir, &e.name))
    };
    if let Some(g) = find("", "Google Photos") {
        return g;
    }
    if let Some(t) = find("", "Takeout") {
        if let Some(g) = find(&t, "Google Photos") {
            return g;
        }
    }
    String::new()
}

fn timestamp(v: &Value) -> Option<i64> {
    let t = v.get("photoTakenTime")?.get("timestamp")?;
    t.as_str().and_then(|s| s.trim().parse().ok()).or_else(|| t.as_i64())
}

/// Days since 1970-01-01 to (year, month, day), proleptic Gregorian.
fn civil(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

fn rfc3339_utc(ts: i64) -> String {
    let (y, m, d) = civil(ts.div_euclid(86_400));
    let s = ts.rem_euclid(86_400);
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z", s / 3600, s % 3600 / 60, s % 60)
}

fn coords(v: &Value, key: &str) -> Option<(f64, f64)> {
    let g = v.get(key)?;
    Some((g.get("latitude")?.as_f64()?, g.get("longitude")?.as_f64()?))
}

/// `geoData`; when that is 0, 0 (Google's "unknown") `geoDataExif`; when that too, none.
fn geo(v: &Value) -> Option<(f64, f64)> {
    let nonzero = |c: &(f64, f64)| !(c.0 == 0.0 && c.1 == 0.0);
    coords(v, "geoData").filter(nonzero).or_else(|| coords(v, "geoDataExif").filter(nonzero))
}

fn text(v: &Value, key: &str) -> Option<String> {
    v.get(key)?.as_str().map(str::trim).filter(|s| !s.is_empty()).map(str::to_string)
}

/// Formats whose files carry their own capture time, which the host must keep.
fn has_exif(name: &str) -> bool {
    const EXIF: &[&str] = &["jpg", "jpeg", "heic", "heif", "avif", "tif", "tiff", "dng", "raw", "nef", "cr2", "cr3", "arw", "orf", "rw2", "raf", "srw"];
    name.rsplit_once('.').is_some_and(|(_, e)| EXIF.contains(&e.to_lowercase().as_str()))
}

fn extras_from(v: &Value) -> ScanExtras {
    let (lat, lon) = geo(v).map_or((None, None), |(a, b)| (Some(a), Some(b)));
    ScanExtras {
        favourite: v.get("favorited").and_then(Value::as_bool).unwrap_or(false),
        albums: Vec::new(),
        title: text(v, "title"),
        caption: text(v, "description"),
        keywords: Vec::new(),
        captured_at: timestamp(v).map(rfc3339_utc),
        lat,
        lon,
    }
}

struct Found {
    entry: Entry,
    path: String,
    /// Set when the file is inside a zip.
    vkey: Option<String>,
    album: Option<String>,
    /// What the photo's sidecar said, reduced as soon as it was read (the parsed JSON is large).
    meta: Option<(Option<i64>, ScanExtras)>,
}

fn read_json(fs: &dyn Fs, path: &str) -> Option<Value> {
    serde_json::from_slice(&fs.read(path).ok()?).ok()
}

fn walk(fs: &dyn Fs, dir: &str, folder: &str, albums: bool, out: &mut Vec<Found>) {
    let Ok(mut entries) = fs.list(dir) else { return };
    // Listing order is the file system's; sort so paths and keys are the same every run.
    entries.sort_by(|a, b| a.name.cmp(&b.name));
    let names: HashSet<&str> = entries.iter().filter(|e| !e.is_dir).map(|e| e.name.as_str()).collect();
    let album = if !albums || folder.is_empty() || is_year_folder(folder).is_some() {
        None
    } else {
        let title = names
            .contains("metadata.json")
            .then(|| read_json(fs, &join(dir, "metadata.json")))
            .flatten()
            .and_then(|v| text(&v, "title"));
        Some(title.unwrap_or_else(|| folder.to_string()))
    };
    for e in &entries {
        if e.is_dir {
            walk(fs, &join(dir, &e.name), &e.name, albums, out);
        } else if is_media(&e.name) {
            // An edited copy takes the original's sidecar.
            let base = edited_original(&e.name).unwrap_or_else(|| e.name.clone());
            let mut meta = None;
            for c in sidecar_candidates(&base) {
                if names.contains(c.as_str()) {
                    if let Some(j) = read_json(fs, &join(dir, &c)) {
                        meta = Some((timestamp(&j), extras_from(&j)));
                        break;
                    }
                }
            }
            let path = join(dir, &e.name);
            out.push(Found { entry: e.clone(), vkey: fs.virtual_key(&path), path, album: album.clone(), meta });
        }
    }
}

/// Every photo and video in the export, once each, with its extras. `albums_from_folders`
/// false leaves `albums` empty.
pub fn scan(fs: &dyn Fs, albums_from_folders: bool) -> Vec<(ScanItem, ScanExtras)> {
    let root = find_root(fs);
    let mut found = Vec::new();
    walk(fs, &root, "", albums_from_folders, &mut found);

    // Identity: the same media name and the same size (the listing always has both). When both
    // copies have a sidecar, their photoTakenTime must also agree; a copy without a sidecar
    // matches either. A same-name photo of another size or time stays its own item.
    struct Slot {
        at: usize,
        when: Option<i64>,
        has_json: bool,
    }
    let mut index: HashMap<(String, u64), Vec<Slot>> = HashMap::new();
    let mut out: Vec<(ScanItem, ScanExtras)> = Vec::new();
    for f in found {
        let has_json = f.meta.is_some();
        let (when, mut extras) = f.meta.unwrap_or_default();
        if has_exif(&f.entry.name) {
            extras.captured_at = None;
        }
        let item = ScanItem {
            key: f.vkey.clone().unwrap_or_else(|| f.path.clone()),
            name: f.entry.name.clone(),
            size: f.entry.size,
            modified: f.entry.modified,
            path: f.vkey.is_none().then_some(f.path),
        };
        let slots = index.entry((f.entry.name.clone(), f.entry.size)).or_default();
        let hit = slots.iter_mut().find(|s| s.when.is_none() || when.is_none() || s.when == when);
        match hit {
            Some(slot) => {
                let (it, ex) = &mut out[slot.at];
                let mut albums = std::mem::take(&mut ex.albums);
                let first_album = f.album.is_some() && albums.is_empty();
                if let Some(a) = f.album {
                    if !albums.contains(&a) {
                        albums.push(a);
                    }
                }
                // The first album copy becomes the item's path; its own sidecar wins, else the
                // earlier copy's extras stay. A copy with a sidecar fills in for one without.
                if first_album {
                    *it = item;
                }
                if has_json && (first_album || !slot.has_json) {
                    *ex = extras;
                }
                ex.albums = albums;
                slot.has_json |= has_json;
                slot.when = slot.when.or(when);
            }
            None => {
                let mut extras = extras;
                extras.albums.extend(f.album);
                slots.push(Slot { at: out.len(), when, has_json });
                out.push((item, extras));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::path::PathBuf;

    struct TempFs {
        root: PathBuf,
        reads: RefCell<Vec<String>>,
    }

    impl TempFs {
        fn new(tag: &str) -> TempFs {
            let root = std::env::temp_dir().join(format!("lumen-takeout-{tag}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&root);
            std::fs::create_dir_all(&root).unwrap();
            TempFs { root, reads: RefCell::new(Vec::new()) }
        }
        fn put(&self, rel: &str, bytes: &[u8]) {
            let p = self.root.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, bytes).unwrap();
        }
    }

    impl Drop for TempFs {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    impl Fs for TempFs {
        fn list(&self, dir: &str) -> Result<Vec<Entry>, String> {
            let mut v = Vec::new();
            for e in std::fs::read_dir(self.root.join(dir)).map_err(|e| e.to_string())? {
                let e = e.map_err(|e| e.to_string())?;
                let m = e.metadata().map_err(|e| e.to_string())?;
                v.push(Entry { name: e.file_name().to_string_lossy().into_owned(), is_dir: m.is_dir(), size: m.len(), modified: None });
            }
            Ok(v)
        }
        fn read(&self, path: &str) -> Result<Vec<u8>, String> {
            self.reads.borrow_mut().push(path.to_string());
            std::fs::read(self.root.join(path)).map_err(|e| e.to_string())
        }
    }

    const GP: &str = "Takeout/Google Photos";

    fn sidecar(ts: &str, geo: (f64, f64), exif: (f64, f64), fav: bool) -> String {
        format!(
            r#"{{"title":"IMG.jpg","description":"On the steps","favorited":{fav},"photoTakenTime":{{"timestamp":"{ts}"}},
"geoData":{{"latitude":{},"longitude":{}}},"geoDataExif":{{"latitude":{},"longitude":{}}}}}"#,
            geo.0, geo.1, exif.0, exif.1
        )
    }

    fn by_name<'a>(v: &'a [(ScanItem, ScanExtras)], n: &str) -> &'a (ScanItem, ScanExtras) {
        v.iter().find(|(i, _)| i.name == n).unwrap_or_else(|| panic!("no {n} in {:?}", v.iter().map(|x| &x.0.name).collect::<Vec<_>>()))
    }

    fn takeout(tag: &str) -> TempFs {
        let t = TempFs::new(tag);
        let one = sidecar("1562241600", (41.9, 12.5), (0.0, 0.0), true);
        t.put(&format!("{GP}/Photos from 2019/IMG_1.jpg"), b"jpg1");
        t.put(&format!("{GP}/Photos from 2019/IMG_1.jpg.supplemental-metadata.json"), one.as_bytes());
        t.put(&format!("{GP}/Photos from 2019/IMG_2.jpg"), b"jpg22");
        t.put(&format!("{GP}/Photos from 2019/IMG_2.jpg.supplemental-metadata.json"), sidecar("1562241700", (0.0, 0.0), (0.0, 0.0), false).as_bytes());
        t.put(&format!("{GP}/Trip to Rome/IMG_1.jpg"), b"jpg1");
        t.put(&format!("{GP}/Trip to Rome/IMG_1.jpg.supplemental-metadata.json"), one.as_bytes());
        t.put(&format!("{GP}/Trip to Rome/metadata.json"), br#"{"title":"Trip to Rome"}"#);
        t.put(&format!("{GP}/Archive/IMG_1.jpg"), b"jpg1");
        t.put(&format!("{GP}/Archive/IMG_1.jpg.supplemental-metadata.json"), one.as_bytes());
        t.put(&format!("{GP}/Archive/print-subscriptions.json"), b"{}");
        t.put(&format!("{GP}/Archive/shared_album_comments.json"), b"{}");
        t
    }

    #[test]
    fn items_have_paths() {
        let t = takeout("paths");
        let v = scan(&t, true);
        let (i, _) = by_name(&v, "IMG_2.jpg");
        assert_eq!(i.path.as_deref(), Some("Takeout/Google Photos/Photos from 2019/IMG_2.jpg"));
        assert_eq!(i.key, i.path.clone().unwrap());
        assert_eq!(i.size, 5);
        assert!(v.iter().all(|(i, _)| !i.name.ends_with(".json")));
        // Only sidecars and metadata.json are read.
        let reads = t.reads.borrow();
        assert!(!reads.is_empty() && reads.iter().all(|r| r.ends_with(".json")), "{reads:?}");
    }

    #[test]
    fn exif_date_wins() {
        let t = TempFs::new("exifdate");
        let d = format!("{GP}/Photos from 2019");
        for n in ["a.png", "b.mp4", "c.heic", "d.JPG"] {
            t.put(&format!("{d}/{n}"), n.as_bytes());
            t.put(&format!("{d}/{n}.supplemental-metadata.json"), sidecar("1562241600", (0.0, 0.0), (0.0, 0.0), false).as_bytes());
        }
        let v = scan(&t, true);
        for n in ["a.png", "b.mp4"] {
            assert_eq!(by_name(&v, n).1.captured_at.as_deref(), Some("2019-07-04T12:00:00Z"), "{n}");
        }
        for n in ["c.heic", "d.JPG"] {
            assert_eq!(by_name(&v, n).1.captured_at, None, "{n}");
        }
    }

    #[test]
    fn extras_mapped() {
        let t = takeout("extras");
        let v = scan(&t, true);
        let (_, x) = by_name(&v, "IMG_1.jpg");
        // A JPEG keeps its EXIF date (`exif_date_wins`): no date is sent for it.
        assert_eq!(x.captured_at, None);
        assert_eq!((x.lat, x.lon), (Some(41.9), Some(12.5)));
        assert_eq!(x.title.as_deref(), Some("IMG.jpg"));
        assert_eq!(x.caption.as_deref(), Some("On the steps"));
        assert!(x.favourite);
        assert!(x.albums.contains(&"Trip to Rome".to_string()));
    }

    #[test]
    fn album_and_year_once() {
        let t = takeout("once");
        let v = scan(&t, true);
        assert_eq!(v.iter().filter(|(i, _)| i.name == "IMG_1.jpg").count(), 1);
        let (i, x) = by_name(&v, "IMG_1.jpg");
        assert_eq!(x.albums.len(), 2, "{:?}", x.albums);
        assert!(x.albums.contains(&"Trip to Rome".to_string()) && x.albums.contains(&"Archive".to_string()));
        assert!(!i.path.as_deref().unwrap().contains("Photos from"));
        // The year-only photo has no album.
        assert!(by_name(&v, "IMG_2.jpg").1.albums.is_empty());
        assert_eq!(v.len(), 2);
    }

    #[test]
    fn no_sidecar_ok() {
        let t = takeout("nosc");
        t.put(&format!("{GP}/Archive/LOOSE.png"), b"png");
        let v = scan(&t, true);
        let (i, x) = by_name(&v, "LOOSE.png");
        assert_eq!(i.size, 3);
        assert_eq!(x.captured_at, None);
        assert_eq!(x.albums, vec!["Archive".to_string()]);
        assert!(!x.favourite);
    }

    #[test]
    fn albums_setting_off() {
        let t = takeout("off");
        let v = scan(&t, false);
        assert_eq!(v.len(), 2);
        assert!(v.iter().all(|(_, x)| x.albums.is_empty()));
    }

    #[test]
    fn geo_zero_is_none() {
        let t = TempFs::new("geo");
        t.put(&format!("{GP}/Photos from 2020/A.jpg"), b"a");
        t.put(&format!("{GP}/Photos from 2020/A.jpg.supplemental-metadata.json"), sidecar("1", (0.0, 0.0), (0.0, 0.0), false).as_bytes());
        t.put(&format!("{GP}/Photos from 2020/B.jpg"), b"b");
        t.put(&format!("{GP}/Photos from 2020/B.jpg.supplemental-metadata.json"), sidecar("2", (0.0, 0.0), (46.0, 14.0), false).as_bytes());
        let v = scan(&t, true);
        let a = &by_name(&v, "A.jpg").1;
        assert_eq!((a.lat, a.lon), (None, None));
        let b = &by_name(&v, "B.jpg").1;
        assert_eq!((b.lat, b.lon), (Some(46.0), Some(14.0)));
    }

    #[test]
    fn edited_takes_original_json() {
        let t = TempFs::new("edited");
        let d = format!("{GP}/Photos from 2019");
        t.put(&format!("{d}/IMG_1.jpg"), b"o");
        t.put(&format!("{d}/IMG_1-edited.jpg"), b"e");
        t.put(&format!("{d}/IMG_1.jpg.supplemental-metadata.json"), sidecar("1562241600", (1.0, 2.0), (0.0, 0.0), true).as_bytes());
        t.put(&format!("{d}/IMG_3(1).jpg"), b"o");
        t.put(&format!("{d}/IMG_3-edited(1).jpg"), b"e");
        t.put(&format!("{d}/IMG_3.jpg.supplemental-metadata(1).json"), sidecar("1562241700", (3.0, 4.0), (0.0, 0.0), false).as_bytes());
        let v = scan(&t, true);
        assert_eq!(v.len(), 4);
        let e = by_name(&v, "IMG_1-edited.jpg");
        assert_eq!(e.1.captured_at, None);
        assert!(e.1.favourite);
        let e = by_name(&v, "IMG_3-edited(1).jpg");
        assert_eq!(e.1.lat, Some(3.0));
    }

    #[test]
    fn same_name_different_content_kept() {
        let t = TempFs::new("diff");
        let d = format!("{GP}/Photos from 2019");
        t.put(&format!("{d}/IMG_1.jpg"), b"aaaa");
        t.put(&format!("{d}/IMG_1.jpg.supplemental-metadata.json"), sidecar("100", (0.0, 0.0), (0.0, 0.0), false).as_bytes());
        t.put(&format!("{GP}/Other/IMG_1.jpg"), b"bbbbbb");
        t.put(&format!("{GP}/Other/IMG_1.jpg.supplemental-metadata.json"), sidecar("100", (0.0, 0.0), (0.0, 0.0), false).as_bytes());
        // Same name and size but another time is another photo too.
        t.put(&format!("{GP}/Third/IMG_1.jpg"), b"cccc");
        t.put(&format!("{GP}/Third/IMG_1.jpg.supplemental-metadata.json"), sidecar("200", (0.0, 0.0), (0.0, 0.0), false).as_bytes());
        assert_eq!(scan(&t, true).len(), 3);
    }

    #[test]
    fn sidecar_only_in_year_folder_deduped() {
        let t = TempFs::new("halfsc");
        t.put(&format!("{GP}/Photos from 2019/IMG_1.jpg"), b"aaaa");
        t.put(&format!("{GP}/Photos from 2019/IMG_1.jpg.supplemental-metadata.json"), sidecar("100", (5.0, 6.0), (0.0, 0.0), true).as_bytes());
        t.put(&format!("{GP}/Trip/IMG_1.jpg"), b"aaaa");
        let v = scan(&t, true);
        assert_eq!(v.len(), 1);
        let (i, x) = &v[0];
        assert!(i.path.as_deref().unwrap().contains("Trip"));
        assert_eq!(x.albums, vec!["Trip".to_string()]);
        assert_eq!(x.lat, Some(5.0));
        assert!(x.favourite);
    }

    struct Reversed(TempFs);
    impl Fs for Reversed {
        fn list(&self, dir: &str) -> Result<Vec<Entry>, String> {
            let mut v = self.0.list(dir)?;
            v.reverse();
            Ok(v)
        }
        fn read(&self, path: &str) -> Result<Vec<u8>, String> {
            self.0.read(path)
        }
    }

    #[test]
    fn order_is_deterministic() {
        let t = takeout("order");
        let a = scan(&t, true);
        let r = Reversed(t);
        let b = scan(&r, true);
        assert_eq!(a, b);
        assert!(a.iter().any(|(i, x)| i.path.as_deref() == Some("Takeout/Google Photos/Archive/IMG_1.jpg") && x.albums == ["Archive", "Trip to Rome"]));
    }

    #[test]
    fn chosen_is_google_photos_itself() {
        let t = TempFs::new("self");
        t.put("Photos from 2019/A.jpg", b"a");
        t.put("Trip/A.jpg", b"a");
        let v = scan(&t, true);
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].1.albums, vec!["Trip".to_string()]);
    }
}
