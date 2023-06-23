use anyhow::Error;
use serde_json::Value;

use std::collections::HashMap;

use proxmox_router::{cli::*, ApiHandler, RpcEnvironment};
use proxmox_schema::api;

use pbs_api_types::{Authid, Userid, ACL_PATH_SCHEMA};

use proxmox_backup::api2;

fn render_expire(value: &Value, _record: &Value) -> Result<String, Error> {
    let never = String::from("never");
    if value.is_null() {
        return Ok(never);
    }
    let text = match value.as_i64() {
        Some(epoch) if epoch == 0 => never,
        Some(epoch) => {
            if let Ok(epoch_string) = proxmox_time::strftime_local("%c", epoch) {
                epoch_string
            } else {
                epoch.to_string()
            }
        }
        None => value.to_string(),
    };
    Ok(text)
}

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
/// List configured users.
fn list_users(param: Value, rpcenv: &mut dyn RpcEnvironment) -> Result<Value, Error> {
    let output_format = get_output_format(&param);

    let info = &api2::access::user::API_METHOD_LIST_USERS;
    let mut data = match info.handler {
        ApiHandler::Sync(handler) => (handler)(param, info, rpcenv)?,
        _ => unreachable!(),
    };

    let options = default_table_format_options()
        .column(ColumnConfig::new("userid"))
        .column(
            ColumnConfig::new("enable").renderer(pbs_tools::format::render_bool_with_default_true),
        )
        .column(ColumnConfig::new("expire").renderer(render_expire))
        .column(ColumnConfig::new("firstname"))
        .column(ColumnConfig::new("lastname"))
        .column(ColumnConfig::new("email"))
        .column(ColumnConfig::new("comment"));

    format_and_print_result_full(&mut data, &info.returns, &output_format, &options);

    Ok(Value::Null)
}

#[api(
    input: {
        properties: {
            "output-format": {
                schema: OUTPUT_FORMAT,
                optional: true,
            },
            userid: {
                type: Userid,
            }
        }
    }
)]
/// List tokens associated with user.
fn list_tokens(param: Value, rpcenv: &mut dyn RpcEnvironment) -> Result<Value, Error> {
    let output_format = get_output_format(&param);

    let info = &api2::access::user::API_METHOD_LIST_TOKENS;
    let mut data = match info.handler {
        ApiHandler::Sync(handler) => (handler)(param, info, rpcenv)?,
        _ => unreachable!(),
    };

    let options = default_table_format_options()
        .column(ColumnConfig::new("tokenid"))
        .column(
            ColumnConfig::new("enable").renderer(pbs_tools::format::render_bool_with_default_true),
        )
        .column(ColumnConfig::new("expire").renderer(render_expire))
        .column(ColumnConfig::new("comment"));

    format_and_print_result_full(&mut data, &info.returns, &output_format, &options);

    Ok(Value::Null)
}

#[api(
    input: {
        properties: {
            "output-format": {
                schema: OUTPUT_FORMAT,
                optional: true,
            },
            "auth-id": {
                type: Authid,
            },
            path: {
                schema: ACL_PATH_SCHEMA,
                optional: true,
            },
        }
    }
)]
/// List permissions of user/token.
fn list_permissions(param: Value, rpcenv: &mut dyn RpcEnvironment) -> Result<Value, Error> {
    let output_format = get_output_format(&param);

    let info = &api2::access::API_METHOD_LIST_PERMISSIONS;
    let data = match info.handler {
        ApiHandler::Sync(handler) => (handler)(param, info, rpcenv)?,
        _ => unreachable!(),
    };

    if output_format == "text" {
        println!("Privileges with (*) have the propagate flag set\n");
        let data: HashMap<String, HashMap<String, bool>> = serde_json::from_value(data)?;
        let mut paths: Vec<String> = data.keys().cloned().collect();
        paths.sort_unstable();
        for path in paths {
            println!("Path: {}", path);
            let priv_map = data.get(&path).unwrap();
            let mut privs: Vec<String> = priv_map.keys().cloned().collect();
            if privs.is_empty() {
                println!("- NoAccess");
            } else {
                privs.sort_unstable();
                for privilege in privs {
                    if *priv_map.get(&privilege).unwrap() {
                        println!("- {} (*)", privilege);
                    } else {
                        println!("- {}", privilege);
                    }
                }
            }
        }
    } else {
        format_and_print_result(&data, &output_format);
    }

    Ok(Value::Null)
}

