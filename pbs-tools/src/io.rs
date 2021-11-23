//! I/O utilities.

use proxmox_sys::fd::Fd;

/// The `BufferedRead` trait provides a single function
/// `buffered_read`. It returns a reference to an internal buffer. The
/// purpose of this traid is to avoid unnecessary data copies.
pub trait BufferedRead {
    /// This functions tries to fill the internal buffers, then
    /// returns a reference to the available data. It returns an empty
    /// buffer if `offset` points to the end of the file.
    fn buffered_read(&mut self, offset: u64) -> Result<&[u8], anyhow::Error>;
}

/// safe wrapper for `nix::unistd::pipe2` defaulting to `O_CLOEXEC` and guarding the file
/// descriptors.
pub fn pipe() -> Result<(Fd, Fd), nix::Error> {
    let (pin, pout) = nix::unistd::pipe2(nix::fcntl::OFlag::O_CLOEXEC)?;
    Ok((Fd(pin), Fd(pout)))
}
