use std::io::{Write};
use std::collections::{HashSet, HashMap};

use anyhow::{Error, bail};

use proxmox::tools::{fs::replace_file, fs::CreateOptions};

mod helper;
pub use helper::*;

mod lexer;
pub use lexer::*;

mod parser;
pub use parser::*;

use crate::api2::types::{Interface, NetworkConfigMethod};

impl Interface {

    pub fn new(name: String) -> Self {
        Self {
            name,
            autostart: false,
            exists: false,
            active: false,
            method_v4: None,
            method_v6: None,
            cidr_v4: None,
            gateway_v4: None,
            cidr_v6: None,
            gateway_v6: None,
            options_v4: Vec::new(),
            options_v6: Vec::new(),
        }
    }

    fn set_method_v4(&mut self, method: NetworkConfigMethod) -> Result<(), Error> {
        if self.method_v4.is_none() {
            self.method_v4 = Some(method);
        } else {
            bail!("inet configuration method already set.");
        }
        Ok(())
    }

    fn set_method_v6(&mut self, method: NetworkConfigMethod) -> Result<(), Error> {
        if self.method_v6.is_none() {
            self.method_v6 = Some(method);
        } else {
            bail!("inet6 configuration method already set.");
        }
        Ok(())
    }

    fn set_cidr_v4(&mut self, address: String) -> Result<(), Error> {
        if self.cidr_v4.is_none() {
            self.cidr_v4 = Some(address);
        } else {
            bail!("duplicate IPv4 address.");
        }
        Ok(())
    }

    fn set_gateway_v4(&mut self, gateway: String) -> Result<(), Error> {
        if self.gateway_v4.is_none() {
            self.gateway_v4 = Some(gateway);
        } else {
            bail!("duplicate IPv4 gateway.");
        }
        Ok(())
    }

    fn set_cidr_v6(&mut self, address: String) -> Result<(), Error> {
        if self.cidr_v6.is_none() {
            self.cidr_v6 = Some(address);
        } else {
            bail!("duplicate IPv6 address.");
        }
        Ok(())
    }

    fn set_gateway_v6(&mut self, gateway: String) -> Result<(), Error> {
        if self.gateway_v6.is_none() {
            self.gateway_v6 = Some(gateway);
        } else {
            bail!("duplicate IPv4 gateway.");
        }
        Ok(())
    }

    fn push_addon_option(&mut self, text: String) {
        if self.method_v4.is_none() && self.method_v6.is_some() {
            self.options_v6.push(text);
        } else {
            self.options_v4.push(text);
        }
    }

    fn write_iface_attributes_v4(&self, w: &mut dyn Write) -> Result<(), Error> {
        if let Some(address) = &self.cidr_v4 {
            writeln!(w, "    address {}", address)?;
        }
        if let Some(gateway) = &self.gateway_v4 {
            writeln!(w, "    gateway {}", gateway)?;
        }
        for option in &self.options_v4 {
            writeln!(w, "    {}", option)?;
        }

        Ok(())
    }

    fn write_iface_attributes_v6(&self, w: &mut dyn Write) -> Result<(), Error> {
        if let Some(address) = &self.cidr_v6 {
            writeln!(w, "    address {}", address)?;
        }
        if let Some(gateway) = &self.gateway_v6 {
            writeln!(w, "    gateway {}", gateway)?;
        }
        for option in &self.options_v6 {
            writeln!(w, "    {}", option)?;
        }

        Ok(())
    }

    fn write_iface(&self, w: &mut dyn Write) -> Result<(), Error> {

        fn method_to_str(method: NetworkConfigMethod) -> &'static str {
            match method {
                NetworkConfigMethod::Static => "static",
                NetworkConfigMethod::Loopback => "loopback",
                NetworkConfigMethod::Manual => "manual",
                NetworkConfigMethod::DHCP => "dhcp",
            }
        }

