//! An app's own icon as a 64 × 64 PNG under `cache/app-icons/` (Round 22 lane C, spec §7 *Icons*).
//! Read from the installed app (the executable's resource on Windows, the bundle's `.icns` on a
//! Mac, the `.desktop` entry's `Icon` on Linux); nothing is bundled. A failure is `None`: the route
//! then answers a generic icon.

use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use image::{imageops::FilterType, DynamicImage};
use photos_core::external_apps::InstalledApp;

const EDGE: u32 = 64;

/// `<id>-<seconds the target was last changed>.png`: an updated app gets a new name, so its old
/// icon is never served again.
pub fn cache_name(app_id: &str, target_mtime_secs: u64) -> String {
    let id: String = app_id.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' { c } else { '_' }).collect();
    format!("{id}-{target_mtime_secs}.png")
}

fn mtime_secs(path: &Path) -> u64 {
    std::fs::metadata(path).and_then(|m| m.modified()).ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map_or(0, |d| d.as_secs())
}

/// The cached icon of `app`, made when it is not there yet.
pub fn icon_png(app: &InstalledApp, cache_dir: &Path) -> Option<PathBuf> {
    // A Store app's target is a `shell:AppsFolder\…` name, not a file: nothing to stat.
    let store = super::system::is_store(&app.target);
    if !store && !app.target.exists() {
        return None;
    }
    let name = cache_name(&app.id, if store { 0 } else { mtime_secs(&app.target) });
    let out = cache_dir.join(&name);
    if out.is_file() {
        return Some(out);
    }
    std::fs::create_dir_all(cache_dir).ok()?;
    let img = extract(app)?;
    let part = cache_dir.join(format!("{name}.part"));
    let saved = img.save_with_format(&part, image::ImageFormat::Png).is_ok() && std::fs::rename(&part, &out).is_ok();
    if !saved {
        let _ = std::fs::remove_file(&part);
        return None;
    }
    forget_older(cache_dir, &app.id, &name);
    Some(out)
}

/// Removes this app's icons made for an older version of it (`<id>-<digits>.png` only: another
/// app whose id starts with this one's is not touched).
fn forget_older(cache_dir: &Path, app_id: &str, keep: &str) {
    let prefix = cache_name(app_id, 0);
    let prefix = &prefix[..prefix.len() - "0.png".len()];
    let Ok(entries) = std::fs::read_dir(cache_dir) else { return };
    for e in entries.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        let is_older = name != keep
            && name.strip_prefix(prefix).and_then(|r| r.strip_suffix(".png")).is_some_and(|d| !d.is_empty() && d.bytes().all(|b| b.is_ascii_digit()));
        if is_older {
            let _ = std::fs::remove_file(e.path());
        }
    }
}

/// Any size in, 64 × 64 out.
fn square(img: DynamicImage) -> DynamicImage {
    if img.width() == EDGE && img.height() == EDGE {
        img
    } else {
        img.resize_exact(EDGE, EDGE, FilterType::Lanczos3)
    }
}

#[cfg(windows)]
fn extract(app: &InstalledApp) -> Option<DynamicImage> {
    if super::system::is_store(&app.target) {
        return store_icon(&app.target).map(square);
    }
    windows_icon(&app.target).map(square)
}

