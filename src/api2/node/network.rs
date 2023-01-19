use anyhow::{bail, Error};
use hex::FromHex;
use serde::{Deserialize, Serialize};
use serde_json::{to_value, Value};

use proxmox_router::{ApiMethod, Permission, Router, RpcEnvironment};
use proxmox_schema::api;

use pbs_api_types::{
    Authid, BondXmitHashPolicy, Interface, LinuxBondMode, NetworkConfigMethod,
    NetworkInterfaceType, CIDR_V4_SCHEMA, CIDR_V6_SCHEMA, IP_V4_SCHEMA, IP_V6_SCHEMA,
    NETWORK_INTERFACE_ARRAY_SCHEMA, NETWORK_INTERFACE_LIST_SCHEMA, NETWORK_INTERFACE_NAME_SCHEMA,
    NODE_SCHEMA, PRIV_SYS_AUDIT, PRIV_SYS_MODIFY, PROXMOX_CONFIG_DIGEST_SCHEMA,
};
use pbs_config::network::{self, NetworkConfig};

use proxmox_rest_server::WorkerTask;

fn split_interface_list(list: &str) -> Result<Vec<String>, Error> {
    let value = NETWORK_INTERFACE_ARRAY_SCHEMA.parse_property_string(list)?;
    Ok(value
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect())
}

fn check_duplicate_gateway_v4(config: &NetworkConfig, iface: &str) -> Result<(), Error> {
    let current_gateway_v4 = config
        .interfaces
        .iter()
        .find(|(_, interface)| interface.gateway.is_some())
        .map(|(name, _)| name.to_string());

    if let Some(current_gateway_v4) = current_gateway_v4 {
        if current_gateway_v4 != iface {
            bail!(
                "Default IPv4 gateway already exists on interface '{}'",
                current_gateway_v4
            );
        }
    }
    Ok(())
}

fn check_duplicate_gateway_v6(config: &NetworkConfig, iface: &str) -> Result<(), Error> {
    let current_gateway_v6 = config
        .interfaces
        .iter()
        .find(|(_, interface)| interface.gateway6.is_some())
        .map(|(name, _)| name.to_string());

    if let Some(current_gateway_v6) = current_gateway_v6 {
        if current_gateway_v6 != iface {
            bail!(
                "Default IPv6 gateway already exists on interface '{}'",
                current_gateway_v6
            );
        }
    }
    Ok(())
}

fn set_bridge_ports(iface: &mut Interface, ports: Vec<String>) -> Result<(), Error> {
    if iface.interface_type != NetworkInterfaceType::Bridge {
        bail!(
            "interface '{}' is no bridge (type is {:?})",
            iface.name,
            iface.interface_type
        );
    }
    iface.bridge_ports = Some(ports);
    Ok(())
}

fn set_bond_slaves(iface: &mut Interface, slaves: Vec<String>) -> Result<(), Error> {
    if iface.interface_type != NetworkInterfaceType::Bond {
        bail!(
            "interface '{}' is no bond (type is {:?})",
            iface.name,
            iface.interface_type
        );
    }
    iface.slaves = Some(slaves);
    Ok(())
}

