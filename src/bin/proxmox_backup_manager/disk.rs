use anyhow::{bail, Error};
use serde_json::Value;

use proxmox_router::{cli::*, ApiHandler, RpcEnvironment};
use proxmox_schema::api;
use std::io::{IsTerminal, Write};

use pbs_api_types::{
    ZfsCompressionType, ZfsRaidLevel, BLOCKDEVICE_DISK_AND_PARTITION_NAME_SCHEMA,
    BLOCKDEVICE_NAME_SCHEMA, DATASTORE_SCHEMA, DISK_LIST_SCHEMA, ZFS_ASHIFT_SCHEMA,
};
use proxmox_backup::tools::disks::{
    complete_disk_name, complete_partition_name, FileSystemType, SmartAttribute,
};

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
/// Local disk list.
fn list_disks(mut param: Value, rpcenv: &mut dyn RpcEnvironment) -> Result<Value, Error> {
    let output_format = get_output_format(&param);

    param["node"] = "localhost".into();

    let info = &api2::node::disks::API_METHOD_LIST_DISKS;
    let mut data = match info.handler {
        ApiHandler::Sync(handler) => (handler)(param, info, rpcenv)?,
        _ => unreachable!(),
    };

    let render_wearout = |value: &Value, _record: &Value| -> Result<String, Error> {
        match value.as_f64() {
            Some(value) => Ok(format!(
                "{:.2} %",
                if value <= 100.0 { 100.0 - value } else { 0.0 }
            )),
            None => Ok(String::from("-")),
        }
    };

    let options = default_table_format_options()
        .column(ColumnConfig::new("name"))
        .column(ColumnConfig::new("used"))
        .column(ColumnConfig::new("gpt"))
        .column(ColumnConfig::new("disk-type"))
        .column(ColumnConfig::new("size"))
        .column(ColumnConfig::new("model"))
        .column(ColumnConfig::new("wearout").renderer(render_wearout))
        .column(ColumnConfig::new("status"));

    format_and_print_result_full(&mut data, &info.returns, &output_format, &options);

    Ok(Value::Null)
}

#[api(
    input: {
        properties: {
            disk: {
                schema: BLOCKDEVICE_NAME_SCHEMA,
            },
            "output-format": {
                schema: OUTPUT_FORMAT,
                optional: true,
            },
        }
    },
    returns: {
        description: "SMART attributes.",
        type: Array,
        items: {
            type: SmartAttribute,
        },
    }
)]
/// Show SMART attributes.
fn smart_attributes(mut param: Value, rpcenv: &mut dyn RpcEnvironment) -> Result<Value, Error> {
    let output_format = get_output_format(&param);

    param["node"] = "localhost".into();

    let info = &api2::node::disks::API_METHOD_SMART_STATUS;
    let mut data = match info.handler {
        ApiHandler::Sync(handler) => (handler)(param, info, rpcenv)?,
        _ => unreachable!(),
    };

    let mut data = data["attributes"].take();

    let options = default_table_format_options();
    format_and_print_result_full(
        &mut data,
        &API_METHOD_SMART_ATTRIBUTES.returns,
        &output_format,
        &options,
    );

    Ok(Value::Null)
}