/// A Store app's icon through the shell (its package folder is locked, so no resource to read).
/// COM is set up for this thread and any failure is `None`, which the route answers with the
/// generic icon.
#[cfg(windows)]
fn store_icon(target: &Path) -> Option<DynamicImage> {
    use std::os::windows::ffi::OsStrExt;
    use std::ptr::null_mut;
    use windows_sys::Win32::System::Com::{CoInitializeEx, CoTaskMemFree, CoUninitialize, COINIT_APARTMENTTHREADED};
    use windows_sys::Win32::UI::Shell::{SHGetFileInfoW, SHParseDisplayName, SHFILEINFOW, SHGFI_ICON, SHGFI_LARGEICON, SHGFI_PIDL};

    let wide: Vec<u16> = target.as_os_str().encode_wide().chain(Some(0)).collect();
    // SAFETY: balanced below; a thread already in another COM mode (a negative result) is left alone.
    let com = unsafe { CoInitializeEx(std::ptr::null(), COINIT_APARTMENTTHREADED as u32) } >= 0;
    let mut pidl = null_mut();
    // SAFETY: `wide` is NUL-terminated and `pidl` a valid out-pointer.
    let parsed = unsafe { SHParseDisplayName(wide.as_ptr(), null_mut(), &mut pidl, 0, null_mut()) };
    let mut image = None;
    if parsed >= 0 && !pidl.is_null() {
        let mut info: SHFILEINFOW = unsafe { std::mem::zeroed() };
        // SAFETY: with `SHGFI_PIDL` the first argument is the item id list, not a string.
        let got = unsafe { SHGetFileInfoW(pidl as *const u16, 0, &mut info, std::mem::size_of::<SHFILEINFOW>() as u32, SHGFI_PIDL | SHGFI_ICON | SHGFI_LARGEICON) };
        if got != 0 && !info.hIcon.is_null() {
            image = hicon_image(info.hIcon);
        }
    }
    if !pidl.is_null() {
        // SAFETY: the list was allocated by `SHParseDisplayName`.
        unsafe { CoTaskMemFree(pidl.cast()) };
    }
    if com {
        // SAFETY: pairs the successful `CoInitializeEx` above.
        unsafe { CoUninitialize() };
    }
    image
}

#[cfg(windows)]
fn windows_icon(exe: &Path) -> Option<DynamicImage> {
    use std::os::windows::ffi::OsStrExt;
    use std::ptr::null_mut;
    use windows_sys::Win32::UI::WindowsAndMessaging::{PrivateExtractIconsW, HICON};

    let wide: Vec<u16> = exe.as_os_str().encode_wide().chain(Some(0)).collect();
    let mut icon: HICON = null_mut();
    // The icon nearest 64 px in the executable's resources.
    let got = unsafe { PrivateExtractIconsW(wide.as_ptr(), 0, EDGE as i32, EDGE as i32, &mut icon, null_mut(), 1, 0) };
    if got == 0 || got == u32::MAX || icon.is_null() {
        return None;
    }
    hicon_image(icon)
}

/// An icon handle as an image; the handle is destroyed here.
#[cfg(windows)]
fn hicon_image(icon: windows_sys::Win32::UI::WindowsAndMessaging::HICON) -> Option<DynamicImage> {
    use std::ptr::null_mut;
    use windows_sys::Win32::Graphics::Gdi::{
        CreateCompatibleDC, DeleteDC, DeleteObject, GetDIBits, GetObjectW, BITMAP, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{DestroyIcon, GetIconInfo, ICONINFO};

    let mut info: ICONINFO = unsafe { std::mem::zeroed() };
    let ok = unsafe { GetIconInfo(icon, &mut info) } != 0;
    let mut result = None;
    if ok && !info.hbmColor.is_null() {
        let mut bm: BITMAP = unsafe { std::mem::zeroed() };
        let have = unsafe { GetObjectW(info.hbmColor, std::mem::size_of::<BITMAP>() as i32, (&mut bm as *mut BITMAP).cast()) };
        let (w, h) = (bm.bmWidth, bm.bmHeight);
        if have != 0 && w > 0 && h > 0 && w <= 1024 && h <= 1024 {
            let dc = unsafe { CreateCompatibleDC(null_mut()) };
            let mut header: BITMAPINFO = unsafe { std::mem::zeroed() };
            header.bmiHeader = BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: w,
                biHeight: -h, // top-down rows
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB,
                biSizeImage: 0,
                biXPelsPerMeter: 0,
                biYPelsPerMeter: 0,
                biClrUsed: 0,
                biClrImportant: 0,
            };
            let len = (w * h * 4) as usize;
            let mut color = vec![0u8; len];
            let mut mask = vec![0u8; len];
            let c = unsafe { GetDIBits(dc, info.hbmColor, 0, h as u32, color.as_mut_ptr().cast(), &mut header, DIB_RGB_COLORS) };
            let m = if info.hbmMask.is_null() {
                0
            } else {
                unsafe { GetDIBits(dc, info.hbmMask, 0, h as u32, mask.as_mut_ptr().cast(), &mut header, DIB_RGB_COLORS) }
            };
            unsafe { DeleteDC(dc) };
            if c != 0 {
                // BGRA → RGBA. An icon without an alpha channel says what is see-through in its mask.
                let (pixels, _) = color.as_chunks::<4>();
                let has_alpha = pixels.iter().any(|p| p[3] != 0);
                let mut rgba = Vec::with_capacity(len);
                for (i, p) in pixels.iter().enumerate() {
                    let a = if has_alpha {
                        p[3]
                    } else if m != 0 && mask[i * 4] != 0 {
                        0
                    } else {
                        255
                    };
                    rgba.extend_from_slice(&[p[2], p[1], p[0], a]);
                }
                result = image::RgbaImage::from_raw(w as u32, h as u32, rgba).map(DynamicImage::ImageRgba8);
            }
        }
    }
    unsafe {
        if !info.hbmColor.is_null() {
            DeleteObject(info.hbmColor);
        }
        if !info.hbmMask.is_null() {
            DeleteObject(info.hbmMask);
        }
        DestroyIcon(icon);
    }
    result
}

