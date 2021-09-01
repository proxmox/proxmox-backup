//! Raw file descriptor related utilities.

use std::os::unix::io::RawFd;

use anyhow::Error;
use nix::fcntl::{fcntl, FdFlag, F_GETFD, F_SETFD};

/// Change the `O_CLOEXEC` flag of an existing file descriptor.
pub fn fd_change_cloexec(fd: RawFd, on: bool) -> Result<(), Error> {
    let mut flags = unsafe { FdFlag::from_bits_unchecked(fcntl(fd, F_GETFD)?) };
    flags.set(FdFlag::FD_CLOEXEC, on);
    fcntl(fd, F_SETFD(flags))?;
    Ok(())
}
