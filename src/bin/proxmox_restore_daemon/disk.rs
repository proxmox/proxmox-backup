//! Low-level disk (image) access functions for file restore VMs.
use anyhow::{bail, format_err, Error};
use lazy_static::lazy_static;
use log::{info, warn};

use std::collections::HashMap;
use std::fs::{create_dir_all, File};
use std::io::{BufRead, BufReader};
use std::path::{Component, Path, PathBuf};
use std::process::Command;

use proxmox::const_regex;
use proxmox::tools::fs;
use proxmox_backup::api2::types::BLOCKDEVICE_NAME_REGEX;
use proxmox_backup::tools::run_command;

const_regex! {
    VIRTIO_PART_REGEX = r"^vd[a-z]+(\d+)$";
    ZPOOL_POOL_NAME_REGEX = r"^ {3}pool: (.*)$";
    ZPOOL_IMPORT_DISK_REGEX = r"^\t {2,4}(vd[a-z]+(?:\d+)?)\s+ONLINE$";
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
    BucketComponents(Vec<(String, Option<u64>)>),
}

#[derive(Clone)]
struct PartitionBucketData {
    dev_node: String,
    number: i32,
    mountpoint: Option<PathBuf>,
    size: u64,
}

#[derive(Clone)]
struct ZFSBucketData {
    name: String,
    mountpoint: Option<PathBuf>,
    size: Option<u64>,
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
#[derive(Clone)]
enum Bucket {
    Partition(PartitionBucketData),
    RawFs(PartitionBucketData),
    ZPool(ZFSBucketData),
}

impl Bucket {
    fn filter_mut<'a, A: AsRef<str>, B: AsRef<str>>(
        haystack: &'a mut Vec<Bucket>,
        ty: A,
        comp: &[B],
    ) -> Option<&'a mut Bucket> {
        let ty = ty.as_ref();
        haystack.iter_mut().find(|b| match b {
            Bucket::Partition(data) => {
                if let Some(comp) = comp.get(0) {
                    ty == "part" && comp.as_ref().parse::<i32>().unwrap() == data.number
                } else {
                    false
                }
            }
            Bucket::RawFs(_) => ty == "raw",
            Bucket::ZPool(data) => {
                if let Some(ref comp) = comp.get(0) {
                    ty == "zpool" && comp.as_ref() == &data.name
                } else {
                    false
                }
            }
        })
    }

    fn type_string(&self) -> &'static str {
        match self {
            Bucket::Partition(_) => "part",
            Bucket::RawFs(_) => "raw",
            Bucket::ZPool(_) => "zpool",
        }
    }

    fn component_string(&self, idx: usize) -> Result<String, Error> {
        let max_depth = Self::component_depth(self.type_string())?;
        if idx >= max_depth {
            bail!(
                "internal error: component index out of range {}/{} ({})",
                idx,
                max_depth,
                self.type_string()
            );
        }
        Ok(match self {
            Bucket::Partition(data) => data.number.to_string(),
            Bucket::RawFs(_) => "raw".to_owned(),
            Bucket::ZPool(data) => data.name.clone(),
        })
    }

    fn component_depth(type_string: &str) -> Result<usize, Error> {
        Ok(match type_string {
            "part" => 1,
            "raw" => 0,
            "zpool" => 1,
            _ => bail!("invalid bucket type for component depth: {}", type_string),
        })
    }

    fn size(&self, idx: usize) -> Option<u64> {
        match self {
            Bucket::Partition(data) | Bucket::RawFs(data) => Some(data.size),
            Bucket::ZPool(data) => data.size,
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
            Bucket::Partition(data) | Bucket::RawFs(data) => {
                // regular data partition à la "/dev/vdxN" or FS directly on a disk
                if let Some(mp) = &data.mountpoint {
                    return Ok(mp.clone());
                }

                let mp = format!("/mnt{}/", data.dev_node);
                self.try_mount(&data.dev_node, &mp)?;
                let mp = PathBuf::from(mp);
                data.mountpoint = Some(mp.clone());
                Ok(mp)
            }
            Bucket::ZPool(data) => {
                if let Some(mp) = &data.mountpoint {
                    return Ok(mp.clone());
                }

                let mntpath = format!("/mnt/{}", &data.name);
                create_dir_all(&mntpath)?;

                // call ZFS tools to import and mount the pool with the root mount at 'mntpath'
                let mut cmd = Command::new("/sbin/zpool");
                cmd.args(
                    [
                        "import",
                        "-f",
                        "-o",
                        "readonly=on",
                        "-d",
                        "/dev",
                        "-R",
                        &mntpath,
                        &data.name,
                    ]
                    .iter(),
                );
                if let Err(msg) = run_command(cmd, None) {
                    // ignore double import, this may happen if a previous attempt failed further
                    // down below - this way we can at least try again
                    if !msg
                        .to_string()
                        .contains("a pool with that name already exists")
                    {
                        return Err(msg);
                    }
                }

                // 'mount -a' simply mounts all datasets that haven't been automounted, which
                // should only be ones that we've imported just now
                let mut cmd = Command::new("/sbin/zfs");
                cmd.args(["mount", "-a"].iter());
                run_command(cmd, None)?;

                // detect any datasets with 'legacy' mountpoints
                let mut cmd = Command::new("/sbin/zfs");
                cmd.args(["list", "-Hpro", "name,mountpoint", &data.name].iter());
                let mps = run_command(cmd, None)?;
                for subvol in mps.lines() {
                    let subvol = subvol.splitn(2, '\t').collect::<Vec<&str>>();
                    if subvol.len() != 2 {
                        continue;
                    }
                    let name = subvol[0];
                    let mp = subvol[1];

                    if mp == "legacy" {
                        let mut newmp = PathBuf::from(format!(
                            "{}/legacy-{}",
                            &mntpath,
                            name.replace('/', "_")
                        ));
                        let mut i = 1;
                        while newmp.exists() {
                            newmp.set_extension(i.to_string());
                            i += 1;
                        }
                        create_dir_all(&newmp)?;
                        self.do_mount(Some(name), newmp.to_string_lossy().as_ref(), "zfs")?;
                    }
                }

                // Now that we have imported the pool, we can also query the size
                let mut cmd = Command::new("/sbin/zpool");
                cmd.args(["list", "-o", "size", "-Hp", &data.name].iter());
                let size = run_command(cmd, None)?;
                if let Ok(size) = size.trim().parse::<u64>() {
                    data.size = Some(size);
                }

                let mp = PathBuf::from(mntpath);
                data.mountpoint = Some(mp.clone());
                Ok(mp)
            }
        }
    }

    fn try_mount(&self, source: &str, target: &str) -> Result<(), Error> {
        create_dir_all(target)?;

        // try all supported fs until one works - this is the way Busybox's 'mount' does it too:
        // https://git.busybox.net/busybox/tree/util-linux/mount.c?id=808d93c0eca49e0b22056e23d965f0d967433fbb#n2152
        // note that ZFS is intentionally left out (see scan())
        for fs in &self.supported_fs {
            let fs: &str = fs.as_ref();
            match self.do_mount(Some(source), target, fs) {
                Ok(()) => {
                    info!("mounting '{}' succeeded, fstype: '{}'", source, fs);
                    return Ok(());
                }
                Err(nix::Error::Sys(nix::errno::Errno::EINVAL)) => {}
                Err(err) => {
                    warn!("mount error on '{}' ({}) - {}", source, fs, err);
                }
            }
        }

        bail!("all mounts failed or no supported file system")
    }

    fn do_mount(&self, source: Option<&str>, target: &str, fs: &str) -> Result<(), nix::Error> {
        use nix::mount::*;
        let flags =
            MsFlags::MS_RDONLY | MsFlags::MS_NOEXEC | MsFlags::MS_NOSUID | MsFlags::MS_NODEV;
        let opts = FS_OPT_MAP.get(fs).copied();
        mount(source, target, Some(fs), flags, opts)
    }
}