#[api(
   input: {
        properties: {
            disk: {
                schema: BLOCKDEVICE_NAME_SCHEMA,
            },
            uuid: {
                description: "UUID for the GPT table.",
                type: String,
                optional: true,
                max_length: 36,
            },
        },
   },
)]
/// Initialize empty Disk with GPT
async fn initialize_disk(
    mut param: Value,
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<Value, Error> {
    param["node"] = "localhost".into();

    let info = &api2::node::disks::API_METHOD_INITIALIZE_DISK;
    let result = match info.handler {
        ApiHandler::Sync(handler) => (handler)(param, info, rpcenv)?,
        _ => unreachable!(),
    };

    crate::wait_for_local_worker(result.as_str().unwrap()).await?;

    Ok(Value::Null)
}

#[api(
   input: {
        properties: {
            disk: {
                schema: BLOCKDEVICE_DISK_AND_PARTITION_NAME_SCHEMA,
            },
        },
   },
)]
/// wipe disk
async fn wipe_disk(mut param: Value, rpcenv: &mut dyn RpcEnvironment) -> Result<Value, Error> {
    param["node"] = "localhost".into();

    // If we're on a TTY, query the user
    if std::io::stdin().is_terminal() {
        println!("You are about to wipe block device {}.", param["disk"]);
        print!("Are you sure you want to continue? (y/N): ");
        let _ = std::io::stdout().flush();
        use std::io::{BufRead, BufReader};
        let mut line = String::new();
        match BufReader::new(std::io::stdin()).read_line(&mut line) {
            Ok(_) => match line.trim() {
                "y" | "Y" => (), // continue
                _ => bail!("Aborting."),
            },
            Err(err) => bail!("Failed to read line - {err}."),
        }
    }

    let info = &api2::node::disks::API_METHOD_WIPE_DISK;
    let result = match info.handler {
        ApiHandler::Sync(handler) => (handler)(param, info, rpcenv)?,
        _ => unreachable!(),
    };

    crate::wait_for_local_worker(result.as_str().unwrap()).await?;

    Ok(Value::Null)
}

