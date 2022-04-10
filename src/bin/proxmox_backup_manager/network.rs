use anyhow::Error;
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
/// Network device list.
fn list_network_devices(mut param: Value, rpcenv: &mut dyn RpcEnvironment) -> Result<Value, Error> {
    let output_format = get_output_format(&param);

    param["node"] = "localhost".into();

    let info = &api2::node::network::API_METHOD_LIST_NETWORK_DEVICES;
    let mut data = match info.handler {
        ApiHandler::Sync(handler) => (handler)(param, info, rpcenv)?,
        _ => unreachable!(),
    };

    if let Value::String(ref diff) = rpcenv["changes"] {
        if output_format == "text" {
            eprintln!("pending changes:\n{}\n", diff);
        }
    }

    fn render_address(_value: &Value, record: &Value) -> Result<String, Error> {
        let mut text = String::new();

        if let Some(cidr) = record["cidr"].as_str() {
            text.push_str(cidr);
        }
        if let Some(cidr) = record["cidr6"].as_str() {
            if !text.is_empty() {
                text.push('\n');
            }
            text.push_str(cidr);
        }

        Ok(text)
    }

    fn render_ports(_value: &Value, record: &Value) -> Result<String, Error> {
        let mut text = String::new();

        if let Some(ports) = record["bridge_ports"].as_array() {
            let list: Vec<&str> = ports.iter().filter_map(|v| v.as_str()).collect();
            text.push_str(&list.join(" "));
        }
        if let Some(slaves) = record["slaves"].as_array() {
            let list: Vec<&str> = slaves.iter().filter_map(|v| v.as_str()).collect();
            text.push_str(&list.join(" "));
        }

        Ok(text)
    }

    fn render_gateway(_value: &Value, record: &Value) -> Result<String, Error> {
        let mut text = String::new();

        if let Some(gateway) = record["gateway"].as_str() {
            text.push_str(gateway);
        }
        if let Some(gateway) = record["gateway6"].as_str() {
            if !text.is_empty() {
                text.push('\n');
            }
            text.push_str(gateway);
        }

        Ok(text)
    }

    let options = default_table_format_options()
        .column(ColumnConfig::new("name"))
        .column(ColumnConfig::new("type").header("type"))
        .column(ColumnConfig::new("autostart"))
        .column(ColumnConfig::new("method"))
        .column(ColumnConfig::new("method6"))
        .column(
            ColumnConfig::new("cidr")
                .header("address")
                .renderer(render_address),
        )
        .column(
            ColumnConfig::new("gateway")
                .header("gateway")
                .renderer(render_gateway),
        )
        .column(
            ColumnConfig::new("bridge_ports")
                .header("ports/slaves")
                .renderer(render_ports),
        );

    format_and_print_result_full(&mut data, &info.returns, &output_format, &options);

    Ok(Value::Null)
}

#[api()]
/// Show pending configuration changes (diff)
fn pending_network_changes(
    mut param: Value,
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<Value, Error> {
    param["node"] = "localhost".into();

    let info = &api2::node::network::API_METHOD_LIST_NETWORK_DEVICES;
    let _data = match info.handler {
        ApiHandler::Sync(handler) => (handler)(param, info, rpcenv)?,
        _ => unreachable!(),
    };

    if let Value::String(ref diff) = rpcenv["changes"] {
        println!("{}", diff);
    }

    Ok(Value::Null)
}

pub fn network_commands() -> CommandLineInterface {
    let cmd_def = CliCommandMap::new()
        .insert("list", CliCommand::new(&API_METHOD_LIST_NETWORK_DEVICES))
        .insert(
            "changes",
            CliCommand::new(&API_METHOD_PENDING_NETWORK_CHANGES),
        )
        .insert(
            "create",
            CliCommand::new(&api2::node::network::API_METHOD_CREATE_INTERFACE)
                .fixed_param("node", String::from("localhost"))
                .arg_param(&["iface"])
                .completion_cb("iface", pbs_config::network::complete_interface_name)
                .completion_cb("bridge_ports", pbs_config::network::complete_port_list)
                .completion_cb("slaves", pbs_config::network::complete_port_list),
        )
        .insert(
            "update",
            CliCommand::new(&api2::node::network::API_METHOD_UPDATE_INTERFACE)
                .fixed_param("node", String::from("localhost"))
                .arg_param(&["iface"])
                .completion_cb("iface", pbs_config::network::complete_interface_name)
                .completion_cb("bridge_ports", pbs_config::network::complete_port_list)
                .completion_cb("slaves", pbs_config::network::complete_port_list),
        )
        .insert(
            "remove",
            CliCommand::new(&api2::node::network::API_METHOD_DELETE_INTERFACE)
                .fixed_param("node", String::from("localhost"))
                .arg_param(&["iface"])
                .completion_cb("iface", pbs_config::network::complete_interface_name),
        )
        .insert(
            "revert",
            CliCommand::new(&api2::node::network::API_METHOD_REVERT_NETWORK_CONFIG)
                .fixed_param("node", String::from("localhost")),
        )
        .insert(
            "reload",
            CliCommand::new(&api2::node::network::API_METHOD_RELOAD_NETWORK_CONFIG)
                .fixed_param("node", String::from("localhost")),
        );

    cmd_def.into()
}
