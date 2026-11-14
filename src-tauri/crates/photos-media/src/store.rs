use std::path::{Path, PathBuf};

/// Which reader left an unreadable mark. Round 17 taught RAW previews to fall back to a file's
/// own embedded JPEG and its sensor data, so the RAWs older readers gave up on are tried again.
const READER: &str = "reader2";

/// Content-addressed derivative paths under `<library>/derivatives`.
/// Fan-out by the first two hex characters keeps directories small on Windows.
#[derive(Debug, Clone)]
pub struct DerivativeStore {
    root: PathBuf,
}

impl DerivativeStore {
    pub fn new(library_root: &Path) -> Self {
        Self { root: library_root.join("derivatives") }
    }

    /// `<library>/derivatives`, for work that is not content-addressed (a slideshow's
    /// stills, which live only as long as the export).
    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn thumb_path(&self, hash: &str, size: u32) -> PathBuf {
        self.root.join("thumbs").join(&hash[..2]).join(format!("{hash}_{size}.jpg"))
    }

    pub fn preview_path(&self, hash: &str) -> PathBuf {
        self.root.join("previews").join(&hash[..2]).join(format!("{hash}_2560.jpg"))
    }

    /// Full-resolution render of a RAW/HEIC original.
    pub fn full_path(&self, hash: &str) -> PathBuf {
        self.root.join("full").join(&hash[..2]).join(format!("{hash}_full.jpg"))
    }

    /// Why a photo could not be rendered, written by the version of Lumen that tried (Round 8A):
    /// later requests answer at once instead of decoding, and warning, again. A new version
    /// tries once more.
    pub fn failure_path(&self, hash: &str) -> PathBuf {
        self.root.join("failed").join(&hash[..2]).join(format!("{hash}.txt"))
    }

    /// The reason recorded by `version` for `hash`, if that version recorded one.
    pub fn failure(&self, hash: &str, version: &str) -> Option<String> {
        let text = std::fs::read_to_string(self.failure_path(hash)).ok()?;
        let (by, why) = text.split_once('\n')?;
        (by == version).then(|| why.to_string())
    }

    /// The last recorded reason for `hash`, whichever version wrote it (Round 22: Repair sorts
    /// the flagged files by it).
    pub fn failure_text(&self, hash: &str) -> Option<String> {
        if hash.len() < 2 {
            return None;
        }
        let text = std::fs::read_to_string(self.failure_path(hash)).ok()?;
        text.split_once('\n').map(|(_, why)| why.to_string())
    }

    /// Records that `version` could not render `hash`, and why.
    pub fn record_failure(&self, hash: &str, version: &str, why: &str) {
        let path = self.failure_path(hash);
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(path, format!("{version}\n{why}"));
    }

    /// H.264 playback proxy of a video the WebView cannot play natively.
    pub fn proxy_path(&self, hash: &str) -> PathBuf {
        self.root.join("video").join(&hash[..2]).join(format!("{hash}_proxy.mp4"))
    }

    /// Marker beside the proxy for a video FFmpeg could not convert (Round 9A), so the
    /// background prep does not retry it on every launch. Cleared with the proxies.
    fn proxy_failed_path(&self, hash: &str) -> PathBuf {
        self.root.join("video").join(&hash[..2]).join(format!("{hash}_proxy.failed"))
    }

    pub fn mark_proxy_failed(&self, hash: &str) {
        let p = self.proxy_failed_path(hash);
        if let Some(dir) = p.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(p, b"");
    }

    pub fn proxy_failed(&self, hash: &str) -> bool {
        self.proxy_failed_path(hash).is_file()
    }

    /// Bytes used by playback proxies (Settings shows this next to *Clear*).
    pub fn proxy_bytes(&self) -> u64 {
        let mut total = 0;
        let Ok(dirs) = std::fs::read_dir(self.root.join("video")) else { return 0 };
        for d in dirs.flatten() {
            if let Ok(files) = std::fs::read_dir(d.path()) {
                total += files.flatten().filter_map(|f| f.metadata().ok()).map(|m| m.len()).sum::<u64>();
            }
        }
        total
    }

    /// Delete every playback proxy. Returns bytes freed.
    pub fn clear_proxies(&self) -> u64 {
        let freed = self.proxy_bytes();
        let _ = std::fs::remove_dir_all(self.root.join("video"));
        freed
    }

    /// An edited size, per recipe version: a new edit never overwrites a file the webview
    /// may hold as `immutable`, and a revert leaves the original's derivatives untouched.
    pub fn edit_path(&self, hash: &str, size: u32, version: i64) -> PathBuf {
        self.root.join("edits").join(&hash[..2]).join(format!("{hash}_v{version}_{size}.jpg"))
    }

