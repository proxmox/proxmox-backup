//! Feature flags for *pxar* allow to control what is stored/restored in/from the
//! archive.
//! Flags for known supported features for a given filesystem can be derived
//! from the superblocks magic number.

// FIXME: use bitflags!() here!

/// FAT-style 2s time granularity
pub const WITH_2SEC_TIME: u64                   = 0x40;
/// Preserve read only flag of files
pub const WITH_READ_ONLY: u64                   = 0x80;
/// Preserve unix permissions
pub const WITH_PERMISSIONS: u64                 = 0x100;
/// Include symbolik links
pub const WITH_SYMLINKS: u64                    = 0x200;
/// Include device nodes
pub const WITH_DEVICE_NODES: u64                = 0x400;
/// Include FIFOs
pub const WITH_FIFOS: u64                       = 0x800;
/// Include Sockets
pub const WITH_SOCKETS: u64                     = 0x1000;

/// Preserve DOS file flag `HIDDEN`
pub const WITH_FLAG_HIDDEN: u64                 = 0x2000;
/// Preserve DOS file flag `SYSTEM`
pub const WITH_FLAG_SYSTEM: u64                 = 0x4000;
/// Preserve DOS file flag `ARCHIVE`
pub const WITH_FLAG_ARCHIVE: u64                = 0x8000;

// chattr() flags
/// Linux file attribute `APPEND`
pub const WITH_FLAG_APPEND: u64                 = 0x10000;
/// Linux file attribute `NOATIME`
pub const WITH_FLAG_NOATIME: u64                = 0x20000;
/// Linux file attribute `COMPR`
pub const WITH_FLAG_COMPR: u64                  = 0x40000;
/// Linux file attribute `NOCOW`
pub const WITH_FLAG_NOCOW: u64                  = 0x80000;
/// Linux file attribute `NODUMP`
pub const WITH_FLAG_NODUMP: u64                 = 0x0010_0000;
/// Linux file attribute `DIRSYNC`
pub const WITH_FLAG_DIRSYNC: u64                = 0x0020_0000;
/// Linux file attribute `IMMUTABLE`
pub const WITH_FLAG_IMMUTABLE: u64              = 0x0040_0000;
/// Linux file attribute `SYNC`
pub const WITH_FLAG_SYNC: u64                   = 0x0080_0000;
/// Linux file attribute `NOCOMP`
pub const WITH_FLAG_NOCOMP: u64                 = 0x0100_0000;
/// Linux file attribute `PROJINHERIT`
pub const WITH_FLAG_PROJINHERIT: u64            = 0x0200_0000;


/// Preserve BTRFS subvolume flag
pub const WITH_SUBVOLUME: u64                   = 0x0400_0000;
/// Preserve BTRFS read-only subvolume flag
pub const WITH_SUBVOLUME_RO: u64                = 0x0800_0000;

/// Preserve Extended Attribute metadata
pub const WITH_XATTRS: u64                      = 0x1000_0000;
/// Preserve Access Control List metadata
pub const WITH_ACL: u64                         = 0x2000_0000;
/// Preserve SELinux security context
pub const WITH_SELINUX: u64                     = 0x4000_0000;
/// Preserve "security.capability" xattr
pub const WITH_FCAPS: u64                       = 0x8000_0000;

/// Preserve XFS/ext4/ZFS project quota ID
pub const WITH_QUOTA_PROJID: u64                = 0x0001_0000_0000;

/// Support ".pxarexclude" files
pub const EXCLUDE_FILE: u64                     = 0x1000_0000_0000_0000;
/// Exclude submounts
pub const EXCLUDE_SUBMOUNTS: u64                = 0x4000_0000_0000_0000;
/// Exclude entries with chattr flag NODUMP
pub const EXCLUDE_NODUMP: u64                   = 0x8000_0000_0000_0000;

/// Definitions of typical feature flags for the *pxar* encoder/decoder.
/// By this expensive syscalls for unsupported features are avoided.

/// All chattr file attributes
pub const WITH_CHATTR: u64 =
    WITH_FLAG_APPEND|
    WITH_FLAG_NOATIME|
    WITH_FLAG_COMPR|
    WITH_FLAG_NOCOW|
    WITH_FLAG_NODUMP|
    WITH_FLAG_DIRSYNC|
    WITH_FLAG_IMMUTABLE|
    WITH_FLAG_SYNC|
    WITH_FLAG_NOCOMP|
    WITH_FLAG_PROJINHERIT;

