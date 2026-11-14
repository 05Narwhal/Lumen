//! Throwaway probe (round-2 plan Task C4, step 1): does usbmuxd + AFC work on this
//! machine with a real iPhone? Ignored by default; run it with the phone plugged in:
//!
//! ```sh
//! cargo test -p photos-import --test iphone_spike -- --ignored --nocapture
//! ```
//!
//! It prints what it finds and never asserts on hardware being present, so the outcome
//! is a readable report rather than a failure.

use std::time::Instant;

use photos_import::iphone;

#[test]
#[ignore = "needs an iPhone plugged in and trusted"]
fn spike_list_devices_and_pull_one_file() {
    match iphone::list_iphones() {
        Err(e) => println!("list_iphones failed: {e:?}"),
        Ok(devices) if devices.is_empty() => println!("usbmuxd reachable, no devices connected"),
        Ok(devices) => {
            for d in &devices {
                println!("device: udid={} name={:?} model={:?} paired={}", d.udid, d.name, d.model, d.paired);
            }
            let d = &devices[0];
            if !d.paired {
                println!("not paired: tap Trust on the phone, then re-run");
                return;
            }
            let source = iphone::IphoneSource::new(&d.udid, &d.name);
            let t = Instant::now();
            match photos_import::ImportSource::enumerate(&source) {
                Err(e) => println!("enumerate failed: {e}"),
                Ok(items) => {
                    println!("enumerated {} items in {:?}", items.len(), t.elapsed());
                    for i in items.iter().take(5) {
                        println!("  {} ({} bytes, mtime {:?})", i.rel_path, i.size, i.modified_at);
                    }
                    if let Some(first) = items.first() {
                        let t = Instant::now();
                        match photos_import::ImportSource::open(&source, first) {
                            Err(e) => println!("open failed: {e}"),
                            Ok(mut r) => {
                                let mut buf = Vec::new();
                                let n = std::io::Read::read_to_end(&mut r, &mut buf);
                                println!("pulled {} -> {:?} bytes in {:?}", first.rel_path, n.map(|_| buf.len()), t.elapsed());
                            }
                        }
                    }
                }
            }
        }
    }
}

/// What the phone says about its storage, from both places that can say it: AFC's device
/// info (per filesystem) and lockdown's `com.apple.disk_usage` domain (what Finder shows).
#[test]
#[ignore = "needs an iPhone plugged in and trusted"]
fn spike_storage_numbers() {
    use idevice::lockdown::LockdownClient;
    use idevice::usbmuxd::{UsbmuxdAddr, UsbmuxdConnection};
    use idevice::{afc::AfcClient, IdeviceService};
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    rt.block_on(async {
        let mut mux = UsbmuxdConnection::default().await.unwrap();
        let Some(d) = mux.get_devices().await.unwrap().into_iter().next() else {
            println!("no device");
            return;
        };
        let pair = mux.get_pair_record(&d.udid).await.unwrap();
        let provider = d.to_provider(UsbmuxdAddr::default(), "lumen-spike");
        let mut afc = AfcClient::connect(&provider).await.unwrap();
        println!("AFC device info: {:#?}", afc.get_device_info_raw().await);
        let mut lockdown = LockdownClient::connect(&provider).await.unwrap();
        println!("start_session: {:?}", lockdown.start_session(&pair).await);
        println!("disk_usage: {:#?}", lockdown.get_value(None, Some("com.apple.disk_usage")).await);
        println!("disk_usage.factory: {:#?}", lockdown.get_value(None, Some("com.apple.disk_usage.factory")).await);
    });
}
