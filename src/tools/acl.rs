//! Implementation of the calls to handle POSIX access control lists

// see C header file <sys/acl.h> for reference
extern crate libc;

use std::ffi::CString;
use std::marker::PhantomData;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::io::RawFd;
use std::path::Path;
use std::ptr;

use libc::{c_char, c_int, c_uint, c_void};
use nix::errno::Errno;

// acl_perm_t values
pub type ACLPerm = c_uint;
pub const ACL_READ: ACLPerm     = 0x04;
pub const ACL_WRITE: ACLPerm    = 0x02;
pub const ACL_EXECUTE: ACLPerm  = 0x01;

// acl_tag_t values
pub type ACLTag = c_int;
pub const ACL_UNDEFINED_TAG: ACLTag = 0x00;
pub const ACL_USER_OBJ: ACLTag      = 0x01;
pub const ACL_USER: ACLTag          = 0x02;
pub const ACL_GROUP_OBJ: ACLTag     = 0x04;
pub const ACL_GROUP: ACLTag         = 0x08;
pub const ACL_MASK: ACLTag          = 0x10;
pub const ACL_OTHER: ACLTag         = 0x20;

// acl_type_t values
pub type ACLType = c_uint;
pub const ACL_TYPE_ACCESS: ACLType  = 0x8000;
pub const ACL_TYPE_DEFAULT: ACLType = 0x4000;

// acl entry constants
pub const ACL_FIRST_ENTRY: c_int = 0;
pub const ACL_NEXT_ENTRY: c_int  = 1;

#[link(name = "acl")]
extern "C" {
    fn acl_get_file(path: *const c_char, acl_type: ACLType) -> *mut c_void;
    fn acl_set_file(path: *const c_char, acl_type: ACLType, acl: *mut c_void) -> c_int;
    fn acl_get_fd(fd: RawFd) -> *mut c_void;
    fn acl_get_entry(acl: *const c_void, entry_id: c_int, entry: *mut *mut c_void) -> c_int;
    fn acl_create_entry(acl: *mut *mut c_void, entry: *mut *mut c_void) -> c_int;
    fn acl_get_tag_type(entry: *mut c_void, tag_type: *mut ACLTag) -> c_int;
    fn acl_set_tag_type(entry: *mut c_void, tag_type: ACLTag) -> c_int;
    fn acl_get_permset(entry: *mut c_void, permset: *mut *mut c_void) -> c_int;
    fn acl_clear_perms(permset: *mut c_void) -> c_int;
    fn acl_get_perm(permset: *mut c_void, perm: ACLPerm) -> c_int;
    fn acl_add_perm(permset: *mut c_void, perm: ACLPerm) -> c_int;
    fn acl_get_qualifier(entry: *mut c_void) -> *mut c_void;
    fn acl_set_qualifier(entry: *mut c_void, qualifier: *const c_void) -> c_int;
    fn acl_init(count: c_int) -> *mut c_void;
    fn acl_valid(ptr: *const c_void) -> c_int;
    fn acl_free(ptr: *mut c_void) -> c_int;
}

#[derive(Debug)]
pub struct ACL {
    ptr: *mut c_void,
}

impl Drop for ACL {
    fn drop(&mut self) {
        let ret = unsafe { acl_free(self.ptr) };
        if ret != 0 {
            panic!("invalid pointer encountered while dropping ACL - {}", Errno::last());
        }
    }
}

impl ACL {
    pub fn init(count: usize) -> Result<ACL, nix::errno::Errno> {
        let ptr = unsafe { acl_init(count as i32 as c_int) };
        if ptr.is_null() {
            return Err(Errno::last());
        }

        Ok(ACL { ptr })
    }

    pub fn get_file<P: AsRef<Path>>(path: P, acl_type: ACLType) -> Result<ACL, nix::errno::Errno> {
        let path_cstr = CString::new(path.as_ref().as_os_str().as_bytes()).unwrap();
        let ptr = unsafe { acl_get_file(path_cstr.as_ptr(), acl_type) };
        if ptr.is_null() {
            return Err(Errno::last());
        }
 
        Ok(ACL { ptr })
    }

    pub fn set_file<P: AsRef<Path>>(&self, path: P, acl_type: ACLType) -> Result<(), nix::errno::Errno> {
        let path_cstr = CString::new(path.as_ref().as_os_str().as_bytes()).unwrap();
        let res = unsafe { acl_set_file(path_cstr.as_ptr(), acl_type, self.ptr) };
        if res < 0 {
            return Err(Errno::last());
        }
 
        Ok(())
    }

    pub fn get_fd(fd: RawFd) -> Result<ACL, nix::errno::Errno> {
        let ptr = unsafe { acl_get_fd(fd) };
        if ptr.is_null() {
            return Err(Errno::last());
        }

        Ok(ACL { ptr })
    }

