//! Bringing a photo's sidecars in with it (Round 19 C3, spec 13 item 3): `.xmp`, `.aae` and
//! `.thm` files are copied beside the original and recorded, and the marks an XMP packet holds
//! (stars, colour label, reject, keywords, title, caption) reach the catalogue through
//! `extras::apply`. Lightroom's `crs:` develop settings are not read.

use std::io::Read;
use std::path::Path;

use photos_core::extras::ItemExtras;
use photos_core::sidecars::{self, NewSidecar};
use photos_core::{now_unix, Library};
use photos_media::xmp::{self, Marks};

use crate::source::{ImportSource, SourceItem};
use crate::ImportError;

/// A sidecar bigger than this is not a sidecar (an `.xmp` holds kilobytes).
const MAX_SIDECAR_BYTES: u64 = 16 << 20;

/// `first`'s marks, with `then` filling what `first` leaves out.
fn merge(first: Marks, then: Marks) -> Marks {
    Marks {
        rating: first.rating.or(then.rating),
        flag: first.flag.or(then.flag),
        color_label: first.color_label.or(then.color_label),
        keywords: if first.keywords.is_empty() { then.keywords } else { first.keywords },
        title: first.title.or(then.title),
        caption: first.caption.or(then.caption),
    }
}

fn extras_of(m: Marks) -> ItemExtras {
    ItemExtras { rating: m.rating, flag: m.flag, color_label: m.color_label, keywords: m.keywords, title: m.title, caption: m.caption, ..Default::default() }
}

/// Where a sidecar of the asset at `original_rel` goes: the same path with the sidecar's own
/// extension (`2025/01/02/ab12cd34_IMG_0001.xmp`). The extension is the kind, in lower case.
fn sidecar_rel(original_rel: &str, kind: &str) -> String {
    let stem = Path::new(original_rel).with_extension("");
    format!("{}.{kind}", stem.to_string_lossy().replace('\\', "/"))
}

/// Copies one sidecar beside the original. `Ok(None)` for one too big to be a sidecar or whose
/// place is taken by different bytes: neither overwrites anything.
fn copy_one(lib: &Library, source: &dyn ImportSource, side: &SourceItem, original_rel: &str) -> Result<Option<(NewSidecar, Vec<u8>)>, ImportError> {
    let Some(kind) = sidecars::kind_for_extension(&photos_media::extension_of(Path::new(&side.file_name))) else { return Ok(None) };
    let mut bytes = Vec::new();
    source.open(side)?.take(MAX_SIDECAR_BYTES + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_SIDECAR_BYTES {
        return Ok(None);
    }
    let hash = blake3::hash(&bytes).to_hex().to_string();
    let relative_path = sidecar_rel(original_rel, kind);
    let dest = lib.originals_dir().join(&relative_path);
    if dest.exists() {
        // Already there (a re-import): fine when it is the same file, never overwritten when not.
        if std::fs::read(&dest).ok().map(|d| blake3::hash(&d).to_hex().to_string()).as_deref() != Some(hash.as_str()) {
            return Ok(None);
        }
    } else {
        let tmp = dest.with_extension(format!("{kind}.part"));
        std::fs::write(&tmp, &bytes)?;
        std::fs::rename(&tmp, &dest)?;
    }
    Ok(Some((NewSidecar { kind, relative_path, source_name: side.file_name.clone(), byte_size: bytes.len() as i64, content_hash: hash }, bytes)))
}

/// Brings `item`'s sidecars in beside the new asset's original and applies the marks of its
/// sidecar `.xmp` (first) and of its own embedded packet. A failure costs the sidecar, never
/// the photo, so it is logged.
pub fn bring_sidecars(lib: &Library, source: &dyn ImportSource, asset_id: i64, item: &SourceItem, original_rel: &str) {
    let mut marks = Marks::default();
    for side in source.sidecars(item) {
        match copy_one(lib, source, &side, original_rel) {
            Ok(Some((new, bytes))) => {
                if new.kind == sidecars::KIND_XMP {
                    marks = merge(marks, xmp::marks_in(&bytes));
                }
                if let Err(e) = lib.db.with_conn(|c| sidecars::record_sidecar(c, asset_id, &new, now_unix())) {
                    tracing::warn!(file = %side.rel_path, "could not record the sidecar: {e}");
                }
            }
            Ok(None) => tracing::warn!(file = %side.rel_path, "sidecar left behind: too big, or its place is taken"),
            Err(e) => tracing::warn!(file = %side.rel_path, "could not copy the sidecar: {e}"),
        }
    }
    marks = merge(marks, xmp::marks(&lib.originals_dir().join(original_rel)));
    apply_marks(lib, asset_id, marks, &item.rel_path);
}

/// *Import → Repair* (Round 21 §6): one sidecar the catalogue lacks, copied beside the asset's
/// original and recorded; an `.xmp` adds its marks. `Ok(false)` when it was left behind (too big,
/// or its place is taken by other bytes).
pub fn bring_one(lib: &Library, source: &dyn ImportSource, asset_id: i64, side: &SourceItem, original_rel: &str) -> Result<bool, ImportError> {
    let Some((new, bytes)) = copy_one(lib, source, side, original_rel)? else { return Ok(false) };
    lib.db.with_conn(|c| sidecars::record_sidecar(c, asset_id, &new, now_unix()))?;
    if new.kind == sidecars::KIND_XMP {
        apply_marks(lib, asset_id, xmp::marks_in(&bytes), &side.rel_path);
    }
    Ok(true)
}

/// A duplicate has its copy already; a sidecar that arrives with a second copy of the file
/// still adds its marks to it (adding only, as `extras::apply` does).
pub fn marks_for_duplicate(lib: &Library, source: &dyn ImportSource, asset_id: i64, item: &SourceItem) {
    let mut marks = Marks::default();
    for side in source.sidecars(item).iter().filter(|s| s.file_name.to_ascii_lowercase().ends_with(".xmp")) {
        if let Ok(mut r) = source.open(side) {
            let mut bytes = Vec::new();
            if r.by_ref().take(MAX_SIDECAR_BYTES).read_to_end(&mut bytes).is_ok() {
                marks = merge(marks, xmp::marks_in(&bytes));
            }
        }
    }
    apply_marks(lib, asset_id, marks, &item.rel_path);
}

fn apply_marks(lib: &Library, asset_id: i64, marks: Marks, rel_path: &str) {
    if marks.is_empty() {
        return;
    }
    if let Err(e) = lib.db.with_conn(|c| photos_core::extras::apply(c, asset_id, &extras_of(marks), now_unix())) {
        tracing::warn!(file = %rel_path, "could not apply the XMP marks: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_sidecar_lives_beside_its_original_under_its_kind() {
        assert_eq!(sidecar_rel("2025/01/02/ab12cd34_IMG_0001.jpg", "xmp"), "2025/01/02/ab12cd34_IMG_0001.xmp");
        assert_eq!(sidecar_rel("2025/01/02/ab12cd34_IMG_0001_2.heic", "aae"), "2025/01/02/ab12cd34_IMG_0001_2.aae");
    }

    #[test]
    fn the_first_marks_win_and_the_second_fill_the_gaps() {
        let a = Marks { rating: Some(5), title: Some("A".into()), ..Default::default() };
        let b = Marks { rating: Some(2), title: Some("B".into()), caption: Some("c".into()), keywords: vec!["k".into()], ..Default::default() };
        let m = merge(a, b);
        assert_eq!((m.rating, m.title.as_deref(), m.caption.as_deref(), m.keywords.len()), (Some(5), Some("A"), Some("c"), 1));
    }
}