        if self.autostart {
            writeln!(w, "auto {}", self.name)?;
        }

        if self.method_v4 == self.method_v6 {
            let method = self.method_v4.unwrap_or(NetworkConfigMethod::Static);
            writeln!(w, "iface {} {}", self.name, method_to_str(method))?;
            self.write_iface_attributes_v4(w)?;
            self.write_iface_attributes_v6(w)?;
            writeln!(w)?;
        } else {
            if let Some(method) = self.method_v4 {
                writeln!(w, "iface {} inet {}", self.name, method_to_str(method))?;
                self.write_iface_attributes_v4(w)?;
                writeln!(w)?;
            }
            if let Some(method) = self.method_v6 {
                writeln!(w, "iface {} inet6 {}", self.name, method_to_str(method))?;
                self.write_iface_attributes_v6(w)?;
                writeln!(w)?;
            }
        }
        Ok(())
    }
}

#[derive(Debug)]
enum NetworkOrderEntry {
    Iface(String),
    Comment(String),
    Option(String),
}

#[derive(Debug)]
pub struct NetworkConfig {
    pub interfaces: HashMap<String, Interface>,
    order: Vec<NetworkOrderEntry>,
}

impl NetworkConfig {

    pub fn new() -> Self {
        Self {
            interfaces: HashMap::new(),
            order: Vec::new(),
        }
    }

    pub fn write_config(&self, w: &mut dyn Write) -> Result<(), Error> {

        let mut done = HashSet::new();

        let mut last_entry_was_comment = false;

        for entry in self.order.iter() {
             match entry {
                NetworkOrderEntry::Comment(comment) => {
                    writeln!(w, "#{}", comment)?;
                    last_entry_was_comment = true;
                }
                NetworkOrderEntry::Option(option) => {
                    if last_entry_was_comment {  writeln!(w)?; }
                    last_entry_was_comment = false;
                    writeln!(w, "{}", option)?;
                    writeln!(w)?;
                }
                NetworkOrderEntry::Iface(name) => {
                    let interface = match self.interfaces.get(name) {
                        Some(interface) => interface,
                        None => continue,
                    };

                    if last_entry_was_comment {  writeln!(w)?; }
                    last_entry_was_comment = false;

                    if done.contains(name) { continue; }
                    done.insert(name);

                    interface.write_iface(w)?;
                }
            }
        }

        for (name, interface) in &self.interfaces {
            if done.contains(name) { continue; }
            interface.write_iface(w)?;
        }
        Ok(())
    }
}

pub const NETWORK_INTERFACES_FILENAME: &str = "/etc/network/interfaces";
pub const NETWORK_LOCKFILE: &str = "/var/lock/pve-network.lck";

pub fn config() -> Result<(NetworkConfig, [u8;32]), Error> {
    let content = match std::fs::read(NETWORK_INTERFACES_FILENAME) {
        Ok(c) => c,
        Err(err) => {
            if err.kind() == std::io::ErrorKind::NotFound {
                Vec::new()
            } else {
                bail!("unable to read '{}' - {}", NETWORK_INTERFACES_FILENAME, err);
            }
        }
    };

    let digest = openssl::sha::sha256(&content);

    let mut parser = NetworkParser::new(&content[..]);
    let data = parser.parse_interfaces()?;

    Ok((data, digest))
}

pub fn save_config(config: &NetworkConfig) -> Result<(), Error> {

    let mut raw = Vec::new();
    config.write_config(&mut raw)?;

    let mode = nix::sys::stat::Mode::from_bits_truncate(0o0644);
    // set the correct owner/group/permissions while saving file
    // owner(rw) = root, group(r)=root, others(r)
    let options = CreateOptions::new()
        .perm(mode)
        .owner(nix::unistd::ROOT)
        .group(nix::unistd::Gid::from_raw(0));

    replace_file(NETWORK_INTERFACES_FILENAME, &raw, options)?;

    Ok(())
}