    pub fn edit_full_path(&self, hash: &str, version: i64) -> PathBuf {
        self.root.join("edits").join(&hash[..2]).join(format!("{hash}_v{version}_full.jpg"))
    }

    /// The denoised base (Phase 8d): full size, oriented, geometry-free, 16-bit sRGB PNG, one
    /// per denoise model. `model` is a manifest id; anything but letters, digits, `-` and `.`
    /// becomes `-`.
    pub fn denoise_path(&self, hash: &str, model: &str) -> PathBuf {
        let model: String = model.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '.' { c } else { '-' }).collect();
        self.root.join("denoise").join(&hash[..2]).join(format!("{hash}_{model}.png"))
    }

    /// Every denoised base of `hash`.
    pub fn delete_denoised(&self, hash: &str) {
        let Ok(dir) = std::fs::read_dir(self.root.join("denoise").join(&hash[..2])) else { return };
        let prefix = format!("{hash}_");
        for f in dir.flatten() {
            if f.file_name().to_string_lossy().starts_with(&prefix) {
                let _ = std::fs::remove_file(f.path());
            }
        }
    }

    /// Every edited size of every version of `hash`.
    pub fn delete_edits(&self, hash: &str) {
        let Ok(dir) = std::fs::read_dir(self.root.join("edits").join(&hash[..2])) else { return };
        let prefix = format!("{hash}_v");
        for f in dir.flatten() {
            if f.file_name().to_string_lossy().starts_with(&prefix) {
                let _ = std::fs::remove_file(f.path());
            }
        }
    }

    /// Marker for an original whose bytes did not decode (truncated, mislabelled, not media
    /// at all), so start-up does not retry it on every launch.
    pub fn unreadable_path(&self, hash: &str) -> PathBuf {
        self.root.join("unreadable").join(&hash[..2]).join(hash)
    }

    /// Records that `hash` did not decode, and whether FFmpeg was there to try.
    pub fn mark_unreadable(&self, hash: &str, with_ffmpeg: bool) {
        let p = self.unreadable_path(hash);
        if let Some(dir) = p.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let tried = if with_ffmpeg { format!("{READER} ffmpeg") } else { READER.to_string() };
        let _ = std::fs::write(p, tried);
    }

    /// Whether decoding `hash` again is pointless: it failed before with this reader, and FFmpeg
    /// has not appeared since (the only other thing that can change the outcome for the same
    /// bytes). A mark an older reader left is tried once more.
    pub fn is_unreadable(&self, hash: &str, ffmpeg_available: bool) -> bool {
        match std::fs::read_to_string(self.unreadable_path(hash)) {
            Ok(tried) => {
                let tried: Vec<&str> = tried.split_whitespace().collect();
                tried.contains(&READER) && (tried.contains(&"ffmpeg") || !ffmpeg_available)
            }
            Err(_) => false,
        }
    }

    /// Whether the mark says this reader had FFmpeg to try and still could not decode `hash`:
    /// the evidence for *Files Lumen can't open*. A mark without FFmpeg only means "no reader
    /// here yet", which must never hide a photo.
    pub fn failed_with_ffmpeg(&self, hash: &str) -> bool {
        self.is_unreadable(hash, true)
    }

    /// *Try again*: forgets the unreadable mark and the failure marker of `hash` and nothing
    /// else, so the next ingest decodes the original afresh. Missing files are fine.
    pub fn forget_unreadable(&self, hash: &str) {
        let _ = std::fs::remove_file(self.unreadable_path(hash));
        let _ = std::fs::remove_file(self.failure_path(hash));
    }

    /// Remove every derivative of `hash`, edits included. Missing files are fine.
    pub fn delete_all(&self, hash: &str) {
        self.delete_edits(hash);
        self.delete_denoised(hash);
        for p in [
            self.unreadable_path(hash),
            self.thumb_path(hash, 256),
            self.thumb_path(hash, 1024),
            self.preview_path(hash),
            self.full_path(hash),
            self.proxy_path(hash),
            self.proxy_failed_path(hash),
        ] {
            let _ = std::fs::remove_file(p);
        }
    }

    pub fn path_for(&self, hash: &str, size: u32) -> PathBuf {
        if size >= 2560 {
            self.preview_path(hash)
        } else {
            self.thumb_path(hash, size)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_mark_left_by_an_older_reader_is_tried_once_more() {
        let dir = tempfile::tempdir().unwrap();
        let s = DerivativeStore::new(dir.path());
        // What Lumen before Round 17 wrote: a RAW with no preview rawler could give.
        for (hash, old) in [("abcdef11", ""), ("abcdef12", "ffmpeg")] {
            let p = s.unreadable_path(hash);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(&p, old).unwrap();
            assert!(!s.is_unreadable(hash, false) && !s.is_unreadable(hash, true), "{old:?}");
        }
        s.mark_unreadable("abcdef11", true);
        assert!(s.is_unreadable("abcdef11", false), "this reader's own mark counts");
    }

    #[test]
    fn a_failure_is_remembered_by_the_version_that_saw_it() {
        let dir = tempfile::tempdir().unwrap();
        let s = DerivativeStore::new(dir.path());
        assert_eq!(s.failure("abcdef", "2026.1.0"), None);
        s.record_failure("abcdef", "2026.1.0", "moov atom not found");
        assert_eq!(s.failure("abcdef", "2026.1.0").as_deref(), Some("moov atom not found"));
        assert_eq!(s.failure("abcdef", "2026.2.0"), None, "a new version tries again");
    }

    #[test]
    fn a_proxy_that_failed_is_remembered_until_the_cache_is_cleared() {
        // Round 9A: the background prep does not retry a video FFmpeg could not convert on
        // every launch; clearing the proxies (Settings) or deleting the video forgets it.
        let dir = tempfile::tempdir().unwrap();
        let s = DerivativeStore::new(dir.path());
        assert!(!s.proxy_failed("abcdef"));
        s.mark_proxy_failed("abcdef");
        assert!(s.proxy_failed("abcdef"));
        assert!(!s.proxy_failed("abcdee"), "only that video");
        s.clear_proxies();
        assert!(!s.proxy_failed("abcdef"));
        s.mark_proxy_failed("abcdef");
        s.delete_all("abcdef");
        assert!(!s.proxy_failed("abcdef"));
    }

    #[test]
    fn edit_paths_are_per_version_and_deleted_together() {
        let dir = tempfile::tempdir().unwrap();
        let s = DerivativeStore::new(dir.path());
        let h = "abcdef0123";
        assert_ne!(s.edit_path(h, 256, 1), s.edit_path(h, 256, 2));
        assert_ne!(s.edit_path(h, 256, 1), s.thumb_path(h, 256), "the original's thumbnail survives an edit");
        for p in [s.edit_path(h, 256, 1), s.edit_path(h, 2560, 2), s.edit_full_path(h, 2), s.thumb_path(h, 256), s.edit_path("abcdef9999", 256, 1)] {
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(&p, b"x").unwrap();
        }
        s.delete_edits(h);
        assert!(!s.edit_path(h, 256, 1).exists() && !s.edit_full_path(h, 2).exists());
        assert!(s.thumb_path(h, 256).exists(), "revert must not cost a re-decode");
        assert!(s.edit_path("abcdef9999", 256, 1).exists(), "another photo's edits are left alone");
        s.delete_all("abcdef9999");
        assert!(!s.edit_path("abcdef9999", 256, 1).exists(), "deleting a photo deletes its edits");
    }

    #[test]
    fn only_a_mark_that_says_ffmpeg_tried_counts_as_failed_and_try_again_forgets_just_the_marks() {
        let dir = tempfile::tempdir().unwrap();
        let s = DerivativeStore::new(dir.path());
        s.mark_unreadable("abcdef10", false);
        assert!(!s.failed_with_ffmpeg("abcdef10"), "no FFmpeg was there: not evidence");
        s.mark_unreadable("abcdef10", true);
        assert!(s.failed_with_ffmpeg("abcdef10"));
        s.record_failure("abcdef10", "2026.1.0", "moov atom not found");
        let thumb = s.thumb_path("abcdef10", 256);
        std::fs::create_dir_all(thumb.parent().unwrap()).unwrap();
        std::fs::write(&thumb, b"x").unwrap();
        s.forget_unreadable("abcdef10");
        assert!(!s.failed_with_ffmpeg("abcdef10") && s.failure("abcdef10", "2026.1.0").is_none());
        assert!(thumb.is_file(), "nothing else is touched");
        s.forget_unreadable("abcdef10");
    }

    #[test]
    fn an_unreadable_original_is_retried_only_once_ffmpeg_appears() {
        let dir = tempfile::tempdir().unwrap();
        let s = DerivativeStore::new(dir.path());
        assert!(!s.is_unreadable("abcdef01", false) && !s.is_unreadable("abcdef01", true));
        s.mark_unreadable("abcdef01", false);
        assert!(s.is_unreadable("abcdef01", false));
        assert!(!s.is_unreadable("abcdef01", true), "FFmpeg may decode what the image crate could not");
        s.mark_unreadable("abcdef02", true);
        assert!(s.is_unreadable("abcdef02", false) && s.is_unreadable("abcdef02", true));
        s.delete_all("abcdef02");
        assert!(!s.is_unreadable("abcdef02", true));
    }
}
