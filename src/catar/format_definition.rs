use failure::*;

const CA_FORMAT_ENTRY: u64 = 0x1396fabcea5bbb51;
const CA_FORMAT_FILENAME: u64 = 0x6dbb6ebcb3161f0b;

const CA_FORMAT_GOODBYE: u64 = 0xdfd35c5e8327c403;
/* The end marker used in the GOODBYE object */
const CA_FORMAT_GOODBYE_TAIL_MARKER: u64 = 0x57446fa533702943;

#[repr(C)]
pub struct CaFormatHeader {
    size: u64,
    htype: u64,
}

#[repr(C)]
pub struct CaFormatEntry {
    feature_flags: u64,
    mode: u64,
    flags: u64,
    uid: u64,
    gid: u64,
    mtime: u64,
}

#[repr(C)]
pub struct CaFormatGoodbyeItem {
    offset: u64,
    size: u64,
    hash: u64,
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