/// All FAT file attributes
pub const WITH_FAT_ATTRS: u64 =
    WITH_FLAG_HIDDEN|
    WITH_FLAG_SYSTEM|
    WITH_FLAG_ARCHIVE;

/// All bits that may also be exposed via fuse
pub const WITH_FUSE: u64 =
    WITH_2SEC_TIME|
    WITH_READ_ONLY|
    WITH_PERMISSIONS|
    WITH_SYMLINKS|
    WITH_DEVICE_NODES|
    WITH_FIFOS|
    WITH_SOCKETS|
    WITH_FAT_ATTRS|
    WITH_CHATTR|
    WITH_XATTRS;


/// Default feature flags for encoder/decoder
pub const DEFAULT: u64 =
    WITH_SYMLINKS|
    WITH_DEVICE_NODES|
    WITH_FIFOS|
    WITH_SOCKETS|
    WITH_FLAG_HIDDEN|
    WITH_FLAG_SYSTEM|
    WITH_FLAG_ARCHIVE|
    WITH_FLAG_APPEND|
    WITH_FLAG_NOATIME|
    WITH_FLAG_COMPR|
    WITH_FLAG_NOCOW|
    //WITH_FLAG_NODUMP|
    WITH_FLAG_DIRSYNC|
    WITH_FLAG_IMMUTABLE|
    WITH_FLAG_SYNC|
    WITH_FLAG_NOCOMP|
    WITH_FLAG_PROJINHERIT|
    WITH_SUBVOLUME|
    WITH_SUBVOLUME_RO|
    WITH_XATTRS|
    WITH_ACL|
    WITH_SELINUX|
    WITH_FCAPS|
    WITH_QUOTA_PROJID|
    EXCLUDE_NODUMP|
    EXCLUDE_FILE;

// form /usr/include/linux/fs.h
const FS_APPEND_FL: u32 =      0x0000_0020;
const FS_NOATIME_FL: u32 =     0x0000_0080;
const FS_COMPR_FL: u32 =       0x0000_0004;
const FS_NOCOW_FL: u32 =       0x0080_0000;
const FS_NODUMP_FL: u32 =      0x0000_0040;
const FS_DIRSYNC_FL: u32 =     0x0001_0000;
const FS_IMMUTABLE_FL: u32 =   0x0000_0010;
const FS_SYNC_FL: u32 =        0x0000_0008;
const FS_NOCOMP_FL: u32 =      0x0000_0400;
const FS_PROJINHERIT_FL: u32 = 0x2000_0000;

static CHATTR_MAP: [(u64, u32); 10] = [
    ( WITH_FLAG_APPEND,      FS_APPEND_FL      ),
    ( WITH_FLAG_NOATIME,     FS_NOATIME_FL     ),
    ( WITH_FLAG_COMPR,       FS_COMPR_FL       ),
    ( WITH_FLAG_NOCOW,       FS_NOCOW_FL       ),
    ( WITH_FLAG_NODUMP,      FS_NODUMP_FL      ),
    ( WITH_FLAG_DIRSYNC,     FS_DIRSYNC_FL     ),
    ( WITH_FLAG_IMMUTABLE,   FS_IMMUTABLE_FL   ),
    ( WITH_FLAG_SYNC,        FS_SYNC_FL        ),
    ( WITH_FLAG_NOCOMP,      FS_NOCOMP_FL      ),
    ( WITH_FLAG_PROJINHERIT, FS_PROJINHERIT_FL ),
];

pub fn feature_flags_from_chattr(attr: u32) -> u64 {

    let mut flags = 0u64;

    for (fe_flag, fs_flag) in &CHATTR_MAP {
        if (attr & fs_flag) != 0 { flags |= fe_flag; }
    }

    flags
}

// from /usr/include/linux/msdos_fs.h
const ATTR_HIDDEN: u32 =      2;
const ATTR_SYS: u32 =         4;
const ATTR_ARCH: u32 =       32;

static FAT_ATTR_MAP: [(u64, u32); 3] = [
    ( WITH_FLAG_HIDDEN,  ATTR_HIDDEN ),
    ( WITH_FLAG_SYSTEM,  ATTR_SYS    ),
    ( WITH_FLAG_ARCHIVE, ATTR_ARCH   ),
];

