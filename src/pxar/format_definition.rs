//! *pxar* binary format definition
//!
//! Please note the all values are stored in little endian ordering.
//!
//! The Archive contains a list of items. Each item starts with a
//! `CaFormatHeader`, followed by the item data.

use failure::*;
use std::cmp::Ordering;
use endian_trait::Endian;

use siphasher::sip::SipHasher24;

pub const CA_FORMAT_ENTRY: u64 = 0x1396fabcea5bbb51;
pub const CA_FORMAT_FILENAME: u64 = 0x6dbb6ebcb3161f0b;
pub const CA_FORMAT_SYMLINK: u64 = 0x664a6fb6830e0d6c;
pub const CA_FORMAT_DEVICE: u64 = 0xac3dace369dfe643;
pub const CA_FORMAT_XATTR: u64 = 0xb8157091f80bc486;
pub const CA_FORMAT_FCAPS: u64 = 0xf7267db0afed0629;

// compute_goodbye_hash(b"__PROXMOX_FORMAT_HARDLINK__");
pub const PXAR_FORMAT_HARDLINK: u64 = 0x2c5e06f634f65b86;

pub const CA_FORMAT_PAYLOAD: u64 = 0x8b9e1d93d6dcffc9;

pub const CA_FORMAT_GOODBYE: u64 = 0xdfd35c5e8327c403;
/* The end marker used in the GOODBYE object */
pub const CA_FORMAT_GOODBYE_TAIL_MARKER: u64 = 0x57446fa533702943;


// Feature flags

/// restrict UIDs toö 16 bit
pub const CA_FORMAT_WITH_16BIT_UIDS: u64       = 0x1;
/// assume UIDs are 32 bit
pub const CA_FORMAT_WITH_32BIT_UIDS: u64       = 0x2;
/// include user and group name
pub const CA_FORMAT_WITH_USER_NAMES: u64       = 0x4;
pub const CA_FORMAT_WITH_SEC_TIME: u64         = 0x8;
pub const CA_FORMAT_WITH_USEC_TIME: u64        = 0x10;
pub const CA_FORMAT_WITH_NSEC_TIME: u64        = 0x20;
/// FAT-style 2s time granularity
pub const CA_FORMAT_WITH_2SEC_TIME: u64        = 0x40;
pub const CA_FORMAT_WITH_READ_ONLY: u64        = 0x80;
pub const CA_FORMAT_WITH_PERMISSIONS: u64      = 0x100;
/// include symbolik links
pub const CA_FORMAT_WITH_SYMLINKS: u64         = 0x200;
/// include device nodes
pub const CA_FORMAT_WITH_DEVICE_NODES: u64     = 0x400;
/// include FIFOs
pub const CA_FORMAT_WITH_FIFOS: u64            = 0x800;
/// include Sockets
pub const CA_FORMAT_WITH_SOCKETS: u64          = 0x1000;

/// DOS file flag `HIDDEN`
pub const CA_FORMAT_WITH_FLAG_HIDDEN: u64      = 0x2000;
/// DOS file flag `SYSTEM`
pub const CA_FORMAT_WITH_FLAG_SYSTEM: u64      = 0x4000;
/// DOS file flag `ARCHIVE`
pub const CA_FORMAT_WITH_FLAG_ARCHIVE: u64     = 0x8000;

// chattr() flags#
/// Linux file attribute `APPEND`
pub const CA_FORMAT_WITH_FLAG_APPEND: u64      = 0x10000;
/// Linux file attribute `NOATIME`
pub const CA_FORMAT_WITH_FLAG_NOATIME: u64     = 0x20000;
/// Linux file attribute `COMPR`
pub const CA_FORMAT_WITH_FLAG_COMPR: u64       = 0x40000;
/// Linux file attribute `NOCOW`
pub const CA_FORMAT_WITH_FLAG_NOCOW: u64       = 0x80000;
/// Linux file attribute `NODUMP`
pub const CA_FORMAT_WITH_FLAG_NODUMP: u64      = 0x100000;
/// Linux file attribute `DIRSYNC`
pub const CA_FORMAT_WITH_FLAG_DIRSYNC: u64     = 0x200000;
/// Linux file attribute `IMMUTABLE`
pub const CA_FORMAT_WITH_FLAG_IMMUTABLE: u64   = 0x400000;
/// Linux file attribute `SYNC`
pub const CA_FORMAT_WITH_FLAG_SYNC: u64        = 0x800000;
/// Linux file attribute `NOCOMP`
pub const CA_FORMAT_WITH_FLAG_NOCOMP: u64      = 0x1000000;
/// Linux file attribute `PROJINHERIT`
pub const CA_FORMAT_WITH_FLAG_PROJINHERIT: u64 = 0x2000000;


