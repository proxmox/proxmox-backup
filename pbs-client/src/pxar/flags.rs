//! Feature flags for *pxar* allow to control what is stored/restored in/from the
//! archive.
//! Flags for known supported features for a given filesystem can be derived
//! from the superblocks magic number.

use libc::c_long;

use bitflags::bitflags;

bitflags! {
    pub struct Flags: u64 {
        /// FAT-style 2s time granularity
        const WITH_2SEC_TIME                   = 0x40;
        /// Preserve read only flag of files
        const WITH_READ_ONLY                   = 0x80;
        /// Preserve unix permissions
        const WITH_PERMISSIONS                 = 0x100;
        /// Include symbolik links
        const WITH_SYMLINKS                    = 0x200;
        /// Include device nodes
        const WITH_DEVICE_NODES                = 0x400;
        /// Include FIFOs
        const WITH_FIFOS                       = 0x800;
        /// Include Sockets
        const WITH_SOCKETS                     = 0x1000;

        /// Preserve DOS file flag `HIDDEN`
        const WITH_FLAG_HIDDEN                 = 0x2000;
        /// Preserve DOS file flag `SYSTEM`
        const WITH_FLAG_SYSTEM                 = 0x4000;
        /// Preserve DOS file flag `ARCHIVE`
        const WITH_FLAG_ARCHIVE                = 0x8000;

        // chattr() flags
        /// Linux file attribute `APPEND`
        const WITH_FLAG_APPEND                 = 0x10000;
        /// Linux file attribute `NOATIME`
        const WITH_FLAG_NOATIME                = 0x20000;
        /// Linux file attribute `COMPR`
        const WITH_FLAG_COMPR                  = 0x40000;
        /// Linux file attribute `NOCOW`
        const WITH_FLAG_NOCOW                  = 0x80000;
        /// Linux file attribute `NODUMP`
        const WITH_FLAG_NODUMP                 = 0x0010_0000;
        /// Linux file attribute `DIRSYNC`
        const WITH_FLAG_DIRSYNC                = 0x0020_0000;
        /// Linux file attribute `IMMUTABLE`
        const WITH_FLAG_IMMUTABLE              = 0x0040_0000;
        /// Linux file attribute `SYNC`
        const WITH_FLAG_SYNC                   = 0x0080_0000;
        /// Linux file attribute `NOCOMP`
        const WITH_FLAG_NOCOMP                 = 0x0100_0000;
        /// Linux file attribute `PROJINHERIT`
        const WITH_FLAG_PROJINHERIT            = 0x0200_0000;


        /// Preserve BTRFS subvolume flag
        const WITH_SUBVOLUME                   = 0x0400_0000;
        /// Preserve BTRFS read-only subvolume flag
        const WITH_SUBVOLUME_RO                = 0x0800_0000;

        /// Preserve Extended Attribute metadata
        const WITH_XATTRS                      = 0x1000_0000;
        /// Preserve Access Control List metadata
        const WITH_ACL                         = 0x2000_0000;
        /// Preserve SELinux security context
        const WITH_SELINUX                     = 0x4000_0000;
        /// Preserve "security.capability" xattr
        const WITH_FCAPS                       = 0x8000_0000;

        /// Preserve XFS/ext4/ZFS project quota ID
        const WITH_QUOTA_PROJID                = 0x0001_0000_0000;

        /// UNIX OWNERSHIP
        const WITH_OWNER                       = 0x0002_0000_0000;

        /// Support ".pxarexclude" files
        const EXCLUDE_FILE                     = 0x1000_0000_0000_0000;
        /// Exclude submounts
        const EXCLUDE_SUBMOUNTS                = 0x4000_0000_0000_0000;
        /// Exclude entries with chattr flag NODUMP
        const EXCLUDE_NODUMP                   = 0x8000_0000_0000_0000;

        // Definitions of typical feature flags for the *pxar* encoder/decoder.
        // By this expensive syscalls for unsupported features are avoided.

        /// All chattr file attributes
        const WITH_CHATTR =
            Flags::WITH_FLAG_APPEND.bits() |
            Flags::WITH_FLAG_NOATIME.bits() |
            Flags::WITH_FLAG_COMPR.bits() |
            Flags::WITH_FLAG_NOCOW.bits() |
            Flags::WITH_FLAG_NODUMP.bits() |
            Flags::WITH_FLAG_DIRSYNC.bits() |
            Flags::WITH_FLAG_IMMUTABLE.bits() |
            Flags::WITH_FLAG_SYNC.bits() |
            Flags::WITH_FLAG_NOCOMP.bits() |
            Flags::WITH_FLAG_PROJINHERIT.bits();

        /// All FAT file attributes
        const WITH_FAT_ATTRS =
            Flags::WITH_FLAG_HIDDEN.bits() |
            Flags::WITH_FLAG_SYSTEM.bits() |
            Flags::WITH_FLAG_ARCHIVE.bits();

        /// All bits that may also be exposed via fuse
        const WITH_FUSE =
            Flags::WITH_2SEC_TIME.bits() |
            Flags::WITH_READ_ONLY.bits() |
            Flags::WITH_PERMISSIONS.bits() |
            Flags::WITH_OWNER.bits() |
            Flags::WITH_SYMLINKS.bits() |
            Flags::WITH_DEVICE_NODES.bits() |
            Flags::WITH_FIFOS.bits() |
            Flags::WITH_SOCKETS.bits() |
            Flags::WITH_FAT_ATTRS.bits() |
            Flags::WITH_CHATTR.bits() |
            Flags::WITH_XATTRS.bits();


        /// Default feature flags for encoder/decoder
        const DEFAULT =
            Flags::WITH_SYMLINKS.bits() |
            Flags::WITH_DEVICE_NODES.bits() |
            Flags::WITH_FIFOS.bits() |
            Flags::WITH_SOCKETS.bits() |
            Flags::WITH_FLAG_HIDDEN.bits() |
            Flags::WITH_FLAG_SYSTEM.bits() |
            Flags::WITH_FLAG_ARCHIVE.bits() |
            Flags::WITH_FLAG_APPEND.bits() |
            Flags::WITH_FLAG_NOATIME.bits() |
            Flags::WITH_FLAG_COMPR.bits() |
            Flags::WITH_FLAG_NOCOW.bits() |
            //WITH_FLAG_NODUMP.bits() |
            Flags::WITH_FLAG_DIRSYNC.bits() |
            Flags::WITH_FLAG_IMMUTABLE.bits() |
            Flags::WITH_FLAG_SYNC.bits() |
            Flags::WITH_FLAG_NOCOMP.bits() |
            Flags::WITH_FLAG_PROJINHERIT.bits() |
            Flags::WITH_SUBVOLUME.bits() |
            Flags::WITH_SUBVOLUME_RO.bits() |
            Flags::WITH_PERMISSIONS.bits() |
            Flags::WITH_OWNER.bits() |
            Flags::WITH_XATTRS.bits() |
            Flags::WITH_ACL.bits() |
            Flags::WITH_SELINUX.bits() |
            Flags::WITH_FCAPS.bits() |
            Flags::WITH_QUOTA_PROJID.bits() |
            Flags::EXCLUDE_NODUMP.bits() |
            Flags::EXCLUDE_FILE.bits();
    }
}

