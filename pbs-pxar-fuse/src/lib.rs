//! Asynchronous fuse implementation.

use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};
use std::future::Future;
use std::io;
use std::mem;
use std::ops::Range;
use std::os::unix::ffi::OsStrExt;
use std::path::Path;
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, RwLock};
use std::task::{Context, Poll};

use anyhow::{format_err, Error};
use futures::channel::mpsc::UnboundedSender;
use futures::select;
use futures::sink::SinkExt;
use futures::stream::{StreamExt, TryStreamExt};

use proxmox_io::vec;
use pxar::accessor::{self, EntryRangeInfo, ReadAt};

use proxmox_fuse::requests::{self, FuseRequest};
use proxmox_fuse::{EntryParam, Fuse, ReplyBufState, Request, ROOT_ID};
use proxmox_lang::io_format_err;
use proxmox_sys::fs::xattr;

/// We mark inodes for regular files this way so we know how to access them.
const NON_DIRECTORY_INODE: u64 = 1u64 << 63;

#[inline]
fn is_dir_inode(inode: u64) -> bool {
    0 == (inode & NON_DIRECTORY_INODE)
}

/// Our reader type instance used for accessors.
pub type Reader = Arc<dyn ReadAt + Send + Sync + 'static>;

/// Our Accessor type instance.
pub type Accessor = accessor::aio::Accessor<Reader>;

/// Our Directory type instance.
pub type Directory = accessor::aio::Directory<Reader>;

/// Our FileEntry type instance.
pub type FileEntry = accessor::aio::FileEntry<Reader>;

/// Our FileContents type instance.
pub type FileContents = accessor::aio::FileContents<Reader>;

pub struct Session {
    fut: Pin<Box<dyn Future<Output = Result<(), Error>> + Send + Sync + 'static>>,
}

impl Session {
    /// Create a fuse session for an archive.
    pub async fn mount_path(
        archive_path: &Path,
        options: &OsStr,
        verbose: bool,
        mountpoint: &Path,
    ) -> Result<Self, Error> {
        // TODO: Add a buffered/caching ReadAt layer?
        let file = std::fs::File::open(archive_path)?;
        let file_size = file.metadata()?.len();
        let reader: Reader = Arc::new(accessor::sync::FileReader::new(file));
        let accessor = Accessor::new(reader, file_size).await?;
        Self::mount(accessor, options, verbose, mountpoint)
    }

    /// Create a new fuse session for the given pxar `Accessor`.
    pub fn mount(
        accessor: Accessor,
        options: &OsStr,
        verbose: bool,
        path: &Path,
    ) -> Result<Self, Error> {
        let fuse = Fuse::builder("pxar-mount")?
            .debug()
            .options_os(options)?
            .enable_readdirplus()
            .enable_read()
            .enable_readlink()
            .enable_read_xattr()
            .build()?
            .mount(path)?;

        let session = SessionImpl::new(accessor, verbose);

        Ok(Self {
            fut: Box::pin(session.main(fuse)),
        })
    }
}

impl Future for Session {
    type Output = Result<(), Error>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context) -> Poll<Self::Output> {
        Pin::new(&mut self.fut).poll(cx)
    }
}

/// We use this to return an errno value back to the kernel.
macro_rules! io_return {
    ($errno:expr) => {{
        return Err(::std::io::Error::from_raw_os_error($errno).into());
    }};
}

/// This is what we need to cache as a "lookup" entry. The kernel assumes that these are easily
/// accessed.
struct Lookup {
    refs: AtomicUsize,

    inode: u64,
    parent: u64,
    entry_range_info: EntryRangeInfo,
    content_range: Option<Range<u64>>,
}

impl Lookup {
    fn new(
        inode: u64,
        parent: u64,
        entry_range_info: EntryRangeInfo,
        content_range: Option<Range<u64>>,
    ) -> Box<Lookup> {
        Box::new(Self {
            refs: AtomicUsize::new(1),
            inode,
            parent,
            entry_range_info,
            content_range,
        })
    }

    /// Decrease the reference count by `count`. Note that this must not include the reference held
    /// by `self` itself, so this must not decrease the count below 2.
    fn forget(&self, count: usize) -> Result<(), Error> {
        loop {
            let old = self.refs.load(Ordering::Acquire);
            if count >= old {
                // We use this to bail out of a functionin an unexpected error case. This will cause the fuse
                // request to be answered with a generic `EIO` error code. The error message contained in here
                // will be printed to stdout if the verbose flag is used, otherwise silently dropped.
                return Err(io_format_err!("reference count underflow").into());
            }
            let new = old - count;
            match self
                .refs
                .compare_exchange(old, new, Ordering::SeqCst, Ordering::SeqCst)
            {
                Ok(_) => break Ok(()),
                Err(_) => continue,
            }
        }
    }