#[api(
    input: {
        properties: {
            node: {
                schema: NODE_SCHEMA,
            },
        },
    },
    returns: {
        description: "List network devices (with config digest).",
        type: Array,
        items: {
            type: Interface,
        },
    },
    access: {
        permission: &Permission::Privilege(&["system", "network", "interfaces"], PRIV_SYS_AUDIT, false),
    },
)]
/// List all datastores
pub fn list_network_devices(
    _param: Value,
    _info: &ApiMethod,
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<Value, Error> {
    let (config, digest) = network::config()?;
    let digest = hex::encode(digest);

    let mut list = Vec::new();

    for (iface, interface) in config.interfaces.iter() {
        if iface == "lo" {
            continue;
        } // do not list lo
        let mut item: Value = to_value(interface)?;
        item["digest"] = digest.clone().into();
        item["iface"] = iface.to_string().into();
        list.push(item);
    }

    let diff = network::changes()?;
    if !diff.is_empty() {
        rpcenv["changes"] = diff.into();
    }

    Ok(list.into())
}

#[api(
    input: {
        properties: {
            node: {
                schema: NODE_SCHEMA,
            },
            iface: {
                schema: NETWORK_INTERFACE_NAME_SCHEMA,
            },
        },
    },
    returns: { type: Interface },
    access: {
        permission: &Permission::Privilege(&["system", "network", "interfaces", "{name}"], PRIV_SYS_AUDIT, false),
    },
)]
/// Read a network interface configuration.
pub fn read_interface(iface: String) -> Result<Value, Error> {
    let (config, digest) = network::config()?;

    let interface = config.lookup(&iface)?;

    let mut data: Value = to_value(interface)?;
    data["digest"] = hex::encode(digest).into();

    Ok(data)
}

