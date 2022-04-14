use std::collections::HashMap;
use std::io::{self, Write};

use anyhow::Error;
use serde_json::{json, Value};

use proxmox_router::{cli::*, RpcEnvironment};
use proxmox_schema::api;
use proxmox_sys::fs::CreateOptions;

use pbs_api_types::percent_encoding::percent_encode_component;
use pbs_api_types::{
    GroupFilter, SyncJobConfig, DATASTORE_SCHEMA, GROUP_FILTER_LIST_SCHEMA,
    IGNORE_VERIFIED_BACKUPS_SCHEMA, REMOTE_ID_SCHEMA, REMOVE_VANISHED_BACKUPS_SCHEMA, UPID_SCHEMA,
    VERIFICATION_OUTDATED_AFTER_SCHEMA,
};
use pbs_client::{display_task_log, view_task_result};
use pbs_config::sync;
use pbs_tools::json::required_string_param;

use proxmox_rest_server::wait_for_local_worker;

use proxmox_backup::api2;
use proxmox_backup::client_helpers::connect_to_localhost;
use proxmox_backup::config;

mod proxmox_backup_manager;
use proxmox_backup_manager::*;

#[api(
   input: {
        properties: {
            store: {
                schema: DATASTORE_SCHEMA,
            },
            "output-format": {
                schema: OUTPUT_FORMAT,
                optional: true,
            },
        }
   }
)]
/// Start garbage collection for a specific datastore.
async fn start_garbage_collection(param: Value) -> Result<Value, Error> {
    let output_format = get_output_format(&param);

    let store = required_string_param(&param, "store")?;

    let client = connect_to_localhost()?;

    let path = format!("api2/json/admin/datastore/{}/gc", store);

    let result = client.post(&path, None).await?;

    view_task_result(&client, result, &output_format).await?;

    Ok(Value::Null)
}

#[api(
   input: {
        properties: {
            store: {
                schema: DATASTORE_SCHEMA,
            },
            "output-format": {
                schema: OUTPUT_FORMAT,
                optional: true,
            },
        }
   }
)]
/// Show garbage collection status for a specific datastore.
async fn garbage_collection_status(param: Value) -> Result<Value, Error> {
    let output_format = get_output_format(&param);

    let store = required_string_param(&param, "store")?;

    let client = connect_to_localhost()?;

    let path = format!("api2/json/admin/datastore/{}/gc", store);

    let mut result = client.get(&path, None).await?;
    let mut data = result["data"].take();
    let return_type = &api2::admin::datastore::API_METHOD_GARBAGE_COLLECTION_STATUS.returns;

    let options = default_table_format_options();

    format_and_print_result_full(&mut data, return_type, &output_format, &options);

    Ok(Value::Null)
}

fn garbage_collection_commands() -> CommandLineInterface {
    let cmd_def = CliCommandMap::new()
        .insert(
            "status",
            CliCommand::new(&API_METHOD_GARBAGE_COLLECTION_STATUS)
                .arg_param(&["store"])
                .completion_cb("store", pbs_config::datastore::complete_datastore_name),
        )
        .insert(
            "start",
            CliCommand::new(&API_METHOD_START_GARBAGE_COLLECTION)
                .arg_param(&["store"])
                .completion_cb("store", pbs_config::datastore::complete_datastore_name),
        );

    cmd_def.into()
}

