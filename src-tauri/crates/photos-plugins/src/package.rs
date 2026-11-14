//! A `.lumenplugin` package (spec §4): unpacked with hard limits, every path checked before
//! anything is written, every file hashed while it is written. A Developer-mode folder gets the
//! same checks without being copied.

use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use crate::licences;
use crate::manifest::{self, Manifest};
use crate::{PluginError, PluginKind};

/// 50 MB uncompressed, counted on the bytes actually read.
pub const MAX_BYTES: u64 = 50 * 1024 * 1024;
pub const MAX_FILES: usize = 2_000;
/// The largest `icon.png`, in pixels per side.
pub const MAX_ICON: u32 = 256;
const MAX_SIGNATURE: u64 = 64 * 1024;

const ARCHIVE_EXTENSIONS: &[&str] = &[
    "zip",
    "lumenplugin",
    "7z",
    "rar",
    "tar",
    "gz",
    "tgz",
    "bz2",
    "xz",
    "zst",
    "jar",
];
const WINDOWS_RESERVED: &[&str] = &[
    "con", "prn", "aux", "nul", "com1", "com2", "com3", "com4", "com5", "com6", "com7", "com8",
    "com9", "lpt1", "lpt2", "lpt3", "lpt4", "lpt5", "lpt6", "lpt7", "lpt8", "lpt9",
];

/// A checked package on disk.
#[derive(Debug, Clone)]
pub struct Unpacked {
    pub root: PathBuf,
    pub manifest: Manifest,
    /// Relative path → blake3, `SIGNATURE` excluded.
    pub files: BTreeMap<String, [u8; 32]>,
    pub signature: Option<String>,
}

impl Unpacked {
    /// Reads a file of the package; only paths in [`Unpacked::files`] can be read.
    pub fn read(&self, rel: &str) -> Result<Vec<u8>, PluginError> {
        if !self.files.contains_key(rel) {
            return Err(PluginError::invalid(format!("{rel} isn't in the package")));
        }
        fs::read(self.root.join(rel))
            .map_err(|e| PluginError::invalid(format!("{rel} couldn't be read: {e}")))
    }

    pub fn has(&self, rel: &str) -> bool {
        self.files.contains_key(rel)
    }
}

/// What a kind checker found: `adds` are the *Adds* lines of the install sheet, `problems`
/// refuse the install, `notes` are shown but don't.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Report {
    pub adds: Vec<String>,
    pub problems: Vec<String>,
    pub notes: Vec<String>,
}

/// A lane's checks for the kinds it owns. A kind no checker claims fails closed in the store.
pub trait KindCheck: Send + Sync {
    /// The kinds this checker claims.
    fn kinds(&self) -> &'static [PluginKind];
    fn check(&self, pkg: &Unpacked, dev: bool, report: &mut Report);
}

fn invalid(msg: impl Into<String>) -> PluginError {
    PluginError::invalid(msg)
}

/// Collects paths while a package or folder is read, refusing as early as possible.
#[derive(Default)]
struct Collector {
    files: BTreeMap<String, [u8; 32]>,
    lower: HashMap<String, String>,
    dirs: HashMap<String, String>,
    signature: Option<String>,
    total: u64,
}

