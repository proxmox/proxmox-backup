use std::io::{Write};
use std::collections::{HashSet, HashMap, BTreeMap};

use anyhow::{Error, format_err, bail};
use serde::de::{value, IntoDeserializer, Deserialize};

use proxmox::tools::{fs::replace_file, fs::CreateOptions};

mod helper;
pub use helper::*;

mod lexer;
pub use lexer::*;

mod parser;
pub use parser::*;

use crate::api2::types::{Interface, NetworkConfigMethod, NetworkInterfaceType, LinuxBondMode};

pub fn bond_mode_from_str(s: &str) -> Result<LinuxBondMode, Error> {
    LinuxBondMode::deserialize(s.into_deserializer())
        .map_err(|_: value::Error| format_err!("invalid bond_mode '{}'", s))
}

pub fn bond_mode_to_str(mode: LinuxBondMode) -> &'static str {
    match mode {
        LinuxBondMode::balance_rr => "balance-rr",
        LinuxBondMode::active_backup => "active-backup",
        LinuxBondMode::balance_xor => "balance-xor",
        LinuxBondMode::broadcast => "broadcast",
        LinuxBondMode::ieee802_3ad => "802.3ad",
        LinuxBondMode::balance_tlb => "balance-tlb",
        LinuxBondMode::balance_alb => "balance-alb",
    }
}

impl Interface {

    pub fn new(name: String) -> Self {
        Self {
            name,
            interface_type: NetworkInterfaceType::Unknown,
            autostart: false,
            active: false,
            method: None,
            method6: None,
            cidr: None,
            gateway: None,
            cidr6: None,
            gateway6: None,
            options: Vec::new(),
            options6: Vec::new(),
            comments: None,
            comments6: None,
            mtu: None,
            bridge_ports: None,
            bridge_vlan_aware: None,
            slaves: None,
            bond_mode: None,
        }
    }

    fn set_method_v4(&mut self, method: NetworkConfigMethod) -> Result<(), Error> {
        if self.method.is_none() {
            self.method = Some(method);
        } else {
            bail!("inet configuration method already set.");
        }
        Ok(())
    }

    fn set_method_v6(&mut self, method: NetworkConfigMethod) -> Result<(), Error> {
        if self.method6.is_none() {
            self.method6 = Some(method);
        } else {
            bail!("inet6 configuration method already set.");
        }
        Ok(())
    }

    fn set_cidr_v4(&mut self, address: String) -> Result<(), Error> {
        if self.cidr.is_none() {
            self.cidr = Some(address);
        } else {
            bail!("duplicate IPv4 address.");
        }
        Ok(())
    }

    fn set_gateway_v4(&mut self, gateway: String) -> Result<(), Error> {
        if self.gateway.is_none() {
            self.gateway = Some(gateway);
        } else {
            bail!("duplicate IPv4 gateway.");
        }
        Ok(())
    }

    fn set_cidr_v6(&mut self, address: String) -> Result<(), Error> {
        if self.cidr6.is_none() {
            self.cidr6 = Some(address);
        } else {
            bail!("duplicate IPv6 address.");
        }
        Ok(())
    }