impl Default for Flags {
    fn default() -> Flags {
        Flags::DEFAULT
    }
}

#[rustfmt::skip]
mod fs_flags {
use libc::c_long;
    // form /usr/include/linux/fs.h
    pub const FS_APPEND_FL: c_long =      0x0000_0020;
    pub const FS_NOATIME_FL: c_long =     0x0000_0080;
    pub const FS_COMPR_FL: c_long =       0x0000_0004;
    pub const FS_NOCOW_FL: c_long =       0x0080_0000;
    pub const FS_NODUMP_FL: c_long =      0x0000_0040;
    pub const FS_DIRSYNC_FL: c_long =     0x0001_0000;
    pub const FS_IMMUTABLE_FL: c_long =   0x0000_0010;
    pub const FS_SYNC_FL: c_long =        0x0000_0008;
    pub const FS_NOCOMP_FL: c_long =      0x0000_0400;
    pub const FS_PROJINHERIT_FL: c_long = 0x2000_0000;

    // from /usr/include/linux/msdos_fs.h
    pub const ATTR_HIDDEN: u32 =      2;
    pub const ATTR_SYS: u32 =         4;
    pub const ATTR_ARCH: u32 =       32;

    pub(crate) const INITIAL_FS_FLAGS: c_long =
        FS_NOATIME_FL
        | FS_COMPR_FL
        | FS_NOCOW_FL
        | FS_NOCOMP_FL
        | FS_PROJINHERIT_FL;

}
use fs_flags::*; // for code formatting/rusfmt