    fn get_ref<'a>(&self, session: &'a SessionImpl) -> LookupRef<'a> {
        if self.refs.fetch_add(1, Ordering::AcqRel) == 0 {
            panic!("atomic refcount increased from 0 to 1");
        }

        LookupRef {
            session,
            lookup: self as *const Lookup,
        }
    }
}

struct LookupRef<'a> {
    session: &'a SessionImpl,
    lookup: *const Lookup,
}

unsafe impl<'a> Send for LookupRef<'a> {}
unsafe impl<'a> Sync for LookupRef<'a> {}

impl<'a> Clone for LookupRef<'a> {
    fn clone(&self) -> Self {
        self.get_ref(self.session)
    }
}

impl<'a> std::ops::Deref for LookupRef<'a> {
    type Target = Lookup;

    fn deref(&self) -> &Self::Target {
        unsafe { &*self.lookup }
    }
}

impl<'a> Drop for LookupRef<'a> {
    fn drop(&mut self) {
        if self.lookup.is_null() {
            return;
        }

        if self.refs.fetch_sub(1, Ordering::AcqRel) == 1 {
            let inode = self.inode;
            drop(self.session.lookups.write().unwrap().remove(&inode));
        }
    }
}

impl<'a> LookupRef<'a> {
    fn leak(mut self) -> &'a Lookup {
        unsafe { &*mem::replace(&mut self.lookup, std::ptr::null()) }
    }
}

struct SessionImpl {
    accessor: Accessor,
    verbose: bool,
    lookups: RwLock<BTreeMap<u64, Box<Lookup>>>,
}

impl SessionImpl {
    fn new(accessor: Accessor, verbose: bool) -> Self {
        let root = Lookup::new(
            ROOT_ID,
            ROOT_ID,
            EntryRangeInfo::toplevel(0..accessor.size()),
            None,
        );

        let mut tree = BTreeMap::new();
        tree.insert(ROOT_ID, root);

        Self {
            accessor,
            verbose,
            lookups: RwLock::new(tree),
        }
    }

    /// Here's how we deal with errors:
    ///
    /// Any error will be logged if a log level of at least 'debug' was set, otherwise the
    /// message will be silently dropped.
    ///
    /// Opaque errors will cause the fuse main loop to bail out with that error.
    ///
    /// `io::Error`s will cause the fuse request to responded to with the given `io::Error`. An
    /// `io::ErrorKind::Other` translates to a generic `EIO`.
    async fn handle_err(
        &self,
        request: impl FuseRequest,
        err: Error,
        mut sender: UnboundedSender<Error>,
    ) {
        let final_result = match err.downcast::<io::Error>() {
            Ok(err) => {
                if err.kind() == io::ErrorKind::Other {
                    log::error!("an IO error occurred: {}", err);
                }

                // fail the request
                request.io_fail(err).map_err(Error::from)
            }
            Err(err) => {
                // `bail` (non-`io::Error`) is used for fatal errors which should actually cancel:
                log::error!("internal error: {}, bailing out", err);
                Err(err)
            }
        };
        if let Err(err) = final_result {
            // either we failed to send the error code to fuse, or the above was not an
            // `io::Error`, so in this case notify the main loop:
            sender
                .send(err)
                .await
                .expect("failed to propagate error to main loop");
        }
    }

    async fn main(self, fuse: Fuse) -> Result<(), Error> {
        Arc::new(self).main_do(fuse).await
    }

    async fn main_do(self: Arc<Self>, fuse: Fuse) -> Result<(), Error> {
        let (err_send, mut err_recv) = futures::channel::mpsc::unbounded::<Error>();
        let mut fuse = fuse.fuse(); // make this a futures::stream::FusedStream!
        loop {
            select! {
                request = fuse.try_next() => match request? {
                    Some(request) => {
                        tokio::spawn(Arc::clone(&self).handle_request(request, err_send.clone()));
                    }
                    None => break,
                },
                err = err_recv.next() => match err {
                    Some(err) => if self.verbose {
                        log::error!("cancelling fuse main loop due to error: {}", err);
                        return Err(err);
                    },
                    None => panic!("error channel was closed unexpectedly"),
                },
            }
        }
        Ok(())
    }

