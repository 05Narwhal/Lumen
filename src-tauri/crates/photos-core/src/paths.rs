use std::path::{Path, PathBuf};

use crate::{CoreError, Result};

pub const COMPANY_DIR: &str = "JL-Studios";
pub const APP_DIR: &str = "Lumen";
pub const DEFAULT_LIBRARY_NAME: &str = "Lumen Library";

/// Where Lumen keeps its own state (settings, registry, logs) and where a new
/// library goes by default. App data always follows the JL Studios convention:
/// `<config_dir>/JL-Studios/Lumen`.
#[derive(Debug, Clone)]
pub struct AppPaths {
    pub app_data: PathBuf,
    pub default_library: PathBuf,
}

impl AppPaths {
    pub fn new(app_data: impl Into<PathBuf>, default_library: impl Into<PathBuf>) -> Self {
        Self { app_data: app_data.into(), default_library: default_library.into() }
    }

    pub fn app_data_under(config_dir: &Path) -> PathBuf {
        config_dir.join(COMPANY_DIR).join(APP_DIR)
    }

    pub fn default_library_under(pictures_dir: &Path) -> PathBuf {
        pictures_dir.join(DEFAULT_LIBRARY_NAME)
    }

    /// Resolve from the OS, honouring `LUMEN_APP_DATA_DIR` / `LUMEN_DEFAULT_LIBRARY_DIR`. A test
    /// build can also be given defaults when it is compiled (`LUMEN_BAKED_*`, Round 17): an
    /// installer's relaunch starts the app without this environment, and that launch must not
    /// reach a real library. The release script refuses a build with them set.
    pub fn resolve() -> Result<Self> {
        let app_data = match std::env::var_os("LUMEN_APP_DATA_DIR").or_else(|| option_env!("LUMEN_BAKED_APP_DATA_DIR").map(Into::into)) {
            Some(v) => PathBuf::from(v),
            None => {
                let cfg = dirs::config_dir()
                    .ok_or_else(|| CoreError::InvalidPath("no config dir".into()))?;
                Self::app_data_under(&cfg)
            }
        };
        let default_library = match std::env::var_os("LUMEN_DEFAULT_LIBRARY_DIR").or_else(|| option_env!("LUMEN_BAKED_LIBRARY_DIR").map(Into::into)) {
            Some(v) => PathBuf::from(v),
            None => {
                let pics = dirs::picture_dir()
                    .or_else(dirs::home_dir)
                    .ok_or_else(|| CoreError::InvalidPath("no pictures dir".into()))?;
                Self::default_library_under(&pics)
            }
        };
        Ok(Self { app_data, default_library })
    }

    pub fn settings_file(&self) -> PathBuf {
        self.app_data.join("settings.json")
    }

    pub fn registry_file(&self) -> PathBuf {
        self.app_data.join("libraries.json")
    }

    /// Each library's backup: destination, schedule, last run (Round 5A).
    pub fn backups_file(&self) -> PathBuf {
        self.app_data.join("backups.json")
    }

    pub fn logs_dir(&self) -> PathBuf {
        self.app_data.join("logs")
    }

    /// Downloaded ML models (Phase 7). Under app data, never the library: a library is
    /// backed up and moved, and models can always be fetched again.
    pub fn models_dir(&self) -> PathBuf {
        self.app_data.join("models")
    }

    /// A downloaded update waiting for Close, and what the updater remembers (Round 17).
    pub fn updates_dir(&self) -> PathBuf {
        self.app_data.join("updates")
    }

    /// Copies taken before a library is upgraded, one folder per library (Round 17). Under app
    /// data, never in a library: a safety copy must not be lost with what it protects.
    pub fn pre_migration_dir(&self) -> PathBuf {
        self.app_data.join("backups").join("pre-migration")
    }

    pub fn ensure_dirs(&self) -> Result<()> {
        std::fs::create_dir_all(self.logs_dir())?;
        std::fs::create_dir_all(self.models_dir())?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derived_files_live_under_app_data() {
        let p = AppPaths::new("/tmp/appdata", "/tmp/pics/Lumen Library");
        assert_eq!(p.settings_file(), Path::new("/tmp/appdata/settings.json"));
        assert_eq!(p.registry_file(), Path::new("/tmp/appdata/libraries.json"));
        assert_eq!(p.logs_dir(), Path::new("/tmp/appdata/logs"));
    }

    #[test]
    fn app_data_dir_follows_jl_studios_convention() {
        let root = AppPaths::app_data_under(Path::new("/cfg"));
        assert_eq!(root, Path::new("/cfg/JL-Studios/Lumen"));
        let lib = AppPaths::default_library_under(Path::new("/home/u/Pictures"));
        assert_eq!(lib, Path::new("/home/u/Pictures/Lumen Library"));
    }

    #[test]
    fn ensure_dirs_creates_app_data_and_logs() {
        let tmp = tempfile::tempdir().unwrap();
        let p = AppPaths::new(tmp.path().join("app"), tmp.path().join("lib"));
        p.ensure_dirs().unwrap();
        assert!(p.logs_dir().is_dir());
        assert!(p.models_dir().is_dir());
        assert!(!p.default_library.exists(), "library is created explicitly, not here");
    }
}