impl Collector {
    /// Checks a file path and claims it (case-insensitively unique, not under a file).
    fn claim(&mut self, rel: &str) -> Result<(), PluginError> {
        check_path(rel)?;
        let ext = rel.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase());
        if ext.is_some_and(|e| ARCHIVE_EXTENSIONS.contains(&e.as_str())) {
            return Err(invalid(format!(
                "{rel}: packages can't contain other archives"
            )));
        }
        let low = rel.to_lowercase();
        if let Some(prev) = self.lower.get(&low) {
            return Err(invalid(format!("{rel} and {prev} differ only in case")));
        }
        if self.dirs.contains_key(&low) {
            return Err(invalid(format!("{rel} is both a file and a folder")));
        }
        // Every parent folder must be spelled one way and must not be a file.
        let mut at = 0;
        while let Some(i) = rel[at..].find('/') {
            let dir = &rel[..at + i];
            let dlow = dir.to_lowercase();
            if self.lower.contains_key(&dlow) {
                return Err(invalid(format!("{dir} is both a file and a folder")));
            }
            match self.dirs.get(&dlow) {
                Some(seen) if seen != dir => {
                    return Err(invalid(format!("{dir} and {seen} differ only in case")))
                }
                Some(_) => {}
                None => {
                    self.dirs.insert(dlow, dir.to_string());
                }
            }
            at += i + 1;
        }
        self.lower.insert(low, rel.to_string());
        if rel != "SIGNATURE" && self.files.len() >= MAX_FILES {
            return Err(invalid(format!(
                "A package can hold at most {MAX_FILES} files"
            )));
        }
        Ok(())
    }

    /// Streams `src` into `dst` (when given), counting and hashing; stops at the limit.
    fn take(
        &mut self,
        rel: &str,
        src: &mut dyn Read,
        mut dst: Option<&mut dyn Write>,
    ) -> Result<(), PluginError> {
        let is_sig = rel == "SIGNATURE";
        let mut hasher = blake3::Hasher::new();
        let mut sig = Vec::new();
        let mut buf = vec![0u8; 64 * 1024];
        let mut own = 0u64;
        loop {
            let n = src
                .read(&mut buf)
                .map_err(|e| invalid(format!("{rel} couldn't be read: {e}")))?;
            if n == 0 {
                break;
            }
            self.total += n as u64;
            own += n as u64;
            if self.total > MAX_BYTES {
                return Err(invalid("A package can be at most 50 MB unpacked"));
            }
            if is_sig && own > MAX_SIGNATURE {
                return Err(invalid("SIGNATURE is too large"));
            }
            let chunk = &buf[..n];
            if let Some(d) = dst.as_deref_mut() {
                d.write_all(chunk)
                    .map_err(|e| invalid(format!("{rel} couldn't be written: {e}")))?;
            }
            if is_sig {
                sig.extend_from_slice(chunk);
            } else {
                hasher.update(chunk);
            }
        }
        if is_sig {
            let text = String::from_utf8(sig).map_err(|_| invalid("SIGNATURE isn't text"))?;
            self.signature = Some(text);
        } else {
            self.files
                .insert(rel.to_string(), *hasher.finalize().as_bytes());
        }
        Ok(())
    }
}

/// Relative, `/`-separated, no `..`, `.` or empty segments, no drive letters, backslashes,
/// control characters or Windows device names.
fn check_path(rel: &str) -> Result<(), PluginError> {
    let bad = |why: &str| Err(invalid(format!("{rel:?}: {why}")));
    if rel.is_empty() {
        return bad("an empty path");
    }
    if rel.starts_with('/') {
        return bad("paths must be relative");
    }
    if rel.contains('\\') || rel.contains(':') {
        return bad("paths can't contain '\\' or ':'");
    }
    if rel.chars().any(|c| c.is_control()) {
        return bad("paths can't contain control characters");
    }
    for seg in rel.split('/') {
        if seg.is_empty() || seg == "." || seg == ".." {
            return bad("paths can't contain '..', '.' or empty parts");
        }
        if seg.ends_with('.') || seg.ends_with(' ') {
            return bad("names can't end in '.' or a space");
        }
        let stem = seg.split('.').next().unwrap_or(seg).to_ascii_lowercase();
        if WINDOWS_RESERVED.contains(&stem.as_str()) {
            return bad("a reserved Windows name");
        }
    }
    Ok(())
}