#[api(
    input: {
        properties: {
            limit: {
                description: "The maximal number of tasks to list.",
                type: Integer,
                optional: true,
                minimum: 1,
                maximum: 1000,
                default: 50,
            },
            "output-format": {
                schema: OUTPUT_FORMAT,
                optional: true,
            },
            all: {
                type: Boolean,
                description: "Also list stopped tasks.",
                optional: true,
            }
        }
    }
)]
/// List running server tasks.
async fn task_list(param: Value) -> Result<Value, Error> {
    let output_format = get_output_format(&param);

    let client = connect_to_localhost()?;

    let limit = param["limit"].as_u64().unwrap_or(50) as usize;
    let running = !param["all"].as_bool().unwrap_or(false);
    let args = json!({
        "running": running,
        "start": 0,
        "limit": limit,
    });
    let mut result = client
        .get("api2/json/nodes/localhost/tasks", Some(args))
        .await?;

    let mut data = result["data"].take();
    let return_type = &api2::node::tasks::API_METHOD_LIST_TASKS.returns;

    use pbs_tools::format::{render_epoch, render_task_status};
    let options = default_table_format_options()
        .column(
            ColumnConfig::new("starttime")
                .right_align(false)
                .renderer(render_epoch),
        )
        .column(
            ColumnConfig::new("endtime")
                .right_align(false)
                .renderer(render_epoch),
        )
        .column(ColumnConfig::new("upid"))
        .column(ColumnConfig::new("status").renderer(render_task_status));

    format_and_print_result_full(&mut data, return_type, &output_format, &options);

    Ok(Value::Null)
}

#[api(
    input: {
        properties: {
            upid: {
                schema: UPID_SCHEMA,
            },
        }
    }
)]
/// Display the task log.
async fn task_log(param: Value) -> Result<Value, Error> {
    let upid = required_string_param(&param, "upid")?;

    let client = connect_to_localhost()?;

    display_task_log(&client, upid, true).await?;

    Ok(Value::Null)
}

#[api(
    input: {
        properties: {
            upid: {
                schema: UPID_SCHEMA,
            },
        }
    }
)]
/// Try to stop a specific task.
async fn task_stop(param: Value) -> Result<Value, Error> {
    let upid_str = required_string_param(&param, "upid")?;

    let client = connect_to_localhost()?;

    let path = format!(
        "api2/json/nodes/localhost/tasks/{}",
        percent_encode_component(upid_str)
    );
    let _ = client.delete(&path, None).await?;

    Ok(Value::Null)
}

fn task_mgmt_cli() -> CommandLineInterface {
    let task_log_cmd_def = CliCommand::new(&API_METHOD_TASK_LOG).arg_param(&["upid"]);

    let task_stop_cmd_def = CliCommand::new(&API_METHOD_TASK_STOP).arg_param(&["upid"]);

    let cmd_def = CliCommandMap::new()
        .insert("list", CliCommand::new(&API_METHOD_TASK_LIST))
        .insert("log", task_log_cmd_def)
        .insert("stop", task_stop_cmd_def);

    cmd_def.into()
}

// fixme: avoid API redefinition
#[api(
   input: {
        properties: {
            "local-store": {
                schema: DATASTORE_SCHEMA,
            },
            remote: {
                schema: REMOTE_ID_SCHEMA,
            },
            "remote-store": {
                schema: DATASTORE_SCHEMA,
            },
            "remove-vanished": {
                schema: REMOVE_VANISHED_BACKUPS_SCHEMA,
                optional: true,
            },
            "group-filter": {
                schema: GROUP_FILTER_LIST_SCHEMA,
                optional: true,
            },
            "output-format": {
                schema: OUTPUT_FORMAT,
                optional: true,
            },
        }
   }
)]
/// Sync datastore from another repository
async fn pull_datastore(
    remote: String,
    remote_store: String,
    local_store: String,
    remove_vanished: Option<bool>,
    group_filter: Option<Vec<GroupFilter>>,
    param: Value,
) -> Result<Value, Error> {
    let output_format = get_output_format(&param);

    let client = connect_to_localhost()?;

    let mut args = json!({
        "store": local_store,
        "remote": remote,
        "remote-store": remote_store,
    });

    if group_filter.is_some() {
        args["group-filter"] = json!(group_filter);
    }

    if let Some(remove_vanished) = remove_vanished {
        args["remove-vanished"] = Value::from(remove_vanished);
    }

    let result = client.post("api2/json/pull", Some(args)).await?;

    view_task_result(&client, result, &output_format).await?;

    Ok(Value::Null)
}

