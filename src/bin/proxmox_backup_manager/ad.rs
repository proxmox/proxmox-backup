use anyhow::Error;
use serde_json::Value;

use proxmox_router::{cli::*, ApiHandler, RpcEnvironment};
use proxmox_schema::api;

use pbs_api_types::REALM_ID_SCHEMA;

use crate::api2;

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
/// List configured AD realms
fn list_ad_realms(param: Value, rpcenv: &mut dyn RpcEnvironment) -> Result<Value, Error> {
    let output_format = get_output_format(&param);

    let info = &api2::config::access::ad::API_METHOD_LIST_AD_REALMS;
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
/// Show AD realm configuration
pub fn show_ad_realm(param: Value, rpcenv: &mut dyn RpcEnvironment) -> Result<Value, Error> {
    let output_format = get_output_format(&param);

    let info = &api2::config::access::ad::API_METHOD_READ_AD_REALM;
    let mut data = match info.handler {
        ApiHandler::Sync(handler) => (handler)(param, info, rpcenv)?,
        _ => unreachable!(),
    };

    let options = default_table_format_options();
    format_and_print_result_full(&mut data, &info.returns, &output_format, &options);

    Ok(Value::Null)
}

pub fn ad_commands() -> CommandLineInterface {
    let cmd_def = CliCommandMap::new()
        .insert("list", CliCommand::new(&API_METHOD_LIST_AD_REALMS))
        .insert(
            "show",
            CliCommand::new(&crate::API_METHOD_SHOW_AD_REALM)
                .arg_param(&["realm"])
                .completion_cb("realm", pbs_config::domains::complete_ad_realm_name),
        )
        .insert(
            "create",
            CliCommand::new(&api2::config::access::ad::API_METHOD_CREATE_AD_REALM)
                .arg_param(&["realm"])
                .completion_cb("realm", pbs_config::domains::complete_ad_realm_name),
        )
        .insert(
            "update",
            CliCommand::new(&api2::config::access::ad::API_METHOD_UPDATE_AD_REALM)
                .arg_param(&["realm"])
                .completion_cb("realm", pbs_config::domains::complete_ad_realm_name),
        )
        .insert(
            "delete",
            CliCommand::new(&api2::config::access::ldap::API_METHOD_DELETE_LDAP_REALM)
                .arg_param(&["realm"])
                .completion_cb("realm", pbs_config::domains::complete_ad_realm_name),
        )
        .insert(
            "sync",
            CliCommand::new(&crate::API_METHOD_SYNC_LDAP_REALM)
                .arg_param(&["realm"])
                .completion_cb("realm", pbs_config::domains::complete_ad_realm_name),
        );

    cmd_def.into()
}