#[api(
   input: {
        properties: {
           name: {
                schema: DATASTORE_SCHEMA,
            },
            devices: {
                schema: DISK_LIST_SCHEMA,
            },
            raidlevel: {
                type: ZfsRaidLevel,
            },
            ashift: {
                schema: ZFS_ASHIFT_SCHEMA,
                optional: true,
            },
            compression: {
                type: ZfsCompressionType,
                optional: true,
            },
            "add-datastore": {
                description: "Configure a datastore using the zpool.",
                type: bool,
                optional: true,
            },
       },
   },
)]
/// create a zfs pool
async fn create_zpool(mut param: Value, rpcenv: &mut dyn RpcEnvironment) -> Result<Value, Error> {
    param["node"] = "localhost".into();

    let info = &api2::node::disks::zfs::API_METHOD_CREATE_ZPOOL;
    let result = match info.handler {
        ApiHandler::Sync(handler) => (handler)(param, info, rpcenv)?,
        _ => unreachable!(),
    };

    crate::wait_for_local_worker(result.as_str().unwrap()).await?;

    Ok(Value::Null)
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
/// Local zfs pools.
fn list_zpools(mut param: Value, rpcenv: &mut dyn RpcEnvironment) -> Result<Value, Error> {
    let output_format = get_output_format(&param);

    param["node"] = "localhost".into();

    let info = &api2::node::disks::zfs::API_METHOD_LIST_ZPOOLS;
    let mut data = match info.handler {
        ApiHandler::Sync(handler) => (handler)(param, info, rpcenv)?,
        _ => unreachable!(),
    };

    let render_usage = |value: &Value, record: &Value| -> Result<String, Error> {
        let value = value.as_u64().unwrap_or(0);
        let size = match record["size"].as_u64() {
            Some(size) => size,
            None => bail!("missing size property"),
        };
        if size == 0 {
            bail!("got zero size");
        }
        Ok(format!("{:.2} %", (value as f64) / (size as f64)))
    };

    let options = default_table_format_options()
        .column(ColumnConfig::new("name"))
        .column(ColumnConfig::new("size"))
        .column(
            ColumnConfig::new("alloc")
                .right_align(true)
                .renderer(render_usage),
        )
        .column(ColumnConfig::new("health"));

    format_and_print_result_full(&mut data, &info.returns, &output_format, &options);

    Ok(Value::Null)
}

pub fn zpool_commands() -> CommandLineInterface {
    let cmd_def = CliCommandMap::new()
        .insert("list", CliCommand::new(&API_METHOD_LIST_ZPOOLS))
        .insert(
            "create",
            CliCommand::new(&API_METHOD_CREATE_ZPOOL)
                .arg_param(&["name"])
                .completion_cb("devices", complete_disk_name), // fixme: complete the list
        );

    cmd_def.into()
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
/// List systemd datastore mount units.
fn list_datastore_mounts(
    mut param: Value,
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<Value, Error> {
    let output_format = get_output_format(&param);

    param["node"] = "localhost".into();

    let info = &api2::node::disks::directory::API_METHOD_LIST_DATASTORE_MOUNTS;
    let mut data = match info.handler {
        ApiHandler::Sync(handler) => (handler)(param, info, rpcenv)?,
        _ => unreachable!(),
    };

    let options = default_table_format_options()
        .column(ColumnConfig::new("path"))
        .column(ColumnConfig::new("device"))
        .column(ColumnConfig::new("filesystem"))
        .column(ColumnConfig::new("options"));

    format_and_print_result_full(&mut data, &info.returns, &output_format, &options);

    Ok(Value::Null)
}

#[api(
   input: {
        properties: {
            name: {
                schema: DATASTORE_SCHEMA,
            },
            disk: {
                schema: BLOCKDEVICE_NAME_SCHEMA,
            },
            "add-datastore": {
                description: "Configure a datastore using the directory.",
                type: bool,
                optional: true,
            },
            filesystem: {
                type: FileSystemType,
                optional: true,
            },
        },
   },
)]
/// Create a Filesystem on an unused disk. Will be mounted under `/mnt/datastore/<name>`.
async fn create_datastore_disk(
    mut param: Value,
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<Value, Error> {
    param["node"] = "localhost".into();

    let info = &api2::node::disks::directory::API_METHOD_CREATE_DATASTORE_DISK;
    let result = match info.handler {
        ApiHandler::Sync(handler) => (handler)(param, info, rpcenv)?,
        _ => unreachable!(),
    };

    crate::wait_for_local_worker(result.as_str().unwrap()).await?;

    Ok(Value::Null)
}

#[api(
   input: {
        properties: {
            name: {
                schema: DATASTORE_SCHEMA,
            },
        },
   },
)]
/// Remove a Filesystem mounted under `/mnt/datastore/<name>`.
async fn delete_datastore_disk(
    mut param: Value,
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<Value, Error> {
    param["node"] = "localhost".into();

    let info = &api2::node::disks::directory::API_METHOD_DELETE_DATASTORE_DISK;
    let _result = match info.handler {
        ApiHandler::Sync(handler) => (handler)(param, info, rpcenv)?,
        _ => unreachable!(),
    };

    Ok(Value::Null)
}

pub fn filesystem_commands() -> CommandLineInterface {
    let cmd_def = CliCommandMap::new()
        .insert("list", CliCommand::new(&API_METHOD_LIST_DATASTORE_MOUNTS))
        .insert(
            "create",
            CliCommand::new(&API_METHOD_CREATE_DATASTORE_DISK)
                .arg_param(&["name"])
                .completion_cb("disk", complete_disk_name),
        )
        .insert(
            "delete",
            CliCommand::new(&API_METHOD_DELETE_DATASTORE_DISK).arg_param(&["name"]),
        );

    cmd_def.into()
}

pub fn disk_commands() -> CommandLineInterface {
    let cmd_def = CliCommandMap::new()
        .insert("list", CliCommand::new(&API_METHOD_LIST_DISKS))
        .insert(
            "smart-attributes",
            CliCommand::new(&API_METHOD_SMART_ATTRIBUTES)
                .arg_param(&["disk"])
                .completion_cb("disk", complete_disk_name),
        )
        .insert("fs", filesystem_commands())
        .insert("zpool", zpool_commands())
        .insert(
            "initialize",
            CliCommand::new(&API_METHOD_INITIALIZE_DISK)
                .arg_param(&["disk"])
                .completion_cb("disk", complete_disk_name),
        )
        .insert(
            "wipe",
            CliCommand::new(&API_METHOD_WIPE_DISK)
                .arg_param(&["disk"])
                .completion_cb("disk", complete_partition_name),
        );

    cmd_def.into()
}