#[rustfmt::skip]
const CHATTR_MAP: [(Flags, c_long); 10] = [
    ( Flags::WITH_FLAG_APPEND,      FS_APPEND_FL      ),
    ( Flags::WITH_FLAG_NOATIME,     FS_NOATIME_FL     ),
    ( Flags::WITH_FLAG_COMPR,       FS_COMPR_FL       ),
    ( Flags::WITH_FLAG_NOCOW,       FS_NOCOW_FL       ),
    ( Flags::WITH_FLAG_NODUMP,      FS_NODUMP_FL      ),
    ( Flags::WITH_FLAG_DIRSYNC,     FS_DIRSYNC_FL     ),
    ( Flags::WITH_FLAG_IMMUTABLE,   FS_IMMUTABLE_FL   ),
    ( Flags::WITH_FLAG_SYNC,        FS_SYNC_FL        ),
    ( Flags::WITH_FLAG_NOCOMP,      FS_NOCOMP_FL      ),
    ( Flags::WITH_FLAG_PROJINHERIT, FS_PROJINHERIT_FL ),
];

#[rustfmt::skip]
const FAT_ATTR_MAP: [(Flags, u32); 3] = [
    ( Flags::WITH_FLAG_HIDDEN,  ATTR_HIDDEN ),
    ( Flags::WITH_FLAG_SYSTEM,  ATTR_SYS    ),
    ( Flags::WITH_FLAG_ARCHIVE, ATTR_ARCH   ),
];

impl Flags {
    /// Get a set of feature flags from file attributes.
    pub fn from_chattr(attr: c_long) -> Flags {
        let mut flags = Flags::empty();

        for (fe_flag, fs_flag) in &CHATTR_MAP {
            if (attr & fs_flag) != 0 {
                flags |= *fe_flag;
            }
        }

        flags
    }

    /// Get the chattr bit representation of these feature flags.
    pub fn to_chattr(self) -> c_long {
        let mut flags: c_long = 0;

        for (fe_flag, fs_flag) in &CHATTR_MAP {
            if self.contains(*fe_flag) {
                flags |= *fs_flag;
            }
        }

        flags
    }

    pub fn to_initial_chattr(self) -> c_long {
        self.to_chattr() & INITIAL_FS_FLAGS
    }

    /// Get a set of feature flags from FAT attributes.
    pub fn from_fat_attr(attr: u32) -> Flags {
        let mut flags = Flags::empty();

        for (fe_flag, fs_flag) in &FAT_ATTR_MAP {
            if (attr & fs_flag) != 0 {
                flags |= *fe_flag;
            }
        }

        flags
    }

    /// Get the fat attribute bit representation of these feature flags.
    pub fn to_fat_attr(self) -> u32 {
        let mut flags = 0u32;

        for (fe_flag, fs_flag) in &FAT_ATTR_MAP {
            if self.contains(*fe_flag) {
                flags |= *fs_flag;
            }
        }

        flags
    }

