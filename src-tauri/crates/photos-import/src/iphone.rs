//! iPhone and iPad over USB through usbmuxd and AFC (architecture spec §6.2).
//!
//! `idevice` is async; the ingest pipeline is not. Every call here owns a current-thread
//! tokio runtime, and the AFC session lives behind a mutex so the rayon staging pool
//! shares one USB connection instead of opening one per file. Tauri runs `async` commands
//! on its own tokio workers, where `block_on` panics, so each entry point goes through
//! [`off_runtime`].

use std::io::{Read, Seek, SeekFrom, Write};
use std::sync::Mutex;

use idevice::afc::AfcClient;
use idevice::lockdown::LockdownClient;
use idevice::provider::UsbmuxdProvider;
use idevice::usbmuxd::{Connection, UsbmuxdAddr, UsbmuxdConnection, UsbmuxdDevice};
use idevice::{IdeviceError, IdeviceService};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::source::{ImportSource, SourceItem};
use crate::ImportError;

/// Label sent to lockdown; shows up in the device's pairing records.
const LABEL: &str = "Lumen";
/// AFC reads are chunked so a 4 GB video never lands in memory.
const CHUNK: usize = 1024 * 1024;
/// Only this tree is read; Lumen never writes to the phone.
const DCIM: &str = "/DCIM";

/// A connected iPhone or iPad.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct IphoneInfo {
    pub udid: String,
    pub name: String,
    pub model: Option<String>,
    /// False until the phone is unlocked and *Trust* has been tapped.
    pub paired: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum IphoneError {
    /// No usbmuxd: iTunes / Apple Devices on Windows, usbmuxd on Linux, built in on macOS.
    #[error("usbmuxd is not available")]
    UsbmuxdUnavailable,
    #[error("the device is not paired with this computer")]
    NotPaired,
    #[error("unlock the phone and tap Trust")]
    TrustRequired,
    #[error("{0}")]
    Device(String),
}

impl IphoneError {
    /// Stable `AppError.code` for the frontend.
    pub fn code(&self) -> &'static str {
        match self {
            IphoneError::UsbmuxdUnavailable => "usbmuxd_missing",
            IphoneError::NotPaired => "iphone_not_paired",
            IphoneError::TrustRequired => "iphone_trust_required",
            IphoneError::Device(_) => "iphone",
        }
    }
}

/// Maps the library's errors onto the four states the UI can act on.
pub fn classify(e: IdeviceError) -> IphoneError {
    match e {
        IdeviceError::InvalidHostID | IdeviceError::PairingDialogResponsePending => IphoneError::TrustRequired,
        IdeviceError::UserDeniedPairing => IphoneError::NotPaired,
        IdeviceError::DeviceNotFound | IdeviceError::NoEstablishedConnection => IphoneError::Device("the phone disconnected".into()),
        IdeviceError::DeviceLocked => IphoneError::Device("unlock the phone and try again".into()),
        other => IphoneError::Device(other.to_string()),
    }
}

fn runtime() -> Result<tokio::runtime::Runtime, IphoneError> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| IphoneError::Device(e.to_string()))
}

/// Runs `f` where blocking on a runtime is allowed: here, or on a scoped thread when the
/// caller is itself driving a tokio runtime (a Tauri command).
fn off_runtime<T: Send>(f: impl FnOnce() -> T + Send) -> T {
    if tokio::runtime::Handle::try_current().is_err() {
        return f();
    }
    std::thread::scope(|s| s.spawn(f).join().unwrap_or_else(|p| std::panic::resume_unwind(p)))
}

/// Every device usbmuxd knows about, with its name and pairing state.
///
/// An unreachable usbmuxd is [`IphoneError::UsbmuxdUnavailable`], never an empty list:
/// "no phone plugged in" and "no Apple drivers installed" need different UI.
pub fn list_iphones() -> Result<Vec<IphoneInfo>, IphoneError> {
    off_runtime(list_iphones_blocking)
}

