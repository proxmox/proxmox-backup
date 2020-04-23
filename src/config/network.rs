use std::io::{Write};
use std::collections::{HashSet, HashMap};

use anyhow::{Error, format_err, bail};

use proxmox::tools::{fs::replace_file, fs::CreateOptions};

mod helper;
pub use helper::*;

mod lexer;
pub use lexer::*;

mod parser;
pub use parser::*;

use crate::api2::types::{Interface, NetworkConfigMethod, NetworkInterfaceType};

impl Interface {

    pub fn new(name: String) -> Self {
        Self {
            name,
            interface_type: NetworkInterfaceType::Unknown,
            auto: false,
            active: false,
            method_v4: None,
            method_v6: None,
            cidr_v4: None,
            gateway_v4: None,
            cidr_v6: None,
            gateway_v6: None,
            options_v4: Vec::new(),
            options_v6: Vec::new(),
            mtu: None,
            bridge_ports: None,
            bond_slaves: None,
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

    fn set_interface_type(&mut self, interface_type: NetworkInterfaceType) -> Result<(), Error> {
        if self.interface_type == NetworkInterfaceType::Unknown {
            self.interface_type = interface_type;
        } else if self.interface_type != interface_type {
            bail!("interface type already defined - cannot change from {:?} to {:?}", self.interface_type, interface_type);
        }
        Ok(())
    }

    pub(crate) fn set_bridge_ports(&mut self, ports: Vec<String>) -> Result<(), Error> {
        if self.interface_type != NetworkInterfaceType::Bridge {
            bail!("interface '{}' is no bridge (type is {:?})", self.name, self.interface_type);
        }
        self.bridge_ports = Some(ports);
        Ok(())
    }

    pub(crate) fn set_bond_slaves(&mut self, slaves: Vec<String>) -> Result<(), Error> {
        if self.interface_type != NetworkInterfaceType::Bond {
            bail!("interface '{}' is no bond (type is {:?})", self.name, self.interface_type);
        }
        self.bond_slaves = Some(slaves);
        Ok(())
    }

    fn push_addon_option(&mut self, text: String) {
        if self.method_v4.is_none() && self.method_v6.is_some() {
            self.options_v6.push(text);
        } else {
            self.options_v4.push(text);
        }
    }

    /// Write attributes not dependening on address family
    fn write_iface_attributes(&self, w: &mut dyn Write) -> Result<(), Error> {

        match self.interface_type {
            NetworkInterfaceType::Bridge => {
                if let Some(ref ports) = self.bridge_ports {
                    if ports.is_empty() {
                        writeln!(w, "    bridge-ports none")?;
                    } else {
                        writeln!(w, "    bridge-ports {}", ports.join(" "))?;
                    }
                }
            }
            NetworkInterfaceType::Bond => {
                if let Some(ref slaves) = self.bond_slaves {
                    if slaves.is_empty() {
                        writeln!(w, "    bond-slaves none")?;
                    } else {
                        writeln!(w, "    bond-slaves {}", slaves.join(" "))?;
                    }
                }
            }
            _ => {}
        }

        if let Some(mtu) = self.mtu {
            writeln!(w, "    mtu {}", mtu)?;
        }

        Ok(())
    }

    /// Write attributes dependening on address family inet (IPv4)
    fn write_iface_attributes_v4(&self, w: &mut dyn Write, method: NetworkConfigMethod) -> Result<(), Error> {
        if method == NetworkConfigMethod::Static {
            if let Some(address) = &self.cidr_v4 {
                writeln!(w, "    address {}", address)?;
            }
            if let Some(gateway) = &self.gateway_v4 {
                writeln!(w, "    gateway {}", gateway)?;
            }
        }

        for option in &self.options_v4 {
            writeln!(w, "    {}", option)?;
        }

        Ok(())
    }

    /// Write attributes dependening on address family inet6 (IPv6)
    fn write_iface_attributes_v6(&self, w: &mut dyn Write, method: NetworkConfigMethod) -> Result<(), Error> {
        if method == NetworkConfigMethod::Static {
            if let Some(address) = &self.cidr_v6 {
                writeln!(w, "    address {}", address)?;
            }
            if let Some(gateway) = &self.gateway_v6 {
                writeln!(w, "    gateway {}", gateway)?;
            }
        }

        for option in &self.options_v6 {
            writeln!(w, "    {}", option)?;
        }

        Ok(())
    }

    /// Return whether we can write a single entry for inet and inet6
    fn combine_entry(&self) -> bool {
        // Note: use match to make sure we considered all values at compile time
        match self {
            Interface {
                method_v4,
                method_v6,
                options_v4,
                options_v6,
                // the rest does not matter
                name: _name,
                interface_type: _interface_type,
                auto: _auto,
                active: _active,
                cidr_v4: _cidr_v4,
                cidr_v6: _cidr_v6,
                gateway_v4: _gateway_v4,
                gateway_v6: _gateway_v6,
                mtu: _mtu,
                bridge_ports: _bridge_ports,
                bond_slaves: _bond_slaves,
            } => {
                method_v4 == method_v6
                    && options_v4.is_empty()
                    && options_v6.is_empty()
            }
        }
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

        if self.method_v4.is_none() && self.method_v6.is_none() { return Ok(()); }

        if self.auto {
            writeln!(w, "auto {}", self.name)?;
        }

        if self.combine_entry() {
            if let Some(method) = self.method_v4 {
                writeln!(w, "iface {} {}", self.name, method_to_str(method))?;
                self.write_iface_attributes_v4(w, method)?;
                self.write_iface_attributes_v6(w, method)?;
                self.write_iface_attributes(w)?;
                writeln!(w)?;
            }
        } else {
            if let Some(method) = self.method_v4 {
                writeln!(w, "iface {} inet {}", self.name, method_to_str(method))?;
                self.write_iface_attributes_v4(w, method)?;
                writeln!(w)?;
            }
            if let Some(method) = self.method_v6 {
                writeln!(w, "iface {} inet6 {}", self.name, method_to_str(method))?;
                self.write_iface_attributes_v6(w, method)?;
                writeln!(w)?;
            }
            self.write_iface_attributes(w)?;
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

    pub fn lookup(&self, name: &str) -> Result<&Interface, Error> {
        let interface = self.interfaces.get(name).ok_or_else(|| {
            format_err!("interface '{}' does not exist.", name)
        })?;
        Ok(interface)
    }

    pub fn lookup_mut(&mut self, name: &str) -> Result<&mut Interface, Error> {
        let interface = self.interfaces.get_mut(name).ok_or_else(|| {
            format_err!("interface '{}' does not exist.", name)
        })?;
        Ok(interface)
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
pub const NETWORK_INTERFACES_NEW_FILENAME: &str = "/etc/network/interfaces.new";
pub const NETWORK_LOCKFILE: &str = "/var/lock/pve-network.lck";


pub fn config() -> Result<(NetworkConfig, [u8;32]), Error> {
    let content = std::fs::read(NETWORK_INTERFACES_NEW_FILENAME)
        .or_else(|err| {
            if err.kind() == std::io::ErrorKind::NotFound {
                std::fs::read(NETWORK_INTERFACES_FILENAME)
                    .or_else(|err| {
                        if err.kind() == std::io::ErrorKind::NotFound {
                            Ok(Vec::new())
                        } else {
                            bail!("unable to read '{}' - {}", NETWORK_INTERFACES_FILENAME, err);
                         }
                    })
            } else {
                bail!("unable to read '{}' - {}", NETWORK_INTERFACES_NEW_FILENAME, err);
            }
        })?;


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

    replace_file(NETWORK_INTERFACES_NEW_FILENAME, &raw, options)?;

    Ok(())
}

// shell completion helper
pub fn complete_interface_name(_arg: &str, _param: &HashMap<String, String>) -> Vec<String> {
    match config() {
        Ok((data, _digest)) => data.interfaces.keys().map(|id| id.to_string()).collect(),
        Err(_) => return vec![],
    }
}
