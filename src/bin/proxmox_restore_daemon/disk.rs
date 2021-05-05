//! Low-level disk (image) access functions for file restore VMs.
use anyhow::{bail, format_err, Error};
use lazy_static::lazy_static;
use log::{info, warn};

use std::collections::HashMap;
use std::fs::{create_dir_all, File};
use std::io::{BufRead, BufReader};
use std::path::{Component, Path, PathBuf};

use proxmox::const_regex;
use proxmox::tools::fs;
use proxmox_backup::api2::types::BLOCKDEVICE_NAME_REGEX;

const_regex! {
    VIRTIO_PART_REGEX = r"^vd[a-z]+(\d+)$";
}

lazy_static! {
    static ref FS_OPT_MAP: HashMap<&'static str, &'static str> = {
        let mut m = HashMap::new();

        // otherwise ext complains about mounting read-only
        m.insert("ext2", "noload");
        m.insert("ext3", "noload");
        m.insert("ext4", "noload");

        m.insert("xfs", "norecovery");

        // ufs2 is used as default since FreeBSD 5.0 released in 2003, so let's assume that
        // whatever the user is trying to restore is not using anything older...
        m.insert("ufs", "ufstype=ufs2");

        m.insert("ntfs", "utf8");

        m
    };
}

pub enum ResolveResult {
    Path(PathBuf),
    BucketTypes(Vec<&'static str>),
    BucketComponents(Vec<(String, u64)>),
}

struct PartitionBucketData {
    dev_node: String,
    number: i32,
    mountpoint: Option<PathBuf>,
    size: u64,
}

/// A "Bucket" represents a mapping found on a disk, e.g. a partition, a zfs dataset or an LV. A
/// uniquely identifying path to a file then consists of four components:
/// "/disk/bucket/component/path"
/// where
///   disk: fidx file name
///   bucket: bucket type
///   component: identifier of the specific bucket
///   path: relative path of the file on the filesystem indicated by the other parts, may contain
///         more subdirectories
/// e.g.: "/drive-scsi0/part/0/etc/passwd"
enum Bucket {
    Partition(PartitionBucketData),
}

impl Bucket {
    fn filter_mut<'a, A: AsRef<str>, B: AsRef<str>>(
        haystack: &'a mut Vec<Bucket>,
        ty: A,
        comp: B,
    ) -> Option<&'a mut Bucket> {
        let ty = ty.as_ref();
        let comp = comp.as_ref();
        haystack.iter_mut().find(|b| match b {
            Bucket::Partition(data) => ty == "part" && comp.parse::<i32>().unwrap() == data.number,
        })
    }

    fn type_string(&self) -> &'static str {
        match self {
            Bucket::Partition(_) => "part",
        }
    }

    fn component_string(&self) -> String {
        match self {
            Bucket::Partition(data) => data.number.to_string(),
        }
    }

    fn size(&self) -> u64 {
        match self {
            Bucket::Partition(data) => data.size,
        }
    }
}

/// Functions related to the local filesystem. This mostly exists so we can use 'supported_fs' in
/// try_mount while a Bucket is still mutably borrowed from DiskState.
struct Filesystems {
    supported_fs: Vec<String>,
}

impl Filesystems {
    fn scan() -> Result<Self, Error> {
        // detect kernel supported filesystems
        let mut supported_fs = Vec::new();
        for f in BufReader::new(File::open("/proc/filesystems")?)
            .lines()
            .filter_map(Result::ok)
        {
            // ZFS is treated specially, don't attempt to do a regular mount with it
            let f = f.trim();
            if !f.starts_with("nodev") && f != "zfs" {
                supported_fs.push(f.to_owned());
            }
        }

        info!("Supported FS: {}", supported_fs.join(", "));

        Ok(Self { supported_fs })
    }

    fn ensure_mounted(&self, bucket: &mut Bucket) -> Result<PathBuf, Error> {
        match bucket {
            Bucket::Partition(data) => {
                // regular data partition à la "/dev/vdxN"
                if let Some(mp) = &data.mountpoint {
                    return Ok(mp.clone());
                }

                let mp = format!("/mnt{}/", data.dev_node);
                self.try_mount(&data.dev_node, &mp)?;
                let mp = PathBuf::from(mp);
                data.mountpoint = Some(mp.clone());
                Ok(mp)
            }
        }
    }

    fn try_mount(&self, source: &str, target: &str) -> Result<(), Error> {
        use nix::mount::*;

        create_dir_all(target)?;

        // try all supported fs until one works - this is the way Busybox's 'mount' does it too:
        // https://git.busybox.net/busybox/tree/util-linux/mount.c?id=808d93c0eca49e0b22056e23d965f0d967433fbb#n2152
        // note that ZFS is intentionally left out (see scan())
        let flags =
            MsFlags::MS_RDONLY | MsFlags::MS_NOEXEC | MsFlags::MS_NOSUID | MsFlags::MS_NODEV;
        for fs in &self.supported_fs {
            let fs: &str = fs.as_ref();
            let opts = FS_OPT_MAP.get(fs).copied();
            match mount(Some(source), target, Some(fs), flags, opts) {
                Ok(()) => {
                    info!("mounting '{}' succeeded, fstype: '{}'", source, fs);
                    return Ok(());
                }
                Err(err) => {
                    warn!("mount error on '{}' ({}) - {}", source, fs, err);
                }
            }
        }

        bail!("all mounts failed or no supported file system")
    }
}

pub struct DiskState {
    filesystems: Filesystems,
    disk_map: HashMap<String, Vec<Bucket>>,
}

