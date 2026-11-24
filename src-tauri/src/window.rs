use std::sync::atomic::Ordering;
use std::sync::{Mutex, MutexGuard, RwLock, TryLockError};
use std::time::{Duration, Instant};

use photos_core::settings::ColorScheme;
use tauri::window::Color;
use tauri::{App, Emitter, Manager, PhysicalPosition, PhysicalSize, Theme, Window, WindowEvent};

use crate::state::AppState;

/// Apply the persisted size/position before the window is shown; `minimized` (the Run key's
/// `--minimized`) then sends it to the taskbar, so restoring it brings back that size or maximised.
pub fn apply_saved_state(app: &mut App, minimized: bool) -> Result<(), Box<dyn std::error::Error>> {
    let window = app.get_webview_window("main").ok_or("main window missing")?;
    let state = app.state::<AppState>();
    let (ws, scheme) = {
        let s = state.settings.read().expect("settings lock");
        (s.window.clone(), s.appearance.scheme)
    };
    // Until the page paints its boot screen the webview shows its own background: white,
    // a flash in dark mode. Paint it the scheme's base colour instead.
    let dark = is_dark(scheme, window.theme().unwrap_or(Theme::Light));
    let _ = window.set_background_color(Some(boot_background(dark)));
    window.set_size(PhysicalSize::new(ws.width.max(900), ws.height.max(600)))?;
    if let (Some(x), Some(y)) = (ws.x, ws.y) {
        // A place saved on a monitor that is gone (or Windows' minimized parking spot) would
        // open the window out of reach: let the OS place it instead.
        let screens: Vec<(i32, i32, u32, u32)> = window
            .available_monitors()
            .unwrap_or_default()
            .iter()
            .map(|m| (m.position().x, m.position().y, m.size().width, m.size().height))
            .collect();
        let (w, h) = (ws.width.max(900), ws.height.max(600));
        if screens.is_empty() || on_a_screen(x, y, w, h, &screens) {
            window.set_position(PhysicalPosition::new(x, y))?;
        } else {
            tracing::warn!(x, y, "saved window place is off every monitor; centring instead");
            let _ = window.center();
        }
    }
    if ws.maximized {
        window.maximize()?;
    }
    window.show()?;
    if minimized {
        window.minimize()?;
    }
    Ok(())
}

/// Whether a window at `(x, y)` of `w`×`h` has at least a 100×40 px piece of its top (the title
/// bar, the part to grab) on one of the `screens` (`x, y, width, height`).
fn on_a_screen(x: i32, y: i32, w: u32, h: u32, screens: &[(i32, i32, u32, u32)]) -> bool {
    let (x, y) = (i64::from(x), i64::from(y));
    let (right, bottom) = (x + i64::from(w), y + i64::from(h).min(40));
    screens.iter().any(|&(sx, sy, sw, sh)| {
        let (sx, sy) = (i64::from(sx), i64::from(sy));
        let (sr, sb) = (sx + i64::from(sw), sy + i64::from(sh));
        right.min(sr) - x.max(sx) >= 100 && bottom.min(sb) - y.max(sy) >= 40.min(i64::from(h))
    })
}

fn is_dark(scheme: ColorScheme, os: Theme) -> bool {
    match scheme {
        ColorScheme::Dark => true,
        ColorScheme::Light => false,
        ColorScheme::Auto => os == Theme::Dark,
    }
}

/// Close to the themes' `--bg-base` (Gallery: white / oklch 12 %).
fn boot_background(dark: bool) -> Color {
    if dark {
        Color(10, 10, 10, 255)
    } else {
        Color(255, 255, 255, 255)
    }
}

/// Whether closing must wait for the user: an import is running and they have not chosen
/// to stop it. Closing would cut it off mid-file.
pub fn should_prevent_close(imports: usize, confirmed: bool) -> bool {
    imports > 0 && !confirmed
}

/// What Lumen was started with (Round 18 A2). `--minimized`: the Run key at sign-in opens Lumen
/// minimised to the taskbar. `--quit`: Lumen Setup asks the running Lumen to close (see [`second_launch`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct LaunchFlags {
    pub quit: bool,
    pub minimized: bool,
}

