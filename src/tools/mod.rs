//! Tools and utilities
//!
//! This is a collection of small and useful tools.
use std::any::Any;

use anyhow::{bail, format_err, Error};
use openssl::hash::{hash, DigestBytes, MessageDigest};

use proxmox_http::{client::SimpleHttp, client::SimpleHttpOptions, ProxyConfig};

pub mod apt;
pub mod config;
pub mod disks;
pub mod fs;

mod shared_rate_limiter;
pub use shared_rate_limiter::SharedRateLimiter;

pub mod statistics;
pub mod subscription;
pub mod systemd;
pub mod ticket;

pub mod parallel_handler;

/// Shortcut for md5 sums.
pub fn md5sum(data: &[u8]) -> Result<DigestBytes, Error> {
    hash(MessageDigest::md5(), data).map_err(Error::from)
}

pub fn get_hardware_address() -> Result<String, Error> {
    static FILENAME: &str = "/etc/ssh/ssh_host_rsa_key.pub";

    let contents = proxmox_sys::fs::file_get_contents(FILENAME)
        .map_err(|e| format_err!("Error getting host key - {}", e))?;
    let digest = md5sum(&contents).map_err(|e| format_err!("Error digesting host key - {}", e))?;

    Ok(hex::encode(&digest).to_uppercase())
}

pub fn assert_if_modified(digest1: &str, digest2: &str) -> Result<(), Error> {
    if digest1 != digest2 {
        bail!("detected modified configuration - file changed by other user? Try again.");
    }
    Ok(())
}

/// Detect modified configuration files
///
/// This function fails with a reasonable error message if checksums do not match.
pub fn detect_modified_configuration_file(
    digest1: &[u8; 32],
    digest2: &[u8; 32],
) -> Result<(), Error> {
    if digest1 != digest2 {
        bail!("detected modified configuration - file changed by other user? Try again.");
    }
    Ok(())
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

pub fn setup_safe_path_env() {
    std::env::set_var("PATH", "/sbin:/bin:/usr/sbin:/usr/bin");
    // Make %ENV safer - as suggested by https://perldoc.perl.org/perlsec.html
    for name in &["IFS", "CDPATH", "ENV", "BASH_ENV"] {
        std::env::remove_var(name);
    }
}
