use std::os::unix::io::RawFd;

use anyhow::{bail, format_err, Error};

use proxmox::tools::fd::Fd;
use proxmox::api::UserInformation;

mod compression;
pub use compression::*;

pub mod daemon;
pub mod formatter;

mod environment;
pub use environment::*;

mod state;
pub use state::*;

mod command_socket;
pub use command_socket::*;

mod file_logger;
pub use file_logger::{FileLogger, FileLogOptions};

mod api_config;
pub use api_config::ApiConfig;

mod rest;
pub use rest::{RestServer, handle_api_request};

pub enum AuthError {
    Generic(Error),
    NoData,
}

impl From<Error> for AuthError {
    fn from(err: Error) -> Self {
        AuthError::Generic(err)
    }
}

pub trait ApiAuth {
    fn check_auth(
        &self,
        headers: &http::HeaderMap,
        method: &hyper::Method,
    ) -> Result<(String, Box<dyn UserInformation + Sync + Send>), AuthError>;
}

static mut SHUTDOWN_REQUESTED: bool = false;

pub fn request_shutdown() {
    unsafe {
        SHUTDOWN_REQUESTED = true;
    }
    crate::server_shutdown();
}

#[inline(always)]
pub fn shutdown_requested() -> bool {
    unsafe { SHUTDOWN_REQUESTED }
}

pub fn fail_on_shutdown() -> Result<(), Error> {
    if shutdown_requested() {
        bail!("Server shutdown requested - aborting task");
    }
    Ok(())
}

/// Helper to set/clear the FD_CLOEXEC flag on file descriptors
pub fn fd_change_cloexec(fd: RawFd, on: bool) -> Result<(), Error> {
    use nix::fcntl::{fcntl, FdFlag, F_GETFD, F_SETFD};
    let mut flags = FdFlag::from_bits(fcntl(fd, F_GETFD)?)
        .ok_or_else(|| format_err!("unhandled file flags"))?; // nix crate is stupid this way...
    flags.set(FdFlag::FD_CLOEXEC, on);
    fcntl(fd, F_SETFD(flags))?;
    Ok(())
}

/// safe wrapper for `nix::sys::socket::socketpair` defaulting to `O_CLOEXEC` and guarding the file
/// descriptors.
pub fn socketpair() -> Result<(Fd, Fd), Error> {
    use nix::sys::socket;
    let (pa, pb) = socket::socketpair(
        socket::AddressFamily::Unix,
        socket::SockType::Stream,
        None,
        socket::SockFlag::SOCK_CLOEXEC,
    )?;
    Ok((Fd(pa), Fd(pb)))
}


/// Extract a specific cookie from cookie header.
/// We assume cookie_name is already url encoded.
pub fn extract_cookie(cookie: &str, cookie_name: &str) -> Option<String> {
    for pair in cookie.split(';') {
        let (name, value) = match pair.find('=') {
            Some(i) => (pair[..i].trim(), pair[(i + 1)..].trim()),
            None => return None, // Cookie format error
        };

        if name == cookie_name {
            use percent_encoding::percent_decode;
            if let Ok(value) = percent_decode(value.as_bytes()).decode_utf8() {
                return Some(value.into());
            } else {
                return None; // Cookie format error
            }
        }
    }

    None
}

/// normalize uri path
///
/// Do not allow ".", "..", or hidden files ".XXXX"
/// Also remove empty path components
pub fn normalize_uri_path(path: &str) -> Result<(String, Vec<&str>), Error> {
    let items = path.split('/');

    let mut path = String::new();
    let mut components = vec![];

    for name in items {
        if name.is_empty() {
            continue;
        }
        if name.starts_with('.') {
            bail!("Path contains illegal components.");
        }
        path.push('/');
        path.push_str(name);
        components.push(name);
    }

    Ok((path, components))
}
