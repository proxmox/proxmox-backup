//! Disk query/management utilities for.

use std::collections::{HashMap, HashSet};
use std::ffi::{OsStr, OsString};
use std::io;
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{bail, format_err, Error};
use libc::dev_t;
use once_cell::sync::OnceCell;

use ::serde::{Deserialize, Serialize};

use proxmox_lang::error::io_err_other;
use proxmox_lang::{io_bail, io_format_err};
use proxmox_rest_server::WorkerTask;
use proxmox_schema::api;
use proxmox_sys::linux::procfs::{mountinfo::Device, MountInfo};
use proxmox_sys::task_log;

use pbs_api_types::{BLOCKDEVICE_DISK_AND_PARTITION_NAME_REGEX, BLOCKDEVICE_NAME_REGEX};

mod zfs;
pub use zfs::*;
mod zpool_status;
pub use zpool_status::*;
mod zpool_list;
pub use zpool_list::*;
mod lvm;
pub use lvm::*;
mod smart;
pub use smart::*;

lazy_static::lazy_static! {
    static ref ISCSI_PATH_REGEX: regex::Regex =
        regex::Regex::new(r"host[^/]*/session[^/]*").unwrap();
}

/// Disk management context.
///
/// This provides access to disk information with some caching for faster querying of multiple
/// devices.
pub struct DiskManage {
    mount_info: OnceCell<MountInfo>,
    mounted_devices: OnceCell<HashSet<dev_t>>,
}

/// Information for a device as returned by lsblk.
#[derive(Deserialize)]
pub struct LsblkInfo {
    /// Path to the device.
    path: String,
    /// Partition type GUID.
    #[serde(rename = "parttype")]
    partition_type: Option<String>,
    /// File system label.
    #[serde(rename = "fstype")]
    file_system_type: Option<String>,
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

    /// Get a `Disk` for a name in `/sys/block/<name>`.
    pub fn disk_by_name(self: Arc<Self>, name: &str) -> io::Result<Disk> {
        let syspath = format!("/sys/block/{}", name);
        self.disk_by_sys_path(syspath)
    }

    /// Get a `Disk` for a name in `/sys/class/block/<name>`.
    pub fn partition_by_name(self: Arc<Self>, name: &str) -> io::Result<Disk> {
        let syspath = format!("/sys/class/block/{}", name);
        self.disk_by_sys_path(syspath)
    }

