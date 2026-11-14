//! The desktop shortcut after a silent install. Tauri's NSIS template decides for itself whether
//! to create it; Setup only removes what the person did not ask for, and never creates one.
use crate::options::{desktop_shortcut_after, InstallOptions};
use crate::registry::Identity;
use std::io;
use std::path::{Path, PathBuf};

/// The two file operations Setup needs, so tests never touch a real desktop.
pub trait ShortcutFs {
    fn exists(&self, path: &Path) -> bool;
    fn remove(&mut self, path: &Path) -> io::Result<()>;
}

pub struct RealFs;

impl ShortcutFs for RealFs {
    fn exists(&self, path: &Path) -> bool {
        path.is_file()
    }
    fn remove(&mut self, path: &Path) -> io::Result<()> {
        std::fs::remove_file(path)
    }
}

/// `<Desktop>\<product>.lnk`, the name Tauri's template gives it.
pub fn desktop_shortcut_path(desktop: &Path, id: &Identity) -> PathBuf {
    desktop.join(format!("{}.lnk", id.product))
}

/// Remove the shortcut when the person's options say it should not be there. Returns whether it
/// was removed. `existed` is whether it was there before the installer ran.
pub fn settle_desktop_shortcut(
    fs: &mut dyn ShortcutFs,
    desktop: &Path,
    id: &Identity,
    opts: &InstallOptions,
    existed: bool,
) -> io::Result<bool> {
    let link = desktop_shortcut_path(desktop, id);
    if desktop_shortcut_after(opts, existed) || !fs.exists(&link) {
        return Ok(false);
    }
    fs.remove(&link)?;
    Ok(true)
}

/// The person's Desktop folder (follows a OneDrive redirect), None when Windows will not say.
#[cfg(windows)]
pub fn desktop_dir() -> Option<PathBuf> {
    use windows_sys::Win32::System::Com::CoTaskMemFree;
    use windows_sys::Win32::UI::Shell::{FOLDERID_Desktop, SHGetKnownFolderPath};
    let mut raw: *mut u16 = std::ptr::null_mut();
    // SAFETY: on success `raw` is a NUL-terminated wide string owned by COM; it is copied and freed.
    unsafe {
        let hr = SHGetKnownFolderPath(&FOLDERID_Desktop, 0, std::ptr::null_mut(), &mut raw);
        let path = if hr == 0 && !raw.is_null() {
            let len = (0..).take_while(|&i| *raw.add(i) != 0).count();
            let wide = std::slice::from_raw_parts(raw, len);
            Some(PathBuf::from(<std::ffi::OsString as std::os::windows::ffi::OsStringExt>::from_wide(wide)))
        } else {
            None
        };
        CoTaskMemFree(raw as *const _);
        path
    }
}

#[cfg(not(windows))]
pub fn desktop_dir() -> Option<PathBuf> {
    None
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use crate::options::InstallMode;
    use std::cell::RefCell;
    use std::collections::HashSet;
    use std::rc::Rc;

    /// A desktop in memory; clones share it, so a test can play the installer from outside.
    #[derive(Default, Clone)]
    pub struct FakeFs(pub Rc<RefCell<HashSet<PathBuf>>>);

    impl ShortcutFs for FakeFs {
        fn exists(&self, p: &Path) -> bool {
            self.0.borrow().contains(p)
        }
        fn remove(&mut self, p: &Path) -> io::Result<()> {
            if self.0.borrow_mut().remove(p) {
                Ok(())
            } else {
                Err(io::ErrorKind::NotFound.into())
            }
        }
    }

    fn opts(mode: InstallMode, desktop: bool) -> InstallOptions {
        InstallOptions {
            folder: PathBuf::from("C:/x"),
            desktop_shortcut: desktop,
            start_with_windows: false,
            download_models: false,
            mode,
        }
    }

    fn setup(has: bool) -> (FakeFs, PathBuf, Identity) {
        let id = Identity::named("Lumen Test");
        let desktop = PathBuf::from("C:/Users/A/Desktop");
        let fs = FakeFs::default();
        if has {
            fs.0.borrow_mut().insert(desktop_shortcut_path(&desktop, &id));
        }
        (fs, desktop, id)
    }

    #[test]
    fn a_silent_install_desktop_shortcut_is_removed_when_off() {
        let (mut fs, desktop, id) = setup(true);
        let removed = settle_desktop_shortcut(&mut fs, &desktop, &id, &opts(InstallMode::Install, false), false).unwrap();
        assert!(removed);
        assert!(!fs.exists(&desktop_shortcut_path(&desktop, &id)));
    }

    #[test]
    fn the_shortcut_stays_when_the_switch_is_on() {
        let (mut fs, desktop, id) = setup(true);
        assert!(!settle_desktop_shortcut(&mut fs, &desktop, &id, &opts(InstallMode::Install, true), false).unwrap());
        assert!(fs.exists(&desktop_shortcut_path(&desktop, &id)));
    }

    #[test]
    fn an_update_keeps_a_shortcut_the_person_already_had() {
        let (mut fs, desktop, id) = setup(true);
        // The switch is off, but the person had it before: an update leaves it.
        assert!(!settle_desktop_shortcut(&mut fs, &desktop, &id, &opts(InstallMode::Update, false), true).unwrap());
        assert!(fs.exists(&desktop_shortcut_path(&desktop, &id)));
    }

    #[test]
    fn nothing_to_remove_is_not_an_error() {
        let (mut fs, desktop, id) = setup(false);
        assert!(!settle_desktop_shortcut(&mut fs, &desktop, &id, &opts(InstallMode::Install, false), false).unwrap());
    }

    #[test]
    fn only_our_own_lnk_is_named() {
        let id = Identity::named("Lumen Test");
        assert_eq!(desktop_shortcut_path(Path::new("D"), &id), Path::new("D").join("Lumen Test.lnk"));
    }
}
