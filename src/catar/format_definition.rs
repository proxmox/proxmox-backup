//! *catar* binary format definition
//!
//! Please note the all values are stored in little endian ordering.
//!
//! The Archive contains a list of items. Each item starts with a
//! `CaFormatHeader`, followed by the item data.

use failure::*;

pub const CA_FORMAT_ENTRY: u64 = 0x1396fabcea5bbb51;
pub const CA_FORMAT_FILENAME: u64 = 0x6dbb6ebcb3161f0b;
pub const CA_FORMAT_SYMLINK: u64 = 0x664a6fb6830e0d6c;
pub const CA_FORMAT_PAYLOAD: u64 = 0x8b9e1d93d6dcffc9;

pub const CA_FORMAT_GOODBYE: u64 = 0xdfd35c5e8327c403;
/* The end marker used in the GOODBYE object */
pub const CA_FORMAT_GOODBYE_TAIL_MARKER: u64 = 0x57446fa533702943;

pub const CA_FORMAT_FEATURE_FLAGS_MAX: u64 = 0xb000_0001_ffef_fe26; // fixme: ?

#[repr(C)]
pub struct CaFormatHeader {
    /// The size of the item, including the size of `CaFormatHeader`.
    pub size: u64,
    /// The item type (see `CA_FORMAT_` constants).
    pub htype: u64,
}

#[repr(C)]
pub struct CaFormatEntry {
    pub feature_flags: u64,
    pub mode: u64,
    pub flags: u64,
    pub uid: u64,
    pub gid: u64,
    pub mtime: u64,
}

#[repr(C)]
pub struct CaFormatGoodbyeItem {
    /// The offset from the start of the GOODBYE object to the start
    /// of the matching directory item (point to a FILENAME). The last
    /// GOODBYE item points to the start of the matching ENTRY
    /// object. repeats the `size`
    pub offset: u64,
    /// The overall size of the directory item. The last GOODBYE item
    /// repeats the size of the GOODBYE item.
    pub size: u64,
    /// SipHash24 of the directory item name. The last GOODBYE item
    /// uses the special hash value `CA_FORMAT_GOODBYE_TAIL_MARKER`.
    pub hash: u64,
}

fn read_os_string(buffer: &[u8]) -> std::ffi::OsString {
    let len = buffer.len();

    use std::os::unix::ffi::OsStrExt;

    let name = if len > 0 && buffer[len-1] == 0 {
        std::ffi::OsStr::from_bytes(&buffer[0..len-1])
    } else {
        std::ffi::OsStr::from_bytes(&buffer)
    };

    name.into()
}
