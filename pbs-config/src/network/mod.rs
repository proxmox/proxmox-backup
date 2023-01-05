use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::Write;

use anyhow::{bail, format_err, Error};
use lazy_static::lazy_static;
use regex::Regex;
use serde::de::{value, Deserialize, IntoDeserializer};

use proxmox_sys::{fs::replace_file, fs::CreateOptions};

mod helper;
pub use helper::*;

mod lexer;
pub use lexer::*;

mod parser;
pub use parser::*;

use pbs_api_types::{
    BondXmitHashPolicy, Interface, LinuxBondMode, NetworkConfigMethod, NetworkInterfaceType,
};

use crate::{open_backup_lockfile, BackupLockGuard};

lazy_static! {
    static ref PHYSICAL_NIC_REGEX: Regex = Regex::new(r"^(?:eth\d+|en[^:.]+|ib\d+)$").unwrap();
}

pub fn is_physical_nic(iface: &str) -> bool {
    PHYSICAL_NIC_REGEX.is_match(iface)
}

pub fn bond_mode_from_str(s: &str) -> Result<LinuxBondMode, Error> {
    LinuxBondMode::deserialize(s.into_deserializer())
        .map_err(|_: value::Error| format_err!("invalid bond_mode '{}'", s))
}

pub fn bond_xmit_hash_policy_from_str(s: &str) -> Result<BondXmitHashPolicy, Error> {
    BondXmitHashPolicy::deserialize(s.into_deserializer())
        .map_err(|_: value::Error| format_err!("invalid bond_xmit_hash_policy '{}'", s))
}

// Write attributes not depending on address family
fn write_iface_attributes(iface: &Interface, w: &mut dyn Write) -> Result<(), Error> {
    static EMPTY_LIST: Vec<String> = Vec::new();

    match iface.interface_type {
        NetworkInterfaceType::Bridge => {
            if let Some(true) = iface.bridge_vlan_aware {
                writeln!(w, "\tbridge-vlan-aware yes")?;
            }
            let ports = iface.bridge_ports.as_ref().unwrap_or(&EMPTY_LIST);
            if ports.is_empty() {
                writeln!(w, "\tbridge-ports none")?;
            } else {
                writeln!(w, "\tbridge-ports {}", ports.join(" "))?;
            }
        }
        NetworkInterfaceType::Bond => {
            let mode = iface.bond_mode.unwrap_or(LinuxBondMode::BalanceRr);
            writeln!(w, "\tbond-mode {mode}")?;
            if let Some(primary) = &iface.bond_primary {
                if mode == LinuxBondMode::ActiveBackup {
                    writeln!(w, "\tbond-primary {}", primary)?;
                }
            }

            if let Some(xmit_policy) = &iface.bond_xmit_hash_policy {
                if mode == LinuxBondMode::Ieee802_3ad || mode == LinuxBondMode::BalanceXor {
                    writeln!(w, "\tbond_xmit_hash_policy {xmit_policy}")?;
                }
            }

            let slaves = iface.slaves.as_ref().unwrap_or(&EMPTY_LIST);
            if slaves.is_empty() {
                writeln!(w, "\tbond-slaves none")?;
            } else {
                writeln!(w, "\tbond-slaves {}", slaves.join(" "))?;
            }
        }
        _ => {}
    }

    if let Some(mtu) = iface.mtu {
        writeln!(w, "\tmtu {}", mtu)?;
    }

    Ok(())
}

// Write attributes depending on address family inet (IPv4)
fn write_iface_attributes_v4(
    iface: &Interface,
    w: &mut dyn Write,
    method: NetworkConfigMethod,
) -> Result<(), Error> {
    if method == NetworkConfigMethod::Static {
        if let Some(address) = &iface.cidr {
            writeln!(w, "\taddress {}", address)?;
        }
        if let Some(gateway) = &iface.gateway {
            writeln!(w, "\tgateway {}", gateway)?;
        }
    }

    for option in &iface.options {
        writeln!(w, "\t{}", option)?;
    }

    if let Some(ref comments) = iface.comments {
        for comment in comments.lines() {
            writeln!(w, "#{}", comment)?;
        }
    }

    Ok(())
}

/// Write attributes depending on address family inet6 (IPv6)
fn write_iface_attributes_v6(
    iface: &Interface,
    w: &mut dyn Write,
    method: NetworkConfigMethod,
) -> Result<(), Error> {
    if method == NetworkConfigMethod::Static {
        if let Some(address) = &iface.cidr6 {
            writeln!(w, "\taddress {}", address)?;
        }
        if let Some(gateway) = &iface.gateway6 {
            writeln!(w, "\tgateway {}", gateway)?;
        }
    }

    for option in &iface.options6 {
        writeln!(w, "\t{}", option)?;
    }

    if let Some(ref comments) = iface.comments6 {
        for comment in comments.lines() {
            writeln!(w, "#{}", comment)?;
        }
    }

    Ok(())
}

