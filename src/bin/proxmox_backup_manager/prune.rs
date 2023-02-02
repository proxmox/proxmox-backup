use std::collections::HashMap;

use anyhow::Error;
use serde::Deserialize;
use serde_json::Value;

use proxmox_router::{cli::*, ApiHandler, RpcEnvironment};
use proxmox_schema::api;

use pbs_api_types::{DataStoreConfig, PruneJobConfig, PruneJobOptions, JOB_ID_SCHEMA};
use pbs_config::prune;

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
/// List all prune jobs
fn list_prune_jobs(param: Value, rpcenv: &mut dyn RpcEnvironment) -> Result<Value, Error> {
    let output_format = get_output_format(&param);

    let info = &api2::config::prune::API_METHOD_LIST_PRUNE_JOBS;
    let mut data = match info.handler {
        ApiHandler::Sync(handler) => (handler)(param, info, rpcenv)?,
        _ => unreachable!(),
    };

    let options = default_table_format_options()
        .column(ColumnConfig::new("id"))
        .column(ColumnConfig::new("disable"))
        .column(ColumnConfig::new("store"))
        .column(ColumnConfig::new("ns"))
        .column(ColumnConfig::new("schedule"))
        .column(ColumnConfig::new("max-depth"))
        .column(ColumnConfig::new("keep-last"))
        .column(ColumnConfig::new("keep-hourly"))
        .column(ColumnConfig::new("keep-daily"))
        .column(ColumnConfig::new("keep-weekly"))
        .column(ColumnConfig::new("keep-monthly"))
        .column(ColumnConfig::new("keep-yearly"));

    format_and_print_result_full(&mut data, &info.returns, &output_format, &options);

    Ok(Value::Null)
}

#[api(
    input: {
        properties: {
            id: {
                schema: JOB_ID_SCHEMA,
            },
            "output-format": {
                schema: OUTPUT_FORMAT,
                optional: true,
            },
        }
    }
)]
/// Show prune job configuration
fn show_prune_job(param: Value, rpcenv: &mut dyn RpcEnvironment) -> Result<Value, Error> {
    let output_format = get_output_format(&param);

    let info = &api2::config::prune::API_METHOD_READ_PRUNE_JOB;
    let mut data = match info.handler {
        ApiHandler::Sync(handler) => (handler)(param, info, rpcenv)?,
        _ => unreachable!(),
    };

    let options = default_table_format_options();
    format_and_print_result_full(&mut data, &info.returns, &output_format, &options);

    Ok(Value::Null)
}

#[api(
    input: {
        properties: {
            id: {
                schema: JOB_ID_SCHEMA,
            },
            "output-format": {
                schema: OUTPUT_FORMAT,
                optional: true,
            },
        }
    }
)]
/// Run the specified prune job
async fn run_prune_job(param: Value) -> Result<Value, Error> {
    crate::run_job("prune", param).await
}

pub fn prune_job_commands() -> CommandLineInterface {
    let cmd_def = CliCommandMap::new()
        .insert("list", CliCommand::new(&API_METHOD_LIST_PRUNE_JOBS))
        .insert(
            "show",
            CliCommand::new(&API_METHOD_SHOW_PRUNE_JOB)
                .arg_param(&["id"])
                .completion_cb("id", pbs_config::prune::complete_prune_job_id),
        )
        .insert(
            "create",
            CliCommand::new(&api2::config::prune::API_METHOD_CREATE_PRUNE_JOB)
                .arg_param(&["id"])
                .completion_cb("id", pbs_config::prune::complete_prune_job_id)
                .completion_cb("schedule", pbs_config::datastore::complete_calendar_event)
                .completion_cb("store", pbs_config::datastore::complete_datastore_name)
                .completion_cb("ns", complete_prune_local_datastore_namespace),
        )
        .insert(
            "update",
            CliCommand::new(&api2::config::prune::API_METHOD_UPDATE_PRUNE_JOB)
                .arg_param(&["id"])
                .completion_cb("id", pbs_config::prune::complete_prune_job_id)
                .completion_cb("schedule", pbs_config::datastore::complete_calendar_event)
                .completion_cb("store", pbs_config::datastore::complete_datastore_name)
                .completion_cb("ns", complete_prune_local_datastore_namespace),
        )
        .insert(
            "run",
            CliCommand::new(&API_METHOD_RUN_PRUNE_JOB)
                .arg_param(&["id"])
                .completion_cb("id", pbs_config::prune::complete_prune_job_id),
        )
        .insert(
            "remove",
            CliCommand::new(&api2::config::prune::API_METHOD_DELETE_PRUNE_JOB)
                .arg_param(&["id"])
                .completion_cb("id", pbs_config::prune::complete_prune_job_id),
        );

    cmd_def.into()
}

