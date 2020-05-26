//! Disk query/management utilities for.

use std::collections::HashSet;
use std::ffi::{OsStr, OsString};
use std::io;
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use bitflags::bitflags;
use anyhow::{format_err, Error};
use libc::dev_t;
use once_cell::sync::OnceCell;

use proxmox::sys::error::io_err_other;
use proxmox::sys::linux::procfs::MountInfo;
use proxmox::{io_bail, io_format_err};

bitflags! {
    /// Ways a device is being used.
    pub struct DiskUse: u32 {
        /// Currently mounted.
        const MOUNTED = 0x0000_0001;

        /// Currently used as member of a device-mapper device.
        const DEVICE_MAPPER = 0x0000_0002;

        /// Contains partitions.
        const PARTITIONS = 0x0001_0000;

        /// The disk has a partition type which belongs to an LVM PV.
        const LVM = 0x0002_0000;

        /// The disk has a partition type which belongs to a zpool.
        const ZFS = 0x0004_0000;

        /// The disk is used by ceph.
        const CEPH = 0x0008_0000;
    }
}

/// Disk management context.
///
/// This provides access to disk information with some caching for faster querying of multiple
/// devices.
pub struct DiskManage {
    mount_info: OnceCell<MountInfo>,
    mounted_devices: OnceCell<HashSet<dev_t>>,
}

impl DiskManage {
    /// Create a new disk management context.
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            mount_info: OnceCell::new(),
            mounted_devices: OnceCell::new(),
        })
    }

    /// Get the current mount info. This simply caches the result of `MountInfo::read` from the
    /// `proxmox::sys` module.
    pub fn mount_info(&self) -> Result<&MountInfo, Error> {
        self.mount_info.get_or_try_init(MountInfo::read)
    }

    /// Get a `Disk` from a device node (eg. `/dev/sda`).
    pub fn disk_by_node<P: AsRef<Path>>(self: Arc<Self>, devnode: P) -> io::Result<Disk> {
        use std::os::unix::fs::MetadataExt;

        let devnode = devnode.as_ref();

        let meta = std::fs::metadata(devnode)?;
        if (meta.mode() & libc::S_IFBLK) == libc::S_IFBLK {
            self.disk_by_dev_num(meta.rdev())
        } else {
            io_bail!("not a block device: {:?}", devnode);
        }
    }

    /// Get a `Disk` for a specific device number.
    pub fn disk_by_dev_num(self: Arc<Self>, devnum: dev_t) -> io::Result<Disk> {
        self.disk_by_sys_path(format!(
            "/sys/dev/block/{}:{}",
            unsafe { libc::major(devnum) },
            unsafe { libc::minor(devnum) },
        ))
    }

    /// Get a `Disk` for a path in `/sys`.
    pub fn disk_by_sys_path<P: AsRef<Path>>(self: Arc<Self>, path: P) -> io::Result<Disk> {
        let device = udev::Device::from_syspath(path.as_ref())?;
        Ok(Disk {
            manager: self,
            device,
            info: Default::default(),
        })
    }

    /// Gather information about mounted disks:
    fn mounted_devices(&self) -> Result<&HashSet<dev_t>, Error> {
        use std::os::unix::fs::MetadataExt;

        self.mounted_devices
            .get_or_try_init(|| -> Result<_, Error> {
                let mut mounted = HashSet::new();

                for (_id, mp) in self.mount_info()? {
                    let source = match mp.mount_source.as_ref().map(OsString::as_os_str) {
                        Some(s) => s,
                        None => continue,
                    };

                    let path = Path::new(source);
                    if !path.is_absolute() {
                        continue;
                    }

                    let meta = match std::fs::metadata(path) {
                        Ok(meta) => meta,
                        Err(ref err) if err.kind() == io::ErrorKind::NotFound => continue,
                        Err(other) => return Err(Error::from(other)),
                    };

                    if (meta.mode() & libc::S_IFBLK) != libc::S_IFBLK {
                        // not a block device
                        continue;
                    }

                    mounted.insert(meta.rdev());
                }

                Ok(mounted)
            })
    }

    /// Check whether a specific device node is mounted.
    ///
    /// Note that this tries to `stat` the sources of all mount points without caching the result
    /// of doing so, so this is always somewhat expensive.
    pub fn is_devnum_mounted(&self, dev: dev_t) -> Result<bool, Error> {
        self.mounted_devices().map(|mounted| mounted.contains(&dev))
    }
}

/// Queries (and caches) various information about a specific disk.
///
/// This belongs to a `Disks` and provides information for a single disk.
pub struct Disk {
    manager: Arc<DiskManage>,
    device: udev::Device,
    info: DiskInfo,
}

