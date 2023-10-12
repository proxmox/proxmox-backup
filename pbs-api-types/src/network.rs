use std::fmt;

use serde::{Deserialize, Serialize};

use proxmox_schema::*;

use crate::{
    CIDR_FORMAT, CIDR_V4_FORMAT, CIDR_V6_FORMAT, IP_FORMAT, IP_V4_FORMAT, IP_V6_FORMAT,
    PROXMOX_SAFE_ID_REGEX,
};

pub const NETWORK_INTERFACE_FORMAT: ApiStringFormat =
    ApiStringFormat::Pattern(&PROXMOX_SAFE_ID_REGEX);

pub const IP_V4_SCHEMA: Schema = StringSchema::new("IPv4 address.")
    .format(&IP_V4_FORMAT)
    .max_length(15)
    .schema();

pub const IP_V6_SCHEMA: Schema = StringSchema::new("IPv6 address.")
    .format(&IP_V6_FORMAT)
    .max_length(39)
    .schema();

pub const IP_SCHEMA: Schema = StringSchema::new("IP (IPv4 or IPv6) address.")
    .format(&IP_FORMAT)
    .max_length(39)
    .schema();

pub const CIDR_V4_SCHEMA: Schema = StringSchema::new("IPv4 address with netmask (CIDR notation).")
    .format(&CIDR_V4_FORMAT)
    .max_length(18)
    .schema();

pub const CIDR_V6_SCHEMA: Schema = StringSchema::new("IPv6 address with netmask (CIDR notation).")
    .format(&CIDR_V6_FORMAT)
    .max_length(43)
    .schema();

pub const CIDR_SCHEMA: Schema =
    StringSchema::new("IP address (IPv4 or IPv6) with netmask (CIDR notation).")
        .format(&CIDR_FORMAT)
        .max_length(43)
        .schema();

#[api()]
#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
/// Interface configuration method
pub enum NetworkConfigMethod {
    /// Configuration is done manually using other tools
    Manual,
    /// Define interfaces with statically allocated addresses.
    Static,
    /// Obtain an address via DHCP
    DHCP,
    /// Define the loopback interface.
    Loopback,
}

#[api()]
#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
#[repr(u8)]
/// Linux Bond Mode
pub enum LinuxBondMode {
    /// Round-robin policy
    BalanceRr = 0,
    /// Active-backup policy
    ActiveBackup = 1,
    /// XOR policy
    BalanceXor = 2,
    /// Broadcast policy
    Broadcast = 3,
    /// IEEE 802.3ad Dynamic link aggregation
    #[serde(rename = "802.3ad")]
    Ieee802_3ad = 4,
    /// Adaptive transmit load balancing
    BalanceTlb = 5,
    /// Adaptive load balancing
    BalanceAlb = 6,
}

impl fmt::Display for LinuxBondMode {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str(match self {
            LinuxBondMode::BalanceRr => "balance-rr",
            LinuxBondMode::ActiveBackup => "active-backup",
            LinuxBondMode::BalanceXor => "balance-xor",
            LinuxBondMode::Broadcast => "broadcast",
            LinuxBondMode::Ieee802_3ad => "802.3ad",
            LinuxBondMode::BalanceTlb => "balance-tlb",
            LinuxBondMode::BalanceAlb => "balance-alb",
        })
    }
}

#[api()]
#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
#[repr(u8)]
/// Bond Transmit Hash Policy for LACP (802.3ad)
pub enum BondXmitHashPolicy {
    /// Layer 2
    Layer2 = 0,
    /// Layer 2+3
    #[serde(rename = "layer2+3")]
    Layer2_3 = 1,
    /// Layer 3+4
    #[serde(rename = "layer3+4")]
    Layer3_4 = 2,
}

impl fmt::Display for BondXmitHashPolicy {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str(match self {
            BondXmitHashPolicy::Layer2 => "layer2",
            BondXmitHashPolicy::Layer2_3 => "layer2+3",
            BondXmitHashPolicy::Layer3_4 => "layer3+4",
        })
    }
}

