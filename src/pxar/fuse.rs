//! Low level FUSE implementation for pxar.
//!
//! Allows to mount the archive as read-only filesystem to inspect its contents.

use std::ffi::{CString, OsStr};
use std::fs::File;
use std::io::BufReader;
use std::os::unix::ffi::OsStrExt;
use std::path::Path;
use std::sync::Mutex;

use failure::{bail, format_err, Error};
use libc;
use libc::{c_char, c_int, c_void, size_t};

use super::decoder::Decoder;
use super::format_definition::PxarGoodbyeItem;

/// Node ID of the root i-node
///
/// Offsets in the archive are used as i-node for the fuse implementation, as
/// they are unique and enough to reference each item in the pxar archive.
/// The only exception to this is the `FUSE_ROOT_ID`, which is defined as 1 by
/// the fuse library.
/// This is okay since offset 1 is part of the root directory entry header and
/// will therefore not occur again, but remapping to the correct offset of 0 is
/// required.
const FUSE_ROOT_ID: u64 = 1;

const GOODBYE_ITEM_SIZE: u64 = std::mem::size_of::<PxarGoodbyeItem>() as u64;
/// Callback function for `super::decoder::Decoder`.
///
/// At the moment, this is only needed to satisfy the `SequentialDecoder`.
fn decoder_callback(_path: &Path) -> Result<(), Error> {
    Ok(())
}

/// FFI types for easier readability
type Request = *mut c_void;
type MutPtr = *mut c_void;
type ConstPtr = *const c_void;
type StrPtr = *const c_char;

#[rustfmt::skip]
#[link(name = "fuse3")]
extern "C" {
    fn fuse_session_new(args: Option<&FuseArgs>, oprs: Option<&Operations>, size: size_t, op: ConstPtr) -> MutPtr;
    fn fuse_set_signal_handlers(session: ConstPtr) -> c_int;
    fn fuse_remove_signal_handlers(session: ConstPtr);
    fn fuse_daemonize(foreground: c_int) -> c_int;
    fn fuse_session_mount(session: ConstPtr, mountpoint: StrPtr) -> c_int;
    fn fuse_session_unmount(session: ConstPtr);
    fn fuse_session_loop(session: ConstPtr) -> c_int;
    fn fuse_session_loop_mt_31(session: ConstPtr, clone_fd: c_int) -> c_int;
    fn fuse_session_destroy(session: ConstPtr);
    //    fn fuse_reply_attr(req: Request, attr: *const libc::stat, timeout: f64) -> c_int;
    fn fuse_reply_err(req: Request, errno: c_int) -> c_int;
    fn fuse_req_userdata(req: Request) -> MutPtr;
}

/// Command line arguments passed to fuse.
#[repr(C)]
#[derive(Debug)]
struct FuseArgs {
    argc: c_int,
    argv: *const StrPtr,
    allocated: c_int,
}

/// `Session` stores a pointer to the session context and is used to mount the
/// archive to the given mountpoint.
pub struct Session {
    ptr: MutPtr,
    verbose: bool,
}