#[api(
    protected: true,
    input: {
        properties: {
            node: {
                schema: NODE_SCHEMA,
            },
            iface: {
                schema: NETWORK_INTERFACE_NAME_SCHEMA,
            },
            "type": {
                type: NetworkInterfaceType,
                optional: true,
            },
            autostart: {
                description: "Autostart interface.",
                type: bool,
                optional: true,
            },
            method: {
                type: NetworkConfigMethod,
                optional: true,
            },
            method6: {
                type: NetworkConfigMethod,
                optional: true,
            },
            comments: {
                description: "Comments (inet, may span multiple lines)",
                type: String,
                optional: true,
            },
            comments6: {
                description: "Comments (inet5, may span multiple lines)",
                type: String,
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
            mtu: {
                description: "Maximum Transmission Unit.",
                optional: true,
                minimum: 46,
                maximum: 65535,
                default: 1500,
            },
            bridge_ports: {
                schema: NETWORK_INTERFACE_LIST_SCHEMA,
                optional: true,
            },
            bridge_vlan_aware: {
	        description: "Enable bridge vlan support.",
	        type: bool,
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
            slaves: {
                schema: NETWORK_INTERFACE_LIST_SCHEMA,
                optional: true,
            },
        },
    },
    access: {
        permission: &Permission::Privilege(&["system", "network", "interfaces", "{iface}"], PRIV_SYS_MODIFY, false),
    },
)]
/// Create network interface configuration.
#[allow(clippy::too_many_arguments)]
pub fn create_interface(
    iface: String,
    autostart: Option<bool>,
    method: Option<NetworkConfigMethod>,
    method6: Option<NetworkConfigMethod>,
    comments: Option<String>,
    comments6: Option<String>,
    cidr: Option<String>,
    gateway: Option<String>,
    cidr6: Option<String>,
    gateway6: Option<String>,
    mtu: Option<u64>,
    bridge_ports: Option<String>,
    bridge_vlan_aware: Option<bool>,
    bond_mode: Option<LinuxBondMode>,
    bond_primary: Option<String>,
    bond_xmit_hash_policy: Option<BondXmitHashPolicy>,
    slaves: Option<String>,
    param: Value,
) -> Result<(), Error> {
    let interface_type = pbs_tools::json::required_string_param(&param, "type")?;
    let interface_type: NetworkInterfaceType = serde_json::from_value(interface_type.into())?;

    let _lock = network::lock_config()?;

    let (mut config, _digest) = network::config()?;

    if config.interfaces.contains_key(&iface) {
        bail!("interface '{}' already exists", iface);
    }

    let mut interface = Interface::new(iface.clone());
    interface.interface_type = interface_type;

    if let Some(autostart) = autostart {
        interface.autostart = autostart;
    }
    if method.is_some() {
        interface.method = method;
    }
    if method6.is_some() {
        interface.method6 = method6;
    }
    if mtu.is_some() {
        interface.mtu = mtu;
    }
    if comments.is_some() {
        interface.comments = comments;
    }
    if comments6.is_some() {
        interface.comments6 = comments6;
    }

    if let Some(cidr) = cidr {
        let (_, _, is_v6) = network::parse_cidr(&cidr)?;
        if is_v6 {
            bail!("invalid address type (expected IPv4, got IPv6)");
        }
        interface.cidr = Some(cidr);
    }

    if let Some(cidr6) = cidr6 {
        let (_, _, is_v6) = network::parse_cidr(&cidr6)?;
        if !is_v6 {
            bail!("invalid address type (expected IPv6, got IPv4)");
        }
        interface.cidr6 = Some(cidr6);
    }

    if let Some(gateway) = gateway {
        let is_v6 = gateway.contains(':');
        if is_v6 {
            bail!("invalid address type (expected IPv4, got IPv6)");
        }
        check_duplicate_gateway_v4(&config, &iface)?;
        interface.gateway = Some(gateway);
    }

    if let Some(gateway6) = gateway6 {
        let is_v6 = gateway6.contains(':');
        if !is_v6 {
            bail!("invalid address type (expected IPv6, got IPv4)");
        }
        check_duplicate_gateway_v6(&config, &iface)?;
        interface.gateway6 = Some(gateway6);
    }

    match interface_type {
        NetworkInterfaceType::Bridge => {
            if let Some(ports) = bridge_ports {
                let ports = split_interface_list(&ports)?;
                set_bridge_ports(&mut interface, ports)?;
            }
            if bridge_vlan_aware.is_some() {
                interface.bridge_vlan_aware = bridge_vlan_aware;
            }
        }
        NetworkInterfaceType::Bond => {
            if let Some(mode) = bond_mode {
                interface.bond_mode = bond_mode;
                if bond_primary.is_some() {
                    if mode != LinuxBondMode::ActiveBackup {
                        bail!("bond-primary is only valid with Active/Backup mode");
                    }
                    interface.bond_primary = bond_primary;
                }
                if bond_xmit_hash_policy.is_some() {
                    if mode != LinuxBondMode::Ieee802_3ad && mode != LinuxBondMode::BalanceXor {
                        bail!("bond_xmit_hash_policy is only valid with LACP(802.3ad) or balance-xor mode");
                    }
                    interface.bond_xmit_hash_policy = bond_xmit_hash_policy;
                }
            }
            if let Some(slaves) = slaves {
                let slaves = split_interface_list(&slaves)?;
                set_bond_slaves(&mut interface, slaves)?;
            }
        }
        _ => bail!(
            "creating network interface type '{:?}' is not supported",
            interface_type
        ),
    }

    if interface.cidr.is_some() || interface.gateway.is_some() {
        interface.method = Some(NetworkConfigMethod::Static);
    } else if interface.method.is_none() {
        interface.method = Some(NetworkConfigMethod::Manual);
    }

    if interface.cidr6.is_some() || interface.gateway6.is_some() {
        interface.method6 = Some(NetworkConfigMethod::Static);
    } else if interface.method6.is_none() {
        interface.method6 = Some(NetworkConfigMethod::Manual);
    }

    config.interfaces.insert(iface, interface);

    network::save_config(&config)?;

    Ok(())
}

#[api()]
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
/// Deletable property name
pub enum DeletableProperty {
    /// Delete the IPv4 address property.
    Cidr,
    /// Delete the IPv6 address property.
    Cidr6,
    /// Delete the IPv4 gateway property.
    Gateway,
    /// Delete the IPv6 gateway property.
    Gateway6,
    /// Delete the whole IPv4 configuration entry.
    Method,
    /// Delete the whole IPv6 configuration entry.
    Method6,
    /// Delete IPv4 comments
    Comments,
    /// Delete IPv6 comments
    Comments6,
    /// Delete mtu.
    Mtu,
    /// Delete autostart flag
    Autostart,
    /// Delete bridge ports (set to 'none')
    #[serde(rename = "bridge_ports")]
    BridgePorts,
    /// Delete bridge-vlan-aware flag
    #[serde(rename = "bridge_vlan_aware")]
    BridgeVlanAware,
    /// Delete bond-slaves (set to 'none')
    Slaves,
    /// Delete bond-primary
    BondPrimary,
    /// Delete bond transmit hash policy
    #[serde(rename = "bond_xmit_hash_policy")]
    BondXmitHashPolicy,
}