/// Helper struct (so we can initialize this with Default)
///
/// We probably want this to be serializable to the same hash type we use in perl currently.
#[derive(Default)]
struct DiskInfo {
    size: OnceCell<u64>,
    vendor: OnceCell<Option<OsString>>,
    model: OnceCell<Option<OsString>>,
    rotational: OnceCell<Option<bool>>,
    // for perl: #[serde(rename = "devpath")]
    ata_rotation_rate_rpm: OnceCell<Option<u64>>,
    // for perl: #[serde(rename = "devpath")]
    device_path: OnceCell<Option<PathBuf>>,
    wwn: OnceCell<Option<OsString>>,
    serial: OnceCell<Option<OsString>>,
    // for perl: #[serde(skip_serializing)]
    partition_table_type: OnceCell<Option<OsString>>,
    gpt: OnceCell<bool>,
    // ???
    bus: OnceCell<Option<OsString>>,
    // ???
    fs_type: OnceCell<Option<OsString>>,
    // ???
    has_holders: OnceCell<bool>,
    // ???
    is_mounted: OnceCell<bool>,
}

impl Disk {
    /// Try to get the device number for this disk.
    ///
    /// (In udev this can fail...)
    pub fn devnum(&self) -> Result<dev_t, Error> {
        // not sure when this can fail...
        self.device
            .devnum()
            .ok_or_else(|| format_err!("failed to get device number"))
    }

    /// Get the sys-name of this device. (The final component in the `/sys` path).
    pub fn sysname(&self) -> &OsStr {
        self.device.sysname()
    }

    /// Get the this disk's `/sys` path.
    pub fn syspath(&self) -> &Path {
        self.device.syspath()
    }

    /// Get the device node in `/dev`, if any.
    pub fn device_path(&self) -> Option<&Path> {
        //self.device.devnode()
        self.info
            .device_path
            .get_or_init(|| self.device.devnode().map(Path::to_owned))
            .as_ref()
            .map(PathBuf::as_path)
    }

    /// Get the parent device.
    pub fn parent(&self) -> Option<Self> {
        self.device.parent().map(|parent| Self {
            manager: self.manager.clone(),
            device: parent,
            info: Default::default(),
        })
    }

    /// Read from a file in this device's sys path.
    ///
    /// Note: path must be a relative path!
    pub fn read_sys(&self, path: &Path) -> io::Result<Option<Vec<u8>>> {
        assert!(path.is_relative());

        std::fs::read(self.syspath().join(path))
            .map(Some)
            .or_else(|err| {
                if err.kind() == io::ErrorKind::NotFound {
                    Ok(None)
                } else {
                    Err(err)
                }
            })
    }

    /// Convenience wrapper for reading a `/sys` file which contains just a simple `OsString`.
    fn read_sys_os_str<P: AsRef<Path>>(&self, path: P) -> io::Result<Option<OsString>> {
        Ok(self.read_sys(path.as_ref())?.map(OsString::from_vec))
    }

    /// Convenience wrapper for reading a `/sys` file which contains just a simple utf-8 string.
    fn read_sys_str<P: AsRef<Path>>(&self, path: P) -> io::Result<Option<String>> {
        Ok(match self.read_sys(path.as_ref())? {
            Some(data) => Some(String::from_utf8(data).map_err(io_err_other)?),
            None => None,
        })
    }

    /// Convenience wrapper for unsigned integer `/sys` values up to 64 bit.
    fn read_sys_u64<P: AsRef<Path>>(&self, path: P) -> io::Result<Option<u64>> {
        Ok(match self.read_sys_str(path)? {
            Some(data) => Some(data.trim().parse().map_err(io_err_other)?),
            None => None,
        })
    }

    /// Get the disk's size in bytes.
    pub fn size(&self) -> io::Result<u64> {
        Ok(*self.info.size.get_or_try_init(|| {
            self.read_sys_u64("size")?.ok_or_else(|| {
                io_format_err!(
                    "failed to get disk size from {:?}",
                    self.syspath().join("size"),
                )
            })
        })?)
    }

    /// Get the device vendor (`/sys/.../device/vendor`) entry if available.
    pub fn vendor(&self) -> io::Result<Option<&OsStr>> {
        Ok(self
            .info
            .vendor
            .get_or_try_init(|| self.read_sys_os_str("device/vendor"))?
            .as_ref()
            .map(OsString::as_os_str))
    }

    /// Get the device model (`/sys/.../device/model`) entry if available.
    pub fn model(&self) -> Option<&OsStr> {
        self.info
            .model
            .get_or_init(|| self.device.property_value("ID_MODEL").map(OsStr::to_owned))
            .as_ref()
            .map(OsString::as_os_str)
    }

