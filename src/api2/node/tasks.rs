use std::fs::File;
use std::io::{BufRead, BufReader};

use failure::*;
use serde_json::{json, Value};

use proxmox::api::{api, Router, RpcEnvironment, Permission};
use proxmox::api::router::SubdirMap;
use proxmox::{identity, list_subdirs_api_method, sortable};

use crate::tools;
use crate::api2::types::*;
use crate::server::{self, UPID};
use crate::config::acl::{PRIV_SYS_AUDIT, PRIV_SYS_MODIFY};

#[api(
    input: {
        properties: {
            node: {
                schema: NODE_SCHEMA,
            },
            upid: {
                schema: UPID_SCHEMA,
            },
        },
    },
    returns: {
        description: "Task status nformation.",
        properties: {
            node: {
                schema: NODE_SCHEMA,
            },
            upid: {
                schema: UPID_SCHEMA,
            },
            pid: {
                type: i64,
                description: "The Unix PID.",
            },
            pstart: {
                type: u64,
                description: "The Unix process start time from `/proc/pid/stat`",
            },
            starttime: {
                type: i64,
                description: "The task start time (Epoch)",
            },
            "type": {
                type: String,
                description: "Worker type (arbitrary ASCII string)",
            },
            id: {
                type: String,
                optional: true,
                description: "Worker ID (arbitrary ASCII string)",
            },
            user: {
                type: String,
                description: "The user who started the task.",
            },
            status: {
                type: String,
                description: "'running' or 'stopped'",
            },
            exitstatus: {
                type: String,
                optional: true,
                description: "'OK', 'Error: <msg>', or 'unkwown'.",
            },
        },
    },
    access: {
        permission: &Permission::Privilege(&[], PRIV_SYS_AUDIT, false),
    },
)]
/// Get task status.
fn get_task_status(
    param: Value,
) -> Result<Value, Error> {

    let upid = extract_upid(&param)?;

    let mut result = json!({
        "upid": param["upid"],
        "node": upid.node,
        "pid": upid.pid,
        "pstart": upid.pstart,
        "starttime": upid.starttime,
        "type": upid.worker_type,
        "id": upid.worker_id,
        "user": upid.username,
    });

    if crate::server::worker_is_active(&upid) {
        result["status"] = Value::from("running");
    } else {
        let exitstatus = crate::server::upid_read_status(&upid).unwrap_or(String::from("unknown"));
        result["status"] = Value::from("stopped");
        result["exitstatus"] = Value::from(exitstatus);
    };

    Ok(result)
}

fn extract_upid(param: &Value) -> Result<UPID, Error> {

    let upid_str = tools::required_string_param(&param, "upid")?;

    upid_str.parse::<UPID>()
}

#[api(
    input: {
        properties: {
            node: {
                schema: NODE_SCHEMA,
            },
            upid: {
                schema: UPID_SCHEMA,
            },
            "test-status": {
                type: bool,
                optional: true,
                description: "Test task status, and set result attribute \"active\" accordingly.",
            },
            start: {
                type: u64,
                optional: true,
                description: "Start at this line.",
                default: 0,
            },
            limit: {
                type: u64,
                optional: true,
                description: "Only list this amount of lines.",
                default: 50,
            },
        },
    },
    access: {
        permission: &Permission::Privilege(&[], PRIV_SYS_AUDIT, false),
    },
)]
/// Read task log.
fn read_task_log(
    param: Value,
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<Value, Error> {

    let upid = extract_upid(&param)?;

    let test_status = param["test-status"].as_bool().unwrap_or(false);

    let start = param["start"].as_u64().unwrap_or(0);
    let mut limit = param["limit"].as_u64().unwrap_or(50);

    let mut count: u64 = 0;

    let path = upid.log_path();

    let file = File::open(path)?;

    let mut lines: Vec<Value> = vec![];

    for line in BufReader::new(file).lines() {
        match line {
            Ok(line) => {
                count += 1;
                if count < start { continue };
	        if limit == 0 { continue };

                lines.push(json!({ "n": count, "t": line }));

                limit -= 1;
            }
            Err(err) => {
                log::error!("reading task log failed: {}", err);
                break;
            }
        }
    }

    rpcenv.set_result_attrib("total", Value::from(count));

    if test_status {
        let active = crate::server::worker_is_active(&upid);
        rpcenv.set_result_attrib("active", Value::from(active));
    }

    Ok(json!(lines))
}

#[api(
    protected: true,
    input: {
        properties: {
            node: {
                schema: NODE_SCHEMA,
            },
            upid: {
                schema: UPID_SCHEMA,
            },
        },
    },
    access: {
        permission: &Permission::Privilege(&[], PRIV_SYS_MODIFY, false),
    },
)]
/// Try to stop a task.
fn stop_task(
    param: Value,
) -> Result<Value, Error> {

    let upid = extract_upid(&param)?;

    if crate::server::worker_is_active(&upid) {
        server::abort_worker_async(upid);
    }

    Ok(Value::Null)
}

