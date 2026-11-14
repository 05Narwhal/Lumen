//! Is this folder fine to install into?
use serde::Serialize;
use std::path::Path;
use ts_rs::TS;

/// Every variant carries the free space of the drive the folder is on (Options' footer shows it).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[ts(export)]
pub enum FolderCheck {
    Ok {
        #[ts(type = "number")]
        free: u64,
    },
    NeedsAdmin {
        #[ts(type = "number")]
        free: u64,
    },
    NotEmpty {
        #[ts(type = "number")]
        free: u64,
    },
    NoSpace {
        #[ts(type = "number")]
        free: u64,
    },
    IsFile {
        #[ts(type = "number")]
        free: u64,
    },
}

/// `lumen_exe` is the installed program's file name; a folder holding it is an existing install.
pub fn folder_check(path: &Path, need_bytes: u64, lumen_exe: &str) -> FolderCheck {
    folder_check_with(path, need_bytes, lumen_exe, free_bytes)
}

pub fn folder_check_with(path: &Path, need_bytes: u64, lumen_exe: &str, free: impl Fn(&Path) -> u64) -> FolderCheck {
    let free = free(path);
    if path.is_file() {
        return FolderCheck::IsFile { free };
    }
    if needs_admin(path) {
        return FolderCheck::NeedsAdmin { free };
    }
    if path.is_dir() && !path.join(lumen_exe).is_file() && has_entries(path) {
        return FolderCheck::NotEmpty { free };
    }
    if free < need_bytes {
        return FolderCheck::NoSpace { free };
    }
    FolderCheck::Ok { free }
}

fn has_entries(dir: &Path) -> bool {
    // An unreadable folder counts as holding something.
    std::fs::read_dir(dir).map(|mut d| d.next().is_some()).unwrap_or(true)
}

/// A folder the person cannot write without elevation: under the machine's program folders, or
/// one where creating a file fails. Nothing is created in a protected root.
fn needs_admin(path: &Path) -> bool {
    let lower = |p: &Path| p.to_string_lossy().to_lowercase().replace('/', "\\");
    let here = lower(path);
    for var in ["ProgramFiles", "ProgramFiles(x86)", "ProgramW6432", "SystemRoot"] {
        if let Some(root) = std::env::var_os(var) {
            let root = lower(Path::new(&root));
            let root = root.trim_end_matches('\\');
            if !root.is_empty() && (here == root || here.starts_with(&format!("{root}\\"))) {
                return true;
            }
        }
    }
    let Some(existing) = nearest_existing(path) else { return false };
    let probe = existing.join(format!(".lumen-setup-probe-{}", std::process::id()));
    match std::fs::File::create(&probe) {
        Ok(_) => {
            let _ = std::fs::remove_file(&probe);
            false
        }
        Err(_) => true,
    }
}

fn nearest_existing(path: &Path) -> Option<&Path> {
    path.ancestors().find(|a| !a.as_os_str().is_empty() && a.is_dir())
}

/// Free bytes on the drive holding `path` (its nearest existing ancestor); 0 when unknown.
pub fn free_bytes(path: &Path) -> u64 {
    nearest_existing(path).map(disk_free).unwrap_or(0)
}

#[cfg(windows)]
fn disk_free(dir: &Path) -> u64 {
    use std::os::windows::ffi::OsStrExt;
    let wide: Vec<u16> = dir.as_os_str().encode_wide().chain(Some(0)).collect();
    let mut free_to_caller = 0u64;
    // SAFETY: `wide` is NUL-terminated and outlives the call; the out pointer is a live local and
    // the two optional out parameters are null, which the API allows.
    let ok = unsafe {
        windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW(wide.as_ptr(), &mut free_to_caller, std::ptr::null_mut(), std::ptr::null_mut())
    };
    if ok == 0 {
        0
    } else {
        free_to_caller
    }
}

#[cfg(not(windows))]
fn disk_free(_dir: &Path) -> u64 {
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXE: &str = "lumen.exe";
    fn plenty(_: &Path) -> u64 {
        10_000_000_000
    }

    #[test]
    fn a_new_folder_is_ok() {
        let t = tempfile::tempdir().unwrap();
        let p = t.path().join("Lumen");
        assert_eq!(folder_check_with(&p, 300, EXE, plenty), FolderCheck::Ok { free: 10_000_000_000 });
    }

    #[test]
    fn an_empty_existing_folder_is_ok() {
        let t = tempfile::tempdir().unwrap();
        assert!(matches!(folder_check_with(t.path(), 300, EXE, plenty), FolderCheck::Ok { .. }));
    }

    #[test]
    fn a_file_is_is_file() {
        let t = tempfile::tempdir().unwrap();
        let f = t.path().join("x.txt");
        std::fs::write(&f, "x").unwrap();
        assert!(matches!(folder_check_with(&f, 300, EXE, plenty), FolderCheck::IsFile { .. }));
    }

    #[test]
    fn a_non_lumen_folder_with_files_is_not_empty() {
        let t = tempfile::tempdir().unwrap();
        std::fs::write(t.path().join("notes.txt"), "x").unwrap();
        assert!(matches!(folder_check_with(t.path(), 300, EXE, plenty), FolderCheck::NotEmpty { .. }));
    }

    #[test]
    fn an_existing_lumen_install_is_ok() {
        let t = tempfile::tempdir().unwrap();
        std::fs::write(t.path().join(EXE), "x").unwrap();
        std::fs::write(t.path().join("other.dll"), "x").unwrap();
        assert!(matches!(folder_check_with(t.path(), 300, EXE, plenty), FolderCheck::Ok { .. }));
    }

    #[test]
    fn too_little_free_space_is_no_space_with_the_free_figure() {
        let t = tempfile::tempdir().unwrap();
        let small = |_: &Path| 100u64;
        assert_eq!(folder_check_with(t.path(), 300, EXE, small), FolderCheck::NoSpace { free: 100 });
    }

    #[test]
    fn program_files_needs_admin_without_touching_it() {
        let Some(pf) = std::env::var_os("ProgramFiles") else { return };
        let p = Path::new(&pf).join("Lumen Test Folder That Must Not Be Created");
        assert!(matches!(folder_check_with(&p, 300, EXE, plenty), FolderCheck::NeedsAdmin { .. }));
        assert!(!p.exists());
    }

    #[test]
    fn free_bytes_of_a_real_folder_is_positive() {
        let t = tempfile::tempdir().unwrap();
        assert!(free_bytes(&t.path().join("not").join("yet")) > 0);
    }
}