impl DiskState {
    /// Scan all disks for supported buckets.
    pub fn scan() -> Result<Self, Error> {
        // create mapping for virtio drives and .fidx files (via serial description)
        // note: disks::DiskManager relies on udev, which we don't have
        let mut disk_map = HashMap::new();
        for entry in proxmox_backup::tools::fs::scan_subdir(
            libc::AT_FDCWD,
            "/sys/block",
            &BLOCKDEVICE_NAME_REGEX,
        )?
        .filter_map(Result::ok)
        {
            let name = unsafe { entry.file_name_utf8_unchecked() };
            if !name.starts_with("vd") {
                continue;
            }

            let sys_path: &str = &format!("/sys/block/{}", name);

            let serial = fs::file_read_string(&format!("{}/serial", sys_path));
            let fidx = match serial {
                Ok(serial) => serial,
                Err(err) => {
                    warn!("disk '{}': could not read serial file - {}", name, err);
                    continue;
                }
            };

            let mut parts = Vec::new();
            for entry in proxmox_backup::tools::fs::scan_subdir(
                libc::AT_FDCWD,
                sys_path,
                &VIRTIO_PART_REGEX,
            )?
            .filter_map(Result::ok)
            {
                let part_name = unsafe { entry.file_name_utf8_unchecked() };
                let devnode = format!("/dev/{}", part_name);
                let part_path = format!("/sys/block/{}/{}", name, part_name);

                // create partition device node for further use
                let dev_num_str = fs::file_read_firstline(&format!("{}/dev", part_path))?;
                let (major, minor) = dev_num_str.split_at(dev_num_str.find(':').unwrap());
                Self::mknod_blk(&devnode, major.parse()?, minor[1..].trim_end().parse()?)?;

                let number = fs::file_read_firstline(&format!("{}/partition", part_path))?
                    .trim()
                    .parse::<i32>()?;

                // this *always* contains the number of 512-byte sectors, regardless of the true
                // blocksize of this disk - which should always be 512 here anyway
                let size = fs::file_read_firstline(&format!("{}/size", part_path))?
                    .trim()
                    .parse::<u64>()?
                    * 512;

                info!(
                    "drive '{}' ('{}'): found partition '{}' ({}, {}B)",
                    name, fidx, devnode, number, size
                );

                let bucket = Bucket::Partition(PartitionBucketData {
                    dev_node: devnode,
                    mountpoint: None,
                    number,
                    size,
                });

                parts.push(bucket);
            }

            disk_map.insert(fidx.to_owned(), parts);
        }

        Ok(Self {
            filesystems: Filesystems::scan()?,
            disk_map,
        })
    }

    /// Given a path like "/drive-scsi0.img.fidx/part/0/etc/passwd", this will mount the first
    /// partition of 'drive-scsi0' on-demand (i.e. if not already mounted) and return a path
    /// pointing to the requested file locally, e.g. "/mnt/vda1/etc/passwd", which can be used to
    /// read the file.  Given a partial path, i.e. only "/drive-scsi0.img.fidx" or
    /// "/drive-scsi0.img.fidx/part", it will return a list of available bucket types or bucket
    /// components respectively
    pub fn resolve(&mut self, path: &Path) -> Result<ResolveResult, Error> {
        let mut cmp = path.components().peekable();
        match cmp.peek() {
            Some(Component::RootDir) | Some(Component::CurDir) => {
                cmp.next();
            }
            None => bail!("empty path cannot be resolved to file location"),
            _ => {}
        }

        let req_fidx = match cmp.next() {
            Some(Component::Normal(x)) => x.to_string_lossy(),
            _ => bail!("no or invalid image in path"),
        };

        let buckets = match self.disk_map.get_mut(
            req_fidx
                .strip_suffix(".img.fidx")
                .unwrap_or_else(|| req_fidx.as_ref()),
        ) {
            Some(x) => x,
            None => bail!("given image '{}' not found", req_fidx),
        };

        let bucket_type = match cmp.next() {
            Some(Component::Normal(x)) => x.to_string_lossy(),
            Some(c) => bail!("invalid bucket in path: {:?}", c),
            None => {
                // list bucket types available
                let mut types = buckets
                    .iter()
                    .map(|b| b.type_string())
                    .collect::<Vec<&'static str>>();
                // dedup requires duplicates to be consecutive, which is the case - see scan()
                types.dedup();
                return Ok(ResolveResult::BucketTypes(types));
            }
        };

        let component = match cmp.next() {
            Some(Component::Normal(x)) => x.to_string_lossy(),
            Some(c) => bail!("invalid bucket component in path: {:?}", c),
            None => {
                // list bucket components available
                let comps = buckets
                    .iter()
                    .filter(|b| b.type_string() == bucket_type)
                    .map(|b| (b.component_string(), b.size()))
                    .collect();
                return Ok(ResolveResult::BucketComponents(comps));
            }
        };

        let mut bucket = match Bucket::filter_mut(buckets, &bucket_type, &component) {
            Some(bucket) => bucket,
            None => bail!(
                "bucket/component path not found: {}/{}/{}",
                req_fidx,
                bucket_type,
                component
            ),
        };

        // bucket found, check mount
        let mountpoint = self
            .filesystems
            .ensure_mounted(&mut bucket)
            .map_err(|err| {
                format_err!(
                    "mounting '{}/{}/{}' failed: {}",
                    req_fidx,
                    bucket_type,
                    component,
                    err
                )
            })?;

        let mut local_path = PathBuf::new();
        local_path.push(mountpoint);
        for rem in cmp {
            local_path.push(rem);
        }

        Ok(ResolveResult::Path(local_path))
    }

    fn mknod_blk(path: &str, maj: u64, min: u64) -> Result<(), Error> {
        use nix::sys::stat;
        let dev = stat::makedev(maj, min);
        stat::mknod(path, stat::SFlag::S_IFBLK, stat::Mode::S_IRWXU, dev)?;
        Ok(())
    }
}
