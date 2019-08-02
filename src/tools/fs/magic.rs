//! Filesystem related magic numbers

// from /usr/include/linux/magic.h
// and from casync util.h
pub const BINFMTFS_MAGIC: i64 =        0x42494e4d;
pub const CGROUP2_SUPER_MAGIC: i64 =   0x63677270;
pub const CGROUP_SUPER_MAGIC: i64 =    0x0027e0eb;
pub const CONFIGFS_MAGIC: i64 =        0x62656570;
pub const DEBUGFS_MAGIC: i64 =         0x64626720;
pub const DEVPTS_SUPER_MAGIC: i64 =    0x00001cd1;
pub const EFIVARFS_MAGIC: i64 =        0xde5e81e4;
pub const FUSE_CTL_SUPER_MAGIC: i64 =  0x65735543;
pub const HUGETLBFS_MAGIC: i64 =       0x958458f6;
pub const MQUEUE_MAGIC: i64 =          0x19800202;
pub const NFSD_MAGIC: i64 =            0x6e667364;
pub const PROC_SUPER_MAGIC: i64 =      0x00009fa0;
pub const PSTOREFS_MAGIC: i64 =        0x6165676C;
pub const RPCAUTH_GSSMAGIC: i64 =      0x67596969;
pub const SECURITYFS_MAGIC: i64 =      0x73636673;
pub const SELINUX_MAGIC: i64 =         0xf97cff8c;
pub const SMACK_MAGIC: i64 =           0x43415d53;
pub const RAMFS_MAGIC: i64 =           0x858458f6;
pub const TMPFS_MAGIC: i64 =           0x01021994;
pub const SYSFS_MAGIC: i64 =           0x62656572;
pub const MSDOS_SUPER_MAGIC: i64 =     0x00004d44;
pub const BTRFS_SUPER_MAGIC: i64 =     0x9123683E;
pub const FUSE_SUPER_MAGIC: i64 =      0x65735546;
pub const EXT4_SUPER_MAGIC: i64 =      0x0000EF53;
pub const XFS_SUPER_MAGIC: i64 =       0x58465342;
pub const ZFS_SUPER_MAGIC: i64 =       0x2FC12FC1;
