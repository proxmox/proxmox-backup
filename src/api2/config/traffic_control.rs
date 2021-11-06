use anyhow::{bail, Error};
use serde_json::Value;
use ::serde::{Deserialize, Serialize};

use proxmox_router::{ApiMethod, Router, RpcEnvironment, Permission};
use proxmox_schema::api;

use pbs_api_types::{
    TrafficControlRule, TrafficControlRuleUpdater,
    PROXMOX_CONFIG_DIGEST_SCHEMA, TRAFFIC_CONTROL_ID_SCHEMA,
    PRIV_SYS_AUDIT, PRIV_SYS_MODIFY,
};

#[api(
    input: {
        properties: {},
    },
    returns: {
        description: "The list of configured traffic control rules (with config digest).",
        type: Array,
        items: { type: TrafficControlRule },
    },
    access: {
        permission: &Permission::Privilege(&[], PRIV_SYS_MODIFY, false),
    },
)]
/// List traffic control rules
pub fn list_traffic_controls(
    _param: Value,
    _info: &ApiMethod,
    mut rpcenv: &mut dyn RpcEnvironment,
) -> Result<Vec<TrafficControlRule>, Error> {
    let (config, digest) = pbs_config::traffic_control::config()?;

    let list: Vec<TrafficControlRule> = config.convert_to_typed_array("rule")?;

    rpcenv["digest"] = proxmox::tools::digest_to_hex(&digest).into();

    Ok(list)
}

#[api(
    protected: true,
    input: {
        properties: {
             config: {
                type: TrafficControlRule,
                flatten: true,
            },
         },
    },
    access: {
        permission: &Permission::Privilege(&[], PRIV_SYS_MODIFY, false),
    },
)]
/// Create new traffic control rule.
pub fn create_traffic_control(config: TrafficControlRule) -> Result<(), Error> {

    let _lock = pbs_config::traffic_control::lock_config()?;

    let (mut section_config, _digest) = pbs_config::traffic_control::config()?;

    if section_config.sections.get(&config.name).is_some() {
        bail!("traffic control rule '{}' already exists.", config.name);
    }

    section_config.set_data(&config.name, "rule", &config)?;

    pbs_config::traffic_control::save_config(&section_config)?;

    Ok(())
}

#[api(
   input: {
        properties: {
            name: {
                schema: TRAFFIC_CONTROL_ID_SCHEMA,
            },
        },
    },
    returns: { type: TrafficControlRule },
    access: {
        permission: &Permission::Privilege(&[], PRIV_SYS_AUDIT, false),
    }
)]
/// Read traffic control configuration data.
pub fn read_traffic_control(
    name: String,
    _info: &ApiMethod,
    mut rpcenv: &mut dyn RpcEnvironment,
) -> Result<TrafficControlRule, Error> {
    let (config, digest) = pbs_config::traffic_control::config()?;
    let data: TrafficControlRule = config.lookup("rule", &name)?;
    rpcenv["digest"] = proxmox::tools::digest_to_hex(&digest).into();
    Ok(data)
}

#[api()]
#[derive(Serialize, Deserialize)]
#[allow(non_camel_case_types)]
/// Deletable property name
pub enum DeletableProperty {
    /// Delete the rate_in property.
    rate_in,
    /// Delete the burst_in property.
    burst_in,
    /// Delete the rate_out property.
    rate_out,
    /// Delete the burst_out property.
    burst_out,
    /// Delete the comment property.
    comment,
    /// Delete the timeframe property
    timeframe,
}

// fixme: use  TrafficControlUpdater
#[api(
    protected: true,
    input: {
        properties: {
            name: {
                schema: TRAFFIC_CONTROL_ID_SCHEMA,
            },
            update: {
                type: TrafficControlRuleUpdater,
                flatten: true,
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
/// Update traffic control configuration.
pub fn update_traffic_control(
    name: String,
    update: TrafficControlRuleUpdater,
    delete: Option<Vec<DeletableProperty>>,
    digest: Option<String>,
) -> Result<(), Error> {

    let _lock = pbs_config::traffic_control::lock_config()?;

    let (mut config, expected_digest) = pbs_config::traffic_control::config()?;

    if let Some(ref digest) = digest {
        let digest = proxmox::tools::hex_to_digest(digest)?;
        crate::tools::detect_modified_configuration_file(&digest, &expected_digest)?;
    }

    let mut data: TrafficControlRule = config.lookup("rule", &name)?;

    if let Some(delete) = delete {
        for delete_prop in delete {
            match delete_prop {
                DeletableProperty::rate_in => { data.rate_in = None; },
                DeletableProperty::rate_out => { data.rate_out = None; },
                DeletableProperty::burst_in => { data.burst_in = None; },
                DeletableProperty::burst_out => { data.burst_out = None; },
                DeletableProperty::comment => { data.comment = None; },
                DeletableProperty::timeframe => { data.timeframe = None; },
            }
        }
    }

    if let Some(comment) = update.comment {
        let comment = comment.trim().to_string();
        if comment.is_empty() {
            data.comment = None;
        } else {
            data.comment = Some(comment);
        }
    }

    if update.rate_in.is_some() { data.rate_in = update.rate_in; }
    if update.rate_out.is_some() { data.rate_out = update.rate_out; }

    if update.burst_in.is_some() { data.burst_in = update.burst_in; }
    if update.burst_out.is_some() { data.burst_out = update.burst_out; }
    
    if let Some(network) = update.network { data.network = network; }
    if update.timeframe.is_some() { data.timeframe = update.timeframe; }

    config.set_data(&name, "rule", &data)?;

    pbs_config::traffic_control::save_config(&config)?;

    Ok(())
}

#[api(
    protected: true,
    input: {
        properties: {
            name: {
                schema: TRAFFIC_CONTROL_ID_SCHEMA,
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
/// Remove a traffic control rule from the configuration file.
pub fn delete_traffic_control(name: String, digest: Option<String>) -> Result<(), Error> {

    let _lock = pbs_config::traffic_control::lock_config()?;

    let (mut config, expected_digest) = pbs_config::traffic_control::config()?;

    if let Some(ref digest) = digest {
        let digest = proxmox::tools::hex_to_digest(digest)?;
        crate::tools::detect_modified_configuration_file(&digest, &expected_digest)?;
    }

    match config.sections.get(&name) {
        Some(_) => { config.sections.remove(&name); },
        None => bail!("traffic control rule '{}' does not exist.", name),
    }

    pbs_config::traffic_control::save_config(&config)?;

    Ok(())
}


const ITEM_ROUTER: Router = Router::new()
    .get(&API_METHOD_READ_TRAFFIC_CONTROL)
    .put(&API_METHOD_UPDATE_TRAFFIC_CONTROL)
    .delete(&API_METHOD_DELETE_TRAFFIC_CONTROL);

pub const ROUTER: Router = Router::new()
    .get(&API_METHOD_LIST_TRAFFIC_CONTROLS)
    .post(&API_METHOD_CREATE_TRAFFIC_CONTROL)
    .match_all("name", &ITEM_ROUTER);