#[cfg(target_os = "macos")]
fn extract(app: &InstalledApp) -> Option<DynamicImage> {
    use std::process::Command;
    let plist = app.target.join("Contents/Info");
    let read = Command::new("/usr/bin/defaults").arg("read").arg(&plist).arg("CFBundleIconFile").output().ok()?;
    let file = String::from_utf8_lossy(&read.stdout).trim().to_string();
    if !read.status.success() || file.is_empty() || file.contains(['/', '\\']) {
        return None;
    }
    let file = if Path::new(&file).extension().is_some() { file } else { format!("{file}.icns") };
    let icns = app.target.join("Contents/Resources").join(file);
    let tmp = std::env::temp_dir().join(format!("lumen-icon-{}-{}.png", std::process::id(), app.id));
    let done = Command::new("/usr/bin/sips").args(["-s", "format", "png", "-z", "64", "64"]).arg(&icns).arg("--out").arg(&tmp).output().ok()?;
    let img = if done.status.success() { image::open(&tmp).ok() } else { None };
    let _ = std::fs::remove_file(&tmp);
    img.map(square)
}

#[cfg(target_os = "linux")]
fn extract(app: &InstalledApp) -> Option<DynamicImage> {
    let text = std::fs::read_to_string(&app.target).ok()?;
    let (_, icon) = super::system::parse_desktop(&text)?;
    let mut data_dirs: Vec<PathBuf> = Vec::new();
    if let Some(h) = dirs::home_dir() {
        data_dirs.push(h.join(".local/share"));
        data_dirs.push(h.join(".local/share/flatpak/exports/share"));
    }
    let xdg = std::env::var("XDG_DATA_DIRS").ok().filter(|v| !v.is_empty()).unwrap_or_else(|| "/usr/local/share:/usr/share".into());
    data_dirs.extend(xdg.split(':').filter(|d| !d.is_empty()).map(PathBuf::from));
    data_dirs.push("/var/lib/flatpak/exports/share".into());
    data_dirs.push("/var/lib/snapd/desktop".into());
    linux_icon_candidates(&icon?, &data_dirs).into_iter().find_map(|p| image::open(p).ok()).map(square)
}