// Include BTRFS subvolume flag
pub const CA_FORMAT_WITH_SUBVOLUME: u64         = 0x4000000;
// Include BTRFS read-only subvolume flag
pub const CA_FORMAT_WITH_SUBVOLUME_RO: u64      = 0x8000000;

/// Include Extended Attribute metadata */
pub const CA_FORMAT_WITH_XATTRS: u64            = 0x10000000;
/// Include Access Control List metadata
pub const CA_FORMAT_WITH_ACL: u64               = 0x20000000;
/// Include SELinux security context
pub const CA_FORMAT_WITH_SELINUX: u64           = 0x40000000;
/// Include "security.capability" xattr
pub const CA_FORMAT_WITH_FCAPS: u64             = 0x80000000;

/// XFS/ext4 project quota ID
pub const CA_FORMAT_WITH_QUOTA_PROJID: u64      = 0x100000000;

/// Support ".caexclude" files
pub const CA_FORMAT_EXCLUDE_FILE: u64           = 0x1000000000000000;
/// the purpose of this flag is still unclear
pub const CA_FORMAT_SHA512_256: u64             = 0x2000000000000000;
/// Exclude submounts
pub const CA_FORMAT_EXCLUDE_SUBMOUNTS: u64      = 0x4000000000000000;
/// Exclude entries with chattr flag NODUMP
pub const CA_FORMAT_EXCLUDE_NODUMP: u64         = 0x8000000000000000;

pub const CA_FORMAT_DEFAULT: u64 =
CA_FORMAT_WITH_32BIT_UIDS |
CA_FORMAT_WITH_USER_NAMES |
CA_FORMAT_WITH_NSEC_TIME|
CA_FORMAT_WITH_SYMLINKS|
CA_FORMAT_WITH_DEVICE_NODES|
CA_FORMAT_WITH_FIFOS|
CA_FORMAT_WITH_SOCKETS|
CA_FORMAT_WITH_FLAG_HIDDEN|
CA_FORMAT_WITH_FLAG_SYSTEM|
CA_FORMAT_WITH_FLAG_ARCHIVE|
CA_FORMAT_WITH_FLAG_APPEND|
CA_FORMAT_WITH_FLAG_NOATIME|
CA_FORMAT_WITH_FLAG_COMPR|
CA_FORMAT_WITH_FLAG_NOCOW|
//CA_FORMAT_WITH_FLAG_NODUMP|
CA_FORMAT_WITH_FLAG_DIRSYNC|
CA_FORMAT_WITH_FLAG_IMMUTABLE|
CA_FORMAT_WITH_FLAG_SYNC|
CA_FORMAT_WITH_FLAG_NOCOMP|
CA_FORMAT_WITH_FLAG_PROJINHERIT|
CA_FORMAT_WITH_SUBVOLUME|
CA_FORMAT_WITH_SUBVOLUME_RO|
CA_FORMAT_WITH_XATTRS|
CA_FORMAT_WITH_ACL|
CA_FORMAT_WITH_SELINUX|
CA_FORMAT_WITH_FCAPS|
CA_FORMAT_WITH_QUOTA_PROJID |

CA_FORMAT_EXCLUDE_NODUMP|
CA_FORMAT_EXCLUDE_FILE|
CA_FORMAT_SHA512_256;

#[derive(Debug)]
#[derive(Endian)]
#[repr(C)]
pub struct CaFormatHeader {
    /// The size of the item, including the size of `CaFormatHeader`.
    pub size: u64,
    /// The item type (see `CA_FORMAT_` constants).
    pub htype: u64,
}

#[derive(Endian)]
#[repr(C)]
pub struct CaFormatEntry {
    pub mode: u64,
    pub flags: u64,
    pub uid: u64,
    pub gid: u64,
    pub mtime: u64,
}

#[derive(Endian)]
#[repr(C)]
pub struct CaFormatDevice {
    pub major: u64,
    pub minor: u64,
}

#[derive(Endian)]
#[repr(C)]
pub struct CaFormatGoodbyeItem {
    /// The offset from the start of the GOODBYE object to the start
    /// of the matching directory item (point to a FILENAME). The last
    /// GOODBYE item points to the start of the matching ENTRY
    /// object.
    pub offset: u64,
    /// The overall size of the directory item. The last GOODBYE item
    /// repeats the size of the GOODBYE item.
    pub size: u64,
    /// SipHash24 of the directory item name. The last GOODBYE item
    /// uses the special hash value `CA_FORMAT_GOODBYE_TAIL_MARKER`.
    pub hash: u64,
}


/// Helper function to extract file names from binary archive.
pub fn read_os_string(buffer: &[u8]) -> std::ffi::OsString {
    let len = buffer.len();

    use std::os::unix::ffi::OsStrExt;

    let name = if len > 0 && buffer[len-1] == 0 {
        std::ffi::OsStr::from_bytes(&buffer[0..len-1])
    } else {
        std::ffi::OsStr::from_bytes(&buffer)
    };

    name.into()
}

