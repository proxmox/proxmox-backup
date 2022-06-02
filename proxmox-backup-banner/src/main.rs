use anyhow::{format_err, Error};

use std::fmt::Write;
use std::fs;
use std::net::ToSocketAddrs;
use std::os::unix::prelude::OsStrExt;

use nix::sys::utsname::uname;

fn nodename() -> Result<String, Error> {
    let uname = uname().map_err(|err| format_err!("uname() failed - {err}"))?; // save on stack to avoid to_owned() allocation below
    std::str::from_utf8(uname.nodename().as_bytes())?
        .split('.')
        .next()
        .ok_or_else(|| format_err!("Failed to split FQDN to get hostname"))
        .map(|s| s.to_owned())
}

fn main() {
    let nodename = match nodename() {
        Ok(value) => value,
        Err(err) => {
            eprintln!("Failed to retrieve hostname: {err}");
            "INVALID".to_string()
        }
    };

    let addr = format!("{}:8007", nodename);

    let mut banner = format!(
        "
{:-<78}

Welcome to the Proxmox Backup Server. Please use your web browser to
configure this server - connect to:

",
        ""
    );

    let msg = match addr.to_socket_addrs() {
        Ok(saddrs) => {
            let saddrs: Vec<_> = saddrs
                .filter_map(|s| match !s.ip().is_loopback() {
                    true => Some(format!(" https://{}/", s)),
                    false => None,
                })
                .collect();

            if !saddrs.is_empty() {
                saddrs.join("\n")
            } else {
                format!(
                    "hostname '{}' does not resolves to any non-loopback address",
                    nodename
                )
            }
        }
        Err(e) => format!("could not resolve hostname '{}': {}", nodename, e),
    };
    banner += &msg;

    // unwrap will never fail for write!:
    // https://github.com/rust-lang/rust/blob/1.39.0/src/liballoc/string.rs#L2318-L2331
    write!(&mut banner, "\n\n{:-<78}\n\n", "").unwrap();

    fs::write("/etc/issue", banner.as_bytes()).expect("Unable to write banner to issue file");
}
