use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::io::AsRawFd;
use std::path::{Path, PathBuf};

use anyhow::{bail, format_err, Error};
use nix::fcntl::{fcntl, FcntlArg, OFlag};

use proxmox_sys::error::SysResult;
use proxmox_sys::fs::scan_subdir;

use pbs_api_types::{DeviceKind, OptionalDeviceIdentification, TapeDeviceInfo};

lazy_static::lazy_static! {
    static ref SCSI_GENERIC_NAME_REGEX: regex::Regex =
        regex::Regex::new(r"^sg\d+$").unwrap();
}

/// List linux tape changer devices
pub fn linux_tape_changer_list() -> Vec<TapeDeviceInfo> {
    let mut list = Vec::new();

    let dir_iter = match scan_subdir(
        libc::AT_FDCWD,
        "/sys/class/scsi_generic",
        &SCSI_GENERIC_NAME_REGEX,
    ) {
        Err(_) => return list,
        Ok(iter) => iter,
    };

    for item in dir_iter {
        let item = match item {
            Err(_) => continue,
            Ok(item) => item,
        };

        let name = item.file_name().to_str().unwrap().to_string();

        let mut sys_path = PathBuf::from("/sys/class/scsi_generic");
        sys_path.push(&name);

        let device = match udev::Device::from_syspath(&sys_path) {
            Err(_) => continue,
            Ok(device) => device,
        };

        let devnum = match device.devnum() {
            None => continue,
            Some(devnum) => devnum,
        };

        let parent = match device.parent() {
            None => continue,
            Some(parent) => parent,
        };

        match parent.attribute_value("type") {
            Some(type_osstr) => {
                if type_osstr != "8" {
                    continue;
                }
            }
            _ => {
                continue;
            }
        }

        // let mut test_path = sys_path.clone();
        // test_path.push("device/scsi_changer");
        // if !test_path.exists() { continue; }

        let _dev_path = match device.devnode().map(Path::to_owned) {
            None => continue,
            Some(dev_path) => dev_path,
        };

        let serial = match device
            .property_value("ID_SCSI_SERIAL")
            .map(std::ffi::OsString::from)
            .and_then(|s| {
                if let Ok(s) = s.into_string() {
                    Some(s)
                } else {
                    None
                }
            }) {
            None => continue,
            Some(serial) => serial,
        };

        let vendor = device
            .property_value("ID_VENDOR")
            .map(std::ffi::OsString::from)
            .and_then(|s| {
                if let Ok(s) = s.into_string() {
                    Some(s)
                } else {
                    None
                }
            })
            .unwrap_or_else(|| String::from("unknown"));

        let model = device
            .property_value("ID_MODEL")
            .map(std::ffi::OsString::from)
            .and_then(|s| {
                if let Ok(s) = s.into_string() {
                    Some(s)
                } else {
                    None
                }
            })
            .unwrap_or_else(|| String::from("unknown"));

        let dev_path = format!("/dev/tape/by-id/scsi-{}", serial);

        if PathBuf::from(&dev_path).exists() {
            list.push(TapeDeviceInfo {
                kind: DeviceKind::Changer,
                path: dev_path,
                serial,
                vendor,
                model,
                major: unsafe { libc::major(devnum) },
                minor: unsafe { libc::minor(devnum) },
            });
        }
    }

    list
}