    /// Check whether this is a rotational disk.
    ///
    /// Returns `None` if there's no `queue/rotational` file, in which case no information is
    /// known. `Some(false)` if `queue/rotational` is zero, `Some(true)` if it has a non-zero
    /// value.
    pub fn rotational(&self) -> io::Result<Option<bool>> {
        Ok(*self
            .info
            .rotational
            .get_or_try_init(|| -> io::Result<Option<bool>> {
                Ok(self.read_sys_u64("queue/rotational")?.map(|n| n != 0))
            })?)
    }

    /// Get the WWN if available.
    pub fn wwn(&self) -> Option<&OsStr> {
        self.info
            .wwn
            .get_or_init(|| self.device.property_value("ID_WWN").map(|v| v.to_owned()))
            .as_ref()
            .map(OsString::as_os_str)
    }

    /// Get the device serial if available.
    pub fn serial(&self) -> Option<&OsStr> {
        self.info
            .serial
            .get_or_init(|| {
                self.device
                    .property_value("ID_SERIAL_SHORT")
                    .map(|v| v.to_owned())
            })
            .as_ref()
            .map(OsString::as_os_str)
    }

    /// Get the ATA rotation rate value from udev. This is not necessarily the same as sysfs'
    /// `rotational` value.
    pub fn ata_rotation_rate_rpm(&self) -> Option<u64> {
        *self.info.ata_rotation_rate_rpm.get_or_init(|| {
            std::str::from_utf8(
                self.device
                    .property_value("ID_ATA_ROTATION_RATE_RPM")?
                    .as_bytes(),
            )
            .ok()?
            .parse()
            .ok()
        })
    }

    /// Get the partition table type, if any.
    pub fn partition_table_type(&self) -> Option<&OsStr> {
        self.info
            .partition_table_type
            .get_or_init(|| {
                self.device
                    .property_value("ID_PART_TABLE_TYPE")
                    .map(|v| v.to_owned())
            })
            .as_ref()
            .map(OsString::as_os_str)
    }

    /// Check if this contains a GPT partition table.
    pub fn has_gpt(&self) -> bool {
        *self.info.gpt.get_or_init(|| {
            self.partition_table_type()
                .map(|s| s == "gpt")
                .unwrap_or(false)
        })
    }

    /// Get the bus type used for this disk.
    pub fn bus(&self) -> Option<&OsStr> {
        self.info
            .bus
            .get_or_init(|| self.device.property_value("ID_BUS").map(|v| v.to_owned()))
            .as_ref()
            .map(OsString::as_os_str)
    }

    /// Attempt to guess the disk type.
    pub fn guess_disk_type(&self) -> io::Result<DiskType> {
        Ok(match self.rotational()? {
            Some(true) => DiskType::Hdd,
            _ => match self.ata_rotation_rate_rpm() {
                Some(_) => DiskType::Hdd,
                None => match self.bus() {
                    Some(bus) if bus == "usb" => DiskType::Usb,
                    _ => DiskType::Unknown,
                },
            },
        })
    }

    /// Get the file system type found on the disk, if any.
    ///
    /// Note that `None` may also just mean "unknown".
    pub fn fs_type(&self) -> Option<&OsStr> {
        self.info
            .fs_type
            .get_or_init(|| {
                self.device
                    .property_value("ID_FS_TYPE")
                    .map(|v| v.to_owned())
            })
            .as_ref()
            .map(OsString::as_os_str)
    }

    /// Check if there are any "holders" in `/sys`. This usually means the device is in use by
    /// another kernel driver like the device mapper.
    pub fn has_holders(&self) -> io::Result<bool> {
        Ok(*self
            .info
            .has_holders
            .get_or_try_init(|| -> io::Result<bool> {
                for entry in std::fs::read_dir(self.syspath())? {
                    match entry?.file_name().as_bytes() {
                        b"." | b".." => (),
                        _ => return Ok(true),
                    }
                }
                Ok(false)
            })?)
    }

    /// Check if this disk is mounted.
    pub fn is_mounted(&self) -> Result<bool, Error> {
        Ok(*self
            .info
            .is_mounted
            .get_or_try_init(|| self.manager.is_devnum_mounted(self.devnum()?))?)
    }
}

/// This is just a rough estimate for a "type" of disk.
pub enum DiskType {
    /// We know nothing.
    Unknown,

    /// May also be a USB-HDD.
    Hdd,

    /// May also be a USB-SSD.
    Ssd,

    /// Some kind of USB disk, but we don't know more than that.
    Usb,
}
