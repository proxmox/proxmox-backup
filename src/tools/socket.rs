use std::os::unix::io::RawFd;

use nix::sys::socket::sockopt::{KeepAlive, TcpKeepIdle};
use nix::sys::socket::setsockopt;

pub const PROXMOX_BACKUP_TCP_KEEPALIVE_TIME: u32 = 120;

/// Set TCP keepalive time on a socket
///
/// See "man 7 tcp" for details.
///
/// The default on Linix is 7200 (2 hours) which is much too long for
/// our backup tools.
pub fn set_tcp_keepalive(
    socket_fd: RawFd,
    tcp_keepalive_time: u32,
) -> nix::Result<()> {

    setsockopt(socket_fd, KeepAlive, &true)?;
    setsockopt(socket_fd, TcpKeepIdle, &tcp_keepalive_time)?;

    Ok(())
}
