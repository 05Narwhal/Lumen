//! Thin adapters over the plugin store (Round 23 C1, spec §3, §5, §11). The work is in plain
//! functions taking `&Plugins`, so tests need no Tauri runtime; each command wraps one, then
//! tells the page and the listeners that the list changed.

use std::path::Path;
use std::sync::Arc;

use photos_plugins::store::RemoveToken;
use photos_plugins::{PluginError, PluginInfo, PluginInspect};
use serde_json::Value;
use tauri::{AppHandle, State};

use crate::error::AppResult;
use crate::plugins::Plugins;

/// What the plugin commands do, without Tauri: each takes the store's lock for as long as it
/// needs, so two installs at once run one after the other and `plugins.json` holds both.
/// The scheme forms `src/utils/media.ts` builds: WebView2 only serves custom schemes as
/// `http://photos.localhost/…`. The version keeps an updated plugin's icon from a stale copy.
fn icon_url_for(windows: bool, id: &str, version: &str) -> String {
    let base = if windows { "http://photos.localhost" } else { "photos://localhost" };
    format!("{base}/plugin-icon/{id}?v={version}")
}

/// The store marks "has an icon" with a `photos://` URL of its own; the page needs this OS's form.
fn localised(mut info: PluginInfo) -> PluginInfo {
    if info.icon_url.is_some() {
        info.icon_url = Some(icon_url_for(cfg!(windows), &info.id, &info.version));
    }
    info
}

pub(crate) mod logic {
    use super::*;

    pub fn list(p: &Plugins) -> Vec<PluginInfo> {
        p.read().list().into_iter().map(localised).collect()
    }

    /// Unpacks into a scratch folder the store deletes again; nothing is installed.
    pub fn inspect(p: &Plugins, file: &Path) -> Result<PluginInspect, PluginError> {
        let ctx = p.ctx();
        p.read().inspect(file, &ctx)
    }

    pub fn install(p: &Plugins, file: &Path) -> Result<PluginInfo, PluginError> {
        let ctx = p.ctx();
        p.write().install(file, &ctx).map(localised)
    }

    pub fn set_enabled(p: &Plugins, id: &str, on: bool) -> Result<(), PluginError> {
        p.write().set_enabled(id, on)
    }

    /// The token that undoes the removal in this session.
    pub fn remove(p: &Plugins, id: &str) -> Result<String, PluginError> {
        p.write().remove(id).map(|t| t.0)
    }

    /// Any failure here means the files are gone (the next start's sweep, or a token from an
    /// earlier session), so the person gets one sentence, not a path.
    pub fn undo_remove(p: &Plugins, token: &str) -> Result<(), PluginError> {
        p.write().undo_remove(&RemoveToken(token.to_string())).map_err(|e| {
            tracing::warn!(error = %e, "a removed plugin couldn't be put back");
            PluginError::invalid("That plugin can't be restored any more")
        })
    }

    pub fn set_setting(p: &Plugins, id: &str, key: &str, value: Value) -> Result<(), PluginError> {
        p.write().set_setting(id, key, value)
    }

    pub fn developer_mode(p: &Plugins) -> bool {
        p.read().developer_mode()
    }

    pub fn set_developer_mode(p: &Plugins, on: bool) -> Result<(), PluginError> {
        p.write().set_developer_mode(on)
    }

    pub fn load_dev(p: &Plugins, folder: &Path) -> Result<PluginInfo, PluginError> {
        let ctx = p.ctx();
        p.write().load_dev(folder, &ctx).map(localised)
    }

    pub fn reload_dev(p: &Plugins, id: &str) -> Result<PluginInfo, PluginError> {
        let ctx = p.ctx();
        p.write().reload_dev(id, &ctx).map(localised)
    }

    pub fn log(p: &Plugins, id: &str) -> Vec<String> {
        p.log_lines(id)
    }
}

#[tauri::command(async)]
pub fn list_plugins(plugins: State<'_, Arc<Plugins>>) -> AppResult<Vec<PluginInfo>> {
    Ok(logic::list(&plugins))
}

/// Whether Developer mode is on, read from the store (it survives a restart).
#[tauri::command(async)]
pub fn get_plugin_developer_mode(plugins: State<'_, Arc<Plugins>>) -> AppResult<bool> {
    Ok(logic::developer_mode(&plugins))
}

/// What the install sheet shows for a `.lumenplugin`; nothing is installed.
#[tauri::command(async)]
pub fn inspect_plugin_package(plugins: State<'_, Arc<Plugins>>, path: String) -> AppResult<PluginInspect> {
    Ok(photos_jobs::interactive(|| logic::inspect(&plugins, Path::new(&path)))?)
}

