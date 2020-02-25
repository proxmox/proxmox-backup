//! Low level FUSE implementation for pxar.
//!
//! Allows to mount the archive as read-only filesystem to inspect its contents.

use std::collections::HashMap;
use std::convert::TryFrom;
use std::ffi::{CStr, CString, OsStr};
use std::fs::File;
use std::io::BufReader;
use std::os::unix::ffi::OsStrExt;
use std::path::Path;
use std::sync::Mutex;

use failure::{bail, format_err, Error};
use libc;
use libc::{c_char, c_int, c_void, size_t};

use crate::tools::lru_cache::{Cacher, LruCache};
use crate::tools::acl;
use super::binary_search_tree::search_binary_tree_by;
use super::decoder::{Decoder, DirectoryEntry};
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

/// FFI types for easier readability
type Request = *mut c_void;
type MutPtr = *mut c_void;
type ConstPtr = *const c_void;
type StrPtr = *const c_char;
type MutStrPtr = *mut c_char;

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
    fn fuse_reply_attr(req: Request, attr: Option<&libc::stat>, timeout: f64) -> c_int;
    fn fuse_reply_err(req: Request, errno: c_int) -> c_int;
    fn fuse_reply_buf(req: Request, buf: MutStrPtr, size: size_t) -> c_int;
    fn fuse_reply_entry(req: Request, entry: Option<&EntryParam>) -> c_int;
    fn fuse_reply_xattr(req: Request, size: size_t) -> c_int;
    fn fuse_reply_readlink(req: Request, link: StrPtr) -> c_int;
    fn fuse_req_userdata(req: Request) -> MutPtr;
    fn fuse_add_direntry_plus(req: Request, buf: MutStrPtr, bufsize: size_t, name: StrPtr, stbuf: Option<&EntryParam>, off: c_int) -> c_int;
}

/// Command line arguments passed to fuse.
#[repr(C)]
#[derive(Debug)]
struct FuseArgs {
    argc: c_int,
    argv: *const StrPtr,
    allocated: c_int,
}

/// `Context` for callback functions providing the decoder, caches and the
/// offset within the archive for the i-node given by the caller.
struct Context {
    decoder: Decoder,
    /// The start of each DirectoryEntry is used as inode, used as key for this
    /// hashmap.
    ///
    /// This map stores the corresponding end offset, needed to read the
    /// DirectoryEntry via the Decoder as well as the parent, in order
    /// to be able to include the parent directory on readdirplus calls.
    start_end_parent: HashMap<u64, (u64, u64)>,
    gbt_cache: LruCache<u64, Vec<(PxarGoodbyeItem, u64, u64)>>,
    entry_cache: LruCache<u64, DirectoryEntry>,
}

/// Cacher for the goodbye table.
///
/// Provides the feching of the goodbye table via the decoder on cache misses.
struct GbtCacher<'a> {
    decoder: &'a mut Decoder,
    map: &'a HashMap<u64, (u64, u64)>,
}

impl<'a> Cacher<u64, Vec<(PxarGoodbyeItem, u64, u64)>> for GbtCacher<'a> {
    fn fetch(&mut self, key: u64) -> Result<Option<Vec<(PxarGoodbyeItem, u64, u64)>>, Error> {
        let (end, _) = *self.map.get(&key).unwrap();
        let gbt = self.decoder.goodbye_table(None, end)?;
        Ok(Some(gbt))
    }
}

/// Cacher for the directory entries.
///
/// Provides the feching of directory entries via the decoder on cache misses.
struct EntryCacher<'a> {
    decoder: &'a mut Decoder,
    map: &'a HashMap<u64, (u64, u64)>,
}

impl<'a> Cacher<u64, DirectoryEntry> for EntryCacher<'a> {
    fn fetch(&mut self, key: u64) -> Result<Option<DirectoryEntry>, Error> {
        let entry = match key {
            0 => self.decoder.root()?,
            _ => {
                let (end, _) = *self.map.get(&key).unwrap();
                self.decoder.read_directory_entry(key, end)?
            }
        };
        Ok(Some(entry))
    }
}