#[api(
    protected: true,
    input: {
        properties: {
            node: {
                schema: NODE_SCHEMA,
            },
            iface: {
                schema: NETWORK_INTERFACE_NAME_SCHEMA,
            },
            "type": {
                type: NetworkInterfaceType,
                optional: true,
            },
            autostart: {
                description: "Autostart interface.",
                type: bool,
                optional: true,
            },
            method: {
                type: NetworkConfigMethod,
                optional: true,
            },
            method6: {
                type: NetworkConfigMethod,
                optional: true,
            },
            comments: {
                description: "Comments (inet, may span multiple lines)",
                type: String,
                optional: true,
            },
            comments6: {
                description: "Comments (inet5, may span multiple lines)",
                type: String,
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
            mtu: {
                description: "Maximum Transmission Unit.",
                optional: true,
                minimum: 46,
                maximum: 65535,
                default: 1500,
            },
            bridge_ports: {
                schema: NETWORK_INTERFACE_LIST_SCHEMA,
                optional: true,
            },
            bridge_vlan_aware: {
	        description: "Enable bridge vlan support.",
	        type: bool,
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
            slaves: {
                schema: NETWORK_INTERFACE_LIST_SCHEMA,
                optional: true,
            },
            delete: {
                description: "List of properties to delete.",
                type: Array,
                optional: true,
                items: {
                    type: DeletableProperty,
                }
            },
            digest: {
                optional: true,
                schema: PROXMOX_CONFIG_DIGEST_SCHEMA,
            },
        },
    },
    access: {
        permission: &Permission::Privilege(&["system", "network", "interfaces", "{iface}"], PRIV_SYS_MODIFY, false),
    },
)]
/// Update network interface config.
#[allow(clippy::too_many_arguments)]
pub fn update_interface(
    iface: String,
    autostart: Option<bool>,
    method: Option<NetworkConfigMethod>,
    method6: Option<NetworkConfigMethod>,
    comments: Option<String>,
    comments6: Option<String>,
    cidr: Option<String>,
    gateway: Option<String>,
    cidr6: Option<String>,
    gateway6: Option<String>,
    mtu: Option<u64>,
    bridge_ports: Option<String>,
    bridge_vlan_aware: Option<bool>,
    bond_mode: Option<LinuxBondMode>,
    bond_primary: Option<String>,
    bond_xmit_hash_policy: Option<BondXmitHashPolicy>,
    slaves: Option<String>,
    delete: Option<Vec<DeletableProperty>>,
    digest: Option<String>,
    param: Value,
) -> Result<(), Error> {
    let _lock = network::lock_config()?;

    let (mut config, expected_digest) = network::config()?;

    if let Some(ref digest) = digest {
        let digest = <[u8; 32]>::from_hex(digest)?;
        crate::tools::detect_modified_configuration_file(&digest, &expected_digest)?;
    }

    if gateway.is_some() {
        check_duplicate_gateway_v4(&config, &iface)?;
    }
    if gateway6.is_some() {
        check_duplicate_gateway_v6(&config, &iface)?;
    }

    let interface = config.lookup_mut(&iface)?;

    if let Some(interface_type) = param.get("type") {
        let interface_type = NetworkInterfaceType::deserialize(interface_type)?;
        if interface_type != interface.interface_type {
            bail!(
                "got unexpected interface type ({:?} != {:?})",
                interface_type,
                interface.interface_type
            );
        }
    }

    if let Some(delete) = delete {
        for delete_prop in delete {
            match delete_prop {
                DeletableProperty::Cidr => {
                    interface.cidr = None;
                }
                DeletableProperty::Cidr6 => {
                    interface.cidr6 = None;
                }
                DeletableProperty::Gateway => {
                    interface.gateway = None;
                }
                DeletableProperty::Gateway6 => {
                    interface.gateway6 = None;
                }
                DeletableProperty::Method => {
                    interface.method = None;
                }
                DeletableProperty::Method6 => {
                    interface.method6 = None;
                }
                DeletableProperty::Comments => {
                    interface.comments = None;
                }
                DeletableProperty::Comments6 => {
                    interface.comments6 = None;
                }
                DeletableProperty::Mtu => {
                    interface.mtu = None;
                }
                DeletableProperty::Autostart => {
                    interface.autostart = false;
                }
                DeletableProperty::BridgePorts => {
                    set_bridge_ports(interface, Vec::new())?;
                }
                DeletableProperty::BridgeVlanAware => {
                    interface.bridge_vlan_aware = None;
                }
                DeletableProperty::Slaves => {
                    set_bond_slaves(interface, Vec::new())?;
                }
                DeletableProperty::BondPrimary => {
                    interface.bond_primary = None;
                }
                DeletableProperty::BondXmitHashPolicy => interface.bond_xmit_hash_policy = None,
            }
        }
    }

    if let Some(autostart) = autostart {
        interface.autostart = autostart;
    }
    if method.is_some() {
        interface.method = method;
    }
    if method6.is_some() {
        interface.method6 = method6;
    }
    if mtu.is_some() {
        interface.mtu = mtu;
    }
    if let Some(ports) = bridge_ports {
        let ports = split_interface_list(&ports)?;
        set_bridge_ports(interface, ports)?;
    }
    if bridge_vlan_aware.is_some() {
        interface.bridge_vlan_aware = bridge_vlan_aware;
    }
    if let Some(slaves) = slaves {
        let slaves = split_interface_list(&slaves)?;
        set_bond_slaves(interface, slaves)?;
    }
    if let Some(mode) = bond_mode {
        interface.bond_mode = bond_mode;
        if bond_primary.is_some() {
            if mode != LinuxBondMode::ActiveBackup {
                bail!("bond-primary is only valid with Active/Backup mode");
            }
            interface.bond_primary = bond_primary;
        }
        if bond_xmit_hash_policy.is_some() {
            if mode != LinuxBondMode::Ieee802_3ad && mode != LinuxBondMode::BalanceXor {
                bail!("bond_xmit_hash_policy is only valid with LACP(802.3ad) or balance-xor mode");
            }
            interface.bond_xmit_hash_policy = bond_xmit_hash_policy;
        }
    }

    if let Some(cidr) = cidr {
        let (_, _, is_v6) = network::parse_cidr(&cidr)?;
        if is_v6 {
            bail!("invalid address type (expected IPv4, got IPv6)");
        }
        interface.cidr = Some(cidr);
    }

    if let Some(cidr6) = cidr6 {
        let (_, _, is_v6) = network::parse_cidr(&cidr6)?;
        if !is_v6 {
            bail!("invalid address type (expected IPv6, got IPv4)");
        }
        interface.cidr6 = Some(cidr6);
    }

    if let Some(gateway) = gateway {
        let is_v6 = gateway.contains(':');
        if is_v6 {
            bail!("invalid address type (expected IPv4, got IPv6)");
        }
        interface.gateway = Some(gateway);
    }

    if let Some(gateway6) = gateway6 {
        let is_v6 = gateway6.contains(':');
        if !is_v6 {
            bail!("invalid address type (expected IPv6, got IPv4)");
        }
        interface.gateway6 = Some(gateway6);
    }

    if comments.is_some() {
        interface.comments = comments;
    }
    if comments6.is_some() {
        interface.comments6 = comments6;
    }

    if interface.cidr.is_some() || interface.gateway.is_some() {
        interface.method = Some(NetworkConfigMethod::Static);
    } else {
        interface.method = Some(NetworkConfigMethod::Manual);
    }

    if interface.cidr6.is_some() || interface.gateway6.is_some() {
        interface.method6 = Some(NetworkConfigMethod::Static);
    } else {
        interface.method6 = Some(NetworkConfigMethod::Manual);
    }

    network::save_config(&config)?;

    Ok(())
}