// shell completion helper
fn complete_prune_local_datastore_namespace(
    _arg: &str,
    param: &HashMap<String, String>,
) -> Vec<String> {
    let mut list = Vec::new();
    let mut rpcenv = CliEnvironment::new();
    rpcenv.set_auth_id(Some(String::from("root@pam")));

    let mut job: Option<PruneJobConfig> = None;

    let store = param.get("store").map(|r| r.to_owned()).or_else(|| {
        if let Some(id) = param.get("id") {
            job = get_prune_job(id).ok();
            if let Some(ref job) = job {
                return Some(job.store.clone());
            }
        }
        None
    });

    if let Some(store) = store {
        if let Ok(data) =
            crate::api2::admin::namespace::list_namespaces(store, None, None, &mut rpcenv)
        {
            for item in data {
                list.push(item.ns.name());
            }
        }
    }

    list
}

fn get_prune_job(id: &str) -> Result<PruneJobConfig, Error> {
    let (config, _digest) = prune::config()?;

    config.lookup("prune", id)
}

pub(crate) fn update_to_prune_jobs_config() -> Result<(), Error> {
    use pbs_config::datastore;

    let _prune_lock = prune::lock_config()?;
    let _datastore_lock = datastore::lock_config()?;

    let (mut data, _digest) = prune::config()?;
    let (mut storeconfig, _digest) = datastore::config()?;

    for (store, entry) in storeconfig.sections.iter_mut() {
        let ty = &entry.0;

        if ty != "datastore" {
            continue;
        }

        let mut config = match DataStoreConfig::deserialize(&entry.1) {
            Ok(c) => c,
            Err(err) => {
                eprintln!("failed to parse config of store {store}: {err}");
                continue;
            }
        };

        let options = PruneJobOptions {
            keep: std::mem::take(&mut config.keep),
            ..Default::default()
        };

        let schedule = config.prune_schedule.take();

        entry.1 = serde_json::to_value(config)?;

        let schedule = match schedule {
            Some(s) => s,
            None => {
                if options.keeps_something() {
                    eprintln!(
                        "dropping prune job without schedule from datastore '{store}' in datastore.cfg"
                    );
                } else {
                    eprintln!("ignoring empty prune job of datastore '{store}' in datastore.cfg");
                }
                continue;
            }
        };

        let mut id = format!("storeconfig-{store}");
        id.truncate(32);
        if data.sections.contains_key(&id) {
            eprintln!("skipping existing converted prune job for datastore '{store}': {id}");
            continue;
        }

        if !options.keeps_something() {
            eprintln!("dropping empty prune job of datastore '{store}' in datastore.cfg");
            continue;
        }

        let prune_config = PruneJobConfig {
            id: id.clone(),
            store: store.clone(),
            disable: false,
            comment: None,
            schedule,
            options,
        };

        let prune_config = serde_json::to_value(prune_config)?;

        data.sections
            .insert(id, ("prune".to_string(), prune_config));

        eprintln!(
            "migrating prune job of datastore '{store}' from datastore.cfg to prune.cfg jobs"
        );
    }

    prune::save_config(&data)?;
    datastore::save_config(&storeconfig)?;

    Ok(())
}