impl Context {
    /// Provides mutable references to the `Context` members.
    /// This is needed to avoid borrow conflicts.
    fn as_mut_refs(&mut self) -> (
        &mut Decoder,
        &mut HashMap<u64, (u64, u64)>,
        &mut LruCache<u64, Vec<(PxarGoodbyeItem, u64, u64)>>,
        &mut LruCache<u64, DirectoryEntry>
    ) {
        ( &mut self.decoder, &mut self.start_end_parent, &mut self.gbt_cache, &mut self.entry_cache )
    }
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


impl Session  {

    /// Create a new low level fuse session.
    ///
    /// `Session` is created using the provided mount options and sets the
    /// default signal handlers.
    /// Options have to be provided as comma separated OsStr, e.g.
    /// ("ro,default_permissions").
    pub fn from_path(archive_path: &Path, options: &OsStr, verbose: bool) -> Result<Self, Error> {
        let file = File::open(archive_path)?;
        let reader = BufReader::new(file);
        let decoder = Decoder::new(reader)?;
        Self::new(decoder, options, verbose)
    }

    /// Create a new low level fuse session using the given `Decoder`.
    ///
    /// `Session` is created using the provided mount options and sets the
    /// default signal handlers.
    /// Options have to be provided as comma separated OsStr, e.g.
    /// ("ro,default_permissions").
    pub fn new(decoder: Decoder, options: &OsStr, verbose: bool) -> Result<Self, Error> {
        let args = Self::setup_args(options, verbose)?;
        let oprs = Self::setup_callbacks();
        let mut map = HashMap::new();
        // Insert entry for the root directory, with itself as parent.
        map.insert(0, (decoder.root_end_offset(), 0));

        let ctx = Context {
            decoder,
            start_end_parent: map,
            entry_cache: LruCache::new(1024),
            gbt_cache: LruCache::new(1024),
        };

        let session_ctx = Box::new(Mutex::new(ctx));
        let arg_ptrs: Vec<_> = args.iter().map(|opt| opt.as_ptr()).collect();
        let fuse_args = FuseArgs {
            argc: arg_ptrs.len() as i32,
            argv: arg_ptrs.as_ptr(),
            allocated: 0,
        };
        let session_ptr = unsafe {
            fuse_session_new(
                Some(&fuse_args),
                Some(&oprs),
                std::mem::size_of::<Operations>(),
                // Ownership of session_ctx is passed to the session here.
                // It has to be reclaimed before dropping the session to free
                // the `Context` and close the underlying file. This is done inside
                // the destroy callback function.
                Box::into_raw(session_ctx) as ConstPtr,
            )
        };

        if session_ptr.is_null() {
            bail!("error while creating new fuse session");
        }

        if unsafe { fuse_set_signal_handlers(session_ptr) } != 0 {
            bail!("error while setting signal handlers");
        }

        Ok(Self { ptr: session_ptr, verbose })
    }

    fn setup_args(options: &OsStr, verbose: bool) -> Result<Vec<CString>, Error> {
        // First argument should be the executable name
        let mut arguments = vec![
            CString::new("pxar-mount").unwrap(),
            CString::new("-o").unwrap(),
            CString::new(options.as_bytes())?,
        ];
        if verbose {
            arguments.push(CString::new("--debug").unwrap());
        }

        Ok(arguments)
    }

    fn setup_callbacks() -> Operations {
        // Register the callback functions for the session
        let mut oprs = Operations::default();
        oprs.init = Some(Self::init);
        oprs.destroy = Some(Self::destroy);
        oprs.lookup = Some(Self::lookup);
        oprs.getattr = Some(Self::getattr);
        oprs.readlink = Some(Self::readlink);
        oprs.read = Some(Self::read);
        oprs.getxattr = Some(Self::getxattr);
        oprs.listxattr = Some(Self::listxattr);
        oprs.readdirplus = Some(Self::readdirplus);
        oprs
    }