/// usbmuxd lists a phone once per connection (USB, Wi-Fi sync) under one UDID. Keep one entry per
/// UDID, in first-seen order, preferring the USB connection.
fn unique_phones(devices: Vec<UsbmuxdDevice>) -> Vec<UsbmuxdDevice> {
    let mut out: Vec<UsbmuxdDevice> = Vec::with_capacity(devices.len());
    for d in devices {
        match out.iter().position(|o| o.udid == d.udid) {
            Some(i) => {
                if matches!(d.connection_type, Connection::Usb) && !matches!(out[i].connection_type, Connection::Usb) {
                    out[i] = d;
                }
            }
            None => out.push(d),
        }
    }
    out
}

fn list_iphones_blocking() -> Result<Vec<IphoneInfo>, IphoneError> {
    let rt = runtime()?;
    rt.block_on(async {
        let mut mux = UsbmuxdConnection::default().await.map_err(|_| IphoneError::UsbmuxdUnavailable)?;
        let devices = unique_phones(mux.get_devices().await.map_err(|_| IphoneError::UsbmuxdUnavailable)?);
        let mut out = Vec::with_capacity(devices.len());
        for d in devices {
            // A pair record is what lets us open a session at all.
            let paired = mux.get_pair_record(&d.udid).await.is_ok();
            let (name, model) = if paired { device_names(&d.udid, d.device_id).await } else { (None, None) };
            out.push(IphoneInfo {
                name: name.unwrap_or_else(|| model.clone().unwrap_or_else(|| "iPhone".into())),
                model,
                udid: d.udid,
                paired,
            });
        }
        Ok(out)
    })
}

/// Best-effort `DeviceName` and `ProductType` over lockdown; `None` when the phone is
/// locked or refuses the session.
async fn device_names(udid: &str, device_id: u32) -> (Option<String>, Option<String>) {
    let provider = UsbmuxdProvider { addr: UsbmuxdAddr::default(), tag: device_id, udid: udid.to_string(), device_id, label: LABEL.to_string() };
    let Ok(mut lockdown) = LockdownClient::connect(&provider).await else { return (None, None) };
    let name = lockdown.get_value(Some("DeviceName"), None).await.ok().and_then(|v| v.as_string().map(str::to_string));
    let model = lockdown.get_value(Some("ProductType"), None).await.ok().and_then(|v| v.as_string().map(str::to_string));
    (name, model)
}

/// Phones already named over lockdown: udid → (name, model).
pub type IphoneNames = std::collections::HashMap<String, (String, Option<String>)>;

/// [`list_iphones`] for the device watcher, which looks every few seconds whether or not a
/// screen shows devices (Round 5A: a phone plugged in did not appear until Import was opened).
/// Asking usbmuxd is a local round trip; opening a lockdown session for the name is the costly
/// part, so a paired phone already in `named` is not asked again, and one that answers is
/// added. A phone that could not be named (locked, not yet trusted) is asked next time.
/// Blocks on its own runtime: call it from a plain thread.
pub fn list_iphones_named(named: &mut IphoneNames) -> Result<Vec<IphoneInfo>, IphoneError> {
    let rt = runtime()?;
    rt.block_on(async {
        let mut mux = UsbmuxdConnection::default().await.map_err(|_| IphoneError::UsbmuxdUnavailable)?;
        let devices = unique_phones(mux.get_devices().await.map_err(|_| IphoneError::UsbmuxdUnavailable)?);
        let mut out = Vec::with_capacity(devices.len());
        for d in devices {
            let paired = mux.get_pair_record(&d.udid).await.is_ok();
            let (name, model) = match named.get(&d.udid) {
                Some((name, model)) if paired => (Some(name.clone()), model.clone()),
                _ if paired => {
                    let (name, model) = device_names(&d.udid, d.device_id).await;
                    if let Some(n) = &name {
                        named.insert(d.udid.clone(), (n.clone(), model.clone()));
                    }
                    (name, model)
                }
                _ => (None, None),
            };
            out.push(IphoneInfo {
                name: name.unwrap_or_else(|| model.clone().unwrap_or_else(|| "iPhone".into())),
                model,
                udid: d.udid,
                paired,
            });
        }
        Ok(out)
    })
}