pub struct DiskState {
    filesystems: Filesystems,
    disk_map: HashMap<String, Vec<Bucket>>,
}

impl DiskState {
    /// Scan all disks for supported buckets.
    pub fn scan() -> Result<Self, Error> {
        let filesystems = Filesystems::scan()?;

        let mut disk_map = HashMap::new();
        let mut drive_info = HashMap::new();

        // create mapping for virtio drives and .fidx files (via serial description)
        // note: disks::DiskManager relies on udev, which we don't have
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

            drive_info.insert(name.to_owned(), fidx.clone());

            // attempt to mount device directly
            let dev_node = format!("/dev/{}", name);
            let size = Self::make_dev_node(&dev_node, &sys_path)?;
            let mut dfs_bucket = Bucket::RawFs(PartitionBucketData {
                dev_node: dev_node.clone(),
                number: 0,
                mountpoint: None,
                size,
            });
            if let Ok(_) = filesystems.ensure_mounted(&mut dfs_bucket) {
                // mount succeeded, add bucket and skip any other checks for the disk
                info!(
                    "drive '{}' ('{}', '{}') contains fs directly ({}B)",
                    name, fidx, dev_node, size
                );
                disk_map.insert(fidx, vec![dfs_bucket]);
                continue;
            }

            let mut parts = Vec::new();
            for entry in proxmox_backup::tools::fs::scan_subdir(
                libc::AT_FDCWD,
                sys_path,
                &VIRTIO_PART_REGEX,
            )?
            .filter_map(Result::ok)
            {
                let part_name = unsafe { entry.file_name_utf8_unchecked() };
                let dev_node = format!("/dev/{}", part_name);
                let part_path = format!("/sys/block/{}/{}", name, part_name);

                // create partition device node for further use
                let size = Self::make_dev_node(&dev_node, &part_path)?;

                let number = fs::file_read_firstline(&format!("{}/partition", part_path))?
                    .trim()
                    .parse::<i32>()?;

                info!(
                    "drive '{}' ('{}'): found partition '{}' ({}, {}B)",
                    name, fidx, dev_node, number, size
                );

                let bucket = Bucket::Partition(PartitionBucketData {
                    dev_node,
                    mountpoint: None,
                    number,
                    size,
                });

                parts.push(bucket);

                drive_info.insert(part_name.to_owned(), fidx.clone());
            }

            disk_map.insert(fidx, parts);
        }

