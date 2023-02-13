use anyhow::Error;
use pbs_client::view_task_result;
use pbs_tools::json::required_string_param;
use serde_json::Value;

use proxmox_router::{cli::*, ApiHandler, Permission, RpcEnvironment};
use proxmox_schema::api;

use pbs_api_types::{Realm, PRIV_PERMISSIONS_MODIFY, REALM_ID_SCHEMA, REMOVE_VANISHED_SCHEMA};

use proxmox_backup::{api2, client_helpers::connect_to_localhost};

#[api(
    input: {
        properties: {
            "output-format": {
                schema: OUTPUT_FORMAT,
                optional: true,
            },
        }
    }
)]
/// List configured LDAP realms
fn list_ldap_realms(param: Value, rpcenv: &mut dyn RpcEnvironment) -> Result<Value, Error> {
    let output_format = get_output_format(&param);

    let info = &api2::config::access::ldap::API_METHOD_LIST_LDAP_REALMS;
    let mut data = match info.handler {
        ApiHandler::Sync(handler) => (handler)(param, info, rpcenv)?,
        _ => unreachable!(),
    };

    let options = default_table_format_options()
        .column(ColumnConfig::new("realm"))
        .column(ColumnConfig::new("server1"))
        .column(ColumnConfig::new("comment"));

    format_and_print_result_full(&mut data, &info.returns, &output_format, &options);

    Ok(Value::Null)
}
#[api(
    input: {
        properties: {
            realm: {
                schema: REALM_ID_SCHEMA,
            },
            "output-format": {
                schema: OUTPUT_FORMAT,
                optional: true,
            },
        }
    }
)]

/// Show LDAP realm configuration
fn show_ldap_realm(param: Value, rpcenv: &mut dyn RpcEnvironment) -> Result<Value, Error> {
    let output_format = get_output_format(&param);

    let info = &api2::config::access::ldap::API_METHOD_READ_LDAP_REALM;
    let mut data = match info.handler {
        ApiHandler::Sync(handler) => (handler)(param, info, rpcenv)?,
        _ => unreachable!(),
    };

    let options = default_table_format_options();
    format_and_print_result_full(&mut data, &info.returns, &output_format, &options);

    Ok(Value::Null)
}

#[api(
    protected: true,
    input: {
        properties: {
            realm: {
                type: Realm,
            },
            "dry-run": {
                type: bool,
                description: "If set, do not create/delete anything",
                default: false,
                optional: true,
            },
            "remove-vanished": {
                optional: true,
                schema: REMOVE_VANISHED_SCHEMA,
            },
            "enable-new": {
                description: "Enable newly synced users immediately",
                optional: true,
                type: bool,
            }
         },
    },
    access: {
        permission: &Permission::Privilege(&["access", "users"], PRIV_PERMISSIONS_MODIFY, false),
    },
)]
/// Sync a given LDAP realm
async fn sync_ldap_realm(param: Value) -> Result<Value, Error> {
    let realm = required_string_param(&param, "realm")?;
    let client = connect_to_localhost()?;

    let path = format!("api2/json/access/domains/{}/sync", realm);
    let result = client.post(&path, Some(param)).await?;
    view_task_result(&client, result, "text").await?;

    Ok(Value::Null)
}

pub fn ldap_commands() -> CommandLineInterface {
    let cmd_def = CliCommandMap::new()
        .insert("list", CliCommand::new(&API_METHOD_LIST_LDAP_REALMS))
        .insert(
            "show",
            CliCommand::new(&API_METHOD_SHOW_LDAP_REALM)
                .arg_param(&["realm"])
                .completion_cb("realm", pbs_config::domains::complete_ldap_realm_name),
        )
        .insert(
            "create",
            CliCommand::new(&api2::config::access::ldap::API_METHOD_CREATE_LDAP_REALM)
                .arg_param(&["realm"])
                .completion_cb("realm", pbs_config::domains::complete_ldap_realm_name),
        )
        .insert(
            "update",
            CliCommand::new(&api2::config::access::ldap::API_METHOD_UPDATE_LDAP_REALM)
                .arg_param(&["realm"])
                .completion_cb("realm", pbs_config::domains::complete_ldap_realm_name),
        )
        .insert(
            "delete",
            CliCommand::new(&api2::config::access::ldap::API_METHOD_DELETE_LDAP_REALM)
                .arg_param(&["realm"])
                .completion_cb("realm", pbs_config::domains::complete_ldap_realm_name),
        )
        .insert(
            "sync",
            CliCommand::new(&API_METHOD_SYNC_LDAP_REALM)
                .arg_param(&["realm"])
                .completion_cb("realm", pbs_config::domains::complete_ldap_realm_name),
        );

    cmd_def.into()
}