#[api()]
#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
/// Network interface type
pub enum NetworkInterfaceType {
    /// Loopback
    Loopback,
    /// Physical Ethernet device
    Eth,
    /// Linux Bridge
    Bridge,
    /// Linux Bond
    Bond,
    /// Linux VLAN (eth.10)
    Vlan,
    /// Interface Alias (eth:1)
    Alias,
    /// Unknown interface type
    Unknown,
}

pub const NETWORK_INTERFACE_NAME_SCHEMA: Schema = StringSchema::new("Network interface name.")
    .format(&NETWORK_INTERFACE_FORMAT)
    .min_length(1)
    .max_length(15) // libc::IFNAMSIZ-1
    .schema();

pub const NETWORK_INTERFACE_ARRAY_SCHEMA: Schema =
    ArraySchema::new("Network interface list.", &NETWORK_INTERFACE_NAME_SCHEMA).schema();

pub const NETWORK_INTERFACE_LIST_SCHEMA: Schema =
    StringSchema::new("A list of network devices, comma separated.")
        .format(&ApiStringFormat::PropertyString(
            &NETWORK_INTERFACE_ARRAY_SCHEMA,
        ))
        .schema();

#[api(
    properties: {
        name: {
            schema: NETWORK_INTERFACE_NAME_SCHEMA,
        },
        "type": {
            type: NetworkInterfaceType,
        },
        method: {
            type: NetworkConfigMethod,
            optional: true,
        },
        method6: {
            type: NetworkConfigMethod,
            optional: true,
        },
        cidr: {
            schema: CIDR_V4_SCHEMA,
            optional: true,
        },
        cidr6: {
            schema: CIDR_V6_SCHEMA,
            optional: true,
        },
        gateway: {
            schema: IP_V4_SCHEMA,
            optional: true,
        },
        gateway6: {
            schema: IP_V6_SCHEMA,
            optional: true,
        },
        options: {
            description: "Option list (inet)",
            type: Array,
            items: {
                description: "Optional attribute line.",
                type: String,
            },
        },
        options6: {
            description: "Option list (inet6)",
            type: Array,
            items: {
                description: "Optional attribute line.",
                type: String,
            },
        },
        comments: {
            description: "Comments (inet, may span multiple lines)",
            type: String,
            optional: true,
        },
        comments6: {
            description: "Comments (inet6, may span multiple lines)",
            type: String,
            optional: true,
        },
        bridge_ports: {
            schema: NETWORK_INTERFACE_ARRAY_SCHEMA,
            optional: true,
        },
        slaves: {
            schema: NETWORK_INTERFACE_ARRAY_SCHEMA,
            optional: true,
        },
        bond_mode: {
            type: LinuxBondMode,
            optional: true,
        },
        "bond-primary": {
            schema: NETWORK_INTERFACE_NAME_SCHEMA,
            optional: true,
        },
        bond_xmit_hash_policy: {
            type: BondXmitHashPolicy,
            optional: true,
        },
    }
)]
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
/// Network Interface configuration
pub struct Interface {
    /// Autostart interface
    #[serde(rename = "autostart")]
    pub autostart: bool,
    /// Interface is active (UP)
    pub active: bool,
    /// Interface name
    pub name: String,
    /// Interface type
    #[serde(rename = "type")]
    pub interface_type: NetworkInterfaceType,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub method: Option<NetworkConfigMethod>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub method6: Option<NetworkConfigMethod>,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// IPv4 address with netmask
    pub cidr: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// IPv4 gateway
    pub gateway: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// IPv6 address with netmask
    pub cidr6: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// IPv6 gateway
    pub gateway6: Option<String>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub options: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub options6: Vec<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub comments: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub comments6: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    /// Maximum Transmission Unit
    pub mtu: Option<u64>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub bridge_ports: Option<Vec<String>>,
    /// Enable bridge vlan support.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bridge_vlan_aware: Option<bool>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub slaves: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bond_mode: Option<LinuxBondMode>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "bond-primary")]
    pub bond_primary: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bond_xmit_hash_policy: Option<BondXmitHashPolicy>,
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
            bond_primary: None,
            bond_xmit_hash_policy: None,
        }
    }
}
