//! Wrapper functions for the libc xattr calls

extern crate libc;

use std::os::unix::io::RawFd;
use nix::errno::Errno;
use crate::pxar::{CaFormatXAttr, CaFormatFCaps};

pub fn flistxattr(fd: RawFd) -> Result<Vec<u8>, nix::errno::Errno> {
    // Initial buffer size for the attribute list, if content does not fit
    // it gets dynamically increased until big enough.
    let mut size = 256;
    let mut buffer = vec![0u8; size];
    let mut bytes = unsafe {
        libc::flistxattr(fd, buffer.as_mut_ptr() as *mut i8, buffer.len())
    };
    while bytes < 0 {
        let err = Errno::last();
        match err {
            Errno::ERANGE => {
                // Buffer was not big enough to fit the list, retry with double the size
                if size * 2 < size { return Err(Errno::ENOMEM); }
                size *= 2;
            },
            _ => return Err(err),
        }
        // Retry to read the list with new buffer
        buffer.resize(size, 0);
        bytes = unsafe {
            libc::flistxattr(fd, buffer.as_mut_ptr() as *mut i8, buffer.len())
        };
    }
    buffer.resize(bytes as usize, 0);

    Ok(buffer)
}

pub fn fgetxattr(fd: RawFd, name: &[u8]) -> Result<Vec<u8>, nix::errno::Errno> {
    let mut size = 256;
    let mut buffer = vec![0u8; size];
    let mut bytes = unsafe {
        libc::fgetxattr(fd, name.as_ptr() as *const i8, buffer.as_mut_ptr() as *mut core::ffi::c_void, buffer.len())
    };
    while bytes < 0 {
        let err = Errno::last();
        match err {
            Errno::ERANGE => {
                // Buffer was not big enough to fit the value, retry with double the size
                if size * 2 < size { return Err(Errno::ENOMEM); }
                size *= 2;
            },
            _ => return Err(err),
        }
        buffer.resize(size, 0);
        bytes = unsafe {
            libc::fgetxattr(fd, name.as_ptr() as *const i8, buffer.as_mut_ptr() as *mut core::ffi::c_void, buffer.len())
        };
    }
    buffer.resize(bytes as usize, 0);

    Ok(buffer)
}

pub fn fsetxattr(fd: RawFd, xattr: CaFormatXAttr) -> Result<(), nix::errno::Errno> {
    let mut name = xattr.name.clone();
    name.push('\0' as u8);
    let flags = 0 as libc::c_int;
    let result = unsafe {
        libc::fsetxattr(fd, name.as_ptr() as *const libc::c_char, xattr.value.as_ptr() as *const libc::c_void, xattr.value.len(), flags)
    };
    if result < 0 {
        let err = Errno::last();
        return Err(err);
    }

    Ok(())
}

pub fn fsetxattr_fcaps(fd: RawFd, fcaps: CaFormatFCaps) -> Result<(), nix::errno::Errno> {
    // TODO casync checks and removes capabilities if they are set
    let name = b"security.capability\0";
    let flags = 0 as libc::c_int;
    let result = unsafe {
        libc::fsetxattr(fd, name.as_ptr() as *const libc::c_char, fcaps.data.as_ptr() as *const libc::c_void, fcaps.data.len(), flags)
    };
    if result < 0 {
        let err = Errno::last();
        return Err(err);
    }

    Ok(())
}

