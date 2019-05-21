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

pub fn security_capability(name: &[u8]) -> bool {
    name == b"security.capability"
}

pub fn name_store(name: &[u8]) -> bool {
    if name.is_empty() { return false; }
    if name.starts_with(b"user.") { return true; }
    if name.starts_with(b"trusted.") { return true; }
    if security_capability(name) { return true; }

    false
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::fs::OpenOptions;
    use std::os::unix::io::AsRawFd;
    use nix::errno::Errno;

    #[test]
    fn test_fsetxattr_fgetxattr() {
        let path = "./tests/xattrs.txt";
        let file = OpenOptions::new()
            .write(true)
            .create(true)
            .open(&path)
            .unwrap();

        let fd = file.as_raw_fd();

        let valid_user = CaFormatXAttr {
            name: b"user.attribute0".to_vec(),
            value: b"value0".to_vec(),
        };

        let valid_empty_value = CaFormatXAttr {
            name: b"user.empty".to_vec(),
            value: Vec::new(),
        };

        let invalid_trusted = CaFormatXAttr {
            name: b"trusted.attribute0".to_vec(),
            value: b"value0".to_vec(),
        };

        let invalid_name_prefix = CaFormatXAttr {
            name: b"users.attribte0".to_vec(),
            value: b"value".to_vec(),
        };

        let mut name = b"user.".to_vec();
        for _ in 0..260 {
            name.push(b'a');
        }

        let invalid_name_length = CaFormatXAttr {
            name: name,
            value: b"err".to_vec(),
        };

        assert!(fsetxattr(fd, valid_user).is_ok());
        assert!(fsetxattr(fd, valid_empty_value).is_ok());
        assert_eq!(fsetxattr(fd, invalid_trusted), Err(Errno::EPERM));
        assert_eq!(fsetxattr(fd, invalid_name_prefix), Err(Errno::EOPNOTSUPP));
        assert_eq!(fsetxattr(fd, invalid_name_length), Err(Errno::ERANGE));

        let v0 = fgetxattr(fd, b"user.attribute0\0".as_ref()).unwrap();
        let v1 = fgetxattr(fd, b"user.empty\0".as_ref()).unwrap();

        assert_eq!(v0, b"value0".as_ref());
        assert_eq!(v1, b"".as_ref());
        assert_eq!(fgetxattr(fd, b"user.attribute1\0".as_ref()), Err(Errno::ENODATA));

        std::fs::remove_file(&path).unwrap();
    }
}
