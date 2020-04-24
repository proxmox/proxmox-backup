//! Wrapper functions for the libc xattr calls

use std::ffi::CStr;
use std::os::unix::io::RawFd;

use nix::errno::Errno;

use proxmox::tools::vec;

use crate::pxar::{PxarXAttr, PxarFCaps};

/// Result of `flistxattr`, allows iterating over the attributes as a list of `&CStr`s.
///
/// Listing xattrs produces a list separated by zeroes, inherently making them available as `&CStr`
/// already, so we make use of this fact and reflect this in the interface.
pub struct ListXAttr {
    data: Vec<u8>,
}

impl ListXAttr {
    fn new(data: Vec<u8>) -> Self {
        Self { data }
    }
}

impl<'a> IntoIterator for &'a ListXAttr {
    type Item = &'a CStr;
    type IntoIter = ListXAttrIter<'a>;

    fn into_iter(self) -> Self::IntoIter {
        ListXAttrIter {
            data: &self.data,
            at: 0,
        }
    }
}

/// Iterator over the extended attribute entries in a `ListXAttr`.
pub struct ListXAttrIter<'a> {
    data: &'a [u8],
    at: usize,
}

impl<'a> Iterator for ListXAttrIter<'a> {
    type Item = &'a CStr;

    fn next(&mut self) -> Option<&'a CStr> {
        let data = &self.data[self.at..];
        let next = data.iter().position(|b| *b == 0)? + 1;
        self.at += next;
        Some(unsafe { CStr::from_bytes_with_nul_unchecked(&data[..next]) })
    }
}

/// Return a list of extended attributes accessible as an iterator over items of type `&CStr`.
pub fn flistxattr(fd: RawFd) -> Result<ListXAttr, nix::errno::Errno> {
    // Initial buffer size for the attribute list, if content does not fit
    // it gets dynamically increased until big enough.
    let mut size = 256;
    let mut buffer = vec::undefined(size);
    let mut bytes = unsafe {
        libc::flistxattr(fd, buffer.as_mut_ptr() as *mut i8, buffer.len())
    };
    while bytes < 0 {
        let err = Errno::last();
        match err {
            Errno::ERANGE => {
                // Buffer was not big enough to fit the list, retry with double the size
                size = size.checked_mul(2).ok_or(Errno::ENOMEM)?;
            },
            _ => return Err(err),
        }
        // Retry to read the list with new buffer
        buffer.resize(size, 0);
        bytes = unsafe {
            libc::flistxattr(fd, buffer.as_mut_ptr() as *mut i8, buffer.len())
        };
    }
    buffer.truncate(bytes as usize);

    Ok(ListXAttr::new(buffer))
}

/// Get an extended attribute by name.
///
/// Extended attributes may not contain zeroes, which we enforce in the API by using a `&CStr`
/// type.
pub fn fgetxattr(fd: RawFd, name: &CStr) -> Result<Vec<u8>, nix::errno::Errno> {
    let mut size = 256;
    let mut buffer = vec::undefined(size);
    let mut bytes = unsafe {
        libc::fgetxattr(fd, name.as_ptr(), buffer.as_mut_ptr() as *mut core::ffi::c_void, buffer.len())
    };
    while bytes < 0 {
        let err = Errno::last();
        match err {
            Errno::ERANGE => {
                // Buffer was not big enough to fit the value, retry with double the size
                size = size.checked_mul(2).ok_or(Errno::ENOMEM)?;
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

pub fn fsetxattr(fd: RawFd, xattr: &PxarXAttr) -> Result<(), nix::errno::Errno> {
    let mut name = xattr.name.clone();
    name.push(b'\0');
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

pub fn fsetxattr_fcaps(fd: RawFd, fcaps: &PxarFCaps) -> Result<(), nix::errno::Errno> {
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

pub fn is_security_capability(name: &CStr) -> bool {
    name.to_bytes() == b"security.capability"
}

/// Check if the passed name buffer starts with a valid xattr namespace prefix
/// and is within the length limit of 255 bytes
pub fn is_valid_xattr_name(c_name: &CStr) -> bool {
    let name = c_name.to_bytes();
    if name.is_empty() || name.len() > 255 {
        return false;
    }
    if name.starts_with(b"user.") || name.starts_with(b"trusted.") {
        return true;
    }
    is_security_capability(c_name)
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::fs::OpenOptions;
    use std::os::unix::io::AsRawFd;
    use nix::errno::Errno;

    #[test]
    fn test_fsetxattr_fgetxattr() {
        use proxmox::c_str;

        let path = "./tests/xattrs.txt";
        let file = OpenOptions::new()
            .write(true)
            .create(true)
            .open(&path)
            .unwrap();

        let fd = file.as_raw_fd();

        let valid_user = PxarXAttr {
            name: b"user.attribute0".to_vec(),
            value: b"value0".to_vec(),
        };

        let valid_empty_value = PxarXAttr {
            name: b"user.empty".to_vec(),
            value: Vec::new(),
        };

        let invalid_trusted = PxarXAttr {
            name: b"trusted.attribute0".to_vec(),
            value: b"value0".to_vec(),
        };

        let invalid_name_prefix = PxarXAttr {
            name: b"users.attribte0".to_vec(),
            value: b"value".to_vec(),
        };

        let mut name = b"user.".to_vec();
        for _ in 0..260 {
            name.push(b'a');
        }

        let invalid_name_length = PxarXAttr {
            name,
            value: b"err".to_vec(),
        };

        assert!(fsetxattr(fd, &valid_user).is_ok());
        assert!(fsetxattr(fd, &valid_empty_value).is_ok());

        if nix::unistd::Uid::current() != nix::unistd::ROOT {
            assert_eq!(fsetxattr(fd, &invalid_trusted), Err(Errno::EPERM));
        }

        assert_eq!(fsetxattr(fd, &invalid_name_prefix), Err(Errno::EOPNOTSUPP));
        assert_eq!(fsetxattr(fd, &invalid_name_length), Err(Errno::ERANGE));

        let v0 = fgetxattr(fd, c_str!("user.attribute0")).unwrap();
        let v1 = fgetxattr(fd, c_str!("user.empty")).unwrap();

        assert_eq!(v0, b"value0".as_ref());
        assert_eq!(v1, b"".as_ref());
        assert_eq!(fgetxattr(fd, c_str!("user.attribute1")), Err(Errno::ENODATA));

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn test_is_valid_xattr_name() {
        use std::ffi::CString;

        use proxmox::c_str;

        let too_long = CString::new(vec![b'a'; 265]).unwrap();

        assert!(!is_valid_xattr_name(&too_long));
        assert!(!is_valid_xattr_name(c_str!("system.attr")));
        assert!(is_valid_xattr_name(c_str!("user.attr")));
        assert!(is_valid_xattr_name(c_str!("trusted.attr")));
        assert!(is_valid_xattr_name(c_str!("security.capability")));
    }
}