    async fn handle_request(
        self: Arc<Self>,
        request: Request,
        mut err_sender: UnboundedSender<Error>,
    ) {
        let result: Result<(), Error> = match request {
            Request::Lookup(request) => {
                match self.lookup(request.parent, &request.file_name).await {
                    Ok((entry, lookup)) => match request.reply(&entry) {
                        Ok(()) => {
                            lookup.leak();
                            Ok(())
                        }
                        Err(err) => Err(Error::from(err)),
                    },
                    Err(err) => return self.handle_err(request, err, err_sender).await,
                }
            }
            Request::Forget(request) => match self.forget(request.inode, request.count as usize) {
                Ok(()) => {
                    request.reply();
                    Ok(())
                }
                Err(err) => return self.handle_err(request, err, err_sender).await,
            },
            Request::Getattr(request) => match self.getattr(request.inode).await {
                Ok(stat) => request.reply(&stat, f64::MAX).map_err(Error::from),
                Err(err) => return self.handle_err(request, err, err_sender).await,
            },
            Request::ReaddirPlus(mut request) => match self.readdirplus(&mut request).await {
                Ok(lookups) => match request.reply() {
                    Ok(()) => {
                        for i in lookups {
                            i.leak();
                        }
                        Ok(())
                    }
                    Err(err) => Err(Error::from(err)),
                },
                Err(err) => return self.handle_err(request, err, err_sender).await,
            },
            Request::Read(request) => {
                match self.read(request.inode, request.size, request.offset).await {
                    Ok(data) => request.reply(&data).map_err(Error::from),
                    Err(err) => return self.handle_err(request, err, err_sender).await,
                }
            }
            Request::Readlink(request) => match self.readlink(request.inode).await {
                Ok(data) => request.reply(&data).map_err(Error::from),
                Err(err) => return self.handle_err(request, err, err_sender).await,
            },
            Request::ListXAttrSize(request) => match self.listxattrs(request.inode).await {
                Ok(data) => request
                    .reply(
                        data.into_iter()
                            .fold(0, |sum, i| sum + i.name().to_bytes_with_nul().len()),
                    )
                    .map_err(Error::from),
                Err(err) => return self.handle_err(request, err, err_sender).await,
            },
            Request::ListXAttr(mut request) => match self.listxattrs_into(&mut request).await {
                Ok(ReplyBufState::Ok) => request.reply().map_err(Error::from),
                Ok(ReplyBufState::Full) => request.fail_full().map_err(Error::from),
                Err(err) => return self.handle_err(request, err, err_sender).await,
            },
            Request::GetXAttrSize(request) => {
                match self.getxattr(request.inode, &request.attr_name).await {
                    Ok(xattr) => request.reply(xattr.value().len()).map_err(Error::from),
                    Err(err) => return self.handle_err(request, err, err_sender).await,
                }
            }
            Request::GetXAttr(request) => {
                match self.getxattr(request.inode, &request.attr_name).await {
                    Ok(xattr) => request.reply(xattr.value()).map_err(Error::from),
                    Err(err) => return self.handle_err(request, err, err_sender).await,
                }
            }
            other => {
                log::error!("Received unexpected fuse request");
                other.fail(libc::ENOSYS).map_err(Error::from)
            }
        };

        if let Err(err) = result {
            err_sender
                .send(err)
                .await
                .expect("failed to propagate error to main loop");
        }
    }

    fn get_lookup(&self, inode: u64) -> Result<LookupRef, Error> {
        let lookups = self.lookups.read().unwrap();
        if let Some(lookup) = lookups.get(&inode) {
            return Ok(lookup.get_ref(self));
        }
        io_return!(libc::ENOENT);
    }

    async fn open_dir(&self, inode: u64) -> Result<Directory, Error> {
        if inode == ROOT_ID {
            Ok(self.accessor.open_root().await?)
        } else if !is_dir_inode(inode) {
            io_return!(libc::ENOTDIR);
        } else {
            Ok(unsafe { self.accessor.open_dir_at_end(inode).await? })
        }
    }

    async fn open_entry(&self, lookup: &LookupRef<'_>) -> io::Result<FileEntry> {
        unsafe {
            self.accessor
                .open_file_at_range(&lookup.entry_range_info)
                .await
        }
    }

    fn open_content(&self, lookup: &LookupRef) -> Result<FileContents, Error> {
        if is_dir_inode(lookup.inode) {
            io_return!(libc::EISDIR);
        }

        match lookup.content_range.clone() {
            Some(range) => Ok(unsafe { self.accessor.open_contents_at_range(range) }),
            None => io_return!(libc::EBADF),
        }
    }