#[api(
    input: {
        properties: {
            node: {
                schema: NODE_SCHEMA
            },
            start: {
                type: u64,
                description: "List tasks beginning from this offset.",
                default: 0,
                optional: true,
            },
            limit: {
                type: u64,
                description: "Only list this amount of tasks.",
                default: 50,
                optional: true,
            },
            store: {
                schema: DATASTORE_SCHEMA,
                optional: true,
            },
            running: {
                type: bool,
                description: "Only list running tasks.",
                optional: true,
            },
            errors: {
                type: bool,
                description: "Only list erroneous tasks.",
                optional:true,
            },
            userfilter: {
                optional:true,
                type: String,
                description: "Only list tasks from this user.",
            },
        },
    },
    returns: {
        description: "A list of tasks.",
        type: Array,
        items: { type: TaskListItem },
    },
    access: {
        permission: &Permission::Privilege(&[], PRIV_SYS_AUDIT, false),
    },
)]
/// List tasks.
pub fn list_tasks(
    param: Value,
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<Vec<TaskListItem>, Error> {

    let start = param["start"].as_u64().unwrap_or(0);
    let limit = param["limit"].as_u64().unwrap_or(50);
    let errors = param["errors"].as_bool().unwrap_or(false);
    let running = param["running"].as_bool().unwrap_or(false);

    let store = param["store"].as_str();

    let userfilter = param["userfilter"].as_str();

    let list = server::read_task_list()?;

    let mut result = vec![];

    let mut count = 0;

    for info in list.iter() {
        let mut entry = TaskListItem {
            upid: info.upid_str.clone(),
            node: "localhost".to_string(),
            pid: info.upid.pid as i64,
            pstart: info.upid.pstart,
            starttime: info.upid.starttime,
            worker_type: info.upid.worker_type.clone(),
            worker_id: info.upid.worker_id.clone(),
            user: info.upid.username.clone(),
            endtime: None,
            status: None,
        };

        if let Some(username) = userfilter {
            if !info.upid.username.contains(username) { continue; }
        }

        if let Some(store) = store {
            // Note: useful to select all tasks spawned by proxmox-backup-client
            let worker_id = match &info.upid.worker_id {
                Some(w) => w,
                None => continue, // skip
            };

            if info.upid.worker_type == "backup" || info.upid.worker_type == "restore" ||
                info.upid.worker_type == "prune"
            {
                let prefix = format!("{}_", store);
                if !worker_id.starts_with(&prefix) { continue; }
            } else if info.upid.worker_type == "garbage_collection" {
                if worker_id != store { continue; }
            } else {
                continue; // skip
            }
        }

        if let Some(ref state) = info.state {
            if running { continue; }
            if errors && state.1 == "OK" {
                continue;
            }

            entry.endtime = Some(state.0);
            entry.status = Some(state.1.clone());
        }

        if (count as u64) < start {
            count += 1;
            continue;
        } else {
            count += 1;
        }

        if (result.len() as u64) < limit { result.push(entry); };
    }

    rpcenv.set_result_attrib("total", Value::from(count));

    Ok(result)
}

#[sortable]
const UPID_API_SUBDIRS: SubdirMap = &sorted!([
    (
        "log", &Router::new()
            .get(&API_METHOD_READ_TASK_LOG)
    ),
    (
        "status", &Router::new()
            .get(&API_METHOD_GET_TASK_STATUS)
    )
]);

pub const UPID_API_ROUTER: Router = Router::new()
    .get(&list_subdirs_api_method!(UPID_API_SUBDIRS))
    .delete(&API_METHOD_STOP_TASK)
    .subdirs(&UPID_API_SUBDIRS);

pub const ROUTER: Router = Router::new()
    .get(&API_METHOD_LIST_TASKS)
    .match_all("upid", &UPID_API_ROUTER);
