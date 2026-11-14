//! The detected list, cached for 60 s, and the per-session icon numbers behind
//! `photos://localhost/app-icon/<n>` (`http://photos.localhost/…` on Windows; the route takes a number, see `protocol.rs`).

use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use photos_core::external_apps::{
    installed, AppSource, BuiltIn, Custom, CustomApp, ExternalApp, InstalledApp, Lookup, Media, SendMode,
};
use photos_core::settings::ExternalAppsSettings;

use super::SystemLookup;
use crate::plugins::Plugins;

const STALE_AFTER: Duration = Duration::from_secs(60);

/// The built-in and plugin apps found on this machine, when, and which plugins' apps they were
/// found for (so installing, removing or turning one off shows at once, not after 60 s).
static BUILT_IN: Mutex<Option<(Instant, String, Vec<InstalledApp>)>> = Mutex::new(None);
/// The plugin store, for the apps plugins add (Round 23 B2); set once at start-up.
static PLUGINS: OnceLock<Arc<Plugins>> = OnceLock::new();

/// Lets detection read the enabled plugins' `apps.toml` (called once from `lib.rs`).
pub fn use_plugins(plugins: Arc<Plugins>) {
    let _ = PLUGINS.set(plugins);
}

/// Which enabled plugins list apps, by id and version: a cheap stand-in for "did they change".
fn plugin_signature() -> String {
    PLUGINS
        .get()
        .map(|p| p.read().enabled().iter().filter(|i| i.manifest.apps.is_some()).map(|i| format!("{}@{};", i.manifest.id, i.manifest.version)).collect())
        .unwrap_or_default()
}
/// App ids by their icon number − 1, for this session.
static ICON_IDS: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// Apps already found, offered back as a source.
struct Found(Vec<InstalledApp>);

impl AppSource for Found {
    fn apps(&self, _: &dyn Lookup) -> Vec<InstalledApp> {
        self.0.clone()
    }
}

/// Every installed app: the person's own first, then the built-in table and the plugins' apps as
/// last detected (re-run when that is more than 60 s old, or the plugins changed). The custom list
/// is checked on every call: it is a handful of `exists` calls.
pub fn detected(custom: &[CustomApp]) -> Vec<InstalledApp> {
    let signature = plugin_signature();
    let found = {
        let mut cache = BUILT_IN.lock().expect("app cache lock");
        match cache.as_ref() {
            Some((at, sig, apps)) if at.elapsed() < STALE_AFTER && *sig == signature => apps.clone(),
            _ => {
                let mut apps = BuiltIn::current().apps(&SystemLookup);
                if let Some(plugins) = PLUGINS.get() {
                    for source in crate::plugins::data::plugin_apps(&plugins.read()) {
                        apps.extend(source.apps(&SystemLookup));
                    }
                }
                *cache = Some((Instant::now(), signature, apps.clone()));
                apps
            }
        }
    };
    installed(&[&Custom { entries: custom.to_vec() }, &Found(found)], &SystemLookup)
}

/// Drops the cached list: an app turned out to be gone, so the next menu looks again.
pub fn forget_detected() {
    *BUILT_IN.lock().expect("app cache lock") = None;
}

/// A number for an app id that stays the same until Lumen closes.
fn icon_number(id: &str) -> usize {
    let mut ids = ICON_IDS.lock().expect("icon ids lock");
    let at = ids.iter().position(|i| i == id).unwrap_or_else(|| {
        ids.push(id.to_string());
        ids.len() - 1
    });
    at + 1
}

/// When this run of Lumen started: the number behind an icon URL means another app next time, so
/// the URL carries this and the webview may keep the icon for as long as it likes.
fn session() -> u64 {
    static START: OnceLock<u64> = OnceLock::new();
    *START.get_or_init(|| SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs()))
}

/// The same scheme forms `src/utils/media.ts` builds: WebView2 only serves custom schemes as
/// `http://photos.localhost/…`.
fn icon_url_for(windows: bool, number: usize) -> String {
    let base = if windows { "http://photos.localhost" } else { "photos://localhost" };
    format!("{base}/app-icon/{number}?v={}", session())
}

pub fn icon_url(id: &str) -> String {
    icon_url_for(cfg!(windows), icon_number(id))
}

/// The app an icon number was given to (the `app-icon` route serves its icon).
pub fn icon_target(number: usize, settings: &ExternalAppsSettings) -> Option<InstalledApp> {
    let id = ICON_IDS.lock().expect("icon ids lock").get(number.checked_sub(1)?)?.clone();
    detected(&settings.custom).into_iter().find(|a| a.id == id)
}

/// What the page is told: the apps that accept `media` (all when `None`), with their icon URLs and
/// the person's choice of what is sent.
pub fn apps_for(settings: &ExternalAppsSettings, media: Option<Media>) -> Vec<ExternalApp> {
    detected(&settings.custom)
        .into_iter()
        .filter(|a| media.is_none_or(|m| a.accepts.contains(&m)))
        .map(|a| to_external(&a, settings))
        .collect()
}

pub fn to_external(a: &InstalledApp, settings: &ExternalAppsSettings) -> ExternalApp {
    ExternalApp {
        id: a.id.clone(),
        name: a.name.clone(),
        kind: a.kind,
        accepts: a.accepts.clone(),
        icon_url: icon_url(&a.id),
        custom: a.custom,
        send: settings.send.get(&a.id).copied().unwrap_or(SendMode::Automatic),
        writes: a.writes,
        added_by: a.added_by.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn icon_numbers_are_stable_per_id_and_start_at_one() {
        let a = icon_number("state-test-a");
        let b = icon_number("state-test-b");
        assert_ne!(a, b);
        assert!(a >= 1 && b >= 1);
        assert_eq!(icon_number("state-test-a"), a);
        assert_eq!(icon_url("state-test-b"), icon_url_for(cfg!(windows), b));
        assert_eq!(icon_url_for(true, b), format!("http://photos.localhost/app-icon/{b}?v={}", session()));
        assert_eq!(icon_url_for(false, b), format!("photos://localhost/app-icon/{b}?v={}", session()));
    }

    #[test]
    fn a_custom_app_that_exists_is_listed_and_filtered_by_media() {
        let exe = std::env::current_exe().unwrap();
        let entry = photos_core::external_apps::custom_app_for(&exe);
        let settings = ExternalAppsSettings { custom: vec![entry.clone()], ..Default::default() };
        let all = apps_for(&settings, None);
        assert!(all.iter().any(|a| a.id == entry.id && a.custom));
        // A custom app accepts every kind of media.
        for m in [Media::Photo, Media::Raw, Media::Video] {
            assert!(apps_for(&settings, Some(m)).iter().any(|a| a.id == entry.id));
        }
    }
}