        // After the above, every valid disk should have a device node in /dev, so we can query all
        // of them for zpools
        let mut cmd = Command::new("/sbin/zpool");
        cmd.args(["import", "-d", "/dev"].iter());
        let result = run_command(cmd, None).unwrap();
        for (pool, disks) in Self::parse_zpool_import(&result) {
            let mut bucket = Bucket::ZPool(ZFSBucketData {
                name: pool.clone(),
                size: None,
                mountpoint: None,
            });

            // anything more than 5 disks we assume to take too long to mount, so we don't
            // automatically - this means that no size can be reported
            if disks.len() <= 5 {
                let mp = filesystems.ensure_mounted(&mut bucket);
                info!(
                    "zpool '{}' (on: {:?}) auto-mounted at '{:?}' (size: {:?})",
                    &pool,
                    &disks,
                    mp,
                    bucket.size(0)
                );
            } else {
                info!(
                    "zpool '{}' (on: {:?}) auto-mount skipped, too many disks",
                    &pool, &disks
                );
            }

            for disk in disks {
                if let Some(fidx) = drive_info.get(&disk) {
                    match disk_map.get_mut(fidx) {
                        Some(v) => v.push(bucket.clone()),
                        None => {
                            disk_map.insert(fidx.to_owned(), vec![bucket.clone()]);
                        }
                    }
                }
            }
        }

        Ok(Self {
            filesystems,
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

        let mut components = Vec::new();
        let component_count = Bucket::component_depth(&bucket_type)?;

        while components.len() < component_count {
            let component = match cmp.next() {
                Some(Component::Normal(x)) => x.to_string_lossy(),
                Some(c) => bail!("invalid bucket component in path: {:?}", c),
                None => {
                    // list bucket components available at this level
                    let mut comps = buckets
                        .iter()
                        .filter_map(|b| {
                            if b.type_string() != bucket_type {
                                return None;
                            }
                            match b.component_string(components.len()) {
                                Ok(cs) => Some((cs.to_owned(), b.size(components.len()))),
                                Err(_) => None,
                            }
                        })
                        .collect::<Vec<(String, Option<u64>)>>();
                    comps.sort_by(|a, b| a.0.cmp(&b.0));
                    comps.dedup();
                    return Ok(ResolveResult::BucketComponents(comps));
                }
            };

            components.push(component);
        }

        let mut bucket = match Bucket::filter_mut(buckets, &bucket_type, &components) {
            Some(bucket) => bucket,
            None => bail!(
                "bucket/component path not found: {}/{}/{:?}",
                req_fidx,
                bucket_type,
                components
            ),
        };

        // bucket found, check mount
        let mountpoint = self
            .filesystems
            .ensure_mounted(&mut bucket)
            .map_err(|err| {
                format_err!(
                    "mounting '{}/{}/{:?}' failed: {}",
                    req_fidx,
                    bucket_type,
                    components,
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

    fn make_dev_node(devnode: &str, sys_path: &str) -> Result<u64, Error> {
        let dev_num_str = fs::file_read_firstline(&format!("{}/dev", sys_path))?;
        let (major, minor) = dev_num_str.split_at(dev_num_str.find(':').unwrap());
        Self::mknod_blk(&devnode, major.parse()?, minor[1..].trim_end().parse()?)?;

        // this *always* contains the number of 512-byte sectors, regardless of the true
        // blocksize of this disk - which should always be 512 here anyway
        let size = fs::file_read_firstline(&format!("{}/size", sys_path))?
            .trim()
            .parse::<u64>()?
            * 512;

        Ok(size)
    }

    fn mknod_blk(path: &str, maj: u64, min: u64) -> Result<(), Error> {
        use nix::sys::stat;
        let dev = stat::makedev(maj, min);
        stat::mknod(path, stat::SFlag::S_IFBLK, stat::Mode::S_IRWXU, dev)?;
        Ok(())
    }

    fn parse_zpool_import(data: &str) -> Vec<(String, Vec<String>)> {
        let mut ret = Vec::new();
        let mut disks = Vec::new();
        let mut cur = "".to_string();
        for line in data.lines() {
            if let Some(groups) = (ZPOOL_POOL_NAME_REGEX.regex_obj)().captures(line) {
                if let Some(name) = groups.get(1) {
                    if !disks.is_empty() {
                        ret.push((cur, disks.clone()));
                    }
                    disks.clear();
                    cur = name.as_str().to_owned();
                }
            } else if let Some(groups) = (ZPOOL_IMPORT_DISK_REGEX.regex_obj)().captures(line) {
                if let Some(disk) = groups.get(1) {
                    disks.push(disk.as_str().to_owned());
                }
            }
        }
        if !disks.is_empty() && !cur.is_empty() {
            ret.push((cur, disks));
        }
        ret
    }
}
