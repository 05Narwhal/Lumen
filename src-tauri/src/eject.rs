//! *Eject* on a card or drive (Round 4G, the Connected entries' menu): the OS's own eject,
//! so the volume can be pulled out safely. A device is a read-only source to Lumen, and this
//! writes nothing to it; it only asks the OS to let it go. A drive holding the open library,
//! or one an import is reading from, is refused.

use std::path::Path;
use std::process::Command;

use crate::error::{AppError, AppResult};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Os {
    Windows,
    Mac,
    Linux,
}

impl Os {
    pub fn current() -> Self {
        if cfg!(windows) {
            Os::Windows
        } else if cfg!(target_os = "macos") {
            Os::Mac
        } else {
            Os::Linux
        }
    }
}

/// The program and arguments that eject `mount`, passed as separate arguments (never a shell
/// string the mount could break out of). Windows: the Shell's *Eject* verb on the drive, as
/// Explorer's menu does; the drive letter is checked to be just a letter first.
pub fn eject_command(os: Os, mount: &str) -> Option<(String, Vec<String>)> {
    match os {
        Os::Windows => {
            let letter = mount.trim_end_matches(['\\', '/']).trim_end_matches(':');
            if letter.len() != 1 || !letter.chars().all(|c| c.is_ascii_alphabetic()) {
                return None;
            }
            let script = format!("(New-Object -ComObject Shell.Application).Namespace(17).ParseName('{letter}:').InvokeVerb('Eject')");
            Some(("powershell".into(), vec!["-NoProfile".into(), "-NonInteractive".into(), "-Command".into(), script]))
        }
        Os::Mac => Some(("diskutil".into(), vec!["eject".into(), mount.into()])),
        Os::Linux => Some(("gio".into(), vec!["mount".into(), "--eject".into(), mount.into()])),
    }
}

/// Whether `library_root` lives on the volume mounted at `mount`.
pub fn holds_library(mount: &Path, library_root: &Path) -> bool {
    let norm = |p: &Path| p.to_string_lossy().replace('\\', "/").trim_end_matches('/').to_ascii_lowercase();
    let (m, l) = (norm(mount), norm(library_root));
    !m.is_empty() && (l == m || l.starts_with(&format!("{m}/")))
}

pub fn eject(mount: &str) -> AppResult<()> {
    // Windows: what *Safely Remove Hardware* does, so the drive really leaves the system.
    // The Shell's *Eject* verb reports success even when Windows refuses.
    #[cfg(windows)]
    {
        let letter = mount.trim_end_matches(['\\', '/']).trim_end_matches(':');
        if letter.len() == 1 && letter.chars().all(|c| c.is_ascii_alphabetic()) {
            return win::eject_drive(letter.chars().next().unwrap());
        }
    }
    let (program, args) = eject_command(Os::current(), mount).ok_or_else(|| AppError {
        code: "cannot_eject".into(),
        message: "That drive cannot be ejected from here.".into(),
        detail: Some(mount.into()),
    })?;
    let mut cmd = Command::new(&program);
    cmd.args(&args);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // No console window flashing up.
        cmd.creation_flags(0x0800_0000);
    }
    let out = cmd.output().map_err(|e| AppError { code: "eject_failed".into(), message: "The drive could not be ejected.".into(), detail: Some(e.to_string()) })?;
    if out.status.success() {
        return Ok(());
    }
    // The OS's own words, which say what is holding the drive.
    let said = String::from_utf8_lossy(&out.stderr).trim().to_string();
    Err(AppError {
        code: "eject_failed".into(),
        message: if said.is_empty() { "The drive could not be ejected; something may still be using it.".into() } else { said },
        detail: Some(mount.into()),
    })
}

#[cfg(windows)]
mod win {
    //! The drive letter → its disk (by device number) → the disk's parent device (the USB
    //! stick or card reader) → `CM_Request_Device_EjectW`. Windows asks every program holding
    //! the volume first and names the one that refuses.
    use std::ptr::{null, null_mut};

    use windows_sys::Win32::Devices::DeviceAndDriverInstallation::{
        CM_Get_Parent, CM_Request_Device_EjectW, SetupDiDestroyDeviceInfoList, SetupDiEnumDeviceInterfaces, SetupDiGetClassDevsW,
        SetupDiGetDeviceInterfaceDetailW, CR_SUCCESS, DIGCF_DEVICEINTERFACE, DIGCF_PRESENT, SP_DEVICE_INTERFACE_DATA, SP_DEVICE_INTERFACE_DETAIL_DATA_W,
        SP_DEVINFO_DATA,
    };
    use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::Storage::FileSystem::{CreateFileW, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING};
    use windows_sys::Win32::System::Ioctl::{GUID_DEVINTERFACE_DISK, IOCTL_STORAGE_GET_DEVICE_NUMBER, STORAGE_DEVICE_NUMBER};
    use windows_sys::Win32::System::IO::DeviceIoControl;

    use crate::error::{AppError, AppResult};

    fn failed(message: &str, detail: String) -> AppError {
        AppError { code: "eject_failed".into(), message: message.into(), detail: Some(detail) }
    }

