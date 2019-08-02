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
pub const CA_FORMAT_ACL_USER: u64 = 0x297dc88b2ef12faf;
pub const CA_FORMAT_ACL_GROUP: u64 = 0x36f2acb56cb3dd0b;
pub const CA_FORMAT_ACL_GROUP_OBJ: u64 = 0x23047110441f38f3;
pub const CA_FORMAT_ACL_DEFAULT: u64 = 0xfe3eeda6823c8cd0;
pub const CA_FORMAT_ACL_DEFAULT_USER: u64 = 0xbdf03df9bd010a91;
pub const CA_FORMAT_ACL_DEFAULT_GROUP: u64 = 0xa0cb1168782d1f51;
pub const CA_FORMAT_FCAPS: u64 = 0xf7267db0afed0629;
pub const CA_FORMAT_QUOTA_PROJID:u64 = 0x161baf2d8772a72b;

// compute_goodbye_hash(b"__PROXMOX_FORMAT_HARDLINK__");
pub const PXAR_FORMAT_HARDLINK: u64 = 0x2c5e06f634f65b86;

pub const CA_FORMAT_PAYLOAD: u64 = 0x8b9e1d93d6dcffc9;

pub const CA_FORMAT_GOODBYE: u64 = 0xdfd35c5e8327c403;
/* The end marker used in the GOODBYE object */
pub const CA_FORMAT_GOODBYE_TAIL_MARKER: u64 = 0x57446fa533702943;


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

#[derive(Debug, Endian, Eq)]
#[repr(C)]
pub struct CaFormatACLUser {
    pub uid: u64,
    pub permissions: u64,
    //pub name: Vec<u64>, not impl for now
}

// TODO if also name is impl, sort by uid, then by name and last by permissions
impl Ord for CaFormatACLUser {
    fn cmp(&self, other: &CaFormatACLUser) -> Ordering {
        match self.uid.cmp(&other.uid) {
            // uids are equal, entries ordered by permissions
            Ordering::Equal => self.permissions.cmp(&other.permissions),
            // uids are different, entries ordered by uid
            uid_order => uid_order,
        }
    }
}

impl PartialOrd for CaFormatACLUser {
    fn partial_cmp(&self, other: &CaFormatACLUser) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl PartialEq for CaFormatACLUser {
    fn eq(&self, other: &CaFormatACLUser) -> bool {
        self.uid == other.uid && self.permissions == other.permissions
    }
}

#[derive(Debug, Endian, Eq)]
#[repr(C)]
pub struct CaFormatACLGroup {
    pub gid: u64,
    pub permissions: u64,
    //pub name: Vec<u64>, not impl for now
}

// TODO if also name is impl, sort by gid, then by name and last by permissions
impl Ord for CaFormatACLGroup {
    fn cmp(&self, other: &CaFormatACLGroup) -> Ordering {
        match self.gid.cmp(&other.gid) {
            // gids are equal, entries are ordered by permissions
            Ordering::Equal => self.permissions.cmp(&other.permissions),
            // gids are different, entries ordered by gid
            gid_ordering => gid_ordering,
        }
    }
}

impl PartialOrd for CaFormatACLGroup {
    fn partial_cmp(&self, other: &CaFormatACLGroup) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl PartialEq for CaFormatACLGroup {
    fn eq(&self, other: &CaFormatACLGroup) -> bool {
        self.gid == other.gid && self.permissions == other.permissions
    }
}

#[derive(Debug, Endian)]
#[repr(C)]
pub struct CaFormatACLGroupObj {
    pub permissions: u64,
}

#[derive(Debug, Endian)]
#[repr(C)]
pub struct CaFormatACLDefault {
    pub user_obj_permissions: u64,
    pub group_obj_permissions: u64,
    pub other_permissions: u64,
    pub mask_permissions: u64,
}

pub (crate) struct PxarACL {
    pub users: Vec<CaFormatACLUser>,
    pub groups: Vec<CaFormatACLGroup>,
    pub group_obj: Option<CaFormatACLGroupObj>,
    pub default: Option<CaFormatACLDefault>,
}

pub const CA_FORMAT_ACL_PERMISSION_READ: u64 = 4;
pub const CA_FORMAT_ACL_PERMISSION_WRITE: u64 = 2;
pub const CA_FORMAT_ACL_PERMISSION_EXECUTE: u64 = 1;

#[derive(Debug, Endian)]
#[repr(C)]
pub struct CaFormatQuotaProjID {
    pub projid: u64,
}

#[derive(Debug, Default)]
pub struct PxarAttributes {
    pub xattrs: Vec<CaFormatXAttr>,
    pub fcaps: Option<CaFormatFCaps>,
    pub quota_projid: Option<CaFormatQuotaProjID>,
    pub acl_user: Vec<CaFormatACLUser>,
    pub acl_group: Vec<CaFormatACLGroup>,
    pub acl_group_obj: Option<CaFormatACLGroupObj>,
    pub acl_default: Option<CaFormatACLDefault>,
    pub acl_default_user: Vec<CaFormatACLUser>,
    pub acl_default_group: Vec<CaFormatACLGroup>,
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
        bail!("got wrong header type ({:016x} != {:016x})", head.htype, htype);
    }
    if head.size != (std::mem::size_of::<T>() + std::mem::size_of::<CaFormatHeader>()) as u64 {
        bail!("got wrong header size for type {:016x}", htype);
    }

    Ok(())
}

