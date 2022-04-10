use anyhow::{bail, Error};
use serde_json::Value;

use proxmox_router::{cli::*, ApiHandler, RpcEnvironment};
use proxmox_schema::api;

use proxmox_backup::api2;

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
/// Access Control list.
fn list_acls(param: Value, rpcenv: &mut dyn RpcEnvironment) -> Result<Value, Error> {
    let output_format = get_output_format(&param);

    let info = &api2::access::acl::API_METHOD_READ_ACL;
    let mut data = match info.handler {
        ApiHandler::Sync(handler) => (handler)(param, info, rpcenv)?,
        _ => unreachable!(),
    };

    fn render_ugid(value: &Value, record: &Value) -> Result<String, Error> {
        if value.is_null() {
            return Ok(String::new());
        }
        let ugid = value.as_str().unwrap();
        let ugid_type = record["ugid_type"].as_str().unwrap();

        if ugid_type == "user" {
            Ok(ugid.to_string())
        } else if ugid_type == "group" {
            Ok(format!("@{}", ugid))
        } else {
            bail!("render_ugid: got unknown ugid_type");
        }
    }

    let options = default_table_format_options()
        .column(ColumnConfig::new("ugid").renderer(render_ugid))
        .column(ColumnConfig::new("path"))
        .column(ColumnConfig::new("propagate"))
        .column(ColumnConfig::new("roleid"));

    format_and_print_result_full(&mut data, &info.returns, &output_format, &options);

    Ok(Value::Null)
}

pub fn acl_commands() -> CommandLineInterface {
    let cmd_def = CliCommandMap::new()
        .insert("list", CliCommand::new(&API_METHOD_LIST_ACLS))
        .insert(
            "update",
            CliCommand::new(&api2::access::acl::API_METHOD_UPDATE_ACL)
                .arg_param(&["path", "role"])
                .completion_cb("auth-id", pbs_config::user::complete_authid)
                .completion_cb("path", pbs_config::datastore::complete_acl_path),
        );

    cmd_def.into()
}