/// Unpacks a `.lumenplugin` into `into` (created if missing, expected empty) and checks it.
/// On an error `into` may hold a partial unpack; the caller removes it.
pub fn open_package(file: &Path, into: &Path) -> Result<Unpacked, PluginError> {
    let f = fs::File::open(file)
        .map_err(|e| invalid(format!("The package couldn't be opened: {e}")))?;
    let mut zip = zip::ZipArchive::new(f)
        .map_err(|e| invalid(format!("This isn't a Lumen plugin package: {e}")))?;
    fs::create_dir_all(into).map_err(|e| invalid(format!("Couldn't unpack: {e}")))?;

    // Cheap refusals from the headers first; the real limits count bytes actually read.
    let mut declared = 0u64;
    let mut count = 0usize;
    for i in 0..zip.len() {
        let e = zip
            .by_index_raw(i)
            .map_err(|e| invalid(format!("A damaged package: {e}")))?;
        if !e.is_dir() {
            count += 1;
            declared = declared.saturating_add(e.size());
        }
    }
    if count > MAX_FILES + 1 {
        return Err(invalid(format!(
            "A package can hold at most {MAX_FILES} files"
        )));
    }
    if declared > MAX_BYTES {
        return Err(invalid("A package can be at most 50 MB unpacked"));
    }

    let mut c = Collector::default();
    for i in 0..zip.len() {
        let mut entry = zip
            .by_index(i)
            .map_err(|e| invalid(format!("A damaged package: {e}")))?;
        let name = entry
            .name()
            .map_err(|e| invalid(format!("A file name isn't valid: {e}")))?
            .to_string();
        if entry.unix_mode().is_some_and(|m| m & 0o170000 == 0o120000) {
            return Err(invalid(format!("{name}: packages can't contain links")));
        }
        if entry.is_dir() {
            check_path(name.trim_end_matches('/'))?;
            continue;
        }
        c.claim(&name)?;
        let dest = into.join(&name);
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| invalid(format!("Couldn't unpack {name}: {e}")))?;
        }
        let mut out = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&dest)
            .map_err(|e| invalid(format!("Couldn't unpack {name}: {e}")))?;
        c.take(&name, &mut entry, Some(&mut out))?;
    }
    finish(into.to_path_buf(), c)
}

/// Checks a Developer-mode folder in place. Links are refused, never followed.
pub fn open_folder(dir: &Path) -> Result<Unpacked, PluginError> {
    let mut c = Collector::default();
    walk(dir, "", &mut c)?;
    finish(dir.to_path_buf(), c)
}

fn walk(dir: &Path, prefix: &str, c: &mut Collector) -> Result<(), PluginError> {
    let rd = fs::read_dir(dir).map_err(|e| invalid(format!("Couldn't read the folder: {e}")))?;
    let mut entries: Vec<_> = rd
        .collect::<Result<_, _>>()
        .map_err(|e| invalid(format!("Couldn't read the folder: {e}")))?;
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
        let name = e
            .file_name()
            .to_str()
            .map(str::to_string)
            .ok_or_else(|| invalid("A file name isn't valid text"))?;
        let rel = format!("{prefix}{name}");
        let meta = fs::symlink_metadata(e.path()).map_err(|e| invalid(format!("{rel}: {e}")))?;
        if meta.file_type().is_symlink() {
            return Err(invalid(format!("{rel}: packages can't contain links")));
        }
        if meta.is_dir() {
            check_path(&rel)?;
            walk(&e.path(), &format!("{rel}/"), c)?;
        } else if meta.is_file() {
            c.claim(&rel)?;
            let mut f = fs::File::open(e.path()).map_err(|e| invalid(format!("{rel}: {e}")))?;
            c.take(&rel, &mut f, None)?;
        } else {
            return Err(invalid(format!("{rel}: not a regular file")));
        }
    }
    Ok(())
}