/// One phone as an import source. `enumerate` walks `/DCIM`; `open` streams a file into a
/// temporary file, because AFC handles cannot travel between the staging threads.
pub struct IphoneSource {
    udid: String,
    name: String,
    session: Mutex<Option<Session>>,
}

struct Session {
    rt: tokio::runtime::Runtime,
    afc: AfcClient,
}

impl std::fmt::Debug for IphoneSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IphoneSource").field("udid", &self.udid).field("name", &self.name).finish()
    }
}

impl IphoneSource {
    pub fn new(udid: &str, name: &str) -> Self {
        Self { udid: udid.to_string(), name: name.to_string(), session: Mutex::new(None) }
    }

    pub fn udid(&self) -> &str {
        &self.udid
    }

    /// `import_seen.source_key` for this phone.
    pub fn key(&self) -> String {
        format!("ios:{}", self.udid)
    }

    /// `(capacity, free)` bytes as the phone's Settings shows them: the capacity it was sold
    /// with and the space free now. Space iOS *could* purge (iCloud-optimised originals,
    /// caches) is not free until iOS frees it. Falls back to AFC, whose figures describe
    /// only the data filesystem (a 128 GB phone reports ~110 GiB).
    pub fn storage(&self) -> Result<(u64, u64), IphoneError> {
        match self.disk_usage() {
            Ok(Some(s)) => return Ok(s),
            Ok(None) => tracing::debug!("disk_usage had no capacity; using AFC"),
            Err(e) => tracing::debug!("disk_usage unavailable ({e}); using AFC"),
        }
        self.with_afc(|rt, afc| rt.block_on(afc.get_device_info()).map(|i| (i.total_bytes as u64, i.free_bytes as u64)))
    }

    /// Lockdown's `com.apple.disk_usage` domain, which needs a paired session.
    fn disk_usage(&self) -> Result<Option<(u64, u64)>, IphoneError> {
        off_runtime(|| self.disk_usage_blocking())
    }

    fn disk_usage_blocking(&self) -> Result<Option<(u64, u64)>, IphoneError> {
        let rt = runtime()?;
        let udid = self.udid.clone();
        rt.block_on(async move {
            let mut mux = UsbmuxdConnection::default().await.map_err(|_| IphoneError::UsbmuxdUnavailable)?;
            let device = mux.get_device(&udid).await.map_err(|_| IphoneError::Device("the phone disconnected".into()))?;
            let pair = mux.get_pair_record(&udid).await.map_err(classify)?;
            let provider = device.to_provider(UsbmuxdAddr::default(), LABEL);
            let mut lockdown = LockdownClient::connect(&provider).await.map_err(classify)?;
            lockdown.start_session(&pair).await.map_err(classify)?;
            let usage = lockdown.get_value(None, Some("com.apple.disk_usage")).await.map_err(classify)?;
            Ok(usage.as_dictionary().and_then(storage_from_disk_usage))
        })
    }

    /// Runs `f` with the shared AFC session, opening it on first use. Serialised: one USB
    /// conversation at a time, which is what the bus does anyway.
    fn with_afc<T: Send>(&self, f: impl FnOnce(&tokio::runtime::Runtime, &mut AfcClient) -> Result<T, IdeviceError> + Send) -> Result<T, IphoneError> {
        off_runtime(|| self.with_afc_blocking(f))
    }

    fn with_afc_blocking<T>(&self, f: impl FnOnce(&tokio::runtime::Runtime, &mut AfcClient) -> Result<T, IdeviceError>) -> Result<T, IphoneError> {
        let mut guard = self.session.lock().expect("afc session lock");
        if guard.is_none() {
            let rt = runtime()?;
            let udid = self.udid.clone();
            let afc = rt.block_on(async move {
                let mut mux = UsbmuxdConnection::default().await.map_err(|_| IphoneError::UsbmuxdUnavailable)?;
                let device = mux.get_device(&udid).await.map_err(|_| IphoneError::Device("the phone disconnected".into()))?;
                let provider = device.to_provider(UsbmuxdAddr::default(), LABEL);
                AfcClient::connect(&provider).await.map_err(classify)
            })?;
            *guard = Some(Session { rt, afc });
        }
        let s = guard.as_mut().expect("session present");
        let result = f(&s.rt, &mut s.afc);
        if let Err(e) = &result {
            // A broken pipe stays broken: drop the session so the next call reconnects.
            if matches!(e, IdeviceError::Socket(_) | IdeviceError::NoEstablishedConnection) {
                *guard = None;
            }
        }
        result.map_err(classify)
    }
}