#[tauri::command(async)]
pub fn install_plugin(app: AppHandle, plugins: State<'_, Arc<Plugins>>, path: String) -> AppResult<PluginInfo> {
    let info = photos_jobs::interactive(|| logic::install(&plugins, Path::new(&path)))?;
    plugins.changed(&app);
    Ok(info)
}

#[tauri::command(async)]
pub fn set_plugin_enabled(app: AppHandle, plugins: State<'_, Arc<Plugins>>, id: String, on: bool) -> AppResult<()> {
    logic::set_enabled(&plugins, &id, on)?;
    plugins.changed(&app);
    Ok(())
}

/// Returns the token that `undo_remove_plugin` takes within this session.
#[tauri::command(async)]
pub fn remove_plugin(app: AppHandle, plugins: State<'_, Arc<Plugins>>, id: String) -> AppResult<String> {
    let token = logic::remove(&plugins, &id)?;
    plugins.changed(&app);
    Ok(token)
}

#[tauri::command(async)]
pub fn undo_remove_plugin(app: AppHandle, plugins: State<'_, Arc<Plugins>>, token: String) -> AppResult<()> {
    logic::undo_remove(&plugins, &token)?;
    plugins.changed(&app);
    Ok(())
}

#[tauri::command(async)]
pub fn set_plugin_setting(
    app: AppHandle,
    plugins: State<'_, Arc<Plugins>>,
    id: String,
    key: String,
    value: Value,
) -> AppResult<()> {
    logic::set_setting(&plugins, &id, &key, value)?;
    plugins.changed(&app);
    Ok(())
}

#[tauri::command(async)]
pub fn set_plugin_developer_mode(app: AppHandle, plugins: State<'_, Arc<Plugins>>, on: bool) -> AppResult<()> {
    logic::set_developer_mode(&plugins, on)?;
    plugins.changed(&app);
    Ok(())
}

#[tauri::command(async)]
pub fn load_dev_plugin(app: AppHandle, plugins: State<'_, Arc<Plugins>>, folder: String) -> AppResult<PluginInfo> {
    let info = photos_jobs::interactive(|| logic::load_dev(&plugins, Path::new(&folder)))?;
    plugins.changed(&app);
    Ok(info)
}

#[tauri::command(async)]
pub fn reload_dev_plugin(app: AppHandle, plugins: State<'_, Arc<Plugins>>, id: String) -> AppResult<PluginInfo> {
    let info = photos_jobs::interactive(|| logic::reload_dev(&plugins, &id))?;
    plugins.changed(&app);
    Ok(info)
}

/// The last 200 lines a plugin wrote to its log.
#[tauri::command(async)]
pub fn plugin_log(plugins: State<'_, Arc<Plugins>>, id: String) -> AppResult<Vec<String>> {
    Ok(logic::log(&plugins, &id))
}

/// The `.lumenplugin` this launch was opened with (once): an event sent during boot would fire
/// before the page listens, so the page asks.
#[tauri::command(async)]
pub fn take_pending_plugin_package(plugins: State<'_, Arc<Plugins>>) -> AppResult<Option<String>> {
    Ok(plugins.take_pending().map(|p| p.display().to_string()))
}

#[cfg(test)]
pub(crate) mod fixture {
    use std::path::{Path, PathBuf};

    fn crc32(data: &[u8]) -> u32 {
        let mut crc = !0u32;
        for b in data {
            crc ^= *b as u32;
            for _ in 0..8 {
                crc = if crc & 1 == 1 { (crc >> 1) ^ 0xEDB8_8320 } else { crc >> 1 };
            }
        }
        !crc
    }