#[api(
   input: {
        properties: {
            "store": {
                schema: DATASTORE_SCHEMA,
            },
            "ignore-verified": {
                schema: IGNORE_VERIFIED_BACKUPS_SCHEMA,
                optional: true,
            },
            "outdated-after": {
                schema: VERIFICATION_OUTDATED_AFTER_SCHEMA,
                optional: true,
            },
            "output-format": {
                schema: OUTPUT_FORMAT,
                optional: true,
            },
        }
   }
)]
/// Verify backups
async fn verify(store: String, mut param: Value) -> Result<Value, Error> {
    let output_format = extract_output_format(&mut param);

    let client = connect_to_localhost()?;

    let args = json!(param);

    let path = format!("api2/json/admin/datastore/{}/verify", store);

    let result = client.post(&path, Some(args)).await?;

    view_task_result(&client, result, &output_format).await?;

    Ok(Value::Null)
}

#[api()]
/// System report
async fn report() -> Result<Value, Error> {
    let report = proxmox_backup::server::generate_report();
    io::stdout().write_all(report.as_bytes())?;
    Ok(Value::Null)
}

#[api(
    input: {
        properties: {
            verbose: {
                type: Boolean,
                optional: true,
                default: false,
                description: "Output verbose package information. It is ignored if output-format is specified.",
            },
            "output-format": {
                schema: OUTPUT_FORMAT,
                optional: true,
            }
        }
    }
)]
/// List package versions for important Proxmox Backup Server packages.
async fn get_versions(verbose: bool, param: Value) -> Result<Value, Error> {
    let output_format = get_output_format(&param);

    let packages = crate::api2::node::apt::get_versions()?;
    let mut packages = json!(if verbose {
        &packages[..]
    } else {
        &packages[1..2]
    });

    let options = default_table_format_options()
        .disable_sort()
        .noborder(true) // just not helpful for version info which gets copy pasted often
        .column(ColumnConfig::new("Package"))
        .column(ColumnConfig::new("Version"))
        .column(ColumnConfig::new("ExtraInfo").header("Extra Info"));
    let return_type = &crate::api2::node::apt::API_METHOD_GET_VERSIONS.returns;

    format_and_print_result_full(&mut packages, return_type, &output_format, &options);

    Ok(Value::Null)
}

async fn run() -> Result<(), Error> {
    let cmd_def = CliCommandMap::new()
        .insert("acl", acl_commands())
        .insert("datastore", datastore_commands())
        .insert("disk", disk_commands())
        .insert("dns", dns_commands())
        .insert("network", network_commands())
        .insert("node", node_commands())
        .insert("user", user_commands())
        .insert("openid", openid_commands())
        .insert("remote", remote_commands())
        .insert("traffic-control", traffic_control_commands())
        .insert("garbage-collection", garbage_collection_commands())
        .insert("acme", acme_mgmt_cli())
        .insert("cert", cert_mgmt_cli())
        .insert("subscription", subscription_commands())
        .insert("sync-job", sync_job_commands())
        .insert("verify-job", verify_job_commands())
        .insert("task", task_mgmt_cli())
        .insert(
            "pull",
            CliCommand::new(&API_METHOD_PULL_DATASTORE)
                .arg_param(&["remote", "remote-store", "local-store"])
                .completion_cb(
                    "local-store",
                    pbs_config::datastore::complete_datastore_name,
                )
                .completion_cb("remote", pbs_config::remote::complete_remote_name)
                .completion_cb("remote-store", complete_remote_datastore_name)
                .completion_cb("group_filter", complete_remote_datastore_group_filter),
        )
        .insert(
            "verify",
            CliCommand::new(&API_METHOD_VERIFY)
                .arg_param(&["store"])
                .completion_cb("store", pbs_config::datastore::complete_datastore_name),
        )
        .insert("report", CliCommand::new(&API_METHOD_REPORT))
        .insert("versions", CliCommand::new(&API_METHOD_GET_VERSIONS));

    let args: Vec<String> = std::env::args().take(2).collect();
    let avoid_init = args.len() >= 2 && (args[1] == "bashcomplete" || args[1] == "printdoc");

    if !avoid_init {
        let backup_user = pbs_config::backup_user()?;
        let file_opts = CreateOptions::new()
            .owner(backup_user.uid)
            .group(backup_user.gid);
        proxmox_rest_server::init_worker_tasks(
            pbs_buildcfg::PROXMOX_BACKUP_LOG_DIR_M!().into(),
            file_opts,
        )?;

        let mut commando_sock = proxmox_rest_server::CommandSocket::new(
            proxmox_rest_server::our_ctrl_sock(),
            backup_user.gid,
        );
        proxmox_rest_server::register_task_control_commands(&mut commando_sock)?;
        commando_sock.spawn()?;
    }

    let mut rpcenv = CliEnvironment::new();
    rpcenv.set_auth_id(Some(String::from("root@pam")));

    run_async_cli_command(cmd_def, rpcenv).await; // this call exit(-1) on error

    Ok(())
}