    fn make_lookup(&self, parent: u64, inode: u64, entry: &FileEntry) -> Result<LookupRef, Error> {
        let lookups = self.lookups.read().unwrap();
        if let Some(lookup) = lookups.get(&inode) {
            return Ok(lookup.get_ref(self));
        }
        drop(lookups);

        let entry = Lookup::new(
            inode,
            parent,
            entry.entry_range_info().clone(),
            entry.content_range()?,
        );
        let reference = entry.get_ref(self);
        entry.refs.store(1, Ordering::Release);

        let mut lookups = self.lookups.write().unwrap();
        if let Some(lookup) = lookups.get(&inode) {
            return Ok(lookup.get_ref(self));
        }

        lookups.insert(inode, entry);
        drop(lookups);
        Ok(reference)
    }

    fn forget(&self, inode: u64, count: usize) -> Result<(), Error> {
        let node = self.get_lookup(inode)?;
        node.forget(count)?;
        Ok(())
    }

    async fn lookup(
        &'_ self,
        parent: u64,
        file_name: &OsStr,
    ) -> Result<(EntryParam, LookupRef<'_>), Error> {
        let dir = self.open_dir(parent).await?;

        let entry = match { dir }.lookup(file_name).await? {
            Some(entry) => entry,
            None => io_return!(libc::ENOENT),
        };

        let entry = if let pxar::EntryKind::Hardlink(_) = entry.kind() {
            // we don't know the file's end-offset, so we'll just allow the decoder to decode the
            // entire rest of the archive until we figure out something better...
            let entry = self.accessor.follow_hardlink(&entry).await?;

            if let pxar::EntryKind::Hardlink(_) = entry.kind() {
                // hardlinks must not point to other hardlinks...
                io_return!(libc::ELOOP);
            }

            entry
        } else {
            entry
        };

        let response = to_entry(&entry)?;
        let inode = response.inode;
        Ok((response, self.make_lookup(parent, inode, &entry)?))
    }

    async fn getattr(&self, inode: u64) -> Result<libc::stat, Error> {
        let entry = unsafe {
            self.accessor
                .open_file_at_range(&self.get_lookup(inode)?.entry_range_info)
                .await?
        };
        to_stat(inode, &entry)
    }

    async fn readdirplus(
        &'_ self,
        request: &mut requests::ReaddirPlus,
    ) -> Result<Vec<LookupRef<'_>>, Error> {
        let mut lookups = Vec::new();
        let offset = usize::try_from(request.offset)
            .map_err(|_| io_format_err!("directory offset out of range"))?;

        let dir = self.open_dir(request.inode).await?;
        let dir_lookup = self.get_lookup(request.inode)?;

        let entry_count = dir.read_dir().count() as isize;

        let mut next = offset as isize;
        let mut iter = dir.read_dir().skip(offset);
        while let Some(file) = iter.next().await {
            next += 1;
            let file = file?.decode_entry().await?;
            let stat = to_stat(to_inode(&file), &file)?;
            let name = file.file_name();
            match request.add_entry(name, &stat, next, 1, f64::MAX, f64::MAX)? {
                ReplyBufState::Ok => (),
                ReplyBufState::Full => return Ok(lookups),
            }
            lookups.push(self.make_lookup(request.inode, stat.st_ino, &file)?);
        }

        if next == entry_count {
            next += 1;
            let file = dir.lookup_self().await?;
            let stat = to_stat(to_inode(&file), &file)?;
            let name = OsStr::new(".");
            match request.add_entry(name, &stat, next, 1, f64::MAX, f64::MAX)? {
                ReplyBufState::Ok => (),
                ReplyBufState::Full => return Ok(lookups),
            }
            lookups.push(LookupRef::clone(&dir_lookup));
        }

        if next == entry_count + 1 {
            next += 1;
            let lookup = self.get_lookup(dir_lookup.parent)?;
            let parent_dir = self.open_dir(lookup.inode).await?;
            let file = parent_dir.lookup_self().await?;
            let stat = to_stat(to_inode(&file), &file)?;
            let name = OsStr::new("..");
            match request.add_entry(name, &stat, next, 1, f64::MAX, f64::MAX)? {
                ReplyBufState::Ok => (),
                ReplyBufState::Full => return Ok(lookups),
            }
            lookups.push(lookup);
        }

