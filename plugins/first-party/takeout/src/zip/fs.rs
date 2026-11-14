//! [`Fs`] over zips. Every zip's entries are merged into one tree by their path inside the zip,
//! so a photo in `takeout-001.zip` and its sidecar in `-002.zip` share a directory and pair in
//! `scan`. [`OverlayFs`] adds a folder's real files to that tree (a folder holding both unpacked
//! files and zips), each path once, the real file first.
//!
//! Only small entries are ever read whole (sidecars); media bytes are served in ranges by
//! [`super::Streams`] under the key [`Fs::virtual_key`] returns.

use super::stream::ZipSet;
use crate::fs::{join, Entry, Fs};
use std::cell::RefCell;
use std::collections::{BTreeMap, HashSet};
use std::rc::Rc;

/// The most a whole-entry read inflates into memory: sidecars are a few kilobytes.
const READ_CAP: u64 = 16 << 20;

enum Node {
    Dir,
    File { zi: usize, ei: usize },
}

pub struct ZipFs {
    set: Rc<ZipSet>,
    dirs: BTreeMap<String, BTreeMap<String, Node>>,
    skipped: Vec<(String, String)>,
}

impl ZipFs {
    pub fn new(set: Rc<ZipSet>) -> ZipFs {
        let mut dirs: BTreeMap<String, BTreeMap<String, Node>> = BTreeMap::new();
        let mut skipped = Vec::new();
        dirs.insert(String::new(), BTreeMap::new());
        for (zi, zip) in set.zips.iter().enumerate() {
            for (ei, e) in zip.iter().enumerate() {
                if let Some(reason) = &e.skip {
                    skipped.push((e.name.clone(), reason.clone()));
                    continue;
                }
                let parts: Vec<&str> = e.name.split('/').filter(|p| !p.is_empty()).collect();
                let Some((file, parents)) = parts.split_last() else { continue };
                let mut dir = String::new();
                for p in parents {
                    dirs.entry(dir.clone()).or_default().entry((*p).to_string()).or_insert(Node::Dir);
                    dir = join(&dir, p);
                    dirs.entry(dir.clone()).or_default();
                }
                // The first zip to hold a path keeps it.
                dirs.entry(dir).or_default().entry((*file).to_string()).or_insert(Node::File { zi, ei });
            }
        }
        ZipFs { set, dirs, skipped }
    }

    /// Entries left out, as (name, reason): encrypted, unsupported, cut off.
    pub fn skipped(&self) -> Vec<(String, String)> {
        self.skipped.clone()
    }

    fn file(&self, path: &str) -> Option<(usize, usize)> {
        let (dir, name) = path.rsplit_once('/').unwrap_or(("", path));
        match self.dirs.get(dir)?.get(name)? {
            Node::File { zi, ei } => Some((*zi, *ei)),
            Node::Dir => None,
        }
    }
}

impl Fs for ZipFs {
    fn list(&self, dir: &str) -> Result<Vec<Entry>, String> {
        let d = self.dirs.get(dir).ok_or("no such folder")?;
        Ok(d.iter()
            .map(|(name, n)| match n {
                Node::Dir => Entry { name: name.clone(), is_dir: true, size: 0, modified: None },
                Node::File { zi, ei } => Entry { name: name.clone(), is_dir: false, size: self.set.zips[*zi][*ei].size, modified: self.set.zips[*zi][*ei].modified },
            })
            .collect())
    }

    fn read(&self, path: &str) -> Result<Vec<u8>, String> {
        let (zi, ei) = self.file(path).ok_or("no such file")?;
        self.set.read_whole(zi, ei, READ_CAP)
    }

    fn virtual_key(&self, path: &str) -> Option<String> {
        self.file(path).map(|(zi, ei)| self.set.key_of(zi, ei))
    }
}