    /// A stored (uncompressed) zip: the app crate has no zip writer and needs none otherwise.
    pub fn pack(path: &Path, entries: &[(&str, &[u8])]) {
        let mut out = Vec::new();
        let mut central = Vec::new();
        for (name, data) in entries {
            let offset = out.len() as u32;
            let crc = crc32(data);
            let head = |v: &mut Vec<u8>, local: bool| {
                v.extend_from_slice(&(if local { 0x0403_4b50u32 } else { 0x0201_4b50 }).to_le_bytes());
                if !local {
                    v.extend_from_slice(&20u16.to_le_bytes()); // made by
                }
                v.extend_from_slice(&20u16.to_le_bytes()); // needed
                v.extend_from_slice(&0u16.to_le_bytes()); // flags
                v.extend_from_slice(&0u16.to_le_bytes()); // stored
                v.extend_from_slice(&0u16.to_le_bytes()); // time
                v.extend_from_slice(&33u16.to_le_bytes()); // 1980-01-01
                v.extend_from_slice(&crc.to_le_bytes());
                v.extend_from_slice(&(data.len() as u32).to_le_bytes());
                v.extend_from_slice(&(data.len() as u32).to_le_bytes());
                v.extend_from_slice(&(name.len() as u16).to_le_bytes());
                v.extend_from_slice(&0u16.to_le_bytes()); // extra
            };
            head(&mut out, true);
            out.extend_from_slice(name.as_bytes());
            out.extend_from_slice(data);
            head(&mut central, false);
            central.extend_from_slice(&0u16.to_le_bytes()); // comment
            central.extend_from_slice(&0u16.to_le_bytes()); // disk
            central.extend_from_slice(&0u16.to_le_bytes()); // internal attrs
            central.extend_from_slice(&0u32.to_le_bytes()); // external attrs
            central.extend_from_slice(&offset.to_le_bytes());
            central.extend_from_slice(name.as_bytes());
        }
        let start = out.len() as u32;
        out.extend_from_slice(&central);
        out.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
        out.extend_from_slice(&[0, 0, 0, 0]);
        out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
        out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
        out.extend_from_slice(&(central.len() as u32).to_le_bytes());
        out.extend_from_slice(&start.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        std::fs::write(path, out).unwrap();
    }

    pub fn manifest(id: &str, extra: &str) -> String {
        format!(
            "id = \"{id}\"\nname = \"{id}\"\nversion = \"1.0.0\"\nauthor = \"Tester\"\ndescription = \"A test plugin.\"\nlicense = \"MIT\"\nlumen = \">=26.0.0\"\napi = 1\n{extra}\n"
        )
    }

    pub const SETTING: &str = "\n[[settings]]\nkey = \"loud\"\nlabel = \"Loud\"\ntype = \"bool\"\ndefault = false\n";

    /// A `.lumenplugin` with no kind sections (nothing for a kind check to claim).
    pub fn package(dir: &Path, id: &str, extra: &str) -> PathBuf {
        package_with(dir, id, extra, &[])
    }

    /// The same, with more files in the package.
    pub fn package_with(dir: &Path, id: &str, extra: &str, files: &[(&str, &[u8])]) -> PathBuf {
        let file = dir.join(format!("{id}.lumenplugin"));
        let m = manifest(id, extra);
        let mut entries: Vec<(&str, &[u8])> = vec![("plugin.toml", m.as_bytes())];
        entries.extend_from_slice(files);
        pack(&file, &entries);
        file
    }

    /// The first 24 bytes of a PNG of `w` x `h`: enough for the icon's size check.
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
    use std::path::Path;
    use std::sync::Arc;

    use crate::plugins::Plugins;

    use super::fixture::{manifest, package, SETTING};
    use super::logic;

    /// A store in developer mode, so unsigned test packages install.
    fn dev_plugins(app: &Path) -> Arc<Plugins> {
        let p = Arc::new(Plugins::load(app));
        logic::set_developer_mode(&p, true).unwrap();
        p
    }

    #[test]
    fn install_list_toggle_remove_undo() {
        let t = tempfile::tempdir().unwrap();
        let p = dev_plugins(t.path());
        let file = package(t.path(), "com.test.a", "");
        let info = logic::install(&p, &file).unwrap();
        assert_eq!(info.id, "com.test.a");
        assert_eq!(logic::list(&p).len(), 1);

        logic::set_enabled(&p, "com.test.a", false).unwrap();
        assert!(matches!(logic::list(&p)[0].state, photos_plugins::PluginState::Off));

        let token = logic::remove(&p, "com.test.a").unwrap();
        assert!(logic::list(&p).is_empty());
        logic::undo_remove(&p, &token).unwrap();
        let back = logic::list(&p);
        assert_eq!(back.len(), 1);
        assert!(!back[0].enabled, "undo restores the toggle too");
    }

    #[test]
    fn inspect_leaves_no_temp() {
        let t = tempfile::tempdir().unwrap();
        let p = dev_plugins(t.path());
        let file = package(t.path(), "com.test.a", "");
        let seen = logic::inspect(&p, &file).unwrap();
        assert_eq!(seen.info.id, "com.test.a");
        assert!(logic::list(&p).is_empty(), "inspect installs nothing");
        let left: Vec<_> = std::fs::read_dir(t.path().join("plugins/installed"))
            .map(|rd| rd.flatten().map(|e| e.file_name()).collect())
            .unwrap_or_default();
        assert!(left.is_empty(), "left behind: {left:?}");
    }

    #[test]
    fn concurrent_installs_serialised() {
        let t = tempfile::tempdir().unwrap();
        let p = dev_plugins(t.path());
        let a = package(t.path(), "com.test.a", "");
        let b = package(t.path(), "com.test.b", "");
        let (pa, pb) = (p.clone(), p.clone());
        let ha = std::thread::spawn(move || logic::install(&pa, &a));
        let hb = std::thread::spawn(move || logic::install(&pb, &b));
        ha.join().unwrap().unwrap();
        hb.join().unwrap().unwrap();
        assert_eq!(logic::list(&p).len(), 2);
        let saved = std::fs::read_to_string(t.path().join("plugins/plugins.json")).unwrap();
        assert!(saved.contains("com.test.a") && saved.contains("com.test.b"), "{saved}");
    }

    #[test]
    fn undo_after_sweep_errors() {
        let t = tempfile::tempdir().unwrap();
        let p = dev_plugins(t.path());
        logic::install(&p, &package(t.path(), "com.test.a", "")).unwrap();
        let token = logic::remove(&p, "com.test.a").unwrap();
        photos_plugins::store::Store::sweep_removed(&t.path().join("plugins"));
        let e = logic::undo_remove(&p, &token).unwrap_err();
        assert_eq!(e.code, "plugin_invalid");
        assert_eq!(e.message, "That plugin can't be restored any more");
        assert!(logic::list(&p).is_empty());
        // A token this session never made says the same.
        assert_eq!(logic::undo_remove(&p, "nope").unwrap_err().message, "That plugin can't be restored any more");
    }

    #[test]
    fn developer_mode_and_dev_folder() {
        let t = tempfile::tempdir().unwrap();
        let p = Arc::new(Plugins::load(t.path()));
        let folder = t.path().join("my-plugin");
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join("plugin.toml"), manifest("com.test.dev", "")).unwrap();

        // Off: a folder doesn't load.
        assert_eq!(logic::load_dev(&p, &folder).unwrap_err().code, "plugin_unsigned");

        assert!(!logic::developer_mode(&p));
        logic::set_developer_mode(&p, true).unwrap();
        assert!(logic::developer_mode(&p));
        let info = logic::load_dev(&p, &folder).unwrap();
        assert!(info.dev);
        assert_eq!(logic::list(&p).len(), 1);

        std::fs::write(folder.join("plugin.toml"), manifest("com.test.dev", "").replace("A test plugin.", "Edited.")).unwrap();
        assert_eq!(logic::reload_dev(&p, "com.test.dev").unwrap().description, "Edited.");

        logic::set_developer_mode(&p, false).unwrap();
        assert!(logic::list(&p).is_empty(), "dev folders leave the list with the mode");
    }