#[api(
    protected: true,
    input: {
        properties: {
            node: {
                schema: NODE_SCHEMA,
            },
            iface: {
                schema: NETWORK_INTERFACE_NAME_SCHEMA,
            },
            digest: {
                optional: true,
                schema: PROXMOX_CONFIG_DIGEST_SCHEMA,
            },
        },
    },
    access: {
        permission: &Permission::Privilege(&["system", "network", "interfaces", "{iface}"], PRIV_SYS_MODIFY, false),
    },
)]
/// Remove network interface configuration.
pub fn delete_interface(iface: String, digest: Option<String>) -> Result<(), Error> {
    let _lock = network::lock_config()?;

    let (mut config, expected_digest) = network::config()?;

    if let Some(ref digest) = digest {
        let digest = <[u8; 32]>::from_hex(digest)?;
        crate::tools::detect_modified_configuration_file(&digest, &expected_digest)?;
    }

    let _interface = config.lookup(&iface)?; // check if interface exists

    config.interfaces.remove(&iface);

    network::save_config(&config)?;

    Ok(())
}

#[api(
    protected: true,
    input: {
        properties: {
            node: {
                schema: NODE_SCHEMA,
            },
        },
    },
    access: {
        permission: &Permission::Privilege(&["system", "network", "interfaces"], PRIV_SYS_MODIFY, false),
    },
)]
/// Reload network configuration (requires ifupdown2).
pub async fn reload_network_config(rpcenv: &mut dyn RpcEnvironment) -> Result<String, Error> {
    network::assert_ifupdown2_installed()?;

    let auth_id: Authid = rpcenv.get_auth_id().unwrap().parse()?;

    let upid_str = WorkerTask::spawn(
        "srvreload",
        Some(String::from("networking")),
        auth_id.to_string(),
        true,
        |_worker| async {
            let _ = std::fs::rename(
                network::NETWORK_INTERFACES_NEW_FILENAME,
                network::NETWORK_INTERFACES_FILENAME,
            );

            network::network_reload()?;
            Ok(())
        },
    )?;

    Ok(upid_str)
}

#[api(
    protected: true,
    input: {
        properties: {
            node: {
                schema: NODE_SCHEMA,
            },
        },
    },
    access: {
        permission: &Permission::Privilege(&["system", "network", "interfaces"], PRIV_SYS_MODIFY, false),
    },
)]
/// Revert network configuration (rm /etc/network/interfaces.new).
pub fn revert_network_config() -> Result<(), Error> {
    let _ = std::fs::remove_file(network::NETWORK_INTERFACES_NEW_FILENAME);

    Ok(())
}

const ITEM_ROUTER: Router = Router::new()
    .get(&API_METHOD_READ_INTERFACE)
    .put(&API_METHOD_UPDATE_INTERFACE)
    .delete(&API_METHOD_DELETE_INTERFACE);

pub const ROUTER: Router = Router::new()
    .get(&API_METHOD_LIST_NETWORK_DEVICES)
    .put(&API_METHOD_RELOAD_NETWORK_CONFIG)
    .post(&API_METHOD_CREATE_INTERFACE)
    .delete(&API_METHOD_REVERT_NETWORK_CONFIG)
    .match_all("iface", &ITEM_ROUTER);
