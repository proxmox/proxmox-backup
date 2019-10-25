//! *pxar* binary format definition
//!
//! Please note the all values are stored in little endian ordering.
//!
//! The Archive contains a list of items. Each item starts with a
//! `PxarHeader`, followed by the item data.
use std::cmp::Ordering;

use endian_trait::Endian;
use failure::{bail, Error};
use siphasher::sip::SipHasher24;


/// Header types identifying items stored in the archive
pub const PXAR_ENTRY: u64 = 0x1396fabcea5bbb51;
pub const PXAR_FILENAME: u64 = 0x6dbb6ebcb3161f0b;
pub const PXAR_SYMLINK: u64 = 0x664a6fb6830e0d6c;
pub const PXAR_DEVICE: u64 = 0xac3dace369dfe643;
pub const PXAR_XATTR: u64 = 0xb8157091f80bc486;
pub const PXAR_ACL_USER: u64 = 0x297dc88b2ef12faf;
pub const PXAR_ACL_GROUP: u64 = 0x36f2acb56cb3dd0b;
pub const PXAR_ACL_GROUP_OBJ: u64 = 0x23047110441f38f3;
pub const PXAR_ACL_DEFAULT: u64 = 0xfe3eeda6823c8cd0;
pub const PXAR_ACL_DEFAULT_USER: u64 = 0xbdf03df9bd010a91;
pub const PXAR_ACL_DEFAULT_GROUP: u64 = 0xa0cb1168782d1f51;
pub const PXAR_FCAPS: u64 = 0xf7267db0afed0629;
pub const PXAR_QUOTA_PROJID: u64 = 0x161baf2d8772a72b;

/// Marks item as hardlink
/// compute_goodbye_hash(b"__PROXMOX_FORMAT_HARDLINK__");
pub const PXAR_FORMAT_HARDLINK: u64 = 0x2c5e06f634f65b86;
/// Marks the beginnig of the payload (actual content) of regular files
pub const PXAR_PAYLOAD: u64 = 0x8b9e1d93d6dcffc9;
/// Marks item as entry of goodbye table
pub const PXAR_GOODBYE: u64 = 0xdfd35c5e8327c403;
/// The end marker used in the GOODBYE object
pub const PXAR_GOODBYE_TAIL_MARKER: u64 = 0x57446fa533702943;

#[derive(Debug, Endian)]
#[repr(C)]
pub struct PxarHeader {
    /// The item type (see `PXAR_` constants).
    pub htype: u64,
    /// The size of the item, including the size of `PxarHeader`.
    pub size: u64,
}

#[derive(Endian)]
#[repr(C)]
pub struct PxarEntry {
    pub mode: u64,
    pub flags: u64,
    pub uid: u32,
    pub gid: u32,
    pub mtime: u64,
}

#[derive(Endian)]
#[repr(C)]
pub struct PxarDevice {
    pub major: u64,
    pub minor: u64,
}

#[derive(Endian)]
#[repr(C)]
pub struct PxarGoodbyeItem {
    /// SipHash24 of the directory item name. The last GOODBYE item
    /// uses the special hash value `PXAR_GOODBYE_TAIL_MARKER`.
    pub hash: u64,
    /// The offset from the start of the GOODBYE object to the start
    /// of the matching directory item (point to a FILENAME). The last
    /// GOODBYE item points to the start of the matching ENTRY
    /// object.
    pub offset: u64,
    /// The overall size of the directory item. The last GOODBYE item
    /// repeats the size of the GOODBYE item.
    pub size: u64,
}

/// Helper function to extract file names from binary archive.
pub fn read_os_string(buffer: &[u8]) -> std::ffi::OsString {
    let len = buffer.len();

    use std::os::unix::ffi::OsStrExt;

    let name = if len > 0 && buffer[len - 1] == 0 {
        std::ffi::OsStr::from_bytes(&buffer[0..len - 1])
    } else {
        std::ffi::OsStr::from_bytes(&buffer)
    };

    name.into()
}

#[derive(Debug, Eq)]
#[repr(C)]
pub struct PxarXAttr {
    pub name: Vec<u8>,
    pub value: Vec<u8>,
}

impl Ord for PxarXAttr {
    fn cmp(&self, other: &PxarXAttr) -> Ordering {
        self.name.cmp(&other.name)
    }
}