fn write_iface(iface: &Interface, w: &mut dyn Write) -> Result<(), Error> {
    fn method_to_str(method: NetworkConfigMethod) -> &'static str {
        match method {
            NetworkConfigMethod::Static => "static",
            NetworkConfigMethod::Loopback => "loopback",
            NetworkConfigMethod::Manual => "manual",
            NetworkConfigMethod::DHCP => "dhcp",
        }
    }

    if iface.method.is_none() && iface.method6.is_none() {
        return Ok(());
    }

    if iface.autostart {
        writeln!(w, "auto {}", iface.name)?;
    }

    if let Some(method) = iface.method {
        writeln!(w, "iface {} inet {}", iface.name, method_to_str(method))?;
        write_iface_attributes_v4(iface, w, method)?;
        write_iface_attributes(iface, w)?;
        writeln!(w)?;
    }

    if let Some(method6) = iface.method6 {
        let mut skip_v6 = false; // avoid empty inet6 manual entry
        if iface.method.is_some()
            && method6 == NetworkConfigMethod::Manual
            && iface.comments6.is_none()
            && iface.options6.is_empty()
        {
            skip_v6 = true;
        }

        if !skip_v6 {
            writeln!(w, "iface {} inet6 {}", iface.name, method_to_str(method6))?;
            write_iface_attributes_v6(iface, w, method6)?;
            if iface.method.is_none() {
                // only write common attributes once
                write_iface_attributes(iface, w)?;
            }
            writeln!(w)?;
        }
    }

    Ok(())
}

#[derive(Debug)]
enum NetworkOrderEntry {
    Iface(String),
    Comment(String),
    Option(String),
}

#[derive(Debug, Default)]
pub struct NetworkConfig {
    pub interfaces: BTreeMap<String, Interface>,
    order: Vec<NetworkOrderEntry>,
}

