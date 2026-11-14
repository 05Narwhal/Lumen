//! Settings → RAW formats (Round 17): whether Adobe DNG Converter is installed, and the DNG
//! cache Lumen keeps for RAWs rawler refuses (`photos_media::develop::stand_in`). Lumen never
//! ships, downloads or installs the converter; it runs the copy the person installed.

use std::path::PathBuf;

use photos_media::develop::stand_in::{self, CacheUse};
use serde::Serialize;
use ts_rs::TS;

use crate::error::AppResult;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum ConverterState {
    Found,
    NotFound,
    /// Adobe makes no Linux version: RAWs rawler refuses use the camera's JPEG.
    NoLinuxVersion,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RawFormatsStatus {
    pub converter: ConverterState,
    pub converter_path: Option<String>,
    #[ts(type = "number")]
    pub cache_bytes: u64,
    #[ts(type = "number")]
    pub cache_files: u64,
    /// The environment variable that names (or, pointing at nothing, hides) the converter.
    pub env_var: String,
}

/// What Settings shows, from what was found.
pub fn status_for(converter: Option<PathBuf>, linux: bool, cache: CacheUse) -> RawFormatsStatus {
    let state = match (&converter, linux) {
        (Some(_), _) => ConverterState::Found,
        (None, true) => ConverterState::NoLinuxVersion,
        (None, false) => ConverterState::NotFound,
    };
    RawFormatsStatus {
        converter: state,
        converter_path: converter.map(|p| p.display().to_string()),
        cache_bytes: cache.bytes,
        cache_files: cache.files,
        env_var: stand_in::CONVERTER_ENV.into(),
    }
}

/// Looks for the converter again (an install counts without a restart) and measures the cache.
#[tauri::command(async)]
pub fn get_raw_formats_status() -> AppResult<RawFormatsStatus> {
    Ok(status_for(stand_in::converter(), cfg!(target_os = "linux"), stand_in::configured_cache_use()))
}

/// Empties the DNG cache; each photo converts again the next time it is opened. Returns the
/// bytes freed.
#[tauri::command(async)]
pub fn clear_dng_cache() -> AppResult<u64> {
    let before = stand_in::configured_cache_use().bytes;
    stand_in::clear_configured_cache();
    Ok(before.saturating_sub(stand_in::configured_cache_use().bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn found_not_found_and_linux() {
        let used = CacheUse { bytes: 5_000_000, files: 2 };
        let found = status_for(Some(PathBuf::from("C:/Program Files/Adobe/Adobe DNG Converter/Adobe DNG Converter.exe")), false, used);
        assert_eq!(found.converter, ConverterState::Found);
        assert!(found.converter_path.unwrap().ends_with("Adobe DNG Converter.exe"));
        assert_eq!((found.cache_bytes, found.cache_files), (5_000_000, 2));
        assert_eq!(status_for(None, false, CacheUse::default()).converter, ConverterState::NotFound);
        assert_eq!(status_for(None, true, CacheUse::default()).converter, ConverterState::NoLinuxVersion);
        // A converter named by the variable on Linux (someone's own route) is still found.
        assert_eq!(status_for(Some(PathBuf::from("/opt/dng")), true, CacheUse::default()).converter, ConverterState::Found);
        assert_eq!(found.env_var, "LUMEN_DNG_CONVERTER");
    }
}