    /// Mount the filesystem on the given mountpoint.
    ///
    /// Actually mount the filesystem for this session on the provided mountpoint
    /// and daemonize process.
    pub fn mount(&mut self, mountpoint: &Path, deamonize: bool) -> Result<(), Error> {
        if self.verbose {
            println!("Mounting archive to {:#?}", mountpoint);
        }
        let mountpoint = mountpoint.canonicalize()?;
        let path_cstr = CString::new(mountpoint.as_os_str().as_bytes())
            .map_err(|err| format_err!("invalid mountpoint - {}", err))?;
        if unsafe { fuse_session_mount(self.ptr, path_cstr.as_ptr()) } != 0 {
            bail!("mounting on {:#?} failed", mountpoint);
        }

        // Send process to background if deamonize is set
        if deamonize && unsafe { fuse_daemonize(0) } != 0 {
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

    /// Creates a context providing exclusive mutable references to the members of
    /// `Context`.
    ///
    /// Same as run_in_context except it provides ref mut to the individual members
    /// of `Context` in order to avoid borrow conflicts.
    fn run_with_context_refs<F>(req: Request, inode: u64, code: F)
    where
        F: FnOnce(
            &mut Decoder,
            &mut HashMap<u64, (u64, u64)>,
            &mut LruCache<u64, Vec<(PxarGoodbyeItem, u64, u64)>>,
            &mut LruCache<u64, DirectoryEntry>,
            u64,
        ) -> Result<(), i32>,
    {
        let boxed_ctx = unsafe {
            let ptr = fuse_req_userdata(req) as *mut Mutex<Context>;
            Box::from_raw(ptr)
        };
        let result = boxed_ctx
            .lock()
            .map(|mut ctx| {
                let ino_offset = match inode {
                    FUSE_ROOT_ID => 0,
                    _ => inode,
                };
                let (decoder, map, gbt_cache, entry_cache) = ctx.as_mut_refs();
                code(decoder, map, gbt_cache, entry_cache, ino_offset)
            })
            .unwrap_or(Err(libc::EIO));

        if let Err(err) = result {
            unsafe {
                let _res = fuse_reply_err(req, err);
            }
        }

        // Release ownership of boxed context, do not drop it.
        let _ = Box::into_raw(boxed_ctx);
    }

    /// Callback functions for fuse kernel driver.
    extern "C" fn init(_decoder: MutPtr) {
        // Notting to do here for now
    }

    /// Cleanup the userdata created while creating the session, which is the `Context`
    extern "C" fn destroy(ctx: MutPtr) {
        // Get ownership of the `Context` and drop it when Box goes out of scope.
        unsafe { Box::from_raw(ctx) };
    }

    /// Lookup `name` in the directory referenced by `parent` i-node.
    ///
    /// Inserts also the child and parent file offset in the hashmap to quickly
    /// obtain the parent offset based on the child offset.
    /// Caches goodbye table of parent and attributes of child, if found.
    extern "C" fn lookup(req: Request, parent: u64, name: StrPtr) {
        let filename = unsafe { CStr::from_ptr(name) };
        let hash = super::format_definition::compute_goodbye_hash(filename.to_bytes());

        Self::run_with_context_refs(req, parent, |decoder, map, gbt_cache, entry_cache, ino_offset| {
            let gbt = gbt_cache.access(ino_offset, &mut GbtCacher { decoder, map })
                .map_err(|_| libc::EIO)?
                .ok_or_else(|| libc::EIO)?;
            let mut start_idx = 0;
            let mut skip_multiple = 0;
            loop {
                // Search for the next goodbye entry with matching hash.
                let idx = search_binary_tree_by(
                    start_idx,
                    gbt.len(),
                    skip_multiple,
                    |idx| hash.cmp(&gbt[idx].0.hash),
                ).ok_or_else(|| libc::ENOENT)?;

                let (_item, start, end) = &gbt[idx];
                map.insert(*start, (*end, ino_offset));

                let entry = entry_cache.access(*start, &mut EntryCacher { decoder, map })
                    .map_err(|_| libc::EIO)?
                    .ok_or_else(|| libc::ENOENT)?;

                // Possible hash collision, need to check if the found entry is indeed
                // the filename to lookup.
                if entry.filename.as_bytes() == filename.to_bytes() {
                    let e = EntryParam {
                        inode: *start,
                        generation: 1,
                        attr: stat(*start, &entry)?,
                        attr_timeout: std::f64::MAX,
                        entry_timeout: std::f64::MAX,
                    };

                    let _res = unsafe { fuse_reply_entry(req, Some(&e)) };
                    return Ok(())
                }
                // Hash collision, check the next entry in the goodbye table by starting
                // from given index but skipping one more match (so hash at index itself).
                start_idx = idx;
                skip_multiple = 1;
            }
        });
    }

    extern "C" fn getattr(req: Request, inode: u64, _fileinfo: MutPtr) {
        Self::run_with_context_refs(req, inode, |decoder, map, _, entry_cache, ino_offset| {
            let entry = entry_cache.access(ino_offset, &mut EntryCacher { decoder, map })
                .map_err(|_| libc::EIO)?
                .ok_or_else(|| libc::EIO)?;
            let attr = stat(inode, &entry)?;
            let _res = unsafe {
                // Since fs is read-only, the timeout can be max.
                let timeout = std::f64::MAX;
                fuse_reply_attr(req, Some(&attr), timeout)
            };

            Ok(())
        });
    }

    extern "C" fn readlink(req: Request, inode: u64) {
        Self::run_with_context_refs(req, inode, |decoder, map, _, entry_cache, ino_offset| {
            let entry = entry_cache
                .access(ino_offset, &mut EntryCacher { decoder, map })
                .map_err(|_| libc::EIO)?
                .ok_or_else(|| libc::EIO)?;
            let target = entry.target.as_ref().ok_or_else(|| libc::EIO)?;
            let link = CString::new(target.as_os_str().as_bytes()).map_err(|_| libc::EIO)?;
            let _ret = unsafe { fuse_reply_readlink(req, link.as_ptr()) };

            Ok(())
        });
    }

    extern "C" fn read(req: Request, inode: u64, size: size_t, offset: c_int, _fileinfo: MutPtr) {
        Self::run_with_context_refs(req, inode, |decoder, map, _gbt_cache, entry_cache, ino_offset| {
            let entry = entry_cache.access(ino_offset, &mut EntryCacher { decoder, map })
                .map_err(|_| libc::EIO)?
                .ok_or_else(|| libc::EIO)?;
            let mut data = decoder.read(&entry, size, offset as u64).map_err(|_| libc::EIO)?;

            let _res = unsafe {
                let len = data.len();
                let dptr = data.as_mut_ptr() as *mut c_char;
                fuse_reply_buf(req, dptr, len)
            };

            Ok(())
        });
    }

    /// Read and return the entries of the directory referenced by i-node.
    ///
    /// Replies to the request with the entries fitting into a buffer of length
    /// `size`, as requested by the caller.
    /// `offset` identifies the start index of entries to return. This is used on
    /// repeated calls, occurring if not all entries fitted into the buffer.
    extern "C" fn readdirplus(req: Request, inode: u64, size: size_t, offset: c_int, _fileinfo: MutPtr) {
        let offset = offset as usize;

        Self::run_with_context_refs(req, inode, |decoder, map, gbt_cache, entry_cache, ino_offset| {
            let gbt = gbt_cache.access(ino_offset, &mut GbtCacher { decoder, map })
                .map_err(|_| libc::EIO)?
                .ok_or_else(|| libc::ENOENT)?;
            let n_entries = gbt.len();
            let mut buf = ReplyBuf::new(req, size, offset);

            if offset < n_entries {
                for e in gbt[offset..gbt.len()].iter() {
                    map.insert(e.1, (e.2, ino_offset));
                    let entry = entry_cache.access(e.1, &mut EntryCacher { decoder, map })
                        .map_err(|_| libc::EIO)?
                        .ok_or_else(|| libc::EIO)?;
                    let name = CString::new(entry.filename.as_bytes())
                        .map_err(|_| libc::EIO)?;
                    let attr = EntryParam {
                        inode: e.1,
                        generation: 1,
                        attr: stat(e.1, &entry).map_err(|_| libc::EIO)?,
                        attr_timeout: std::f64::MAX,
                        entry_timeout: std::f64::MAX,
                    };
                    match buf.fill(&name, &attr) {
                        Ok(ReplyBufState::Okay) => {}
                        Ok(ReplyBufState::Overfull) => return buf.reply_filled(),
                        Err(_) => return Err(libc::EIO),
                    }
                }
            }

            // Add current directory entry "."
            if offset <= n_entries {
                let entry = entry_cache.access(ino_offset, &mut EntryCacher { decoder, map })
                    .map_err(|_| libc::EIO)?
                    .ok_or_else(|| libc::EIO)?;
                let name = CString::new(".").unwrap();
                let attr = EntryParam {
                    inode: inode,
                    generation: 1,
                    attr: stat(inode, &entry).map_err(|_| libc::EIO)?,
                    attr_timeout: std::f64::MAX,
                    entry_timeout: std::f64::MAX,
                };
                match buf.fill(&name, &attr) {
                    Ok(ReplyBufState::Okay) => {}
                    Ok(ReplyBufState::Overfull) => return buf.reply_filled(),
                    Err(_) => return Err(libc::EIO),
                }
            }

            // Add parent directory entry ".."
            if offset <= n_entries + 1 {
                let (_, parent) = *map.get(&ino_offset).unwrap();
                let entry = entry_cache.access(parent, &mut EntryCacher { decoder, map })
                    .map_err(|_| libc::EIO)?
                    .ok_or_else(|| libc::EIO)?;
                let inode = if parent == 0 { FUSE_ROOT_ID } else { parent };
                let name = CString::new("..").unwrap();
                let attr = EntryParam {
                    inode: inode,
                    generation: 1,
                    attr: stat(inode, &entry).map_err(|_| libc::EIO)?,
                    attr_timeout: std::f64::MAX,
                    entry_timeout: std::f64::MAX,
                };
                match buf.fill(&name, &attr) {
                    Ok(ReplyBufState::Okay) => {}
                    Ok(ReplyBufState::Overfull) => return buf.reply_filled(),
                    Err(_) => return Err(libc::EIO),
                }
            }

            buf.reply_filled()
        });
    }

    /// Get the value of the extended attribute of `inode` identified by `name`.
    extern "C" fn getxattr(req: Request, inode: u64, name: StrPtr, size: size_t) {
        let name = unsafe { CStr::from_ptr(name) };

        Self::run_with_context_refs(req, inode, |decoder, map, _, entry_cache, ino_offset| {
            let entry = entry_cache.access(ino_offset, &mut EntryCacher { decoder, map })
                .map_err(|_| libc::EIO)?
                .ok_or_else(|| libc::EIO)?;

            // Some of the extended attributes are stored separately in the archive,
            // so check if requested name matches one of those.
            match name.to_bytes() {
                b"security.capability" => {
                    match &mut entry.xattr.fcaps {
                        None => return Err(libc::ENODATA),
                        Some(fcaps) => return Self::xattr_reply_value(req, &mut fcaps.data, size),
                    }
                }
                b"system.posix_acl_access" => {
                    // Make sure to return if there are no matching extended attributes in the archive
                    if entry.xattr.acl_group_obj.is_none()
                        && entry.xattr.acl_user.is_empty()
                        && entry.xattr.acl_group.is_empty() {
                            return Err(libc::ENODATA);
                    }
                    let mut buffer = acl::ACLXAttrBuffer::new(acl::ACL_EA_VERSION);

                    buffer.add_entry(acl::ACL_USER_OBJ, None, acl::mode_user_to_acl_permissions(entry.entry.mode));
                    match &entry.xattr.acl_group_obj {
                        Some(group_obj) => {
                            buffer.add_entry(acl::ACL_MASK, None, acl::mode_group_to_acl_permissions(entry.entry.mode));
                            buffer.add_entry(acl::ACL_GROUP_OBJ, None, group_obj.permissions);
                        }
                        None => {
                            buffer.add_entry(acl::ACL_GROUP_OBJ, None, acl::mode_group_to_acl_permissions(entry.entry.mode));
                        }
                    }
                    buffer.add_entry(acl::ACL_OTHER, None, acl::mode_other_to_acl_permissions(entry.entry.mode));

                    for user in &mut entry.xattr.acl_user {
                        buffer.add_entry(acl::ACL_USER, Some(user.uid), user.permissions);
                    }
                    for group in &mut entry.xattr.acl_group {
                        buffer.add_entry(acl::ACL_GROUP, Some(group.gid), group.permissions);
                    }
                    return Self::xattr_reply_value(req, buffer.as_mut_slice(), size);
                }
                b"system.posix_acl_default" => {
                    if let Some(default) = &entry.xattr.acl_default {
                        let mut buffer = acl::ACLXAttrBuffer::new(acl::ACL_EA_VERSION);

                        buffer.add_entry(acl::ACL_USER_OBJ, None, default.user_obj_permissions);
                        buffer.add_entry(acl::ACL_GROUP_OBJ, None, default.group_obj_permissions);
                        buffer.add_entry(acl::ACL_OTHER, None, default.other_permissions);

                        if default.mask_permissions != std::u64::MAX {
                            buffer.add_entry(acl::ACL_MASK, None, default.mask_permissions);
                        }

                        for user in &mut entry.xattr.acl_default_user {
                            buffer.add_entry(acl::ACL_USER, Some(user.uid), user.permissions);
                        }
                        for group in &mut entry.xattr.acl_default_group {
                            buffer.add_entry(acl::ACL_GROUP, Some(group.gid), group.permissions);
                        }
                        if buffer.len() > 0 {
                            return Self::xattr_reply_value(req, buffer.as_mut_slice(), size);
                        }
                    }
                }
                name => {
                    for xattr in &mut entry.xattr.xattrs {
                        if name == xattr.name.as_slice() {
                            return Self::xattr_reply_value(req, &mut xattr.value, size);
                        }
                    }
                }
            }


            Err(libc::ENODATA)
        });
    }

    /// Get a list of the extended attribute of `inode`.
    extern "C" fn listxattr(req: Request, inode: u64, size: size_t) {
        Self::run_with_context_refs(req, inode, |decoder, map, _, entry_cache, ino_offset| {
            let entry = entry_cache.access(ino_offset, &mut EntryCacher { decoder, map })
                .map_err(|_| libc::EIO)?
                .ok_or_else(|| libc::EIO)?;
            let mut buffer = Vec::new();
            if entry.xattr.fcaps.is_some() {
                buffer.extend_from_slice(b"security.capability\0");
            }
            if entry.xattr.acl_default.is_some() {
                buffer.extend_from_slice(b"system.posix_acl_default\0");
            }
            if entry.xattr.acl_group_obj.is_some()
                || !entry.xattr.acl_user.is_empty()
                || !entry.xattr.acl_group.is_empty() {
                buffer.extend_from_slice(b"system.posix_acl_user\0");
            }
            for xattr in &mut entry.xattr.xattrs {
                buffer.append(&mut xattr.name);
                buffer.push(b'\0');
            }

            Self::xattr_reply_value(req, &mut buffer, size)
        });
    }

    /// Helper function used to respond to get- and listxattr calls in order to
    /// de-duplicate code.
    fn xattr_reply_value(req: Request, value: &mut [u8], size: size_t) -> Result<(), i32> {
        let len = value.len();

        if size == 0 {
            // reply the needed buffer size to fit value
            let _res = unsafe { fuse_reply_xattr(req, len) };
        } else if size < len {
            // value does not fit into requested buffer size
            return Err(libc::ERANGE);
        } else {
            // value fits into requested buffer size, send value
            let _res = unsafe {
                let vptr = value.as_mut_ptr() as *mut c_char;
                fuse_reply_buf(req, vptr, len)
            };
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

/// FUSE entry for fuse_reply_entry in lookup callback
#[repr(C)]
struct EntryParam {
    inode: u64,
    generation: u64,
    attr: libc::stat,
    attr_timeout: f64,
    entry_timeout: f64,
}

/// Create a `libc::stat` with the provided i-node and entry
fn stat(inode: u64, entry: &DirectoryEntry) -> Result<libc::stat, i32> {
    let nlink = match (entry.entry.mode as u32) & libc::S_IFMT {
        libc::S_IFDIR => 2,
        _ => 1,
    };
    let time = i64::try_from(entry.entry.mtime).map_err(|_| libc::EIO)?;
    let sec = time / 1_000_000_000;
    let nsec = time % 1_000_000_000;

    let mut attr: libc::stat = unsafe { std::mem::zeroed() };
    attr.st_ino = inode;
    attr.st_nlink = nlink;
    attr.st_mode = u32::try_from(entry.entry.mode).map_err(|_| libc::EIO)?;
    attr.st_size = i64::try_from(entry.size).map_err(|_| libc::EIO)?;
    attr.st_uid = entry.entry.uid;
    attr.st_gid = entry.entry.gid;
    attr.st_atime = sec;
    attr.st_atime_nsec = nsec;
    attr.st_mtime = sec;
    attr.st_mtime_nsec = nsec;
    attr.st_ctime = sec;
    attr.st_ctime_nsec = nsec;

    Ok(attr)
}

/// State of ReplyBuf after last add_entry call
enum ReplyBufState {
    /// Entry was successfully added to ReplyBuf
    Okay,
    /// Entry did not fit into ReplyBuf, was not added
    Overfull,
}

/// Used to correctly fill and reply the buffer for the readdirplus callback
struct ReplyBuf {
    /// internal buffer holding the binary data
    buffer: Vec<u8>,
    /// offset up to which the buffer is filled already
    filled: usize,
    /// fuse request the buffer is used to reply to
    req: Request,
    /// index of the next item, telling from were to start on the next readdirplus
    /// callback in case not everything fitted in the buffer on the first reply.
    next: usize,
}

impl ReplyBuf {
    /// Create a new empty `ReplyBuf` of `size` with element counting index at `next`.
    fn new(req: Request, size: usize, next: usize) -> Self {
        Self {
            buffer: vec![0; size],
            filled: 0,
            req,
            next,
        }
    }

    /// Reply to the `Request` with the filled buffer
    fn reply_filled(&mut self) -> Result<(), i32> {
        let _res = unsafe {
            let ptr = self.buffer.as_mut_ptr() as *mut c_char;
            fuse_reply_buf(self.req, ptr, self.filled)
        };

        Ok(())
    }

    /// Fill the buffer for the fuse reply with the next dir entry by invoking the
    /// fuse_add_direntry_plus helper function for the readdirplus callback.
    /// The attr type T is has to be `libc::stat` or `EntryParam` accordingly.
    fn fill(&mut self, name: &CString, attr: &EntryParam) -> Result<ReplyBufState, Error> {
        self.next += 1;
        let size = self.buffer.len();
        let bytes = unsafe {
            let bptr = self.buffer.as_mut_ptr() as *mut c_char;
            let nptr = name.as_ptr();
            fuse_add_direntry_plus(
                self.req,
                bptr.offset(self.filled as isize),
                size - self.filled,
                nptr,
                Some(&attr),
                i32::try_from(self.next)?,
            ) as usize
        };
        self.filled += bytes;
        // Never exceed the max size requested in the callback (=buffer.len())
        if self.filled > size {
            // Entry did not fit, so go back to previous state
            self.filled -= bytes;
            self.next -= 1;
            return Ok(ReplyBufState::Overfull);
        }

        Ok(ReplyBufState::Okay)
    }
}