impl TryFrom<NetworkConfig> for String {
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
        let interface = self
            .interfaces
            .get(name)
            .ok_or_else(|| format_err!("interface '{}' does not exist.", name))?;
        Ok(interface)
    }

    pub fn lookup_mut(&mut self, name: &str) -> Result<&mut Interface, Error> {
        let interface = self
            .interfaces
            .get_mut(name)
            .ok_or_else(|| format_err!("interface '{}' does not exist.", name))?;
        Ok(interface)
    }

    /// Check if ports are used only once
    pub fn check_port_usage(&self) -> Result<(), Error> {
        let mut used_ports = HashMap::new();
        let mut check_port_usage = |iface, ports: &Vec<String>| {
            for port in ports.iter() {
                if let Some(prev_iface) = used_ports.get(port) {
                    bail!(
                        "iface '{}' port '{}' is already used on interface '{}'",
                        iface,
                        port,
                        prev_iface
                    );
                }
                used_ports.insert(port.to_string(), iface);
            }
            Ok(())
        };

        for (iface, interface) in self.interfaces.iter() {
            if let Some(ports) = &interface.bridge_ports {
                check_port_usage(iface, ports)?;
            }
            if let Some(slaves) = &interface.slaves {
                check_port_usage(iface, slaves)?;
            }
        }
        Ok(())
    }

    /// Check if child mtu is less or equal than parent mtu
    pub fn check_mtu(&self, parent_name: &str, child_name: &str) -> Result<(), Error> {
        let parent = self
            .interfaces
            .get(parent_name)
            .ok_or_else(|| format_err!("check_mtu - missing parent interface '{}'", parent_name))?;
        let child = self
            .interfaces
            .get(child_name)
            .ok_or_else(|| format_err!("check_mtu - missing child interface '{}'", child_name))?;

        let child_mtu = match child.mtu {
            Some(mtu) => mtu,
            None => return Ok(()),
        };

        let parent_mtu = match parent.mtu {
            Some(mtu) => mtu,
            None => {
                if parent.interface_type == NetworkInterfaceType::Bond {
                    child_mtu
                } else {
                    1500
                }
            }
        };

        if parent_mtu < child_mtu {
            bail!(
                "interface '{}' - mtu {} is lower than '{}' - mtu {}\n",
                parent_name,
                parent_mtu,
                child_name,
                child_mtu
            );
        }

        Ok(())
    }

    /// Check if bond slaves exists
    pub fn check_bond_slaves(&self) -> Result<(), Error> {
        for (iface, interface) in self.interfaces.iter() {
            if let Some(slaves) = &interface.slaves {
                for slave in slaves.iter() {
                    match self.interfaces.get(slave) {
                        Some(entry) => {
                            if entry.interface_type != NetworkInterfaceType::Eth {
                                bail!(
                                    "bond '{}' - wrong interface type on slave '{}' ({:?} != {:?})",
                                    iface,
                                    slave,
                                    entry.interface_type,
                                    NetworkInterfaceType::Eth
                                );
                            }
                        }
                        None => {
                            bail!("bond '{}' - unable to find slave '{}'", iface, slave);
                        }
                    }
                    self.check_mtu(iface, slave)?;
                }
            }
        }
        Ok(())
    }

    /// Check if bridge ports exists
    pub fn check_bridge_ports(&self) -> Result<(), Error> {
        lazy_static! {
            static ref VLAN_INTERFACE_REGEX: Regex = Regex::new(r"^(\S+)\.(\d+)$").unwrap();
        }

        for (iface, interface) in self.interfaces.iter() {
            if let Some(ports) = &interface.bridge_ports {
                for port in ports.iter() {
                    let captures = VLAN_INTERFACE_REGEX.captures(port);
                    let port = if let Some(ref caps) = captures {
                        &caps[1]
                    } else {
                        port.as_str()
                    };
                    if !self.interfaces.contains_key(port) {
                        bail!("bridge '{}' - unable to find port '{}'", iface, port);
                    }
                    self.check_mtu(iface, port)?;
                }
            }
        }
        Ok(())
    }

    pub fn write_config(&self, w: &mut dyn Write) -> Result<(), Error> {
        self.check_port_usage()?;
        self.check_bond_slaves()?;
        self.check_bridge_ports()?;

        let mut done = HashSet::new();

        let mut last_entry_was_comment = false;

        for entry in self.order.iter() {
            match entry {
                NetworkOrderEntry::Comment(comment) => {
                    writeln!(w, "#{}", comment)?;
                    last_entry_was_comment = true;
                }
                NetworkOrderEntry::Option(option) => {
                    if last_entry_was_comment {
                        writeln!(w)?;
                    }
                    last_entry_was_comment = false;
                    writeln!(w, "{}", option)?;
                    writeln!(w)?;
                }
                NetworkOrderEntry::Iface(name) => {
                    let interface = match self.interfaces.get(name) {
                        Some(interface) => interface,
                        None => continue,
                    };

                    if last_entry_was_comment {
                        writeln!(w)?;
                    }
                    last_entry_was_comment = false;

                    if done.contains(name) {
                        continue;
                    }
                    done.insert(name);

                    write_iface(interface, w)?;
                }
            }
        }

        for (name, interface) in &self.interfaces {
            if done.contains(name) {
                continue;
            }
            write_iface(interface, w)?;
        }
        Ok(())
    }
}

pub const NETWORK_INTERFACES_FILENAME: &str = "/etc/network/interfaces";
pub const NETWORK_INTERFACES_NEW_FILENAME: &str = "/etc/network/interfaces.new";
pub const NETWORK_LOCKFILE: &str = "/var/lock/pve-network.lck";

pub fn lock_config() -> Result<BackupLockGuard, Error> {
    open_backup_lockfile(NETWORK_LOCKFILE, None, true)
}

pub fn config() -> Result<(NetworkConfig, [u8; 32]), Error> {
    let content =
        match proxmox_sys::fs::file_get_optional_contents(NETWORK_INTERFACES_NEW_FILENAME)? {
            Some(content) => content,
            None => {
                let content =
                    proxmox_sys::fs::file_get_optional_contents(NETWORK_INTERFACES_FILENAME)?;
                content.unwrap_or_default()
            }
        };

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

    replace_file(NETWORK_INTERFACES_NEW_FILENAME, &raw, options, true)?;

    Ok(())
}

// shell completion helper
pub fn complete_interface_name(_arg: &str, _param: &HashMap<String, String>) -> Vec<String> {
    match config() {
        Ok((data, _digest)) => data.interfaces.keys().map(|id| id.to_string()).collect(),
        Err(_) => Vec::new(),
    }
}

pub fn complete_port_list(arg: &str, _param: &HashMap<String, String>) -> Vec<String> {
    let mut ports = Vec::new();
    match config() {
        Ok((data, _digest)) => {
            for (iface, interface) in data.interfaces.iter() {
                if interface.interface_type == NetworkInterfaceType::Eth {
                    ports.push(iface.to_string());
                }
            }
        }
        Err(_) => return Vec::new(),
    };

    let arg = arg.trim();
    let prefix = if let Some(idx) = arg.rfind(',') {
        &arg[..idx + 1]
    } else {
        ""
    };
    ports
        .iter()
        .map(|port| format!("{}{}", prefix, port))
        .collect()
}