    pub fn create_entry(&mut self) -> Result<ACLEntry, nix::errno::Errno> {
        let mut ptr = ptr::null_mut() as *mut c_void;
        let res = unsafe { acl_create_entry(&mut self.ptr, &mut ptr) };
        if res < 0 {
            return Err(Errno::last());
        }

        Ok(ACLEntry {
            ptr,
            _phantom: PhantomData,
        })
    }

    pub fn is_valid(&self) -> bool {
        let res = unsafe { acl_valid(self.ptr) };
        if res == 0 {
            return true;
        }

        false
    }

    pub fn entries(self) -> ACLEntriesIterator {
        ACLEntriesIterator {
            acl: self,
            current: ACL_FIRST_ENTRY,
        }
    }

    pub fn add_entry_full(&mut self, tag: ACLTag, qualifier: Option<u64>, permissions: u64)
        -> Result<(), nix::errno::Errno>
    {
        let mut entry = self.create_entry()?;
        entry.set_tag_type(tag)?;
        if let Some(qualifier) = qualifier {
            entry.set_qualifier(qualifier)?;
        }
        entry.set_permissions(permissions)?;

        Ok(())
    }
}

#[derive(Debug)]
pub struct ACLEntry<'a> {
    ptr: *mut c_void,
    _phantom: PhantomData<&'a mut ()>,
}

impl<'a> ACLEntry<'a> {
    pub fn get_tag_type(&self) -> Result<ACLTag, nix::errno::Errno> {
        let mut tag = ACL_UNDEFINED_TAG;
        let res = unsafe { acl_get_tag_type(self.ptr, &mut tag as *mut ACLTag) };
        if res < 0 {
            return Err(Errno::last());
        }

        Ok(tag)
    }

    pub fn set_tag_type(&mut self, tag: ACLTag) -> Result<(), nix::errno::Errno> {
        let res = unsafe { acl_set_tag_type(self.ptr, tag) };
        if res < 0 {
            return Err(Errno::last());
        }

        Ok(())
    }

    pub fn get_permissions(&self) -> Result<u64, nix::errno::Errno> {
        let mut permissions = 0;
        let mut permset = ptr::null_mut() as *mut c_void;
        let mut res = unsafe { acl_get_permset(self.ptr, &mut permset) };
        if res < 0 {
            return Err(Errno::last());
        }

        for &perm in &[ACL_READ, ACL_WRITE, ACL_EXECUTE] {
            res = unsafe { acl_get_perm(permset, perm) };
            if res < 0 { 
                return Err(Errno::last());
            } else if res > 0 { 
                permissions |= perm as u64;
            }
        }

        Ok(permissions)
    }

    pub fn set_permissions(&mut self, permissions: u64) -> Result<u64, nix::errno::Errno> {
        let mut permset = ptr::null_mut() as *mut c_void;
        let mut res = unsafe { acl_get_permset(self.ptr, &mut permset) };
        if res < 0 {
            return Err(Errno::last());
        }

        res = unsafe { acl_clear_perms(permset) };
        if res < 0 {
            return Err(Errno::last());
        }

        for &perm in &[ACL_READ, ACL_WRITE, ACL_EXECUTE] {
            if permissions & perm as u64 == perm as u64 {
                res = unsafe { acl_add_perm(permset, perm) };
                if res < 0 {
                    return Err(Errno::last());
                }
            }
        }

        Ok(permissions)
    }

    pub fn get_qualifier(&self) -> Result<u64, nix::errno::Errno> {
        let qualifier = unsafe { acl_get_qualifier(self.ptr) };
        if qualifier.is_null() {
            return Err(Errno::last());
        }
        let result = unsafe { *(qualifier as *const u32) as u64 };
        let ret = unsafe { acl_free(qualifier) };
        if ret != 0 {
            panic!("invalid pointer encountered while dropping ACL qualifier - {}", Errno::last());
        }

        Ok(result)
    }

    pub fn set_qualifier(&mut self, qualifier: u64) -> Result<(), nix::errno::Errno> {
        let val = qualifier as u32;
        let val_ptr: *const u32 = &val;
        let res = unsafe { acl_set_qualifier(self.ptr, val_ptr as *const c_void) };
        if res < 0 {
            return Err(Errno::last());
        }

        Ok(())
    }
}

#[derive(Debug)]
pub struct ACLEntriesIterator {
    acl: ACL,
    current: c_int,
}

impl<'a> Iterator for &'a mut ACLEntriesIterator {
    type Item = ACLEntry<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        let mut entry_ptr = ptr::null_mut();
        let res = unsafe { acl_get_entry(self.acl.ptr, self.current, &mut entry_ptr) };
        self.current = ACL_NEXT_ENTRY;
        if res == 1 {
            return Some(ACLEntry { ptr: entry_ptr, _phantom: PhantomData });
        }

        None
    }
}
