//! Tools and utilities
//!
//! This is a collection of small and useful tools.
use std::any::Any;
use std::fs::File;
use std::io::{self, BufRead};
use std::os::unix::io::RawFd;
use std::path::Path;

use anyhow::{bail, format_err, Error};
use serde_json::Value;
use openssl::hash::{hash, DigestBytes, MessageDigest};

pub use proxmox::tools::fd::Fd;
use proxmox::tools::fs::{create_path, CreateOptions};

use proxmox_http::{
    client::SimpleHttp,
    client::SimpleHttpOptions,
    ProxyConfig,
};

pub use pbs_tools::json;
pub use pbs_tools::nom;
pub use pbs_tools::{run_command, command_output, command_output_as_string};
pub use pbs_tools::process_locker::{
    ProcessLocker, ProcessLockExclusiveGuard, ProcessLockSharedGuard
};

pub mod apt;
pub mod async_io;
pub mod compression;
pub mod config;
pub mod cpio;
pub mod daemon;
pub mod disks;
pub mod fuse_loop;

mod memcom;
pub use memcom::Memcom;

pub mod logrotate;
pub mod loopdev;
pub mod lru_cache;
pub mod async_lru_cache;
pub mod serde_filter;
pub mod statistics;
pub mod subscription;
pub mod systemd;
pub mod ticket;
pub mod sgutils2;
pub mod paperkey;

pub mod parallel_handler;
pub use parallel_handler::ParallelHandler;

mod wrapped_reader_stream;
pub use wrapped_reader_stream::{AsyncReaderStream, StdChannelStream, WrappedReaderStream};

mod async_channel_writer;
pub use async_channel_writer::AsyncChannelWriter;

mod file_logger;
pub use file_logger::{FileLogger, FileLogOptions};

pub use pbs_tools::broadcast_future::{BroadcastData, BroadcastFuture};
pub use pbs_tools::ops::ControlFlow;

/// The `BufferedRead` trait provides a single function
/// `buffered_read`. It returns a reference to an internal buffer. The
/// purpose of this traid is to avoid unnecessary data copies.
pub trait BufferedRead {
    /// This functions tries to fill the internal buffers, then
    /// returns a reference to the available data. It returns an empty
    /// buffer if `offset` points to the end of the file.
    fn buffered_read(&mut self, offset: u64) -> Result<&[u8], Error>;
}

pub fn required_string_param<'a>(param: &'a Value, name: &str) -> Result<&'a str, Error> {
    match param[name].as_str() {
        Some(s) => Ok(s),
        None => bail!("missing parameter '{}'", name),
    }
}

pub fn required_string_property<'a>(param: &'a Value, name: &str) -> Result<&'a str, Error> {
    match param[name].as_str() {
        Some(s) => Ok(s),
        None => bail!("missing property '{}'", name),
    }
}

pub fn required_integer_param(param: &Value, name: &str) -> Result<i64, Error> {
    match param[name].as_i64() {
        Some(s) => Ok(s),
        None => bail!("missing parameter '{}'", name),
    }
}

pub fn required_integer_property(param: &Value, name: &str) -> Result<i64, Error> {
    match param[name].as_i64() {
        Some(s) => Ok(s),
        None => bail!("missing property '{}'", name),
    }
}

pub fn required_array_param<'a>(param: &'a Value, name: &str) -> Result<&'a [Value], Error> {
    match param[name].as_array() {
        Some(s) => Ok(&s),
        None => bail!("missing parameter '{}'", name),
    }
}

pub fn required_array_property<'a>(param: &'a Value, name: &str) -> Result<&'a [Value], Error> {
    match param[name].as_array() {
        Some(s) => Ok(&s),
        None => bail!("missing property '{}'", name),
    }
}

/// Shortcut for md5 sums.
pub fn md5sum(data: &[u8]) -> Result<DigestBytes, Error> {
    hash(MessageDigest::md5(), data).map_err(Error::from)
}

pub fn get_hardware_address() -> Result<String, Error> {
    static FILENAME: &str = "/etc/ssh/ssh_host_rsa_key.pub";

    let contents = proxmox::tools::fs::file_get_contents(FILENAME)
        .map_err(|e| format_err!("Error getting host key - {}", e))?;
    let digest = md5sum(&contents)
        .map_err(|e| format_err!("Error digesting host key - {}", e))?;

    Ok(proxmox::tools::bin_to_hex(&digest).to_uppercase())
}