#[api(
    input: {
        properties: {
            "output-format": {
                schema: OUTPUT_FORMAT,
                optional: true,
            },
            userid: {
                type: Userid,
            }
        },
    }
)]
/// List all tfa methods for a user.
fn list_user_tfa(param: Value, rpcenv: &mut dyn RpcEnvironment) -> Result<Value, Error> {
    let output_format = get_output_format(&param);

    let info = &api2::access::tfa::API_METHOD_LIST_USER_TFA;
    let mut data = match info.handler {
        ApiHandler::Sync(handler) => (handler)(param, info, rpcenv)?,
        _ => unreachable!(),
    };

    let options = default_table_format_options()
        .column(ColumnConfig::new("id"))
        .column(ColumnConfig::new("type"))
        .column(ColumnConfig::new("description"))
        .column(ColumnConfig::new("created").renderer(pbs_tools::format::render_epoch));

    format_and_print_result_full(&mut data, &info.returns, &output_format, &options);

    Ok(Value::Null)
}

pub fn user_commands() -> CommandLineInterface {
    let cmd_def = CliCommandMap::new()
        .insert("list", CliCommand::new(&API_METHOD_LIST_USERS))
        .insert(
            "create",
            // fixme: howto handle password parameter?
            CliCommand::new(&api2::access::user::API_METHOD_CREATE_USER).arg_param(&["userid"]),
        )
        .insert(
            "update",
            CliCommand::new(&api2::access::user::API_METHOD_UPDATE_USER)
                .arg_param(&["userid"])
                .completion_cb("userid", pbs_config::user::complete_userid),
        )
        .insert(
            "remove",
            CliCommand::new(&api2::access::user::API_METHOD_DELETE_USER)
                .arg_param(&["userid"])
                .completion_cb("userid", pbs_config::user::complete_userid),
        )
        .insert(
            "list-tokens",
            CliCommand::new(&API_METHOD_LIST_TOKENS)
                .arg_param(&["userid"])
                .completion_cb("userid", pbs_config::user::complete_userid),
        )
        .insert(
            "generate-token",
            CliCommand::new(&api2::access::user::API_METHOD_GENERATE_TOKEN)
                .arg_param(&["userid", "token-name"])
                .completion_cb("userid", pbs_config::user::complete_userid),
        )
        .insert(
            "delete-token",
            CliCommand::new(&api2::access::user::API_METHOD_DELETE_TOKEN)
                .arg_param(&["userid", "token-name"])
                .completion_cb("userid", pbs_config::user::complete_userid)
                .completion_cb("token-name", pbs_config::user::complete_token_name),
        )
        .insert("tfa", tfa_commands())
        .insert(
            "permissions",
            CliCommand::new(&API_METHOD_LIST_PERMISSIONS)
                .arg_param(&["auth-id"])
                .completion_cb("auth-id", pbs_config::user::complete_authid)
                .completion_cb("path", pbs_config::datastore::complete_acl_path),
        );

    cmd_def.into()
}

fn tfa_commands() -> CommandLineInterface {
    CliCommandMap::new()
        .insert(
            "list",
            CliCommand::new(&API_METHOD_LIST_USER_TFA)
                .arg_param(&["userid"])
                .completion_cb("userid", pbs_config::user::complete_userid),
        )
        .insert(
            "delete",
            CliCommand::new(&api2::access::tfa::API_METHOD_DELETE_TFA)
                .arg_param(&["userid", "id"])
                .completion_cb("userid", pbs_config::user::complete_userid)
                .completion_cb("id", proxmox_backup::config::tfa::complete_tfa_id),
        )
        .insert(
            "unlock",
            CliCommand::new(&api2::access::user::API_METHOD_UNLOCK_TFA)
                .arg_param(&["userid"])
                .completion_cb("userid", pbs_config::user::complete_userid),
        )
        .into()
}