/// `Operations` defines the callback function table of supported operations.
#[repr(C)]
#[derive(Default)]
#[rustfmt::skip]
struct Operations {
    // The order in which the functions are listed matters, as the offset in the
    // struct defines what function the fuse driver uses.
    // It should therefore not be altered!
    init:           Option<extern fn(userdata: MutPtr)>,
    destroy:        Option<extern fn(userdata: MutPtr)>,
    lookup:         Option<extern fn(req: Request, parent: u64, name: StrPtr)>,
    forget:         Option<extern fn(req: Request, inode: u64, nlookup: u64)>,
    getattr:        Option<extern fn(req: Request, inode: u64, fileinfo: MutPtr)>,
    setattr:        Option<extern fn(req: Request, inode: u64, attr: MutPtr, to_set: c_int, fileinfo: MutPtr)>,
    readlink:       Option<extern fn(req: Request, inode: u64)>,
    mknod:          Option<extern fn(req: Request, parent: u64, name: StrPtr, mode: c_int, rdev: c_int)>,
    mkdir:          Option<extern fn(req: Request, parent: u64, name: StrPtr, mode: c_int)>,
    unlink:         Option<extern fn(req: Request, parent: u64, name: StrPtr)>,
    rmdir:          Option<extern fn(req: Request, parent: u64, name: StrPtr)>,
    symlink:        Option<extern fn(req: Request, link: StrPtr, parent: u64, name: StrPtr)>,
    rename:         Option<extern fn(req: Request, parent: u64, name: StrPtr, newparent: u64, newname: StrPtr, flags: c_int)>,
    link:           Option<extern fn(req: Request, inode: u64, newparent: u64, newname: StrPtr)>,
    open:           Option<extern fn(req: Request, indoe: u64, fileinfo: MutPtr)>,
    read:           Option<extern fn(req: Request, inode: u64, size: size_t, offset: c_int, fileinfo: MutPtr)>,
    write:          Option<extern fn(req: Request, inode: u64, buffer: StrPtr, size: size_t, offset: c_void, fileinfo: MutPtr)>,
    flush:          Option<extern fn(req: Request, inode: u64, fileinfo: MutPtr)>,
    release:        Option<extern fn(req: Request, inode: u64, fileinfo: MutPtr)>,
    fsync:          Option<extern fn(req: Request, inode: u64, datasync: c_int, fileinfo: MutPtr)>,
    opendir:        Option<extern fn(req: Request, inode: u64, fileinfo: MutPtr)>,
    readdir:        Option<extern fn(req: Request, inode: u64, size: size_t, offset: c_int, fileinfo: MutPtr)>,
    releasedir:     Option<extern fn(req: Request, inode: u64, fileinfo: MutPtr)>,
    fsyncdir:       Option<extern fn(req: Request, inode: u64, datasync: c_int, fileinfo: MutPtr)>,
    statfs:         Option<extern fn(req: Request, inode: u64)>,
    setxattr:       Option<extern fn(req: Request, inode: u64, name: StrPtr, value: StrPtr, size: size_t, flags: c_int)>,
    getxattr:       Option<extern fn(req: Request, inode: u64, name: StrPtr, size: size_t)>,
    listxattr:      Option<extern fn(req: Request, inode: u64, size: size_t)>,
    removexattr:    Option<extern fn(req: Request, inode: u64, name: StrPtr)>,
    access:         Option<extern fn(req: Request, inode: u64, mask: i32)>,
    create:         Option<extern fn(req: Request, parent: u64, name: StrPtr, mode: c_int, fileinfo: MutPtr)>,
    getlk:          Option<extern fn(req: Request, inode: u64, fileinfo: MutPtr, lock: MutPtr)>,
    setlk:          Option<extern fn(req: Request, inode: u64, fileinfo: MutPtr, lock: MutPtr, sleep: c_int)>,
    bmap:           Option<extern fn(req: Request, inode: u64, blocksize: size_t, idx: u64)>,
    ioctl:          Option<extern fn(req: Request, inode: u64, cmd: c_int, arg: MutPtr, fileinfo: MutPtr, flags: c_int, in_buf: ConstPtr, in_bufsz: size_t, out_bufsz: size_t)>,
    poll:           Option<extern fn(req: Request, inode: u64, fileinfo: MutPtr, pollhandle: MutPtr)>,
    write_buf:      Option<extern fn(req: Request, inode: u64, bufv: MutPtr, offset: c_int, fileinfo: MutPtr)>,
    retrieve_reply: Option<extern fn(req: Request, cookie: ConstPtr, inode: u64, offset: c_int, bufv: MutPtr)>,
    forget_multi:   Option<extern fn(req: Request, count: size_t, forgets: MutPtr)>,
    flock:          Option<extern fn(req: Request, inode: u64, fileinfo: MutPtr, op: c_int)>,
    fallocate:      Option<extern fn(req: Request, inode: u64, mode: c_int, offset: c_int, length: c_int, fileinfo: MutPtr)>,
    readdirplus:    Option<extern fn(req: Request, inode: u64, size: size_t, offset: c_int, fileinfo: MutPtr)>,
    copy_file_range: Option<extern fn(req: Request, ino_in: u64, off_in: c_int, fi_in: MutPtr, ino_out: u64, off_out: c_int, fi_out: MutPtr, len: size_t, flags: c_int)>,
}

impl Session {
    /// Create a new low level fuse session.
    ///
    /// `Session` is created using the provided mount options and sets the
    /// default signal handlers.
    /// Options have to be provided as comma separated OsStr, e.g.
    /// ("ro,default_permissions").
    pub fn new(archive_path: &Path, options: &OsStr, verbose: bool) -> Result<Self, Error> {
        let file = File::open(archive_path)?;
        // First argument should be the executable name
        let mut arguments = vec![
            CString::new("pxar-mount").unwrap(),
            CString::new("-o").unwrap(),
            CString::new(options.as_bytes())?,
        ];
        if verbose {
            arguments.push(CString::new("--debug").unwrap());
        }

        let arg_ptrs: Vec<_> = arguments.iter().map(|opt| opt.as_ptr()).collect();
        let args = FuseArgs {
            argc: arg_ptrs.len() as i32,
            argv: arg_ptrs.as_ptr(),
            allocated: 0,
        };

        // Register the callback funcitons for the session
        let mut oprs = Operations::default();
        oprs.init = Some(init);
        oprs.destroy = Some(destroy);
        oprs.lookup = Some(lookup);
        oprs.getattr = Some(getattr);
        oprs.open = Some(open);
        oprs.read = Some(read);
        oprs.opendir = Some(opendir);
        oprs.readdir = Some(readdir);

        // By storing the decoder as userdata of the session, each request may
        // access it.
        let reader = BufReader::new(file);
        let decoder = Decoder::new(reader, decoder_callback as fn(&Path) -> Result<(), Error>)?;
        let session_decoder = Box::new(Mutex::new(decoder));
        let session_ptr = unsafe {
            fuse_session_new(
                Some(&args),
                Some(&oprs),
                std::mem::size_of::<Operations>(),
                // Ownership of session_decoder is passed to the session here.
                // It has to be reclaimed before dropping the session to free
                // the decoder and close the underlying file. This is done inside
                // the destroy callback function.
                Box::into_raw(session_decoder) as ConstPtr,
            )
        };

        if session_ptr.is_null() {
            bail!("error while creating new fuse session");
        }

        if unsafe { fuse_set_signal_handlers(session_ptr) } != 0 {
            bail!("error while setting signal handlers");
        }

        Ok(Self {
            ptr: session_ptr,
            verbose: verbose,
        })
    }