#[cfg(test)]
mod test {

    use anyhow::Error;

    use super::*;

    #[test]
    fn test_network_config_create_lo_1() -> Result<(), Error> {
        let input = "";

        let mut parser = NetworkParser::new(input.as_bytes());

        let config = parser.parse_interfaces(None)?;

        let output = String::try_from(config)?;

        let expected = "auto lo\niface lo inet loopback\n\n";
        assert_eq!(output, expected);

        // run again using output as input
        let mut parser = NetworkParser::new(output.as_bytes());

        let config = parser.parse_interfaces(None)?;

        let output = String::try_from(config)?;

        assert_eq!(output, expected);

        Ok(())
    }

    #[test]
    fn test_network_config_create_lo_2() -> Result<(), Error> {
        let input = "#c1\n\n#c2\n\niface test inet manual\n";

        let mut parser = NetworkParser::new(input.as_bytes());

        let config = parser.parse_interfaces(None)?;

        let output = String::try_from(config)?;

        // Note: loopback should be added in front of other interfaces
        let expected = "#c1\n#c2\n\nauto lo\niface lo inet loopback\n\niface test inet manual\n\n";
        assert_eq!(output, expected);

        Ok(())
    }

    #[test]
    fn test_network_config_parser_no_blank_1() -> Result<(), Error> {
        let input = "auto lo\n\
                     iface lo inet loopback\n\
                     iface lo inet6 loopback\n\
                     auto ens18\n\
                     iface ens18 inet static\n\
                     \taddress 192.168.20.144/20\n\
                     \tgateway 192.168.16.1\n\
                     # comment\n\
                     iface ens20 inet static\n\
                     \taddress 192.168.20.145/20\n\
                     iface ens21 inet manual\n\
                     iface ens22 inet manual\n";

        let mut parser = NetworkParser::new(input.as_bytes());

        let config = parser.parse_interfaces(None)?;

        let output = String::try_from(config)?;

        let expected = "auto lo\n\
                        iface lo inet loopback\n\
                        \n\
                        iface lo inet6 loopback\n\
                        \n\
                        auto ens18\n\
                        iface ens18 inet static\n\
                        \taddress 192.168.20.144/20\n\
                        \tgateway 192.168.16.1\n\
                        #comment\n\
                        \n\
                        iface ens20 inet static\n\
                        \taddress 192.168.20.145/20\n\
                        \n\
                        iface ens21 inet manual\n\
                        \n\
                        iface ens22 inet manual\n\
                        \n";
        assert_eq!(output, expected);

        Ok(())
    }

    #[test]
    fn test_network_config_parser_no_blank_2() -> Result<(), Error> {
        // Adapted from bug 2926
        let input = "### Hetzner Online GmbH installimage\n\
                     \n\
                     source /etc/network/interfaces.d/*\n\
                     \n\
                     auto lo\n\
                     iface lo inet loopback\n\
                     iface lo inet6 loopback\n\
                     \n\
                     auto enp4s0\n\
                     iface enp4s0 inet static\n\
                     \taddress 10.10.10.10/24\n\
                     \tgateway 10.10.10.1\n\
                     \t# route 10.10.20.10/24 via 10.10.20.1\n\
                     \tup route add -net 10.10.20.10 netmask 255.255.255.0 gw 10.10.20.1 dev enp4s0\n\
                     \n\
                     iface enp4s0 inet6 static\n\
                     \taddress fe80::5496:35ff:fe99:5a6a/64\n\
                     \tgateway fe80::1\n";

        let mut parser = NetworkParser::new(input.as_bytes());

        let config = parser.parse_interfaces(None)?;

        let output = String::try_from(config)?;

        let expected = "### Hetzner Online GmbH installimage\n\
                        \n\
                        source /etc/network/interfaces.d/*\n\
                        \n\
                        auto lo\n\
                        iface lo inet loopback\n\
                        \n\
                        iface lo inet6 loopback\n\
                        \n\
                        auto enp4s0\n\
                        iface enp4s0 inet static\n\
                        \taddress 10.10.10.10/24\n\
                        \tgateway 10.10.10.1\n\
                        \t# route 10.10.20.10/24 via 10.10.20.1\n\
                        \tup route add -net 10.10.20.10 netmask 255.255.255.0 gw 10.10.20.1 dev enp4s0\n\
                        \n\
                        iface enp4s0 inet6 static\n\
                        \taddress fe80::5496:35ff:fe99:5a6a/64\n\
                        \tgateway fe80::1\n\
                        \n";
        assert_eq!(output, expected);

        Ok(())
    }
}