/// The files a `.desktop` `Icon` may be, best first: the path itself, else the `hicolor` theme at
/// 64, 48 and 128 px and the pixmaps folder. A scalable (SVG) icon is not rasterised this round.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn linux_icon_candidates(icon: &str, data_dirs: &[PathBuf]) -> Vec<PathBuf> {
    if icon.starts_with('/') {
        return if icon.ends_with(".png") { vec![PathBuf::from(icon)] } else { Vec::new() };
    }
    if icon.contains(['/', '\\']) || icon.contains("..") {
        return Vec::new();
    }
    let mut out = Vec::new();
    for size in ["64x64", "48x48", "128x128"] {
        for d in data_dirs {
            out.push(d.join("icons/hicolor").join(size).join("apps").join(format!("{icon}.png")));
        }
    }
    for d in data_dirs {
        out.push(d.join("pixmaps").join(format!("{icon}.png")));
    }
    out
}

#[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
fn extract(_: &InstalledApp) -> Option<DynamicImage> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_cache_name_changes_with_the_targets_time() {
        assert_eq!(cache_name("photoshop", 100), "photoshop-100.png");
        assert_ne!(cache_name("photoshop", 100), cache_name("photoshop", 101));
        // An id never reaches outside the folder.
        assert_eq!(cache_name("a/../b", 1), "a____b-1.png");
    }

    #[test]
    fn an_updated_app_forgets_its_old_icon_and_only_its_own() {
        let dir = tempfile::tempdir().unwrap();
        for n in ["gimp-1.png", "gimp-2.png", "gimp-extra-1.png", "krita-1.png"] {
            std::fs::write(dir.path().join(n), b"x").unwrap();
        }
        forget_older(dir.path(), "gimp", "gimp-2.png");
        let mut left: Vec<_> = std::fs::read_dir(dir.path()).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        left.sort();
        assert_eq!(left, ["gimp-2.png", "gimp-extra-1.png", "krita-1.png"]);
    }

    #[test]
    fn linux_icon_names_look_in_the_theme_and_a_path_is_taken_as_is() {
        let dirs = [PathBuf::from("/usr/share")];
        let c = linux_icon_candidates("gimp", &dirs);
        assert_eq!(c[0], PathBuf::from("/usr/share/icons/hicolor/64x64/apps/gimp.png"));
        assert!(c.contains(&PathBuf::from("/usr/share/pixmaps/gimp.png")));
        assert_eq!(linux_icon_candidates("/opt/x/icon.png", &dirs), [PathBuf::from("/opt/x/icon.png")]);
        assert!(linux_icon_candidates("/opt/x/icon.svg", &dirs).is_empty());
        assert!(linux_icon_candidates("../etc/passwd", &dirs).is_empty());
    }

    #[cfg(windows)]
    fn app_at(target: &str) -> InstalledApp {
        use photos_core::external_apps::{AppKind, Media, Writes};
        InstalledApp {
            id: "explorer-test".into(),
            name: "Explorer".into(),
            kind: AppKind::Editor,
            accepts: vec![Media::Photo],
            reads_raw: false,
            writes: Writes::InPlace,
            target: PathBuf::from(target),
            exec: None,
            custom: false,
            added_by: None,
        }
    }

    #[cfg(windows)]
    #[test]
    fn explorer_exe_has_an_icon() {
        let dir = tempfile::tempdir().unwrap();
        let app = app_at(r"C:\Windows\explorer.exe");
        let png = icon_png(&app, dir.path()).expect("an icon for explorer.exe");
        let img = image::open(&png).expect("a decodable PNG");
        assert_eq!((img.width(), img.height()), (64, 64));
        // Not an empty square: some pixels are opaque, and it is not all one colour.
        let rgba = img.to_rgba8();
        assert!(rgba.pixels().any(|p| p[3] > 200));
        let first = *rgba.pixels().next().unwrap();
        assert!(rgba.pixels().any(|p| *p != first));
        // The second ask is the cached file.
        assert_eq!(icon_png(&app, dir.path()), Some(png));
    }

    #[cfg(windows)]
    #[test]
    fn an_app_that_is_gone_has_no_icon() {
        let dir = tempfile::tempdir().unwrap();
        assert!(icon_png(&app_at(r"C:\no\such\app.exe"), dir.path()).is_none());
    }
}