/// The checks that need the whole file list: the manifest, its references, code, icon, licences.
fn finish(root: PathBuf, c: Collector) -> Result<Unpacked, PluginError> {
    let Collector {
        files, signature, ..
    } = c;
    if !files.contains_key("plugin.toml") {
        return Err(invalid("The package has no plugin.toml"));
    }
    let text =
        fs::read(root.join("plugin.toml")).map_err(|e| invalid(format!("plugin.toml: {e}")))?;
    let text = String::from_utf8(text).map_err(|_| invalid("plugin.toml isn't UTF-8 text"))?;
    let manifest = manifest::parse(&text)?;
    let pkg = Unpacked {
        root,
        manifest,
        files,
        signature,
    };
    let m = &pkg.manifest;

    let need = |path: &str, what: &str| -> Result<(), PluginError> {
        if pkg.has(path) {
            Ok(())
        } else {
            Err(invalid(format!(
                "plugin.toml names {what} {path}, which isn't in the package"
            )))
        }
    };
    for t in &m.themes {
        need(&t.file, "the theme")?;
    }
    if let Some(a) = &m.apps {
        need(&a.file, "the apps file")?;
    }
    for p in &m.prompts {
        need(&p.file, "the prompt")?;
    }
    for a in &m.assets {
        need(&a.path, "the asset")?;
    }
    if let Some(d) = &m.presets {
        let prefix = format!("{}/", d.dir.trim_end_matches('/'));
        if !pkg.files.keys().any(|k| k.starts_with(&prefix)) {
            return Err(invalid(format!(
                "plugin.toml names the presets folder {}, which has no files",
                d.dir
            )));
        }
    }

    let wasm = pkg.has("plugin.wasm");
    if m.has_code() && !wasm {
        return Err(invalid("An importer or tools need plugin.wasm"));
    }
    if wasm && !m.has_code() {
        return Err(invalid("plugin.wasm is only for an importer or tools"));
    }

    if pkg.has("icon.png") {
        check_icon(&pkg.read("icon.png")?)?;
    }

    let has_licence = pkg.has("LICENSE");
    licences::check(&m.license, has_licence).map_err(PluginError::licence)?;
    for a in &m.assets {
        licences::check(&a.license, has_licence)
            .map_err(|e| PluginError::licence(format!("{}: {e}", a.path)))?;
    }
    Ok(pkg)
}

/// `icon.png` must be a PNG of at most 256 × 256, read from its IHDR (no decoder).
fn check_icon(bytes: &[u8]) -> Result<(), PluginError> {
    const SIG: &[u8] = b"\x89PNG\r\n\x1a\n";
    if bytes.len() < 24 || &bytes[..8] != SIG || &bytes[12..16] != b"IHDR" {
        return Err(invalid("icon.png isn't a PNG"));
    }
    let w = u32::from_be_bytes(bytes[16..20].try_into().unwrap());
    let h = u32::from_be_bytes(bytes[20..24].try_into().unwrap());
    if w == 0 || h == 0 || w > MAX_ICON || h > MAX_ICON {
        return Err(invalid(format!(
            "icon.png is {w} × {h}; at most {MAX_ICON} × {MAX_ICON}"
        )));
    }
    Ok(())
}

/// Builds packages for tests (this crate's and the store's).
#[cfg(test)]
pub(crate) mod fixture {
    use std::io::Write;
    use std::path::PathBuf;

    use zip::write::SimpleFileOptions;

    pub const BASE: &str = r#"id = "com.test.x"
name = "X"
version = "1.0.0"
author = "Tester"
description = "A test plugin."
license = "MIT"
lumen = ">=26.0.0"
api = 1
"#;

    pub fn manifest(extra: &str) -> String {
        format!("{BASE}{extra}\n")
    }

    /// A zip of `entries` in a temp dir; keep the `TempDir` alive while using the path.
    pub fn pack(entries: &[(&str, &[u8])]) -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("p.lumenplugin");
        pack_to(&path, entries);
        (dir, path)
    }

    pub fn pack_to(path: &std::path::Path, entries: &[(&str, &[u8])]) {
        let mut z = zip::ZipWriter::new(std::fs::File::create(path).unwrap());
        let o = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        for (name, data) in entries {
            z.start_file(*name, o).unwrap();
            z.write_all(data).unwrap();
        }
        z.finish().unwrap();
    }

    /// The first 24 bytes of a PNG of `w` × `h` (enough for the IHDR check).
    pub fn png(w: u32, h: u32) -> Vec<u8> {
        let mut v = b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR".to_vec();
        v.extend_from_slice(&w.to_be_bytes());
        v.extend_from_slice(&h.to_be_bytes());
        v.extend_from_slice(&[8, 6, 0, 0, 0, 0, 0, 0, 0]);
        v
    }
}

#[cfg(test)]
mod tests {
    use super::fixture::*;
    use super::*;
    use zip::write::SimpleFileOptions;

    fn open(entries: &[(&str, &[u8])]) -> Result<Unpacked, PluginError> {
        let (dir, path) = pack(entries);
        let r = open_package(&path, &dir.path().join("out"));
        drop(dir);
        r
    }

