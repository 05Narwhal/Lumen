//! What Setup reads and writes in the person's registry (HKCU only), behind a trait so tests never
//! touch the real one.
use std::collections::HashMap;
use std::io;
use std::path::Path;

pub const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
/// `bundle.publisher` in `src-tauri/tauri.conf.json` (held by `publisher_matches_lumens_bundle`).
pub const PUBLISHER: &str = "JL Studios";

/// Product name, uninstall key and Run value from one place. A test build bakes `Lumen Test`
/// (`LUMEN_SETUP_PRODUCT`) so it never meets the real install.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Identity {
    pub product: String,
}

impl Identity {
    pub fn baked() -> Self {
        Self::named(option_env!("LUMEN_SETUP_PRODUCT").unwrap_or("Lumen"))
    }
    pub fn named(product: &str) -> Self {
        Self { product: product.to_string() }
    }
    /// Tauri's NSIS template keys the uninstall entry by product name.
    pub fn uninstall_key(&self) -> String {
        format!(r"Software\Microsoft\Windows\CurrentVersion\Uninstall\{}", self.product)
    }
    /// `HKCU\Software\<publisher>\<product>`: Tauri's NSIS template writes the install folder there
    /// and its uninstaller leaves it behind.
    pub fn product_key(&self) -> String {
        format!(r"Software\{PUBLISHER}\{}", self.product)
    }
    pub fn run_value_name(&self) -> &str {
        &self.product
    }
    /// `Lumen` becomes `lumen.exe`, `Lumen Test` becomes `lumen-test.exe`.
    pub fn exe_name(&self) -> String {
        format!("{}.exe", self.product.to_lowercase().replace(' ', "-"))
    }
}

pub trait Registry {
    fn read(&self, subkey: &str, name: &str) -> Option<String>;
    fn write(&mut self, subkey: &str, name: &str, value: &str) -> io::Result<()>;
    fn remove(&mut self, subkey: &str, name: &str) -> io::Result<()>;
    /// Delete the key only when it holds no values and no subkeys (never a tree).
    fn remove_key_if_empty(&mut self, subkey: &str) -> io::Result<()>;
}

/// The installed Lumen: `(DisplayVersion if present, InstallLocation)`; None when there is no
/// entry or it names no folder.
pub fn read_installed(reg: &dyn Registry, id: &Identity) -> Option<(Option<String>, String)> {
    let key = id.uninstall_key();
    // Tauri's NSIS template writes the folder in quotes; a path never holds a `"`.
    let location = reg.read(&key, "InstallLocation").map(|l| l.trim().trim_matches('"').trim().to_string()).filter(|l| !l.is_empty())?;
    Some((reg.read(&key, "DisplayVersion"), location))
}

/// `"<exe>" --minimized`, quoted so a path with spaces stays one program.
pub fn run_value(exe: &Path) -> String {
    format!("\"{}\" --minimized", exe.display())
}

/// Turn start-with-Windows on or off. Off removes only our value.
pub fn set_start_with_windows(reg: &mut dyn Registry, id: &Identity, exe: &Path, on: bool) -> io::Result<()> {
    if on {
        reg.write(RUN_KEY, id.run_value_name(), &run_value(exe))
    } else {
        reg.remove(RUN_KEY, id.run_value_name())
    }
}

/// The person's own registry (HKCU). Nothing here ever needs elevation.
#[cfg(windows)]
pub struct HkcuRegistry;

#[cfg(windows)]
impl Registry for HkcuRegistry {
    fn read(&self, subkey: &str, name: &str) -> Option<String> {
        use winreg::{enums::KEY_READ, RegKey};
        RegKey::predef(winreg::enums::HKEY_CURRENT_USER).open_subkey_with_flags(subkey, KEY_READ).ok()?.get_value::<String, _>(name).ok()
    }
    fn write(&mut self, subkey: &str, name: &str, value: &str) -> io::Result<()> {
        let (key, _) = winreg::RegKey::predef(winreg::enums::HKEY_CURRENT_USER).create_subkey(subkey)?;
        key.set_value(name, &value)
    }
    fn remove(&mut self, subkey: &str, name: &str) -> io::Result<()> {
        use winreg::enums::KEY_SET_VALUE;
        let Ok(key) = winreg::RegKey::predef(winreg::enums::HKEY_CURRENT_USER).open_subkey_with_flags(subkey, KEY_SET_VALUE) else { return Ok(()) };
        match key.delete_value(name) {
            Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        }
    }
    fn remove_key_if_empty(&mut self, subkey: &str) -> io::Result<()> {
        use winreg::enums::{HKEY_CURRENT_USER, KEY_READ};
        let hkcu = winreg::RegKey::predef(HKEY_CURRENT_USER);
        let Ok(key) = hkcu.open_subkey_with_flags(subkey, KEY_READ) else { return Ok(()) };
        let info = key.query_info()?;
        drop(key);
        if info.values == 0 && info.sub_keys == 0 {
            // RegDeleteKeyW: refuses a key with subkeys, so this never removes a tree.
            hkcu.delete_subkey(subkey)?;
        }
        Ok(())
    }
}

