use std::fmt::Write;
use std::fs;
use std::net::ToSocketAddrs;

use nix::sys::utsname::uname;

fn main() {
    let uname = uname(); // save on stack to avoid to_owned() allocation below
    let nodename = uname.nodename().split('.').next().unwrap();

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