        Ok(lookups)
    }

    async fn read(&self, inode: u64, len: usize, offset: u64) -> Result<Vec<u8>, Error> {
        let file = self.get_lookup(inode)?;
        let content = self.open_content(&file)?;
        let mut buf = vec::undefined(len);
        let mut pos = 0;
        // fuse' read is different from normal read - no short reads allowed except for EOF!
        // the returned data will be 0-byte padded up to len by fuse
        loop {
            let got = content
                .read_at(&mut buf[pos..], offset + pos as u64)
                .await?;
            pos += got;
            if got == 0 || pos >= len {
                break;
            }
        }
        buf.truncate(pos);
        Ok(buf)
    }

    async fn readlink(&self, inode: u64) -> Result<OsString, Error> {
        let lookup = self.get_lookup(inode)?;
        let file = self.open_entry(&lookup).await?;
        match file.get_symlink() {
            None => io_return!(libc::EINVAL),
            Some(link) => Ok(link.to_owned()),
        }
    }

    async fn listxattrs(&self, inode: u64) -> Result<Vec<pxar::format::XAttr>, Error> {
        let lookup = self.get_lookup(inode)?;
        let metadata = self.open_entry(&lookup).await?.into_entry().into_metadata();

        let mut xattrs = metadata.xattrs;

        use pxar::format::XAttr;

        if let Some(fcaps) = metadata.fcaps {
            xattrs.push(XAttr::new(xattr::xattr_name_fcaps().to_bytes(), fcaps.data));
        }

        // TODO: Special cases:
        //     b"system.posix_acl_access
        //     b"system.posix_acl_default
        //
        // For these we need to be able to create posix acl format entries, at that point we could
        // just ditch libacl as well...

        Ok(xattrs)
    }

    async fn listxattrs_into(
        &self,
        request: &mut requests::ListXAttr,
    ) -> Result<ReplyBufState, Error> {
        let xattrs = self.listxattrs(request.inode).await?;

        for entry in xattrs {
            match request.add_c_string(entry.name()) {
                ReplyBufState::Ok => (),
                ReplyBufState::Full => return Ok(ReplyBufState::Full),
            }
        }

        Ok(ReplyBufState::Ok)
    }

    async fn getxattr(&self, inode: u64, xattr: &OsStr) -> Result<pxar::format::XAttr, Error> {
        // TODO: pxar::Accessor could probably get a more optimized method to fetch a specific
        // xattr for an entry...
        let xattrs = self.listxattrs(inode).await?;
        for entry in xattrs {
            if entry.name().to_bytes() == xattr.as_bytes() {
                return Ok(entry);
            }
        }
        io_return!(libc::ENODATA);
    }
}

#[inline]
fn to_entry(entry: &FileEntry) -> Result<EntryParam, Error> {
    to_entry_param(to_inode(entry), entry)
}

#[inline]
fn to_inode(entry: &FileEntry) -> u64 {
    if entry.is_dir() {
        entry.entry_range_info().entry_range.end
    } else {
        entry.entry_range_info().entry_range.start | NON_DIRECTORY_INODE
    }
}

fn to_entry_param(inode: u64, entry: &pxar::Entry) -> Result<EntryParam, Error> {
    Ok(EntryParam::simple(inode, to_stat(inode, entry)?))
}

fn to_stat(inode: u64, entry: &pxar::Entry) -> Result<libc::stat, Error> {
    let nlink = if entry.is_dir() { 2 } else { 1 };

    let metadata = entry.metadata();

    let mut stat: libc::stat = unsafe { mem::zeroed() };
    stat.st_ino = inode;
    stat.st_nlink = nlink;
    stat.st_mode = u32::try_from(metadata.stat.mode)
        .map_err(|err| format_err!("mode does not fit into st_mode field: {}", err))?;
    stat.st_size = i64::try_from(entry.file_size().unwrap_or(0))
        .map_err(|err| format_err!("size does not fit into st_size field: {}", err))?;
    stat.st_uid = metadata.stat.uid;
    stat.st_gid = metadata.stat.gid;
    stat.st_atime = metadata.stat.mtime.secs;
    stat.st_atime_nsec = metadata.stat.mtime.nanos as _;
    stat.st_mtime = metadata.stat.mtime.secs;
    stat.st_mtime_nsec = metadata.stat.mtime.nanos as _;
    stat.st_ctime = metadata.stat.mtime.secs;
    stat.st_ctime_nsec = metadata.stat.mtime.nanos as _;
    Ok(stat)
}