    fn set_gateway_v6(&mut self, gateway: String) -> Result<(), Error> {
        if self.gateway6.is_none() {
            self.gateway6 = Some(gateway);
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
        self.slaves = Some(slaves);
        Ok(())
    }

    /// Write attributes not dependening on address family
    fn write_iface_attributes(&self, w: &mut dyn Write) -> Result<(), Error> {

        static EMPTY_LIST: Vec<String> = Vec::new();

        match self.interface_type {
            NetworkInterfaceType::Bridge => {
                if let Some(true) = self.bridge_vlan_aware {
                    writeln!(w, "\tbridge-vlan-aware yes")?;
                }
                let ports = self.bridge_ports.as_ref().unwrap_or(&EMPTY_LIST);
                if ports.is_empty() {
                    writeln!(w, "\tbridge-ports none")?;
                } else {
                    writeln!(w, "\tbridge-ports {}", ports.join(" "))?;
                }
            }
            NetworkInterfaceType::Bond => {
                let mode = self.bond_mode.unwrap_or(LinuxBondMode::balance_rr);
                writeln!(w, "\tbond-mode {}", bond_mode_to_str(mode))?;

                let slaves = self.slaves.as_ref().unwrap_or(&EMPTY_LIST);
                if slaves.is_empty() {
                    writeln!(w, "\tbond-slaves none")?;
                } else {
                    writeln!(w, "\tbond-slaves {}", slaves.join(" "))?;
                }
            }
            _ => {}
        }

        if let Some(mtu) = self.mtu {
            writeln!(w, "\tmtu {}", mtu)?;
        }

        Ok(())
    }

    /// Write attributes dependening on address family inet (IPv4)
    fn write_iface_attributes_v4(&self, w: &mut dyn Write, method: NetworkConfigMethod) -> Result<(), Error> {
        if method == NetworkConfigMethod::Static {
            if let Some(address) = &self.cidr {
                writeln!(w, "\taddress {}", address)?;
            }
            if let Some(gateway) = &self.gateway {
                writeln!(w, "\tgateway {}", gateway)?;
            }
        }

        for option in &self.options {
            writeln!(w, "\t{}", option)?;
        }

        if let Some(ref comments) = self.comments {
            for comment in comments.lines() {
                writeln!(w, "#{}", comment)?;
            }
        }

        Ok(())
    }

    /// Write attributes dependening on address family inet6 (IPv6)
    fn write_iface_attributes_v6(&self, w: &mut dyn Write, method: NetworkConfigMethod) -> Result<(), Error> {
        if method == NetworkConfigMethod::Static {
            if let Some(address) = &self.cidr6 {
                writeln!(w, "\taddress {}", address)?;
            }
            if let Some(gateway) = &self.gateway6 {
                writeln!(w, "\tgateway {}", gateway)?;
            }
        }

        for option in &self.options6 {
            writeln!(w, "\t{}", option)?;
        }

        if let Some(ref comments) = self.comments6 {
            for comment in comments.lines() {
                writeln!(w, "#{}", comment)?;
            }
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

        if self.method.is_none() && self.method6.is_none() { return Ok(()); }

        if self.autostart {
            writeln!(w, "auto {}", self.name)?;
        }

        if let Some(method) = self.method {
            writeln!(w, "iface {} inet {}", self.name, method_to_str(method))?;
            self.write_iface_attributes_v4(w, method)?;
            self.write_iface_attributes(w)?;
            writeln!(w)?;
        }

        if let Some(method6) = self.method6 {
            let mut skip_v6 = false; // avoid empty inet6 manual entry
            if self.method.is_some() && method6 == NetworkConfigMethod::Manual {
                if self.comments6.is_none() && self.options6.is_empty() { skip_v6 = true; }
            }

            if !skip_v6 {
                writeln!(w, "iface {} inet6 {}", self.name, method_to_str(method6))?;
                self.write_iface_attributes_v6(w, method6)?;
                if self.method.is_none() { // only write common attributes once
                    self.write_iface_attributes(w)?;
                }
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
    pub interfaces: BTreeMap<String, Interface>,
    order: Vec<NetworkOrderEntry>,
}

use std::convert::TryFrom;

impl TryFrom<NetworkConfig> for String  {

    type Error = Error;

    fn try_from(config: NetworkConfig) -> Result<Self, Self::Error> {
        let mut output = Vec::new();
        config.write_config(&mut output)?;
        let res = String::from_utf8(output)?;
        Ok(res)
    }
}

impl NetworkConfig {

    pub fn new() -> Self {
        Self {
            interfaces: BTreeMap::new(),
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

    /// Check if ports are used only once
    pub fn check_port_usage(&self) -> Result<(), Error> {
        let mut used_ports = HashMap::new();
        let mut check_port_usage = |iface, ports: &Vec<String>| {
            for port in ports.iter() {
                if let Some(prev_iface) = used_ports.get(port) {
                    bail!("iface '{}' port '{}' is already used on interface '{}'",
                          iface, port, prev_iface);
                }
                used_ports.insert(port.to_string(), iface);
            }
            Ok(())
        };

        for (iface, interface) in self.interfaces.iter() {
            if let Some(ports) = &interface.bridge_ports { check_port_usage(iface, ports)?; }
            if let Some(slaves) = &interface.slaves { check_port_usage(iface, slaves)?; }
        }
        Ok(())
    }

    pub fn write_config(&self, w: &mut dyn Write) -> Result<(), Error> {

        self.check_port_usage()?;

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

    let existing_interfaces = get_network_interfaces()?;
    let mut parser = NetworkParser::new(&content[..]);
    let data = parser.parse_interfaces(Some(&existing_interfaces))?;

    Ok((data, digest))
}

pub fn changes() -> Result<String, Error> {

    if !std::path::Path::new(NETWORK_INTERFACES_NEW_FILENAME).exists() {
        return Ok(String::new());
    }

    compute_file_diff(NETWORK_INTERFACES_FILENAME, NETWORK_INTERFACES_NEW_FILENAME)
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

#[cfg(test)]
mod test {

    use anyhow::{Error};

    use super::*;

    #[test]
    fn test_network_config_create_lo_1() -> Result<(), Error> {

        let input = "";

        let mut parser = NetworkParser::new(&input.as_bytes()[..]);

        let config = parser.parse_interfaces(None)?;

        let output = String::try_from(config)?;

        let expected = "auto lo\niface lo inet loopback\n\n";
        assert_eq!(output, expected);

        // run again using output as input
        let mut parser = NetworkParser::new(&output.as_bytes()[..]);

        let config = parser.parse_interfaces(None)?;

        let output = String::try_from(config)?;

        assert_eq!(output, expected);

        Ok(())
    }

    #[test]
    fn test_network_config_create_lo_2() -> Result<(), Error> {

        let input = "#c1\n\n#c2\n\niface test inet manual\n";

        let mut parser = NetworkParser::new(&input.as_bytes()[..]);

        let config = parser.parse_interfaces(None)?;

        let output = String::try_from(config)?;

        // Note: loopback should be added in front of other interfaces
        let expected = "#c1\n#c2\n\nauto lo\niface lo inet loopback\n\niface test inet manual\n\n";
        assert_eq!(output, expected);

        Ok(())
    }
}
