//! C API for the Chunker.

use std::os::raw::c_void;

use libc::size_t;

use crate::Chunker;

/// Creates a new chunker instance.
#[no_mangle]
pub extern "C" fn proxmox_chunker_new(chunk_size_avg: size_t) -> *mut Chunker {
    Box::leak(Box::new(Chunker::new(chunk_size_avg as usize)))
}

/// Drops an instance of a chunker. The pointer must be valid or `NULL`.
#[no_mangle]
pub extern "C" fn proxmox_chunker_done(me: *mut Chunker) {
    if !me.is_null() {
        unsafe {
            Box::from_raw(me);
        }
    }
}

/// Scan the specified data for a chunk border. Returns 0 if none was found, or a positive offset
/// to a border.
#[no_mangle]
pub extern "C" fn proxmox_chunker_scan(
    me: *mut Chunker,
    data: *const c_void,
    size: size_t,
) -> size_t {
    let me = unsafe { &mut *me };
    me.scan(unsafe { std::slice::from_raw_parts(data as *const u8, size as usize) }) as size_t
}

/// Compute a chunk digest. This is mostly a convenience method to avoid having to lookup the right
/// digest method for your language of choice.
#[no_mangle]
pub extern "C" fn proxmox_chunk_digest(
    data: *const c_void,
    size: size_t,
    out_digest: *mut [u8; 32],
) {
    let digest = crate::FixedChunk::from_data(unsafe {
        std::slice::from_raw_parts(data as *const u8, size as usize)
    });
    unsafe { *out_digest = digest.0 };
}
