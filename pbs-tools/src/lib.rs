pub mod cert;
pub mod crypt_config;
pub mod format;
pub mod json;
pub mod lru_cache;
pub mod nom;
pub mod sha;

pub mod async_lru_cache;

/// Set MMAP_THRESHOLD to a fixed value (128 KiB)
///
/// This avoids the "dynamic" mmap-treshold logic from glibc's malloc, which seems misguided and
/// effectively avoids using mmap for all allocations smaller than 32 MiB. Which, in combination
/// with the allocation pattern from our/tokio's complex async machinery, resulted in very large
/// RSS sizes due to defragmentation and long-living (smaller) allocation on top of the heap
/// avoiding that the (big) now free'd allocations below couldn't get given back to the OS. This is
/// not an issue with mmap'd memory chunks, those can be given back at any time.
///
/// Lowering effective MMAP threshold to 128 KiB allows freeing up memory to the OS better and with
/// lower latency, which reduces the peak *and* average RSS size by an order of magnitude when
/// running backup jobs. We measured a reduction by a factor of 10-20 in experiments and see much
/// less erratic behavior in the overall's runtime RSS size.
pub fn setup_libc_malloc_opts() {
    unsafe {
        libc::mallopt(libc::M_MMAP_THRESHOLD, 4096 * 32);
    }
}
