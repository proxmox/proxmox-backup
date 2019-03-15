//! For the C API we need to provide a `Client` compatible with C. In rust `Client` takes a
//! `T: Read + Write`, so we need to provide a way for C to provide callbacks to us to
//! implement this.

use std::ffi::{CStr, CString};
use std::io::{self, Read, Write};
use std::os::raw::{c_char, c_int, c_void};

use failure::{bail, format_err, Error};
use libc::size_t;

/// Read callback. The first parameter is the `opaque` parameter passed to `proxmox_backup_new`,
/// the rest are the usual read function parameters. This should return the number of bytes
/// actually read, zero on EOF, or a negative `errno` value on error (eg. `-EAGAIN`).
pub type ReadFn = extern "C" fn(opaque: *mut c_void, buf: *mut u8, size: u64) -> i64;

/// Write callback. The first parameter is the `opaque` parameter passed to `proxmox_backup_new`,
/// the rest are the usual write function parameters. This should return the number of bytes
/// actually written, or a negative `errno` value on error (eg. `-EAGAIN`).
pub type WriteFn = extern "C" fn(opaque: *mut c_void, buf: *const u8, size: u64) -> i64;

/// Optional drop callback. This is called when the Client gets destroyed and allows freeing
/// resources associated with the opaque object behind the C API socket.
pub type DropFn = extern "C" fn(opaque: *mut c_void);

/// Stores the external C callbacks for communicating with the protocol socket.
pub struct CApiSocket {
    opaque: *mut c_void,
    read: ReadFn,
    write: WriteFn,
    drop: Option<DropFn>,
}

impl CApiSocket {
    fn from_io<T: Read + Write>(stream: T) -> Self {
        let opaque = Box::leak(Box::new(stream));
        Self {
            opaque: opaque as *mut T as _,
            read: c_read_fn::<T>,
            write: c_write_fn::<T>,
            drop: Some(c_drop_fn::<T>),
        }
    }
}

/// A client instance using C callbacks for reading from and writing to the protocol socket.
pub struct CClient {
    client: crate::Client<CApiSocket>,
    error: Option<CString>,
    upload: Option<(*const u8, usize)>,
}

impl CClient {
    fn set_error(&mut self, err: Error) -> c_int {
        self.error = Some(match CString::new(err.to_string()) {
            Ok(cs) => cs,
            Err(_) => CString::new("<bad bytes in error string>").unwrap(),
        });
        -1
    }

    #[inline(always)]
    fn bool_result(&mut self, res: Result<bool, Error>) -> c_int {
        match res {
            Ok(false) => 0,
            Ok(true) => 1,
            Err(e) => self.set_error(e),
        }
    }

    #[inline(always)]
    fn bool_call<F>(&mut self, func: F) -> c_int
    where
        F: FnOnce(&mut crate::Client<CApiSocket>) -> Result<bool, Error>,
    {
        let res = func(&mut self.client);
        self.bool_result(res)
    }

    #[inline(always)]
    fn int_call<F>(&mut self, func: F) -> c_int
    where
        F: FnOnce(&mut crate::Client<CApiSocket>) -> Result<c_int, Error>,
    {
        match func(&mut self.client) {
            Ok(v) => v,
            Err(e) => self.set_error(e.into()),
        }
    }
}

impl Read for CApiSocket {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let rc = (self.read)(self.opaque, buf.as_mut_ptr(), buf.len() as u64);
        if rc < 0 {
            Err(io::Error::from_raw_os_error((-rc) as i32))
        } else {
            Ok(rc as usize)
        }
    }
}