#[derive(Debug, Eq)]
#[repr(C)]
pub struct CaFormatXAttr {
    pub name: Vec<u8>,
    pub value: Vec<u8>,
}

impl Ord for CaFormatXAttr {
    fn cmp(&self, other: &CaFormatXAttr) -> Ordering {
        self.name.cmp(&other.name)
    }
}

impl PartialOrd for CaFormatXAttr {
    fn partial_cmp(&self, other: &CaFormatXAttr) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl PartialEq for CaFormatXAttr {
    fn eq(&self, other: &CaFormatXAttr) -> bool {
        self.name == other.name
    }
}

#[derive(Debug)]
#[repr(C)]
pub struct CaFormatFCaps {
    pub data: Vec<u8>,
}

/// Create SipHash values for goodby tables.
//pub fn compute_goodbye_hash(name: &std::ffi::CStr) -> u64 {
pub fn compute_goodbye_hash(name: &[u8]) -> u64 {

    use std::hash::Hasher;
    let mut hasher = SipHasher24::new_with_keys(0x8574442b0f1d84b3, 0x2736ed30d1c22ec1);
    hasher.write(name);
    hasher.finish()
}

pub fn check_ca_header<T>(head: &CaFormatHeader, htype: u64) -> Result<(), Error> {
    if head.htype != htype {
        bail!("got wrong header type ({:016x} != {:016x}", head.htype, htype);
    }
    if head.size != (std::mem::size_of::<T>() + std::mem::size_of::<CaFormatHeader>()) as u64 {
        bail!("got wrong header size for type {:016x}", htype);
    }

    Ok(())
}

// form /usr/include/linux/fs.h
const FS_APPEND_FL: u32 =      0x00000020;
const FS_NOATIME_FL: u32 =     0x00000080;
const FS_COMPR_FL: u32 =       0x00000004;
const FS_NOCOW_FL: u32 =       0x00800000;
const FS_NODUMP_FL: u32 =      0x00000040;
const FS_DIRSYNC_FL: u32 =     0x00010000;
const FS_IMMUTABLE_FL: u32 =   0x00000010;
const FS_SYNC_FL: u32 =        0x00000008;
const FS_NOCOMP_FL: u32 =      0x00000400;
const FS_PROJINHERIT_FL: u32 = 0x20000000;

static CHATTR_MAP: [(u64, u32); 10] = [
    ( CA_FORMAT_WITH_FLAG_APPEND,      FS_APPEND_FL      ),
    ( CA_FORMAT_WITH_FLAG_NOATIME,     FS_NOATIME_FL     ),
    ( CA_FORMAT_WITH_FLAG_COMPR,       FS_COMPR_FL       ),
    ( CA_FORMAT_WITH_FLAG_NOCOW,       FS_NOCOW_FL       ),
    ( CA_FORMAT_WITH_FLAG_NODUMP,      FS_NODUMP_FL      ),
    ( CA_FORMAT_WITH_FLAG_DIRSYNC,     FS_DIRSYNC_FL     ),
    ( CA_FORMAT_WITH_FLAG_IMMUTABLE,   FS_IMMUTABLE_FL   ),
    ( CA_FORMAT_WITH_FLAG_SYNC,        FS_SYNC_FL        ),
    ( CA_FORMAT_WITH_FLAG_NOCOMP,      FS_NOCOMP_FL      ),
    ( CA_FORMAT_WITH_FLAG_PROJINHERIT, FS_PROJINHERIT_FL ),
];

pub fn ca_feature_flags_from_chattr(attr: u32) -> u64 {

    let mut flags = 0u64;

    for (ca_flag, fs_flag) in &CHATTR_MAP {
        if (attr & fs_flag) != 0 { flags = flags | ca_flag; }
    }

    flags
}



// from /usr/include/linux/msdos_fs.h
const ATTR_HIDDEN: u32 =      2;
const ATTR_SYS: u32 =         4;
const ATTR_ARCH: u32 =       32;

static FAT_ATTR_MAP: [(u64, u32); 3] = [
    ( CA_FORMAT_WITH_FLAG_HIDDEN, ATTR_HIDDEN ),
    ( CA_FORMAT_WITH_FLAG_SYSTEM, ATTR_SYS ),
    ( CA_FORMAT_WITH_FLAG_ARCHIVE,  ATTR_ARCH ),
];

pub fn ca_feature_flags_from_fat_attr(attr: u32) -> u64 {

    let mut flags = 0u64;

    for (ca_flag, fs_flag) in &FAT_ATTR_MAP {
        if (attr & fs_flag) != 0 { flags = flags | ca_flag; }
    }

    flags
}
