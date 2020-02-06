use std::fmt::Write;
use std::fs;
use std::net::ToSocketAddrs;

use proxmox::tools;

fn main() {
    let nodename = tools::nodename();
    let addr = format!("{}:8007", nodename);

    let mut banner = format!("
{:-<78}

Welcome to the Proxmox Backup Server. Please use your web browser to
configure this server - connect to:

",
        ""
    );

    if let Ok(saddrs) = addr.to_socket_addrs() {
        let saddrs: Vec<_> = saddrs
            .filter_map(|s| match !s.ip().is_loopback() {
                true => Some(format!(" https://{}/", s)),
                false => None,
            })
            .collect();

        if !saddrs.is_empty() {
            writeln!(&mut banner, "{}", saddrs.join("\n")).unwrap();
        } else {
            writeln!(
                &mut banner,
                "hostname '{}' does not resolves to any non-loopback address",
                nodename
            )
            .unwrap();
        }
    } else {
        writeln!(&mut banner, "could not resolve hostname '{}'", nodename).unwrap();
    }

    // unwrap will never fail for write!:
    // https://github.com/rust-lang/rust/blob/1.39.0/src/liballoc/string.rs#L2318-L2331
    write!(&mut banner, "\n{:-<78}\n\n", "").unwrap();

    fs::write("/etc/issue", banner.as_bytes()).expect("Unable to write banner to issue file");
}