impl Write for CApiSocket {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let rc = (self.write)(self.opaque, buf.as_ptr(), buf.len() as u64);
        if rc < 0 {
            Err(io::Error::from_raw_os_error((-rc) as i32))
        } else {
            Ok(rc as usize)
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Drop for CApiSocket {
    fn drop(&mut self) {
        if let Some(drop) = self.drop {
            drop(self.opaque);
        }
    }
}

extern "C" fn c_read_fn<T: Read>(opaque: *mut c_void, buf: *mut u8, size: u64) -> i64 {
    let stream = unsafe { &mut *(opaque as *mut T) };
    let buf = unsafe { std::slice::from_raw_parts_mut(buf, size as usize) };

    match stream.read(buf) {
        Ok(size) => size as i64,
        Err(err) => {
            match err.raw_os_error() {
                Some(err) => -(err as i64),
                None => {
                    eprintln!("error reading from stream: {}", err);
                    -libc::EIO as i64
                }
            }
        },
    }
}

extern "C" fn c_write_fn<T: Write>(opaque: *mut c_void, buf: *const u8, size: u64) -> i64 {
    let stream = unsafe { &mut *(opaque as *mut T) };
    let buf = unsafe { std::slice::from_raw_parts(buf, size as usize) };

    match stream.write(buf) {
        Ok(size) => size as i64,
        Err(err) => {
            match err.raw_os_error() {
                Some(err) => -(err as i64),
                None => {
                    eprintln!("error writing to stream: {}", err);
                    -libc::EIO as i64
                }
            }
        },
    }
}

extern "C" fn c_drop_fn<T>(opaque: *mut c_void) {
    unsafe {
        Box::from_raw(opaque as *mut T);
    }
}

pub(crate) fn make_c_compatible_client<T: Read + Write>(stream: T) -> crate::Client<CApiSocket> {
    crate::Client::new(CApiSocket::from_io(stream))
}

pub(crate) fn make_c_client(client: crate::Client<CApiSocket>) -> *mut CClient {
    Box::leak(Box::new(CClient {
        client,
        error: None,
        upload: None,
    }))
}

/// Creates a new instance of a backup protocol client.
///
/// # Arguments
///
/// * `opaque` - An opaque pointer passed to the two provided callback methods.
/// * `read` - The read callback.
/// * `write` - The write callback.
#[no_mangle]
pub extern "C" fn proxmox_backup_new(
    opaque: *mut c_void,
    read: ReadFn,
    write: WriteFn,
    drop: DropFn,
) -> *mut CClient {
    let drop_ptr: *const () = unsafe { std::mem::transmute(drop) };
    let drop = if drop_ptr.is_null() {
        None
    } else {
        Some(drop)
    };
    make_c_client(crate::Client::new(CApiSocket {
        opaque,
        read,
        write,
        drop,
    }))
}

/// Drops an instance of a backup protocol client. The pointer must be valid or `NULL`.
#[no_mangle]
pub extern "C" fn proxmox_backup_done(me: *mut CClient) {
    if !me.is_null() {
        unsafe {
            Box::from_raw(me);
        }
    }
}

/// Returns a C String describing the last error or `NULL` if there was none.
#[no_mangle]
pub extern "C" fn proxmox_backup_get_error(me: *const CClient) -> *const c_char {
    let me = unsafe { &*me };
    match me.error {
        Some(ref e) => e.as_ptr(),
        None => std::ptr::null(),
    }
}

/// Returns true if the `read` callback had previously returned `EOF`.
#[no_mangle]
pub extern "C" fn proxmox_backup_is_eof(me: *const CClient) -> bool {
    let me = unsafe { &*me };
    me.client.eof()
}

/// The data polling methods usually pass errors from the callbacks through to the original caller.
/// Since the protocol needs to be non-blocking-IO safe and therefore able to resumine at any point
/// where `-EAGAIN` can be returned by the callbacks, it is up to the caller which errors are to be
/// considered fatal, but any error returned by callbacks which is not `-EAGAIN` will result in an
/// internal error flag to be set which has to be cleared before trying to resume normal
/// operations.
#[no_mangle]
pub extern "C" fn proxmox_backup_clear_err(me: *mut CClient) {
    let me = unsafe { &mut *me };
    me.client.clear_err();
    me.error = None;
}

/// Polls for data and checks whether the protocol handshake has been made successfully.
/// Returns `1` if the handshake was successful, `0` if it is not yet complete or `-1` on error.
#[no_mangle]
pub extern "C" fn proxmox_backup_wait_for_handshake(me: *mut CClient) -> c_int {
    let me = unsafe { &mut *me };
    me.bool_call(move |c| c.wait_for_handshake())
}

fn check_string(s: *const c_char) -> Result<&'static str, Error> {
    if s.is_null() {
        bail!("NULL string");
    }
    Ok(std::str::from_utf8(unsafe {
        CStr::from_ptr(s).to_bytes()
    })?)
}

/// Request the list of hashes for a backup file in order to prevent duplicates from being sent to
/// the server. This simply causes an internal list to be filled. Only one such operation can be
/// performed simultaneously. To wait for its completion see `proxmox_backup_wait_for_hashes`.
///
/// If the file name is `NULL` or not a valid UTF-8 string, this function returns an error without
/// putting the protocol handler in an error state.
///
/// Returns `0` on success, `-1` otherwise.
#[no_mangle]
pub extern "C" fn proxmox_backup_query_hashes(me: *mut CClient, file_name: *const c_char) -> c_int {
    let me = unsafe { &mut *me };

    me.int_call(move |client| {
        let file_name = check_string(file_name)?;
        client.query_hashes(file_name)?;
        Ok(0)
    })
}

/// If there is an ongoing hash list request, this will poll the data stream.
///
/// Returns `1` if the transfer is complete (or there was no transfer to begin with), `0` if it is
/// incomplete, or `-1` if an error occurred.
#[no_mangle]
pub extern "C" fn proxmox_backup_wait_for_hashes(me: *mut CClient) -> c_int {
    let me = unsafe { &mut *me };
    me.bool_call(move |c| c.wait_for_hashes())
}

/// Check if a chunk of the provided digest is known to the this client instance. Note that this
/// does not query the server for this information, and is only useful after a call to
/// `proxmox_backup_query_hashes` or after uploading something.
#[no_mangle]
pub extern "C" fn proxmox_backup_is_chunk_available(me: *const CClient, digest: *const u8) -> bool {
    let me = unsafe { &*me };
    let digest = unsafe { &*(digest as *const [u8; 32]) };
    me.client.is_chunk_available(digest)
}

/// Begin uploading a chunk to the server. This attempts to upload the data right away, but if the
/// writer may fail due to non-blocking I/O in which case the `proxmox_backup_continue_upload`
/// function must be used.
///
/// Returns `0` if the upload is incomplete, a positive ID if the upload was completed immediately,
/// or `-1` on error.
///
/// The ID returned on success can be used to wait for the server to acknowledge that the chunk has
/// been written successfully. Use `proxmox_backup_wait_for_id` to do this. If confirmation is not
/// required, the ID should be released via `proxmox_backup_discard_id`.
#[no_mangle]
pub extern "C" fn proxmox_backup_upload_chunk(
    me: *mut CClient,
    digest: *const u8,
    data: *const u8,
    size: u64,
) -> c_int {
    let me = unsafe { &mut *me };
    let digest: &[u8; 32] = unsafe { &*(digest as *const [u8; 32]) };
    let size = size as usize;
    let slice: &[u8] = unsafe { std::slice::from_raw_parts(data, size) };
    match me.client.upload_chunk(digest, slice) {
        Ok(Some(id)) => id.0 as c_int,
        Ok(None) => {
            me.upload = Some((data, size));
            0
        }
        Err(e) => me.set_error(e),
    }
}

/// If an upload did not finish immediately (`proxmox_backup_upload_chunk` returned `0`), this
/// function must be used to retry sending the rest of the data.
///
/// Returns `0` if the upload is incomplete, a positive ID if the upload was completed immediately,
/// or `-1` on error.
#[no_mangle]
pub extern "C" fn proxmox_backup_continue_upload(me: *mut CClient) -> c_int {
    let me = unsafe { &mut *me };
    match me.upload {
        Some((data, len)) => {
            let slice: &[u8] = unsafe { std::slice::from_raw_parts(data, len) };
            match me.client.continue_upload_chunk(slice) {
                Ok(Some(id)) => id.0 as c_int,
                Ok(None) => 0,
                Err(e) => me.set_error(e),
            }
        }
        None => me.set_error(format_err!("no upload currently running")),
    }
}

/// Run the main receive loop. Returns `0` on success, `-1` on error.
#[no_mangle]
pub extern "C" fn proxmox_backup_poll_read(me: *mut CClient) -> c_int {
    let me = unsafe { &mut *me };
    match me.client.poll_read(false) {
        Ok(_) => 0,
        Err(e) => me.set_error(e),
    }
}

/// Run the main send loop. If the `write` callback returned `-EAGAIN`, during an operation, the
/// protocol handler keeps the data to be sent in a write queue. This function will attempt to
/// continue writing out the remaining data. See individual function descriptions for when this is
/// necessary.
///
/// Returns `1` if the queue is now empty, `0` if there is still data in the queue, or `-1` on
/// error.
#[no_mangle]
pub extern "C" fn proxmox_backup_poll_send(me: *mut CClient) -> c_int {
    let me = unsafe { &mut *me };
    me.bool_call(move |c| Ok(c.poll_send()?.unwrap_or(true)))
}

/// Run the main receive loop and check for confirmation of a stream with the specified ID.
///
/// Returns `1` if the transaction was confirmed, `0` if not, or `-1` on error.
///
/// Note that once this function returned `1` for an ID, the id is considered to be free for
/// recycling and should not be used for further calls.
#[no_mangle]
pub extern "C" fn proxmox_backup_wait_for_id(me: *mut CClient, id: c_int) -> c_int {
    let me = unsafe { &mut *me };
    me.bool_call(move |c| c.wait_for_id(crate::StreamId(id as u8)))
}

/// Notifies the protocol handler that we do not bother waiting for confirmation of an ID. The ID
/// may immediately be recycled for future transactions, thus the user should not use it for any
/// further function calls.
///
/// Returns `0` on success, `-1` on error.
#[no_mangle]
pub extern "C" fn proxmox_backup_discard_id(me: *mut CClient, id: c_int) -> c_int {
    let me = unsafe { &mut *me };
    match me.client.discard_id(crate::StreamId(id as u8)) {
        Ok(_) => 0,
        Err(e) => me.set_error(e),
    }
}

/// Create a new backup. The returned ID should be waited upon via `proxmox_backup_wait_for_id`,
/// which returns true once the server confirmed the creation of the backup.
#[no_mangle]
pub extern "C" fn proxmox_backup_create(
    me: *mut CClient,
    dynamic: bool,
    backup_type: *const c_char, // "host", "ct", "vm"
    backup_id: *const c_char,
    time_epoch: i64,
    file_name: *const c_char,
    chunk_size: size_t,
    file_size: i64,
    is_new: bool,
) -> c_int {
    let me = unsafe { &mut *me };
    me.int_call(move |client| {
        let index_type = match dynamic {
            false => crate::IndexType::Fixed,
            _ => crate::IndexType::Dynamic,
        };

        let backup_type = check_string(backup_type)?;
        let backup_id = check_string(backup_id)?;
        let file_name = check_string(file_name)?;

        Ok(client
            .create_backup(
                index_type,
                backup_type,
                backup_id,
                time_epoch,
                file_name,
                chunk_size as _,
                if file_size < 0 {
                    None
                } else {
                    Some(file_size as u64)
                },
                is_new,
            )?
            .0 as c_int)
    })
}

/// Send a dynamic chunk entry.
///
/// If the entry was sent out successfully this returns `1`. If the `write` callback returned
/// `-EAGAIN` this returns `0` and the data is queued, after which `proxmox_backup_poll_send`
/// should be used to continue sending the data.
/// On error `-1` is returned.
#[no_mangle]
pub extern "C" fn proxmox_backup_dynamic_data(
    me: *mut CClient,
    stream: c_int,
    digest: *const [u8; 32],
    size: u64,
) -> c_int {
    let me = unsafe { &mut *me };
    me.bool_call(move |client| {
        client.dynamic_data(crate::BackupStream(stream as u8), unsafe { &*digest }, size)
    })
}

/// Send a fixed chunk entry.
///
/// If the entry was sent out successfully this returns `1`. If the `write` callback returned
/// `-EAGAIN` this returns `0` and the data is queued, after which `proxmox_backup_poll_send`
/// should be used to continue sending the data.
/// On error `-1` is returned.
#[no_mangle]
pub extern "C" fn proxmox_backup_fixed_data(
    me: *mut CClient,
    stream: c_int,
    index: size_t,
    digest: *const [u8; 32],
) -> c_int {
    let me = unsafe { &mut *me };
    me.bool_call(move |client| {
        client.fixed_data(crate::BackupStream(stream as u8), index as usize, unsafe {
            &*digest
        })
    })
}

/// Finish a running backup.
///
/// Tells the server that the backup is supposed to be considered complete. If the request could be
/// sent out entirely `1` is returned. If the underlying socket is non-blocking and the packet
/// wasn't finished `0` is returned, after which `proxmox_backup_poll_send` should be used.
///
/// Once the request was sent out successfully, the client should wait for acknowledgement by the
/// remote server via `proxmox_backup_wait_for_id`, passing the backup stream ID as parameter.
///
/// Finally, if the client wishes to know the exact name the server stored the file under, the
/// `remote_path` parameter can be non-`NULL` to receive a string containing the file name, which
/// must be freed by the caller!
///
/// Returns: `1` on success, possibly `0` for non-blocking I/O, `-1` on error.
#[no_mangle]
pub extern "C" fn proxmox_backup_finish_backup(
    me: *mut CClient,
    stream: c_int,
    remote_path: *mut *mut c_char,
) -> c_int {
    let me = unsafe { &mut *me };
    me.int_call(move |client| {
        let (path, ack) = client.finish_backup(crate::BackupStream(stream as u8))?;

        if !remote_path.is_null() {
            // would be shorter with the unstable map_or_else
            let cstr = CString::new(path)
                .map(|cs| cs.into_raw())
                .unwrap_or(std::ptr::null_mut());
            unsafe {
                *remote_path = cstr;
            }
        }

        Ok(if ack { 1 } else { 0 })
    })
}