    /// `(device type, device number)` of a volume or disk path, opened with no access rights.
    fn device_number(path: &[u16]) -> Option<(u32, u32)> {
        unsafe {
            let h = CreateFileW(path.as_ptr(), 0, FILE_SHARE_READ | FILE_SHARE_WRITE, null(), OPEN_EXISTING, 0, null_mut());
            if h == INVALID_HANDLE_VALUE {
                return None;
            }
            let mut n = STORAGE_DEVICE_NUMBER { DeviceType: 0, DeviceNumber: 0, PartitionNumber: 0 };
            let mut got = 0u32;
            let ok = DeviceIoControl(
                h,
                IOCTL_STORAGE_GET_DEVICE_NUMBER,
                null(),
                0,
                (&mut n as *mut STORAGE_DEVICE_NUMBER).cast(),
                size_of::<STORAGE_DEVICE_NUMBER>() as u32,
                &mut got,
                null_mut(),
            );
            CloseHandle(h);
            (ok != 0).then_some((n.DeviceType, n.DeviceNumber))
        }
    }

    /// The device instance of the disk with this number.
    fn disk_instance(wanted: (u32, u32)) -> Option<u32> {
        unsafe {
            let set = SetupDiGetClassDevsW(&GUID_DEVINTERFACE_DISK, null(), null_mut(), DIGCF_PRESENT | DIGCF_DEVICEINTERFACE);
            if set == -1 {
                return None;
            }
            let mut found = None;
            for index in 0.. {
                let mut iface: SP_DEVICE_INTERFACE_DATA = std::mem::zeroed();
                iface.cbSize = size_of::<SP_DEVICE_INTERFACE_DATA>() as u32;
                if SetupDiEnumDeviceInterfaces(set, null(), &GUID_DEVINTERFACE_DISK, index, &mut iface) == 0 {
                    break;
                }
                // The detail is a u32 size then the path; a u64 buffer keeps it aligned.
                let mut buf = vec![0u64; 256];
                let detail = buf.as_mut_ptr().cast::<SP_DEVICE_INTERFACE_DETAIL_DATA_W>();
                (*detail).cbSize = size_of::<SP_DEVICE_INTERFACE_DETAIL_DATA_W>() as u32;
                let mut info: SP_DEVINFO_DATA = std::mem::zeroed();
                info.cbSize = size_of::<SP_DEVINFO_DATA>() as u32;
                if SetupDiGetDeviceInterfaceDetailW(set, &iface, detail, (buf.len() * 8) as u32, null_mut(), &mut info) == 0 {
                    continue;
                }
                let path = std::ptr::addr_of!((*detail).DevicePath).cast::<u16>();
                let len = (0..).take_while(|&i| *path.add(i) != 0).count();
                let mut wide: Vec<u16> = std::slice::from_raw_parts(path, len).to_vec();
                wide.push(0);
                if device_number(&wide) == Some(wanted) {
                    found = Some(info.DevInst);
                    break;
                }
            }
            SetupDiDestroyDeviceInfoList(set);
            found
        }
    }

    pub fn eject_drive(letter: char) -> AppResult<()> {
        let volume: Vec<u16> = format!(r"\\.\{letter}:").encode_utf16().chain([0]).collect();
        let wanted = device_number(&volume).ok_or_else(|| failed("That drive is no longer there.", format!("{letter}:")))?;
        let disk = disk_instance(wanted).ok_or_else(|| failed("Windows does not list that drive as removable.", format!("{letter}:")))?;
        let mut parent = 0u32;
        // A card reader or stick: eject the USB device above the disk, as the tray icon does.
        let target = if unsafe { CM_Get_Parent(&mut parent, disk, 0) } == CR_SUCCESS { parent } else { disk };
        let mut last = String::new();
        // A program that just let go may take a moment; Windows' own tools retry too.
        for attempt in 0..3 {
            let mut veto = 0i32;
            let mut name = [0u16; 260];
            let r = unsafe { CM_Request_Device_EjectW(target, &mut veto, name.as_mut_ptr(), name.len() as u32, 0) };
            if r == CR_SUCCESS && veto == 0 {
                return Ok(());
            }
            let len = name.iter().position(|&c| c == 0).unwrap_or(0);
            last = String::from_utf16_lossy(&name[..len]);
            if attempt < 2 {
                std::thread::sleep(std::time::Duration::from_millis(400));
            }
        }
        Err(failed("Windows could not eject the drive: a program is still using it. Close it and try again.", last))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_os_ejects_with_its_own_tool_and_arguments_are_never_a_shell_string() {
        let (p, a) = eject_command(Os::Windows, "E:\\").unwrap();
        assert_eq!(p, "powershell");
        assert!(a.last().unwrap().contains("ParseName('E:')"));
        assert!(eject_command(Os::Windows, "E:\\'); Remove-Item C:\\ #").is_none(), "only a drive letter gets into the script");
        assert!(eject_command(Os::Windows, "C:\\Users").is_none());
        assert_eq!(eject_command(Os::Mac, "/Volumes/EOS_DIGITAL").unwrap(), ("diskutil".into(), vec!["eject".into(), "/Volumes/EOS_DIGITAL".into()]));
        assert_eq!(eject_command(Os::Linux, "/media/j/CARD").unwrap().1, ["mount", "--eject", "/media/j/CARD"]);
    }

    #[test]
    fn a_drive_holding_the_library_is_recognised() {
        assert!(holds_library(Path::new("E:\\"), Path::new("E:\\Lumen Library")));
        assert!(holds_library(Path::new("/Volumes/T7"), Path::new("/Volumes/T7/Photos/Lumen Library")));
        assert!(!holds_library(Path::new("/Volumes/T7"), Path::new("/Volumes/T70/Lumen Library")));
        assert!(!holds_library(Path::new("E:\\"), Path::new("C:\\Users\\j\\Pictures\\Lumen Library")));
    }
}