pub fn assert_if_modified(digest1: &str, digest2: &str) -> Result<(), Error> {
    if digest1 != digest2 {
        bail!("detected modified configuration - file changed by other user? Try again.");
    }
    Ok(())
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

/// Detect modified configuration files
///
/// This function fails with a reasonable error message if checksums do not match.
pub fn detect_modified_configuration_file(digest1: &[u8;32], digest2: &[u8;32]) -> Result<(), Error> {
    if digest1 != digest2 {
        bail!("detected modified configuration - file changed by other user? Try again.");
    }
    Ok(())
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

pub fn fd_change_cloexec(fd: RawFd, on: bool) -> Result<(), Error> {
    use nix::fcntl::{fcntl, FdFlag, F_GETFD, F_SETFD};
    let mut flags = FdFlag::from_bits(fcntl(fd, F_GETFD)?)
        .ok_or_else(|| format_err!("unhandled file flags"))?; // nix crate is stupid this way...
    flags.set(FdFlag::FD_CLOEXEC, on);
    fcntl(fd, F_SETFD(flags))?;
    Ok(())
}

static mut SHUTDOWN_REQUESTED: bool = false;

pub fn request_shutdown() {
    unsafe {
        SHUTDOWN_REQUESTED = true;
    }
    crate::server::server_shutdown();
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

/// safe wrapper for `nix::unistd::pipe2` defaulting to `O_CLOEXEC` and guarding the file
/// descriptors.
pub fn pipe() -> Result<(Fd, Fd), Error> {
    let (pin, pout) = nix::unistd::pipe2(nix::fcntl::OFlag::O_CLOEXEC)?;
    Ok((Fd(pin), Fd(pout)))
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


/// An easy way to convert types to Any
///
/// Mostly useful to downcast trait objects (see RpcEnvironment).
pub trait AsAny {
    fn as_any(&self) -> &dyn Any;
}

impl<T: Any> AsAny for T {
    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// The default 2 hours are far too long for PBS
pub const PROXMOX_BACKUP_TCP_KEEPALIVE_TIME: u32 = 120;
pub const DEFAULT_USER_AGENT_STRING: &'static str = "proxmox-backup-client/1.0";

/// Returns a new instance of `SimpleHttp` configured for PBS usage.
pub fn pbs_simple_http(proxy_config: Option<ProxyConfig>) -> SimpleHttp {
    let options = SimpleHttpOptions {
        proxy_config,
        user_agent: Some(DEFAULT_USER_AGENT_STRING.to_string()),
        tcp_keepalive: Some(PROXMOX_BACKUP_TCP_KEEPALIVE_TIME),
        ..Default::default()
    };

    SimpleHttp::with_options(options)
}

/// Get an iterator over lines of a file, skipping empty lines and comments (lines starting with a
/// `#`).
pub fn file_get_non_comment_lines<P: AsRef<Path>>(
    path: P,
) -> Result<impl Iterator<Item = io::Result<String>>, Error> {
    let path = path.as_ref();

    Ok(io::BufReader::new(
        File::open(path).map_err(|err| format_err!("error opening {:?}: {}", path, err))?,
    )
    .lines()
    .filter_map(|line| match line {
        Ok(line) => {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                None
            } else {
                Some(Ok(line.to_string()))
            }
        }
        Err(err) => Some(Err(err)),
    }))
}

pub fn setup_safe_path_env() {
    std::env::set_var("PATH", "/sbin:/bin:/usr/sbin:/usr/bin");
    // Make %ENV safer - as suggested by https://perldoc.perl.org/perlsec.html
    for name in &["IFS", "CDPATH", "ENV", "BASH_ENV"] {
        std::env::remove_var(name);
    }
}

/// Create the base run-directory.
///
/// This exists to fixate the permissions for the run *base* directory while allowing intermediate
/// directories after it to have different permissions.
pub fn create_run_dir() -> Result<(), Error> {
    let backup_user = crate::backup::backup_user()?;
    let opts = CreateOptions::new()
        .owner(backup_user.uid)
        .group(backup_user.gid);
    let _: bool = create_path(pbs_buildcfg::PROXMOX_BACKUP_RUN_DIR_M!(), None, Some(opts))?;
    Ok(())
}