    fn theme_manifest() -> String {
        manifest("[[themes]]\nfile = \"themes/a.toml\"")
    }

    fn assert_invalid(r: Result<Unpacked, PluginError>, code: &str) {
        match r {
            Ok(_) => panic!("expected {code}, got Ok"),
            Err(e) => assert_eq!(e.code, code, "{e}"),
        }
    }

    #[test]
    fn opens_minimal_data_package() {
        let m = theme_manifest();
        let p = open(&[("plugin.toml", m.as_bytes()), ("themes/a.toml", b"x = 1")]).unwrap();
        assert_eq!(p.files.len(), 2);
        assert_eq!(p.files["themes/a.toml"], *blake3::hash(b"x = 1").as_bytes());
        assert_eq!(
            p.files["plugin.toml"],
            *blake3::hash(m.as_bytes()).as_bytes()
        );
        assert_eq!(p.signature, None);
        assert_eq!(p.manifest.id, "com.test.x");
    }

    #[test]
    fn signature_file_read_not_hashed() {
        let m = theme_manifest();
        let p = open(&[
            ("plugin.toml", m.as_bytes()),
            ("themes/a.toml", b"x"),
            ("SIGNATURE", b"sig text"),
        ])
        .unwrap();
        assert_eq!(p.signature.as_deref(), Some("sig text"));
        assert!(!p.files.contains_key("SIGNATURE"));
    }

    #[test]
    fn limits() {
        let m = theme_manifest();
        let names: Vec<String> = (0..2_000).map(|i| format!("assets/{i}.txt")).collect();
        let mut entries: Vec<(&str, &[u8])> =
            vec![("plugin.toml", m.as_bytes()), ("themes/a.toml", b"x")];
        entries.extend(names.iter().map(|n| (n.as_str(), &b"a"[..])));
        assert_invalid(open(&entries), "plugin_invalid");

        let big = vec![0u8; 26 * 1024 * 1024];
        assert_invalid(
            open(&[
                ("plugin.toml", m.as_bytes()),
                ("themes/a.toml", &big),
                ("assets/b.bin", &big),
            ]),
            "plugin_invalid",
        );
    }

    /// Patches the uncompressed size of the entry called `name` in the zip's headers to `size`.
    fn lie_about_size(bytes: &mut [u8], name: &[u8], size: u32) {
        let at = |b: &[u8], i: usize| u16::from_le_bytes([b[i], b[i + 1]]) as usize;
        let mut i = 0;
        while i + 46 <= bytes.len() {
            match &bytes[i..i + 4] {
                b"PK\x03\x04" if &bytes[i + 30..i + 30 + at(bytes, i + 26)] == name => {
                    bytes[i + 22..i + 26].copy_from_slice(&size.to_le_bytes())
                }
                b"PK\x01\x02" if &bytes[i + 46..i + 46 + at(bytes, i + 28)] == name => {
                    bytes[i + 24..i + 28].copy_from_slice(&size.to_le_bytes())
                }
                _ => {}
            }
            i += 1;
        }
    }