impl PartialOrd for PxarXAttr {
    fn partial_cmp(&self, other: &PxarXAttr) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl PartialEq for PxarXAttr {
    fn eq(&self, other: &PxarXAttr) -> bool {
        self.name == other.name
    }
}

#[derive(Debug)]
#[repr(C)]
pub struct PxarFCaps {
    pub data: Vec<u8>,
}

#[derive(Debug, Endian, Eq)]
#[repr(C)]
pub struct PxarACLUser {
    pub uid: u64,
    pub permissions: u64,
    //pub name: Vec<u64>, not impl for now
}

// TODO if also name is impl, sort by uid, then by name and last by permissions
impl Ord for PxarACLUser {
    fn cmp(&self, other: &PxarACLUser) -> Ordering {
        match self.uid.cmp(&other.uid) {
            // uids are equal, entries ordered by permissions
            Ordering::Equal => self.permissions.cmp(&other.permissions),
            // uids are different, entries ordered by uid
            uid_order => uid_order,
        }
    }
}

impl PartialOrd for PxarACLUser {
    fn partial_cmp(&self, other: &PxarACLUser) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl PartialEq for PxarACLUser {
    fn eq(&self, other: &PxarACLUser) -> bool {
        self.uid == other.uid && self.permissions == other.permissions
    }
}

#[derive(Debug, Endian, Eq)]
#[repr(C)]
pub struct PxarACLGroup {
    pub gid: u64,
    pub permissions: u64,
    //pub name: Vec<u64>, not impl for now
}

// TODO if also name is impl, sort by gid, then by name and last by permissions
impl Ord for PxarACLGroup {
    fn cmp(&self, other: &PxarACLGroup) -> Ordering {
        match self.gid.cmp(&other.gid) {
            // gids are equal, entries are ordered by permissions
            Ordering::Equal => self.permissions.cmp(&other.permissions),
            // gids are different, entries ordered by gid
            gid_ordering => gid_ordering,
        }
    }
}

impl PartialOrd for PxarACLGroup {
    fn partial_cmp(&self, other: &PxarACLGroup) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl PartialEq for PxarACLGroup {
    fn eq(&self, other: &PxarACLGroup) -> bool {
        self.gid == other.gid && self.permissions == other.permissions
    }
}

#[derive(Debug, Endian)]
#[repr(C)]
pub struct PxarACLGroupObj {
    pub permissions: u64,
}

#[derive(Debug, Endian)]
#[repr(C)]
pub struct PxarACLDefault {
    pub user_obj_permissions: u64,
    pub group_obj_permissions: u64,
    pub other_permissions: u64,
    pub mask_permissions: u64,
}

pub(crate) struct PxarACL {
    pub users: Vec<PxarACLUser>,
    pub groups: Vec<PxarACLGroup>,
    pub group_obj: Option<PxarACLGroupObj>,
    pub default: Option<PxarACLDefault>,
}

pub const PXAR_ACL_PERMISSION_READ: u64 = 4;
pub const PXAR_ACL_PERMISSION_WRITE: u64 = 2;
pub const PXAR_ACL_PERMISSION_EXECUTE: u64 = 1;

#[derive(Debug, Endian)]
#[repr(C)]
pub struct PxarQuotaProjID {
    pub projid: u64,
}

#[derive(Debug, Default)]
pub struct PxarAttributes {
    pub xattrs: Vec<PxarXAttr>,
    pub fcaps: Option<PxarFCaps>,
    pub quota_projid: Option<PxarQuotaProjID>,
    pub acl_user: Vec<PxarACLUser>,
    pub acl_group: Vec<PxarACLGroup>,
    pub acl_group_obj: Option<PxarACLGroupObj>,
    pub acl_default: Option<PxarACLDefault>,
    pub acl_default_user: Vec<PxarACLUser>,
    pub acl_default_group: Vec<PxarACLGroup>,
}

/// Create SipHash values for goodby tables.
//pub fn compute_goodbye_hash(name: &std::ffi::CStr) -> u64 {
pub fn compute_goodbye_hash(name: &[u8]) -> u64 {
    use std::hash::Hasher;
    let mut hasher = SipHasher24::new_with_keys(0x8574442b0f1d84b3, 0x2736ed30d1c22ec1);
    hasher.write(name);
    hasher.finish()
}

pub fn check_ca_header<T>(head: &PxarHeader, htype: u64) -> Result<(), Error> {
    if head.htype != htype {
        bail!(
            "got wrong header type ({:016x} != {:016x})",
            head.htype,
            htype
        );
    }
    if head.size != (std::mem::size_of::<T>() + std::mem::size_of::<PxarHeader>()) as u64 {
        bail!("got wrong header size for type {:016x}", htype);
    }

    Ok(())
}
