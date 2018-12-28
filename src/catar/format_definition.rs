use failure::*;

pub const CA_FORMAT_ENTRY: u64 = 0x1396fabcea5bbb51;
pub const CA_FORMAT_FILENAME: u64 = 0x6dbb6ebcb3161f0b;
pub const CA_FORMAT_SYMLINK: u64 = 0x664a6fb6830e0d6c;
 
pub const CA_FORMAT_GOODBYE: u64 = 0xdfd35c5e8327c403;
/* The end marker used in the GOODBYE object */
pub const CA_FORMAT_GOODBYE_TAIL_MARKER: u64 = 0x57446fa533702943;

#[repr(C)]
pub struct CaFormatHeader {
    pub size: u64,
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
    pub offset: u64,
    pub size: u64,
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