    #[test]
    fn setting_validated() {
        let t = tempfile::tempdir().unwrap();
        let p = dev_plugins(t.path());
        logic::install(&p, &package(t.path(), "com.test.a", SETTING)).unwrap();
        let e = logic::set_setting(&p, "com.test.a", "loud", serde_json::json!("yes")).unwrap_err();
        assert_eq!(e.code, "plugin_invalid");
        logic::set_setting(&p, "com.test.a", "loud", serde_json::json!(true)).unwrap();
        assert_eq!(p.read().setting("com.test.a", "loud"), Some(serde_json::json!(true)));
        assert_eq!(logic::set_setting(&p, "com.test.a", "nope", serde_json::json!(1)).unwrap_err().code, "plugin_invalid");
    }

    #[test]
    fn log_lines_come_from_the_ring() {
        let t = tempfile::tempdir().unwrap();
        let p = dev_plugins(t.path());
        p.log_sink("com.test.a")("hello");
        assert_eq!(logic::log(&p, "com.test.a"), vec!["hello".to_string()]);
    }

    #[test]
    fn icon_url_is_the_os_form() {
        assert_eq!(super::icon_url_for(true, "com.test.a", "1.2.0"), "http://photos.localhost/plugin-icon/com.test.a?v=1.2.0");
        assert_eq!(super::icon_url_for(false, "com.test.a", "1.2.0"), "photos://localhost/plugin-icon/com.test.a?v=1.2.0");
    }

    #[test]
    fn list_and_install_fill_the_icon_url() {
        let t = tempfile::tempdir().unwrap();
        let p = dev_plugins(t.path());
        let icon = super::fixture::png(64, 64);
        let file = super::fixture::package_with(t.path(), "com.test.a", "", &[("icon.png", &icon)]);
        let installed = logic::install(&p, &file).unwrap();
        let want = super::icon_url_for(cfg!(windows), "com.test.a", "1.0.0");
        assert_eq!(installed.icon_url.as_deref(), Some(want.as_str()));
        assert_eq!(logic::list(&p)[0].icon_url.as_deref(), Some(want.as_str()));
        logic::install(&p, &package(t.path(), "com.test.b", "")).unwrap();
        assert!(logic::list(&p).iter().any(|i| i.id == "com.test.b" && i.icon_url.is_none()));
    }
}
