//! What the person chose, and the command line the NSIS installer gets for it.
use serde::{Deserialize, Serialize};
use std::ffi::OsString;
use std::path::PathBuf;
use ts_rs::TS;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum InstallMode {
    Install,
    Update,
    Repair,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct InstallOptions {
    #[ts(type = "string")]
    pub folder: PathBuf,
    pub desktop_shortcut: bool,
    pub start_with_windows: bool,
    pub download_models: bool,
    pub mode: InstallMode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum ModelsOutcome {
    Complete,
    Later,
}

/// What `install` resolves with (phase 5 fills it).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct InstallResult {
    pub models: ModelsOutcome,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum SetupTask {
    Lumen,
    Models,
}

/// The `setup://progress` event payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct SetupProgress {
    pub task: SetupTask,
    #[ts(type = "number")]
    pub done: u64,
    #[ts(type = "number")]
    pub total: u64,
}

/// Arguments for the NSIS installer: `/S`, `/UPDATE` when updating, `/D=<folder>` last and unquoted
/// (NSIS reads everything after `/D=` as the folder, spaces included).
pub fn nsis_args(opts: &InstallOptions) -> Vec<OsString> {
    let mut args = vec![OsString::from("/S")];
    if opts.mode == InstallMode::Update {
        args.push(OsString::from("/UPDATE"));
    }
    let mut d = OsString::from("/D=");
    d.push(&opts.folder);
    args.push(d);
    args
}

/// Whether the desktop shortcut exists after the install. A fresh install follows the switch; an
/// update or repair leaves the person's shortcut as it was.
pub fn desktop_shortcut_after(opts: &InstallOptions, existed: bool) -> bool {
    match opts.mode {
        InstallMode::Install => opts.desktop_shortcut,
        InstallMode::Update | InstallMode::Repair => existed,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opts(mode: InstallMode, desktop: bool) -> InstallOptions {
        InstallOptions {
            folder: PathBuf::from(r"C:\Users\A B\Apps\Lumen"),
            desktop_shortcut: desktop,
            start_with_windows: false,
            download_models: true,
            mode,
        }
    }

    #[test]
    fn nsis_args_put_d_last() {
        for mode in [InstallMode::Install, InstallMode::Update, InstallMode::Repair] {
            let args = nsis_args(&opts(mode, true));
            assert_eq!(args.first().unwrap(), "/S");
            assert_eq!(args.last().unwrap(), r"/D=C:\Users\A B\Apps\Lumen", "unquoted, spaces kept");
            assert_eq!(args.iter().filter(|a| a.to_string_lossy().starts_with("/D=")).count(), 1);
            assert_eq!(args.iter().any(|a| a == "/UPDATE"), mode == InstallMode::Update);
        }
    }

    #[test]
    fn desktop_shortcut_kept_on_update() {
        // Update and repair keep what the person had, whatever the switch says.
        for mode in [InstallMode::Update, InstallMode::Repair] {
            assert!(desktop_shortcut_after(&opts(mode, false), true));
            assert!(!desktop_shortcut_after(&opts(mode, true), false));
        }
        // A fresh install follows the switch.
        assert!(desktop_shortcut_after(&opts(InstallMode::Install, true), false));
        assert!(!desktop_shortcut_after(&opts(InstallMode::Install, false), true));
    }
}