/// `(capacity, free)` from a `com.apple.disk_usage` reply. `AmountDataAvailable` is what is
/// free now; `TotalDataAvailable` adds what iOS could purge, which on a phone using iCloud
/// Photos read ~70 GB where Settings said 2 GB, so it is only a fallback.
pub fn storage_from_disk_usage(usage: &plist::Dictionary) -> Option<(u64, u64)> {
    let get = |k: &str| usage.get(k).and_then(plist::Value::as_unsigned_integer);
    let capacity = get("TotalDiskCapacity")?;
    let free = get("AmountDataAvailable").or_else(|| get("TotalDataAvailable"))?;
    Some((capacity, free.min(capacity)))
}

/// `/DCIM/100APPLE/IMG_0001.HEIC` → `100APPLE/IMG_0001.HEIC`.
pub fn rel_path(full: &str) -> String {
    full.trim_start_matches(DCIM).trim_start_matches('/').to_string()
}

/// AFC listings include `.` and `..`; everything else is a real entry.
pub fn is_real_entry(name: &str) -> bool {
    name != "." && name != ".." && !name.is_empty()
}

fn join(dir: &str, name: &str) -> String {
    format!("{}/{}", dir.trim_end_matches('/'), name)
}

impl ImportSource for IphoneSource {
    fn label(&self) -> String {
        self.name.clone()
    }

    fn kind(&self) -> i64 {
        photos_core::batches::SOURCE_IPHONE
    }

    fn enumerate(&self) -> Result<Vec<SourceItem>, ImportError> {
        let items = self
            .with_afc(|rt, afc| {
                rt.block_on(async {
                    let mut out: Vec<SourceItem> = Vec::new();
                    let mut stack = vec![DCIM.to_string()];
                    while let Some(dir) = stack.pop() {
                        let names = match afc.list_dir(dir.clone()).await {
                            Ok(n) => n,
                            // A vanished album is not a reason to abandon the import.
                            Err(IdeviceError::NotFound) => continue,
                            Err(e) => return Err(e),
                        };
                        for name in names.into_iter().filter(|n| is_real_entry(n)) {
                            let path = join(&dir, &name);
                            let Ok(info) = afc.get_file_info(path.clone()).await else { continue };
                            if info.st_ifmt == "S_IFDIR" {
                                stack.push(path);
                                continue;
                            }
                            if !photos_media::is_supported(std::path::Path::new(&name)) {
                                continue;
                            }
                            out.push(SourceItem {
                                rel_path: rel_path(&path),
                                file_name: name,
                                size: info.size as u64,
                                modified_at: Some(info.modified.and_utc().timestamp()),
                                created_at: None,
                            });
                        }
                    }
                    out.sort_by(|a, b| a.rel_path.cmp(&b.rel_path));
                    Ok(out)
                })
            })
            .map_err(|e| ImportError::Source(e.to_string()))?;
        Ok(items)
    }

    fn open(&self, item: &SourceItem) -> Result<Box<dyn Read + Send>, ImportError> {
        let path = format!("{DCIM}/{}", item.rel_path);
        let mut tmp = tempfile::tempfile()?;
        self.with_afc(|rt, afc| {
            rt.block_on(async {
                let mut fd = afc.open(path.clone(), idevice::afc::opcode::AfcFopenMode::RdOnly).await?;
                loop {
                    let chunk = fd.read_n(CHUNK).await?;
                    if chunk.is_empty() {
                        break;
                    }
                    let n = chunk.len();
                    tmp.write_all(&chunk).map_err(IdeviceError::Socket)?;
                    if n < CHUNK {
                        break;
                    }
                }
                fd.close().await?;
                Ok(())
            })
        })
        .map_err(|e| ImportError::Source(e.to_string()))?;
        tmp.seek(SeekFrom::Start(0))?;
        Ok(Box::new(tmp))
    }

