//! Devices: memory cards (volumes with DCIM), the library's device sections, and
//! (Task C4) iPhones.

use std::collections::HashSet;
use std::path::PathBuf;

use photos_core::devices::{self, DeviceSummary};
use photos_core::seen::{self, SeenItem};
use photos_import::{iphone, FilteredSource, ImportSource, IphoneSource, SourceItem, VolumeSource};
use tauri::{AppHandle, State};

use crate::commands::import::spawn_import;
use crate::connected::{self, ConnectedDevice};
use crate::error::{AppError, AppResult};
use crate::events::{IphonesUpdate, VolumeInfo};
use crate::state::AppState;
use crate::iphones;
use crate::volumes;

/// Current DCIM volumes with their new-item counts (the watcher keeps this fresh).
#[tauri::command(async)]
pub fn list_volumes() -> AppResult<Vec<VolumeInfo>> {
    Ok(volumes::snapshot())
}

/// The card at `mount` and what on it is not in the library yet.
pub fn volume_new_items(state: &AppState, mount: &str) -> AppResult<(VolumeSource, Vec<SourceItem>)> {
    let mount_path = PathBuf::from(mount);
    let info = volumes::snapshot().into_iter().find(|v| v.mount == mount).ok_or_else(|| AppError {
        code: "volume_gone".into(),
        message: "That card is no longer connected.".into(),
        detail: Some(mount.into()),
    })?;
    let total_space: u64 = info.key.rsplit(':').next().and_then(|s| s.parse().ok()).unwrap_or(0);
    let source = VolumeSource::detect(&mount_path, &info.label, total_space).ok_or_else(|| AppError {
        code: "no_dcim".into(),
        message: "No DCIM folder on that volume.".into(),
        detail: Some(mount.into()),
    })?;
    let lib = state.library()?;
    let items = source.enumerate().map_err(|e| AppError {
        code: "volume_unreadable".into(),
        message: "The card could not be read.".into(),
        detail: Some(e.to_string()),
    })?;
    let fresh = unseen_only(&lib, source.key(), items)?;
    Ok((source, fresh))
}

/// `items` less what `key` has already brought into the library.
fn unseen_only(lib: &photos_core::Library, key: &str, items: Vec<SourceItem>) -> AppResult<Vec<SourceItem>> {
    let seen_items: Vec<SeenItem<'_>> =
        items.iter().map(|i| SeenItem { rel_path: &i.rel_path, size: i.size, mtime: i.modified_at }).collect();
    let unseen: HashSet<usize> = lib.db.with_conn(|c| seen::unseen_indexes(c, key, &seen_items))?.into_iter().collect();
    Ok(items.into_iter().enumerate().filter(|(i, _)| unseen.contains(i)).map(|(_, it)| it).collect())
}

/// Import what is new on the card at `mount` (with `limit_bytes`, the newest that fit).
#[tauri::command(async)]
pub fn start_volume_import(app: AppHandle, state: State<'_, AppState>, mount: String, limit_bytes: Option<u64>) -> AppResult<i64> {
    let (source, fresh) = volume_new_items(&state, &mount)?;
    let key = source.key().to_string();
    let filtered = FilteredSource::new(source, crate::commands::import::choose(&fresh, limit_bytes));
    let after_mount = PathBuf::from(&mount);
    // Each file is remembered as seen when it lands (ingest), never merely because it was
    // listed: a file that failed, or that an unplugged card never sent, is offered again.
    spawn_import(
        &app,
        &state,
        Box::new(filtered),
        Some(mount),
        Some(key),
        Some(Box::new(move |_summary, _src| volumes::mark_dirty(&after_mount))),
    )
}

/// *Eject* (Round 4G): the OS lets the card or drive at `mount` go. Refused for the drive the
/// open library is on and while an import is running; nothing is written to the device.
#[tauri::command(async)]
pub fn eject_volume(state: State<'_, AppState>, mount: String) -> AppResult<()> {
    if !volumes::snapshot().iter().any(|v| v.mount == mount) {
        return Err(AppError { code: "volume_gone".into(), message: "That card is no longer connected.".into(), detail: Some(mount) });
    }
    if let Ok(lib) = state.library() {
        if crate::eject::holds_library(&PathBuf::from(&mount), &lib.root) {
            return Err(AppError { code: "holds_library".into(), message: "The library is on that drive; close it before ejecting.".into(), detail: Some(mount) });
        }
    }
    if !state.imports.lock().expect("imports lock").is_empty() {
        return Err(AppError { code: "importing".into(), message: "Wait for the import to finish, or cancel it, before ejecting.".into(), detail: Some(mount) });
    }
    crate::eject::eject(&mount)
}

