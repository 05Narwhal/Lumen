//! File formats the catalogue needs to name. Decoding lives in `photos-media`; this is only
//! what the library must know without decoding anything (the RAW collection, the RAW badge).

/// Camera RAW extensions, lower case (`docs/RAW-FORMATS.md`). `.raw` names too many other things.
pub const RAW_EXTENSIONS: &[&str] = &[
    "cr2", "cr3", "crw", "nef", "nrw", "arw", "srf", "sr2", "dng", "raf", "orf", "ori", "rw2", "rwl", "pef", "srw", "3fr",
    "fff", "iiq", "erf", "mos", "mrw", "kdc", "dcr", "gpr", "x3f",
];

pub fn is_raw(extension: &str) -> bool {
    RAW_EXTENSIONS.contains(&extension.to_ascii_lowercase().as_str())
}

/// `('cr2','cr3',…)` for an `IN` clause. The list is constant, so it is inlined, not bound.
pub fn raw_sql_list() -> String {
    format!("({})", RAW_EXTENSIONS.iter().map(|e| format!("'{e}'")).collect::<Vec<_>>().join(","))
}

#[cfg(test)]
mod tests {
    #[test]
    fn raw_is_case_insensitive_and_excludes_ordinary_formats() {
        assert!(super::is_raw("NEF") && super::is_raw("dng"));
        assert!(!super::is_raw("heic") && !super::is_raw("jpg"));
    }

    /// RAWs raw.pixls.us holds that Lumen skipped on import (docs/RAW-FORMATS.md).
    #[test]
    fn every_raw_the_sweep_found_is_a_raw() {
        for e in ["crw", "fff", "rwl", "ori", "gpr"] {
            assert!(super::is_raw(e), "{e}");
        }
        // `.raw` is too common a name for other things (audio, disk images) to import.
        assert!(!super::is_raw("raw"));
    }
}