/// Reads the flags from a full command line; the first item is the program and never a flag.
pub fn launch_flags<I: IntoIterator<Item = S>, S: AsRef<str>>(args: I) -> LaunchFlags {
    let mut f = LaunchFlags::default();
    for a in args.into_iter().skip(1) {
        match a.as_ref() {
            "--quit" => f.quit = true,
            "--minimized" => f.minimized = true,
            _ => {}
        }
    }
    f
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecondLaunch {
    /// Setup is about to install: close the normal way (Develop flush, import question), no update install.
    Quit,
    /// The icon was opened again: bring the open window to the front.
    Focus,
    /// The Run key fired with Lumen already open: leave the desktop alone.
    Nothing,
}

/// What the running Lumen does when the single-instance check hands it another launch's flags.
pub fn second_launch(f: LaunchFlags) -> SecondLaunch {
    if f.quit {
        SecondLaunch::Quit
    } else if f.minimized {
        SecondLaunch::Nothing
    } else {
        SecondLaunch::Focus
    }
}

/// Set once Setup's `--quit` arrived; for the rest of this process the staged update is not
/// installed at exit (`updater::installs_at_this_exit`), since Setup is installing.
static QUITTING_FOR_SETUP: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub fn quit_for_setup() {
    QUITTING_FOR_SETUP.store(true, Ordering::SeqCst);
}

pub fn quitting_for_setup() -> bool {
    QUITTING_FOR_SETUP.load(Ordering::SeqCst)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImportGuard {
    Close,
    /// Hold the close and ask the person; `tell_setup` when Setup's `--quit` is waiting on the answer.
    Ask { tell_setup: bool },
}

/// The import step of a close. No time limit: an import is only ever stopped by the person.
pub fn import_guard(imports: usize, confirmed: bool, quitting: bool) -> ImportGuard {
    if should_prevent_close(imports, confirmed) {
        ImportGuard::Ask { tell_setup: quitting }
    } else {
        ImportGuard::Close
    }
}

/// The file Setup looks for (in the temp folder, by Lumen's pid) to say "Lumen is asking about the
/// import that's running". Not in app data, which Setup never writes and Lumen's uninstall lists.
pub fn quit_held_marker(dir: &std::path::Path, pid: u32) -> std::path::PathBuf {
    dir.join(format!("lumen-quit-held-{pid}"))
}

pub fn mark_quit_held(dir: &std::path::Path, pid: u32) {
    if let Err(e) = std::fs::write(quit_held_marker(dir, pid), b"import") {
        tracing::warn!("could not tell Setup that Lumen is asking about an import: {e}");
    }
}

pub fn clear_quit_held(dir: &std::path::Path, pid: u32) {
    let _ = std::fs::remove_file(quit_held_marker(dir, pid));
}

/// The single-instance check's callback: another launch's command line, in the running Lumen.
pub fn on_second_launch(app: &tauri::AppHandle, args: &[String]) {
    let Some(window) = app.get_webview_window("main") else { return };
    match second_launch(launch_flags(args)) {
        SecondLaunch::Quit => {
            tracing::info!("Lumen Setup asked Lumen to close");
            quit_for_setup();
            // `close` goes through CloseRequested, so `on_window_event` runs its usual steps.
            let _ = window.close();
        }
        SecondLaunch::Focus => bring_to_front(&window),
        SecondLaunch::Nothing => {}
    }
}

fn bring_to_front(window: &tauri::WebviewWindow) {
    let _ = window.unminimize();
    let _ = window.show();
    let _ = window.set_focus();
}

/// `RunEvent::Exit`: Setup's marker goes with the process.
pub fn on_exit() {
    if quitting_for_setup() {
        clear_quit_held(&std::env::temp_dir(), std::process::id());
    }
}

/// How long a close waits for the page to write unsaved edits before it goes through anyway.
pub const FLUSH_WAIT: std::time::Duration = std::time::Duration::from_secs(3);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnsavedClose {
    /// Nothing waits to be saved (or the page never answered): close as usual.
    Proceed,
    /// Hold the close and ask the page to write; it closes again once it has.
    AskToFlush,
}

/// Round 5: closing with Develop edits still waiting to be saved. The page is asked to write
/// them and close again (which then meets the import guard as usual); a page that does not
/// answer within `FLUSH_WAIT` cannot hold the window open.
pub fn unsaved_close(unsaved: bool, asked_at: Option<std::time::Instant>, now: std::time::Instant) -> UnsavedClose {
    match (unsaved, asked_at) {
        (false, _) => UnsavedClose::Proceed,
        (true, Some(at)) if now.duration_since(at) > FLUSH_WAIT => UnsavedClose::Proceed,
        (true, _) => UnsavedClose::AskToFlush,
    }
}

/// After the window is gone, how long the process may take to end by itself before it is
/// ended (Round 15: a worker stuck in a driver or FFmpeg must never keep Lumen alive).
pub const EXIT_GRACE: Duration = Duration::from_secs(4);

/// How long a process that is handing over to the update installer may stay. The window has gone,
/// so nobody waits on this; it only bounds an install that is itself stuck.
pub const INSTALL_GRACE: Duration = Duration::from_secs(20);

/// Whether the exit guard ends the process now. An update being installed gets longer: ending the
/// process then means the installer never starts (Round 17).
pub fn force_exit_due(closed_at: Instant, now: Instant, installing: bool) -> bool {
    now.saturating_duration_since(closed_at) > if installing { INSTALL_GRACE } else { EXIT_GRACE }
}

/// How long the close path tries a lock before it skips that step: it runs on the main
/// thread, and a lock held by a stuck thread must not freeze the window.
const LOCK_TRY: Duration = Duration::from_millis(50);

fn try_lock_for<T>(m: &Mutex<T>, wait: Duration) -> Option<MutexGuard<'_, T>> {
    let until = Instant::now() + wait;
    loop {
        match m.try_lock() {
            Ok(g) => return Some(g),
            Err(TryLockError::Poisoned(p)) => return Some(p.into_inner()),
            Err(TryLockError::WouldBlock) if Instant::now() < until => std::thread::sleep(Duration::from_millis(5)),
            Err(TryLockError::WouldBlock) => return None,
        }
    }
}

/// Changes the settings when the lock is free; returns false (and waits for nothing) when
/// it is busy: closing matters more than remembering the window's size.
pub fn save_window_state_with<T>(lock: &RwLock<T>, f: impl FnOnce(&mut T)) -> bool {
    match lock.try_write() {
        Ok(mut g) => {
            f(&mut g);
            true
        }
        Err(TryLockError::Poisoned(p)) => {
            f(&mut p.into_inner());
            true
        }
        Err(TryLockError::WouldBlock) => false,
    }
}

/// The close goes through: no new jobs start, Denoise runs are cancelled, and a guard ends
/// the process `EXIT_GRACE` from now if it is still alive. Once per process.
pub fn begin_shutdown(app: &tauri::AppHandle, state: &AppState) {
    static BEGUN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    if BEGUN.swap(true, Ordering::SeqCst) {
        return;
    }
    // Armed first (Round 24 §6.3): nothing below may be able to keep the process alive.
    arm_exit_guard();
    // The person closed Lumen: the next start is not a crash.
    crate::crash_marker::clean_exit();
    crate::discord_presence::shutdown();
    state.jobs.shutdown();
    crate::denoise::cancel_all();
    // A staged update installs at `RunEvent::Exit` only if that comes soon after this (Round 17).
    crate::updater::note_closed();
    // A staged update gives the guard the long grace, so a slow Close doesn't skip the install.
    crate::updater::note_close_install(app);
}

/// Ends the process `EXIT_GRACE` from now if it is still alive (`INSTALL_GRACE` while an update is
/// being installed; the guard asks each tick, so an install noted later still gets it). Once.
fn arm_exit_guard() {
    static ARMED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    if ARMED.swap(true, Ordering::SeqCst) {
        return;
    }
    let closed_at = Instant::now();
    let _ = std::thread::Builder::new().name("lumen-exit-guard".into()).spawn(move || loop {
        std::thread::sleep(Duration::from_millis(250));
        if force_exit_due(closed_at, Instant::now(), crate::updater::install_pending()) {
            tracing::warn!(after_ms = closed_at.elapsed().as_millis() as u64, "exit guard: still running after Close; ending the process");
            photos_core::child_jobs::end_this_process();
        }
    });
}

/// The window's place, for the next launch. A maximized window keeps its restored size.
pub fn remember_geometry(
    w: &mut photos_core::settings::WindowState,
    maximized: bool,
    minimized: bool,
    size: Option<PhysicalSize<u32>>,
    pos: Option<PhysicalPosition<i32>>,
) {
    // A minimized window reports Windows' parking spot (-32000,-32000) and a stub size:
    // keep the last real place (and `maximized`, which a minimized window can still be).
    if minimized {
        return;
    }
    w.maximized = maximized;
    if maximized {
        return;
    }
    if let Some(size) = size {
        w.width = size.width;
        w.height = size.height;
    }
    if let Some(pos) = pos {
        w.x = Some(pos.x);
        w.y = Some(pos.y);
    }
}

/// Persist size/position on close so the next launch restores it. While an import runs, the
/// close is held and the frontend asks first (`app://close-requested`). Runs on the main
/// thread, so it never waits on a lock for long (Round 15).
pub fn on_window_event(window: &Window, event: &WindowEvent) {
    // Round 22 lane C: copies saved by the person's editors come back when Lumen is looked at again.
    if matches!(event, WindowEvent::Focused(true)) && window.label() == "main" {
        crate::external_edits::on_focus(window.app_handle());
    }
    let WindowEvent::CloseRequested { api, .. } = event else { return };
    if window.label() != "main" {
        return;
    }
    let state = window.state::<AppState>();
    // Round 24 §6.3: a page that has stopped answering is not asked anything, so Close always
    // closes: unsaved edits and the import question both need the page to reply.
    let page_ok = crate::watchdog::page_is_answering();
    if !page_ok {
        tracing::warn!("close: the page is not answering; closing without asking it");
    }
    match try_lock_for(&state.flush_asked_at, LOCK_TRY) {
        Some(mut asked) => {
            let now = Instant::now();
            match unsaved_close(page_ok && state.unsaved_edits.load(Ordering::SeqCst), *asked, now) {
                UnsavedClose::AskToFlush => {
                    api.prevent_close();
                    if asked.is_none() {
                        // A page that dies after being asked cannot hold the window: close again
                        // once the wait is over (it then goes through, `unsaved_close`).
                        let again = window.clone();
                        let _ = std::thread::Builder::new().name("lumen-close-again".into()).spawn(move || {
                            std::thread::sleep(FLUSH_WAIT + Duration::from_millis(300));
                            let _ = again.close();
                        });
                    }
                    asked.get_or_insert(now);
                    // `true`: close again once written (*Restart* sends `false`).
                    let _ = window.emit(crate::events::FLUSH_REQUESTED, true);
                    return;
                }
                UnsavedClose::Proceed => *asked = None,
            }
        }
        None => tracing::warn!("close: the unsaved-edits check was busy; closing without it"),
    }
    match try_lock_for(&state.imports, LOCK_TRY).map(|m| m.len()) {
        Some(running) => {
            let confirmed = state.close_confirmed.load(Ordering::SeqCst);
            if let ImportGuard::Ask { tell_setup } = import_guard(if page_ok { running } else { 0 }, confirmed, quitting_for_setup()) {
                api.prevent_close();
                if tell_setup {
                    // Setup waits for this answer; the question must be where the person can see it.
                    mark_quit_held(&std::env::temp_dir(), std::process::id());
                    if let Some(w) = window.app_handle().get_webview_window("main") {
                        bring_to_front(&w);
                    }
                }
                let _ = window.emit(crate::events::CLOSE_REQUESTED, running);
                return;
            }
        }
        None => tracing::warn!("close: the import list was busy; closing without asking"),
    }
    // Before the settings write (disk) or any other step that could stall: from here Close ends.
    arm_exit_guard();
    let maximized = window.is_maximized().unwrap_or(false);
    let minimized = window.is_minimized().unwrap_or(false);
    let (size, pos) = (window.inner_size().ok(), window.outer_position().ok());
    let file = state.paths.settings_file();
    let saved = save_window_state_with(&state.settings, |s| {
        remember_geometry(&mut s.window, maximized, minimized, size, pos);
        if let Err(e) = s.save(&file) {
            tracing::warn!("failed to save window state: {e:?}");
        }
    });
    if !saved {
        tracing::warn!("close: settings were busy; the window's size is not remembered this time");
    }
    begin_shutdown(window.app_handle(), &state);
}

#[cfg(test)]
mod tests {
    use super::{
        force_exit_due, is_dark, on_a_screen, remember_geometry, save_window_state_with, should_prevent_close, unsaved_close, UnsavedClose, EXIT_GRACE,
        FLUSH_WAIT, INSTALL_GRACE,
    };
    use photos_core::settings::ColorScheme;
    use tauri::{PhysicalPosition, PhysicalSize, Theme};
    use std::time::{Duration, Instant};

    #[test]
    fn force_exit_after_the_grace() {
        let t = Instant::now();
        assert!(!force_exit_due(t, t + Duration::from_secs(3), false));
        assert!(force_exit_due(t, t + EXIT_GRACE + Duration::from_millis(1), false));
    }

    #[test]
    fn a_running_install_is_not_cut_off() {
        // Seen in the e2e: the guard ended Lumen after the installer was written, before it started.
        let t = Instant::now();
        assert!(!force_exit_due(t, t + EXIT_GRACE + Duration::from_secs(1), true));
        // A stuck install still can't keep Lumen alive for ever.
        assert!(force_exit_due(t, t + INSTALL_GRACE + Duration::from_millis(1), true));
    }

    #[test]
    fn a_busy_settings_lock_skips_saving_the_window_instead_of_waiting() {
        let lock = std::sync::RwLock::new(5u32);
        {
            let _held = lock.read().unwrap();
            assert!(!save_window_state_with(&lock, |v| *v = 6), "busy: skipped, not blocked");
        }
        assert_eq!(*lock.read().unwrap(), 5);
        assert!(save_window_state_with(&lock, |v| *v = 6));
        assert_eq!(*lock.read().unwrap(), 6);
    }

    #[test]
    fn closing_with_unsaved_edits_asks_the_page_to_write_them_first() {
        let now = Instant::now();
        assert_eq!(unsaved_close(false, None, now), UnsavedClose::Proceed);
        assert_eq!(unsaved_close(true, None, now), UnsavedClose::AskToFlush);
        // Asked a moment ago and still unsaved: keep waiting for the page.
        assert_eq!(unsaved_close(true, Some(now - Duration::from_millis(500)), now), UnsavedClose::AskToFlush);
        // A page that never answers cannot keep the window open for ever.
        assert_eq!(unsaved_close(true, Some(now - FLUSH_WAIT - Duration::from_millis(1)), now), UnsavedClose::Proceed);
    }

    #[test]
    fn a_minimized_window_does_not_overwrite_the_remembered_place() {
        // Windows parks a minimized window at -32000,-32000 with a zero-ish size; saving that
        // made the next launch open off-screen (taskbar icon, no window).
        let mut w = photos_core::settings::WindowState { width: 1280, height: 800, x: Some(100), y: Some(50), ..Default::default() };
        remember_geometry(&mut w, false, true, Some(PhysicalSize::new(160, 28)), Some(PhysicalPosition::new(-32000, -32000)));
        assert_eq!((w.width, w.height, w.x, w.y), (1280, 800, Some(100), Some(50)));
        remember_geometry(&mut w, false, false, Some(PhysicalSize::new(1000, 700)), Some(PhysicalPosition::new(20, 30)));
        assert_eq!((w.width, w.height, w.x, w.y), (1000, 700, Some(20), Some(30)));
    }

    #[test]
    fn a_saved_place_off_every_monitor_is_not_restored() {
        let screens = [(0, 0, 1920, 1080), (1920, 0, 1920, 1080)];
        assert!(on_a_screen(100, 50, 1280, 800, &screens));
        assert!(on_a_screen(2000, 100, 900, 600, &screens), "second monitor");
        assert!(!on_a_screen(-32000, -32000, 900, 600, &screens), "Windows' minimized parking spot");
        assert!(!on_a_screen(5000, 100, 900, 600, &screens), "a monitor that was unplugged");
        assert!(!on_a_screen(1900, 1070, 900, 600, &screens), "only a sliver on screen: no title bar to grab");
    }

    #[test]
    fn the_window_background_follows_the_scheme_and_auto_follows_the_os() {
        assert!(is_dark(ColorScheme::Dark, Theme::Light));
        assert!(!is_dark(ColorScheme::Light, Theme::Dark));
        assert!(is_dark(ColorScheme::Auto, Theme::Dark));
        assert!(!is_dark(ColorScheme::Auto, Theme::Light));
    }

    #[test]
    fn minimized_flag_parsed() {
        use super::{launch_flags, LaunchFlags};
        let f = |a: &[&str]| launch_flags(a.iter().copied());
        assert_eq!(f(&["lumen.exe"]), LaunchFlags::default());
        assert_eq!(f(&["lumen.exe", "--minimized"]), LaunchFlags { minimized: true, quit: false });
        assert_eq!(f(&["lumen.exe", "--quit"]), LaunchFlags { minimized: false, quit: true });
        // argv[0] is the program, never a flag; other arguments are left alone.
        assert_eq!(f(&["--minimized"]), LaunchFlags::default());
        assert_eq!(f(&["lumen.exe", "--minimised", "minimized", "--other"]), LaunchFlags::default());
    }

    #[test]
    fn a_second_launch_quits_focuses_or_stays_out_of_the_way() {
        use super::{second_launch, LaunchFlags, SecondLaunch};
        assert_eq!(second_launch(LaunchFlags { quit: true, minimized: false }), SecondLaunch::Quit);
        assert_eq!(second_launch(LaunchFlags { quit: true, minimized: true }), SecondLaunch::Quit);
        // Double-clicking the icon while Lumen is open brings it to the front.
        assert_eq!(second_launch(LaunchFlags::default()), SecondLaunch::Focus);
        // The Run key firing while Lumen is already open must not pull it over the desktop.
        assert_eq!(second_launch(LaunchFlags { quit: false, minimized: true }), SecondLaunch::Nothing);
    }

    #[test]
    fn quit_waits_on_the_import_question() {
        use super::{clear_quit_held, import_guard, mark_quit_held, quit_held_marker, ImportGuard};
        // Setup's --quit meets a running import: the person is asked, exactly as on a normal Close,
        // and Setup is told why it waits. There is no clock in this decision: no timeout closes it.
        assert_eq!(import_guard(1, false, true), ImportGuard::Ask { tell_setup: true });
        assert_eq!(import_guard(1, false, false), ImportGuard::Ask { tell_setup: false });
        // They chose to stop the import (or none runs): the close goes on.
        assert_eq!(import_guard(1, true, true), ImportGuard::Close);
        assert_eq!(import_guard(0, false, true), ImportGuard::Close);

        // What Setup reads: a marker named by Lumen's pid, there until the process ends.
        let dir = tempfile::tempdir().unwrap();
        let marker = quit_held_marker(dir.path(), 4242);
        assert!(!marker.exists());
        mark_quit_held(dir.path(), 4242);
        assert!(marker.exists());
        mark_quit_held(dir.path(), 4242); // asked twice: still one marker, no error
        clear_quit_held(dir.path(), 4242);
        assert!(!marker.exists());
        clear_quit_held(dir.path(), 4242); // already gone: fine
    }

    #[test]
    fn closing_waits_only_for_a_running_import_the_user_has_not_given_up() {
        assert!(!should_prevent_close(0, false));
        assert!(should_prevent_close(1, false));
        assert!(!should_prevent_close(2, true));
    }
}
