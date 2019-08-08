//! Low level FUSE implementation for pxar.
//!
//! Allows to mount the archive as read-only filesystem to inspect its contents.

use std::ffi::{OsStr, CString};
use std::fs::File;
use std::os::unix::ffi::OsStrExt;
use std::path::Path;

use libc;
use libc::{c_int, c_void, c_char, size_t};
use failure::*;

#[link(name = "fuse3")]
extern "C" {
    fn fuse_session_new(args: *const FuseArgs, oprs: *const Operations, size: size_t, op: *const c_void) -> *mut c_void;
    fn fuse_set_signal_handlers(session: *const c_void) -> c_int;
    fn fuse_remove_signal_handlers(session: *const c_void);
    fn fuse_daemonize(foreground: c_int) -> c_int;
    fn fuse_session_mount(session: *const c_void, mountpoint: *const c_char) -> c_int;
    fn fuse_session_unmount(session: *const c_void);
    fn fuse_session_loop(session: *const c_void) -> c_int;
    fn fuse_session_destroy(session: *const c_void);
}

/// Command line arguments passed to fuse.
#[repr(C)]
#[derive(Debug)]
struct FuseArgs {
    argc: c_int,
    argv: *const *const c_char,
    allocated: c_int,
}

/// `Session` stores a pointer to the session context and is used to mount the
/// archive to the given mountpoint.
#[derive(Debug)]
pub struct Session {
    ptr: *mut c_void,
    archive: File,
    verbose: bool,
}

/// `Operations` defines the callback function table of supported operations.
#[repr(C)]
struct Operations {
    init: extern fn(userdata: *mut c_void) -> *mut c_void,
    destroy: extern fn(userdata: *mut c_void) -> *mut c_void,
}

impl Session {
    /// Create a new low level fuse session.
    ///
    /// `Session` is created using the provided mount options and sets the
    /// default signal handlers.
    /// Options have to be provided as comma separated OsStr, e.g.
    /// ("ro,default_permissions").
    pub fn new(archive_path: &Path, options: &OsStr, verbose: bool)-> Result<Self, Error> {
        let file = File::open(archive_path)?;
        // First argument should be the executable name
        let arguments = vec![
            CString::new("pxar-mount").unwrap(),
            CString::new("-o").unwrap(),
            CString::new(options.as_bytes())?,
        ];

        let arg_ptrs: Vec<_> = arguments.iter().map(|opt| opt.as_ptr()).collect();
        let args = FuseArgs {
            argc: arg_ptrs.len() as i32,
            argv: arg_ptrs.as_ptr(),
            allocated: 0,
        };

        // Register the callback funcitons for the session
        let oprs = Operations {
            init: init,
            destroy: destroy,
        };

        let session_ptr = unsafe { fuse_session_new(
            &args as *const FuseArgs,
            &oprs as *const Operations,
            std::mem::size_of::<Operations>(),
            std::ptr::null()
        )};

        if session_ptr.is_null() {
            bail!("error while creating new fuse session");
        }

        if unsafe { fuse_set_signal_handlers(session_ptr) } != 0 {
            bail!("error while setting signal handlers");
        }

        Ok(Self {
            ptr: session_ptr,
            archive: file,
            verbose: verbose,
        })
    }

    /// Actually mount the filesystem for this session on the provided mountpoint
    /// and daemonize process.
    pub fn mount(&mut self, mountpoint: &Path) -> Result<(), Error> {
        if self.verbose {
            println!("Mounting archive to {:#?}", mountpoint);
        }
        let mountpoint = mountpoint.canonicalize()?;
        let path_cstr = CString::new(mountpoint.as_os_str().as_bytes())
            .map_err(|err| format_err!("invalid mountpoint - {}", err))?;
        if unsafe { fuse_session_mount(self.ptr, path_cstr.as_ptr()) } != 0 {
            bail!("mounting on {:#?} failed", mountpoint);
        }

        // Do not send process to background if verbose flag is set
        if !self.verbose && unsafe { fuse_daemonize(0) } != 0 {
            bail!("could not send process to background");
        }

        Ok(())
    }

    /// Execute session loop which handles requests from kernel.
    pub fn run_loop(&mut self) -> Result<(), Error> {
        if self.verbose {
            println!("Executing fuse session loop");
        }
        let result = unsafe { fuse_session_loop(self.ptr) };
        if result < 0 {
            bail!("fuse session loop exited with - {}", result);
        }
        if result > 0 {
            eprintln!("fuse session loop recieved signal - {}", result);
        }

        Ok(())
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        unsafe {
            fuse_session_unmount(self.ptr);
            fuse_remove_signal_handlers(self.ptr);
            fuse_session_destroy(self.ptr);
        }
    }
}

/// Callback functions for fuse kernel driver.
extern "C" fn init(_userdata: *mut c_void) -> *mut c_void {
    println!("Init callback");
    return std::ptr::null_mut();
}

extern "C" fn destroy(_userdata: *mut c_void) -> *mut c_void {
    println!("Destroy callback");
    return std::ptr::null_mut();
}