fn main() -> Result<(), Error> {
    proxmox_backup::tools::setup_safe_path_env();

    proxmox_async::runtime::main(run())
}

fn get_sync_job(id: &str) -> Result<SyncJobConfig, Error> {
    let (config, _digest) = sync::config()?;

    config.lookup("sync", id)
}

fn get_remote(param: &HashMap<String, String>) -> Option<String> {
    param.get("remote").map(|r| r.to_owned()).or_else(|| {
        if let Some(id) = param.get("id") {
            if let Ok(job) = get_sync_job(id) {
                return Some(job.remote);
            }
        }
        None
    })
}

fn get_remote_store(param: &HashMap<String, String>) -> Option<(String, String)> {
    let mut job: Option<SyncJobConfig> = None;

    let remote = param.get("remote").map(|r| r.to_owned()).or_else(|| {
        if let Some(id) = param.get("id") {
            job = get_sync_job(id).ok();
            if let Some(ref job) = job {
                return Some(job.remote.clone());
            }
        }
        None
    });

    if let Some(remote) = remote {
        let store = param
            .get("remote-store")
            .map(|r| r.to_owned())
            .or_else(|| job.map(|job| job.remote_store));

        if let Some(store) = store {
            return Some((remote, store));
        }
    }

    None
}

// shell completion helper
pub fn complete_remote_datastore_name(_arg: &str, param: &HashMap<String, String>) -> Vec<String> {
    let mut list = Vec::new();

    if let Some(remote) = get_remote(param) {
        if let Ok(data) = proxmox_async::runtime::block_on(async move {
            crate::api2::config::remote::scan_remote_datastores(remote).await
        }) {
            for item in data {
                list.push(item.store);
            }
        }
    }

    list
}

// shell completion helper
pub fn complete_remote_datastore_group(_arg: &str, param: &HashMap<String, String>) -> Vec<String> {
    let mut list = Vec::new();

    if let Some((remote, remote_store)) = get_remote_store(param) {
        if let Ok(data) = proxmox_async::runtime::block_on(async move {
            crate::api2::config::remote::scan_remote_groups(remote.clone(), remote_store.clone())
                .await
        }) {
            for item in data {
                list.push(format!("{}/{}", item.backup.ty, item.backup.id));
            }
        }
    }

    list
}

// shell completion helper
pub fn complete_remote_datastore_group_filter(
    _arg: &str,
    param: &HashMap<String, String>,
) -> Vec<String> {
    let mut list = vec![
        "regex:".to_string(),
        "type:ct".to_string(),
        "type:host".to_string(),
        "type:vm".to_string(),
    ];

    list.extend(
        complete_remote_datastore_group(_arg, param)
            .iter()
            .map(|group| format!("group:{}", group)),
    );

    list
}
