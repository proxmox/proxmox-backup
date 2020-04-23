use anyhow::{Error};
use serde_json::{Value, to_value};
use ::serde::{Deserialize, Serialize};

use proxmox::api::{api, ApiMethod, Router, RpcEnvironment, Permission};

use crate::config::network;
use crate::config::acl::{PRIV_SYS_AUDIT, PRIV_SYS_MODIFY};
use crate::api2::types::*;

#[api(
    input: {
        properties: {},
    },
    returns: {
        description: "List network devices (with config digest).",
        type: Array,
        items: {
            type: Interface,
        },
    },
    access: {
        permission: &Permission::Privilege(&[], PRIV_SYS_AUDIT, false),
    },
)]
/// List all datastores
pub fn list_network_devices(
    _param: Value,
    _info: &ApiMethod,
    _rpcenv: &mut dyn RpcEnvironment,
) -> Result<Value, Error> {

    let (config, digest) = network::config()?;
    let digest = proxmox::tools::digest_to_hex(&digest);

    let mut list = Vec::new();

    for interface in config.interfaces.values() {
        let mut item: Value = to_value(interface)?;
        item["digest"] = digest.clone().into();
        list.push(item);
    }

    Ok(list.into())
}

#[api(
   input: {
        properties: {
            name: {
                schema: NETWORK_INTERFACE_NAME_SCHEMA,
            },
        },
    },
    returns: {
        description: "The network interface configuration (with config digest).",
        type: Interface,
    },
    access: {
        permission: &Permission::Privilege(&[], PRIV_SYS_AUDIT, false),
    },
)]
/// Read a network interface configuration.
pub fn read_interface(name: String) -> Result<Value, Error> {

    let (config, digest) = network::config()?;

    let interface = config.lookup(&name)?;

    let mut data: Value = to_value(interface)?;
    data["digest"] = proxmox::tools::digest_to_hex(&digest).into();

    Ok(data)
}

#[api()]
#[derive(Serialize, Deserialize)]
#[allow(non_camel_case_types)]
/// Deletable property name
pub enum DeletableProperty {
    /// Delete the IPv4 address property.
    address_v4,
    /// Delete the IPv6 address property.
    address_v6,
    /// Delete the IPv4 gateway property.
    gateway_v4,
    /// Delete the IPv6 gateway property.
    gateway_v6,
    /// Delete the whole IPv4 configuration entry.
    method_v4,
    /// Delete the whole IPv6 configuration entry.
    method_v6,
    /// Delete mtu.
    mtu,
    /// Delete auto flag
    auto,
}

#[api(
    protected: true,
    input: {
        properties: {
            name: {
                schema: NETWORK_INTERFACE_NAME_SCHEMA,
            },
            auto: {
                description: "Autostart interface.",
                type: bool,
                optional: true,
            },
            method_v4: {
                type: NetworkConfigMethod,
                optional: true,
            },
            method_v6: {
                type: NetworkConfigMethod,
                optional: true,
            },
            address: {
                schema: CIDR_SCHEMA,
                optional: true,
            },
            gateway: {
                schema: IP_SCHEMA,
                optional: true,
            },
            mtu: {
                description: "Maximum Transmission Unit.",
                optional: true,
                minimum: 46,
                maximum: 65535,
                default: 1500,
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
        permission: &Permission::Privilege(&[], PRIV_SYS_MODIFY, false),
    },
)]
/// Update network interface config.
pub fn update_interface(
    name: String,
    auto: Option<bool>,
    method_v4: Option<NetworkConfigMethod>,
    method_v6: Option<NetworkConfigMethod>,
    address: Option<String>,
    gateway: Option<String>,
    mtu: Option<u64>,
    delete: Option<Vec<DeletableProperty>>,
    digest: Option<String>,
) -> Result<(), Error> {

    let _lock = crate::tools::open_file_locked(network::NETWORK_LOCKFILE, std::time::Duration::new(10, 0))?;

    let (mut config, expected_digest) = network::config()?;

    if let Some(ref digest) = digest {
        let digest = proxmox::tools::hex_to_digest(digest)?;
        crate::tools::detect_modified_configuration_file(&digest, &expected_digest)?;
    }

    let interface = config.lookup_mut(&name)?;

    if let Some(delete) = delete {
        for name in delete {
            match name {
                DeletableProperty::address_v4 => { interface.cidr_v4 = None; },
                DeletableProperty::address_v6 => { interface.cidr_v6 = None; },
                DeletableProperty::gateway_v4 => { interface.gateway_v4 = None; },
                DeletableProperty::gateway_v6 => { interface.gateway_v6 = None; },
                DeletableProperty::method_v4 => { interface.method_v4 = None; },
                DeletableProperty::method_v6 => { interface.method_v6 = None; },
                DeletableProperty::mtu => { interface.mtu = None; },
                DeletableProperty::auto => { interface.auto = false; },
            }
        }
    }

    if let Some(auto) = auto { interface.auto = auto; }
    if method_v4.is_some() { interface.method_v4 = method_v4; }
    if method_v6.is_some() { interface.method_v6 = method_v6; }
    if mtu.is_some() { interface.mtu = mtu; }

    if let Some(address) = address {
        let (_, _, is_v6) = network::parse_cidr(&address)?;
        if is_v6 {
            interface.cidr_v6 = Some(address);
        } else {
            interface.cidr_v4 = Some(address);
        }
    }

    if let Some(gateway) = gateway {
        let is_v6 = gateway.contains(':');
        if is_v6 {
            interface.gateway_v6 = Some(gateway);
        } else {
            interface.gateway_v4 = Some(gateway);
        }
    }

    network::save_config(&config)?;

    Ok(())
}

#[api(
    protected: true,
    input: {
        properties: {
            name: {
                schema: NETWORK_INTERFACE_NAME_SCHEMA,
            },
            digest: {
                optional: true,
                schema: PROXMOX_CONFIG_DIGEST_SCHEMA,
            },
        },
    },
    access: {
        permission: &Permission::Privilege(&[], PRIV_SYS_MODIFY, false),
    },
)]
/// Remove network interface configuration.
pub fn delete_interface(name: String, digest: Option<String>) -> Result<(), Error> {

    let _lock = crate::tools::open_file_locked(network::NETWORK_LOCKFILE, std::time::Duration::new(10, 0))?;

    let (mut config, expected_digest) = network::config()?;

    if let Some(ref digest) = digest {
        let digest = proxmox::tools::hex_to_digest(digest)?;
        crate::tools::detect_modified_configuration_file(&digest, &expected_digest)?;
    }

    let _interface = config.lookup(&name)?; // check if interface exists

    config.interfaces.remove(&name);

    network::save_config(&config)?;

    Ok(())
}

const ITEM_ROUTER: Router = Router::new()
    .get(&API_METHOD_READ_INTERFACE)
    .put(&API_METHOD_UPDATE_INTERFACE)
    .delete(&API_METHOD_DELETE_INTERFACE);

pub const ROUTER: Router = Router::new()
    .get(&API_METHOD_LIST_NETWORK_DEVICES)
    .match_all("name", &ITEM_ROUTER);