/// A real folder and the zips' tree as one. Paths in both are listed once and read from the real
/// side; `virtual_key` answers only for paths that exist only in the zips.
pub struct OverlayFs<'a> {
    real: &'a dyn Fs,
    zips: &'a dyn Fs,
    /// Paths seen in a listing that only the zips hold.
    zip_only: RefCell<HashSet<String>>,
}

impl<'a> OverlayFs<'a> {
    pub fn new(real: &'a dyn Fs, zips: &'a dyn Fs) -> OverlayFs<'a> {
        OverlayFs { real, zips, zip_only: RefCell::new(HashSet::new()) }
    }
}

impl Fs for OverlayFs<'_> {
    fn list(&self, dir: &str) -> Result<Vec<Entry>, String> {
        let real = self.real.list(dir);
        let zips = self.zips.list(dir);
        if let (Err(e), Err(_)) = (&real, &zips) {
            return Err(e.clone());
        }
        let mut out = real.unwrap_or_default();
        let names: HashSet<(String, bool)> = out.iter().map(|e| (e.name.clone(), e.is_dir)).collect();
        for e in zips.unwrap_or_default() {
            if names.contains(&(e.name.clone(), e.is_dir)) {
                continue;
            }
            if !e.is_dir {
                self.zip_only.borrow_mut().insert(join(dir, &e.name));
            }
            out.push(e);
        }
        Ok(out)
    }

    fn read(&self, path: &str) -> Result<Vec<u8>, String> {
        if self.zip_only.borrow().contains(path) {
            self.zips.read(path)
        } else {
            self.real.read(path).or_else(|e| self.zips.read(path).map_err(|_| e))
        }
    }

    fn virtual_key(&self, path: &str) -> Option<String> {
        if self.zip_only.borrow().contains(path) {
            self.zips.virtual_key(path)
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scan::scan;
    use crate::zip::{make_key, read_index, Streams};
    use std::cell::Cell;
    use std::io::{Cursor, Write};
    use std::path::PathBuf;
    use zip::write::SimpleFileOptions;
    use zip::CompressionMethod;

    const GP: &str = "Takeout/Google Photos/Photos from 2019";

    fn make(entries: &[(&str, &[u8], CompressionMethod)]) -> Vec<u8> {
        let mut w = zip::ZipWriter::new(Cursor::new(Vec::new()));
        for (n, d, m) in entries {
            w.start_file(*n, SimpleFileOptions::default().compression_method(*m)).unwrap();
            w.write_all(d).unwrap();
        }
        w.finish().unwrap().into_inner()
    }

    /// A real folder in the temp dir.
    struct Dir(PathBuf);
    impl Dir {
        fn new(tag: &str) -> Dir {
            let p = std::env::temp_dir().join(format!("lumen-takeout-zip-{tag}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&p);
            std::fs::create_dir_all(&p).unwrap();
            Dir(p)
        }
        fn put(&self, rel: &str, b: &[u8]) {
            let p = self.0.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, b).unwrap();
        }
    }
    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    impl Fs for Dir {
        fn list(&self, dir: &str) -> Result<Vec<Entry>, String> {
            let mut v = Vec::new();
            for e in std::fs::read_dir(self.0.join(dir)).map_err(|e| e.to_string())? {
                let e = e.map_err(|e| e.to_string())?;
                let m = e.metadata().map_err(|e| e.to_string())?;
                v.push(Entry { name: e.file_name().to_string_lossy().into_owned(), is_dir: m.is_dir(), size: m.len(), modified: None });
            }
            Ok(v)
        }
        fn read(&self, path: &str) -> Result<Vec<u8>, String> {
            std::fs::read(self.0.join(path)).map_err(|e| e.to_string())
        }
    }

    fn set_of(zips: Vec<Vec<u8>>) -> Rc<ZipSet> {
        let names = (0..zips.len()).map(|i| format!("t{i}.zip")).collect();
        set_named(names, zips)
    }

    fn set_named(names: Vec<String>, zips: Vec<Vec<u8>>) -> Rc<ZipSet> {
        let index = zips
            .iter()
            .map(|z| {
                let r = |o: u64, n: u32| {
                    let s = (o as usize).min(z.len());
                    z[s..(s + n as usize).min(z.len())].to_vec()
                };
                read_index(&r, z.len() as u64).unwrap_or_default()
            })
            .collect();
        Rc::new(ZipSet::new(
            names,
            index,
            Box::new(move |zi, o, n| {
                let z = &zips[zi];
                let s = (o as usize).min(z.len());
                z[s..(s + n as usize).min(z.len())].to_vec()
            }),
        ))
    }

    /// A zip written by hand: (name, flags, method, local extra length, data). The central
    /// directory has no extra, so the local header's lengths differ from it.
    fn raw_zip(entries: &[(&str, u16, u16, usize, &[u8])]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut cd = Vec::new();
        for (name, flags, method, extra, data) in entries {
            let off = out.len() as u32;
            out.extend(0x0403_4b50u32.to_le_bytes());
            out.extend([20u8, 0]);
            out.extend(flags.to_le_bytes());
            out.extend(method.to_le_bytes());
            out.extend([0u8; 8]);
            out.extend((data.len() as u32).to_le_bytes());
            out.extend((data.len() as u32).to_le_bytes());
            out.extend((name.len() as u16).to_le_bytes());
            out.extend((*extra as u16).to_le_bytes());
            out.extend(name.as_bytes());
            out.extend(vec![0xEEu8; *extra]);
            out.extend(*data);
            cd.extend(0x0201_4b50u32.to_le_bytes());
            cd.extend([20u8, 0, 20, 0]);
            cd.extend(flags.to_le_bytes());
            cd.extend(method.to_le_bytes());
            cd.extend([0u8; 8]);
            cd.extend((data.len() as u32).to_le_bytes());
            cd.extend((data.len() as u32).to_le_bytes());
            cd.extend((name.len() as u16).to_le_bytes());
            cd.extend([0u8; 12]);
            cd.extend(off.to_le_bytes());
            cd.extend(name.as_bytes());
        }
        let cd_off = out.len() as u32;
        out.extend(&cd);
        out.extend(0x0605_4b50u32.to_le_bytes());
        out.extend([0u8; 4]);
        out.extend((entries.len() as u16).to_le_bytes());
        out.extend((entries.len() as u16).to_le_bytes());
        out.extend((cd.len() as u32).to_le_bytes());
        out.extend(cd_off.to_le_bytes());
        out.extend([0u8; 2]);
        out
    }

    fn sidecar(fav: bool) -> String {
        format!(r#"{{"title":"x","favorited":{fav},"photoTakenTime":{{"timestamp":"1562241600"}}}}"#)
    }

    /// Compressible but not trivial: 16 symbols from a xorshift stream.
    fn noise(n: usize) -> Vec<u8> {
        let mut x = 0x2545_F491_4F6C_DD1Du64;
        (0..n)
            .map(|_| {
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                (x & 0x0f) as u8
            })
            .collect()
    }

    #[test]
    fn cross_zip_pairing() {
        let a = make(&[(&format!("{GP}/IMG_1.png"), b"pngbytes", CompressionMethod::Stored)]);
        let b = make(&[(&format!("{GP}/IMG_1.png.supplemental-metadata.json"), sidecar(true).as_bytes(), CompressionMethod::Deflated)]);
        let fs = ZipFs::new(set_of(vec![a, b]));
        let v = scan(&fs, true);
        assert_eq!(v.len(), 1, "{v:?}");
        let (i, x) = &v[0];
        assert_eq!(i.name, "IMG_1.png");
        assert_eq!(i.size, 8);
        assert_eq!(i.path, None);
        assert_eq!(i.key, make_key("t0.zip", &format!("{GP}/IMG_1.png")));
        assert!(x.favourite);
        assert_eq!(x.captured_at.as_deref(), Some("2019-07-04T12:00:00Z"));
    }

    #[test]
    fn backwards_offset_restarts() {
        let data = noise(9 << 20);
        let z = make(&[("a/big.mp4", &data, CompressionMethod::Deflated)]);
        let set = set_of(vec![z]);
        let st = Streams::new(set);
        let key = make_key("t0.zip", "a/big.mp4");
        const MB4: u32 = 4 << 20;
        let c0 = st.read(&key, 0, MB4).unwrap();
        let c1 = st.read(&key, MB4 as u64, MB4).unwrap();
        let c2 = st.read(&key, 2 * MB4 as u64, MB4).unwrap();
        assert_eq!(c0, &data[..MB4 as usize]);
        assert_eq!(c1, &data[MB4 as usize..2 * MB4 as usize]);
        assert_eq!(c2, &data[2 * MB4 as usize..]);
        // The host retries an earlier chunk, then asks past the end.
        assert_eq!(st.read(&key, MB4 as u64, MB4).unwrap(), c1);
        assert_eq!(st.read(&key, 5, 100).unwrap(), &data[5..105]);
        assert_eq!(st.read(&key, 0, MB4).unwrap(), c0);
        assert!(st.read(&key, data.len() as u64, 10).unwrap().is_empty());
    }

    #[test]
    fn stored_and_deflate_equal() {
        let data = noise(300_000);
        let z = make(&[("s.bin", &data, CompressionMethod::Stored), ("d.bin", &data, CompressionMethod::Deflated)]);
        let set = set_of(vec![z]);
        let st = Streams::new(set.clone());
        let stored = set.zips[0].iter().position(|e| e.name == "s.bin").unwrap();
        let defl = set.zips[0].iter().position(|e| e.name == "d.bin").unwrap();
        for ei in [stored, defl] {
            let k = make_key("t0.zip", if ei == stored { "s.bin" } else { "d.bin" });
            let mut got = Vec::new();
            let mut off = 0u64;
            loop {
                let c = st.read(&k, off, 70_001).unwrap();
                if c.is_empty() {
                    break;
                }
                off += c.len() as u64;
                got.extend(c);
            }
            assert_eq!(got, data, "entry {ei}");
            assert_eq!(st.read(&k, 1234, 99).unwrap(), &data[1234..1333]);
            assert_eq!(set.read_whole(0, ei, 1 << 20).unwrap(), data);
        }
        assert!(set.read_whole(0, defl, 1000).is_err());
    }

    #[test]
    fn mixed_folder_once() {
        let d = Dir::new("mixed");
        d.put(&format!("{GP}/IMG_1.jpg"), b"jpg1");
        d.put(&format!("{GP}/IMG_1.jpg.supplemental-metadata.json"), sidecar(false).as_bytes());
        let z = make(&[
            (&format!("{GP}/IMG_1.jpg"), b"jpg1", CompressionMethod::Stored),
            (&format!("{GP}/IMG_2.jpg"), b"jpg22", CompressionMethod::Deflated),
            (&format!("{GP}/IMG_2.jpg.supplemental-metadata.json"), sidecar(true).as_bytes(), CompressionMethod::Deflated),
        ]);
        let zfs = ZipFs::new(set_of(vec![z]));
        let both = OverlayFs::new(&d, &zfs);
        let v = scan(&both, true);
        assert_eq!(v.len(), 2, "{v:?}");
        let one = v.iter().find(|(i, _)| i.name == "IMG_1.jpg").unwrap();
        let two = v.iter().find(|(i, _)| i.name == "IMG_2.jpg").unwrap();
        assert_eq!(one.0.path.as_deref(), Some(format!("{GP}/IMG_1.jpg").as_str()));
        assert_eq!(two.0.path, None);
        assert!(two.0.key.starts_with("zip:"));
        assert!(two.1.favourite && !one.1.favourite);
    }

    #[test]
    fn keys_survive_a_new_earlier_zip_and_a_failed_index() {
        let data = noise(100_000);
        let b = make(&[("p/one.bin", &data, CompressionMethod::Deflated)]);
        let before = set_named(vec!["b.zip".into()], vec![b.clone()]);
        let key = before.key_of(0, 0);
        // A zip that sorts earlier appears; another one's index fails (an empty list).
        let a = make(&[("p/one.bin", b"WRONG", CompressionMethod::Stored)]);
        let after = set_named(vec!["a.zip".into(), "b.zip".into(), "c.zip".into()], vec![a, b, b"not a zip at all, really".to_vec()]);
        assert_eq!(after.zips[2].len(), 0);
        assert_eq!(Streams::new(after).read(&key, 10, 50).unwrap(), &data[10..60]);
        let st = Streams::new(before);
        assert!(st.read(&make_key("zzz.zip", "p/one.bin"), 0, 5).is_err());
        assert!(st.read(&make_key("b.zip", "p/other.bin"), 0, 5).is_err());
        assert!(st.read("t0.zip:3", 0, 5).is_err());
    }

    #[test]
    fn local_header_longer_than_central_directory() {
        let z = raw_zip(&[("a/x.bin", 0, 0, 40, b"hello stored"), ("a/y.bin", 0, 0, 3, b"second")]);
        let set = set_of(vec![z]);
        let st = Streams::new(set.clone());
        assert_eq!(st.read(&set.key_of(0, 0), 6, 100).unwrap(), b"stored");
        assert_eq!(st.read(&set.key_of(0, 1), 0, 100).unwrap(), b"second");
        let fs = ZipFs::new(set);
        assert_eq!(fs.read("a/x.bin").unwrap(), b"hello stored");
        assert_eq!(fs.read("a/y.bin").unwrap(), b"second");
    }

    #[test]
    fn read_whole_asks_the_host_once_for_a_small_entry() {
        let z = make(&[("m.json", b"{\"a\":1}", CompressionMethod::Deflated)]);
        let set = set_of(vec![z.clone()]);
        let calls = Rc::new(Cell::new(0));
        let c = calls.clone();
        let counted = ZipSet::new(
            set.names.clone(),
            set.zips.clone(),
            Box::new(move |_, o, n| {
                c.set(c.get() + 1);
                let s = (o as usize).min(z.len());
                z[s..(s + n as usize).min(z.len())].to_vec()
            }),
        );
        assert_eq!(counted.read_whole(0, 0, 1 << 20).unwrap(), b"{\"a\":1}");
        assert_eq!(calls.get(), 1);
    }

    #[test]
    fn skipped_entries_are_left_out() {
        let z = raw_zip(&[("a/enc.jpg", 1, 0, 0, b"xx"), ("a/odd.jpg", 0, 99, 0, b"yy"), ("a/ok.jpg", 0, 0, 0, b"zz")]);
        let set = set_of(vec![z]);
        let fs = ZipFs::new(set.clone());
        let names: Vec<String> = fs.list("a").unwrap().into_iter().map(|e| e.name).collect();
        assert_eq!(names, ["ok.jpg"]);
        let skipped = fs.skipped();
        assert_eq!(skipped.len(), 2);
        assert!(skipped.iter().any(|(n, r)| n == "a/enc.jpg" && r == "encrypted"));
        assert!(skipped.iter().any(|(n, r)| n == "a/odd.jpg" && r.contains("99")));
        assert!(Streams::new(set.clone()).read(&set.key_of(0, 0), 0, 2).is_err());
    }

    #[test]
    fn duplicate_path_first_zip_wins() {
        let a = make(&[("x/p.png", b"AAA", CompressionMethod::Stored)]);
        let b = make(&[("x/p.png", b"BBBB", CompressionMethod::Stored)]);
        let fs = ZipFs::new(set_of(vec![a, b]));
        let l = fs.list("x").unwrap();
        assert_eq!(l.len(), 1);
        assert_eq!(l[0].size, 3);
        assert_eq!(fs.read("x/p.png").unwrap(), b"AAA");
        assert_eq!(fs.virtual_key("x/p.png"), Some(make_key("t0.zip", "x/p.png")));
    }

    #[test]
    fn overlay_read_prefers_the_real_file() {
        let d = Dir::new("overlay-read");
        d.put("x/both.json", b"real");
        d.put("x/real.json", b"only real");
        let z = make(&[("x/both.json", b"zip", CompressionMethod::Stored), ("x/zip.json", b"only zip", CompressionMethod::Deflated)]);
        let zfs = ZipFs::new(set_of(vec![z]));
        let o = OverlayFs::new(&d, &zfs);
        let names: Vec<String> = o.list("x").unwrap().into_iter().map(|e| e.name).collect();
        assert_eq!(names.len(), 3, "{names:?}");
        assert_eq!(o.read("x/both.json").unwrap(), b"real");
        assert_eq!(o.read("x/real.json").unwrap(), b"only real");
        assert_eq!(o.read("x/zip.json").unwrap(), b"only zip");
        assert_eq!(o.virtual_key("x/both.json"), None);
        assert!(o.virtual_key("x/zip.json").is_some());
    }

    #[test]
    fn many_interleaved_deflated_entries_cost_about_one_pass() {
        // 20 entries read round-robin in 64 KB steps, as a dozen ingest threads do: each must
        // keep its inflate state, or every step restarts it from the beginning.
        let mut seed = 1u32;
        let datas: Vec<Vec<u8>> = (0..20)
            .map(|_| {
                (0..300_000)
                    .map(|_| {
                        seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
                        (seed >> 24) as u8
                    })
                    .collect()
            })
            .collect();
        let names: Vec<String> = (0..20).map(|i| format!("v/{i}.bin")).collect();
        let entries: Vec<(&str, &[u8], CompressionMethod)> = names.iter().zip(&datas).map(|(n, d)| (n.as_str(), d.as_slice(), CompressionMethod::Deflated)).collect();
        let z = make(&entries);
        let set = set_of(vec![z.clone()]);
        let fetched = Rc::new(Cell::new(0u64));
        let f = fetched.clone();
        let counted = Rc::new(ZipSet::new(
            set.names.clone(),
            set.zips.clone(),
            Box::new(move |_, o, n| {
                let s = (o as usize).min(z.len());
                let out = z[s..(s + n as usize).min(z.len())].to_vec();
                f.set(f.get() + out.len() as u64);
                out
            }),
        ));
        let total: u64 = counted.zips[0].iter().map(|e| e.compressed).sum();
        let st = Streams::new(counted.clone());
        for step in 0..4u64 {
            for (i, d) in datas.iter().enumerate() {
                let key = counted.key_of(0, counted.zips[0].iter().position(|e| e.name == names[i]).unwrap());
                let got = st.read(&key, step * 65536, 65536).unwrap();
                assert_eq!(got, &d[(step * 65536) as usize..((step + 1) * 65536) as usize]);
            }
        }
        assert!(fetched.get() < total + total / 5, "fetched {} of {total} compressed bytes", fetched.get());
    }

    #[test]
    fn zip_entry_dates_become_modified() {
        let mut w = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let dated = SimpleFileOptions::default().compression_method(CompressionMethod::Stored).last_modified_time(zip::DateTime::from_date_and_time(2019, 5, 17, 12, 30, 10).unwrap());
        w.start_file("p/dated.jpg", dated).unwrap();
        w.write_all(b"x").unwrap();
        let z = w.finish().unwrap().into_inner();
        let fs = ZipFs::new(set_of(vec![z]));
        assert_eq!(fs.list("p").unwrap()[0].modified, Some(1_558_096_210));
    }
}