    /// Gather information about mounted disks:
    fn mounted_devices(&self) -> Result<&HashSet<dev_t>, Error> {
        self.mounted_devices
            .get_or_try_init(|| -> Result<_, Error> {
                let mut mounted = HashSet::new();

                for (_id, mp) in self.mount_info()? {
                    let source = match mp.mount_source.as_deref() {
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

    /// Information about file system type and used device for a path
    ///
    /// Returns tuple (fs_type, device, mount_source)
    pub fn find_mounted_device(
        &self,
        path: &std::path::Path,
    ) -> Result<Option<(String, Device, Option<OsString>)>, Error> {
        let stat = nix::sys::stat::stat(path)?;
        let device = Device::from_dev_t(stat.st_dev);

        let root_path = std::path::Path::new("/");

        for (_id, entry) in self.mount_info()? {
            if entry.root == root_path && entry.device == device {
                return Ok(Some((
                    entry.fs_type.clone(),
                    entry.device,
                    entry.mount_source.clone(),
                )));
            }
        }

        Ok(None)
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
    // for perl: #[serde(skip_serializing)]
    partition_entry_scheme: OnceCell<Option<OsString>>,
    // for perl: #[serde(skip_serializing)]
    partition_entry_uuid: OnceCell<Option<OsString>>,
    // for perl: #[serde(skip_serializing)]
    partition_entry_type: OnceCell<Option<OsString>>,
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
    pub fn read_sys_os_str<P: AsRef<Path>>(&self, path: P) -> io::Result<Option<OsString>> {
        Ok(self.read_sys(path.as_ref())?.map(|mut v| {
            if Some(&b'\n') == v.last() {
                v.pop();
            }
            OsString::from_vec(v)
        }))
    }

    /// Convenience wrapper for reading a `/sys` file which contains just a simple utf-8 string.
    pub fn read_sys_str<P: AsRef<Path>>(&self, path: P) -> io::Result<Option<String>> {
        Ok(match self.read_sys(path.as_ref())? {
            Some(data) => Some(String::from_utf8(data).map_err(io_err_other)?),
            None => None,
        })
    }

    /// Convenience wrapper for unsigned integer `/sys` values up to 64 bit.
    pub fn read_sys_u64<P: AsRef<Path>>(&self, path: P) -> io::Result<Option<u64>> {
        Ok(match self.read_sys_str(path)? {
            Some(data) => Some(data.trim().parse().map_err(io_err_other)?),
            None => None,
        })
    }

    /// Get the disk's size in bytes.
    pub fn size(&self) -> io::Result<u64> {
        Ok(*self.info.size.get_or_try_init(|| {
            self.read_sys_u64("size")?.map(|s| s * 512).ok_or_else(|| {
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

    /// Get the partitioning scheme of which this device is a partition.
    pub fn partition_entry_scheme(&self) -> Option<&OsStr> {
        self.info
            .partition_entry_scheme
            .get_or_init(|| {
                self.device
                    .property_value("ID_PART_ENTRY_SCHEME")
                    .map(|v| v.to_owned())
            })
            .as_ref()
            .map(OsString::as_os_str)
    }

    /// Check if this is a partition.
    pub fn is_partition(&self) -> bool {
        self.partition_entry_scheme().is_some()
    }

    /// Get the type of partition entry (ie. type UUID from the entry in the GPT partition table).
    pub fn partition_entry_type(&self) -> Option<&OsStr> {
        self.info
            .partition_entry_type
            .get_or_init(|| {
                self.device
                    .property_value("ID_PART_ENTRY_TYPE")
                    .map(|v| v.to_owned())
            })
            .as_ref()
            .map(OsString::as_os_str)
    }

    /// Get the partition entry UUID (ie. the UUID from the entry in the GPT partition table).
    pub fn partition_entry_uuid(&self) -> Option<&OsStr> {
        self.info
            .partition_entry_uuid
            .get_or_init(|| {
                self.device
                    .property_value("ID_PART_ENTRY_UUID")
                    .map(|v| v.to_owned())
            })
            .as_ref()
            .map(OsString::as_os_str)
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
            Some(false) => DiskType::Ssd,
            Some(true) => DiskType::Hdd,
            None => match self.ata_rotation_rate_rpm() {
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
                let mut subdir = self.syspath().to_owned();
                subdir.push("holders");
                for entry in std::fs::read_dir(subdir)? {
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

    /// Read block device stats
    ///
    /// see <https://www.kernel.org/doc/Documentation/block/stat.txt>
    pub fn read_stat(&self) -> std::io::Result<Option<BlockDevStat>> {
        if let Some(stat) = self.read_sys(Path::new("stat"))? {
            let stat = unsafe { std::str::from_utf8_unchecked(&stat) };
            let stat: Vec<u64> = stat
                .split_ascii_whitespace()
                .map(|s| s.parse().unwrap_or_default())
                .collect();

            if stat.len() < 15 {
                return Ok(None);
            }

            return Ok(Some(BlockDevStat {
                read_ios: stat[0],
                read_sectors: stat[2],
                write_ios: stat[4] + stat[11],     // write + discard
                write_sectors: stat[6] + stat[13], // write + discard
                io_ticks: stat[10],
            }));
        }
        Ok(None)
    }

    /// List device partitions
    pub fn partitions(&self) -> Result<HashMap<u64, Disk>, Error> {
        let sys_path = self.syspath();
        let device = self.sysname().to_string_lossy().to_string();

        let mut map = HashMap::new();

        for item in proxmox_sys::fs::read_subdir(libc::AT_FDCWD, sys_path)? {
            let item = item?;
            let name = match item.file_name().to_str() {
                Ok(name) => name,
                Err(_) => continue, // skip non utf8 entries
            };

            if !name.starts_with(&device) {
                continue;
            }

            let mut part_path = sys_path.to_owned();
            part_path.push(name);

            let disk_part = self.manager.clone().disk_by_sys_path(&part_path)?;

            if let Some(partition) = disk_part.read_sys_u64("partition")? {
                map.insert(partition, disk_part);
            }
        }

        Ok(map)
    }
}

#[api()]
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
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

#[derive(Debug)]
/// Represents the contents of the `/sys/block/<dev>/stat` file.
pub struct BlockDevStat {
    pub read_ios: u64,
    pub read_sectors: u64,
    pub write_ios: u64,
    pub write_sectors: u64,
    pub io_ticks: u64, // milliseconds
}

/// Use lsblk to read partition type uuids and file system types.
pub fn get_lsblk_info() -> Result<Vec<LsblkInfo>, Error> {
    let mut command = std::process::Command::new("lsblk");
    command.args(["--json", "-o", "path,parttype,fstype"]);

    let output = proxmox_sys::command::run_command(command, None)?;

    let mut output: serde_json::Value = output.parse()?;

    Ok(serde_json::from_value(output["blockdevices"].take())?)
}

/// Get set of devices with a file system label.
///
/// The set is indexed by using the unix raw device number (dev_t is u64)
fn get_file_system_devices(lsblk_info: &[LsblkInfo]) -> Result<HashSet<u64>, Error> {
    let mut device_set: HashSet<u64> = HashSet::new();

    for info in lsblk_info.iter() {
        if info.file_system_type.is_some() {
            let meta = std::fs::metadata(&info.path)?;
            device_set.insert(meta.rdev());
        }
    }

    Ok(device_set)
}

#[api()]
#[derive(Debug, Serialize, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum PartitionUsageType {
    /// Partition is not used (as far we can tell)
    Unused,
    /// Partition is used by LVM
    LVM,
    /// Partition is used by ZFS
    ZFS,
    /// Partition is ZFS reserved
    ZfsReserved,
    /// Partition is an EFI partition
    EFI,
    /// Partition is a BIOS partition
    BIOS,
    /// Partition contains a file system label
    FileSystem,
}

#[api()]
#[derive(Debug, Serialize, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum DiskUsageType {
    /// Disk is not used (as far we can tell)
    Unused,
    /// Disk is mounted
    Mounted,
    /// Disk is used by LVM
    LVM,
    /// Disk is used by ZFS
    ZFS,
    /// Disk is used by device-mapper
    DeviceMapper,
    /// Disk has partitions
    Partitions,
    /// Disk contains a file system label
    FileSystem,
}

#[api()]
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
/// Basic information about a partition
pub struct PartitionInfo {
    /// The partition name
    pub name: String,
    /// What the partition is used for
    pub used: PartitionUsageType,
    /// Is the partition mounted
    pub mounted: bool,
    /// The filesystem of the partition
    pub filesystem: Option<String>,
    /// The partition devpath
    pub devpath: Option<String>,
    /// Size in bytes
    pub size: Option<u64>,
    /// GPT partition
    pub gpt: bool,
}

#[api(
    properties: {
        used: {
            type: DiskUsageType,
        },
        "disk-type": {
            type: DiskType,
        },
        status: {
            type: SmartStatus,
        },
        partitions: {
            optional: true,
            items: {
                type: PartitionInfo
            }
        }
    }
)]
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
/// Information about how a Disk is used
pub struct DiskUsageInfo {
    /// Disk name (`/sys/block/<name>`)
    pub name: String,
    pub used: DiskUsageType,
    pub disk_type: DiskType,
    pub status: SmartStatus,
    /// Disk wearout
    pub wearout: Option<f64>,
    /// Vendor
    pub vendor: Option<String>,
    /// Model
    pub model: Option<String>,
    /// WWN
    pub wwn: Option<String>,
    /// Disk size
    pub size: u64,
    /// Serisal number
    pub serial: Option<String>,
    /// Partitions on the device
    pub partitions: Option<Vec<PartitionInfo>>,
    /// Linux device path (/dev/xxx)
    pub devpath: Option<String>,
    /// Set if disk contains a GPT partition table
    pub gpt: bool,
    /// RPM
    pub rpm: Option<u64>,
}

fn scan_partitions(
    disk_manager: Arc<DiskManage>,
    lvm_devices: &HashSet<u64>,
    zfs_devices: &HashSet<u64>,
    device: &str,
) -> Result<DiskUsageType, Error> {
    let mut sys_path = std::path::PathBuf::from("/sys/block");
    sys_path.push(device);

    let mut used = DiskUsageType::Unused;

    let mut found_lvm = false;
    let mut found_zfs = false;
    let mut found_mountpoints = false;
    let mut found_dm = false;
    let mut found_partitions = false;

    for item in proxmox_sys::fs::read_subdir(libc::AT_FDCWD, &sys_path)? {
        let item = item?;
        let name = match item.file_name().to_str() {
            Ok(name) => name,
            Err(_) => continue, // skip non utf8 entries
        };
        if !name.starts_with(device) {
            continue;
        }

        found_partitions = true;

        let mut part_path = sys_path.clone();
        part_path.push(name);

        let data = disk_manager.clone().disk_by_sys_path(&part_path)?;

        let devnum = data.devnum()?;

        if lvm_devices.contains(&devnum) {
            found_lvm = true;
        }

        if data.is_mounted()? {
            found_mountpoints = true;
        }

        if data.has_holders()? {
            found_dm = true;
        }

        if zfs_devices.contains(&devnum) {
            found_zfs = true;
        }
    }

    if found_mountpoints {
        used = DiskUsageType::Mounted;
    } else if found_lvm {
        used = DiskUsageType::LVM;
    } else if found_zfs {
        used = DiskUsageType::ZFS;
    } else if found_dm {
        used = DiskUsageType::DeviceMapper;
    } else if found_partitions {
        used = DiskUsageType::Partitions;
    }

    Ok(used)
}

pub struct DiskUsageQuery {
    smart: bool,
    partitions: bool,
}

impl DiskUsageQuery {
    pub const fn new() -> Self {
        Self {
            smart: true,
            partitions: false,
        }
    }

    pub fn smart(&mut self, smart: bool) -> &mut Self {
        self.smart = smart;
        self
    }

    pub fn partitions(&mut self, partitions: bool) -> &mut Self {
        self.partitions = partitions;
        self
    }

    pub fn query(&self) -> Result<HashMap<String, DiskUsageInfo>, Error> {
        get_disks(None, !self.smart, self.partitions)
    }

    pub fn find(&self, disk: &str) -> Result<DiskUsageInfo, Error> {
        let mut map = get_disks(Some(vec![disk.to_string()]), !self.smart, self.partitions)?;
        if let Some(info) = map.remove(disk) {
            Ok(info)
        } else {
            bail!("failed to get disk usage info - internal error"); // should not happen
        }
    }

    pub fn find_all(&self, disks: Vec<String>) -> Result<HashMap<String, DiskUsageInfo>, Error> {
        get_disks(Some(disks), !self.smart, self.partitions)
    }
}

fn get_partitions_info(
    partitions: HashMap<u64, Disk>,
    lvm_devices: &HashSet<u64>,
    zfs_devices: &HashSet<u64>,
    file_system_devices: &HashSet<u64>,
) -> Vec<PartitionInfo> {
    let lsblk_infos = get_lsblk_info().ok();
    partitions
        .values()
        .map(|disk| {
            let devpath = disk
                .device_path()
                .map(|p| p.to_owned())
                .map(|p| p.to_string_lossy().to_string());

            let mut used = PartitionUsageType::Unused;

            if let Ok(devnum) = disk.devnum() {
                if lvm_devices.contains(&devnum) {
                    used = PartitionUsageType::LVM;
                } else if zfs_devices.contains(&devnum) {
                    used = PartitionUsageType::ZFS;
                } else if file_system_devices.contains(&devnum) {
                    used = PartitionUsageType::FileSystem;
                }
            }

            let mounted = disk.is_mounted().unwrap_or(false);
            let mut filesystem = None;
            if let (Some(devpath), Some(infos)) = (devpath.as_ref(), lsblk_infos.as_ref()) {
                for info in infos.iter().filter(|i| i.path.eq(devpath)) {
                    used = match info.partition_type.as_deref() {
                        Some("21686148-6449-6e6f-744e-656564454649") => PartitionUsageType::BIOS,
                        Some("c12a7328-f81f-11d2-ba4b-00a0c93ec93b") => PartitionUsageType::EFI,
                        Some("6a945a3b-1dd2-11b2-99a6-080020736631") => {
                            PartitionUsageType::ZfsReserved
                        }
                        _ => used,
                    };
                    if used == PartitionUsageType::FileSystem {
                        filesystem = info.file_system_type.clone();
                    }
                }
            }

            PartitionInfo {
                name: disk.sysname().to_str().unwrap_or("?").to_string(),
                devpath,
                used,
                mounted,
                filesystem,
                size: disk.size().ok(),
                gpt: disk.has_gpt(),
            }
        })
        .collect()
}

/// Get disk usage information for multiple disks
fn get_disks(
    // filter - list of device names (without leading /dev)
    disks: Option<Vec<String>>,
    // do no include data from smartctl
    no_smart: bool,
    // include partitions
    include_partitions: bool,
) -> Result<HashMap<String, DiskUsageInfo>, Error> {
    let disk_manager = DiskManage::new();

    let lsblk_info = get_lsblk_info()?;

    let zfs_devices =
        zfs_devices(&lsblk_info, None).or_else(|err| -> Result<HashSet<u64>, Error> {
            eprintln!("error getting zfs devices: {}", err);
            Ok(HashSet::new())
        })?;

    let lvm_devices = get_lvm_devices(&lsblk_info)?;

    let file_system_devices = get_file_system_devices(&lsblk_info)?;

    // fixme: ceph journals/volumes

    let mut result = HashMap::new();

    for item in proxmox_sys::fs::scan_subdir(libc::AT_FDCWD, "/sys/block", &BLOCKDEVICE_NAME_REGEX)?
    {
        let item = item?;

        let name = item.file_name().to_str().unwrap().to_string();

        if let Some(ref disks) = disks {
            if !disks.contains(&name) {
                continue;
            }
        }

        let sys_path = format!("/sys/block/{}", name);

        if let Ok(target) = std::fs::read_link(&sys_path) {
            if let Some(target) = target.to_str() {
                if ISCSI_PATH_REGEX.is_match(target) {
                    continue;
                } // skip iSCSI devices
            }
        }

        let disk = disk_manager.clone().disk_by_sys_path(&sys_path)?;

        let devnum = disk.devnum()?;

        let size = match disk.size() {
            Ok(size) => size,
            Err(_) => continue, // skip devices with unreadable size
        };

        let disk_type = match disk.guess_disk_type() {
            Ok(disk_type) => disk_type,
            Err(_) => continue, // skip devices with undetectable type
        };

        let mut usage = DiskUsageType::Unused;

        if lvm_devices.contains(&devnum) {
            usage = DiskUsageType::LVM;
        }

        match disk.is_mounted() {
            Ok(true) => usage = DiskUsageType::Mounted,
            Ok(false) => {}
            Err(_) => continue, // skip devices with undetectable mount status
        }

        if zfs_devices.contains(&devnum) {
            usage = DiskUsageType::ZFS;
        }

        let vendor = disk
            .vendor()
            .unwrap_or(None)
            .map(|s| s.to_string_lossy().trim().to_string());

        let model = disk.model().map(|s| s.to_string_lossy().into_owned());

        let serial = disk.serial().map(|s| s.to_string_lossy().into_owned());

        let devpath = disk
            .device_path()
            .map(|p| p.to_owned())
            .map(|p| p.to_string_lossy().to_string());

        let wwn = disk.wwn().map(|s| s.to_string_lossy().into_owned());

        let partitions: Option<Vec<PartitionInfo>> = if include_partitions {
            disk.partitions().map_or(None, |parts| {
                Some(get_partitions_info(
                    parts,
                    &lvm_devices,
                    &zfs_devices,
                    &file_system_devices,
                ))
            })
        } else {
            None
        };

        if usage != DiskUsageType::Mounted {
            match scan_partitions(disk_manager.clone(), &lvm_devices, &zfs_devices, &name) {
                Ok(part_usage) => {
                    if part_usage != DiskUsageType::Unused {
                        usage = part_usage;
                    }
                }
                Err(_) => continue, // skip devices if scan_partitions fail
            };
        }

        if usage == DiskUsageType::Unused && file_system_devices.contains(&devnum) {
            usage = DiskUsageType::FileSystem;
        }

        if usage == DiskUsageType::Unused && disk.has_holders()? {
            usage = DiskUsageType::DeviceMapper;
        }

        let mut status = SmartStatus::Unknown;
        let mut wearout = None;

        if !no_smart {
            if let Ok(smart) = get_smart_data(&disk, false) {
                status = smart.status;
                wearout = smart.wearout;
            }
        }

        let info = DiskUsageInfo {
            name: name.clone(),
            vendor,
            model,
            partitions,
            serial,
            devpath,
            size,
            wwn,
            disk_type,
            status,
            wearout,
            used: usage,
            gpt: disk.has_gpt(),
            rpm: disk.ata_rotation_rate_rpm(),
        };

        result.insert(name, info);
    }

    Ok(result)
}

/// Try to reload the partition table
pub fn reread_partition_table(disk: &Disk) -> Result<(), Error> {
    let disk_path = match disk.device_path() {
        Some(path) => path,
        None => bail!("disk {:?} has no node in /dev", disk.syspath()),
    };

    let mut command = std::process::Command::new("blockdev");
    command.arg("--rereadpt");
    command.arg(disk_path);

    proxmox_sys::command::run_command(command, None)?;

    Ok(())
}

/// Initialize disk by writing a GPT partition table
pub fn inititialize_gpt_disk(disk: &Disk, uuid: Option<&str>) -> Result<(), Error> {
    let disk_path = match disk.device_path() {
        Some(path) => path,
        None => bail!("disk {:?} has no node in /dev", disk.syspath()),
    };

    let uuid = uuid.unwrap_or("R"); // R .. random disk GUID

    let mut command = std::process::Command::new("sgdisk");
    command.arg(disk_path);
    command.args(["-U", uuid]);

    proxmox_sys::command::run_command(command, None)?;

    Ok(())
}

/// Wipes all labels and the first 200 MiB of a disk/partition (or the whole if it is smaller).
/// If called with a partition, also sets the partition type to 0x83 'Linux filesystem'.
pub fn wipe_blockdev(disk: &Disk, worker: Arc<WorkerTask>) -> Result<(), Error> {
    let disk_path = match disk.device_path() {
        Some(path) => path,
        None => bail!("disk {:?} has no node in /dev", disk.syspath()),
    };

    let is_partition = disk.is_partition();

    let mut to_wipe: Vec<PathBuf> = Vec::new();

    let partitions_map = disk.partitions()?;
    for part_disk in partitions_map.values() {
        let part_path = match part_disk.device_path() {
            Some(path) => path,
            None => bail!("disk {:?} has no node in /dev", part_disk.syspath()),
        };
        to_wipe.push(part_path.to_path_buf());
    }

    to_wipe.push(disk_path.to_path_buf());

    task_log!(worker, "Wiping block device {}", disk_path.display());

    let mut wipefs_command = std::process::Command::new("wipefs");
    wipefs_command.arg("--all").args(&to_wipe);

    let wipefs_output = proxmox_sys::command::run_command(wipefs_command, None)?;
    task_log!(worker, "wipefs output: {}", wipefs_output);

    let size = disk.size().map(|size| size / 1024 / 1024)?;
    let count = size.min(200);

    let mut dd_command = std::process::Command::new("dd");
    let mut of_path = OsString::from("of=");
    of_path.push(disk_path);
    let mut count_str = OsString::from("count=");
    count_str.push(count.to_string());
    let args = [
        "if=/dev/zero".into(),
        of_path,
        "bs=1M".into(),
        "conv=fdatasync".into(),
        count_str.into(),
    ];
    dd_command.args(args);

    let dd_output = proxmox_sys::command::run_command(dd_command, None)?;
    task_log!(worker, "dd output: {}", dd_output);

    if is_partition {
        // set the partition type to 0x83 'Linux filesystem'
        change_parttype(&disk, "8300", worker)?;
    }

    Ok(())
}

pub fn change_parttype(
    part_disk: &Disk,
    part_type: &str,
    worker: Arc<WorkerTask>,
) -> Result<(), Error> {
    let part_path = match part_disk.device_path() {
        Some(path) => path,
        None => bail!("disk {:?} has no node in /dev", part_disk.syspath()),
    };
    if let Ok(stat) = nix::sys::stat::stat(part_path) {
        let mut sgdisk_command = std::process::Command::new("sgdisk");
        let major = unsafe { libc::major(stat.st_rdev) };
        let minor = unsafe { libc::minor(stat.st_rdev) };
        let partnum_path = &format!("/sys/dev/block/{}:{}/partition", major, minor);
        let partnum: u32 = std::fs::read_to_string(partnum_path)?.trim_end().parse()?;
        sgdisk_command.arg(&format!("-t{}:{}", partnum, part_type));
        let part_disk_parent = match part_disk.parent() {
            Some(disk) => disk,
            None => bail!("disk {:?} has no node in /dev", part_disk.syspath()),
        };
        let part_disk_parent_path = match part_disk_parent.device_path() {
            Some(path) => path,
            None => bail!("disk {:?} has no node in /dev", part_disk.syspath()),
        };
        sgdisk_command.arg(part_disk_parent_path);
        let sgdisk_output = proxmox_sys::command::run_command(sgdisk_command, None)?;
        task_log!(worker, "sgdisk output: {}", sgdisk_output);
    }
    Ok(())
}

/// Create a single linux partition using the whole available space
pub fn create_single_linux_partition(disk: &Disk) -> Result<Disk, Error> {
    let disk_path = match disk.device_path() {
        Some(path) => path,
        None => bail!("disk {:?} has no node in /dev", disk.syspath()),
    };

    let mut command = std::process::Command::new("sgdisk");
    command.args(["-n1", "-t1:8300"]);
    command.arg(disk_path);

    proxmox_sys::command::run_command(command, None)?;

    let mut partitions = disk.partitions()?;

    match partitions.remove(&1) {
        Some(partition) => Ok(partition),
        None => bail!("unable to lookup device partition"),
    }
}

#[api()]
#[derive(Debug, Copy, Clone, Serialize, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum FileSystemType {
    /// Linux Ext4
    Ext4,
    /// XFS
    Xfs,
}

impl std::fmt::Display for FileSystemType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let text = match self {
            FileSystemType::Ext4 => "ext4",
            FileSystemType::Xfs => "xfs",
        };
        write!(f, "{}", text)
    }
}

impl std::str::FromStr for FileSystemType {
    type Err = serde_json::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        use serde::de::IntoDeserializer;
        Self::deserialize(s.into_deserializer())
    }
}

/// Create a file system on a disk or disk partition
pub fn create_file_system(disk: &Disk, fs_type: FileSystemType) -> Result<(), Error> {
    let disk_path = match disk.device_path() {
        Some(path) => path,
        None => bail!("disk {:?} has no node in /dev", disk.syspath()),
    };

    let fs_type = fs_type.to_string();

    let mut command = std::process::Command::new("mkfs");
    command.args(["-t", &fs_type]);
    command.arg(disk_path);

    proxmox_sys::command::run_command(command, None)?;

    Ok(())
}
/// Block device name completion helper
pub fn complete_disk_name(_arg: &str, _param: &HashMap<String, String>) -> Vec<String> {
    let dir =
        match proxmox_sys::fs::scan_subdir(libc::AT_FDCWD, "/sys/block", &BLOCKDEVICE_NAME_REGEX) {
            Ok(dir) => dir,
            Err(_) => return vec![],
        };

    dir.flatten()
        .map(|item| item.file_name().to_str().unwrap().to_string())
        .collect()
}

/// Block device partition name completion helper
pub fn complete_partition_name(_arg: &str, _param: &HashMap<String, String>) -> Vec<String> {
    let dir = match proxmox_sys::fs::scan_subdir(
        libc::AT_FDCWD,
        "/sys/class/block",
        &BLOCKDEVICE_DISK_AND_PARTITION_NAME_REGEX,
    ) {
        Ok(dir) => dir,
        Err(_) => return vec![],
    };

    dir.flatten()
        .map(|item| item.file_name().to_str().unwrap().to_string())
        .collect()
}

/// Read the FS UUID (parse blkid output)
///
/// Note: Calling blkid is more reliable than using the udev ID_FS_UUID property.
pub fn get_fs_uuid(disk: &Disk) -> Result<String, Error> {
    let disk_path = match disk.device_path() {
        Some(path) => path,
        None => bail!("disk {:?} has no node in /dev", disk.syspath()),
    };

    let mut command = std::process::Command::new("blkid");
    command.args(["-o", "export"]);
    command.arg(disk_path);

    let output = proxmox_sys::command::run_command(command, None)?;

    for line in output.lines() {
        if let Some(uuid) = line.strip_prefix("UUID=") {
            return Ok(uuid.to_string());
        }
    }

    bail!("get_fs_uuid failed - missing UUID");
}
