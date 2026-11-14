//! iPhone detection. The watcher always runs, so a phone plugged in appears in the sidebar
//! at once (Round 5A: polling only while Import was open meant it appeared only there). Each
//! look is a local usbmuxd round trip; a phone's name is asked over lockdown once
//! (`iphone::list_iphones_named`).

use std::sync::Mutex;
use std::time::Duration;

use photos_import::iphone::{self, IphoneNames};
use photos_import::IphoneError;
use tauri::{AppHandle, Emitter};

use crate::events::{IphonesUpdate, DEVICE_IPHONES};

const TICK: Duration = Duration::from_secs(2);

static SNAPSHOT: Mutex<Option<IphonesUpdate>> = Mutex::new(None);

fn update_from(found: Result<Vec<iphone::IphoneInfo>, IphoneError>) -> IphonesUpdate {
    match found {
        Ok(devices) => IphonesUpdate { usbmuxd_available: true, devices },
        Err(IphoneError::UsbmuxdUnavailable) => IphonesUpdate { usbmuxd_available: false, devices: Vec::new() },
        Err(e) => {
            tracing::warn!("iphone poll failed: {e}");
            IphonesUpdate { usbmuxd_available: true, devices: Vec::new() }
        }
    }
}

/// What the watcher saw last, or a fresh look before its first.
pub fn current() -> IphonesUpdate {
    if let Some(seen) = SNAPSHOT.lock().expect("iphones lock").clone() {
        return seen;
    }
    update_from(iphone::list_iphones())
}

/// Publishes `device://iphones` whenever the answer changes.
pub fn start(app: AppHandle) {
    std::thread::Builder::new()
        .name("lumen-iphones".into())
        .spawn(move || {
            let mut names = IphoneNames::new();
            loop {
                let update = update_from(iphone::list_iphones_named(&mut names));
                let mut guard = SNAPSHOT.lock().expect("iphones lock");
                if guard.as_ref() != Some(&update) {
                    *guard = Some(update.clone());
                    drop(guard);
                    let _ = app.emit(DEVICE_IPHONES, &update);
                }
                std::thread::sleep(TICK);
            }
        })
        .expect("spawn iphone watcher");
}