#[derive(Default)]
pub struct MemoryRegistry(pub HashMap<(String, String), String>);

impl Registry for MemoryRegistry {
    fn read(&self, subkey: &str, name: &str) -> Option<String> {
        self.0.get(&(subkey.to_string(), name.to_string())).cloned()
    }
    fn write(&mut self, subkey: &str, name: &str, value: &str) -> io::Result<()> {
        self.0.insert((subkey.to_string(), name.to_string()), value.to_string());
        Ok(())
    }
    fn remove(&mut self, subkey: &str, name: &str) -> io::Result<()> {
        self.0.remove(&(subkey.to_string(), name.to_string()));
        Ok(())
    }
    /// Keys exist here only through their values, so an empty one is already gone.
    fn remove_key_if_empty(&mut self, _subkey: &str) -> io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn identity_names_follow_the_product() {
        let id = Identity::named("Lumen Test");
        assert_eq!(id.exe_name(), "lumen-test.exe");
        assert!(id.uninstall_key().ends_with(r"\Uninstall\Lumen Test"));
        assert_eq!(Identity::named("Lumen").exe_name(), "lumen.exe");
    }

    #[test]
    fn publisher_matches_lumens_bundle() {
        let conf: serde_json::Value = serde_json::from_str(include_str!("../../../src-tauri/tauri.conf.json")).unwrap();
        assert_eq!(conf["bundle"]["publisher"], PUBLISHER);
        assert_eq!(Identity::named("Lumen Test").product_key(), r"Software\JL Studios\Lumen Test");
    }

    #[test]
    fn run_value_is_quoted_with_minimized() {
        let exe = PathBuf::from(r"C:\Users\A B\Apps\Lumen\lumen.exe");
        assert_eq!(run_value(&exe), r#""C:\Users\A B\Apps\Lumen\lumen.exe" --minimized"#);
    }

    #[test]
    fn start_with_windows_writes_and_removes_only_our_value() {
        let id = Identity::named("Lumen");
        let mut reg = MemoryRegistry::default();
        reg.write(RUN_KEY, "Other App", "other.exe").unwrap();
        let exe = PathBuf::from(r"C:\L\lumen.exe");
        set_start_with_windows(&mut reg, &id, &exe, true).unwrap();
        assert_eq!(reg.read(RUN_KEY, "Lumen").unwrap(), r#""C:\L\lumen.exe" --minimized"#);
        set_start_with_windows(&mut reg, &id, &exe, false).unwrap();
        assert_eq!(reg.read(RUN_KEY, "Lumen"), None);
        assert_eq!(reg.read(RUN_KEY, "Other App").unwrap(), "other.exe");
    }

    #[test]
    fn read_installed_gives_version_and_location() {
        let id = Identity::named("Lumen");
        let mut reg = MemoryRegistry::default();
        assert_eq!(read_installed(&reg, &id), None);
        let k = id.uninstall_key();
        reg.write(&k, "InstallLocation", r"C:\L").unwrap();
        assert_eq!(read_installed(&reg, &id), Some((None, r"C:\L".into())), "no DisplayVersion");
        reg.write(&k, "DisplayVersion", "26.1.0").unwrap();
        assert_eq!(read_installed(&reg, &id), Some((Some("26.1.0".into()), r"C:\L".into())));
    }

    #[test]
    fn a_quoted_location_is_read_without_its_quotes() {
        // Tauri's NSIS template writes InstallLocation as "C:\…\Lumen" (quotes included).
        let id = Identity::named("Lumen");
        let mut reg = MemoryRegistry::default();
        let k = id.uninstall_key();
        reg.write(&k, "InstallLocation", r#""C:\Users\A B\AppData\Local\Lumen""#).unwrap();
        assert_eq!(read_installed(&reg, &id).map(|(_, l)| l), Some(r"C:\Users\A B\AppData\Local\Lumen".into()));
        reg.write(&k, "InstallLocation", r#" "" "#).unwrap();
        assert_eq!(read_installed(&reg, &id), None, "only quotes is no location");
    }

    #[test]
    fn an_entry_with_no_location_is_not_an_install() {
        let id = Identity::named("Lumen");
        let mut reg = MemoryRegistry::default();
        reg.write(&id.uninstall_key(), "DisplayVersion", "26.1.0").unwrap();
        assert_eq!(read_installed(&reg, &id), None);
    }
}