pub fn feature_flags_from_fat_attr(attr: u32) -> u64 {

    let mut flags = 0u64;

    for (fe_flag, fs_flag) in &FAT_ATTR_MAP {
        if (attr & fs_flag) != 0 { flags |= fe_flag; }
    }

    flags
}


/// Return the supported *pxar* feature flags based on the magic number of the filesystem.
pub fn feature_flags_from_magic(magic: i64) -> u64 {
    use proxmox::sys::linux::magic::*;
    match magic {
        MSDOS_SUPER_MAGIC => {
            WITH_2SEC_TIME|
            WITH_READ_ONLY|
            WITH_FAT_ATTRS
        },
        EXT4_SUPER_MAGIC => {
            WITH_2SEC_TIME|
            WITH_READ_ONLY|
            WITH_PERMISSIONS|
            WITH_SYMLINKS|
            WITH_DEVICE_NODES|
            WITH_FIFOS|
            WITH_SOCKETS|
            WITH_FLAG_APPEND|
            WITH_FLAG_NOATIME|
            WITH_FLAG_NODUMP|
            WITH_FLAG_DIRSYNC|
            WITH_FLAG_IMMUTABLE|
            WITH_FLAG_SYNC|
            WITH_XATTRS|
            WITH_ACL|
            WITH_SELINUX|
            WITH_FCAPS|
            WITH_QUOTA_PROJID
        },
        XFS_SUPER_MAGIC => {
            WITH_2SEC_TIME|
            WITH_READ_ONLY|
            WITH_PERMISSIONS|
            WITH_SYMLINKS|
            WITH_DEVICE_NODES|
            WITH_FIFOS|
            WITH_SOCKETS|
            WITH_FLAG_APPEND|
            WITH_FLAG_NOATIME|
            WITH_FLAG_NODUMP|
            WITH_FLAG_IMMUTABLE|
            WITH_FLAG_SYNC|
            WITH_XATTRS|
            WITH_ACL|
            WITH_SELINUX|
            WITH_FCAPS|
            WITH_QUOTA_PROJID
        },
        ZFS_SUPER_MAGIC => {
            WITH_2SEC_TIME|
            WITH_READ_ONLY|
            WITH_PERMISSIONS|
            WITH_SYMLINKS|
            WITH_DEVICE_NODES|
            WITH_FIFOS|
            WITH_SOCKETS|
            WITH_FLAG_APPEND|
            WITH_FLAG_NOATIME|
            WITH_FLAG_NODUMP|
            WITH_FLAG_DIRSYNC|
            WITH_FLAG_IMMUTABLE|
            WITH_FLAG_SYNC|
            WITH_XATTRS|
            WITH_ACL|
            WITH_SELINUX|
            WITH_FCAPS|
            WITH_QUOTA_PROJID
        },
        BTRFS_SUPER_MAGIC => {
            WITH_2SEC_TIME|
            WITH_READ_ONLY|
            WITH_PERMISSIONS|
            WITH_SYMLINKS|
            WITH_DEVICE_NODES|
            WITH_FIFOS|
            WITH_SOCKETS|
            WITH_FLAG_APPEND|
            WITH_FLAG_NOATIME|
            WITH_FLAG_COMPR|
            WITH_FLAG_NOCOW|
            WITH_FLAG_NODUMP|
            WITH_FLAG_DIRSYNC|
            WITH_FLAG_IMMUTABLE|
            WITH_FLAG_SYNC|
            WITH_FLAG_NOCOMP|
            WITH_XATTRS|
            WITH_ACL|
            WITH_SELINUX|
            WITH_SUBVOLUME|
            WITH_SUBVOLUME_RO|
            WITH_FCAPS
        },
        TMPFS_MAGIC => {
            WITH_2SEC_TIME|
            WITH_READ_ONLY|
            WITH_PERMISSIONS|
            WITH_SYMLINKS|
            WITH_DEVICE_NODES|
            WITH_FIFOS|
            WITH_SOCKETS|
            WITH_ACL|
            WITH_SELINUX
        },
        // FUSE mounts are special as the supported feature set
        // is not clear a priori.
        FUSE_SUPER_MAGIC => {
            WITH_FUSE
        },
        _ => {
            WITH_2SEC_TIME|
            WITH_READ_ONLY|
            WITH_PERMISSIONS|
            WITH_SYMLINKS|
            WITH_DEVICE_NODES|
            WITH_FIFOS|
            WITH_SOCKETS
        },
    }
}