/// List LTO drives
pub fn lto_tape_device_list() -> Vec<TapeDeviceInfo> {
    let mut list = Vec::new();

    let dir_iter = match scan_subdir(
        libc::AT_FDCWD,
        "/sys/class/scsi_generic",
        &SCSI_GENERIC_NAME_REGEX,
    ) {
        Err(_) => return list,
        Ok(iter) => iter,
    };

    for item in dir_iter {
        let item = match item {
            Err(_) => continue,
            Ok(item) => item,
        };

        let name = item.file_name().to_str().unwrap().to_string();

        let mut sys_path = PathBuf::from("/sys/class/scsi_generic");
        sys_path.push(&name);

        let device = match udev::Device::from_syspath(&sys_path) {
            Err(_) => continue,
            Ok(device) => device,
        };

        let devnum = match device.devnum() {
            None => continue,
            Some(devnum) => devnum,
        };

        let parent = match device.parent() {
            None => continue,
            Some(parent) => parent,
        };

        match parent.attribute_value("type") {
            Some(type_osstr) => {
                if type_osstr != "1" {
                    continue;
                }
            }
            _ => {
                continue;
            }
        }

        // let mut test_path = sys_path.clone();
        // test_path.push("device/scsi_tape");
        // if !test_path.exists() { continue; }

        let _dev_path = match device.devnode().map(Path::to_owned) {
            None => continue,
            Some(dev_path) => dev_path,
        };

        let serial = match device
            .property_value("ID_SCSI_SERIAL")
            .map(std::ffi::OsString::from)
            .and_then(|s| {
                if let Ok(s) = s.into_string() {
                    Some(s)
                } else {
                    None
                }
            }) {
            None => continue,
            Some(serial) => serial,
        };

        let vendor = device
            .property_value("ID_VENDOR")
            .map(std::ffi::OsString::from)
            .and_then(|s| {
                if let Ok(s) = s.into_string() {
                    Some(s)
                } else {
                    None
                }
            })
            .unwrap_or_else(|| String::from("unknown"));

        let model = device
            .property_value("ID_MODEL")
            .map(std::ffi::OsString::from)
            .and_then(|s| {
                if let Ok(s) = s.into_string() {
                    Some(s)
                } else {
                    None
                }
            })
            .unwrap_or_else(|| String::from("unknown"));

        let dev_path = format!("/dev/tape/by-id/scsi-{}-sg", serial);

        if PathBuf::from(&dev_path).exists() {
            list.push(TapeDeviceInfo {
                kind: DeviceKind::Tape,
                path: dev_path,
                serial,
                vendor,
                model,
                major: unsafe { libc::major(devnum) },
                minor: unsafe { libc::minor(devnum) },
            });
        }
    }

    list
}

/// Test if a device exists, and returns associated `TapeDeviceInfo`
pub fn lookup_device<'a>(devices: &'a [TapeDeviceInfo], path: &str) -> Option<&'a TapeDeviceInfo> {
    if let Ok(stat) = nix::sys::stat::stat(path) {
        let major = unsafe { libc::major(stat.st_rdev) };
        let minor = unsafe { libc::minor(stat.st_rdev) };

        devices
            .iter()
            .find(|d| d.major == major && d.minor == minor)
    } else {
        None
    }
}

/// Lookup optional drive identification attributes
pub fn lookup_device_identification(
    devices: &[TapeDeviceInfo],
    path: &str,
) -> OptionalDeviceIdentification {
    if let Some(info) = lookup_device(devices, path) {
        OptionalDeviceIdentification {
            vendor: Some(info.vendor.clone()),
            model: Some(info.model.clone()),
            serial: Some(info.serial.clone()),
        }
    } else {
        OptionalDeviceIdentification {
            vendor: None,
            model: None,
            serial: None,
        }
    }
}

/// Make sure path is a lto tape device
pub fn check_drive_path(drives: &[TapeDeviceInfo], path: &str) -> Result<(), Error> {
    if lookup_device(drives, path).is_none() {
        bail!("path '{}' is not a lto SCSI-generic tape device", path);
    }
    Ok(())
}

/// Check for correct Major/Minor numbers
pub fn check_tape_is_lto_tape_device(file: &File) -> Result<(), Error> {
    let stat = nix::sys::stat::fstat(file.as_raw_fd())?;

    let devnum = stat.st_rdev;

    let major = unsafe { libc::major(devnum) };
    let _minor = unsafe { libc::minor(devnum) };

    if major == 9 {
        bail!("not a scsi-generic tape device (cannot use linux tape devices)");
    }

    if major != 21 {
        bail!("not a scsi-generic tape device");
    }

    Ok(())
}

/// Opens a Lto tape device
///
/// The open call use O_NONBLOCK, but that flag is cleard after open
/// succeeded. This also checks if the device is a non-rewinding tape
/// device.
pub fn open_lto_tape_device(path: &str) -> Result<File, Error> {
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .custom_flags(libc::O_NONBLOCK)
        .open(path)?;

    // clear O_NONBLOCK from now on.

    let flags = fcntl(file.as_raw_fd(), FcntlArg::F_GETFL).into_io_result()?;

    let mut flags = OFlag::from_bits_truncate(flags);
    flags.remove(OFlag::O_NONBLOCK);

    fcntl(file.as_raw_fd(), FcntlArg::F_SETFL(flags)).into_io_result()?;

    check_tape_is_lto_tape_device(&file)
        .map_err(|err| format_err!("device type check {:?} failed - {}", path, err))?;

    Ok(file)
}

// shell completion helper

/// List changer device paths
pub fn complete_changer_path(_arg: &str, _param: &HashMap<String, String>) -> Vec<String> {
    linux_tape_changer_list()
        .iter()
        .map(|v| v.path.clone())
        .collect()
}

/// List tape device paths
pub fn complete_drive_path(_arg: &str, _param: &HashMap<String, String>) -> Vec<String> {
    lto_tape_device_list()
        .iter()
        .map(|v| v.path.clone())
        .collect()
}