    /// The byte counter itself (zip also refuses an entry larger than its header says, so a
    /// package can't reach this; it is the second fence).
    #[test]
    fn counter_stops_at_limit() {
        struct Sink(u64);
        impl Write for Sink {
            fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
                self.0 += b.len() as u64;
                Ok(b.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let mut c = Collector::default();
        let mut sink = Sink(0);
        let mut endless = std::io::repeat(0);
        let err = c
            .take("assets/z.bin", &mut endless, Some(&mut sink))
            .unwrap_err();
        assert!(err.message.contains("50 MB"), "{err}");
        assert!(sink.0 <= MAX_BYTES, "wrote {} bytes", sink.0);
    }

    #[test]
    fn zip_bomb_stops_at_limit() {
        let m = theme_manifest();
        let zeros = vec![0u8; 60 * 1024 * 1024];
        let (dir, path) = pack(&[
            ("plugin.toml", m.as_bytes()),
            ("themes/a.toml", b"x"),
            ("assets/z.bin", &zeros),
        ]);
        drop(zeros);
        let mut bytes = std::fs::read(&path).unwrap();
        lie_about_size(&mut bytes, b"assets/z.bin", 100);
        std::fs::write(&path, bytes).unwrap();
        let out = dir.path().join("out");
        let err = open_package(&path, &out).unwrap_err();
        assert_eq!(err.code, "plugin_invalid");
        assert!(
            err.message.contains("assets/z.bin"),
            "refused for another reason: {err}"
        );
        let held: u64 = walkdir_size(&out);
        assert!(held < 51 * 1024 * 1024, "unpacked {held} bytes");
    }

    fn walkdir_size(p: &Path) -> u64 {
        let mut n = 0;
        if let Ok(rd) = std::fs::read_dir(p) {
            for e in rd.flatten() {
                let m = e.metadata().unwrap();
                n += if m.is_dir() {
                    walkdir_size(&e.path())
                } else {
                    m.len()
                };
            }
        }
        n
    }

    #[test]
    fn paths_refused() {
        let m = theme_manifest();
        for bad in ["../x", "/x", "a/../../x", "./x", "a//b"] {
            assert_invalid(
                open(&[
                    ("plugin.toml", m.as_bytes()),
                    ("themes/a.toml", b"x"),
                    (bad, b"y"),
                ]),
                "plugin_invalid",
            );
        }
    }

    #[test]
    fn windows_paths_refused() {
        let m = theme_manifest();
        for bad in ["a\\b", "C:x", "\\\\srv\\x", "assets/con.txt", "assets/x."] {
            assert_invalid(
                open(&[
                    ("plugin.toml", m.as_bytes()),
                    ("themes/a.toml", b"x"),
                    (bad, b"y"),
                ]),
                "plugin_invalid",
            );
        }
    }

    #[test]
    fn case_collision_refused() {
        let m = theme_manifest();
        assert_invalid(
            open(&[
                ("plugin.toml", m.as_bytes()),
                ("themes/a.toml", b"x"),
                ("Icon.png", b"1"),
                ("icon.png", b"2"),
            ]),
            "plugin_invalid",
        );
        assert_invalid(
            open(&[
                ("plugin.toml", m.as_bytes()),
                ("themes/a.toml", b"x"),
                ("Assets/a", b"1"),
                ("assets/b", b"2"),
            ]),
            "plugin_invalid",
        );
    }

    #[test]
    fn nested_archive_refused() {
        let m = theme_manifest();
        for bad in ["assets/x.zip", "assets/y.lumenplugin"] {
            assert_invalid(
                open(&[
                    ("plugin.toml", m.as_bytes()),
                    ("themes/a.toml", b"x"),
                    (bad, b"PK"),
                ]),
                "plugin_invalid",
            );
        }
    }

    #[test]
    fn symlink_entry_refused() {
        let m = theme_manifest();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("p.lumenplugin");
        let mut z = zip::ZipWriter::new(std::fs::File::create(&path).unwrap());
        let o = SimpleFileOptions::default();
        z.start_file("plugin.toml", o).unwrap();
        z.write_all(m.as_bytes()).unwrap();
        z.start_file("themes/a.toml", o).unwrap();
        z.write_all(b"x").unwrap();
        z.add_symlink("assets/link", "/etc/passwd", o.unix_permissions(0o777))
            .unwrap();
        z.finish().unwrap();
        assert_invalid(
            open_package(&path, &dir.path().join("out")),
            "plugin_invalid",
        );
    }

    #[test]
    fn code_kind_needs_wasm() {
        let m = manifest("[importer]\nlabel = \"L\"\npick = \"folder\"");
        assert_invalid(open(&[("plugin.toml", m.as_bytes())]), "plugin_invalid");
        let m = theme_manifest();
        assert_invalid(
            open(&[
                ("plugin.toml", m.as_bytes()),
                ("themes/a.toml", b"x"),
                ("plugin.wasm", b"\0asm"),
            ]),
            "plugin_invalid",
        );
        let m = manifest("[importer]\nlabel = \"L\"\npick = \"folder\"");
        open(&[("plugin.toml", m.as_bytes()), ("plugin.wasm", b"\0asm")]).unwrap();
    }

    #[test]
    fn referenced_files_exist() {
        let m = manifest("[[themes]]\nfile = \"themes/missing.toml\"");
        assert_invalid(
            open(&[("plugin.toml", m.as_bytes()), ("themes/a.toml", b"x")]),
            "plugin_invalid",
        );
        let m = manifest("[[themes]]\nfile = \"themes/a.toml\"\n[[assets]]\npath = \"assets/a.png\"\nlicense = \"MIT\"");
        assert_invalid(
            open(&[("plugin.toml", m.as_bytes()), ("themes/a.toml", b"x")]),
            "plugin_invalid",
        );
        let m = manifest("[presets]\ndir = \"presets\"");
        assert_invalid(open(&[("plugin.toml", m.as_bytes())]), "plugin_invalid");
        open(&[("plugin.toml", m.as_bytes()), ("presets/a.json", b"{}")]).unwrap();
    }

    #[test]
    fn icon_size_checked() {
        let m = theme_manifest();
        let big = png(300, 300);
        assert_invalid(
            open(&[
                ("plugin.toml", m.as_bytes()),
                ("themes/a.toml", b"x"),
                ("icon.png", &big),
            ]),
            "plugin_invalid",
        );
        assert_invalid(
            open(&[
                ("plugin.toml", m.as_bytes()),
                ("themes/a.toml", b"x"),
                ("icon.png", b"GIF89a"),
            ]),
            "plugin_invalid",
        );
        let ok = png(256, 128);
        open(&[
            ("plugin.toml", m.as_bytes()),
            ("themes/a.toml", b"x"),
            ("icon.png", &ok),
        ])
        .unwrap();
    }

    #[test]
    fn licences_checked() {
        // Copyleft code and NC assets install: Lumen only wants a declared, well-formed licence.
        let m =
            manifest("license = \"GPL-3.0-only\"
[importer]
label = \"L\"
pick = \"folder\"")
                .replacen("license = \"MIT\"
", "", 1);
        open(&[("plugin.toml", m.as_bytes()), ("plugin.wasm", b" asm")]).unwrap();
        let m = manifest("[[themes]]
file = \"themes/a.toml\"
[[assets]]
path = \"assets/a.png\"
license = \"CC-BY-NC-4.0\"");
        open(&[
            ("plugin.toml", m.as_bytes()),
            ("themes/a.toml", b"x"),
            ("assets/a.png", b"p"),
        ])
        .unwrap();
        // A LicenseRef needs its text in the package.
        let m = manifest("[[themes]]
file = \"themes/a.toml\"
[[assets]]
path = \"assets/a.png\"
license = \"LicenseRef-X\"");
        assert_invalid(
            open(&[
                ("plugin.toml", m.as_bytes()),
                ("themes/a.toml", b"x"),
                ("assets/a.png", b"p"),
            ]),
            "plugin_licence",
        );
        let m = theme_manifest().replace("license = \"MIT\"", "license = \"CC-BY-4.0\"");
        open(&[("plugin.toml", m.as_bytes()), ("themes/a.toml", b"x")]).unwrap();
    }

    #[test]
    fn open_folder_same_checks() {
        let dir = tempfile::tempdir().unwrap();
        let m = manifest("[[themes]]\nfile = \"themes/missing.toml\"");
        std::fs::write(dir.path().join("plugin.toml"), &m).unwrap();
        std::fs::create_dir(dir.path().join("themes")).unwrap();
        std::fs::write(dir.path().join("themes/a.toml"), "x").unwrap();
        let folder_err = open_folder(dir.path()).unwrap_err();
        let pkg_err = open(&[("plugin.toml", m.as_bytes()), ("themes/a.toml", b"x")]).unwrap_err();
        assert_eq!(folder_err, pkg_err);

        std::fs::write(dir.path().join("plugin.toml"), theme_manifest()).unwrap();
        let p = open_folder(dir.path()).unwrap();
        assert_eq!(p.root, dir.path());
        assert_eq!(p.files.len(), 2);
        assert_eq!(p.read("themes/a.toml").unwrap(), b"x");
        assert!(p.read("../x").is_err());
    }
}