    /// Return the supported *pxar* feature flags based on the magic number of the filesystem.
    pub fn from_magic(magic: i64) -> Flags {
        use proxmox_sys::linux::magic::*;
        match magic {
            MSDOS_SUPER_MAGIC => {
                Flags::WITH_2SEC_TIME | Flags::WITH_READ_ONLY | Flags::WITH_FAT_ATTRS
            }
            EXT4_SUPER_MAGIC => {
                Flags::WITH_2SEC_TIME
                    | Flags::WITH_READ_ONLY
                    | Flags::WITH_PERMISSIONS
                    | Flags::WITH_SYMLINKS
                    | Flags::WITH_DEVICE_NODES
                    | Flags::WITH_FIFOS
                    | Flags::WITH_SOCKETS
                    | Flags::WITH_FLAG_APPEND
                    | Flags::WITH_FLAG_NOATIME
                    | Flags::WITH_FLAG_NODUMP
                    | Flags::WITH_FLAG_DIRSYNC
                    | Flags::WITH_FLAG_IMMUTABLE
                    | Flags::WITH_FLAG_SYNC
                    | Flags::WITH_XATTRS
                    | Flags::WITH_ACL
                    | Flags::WITH_SELINUX
                    | Flags::WITH_FCAPS
                    | Flags::WITH_QUOTA_PROJID
            }
            XFS_SUPER_MAGIC => {
                Flags::WITH_2SEC_TIME
                    | Flags::WITH_READ_ONLY
                    | Flags::WITH_PERMISSIONS
                    | Flags::WITH_SYMLINKS
                    | Flags::WITH_DEVICE_NODES
                    | Flags::WITH_FIFOS
                    | Flags::WITH_SOCKETS
                    | Flags::WITH_FLAG_APPEND
                    | Flags::WITH_FLAG_NOATIME
                    | Flags::WITH_FLAG_NODUMP
                    | Flags::WITH_FLAG_IMMUTABLE
                    | Flags::WITH_FLAG_SYNC
                    | Flags::WITH_XATTRS
                    | Flags::WITH_ACL
                    | Flags::WITH_SELINUX
                    | Flags::WITH_FCAPS
                    | Flags::WITH_QUOTA_PROJID
            }
            ZFS_SUPER_MAGIC => {
                Flags::WITH_2SEC_TIME
                    | Flags::WITH_READ_ONLY
                    | Flags::WITH_PERMISSIONS
                    | Flags::WITH_SYMLINKS
                    | Flags::WITH_DEVICE_NODES
                    | Flags::WITH_FIFOS
                    | Flags::WITH_SOCKETS
                    | Flags::WITH_FLAG_APPEND
                    | Flags::WITH_FLAG_NOATIME
                    | Flags::WITH_FLAG_NODUMP
                    | Flags::WITH_FLAG_DIRSYNC
                    | Flags::WITH_FLAG_IMMUTABLE
                    | Flags::WITH_FLAG_SYNC
                    | Flags::WITH_XATTRS
                    | Flags::WITH_ACL
                    | Flags::WITH_SELINUX
                    | Flags::WITH_FCAPS
                    | Flags::WITH_QUOTA_PROJID
            }
            BTRFS_SUPER_MAGIC => {
                Flags::WITH_2SEC_TIME
                    | Flags::WITH_READ_ONLY
                    | Flags::WITH_PERMISSIONS
                    | Flags::WITH_SYMLINKS
                    | Flags::WITH_DEVICE_NODES
                    | Flags::WITH_FIFOS
                    | Flags::WITH_SOCKETS
                    | Flags::WITH_FLAG_APPEND
                    | Flags::WITH_FLAG_NOATIME
                    | Flags::WITH_FLAG_COMPR
                    | Flags::WITH_FLAG_NOCOW
                    | Flags::WITH_FLAG_NODUMP
                    | Flags::WITH_FLAG_DIRSYNC
                    | Flags::WITH_FLAG_IMMUTABLE
                    | Flags::WITH_FLAG_SYNC
                    | Flags::WITH_FLAG_NOCOMP
                    | Flags::WITH_XATTRS
                    | Flags::WITH_ACL
                    | Flags::WITH_SELINUX
                    | Flags::WITH_SUBVOLUME
                    | Flags::WITH_SUBVOLUME_RO
                    | Flags::WITH_FCAPS
            }
            TMPFS_MAGIC => {
                Flags::WITH_2SEC_TIME
                    | Flags::WITH_READ_ONLY
                    | Flags::WITH_PERMISSIONS
                    | Flags::WITH_SYMLINKS
                    | Flags::WITH_DEVICE_NODES
                    | Flags::WITH_FIFOS
                    | Flags::WITH_SOCKETS
                    | Flags::WITH_ACL
                    | Flags::WITH_SELINUX
            }
            // FUSE mounts are special as the supported feature set
            // is not clear a priori.
            FUSE_SUPER_MAGIC => Flags::WITH_FUSE,
            _ => {
                Flags::WITH_2SEC_TIME
                    | Flags::WITH_READ_ONLY
                    | Flags::WITH_PERMISSIONS
                    | Flags::WITH_SYMLINKS
                    | Flags::WITH_DEVICE_NODES
                    | Flags::WITH_FIFOS
                    | Flags::WITH_SOCKETS
                    | Flags::WITH_XATTRS
                    | Flags::WITH_ACL
                    | Flags::WITH_FCAPS
            }
        }
    }
}