    /// Mount the filesystem on the given mountpoint.
    ///
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
    ///
    /// The multi_threaded flag controls if the session loop runs in
    /// single-threaded or multi-threaded mode.
    /// Single-threaded mode is intended for debugging only.
    pub fn run_loop(&mut self, multi_threaded: bool) -> Result<(), Error> {
        if self.verbose {
            println!("Executing fuse session loop");
        }
        let result = match multi_threaded {
            true => unsafe { fuse_session_loop_mt_31(self.ptr, 1) },
            false => unsafe { fuse_session_loop(self.ptr) },
        };
        if result < 0 {
            bail!("fuse session loop exited with - {}", result);
        }
        if result > 0 {
            eprintln!("fuse session loop received signal - {}", result);
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

/// Creates a context providing an exclusive mutable reference to the decoder.
///
/// Each callback function needing access to the decoder can easily get an
/// exclusive handle by running the code inside this context.
/// Responses with error code can easily be generated by returning with the
/// error code.
/// The error code will be used to reply to libfuse.
fn run_in_context<F>(req: Request, inode: u64, code: F)
where
    F: FnOnce(
        &mut Decoder<BufReader<File>, fn(&Path) -> Result<(), Error>>,
        u64,
    ) -> Result<(), i32>,
{
    let ptr = unsafe {
        fuse_req_userdata(req)
            as *mut Mutex<Decoder<BufReader<File>, fn(&Path) -> Result<(), Error>>>
    };
    let boxed_decoder = unsafe { Box::from_raw(ptr) };
    let result = boxed_decoder
        .lock()
        .map(|mut decoder| {
            let ino_offset = match inode {
                FUSE_ROOT_ID => decoder.root_end_offset() - GOODBYE_ITEM_SIZE,
                _ => inode,
            };
            code(&mut decoder, ino_offset)
        })
        .unwrap_or(Err(libc::EIO));

    if let Err(err) = result {
        unsafe {
            let _res = fuse_reply_err(req, err);
        }
    }

    // Release ownership of boxed decoder, do not drop it.
    let _ = Box::into_raw(boxed_decoder);
}

/// Callback functions for fuse kernel driver.
extern "C" fn init(_decoder: MutPtr) {
    // Notting to do here for now
}

/// Cleanup the userdata created while creating the session, which is the decoder
extern "C" fn destroy(decoder: MutPtr) {
    // Get ownership of the decoder and drop it when Box goes out of scope.
    unsafe {
        Box::from_raw(decoder);
    }
}

extern "C" fn lookup(req: Request, parent: u64, _name: StrPtr) {
    run_in_context(req, parent, |_decoder, _ino_offset| {
        // code goes here

        Err(libc::ENOENT)
    });
}

extern "C" fn getattr(req: Request, inode: u64, _fileinfo: MutPtr) {
    run_in_context(req, inode, |_decoder, _ino_offset| {
        // code goes here

        Err(libc::ENOENT)
    });
}

extern "C" fn open(req: Request, inode: u64, _fileinfo: MutPtr) {
    run_in_context(req, inode, |_decoder, _ino_offset| {
        // code goes here

        Err(libc::ENOENT)
    });
}

extern "C" fn read(req: Request, inode: u64, _size: size_t, _offset: c_int, _fileinfo: MutPtr) {
    run_in_context(req, inode, |_decoder, _ino_offset| {
        // code goes here

        Err(libc::ENOENT)
    });
}

extern "C" fn opendir(req: Request, inode: u64, _fileinfo: MutPtr) {
    run_in_context(req, inode, |_decoder, _ino_offset| {
        // code goes here

        Err(libc::ENOENT)
    });
}

extern "C" fn readdir(req: Request, inode: u64, _size: size_t, _offset: c_int, _fileinfo: MutPtr) {
    run_in_context(req, inode, |_decoder, _ino_offset| {
        // code goes here

        Err(libc::ENOENT)
    });
}