    /// Still plugged in: usbmuxd lists it.
    fn is_connected(&self) -> bool {
        list_iphones().is_ok_and(|all| all.iter().any(|d| d.udid == self.udid))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dict(pairs: &[(&str, u64)]) -> plist::Dictionary {
        pairs.iter().map(|(k, v)| (k.to_string(), plist::Value::Integer((*v).into()))).collect()
    }

    #[test]
    fn storage_is_the_phone_as_sold_and_what_is_free_now() {
        // A 128 GB iPhone as `ideviceinfo -q com.apple.disk_usage` reports it. Settings said
        // about 2 GB free on Julian's phone while the purgeable-inclusive figure read ~70 GB.
        let usage = dict(&[
            ("TotalDiskCapacity", 128_000_000_000),
            ("TotalDataCapacity", 118_000_000_000),
            ("TotalDataAvailable", 70_400_000_000),
            ("AmountDataAvailable", 2_100_000_000),
        ]);
        assert_eq!(storage_from_disk_usage(&usage), Some((128_000_000_000, 2_100_000_000)));
        // A reply without AmountDataAvailable falls back to the purgeable-inclusive figure.
        let usage = dict(&[("TotalDiskCapacity", 64_000_000_000), ("TotalDataAvailable", 3_000_000_000)]);
        assert_eq!(storage_from_disk_usage(&usage), Some((64_000_000_000, 3_000_000_000)));
        assert_eq!(storage_from_disk_usage(&dict(&[("TotalDataAvailable", 1)])), None);
    }

    #[test]
    fn one_phone_on_usb_and_wifi_is_listed_once_over_usb() {
        let dev = |connection_type, device_id| UsbmuxdDevice { connection_type, udid: "00008120-A".into(), device_id };
        let ip = std::net::IpAddr::from([192, 168, 1, 5]);
        let got = unique_phones(vec![dev(Connection::Network(ip), 1), dev(Connection::Usb, 2)]);
        assert_eq!(got.len(), 1);
        assert!(matches!(got[0].connection_type, Connection::Usb));
        assert_eq!(got[0].device_id, 2);
    }

    #[test]
    fn listing_from_inside_an_async_runtime_does_not_panic() {
        // Tauri drives `#[tauri::command(async)]` bodies on tokio workers.
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let outcome = rt.block_on(async { std::panic::catch_unwind(|| list_iphones().map(|_| ())) });
        assert!(outcome.is_ok(), "list_iphones panicked inside a runtime");
    }

    #[test]
    fn maps_afc_paths_to_relative_paths() {
        assert_eq!(rel_path("/DCIM/100APPLE/IMG_0001.HEIC"), "100APPLE/IMG_0001.HEIC");
        assert_eq!(rel_path("/DCIM/IMG_0002.MOV"), "IMG_0002.MOV");
        assert_eq!(rel_path("100APPLE/IMG_0003.HEIC"), "100APPLE/IMG_0003.HEIC");
        assert_eq!(join("/DCIM", "100APPLE"), "/DCIM/100APPLE");
        assert_eq!(join("/DCIM/", "100APPLE"), "/DCIM/100APPLE");
        assert!(!is_real_entry("."));
        assert!(!is_real_entry(".."));
        assert!(is_real_entry("100APPLE"));
    }

    #[test]
    fn source_key_and_error_codes_are_stable() {
        let s = IphoneSource::new("00008120-001", "Julian's iPhone");
        assert_eq!(s.key(), "ios:00008120-001");
        assert_eq!(s.label(), "Julian's iPhone");
        assert_eq!(s.kind(), photos_core::batches::SOURCE_IPHONE);
        assert_eq!(IphoneError::UsbmuxdUnavailable.code(), "usbmuxd_missing");
        assert_eq!(IphoneError::NotPaired.code(), "iphone_not_paired");
        assert_eq!(IphoneError::TrustRequired.code(), "iphone_trust_required");
        assert_eq!(IphoneError::Device("x".into()).code(), "iphone");
        assert!(matches!(classify(IdeviceError::InvalidHostID), IphoneError::TrustRequired));
    }
}