/// Forget what was imported from a card, so everything on it counts as new again.
#[tauri::command(async)]
pub fn forget_volume(state: State<'_, AppState>, key: String) -> AppResult<usize> {
    let lib = state.library()?;
    let n = lib.db.with_conn(|c| seen::forget_source(c, &key))?;
    for v in volumes::snapshot() {
        if v.key == key {
            volumes::mark_dirty(&PathBuf::from(v.mount));
        }
    }
    Ok(n)
}

/// The connected-device page: name, storage, what is new, and tokens for the newest
/// previews. `kind` is `volume` (id = mount) or `iphone` (id = UDID). Listing a phone's
/// camera roll takes a while over USB, so this runs off the main thread.
#[tauri::command]
pub async fn get_connected_device(app: AppHandle, kind: String, id: String) -> AppResult<ConnectedDevice> {
    tauri::async_runtime::spawn_blocking(move || connected::describe(&app, &kind, &id))
        .await
        .map_err(|e| AppError { code: "internal".into(), message: "Could not read the device.".into(), detail: Some(e.to_string()) })?
}

/// iPhones usbmuxd can see: the always-running watcher's last look (it looks every 2 s).
#[tauri::command(async)]
pub fn list_iphones() -> AppResult<IphonesUpdate> {
    Ok(iphones::current())
}

/// The phone at `udid` and what on it is not in the library yet.
pub fn iphone_new_items(state: &AppState, udid: &str) -> AppResult<(IphoneSource, Vec<SourceItem>)> {
    let found = iphone::list_iphones()?;
    let info = found.into_iter().find(|d| d.udid == udid).ok_or_else(|| AppError {
        code: "iphone_gone".into(),
        message: "That iPhone is no longer connected.".into(),
        detail: Some(udid.into()),
    })?;
    if !info.paired {
        return Err(photos_import::IphoneError::TrustRequired.into());
    }
    let lib = state.library()?;
    let source = IphoneSource::new(&info.udid, &info.name);
    let key = source.key();
    let items = source.enumerate().map_err(|e| AppError {
        code: "iphone".into(),
        message: "The iPhone's photo library could not be read.".into(),
        detail: Some(e.to_string()),
    })?;
    let fresh = unseen_only(&lib, &key, items)?;
    Ok((source, fresh))
}

/// Import what is new on the phone at `udid` (with `limit_bytes`, the newest that fit).
#[tauri::command(async)]
pub fn start_iphone_import(app: AppHandle, state: State<'_, AppState>, udid: String, limit_bytes: Option<u64>) -> AppResult<i64> {
    let (source, fresh) = iphone_new_items(&state, &udid)?;
    if fresh.is_empty() {
        return Err(AppError { code: "nothing_new".into(), message: "Everything on this iPhone is already imported.".into(), detail: None });
    }
    let key = source.key();
    let filtered = FilteredSource::new(source, crate::commands::import::choose(&fresh, limit_bytes));
    spawn_import(&app, &state, Box::new(filtered), None, Some(key), None)
}

#[tauri::command(async)]
pub fn list_devices(state: State<'_, AppState>) -> AppResult<Vec<DeviceSummary>> {
    let lib = state.library()?;
    Ok(lib.db.with_conn(devices::list_devices)?)
}

#[tauri::command(async)]
pub fn rename_device(app: AppHandle, state: State<'_, AppState>, id: i64, name: String) -> AppResult<()> {
    let lib = state.library()?;
    lib.db.with_conn(|c| devices::rename_device(c, id, &name))?;
    state.changes.changed(&app, true);
    Ok(())
}

#[tauri::command(async)]
pub fn set_device_hidden(app: AppHandle, state: State<'_, AppState>, id: i64, hidden: bool) -> AppResult<()> {
    let lib = state.library()?;
    lib.db.with_conn(|c| devices::set_device_hidden(c, id, hidden))?;
    state.changes.changed(&app, true);
    Ok(())
}

