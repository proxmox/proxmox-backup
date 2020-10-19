use std::fs::File;
use std::io::{BufRead, BufReader};

use anyhow::{Error};
use serde_json::{json, Value};

use proxmox::api::{api, Router, RpcEnvironment, Permission};
use proxmox::api::router::SubdirMap;
use proxmox::{identity, list_subdirs_api_method, sortable};

use crate::tools;
use crate::api2::types::*;
use crate::server::{self, UPID, TaskState, TaskListInfoIterator};
use crate::config::acl::{PRIV_SYS_AUDIT, PRIV_SYS_MODIFY};
use crate::config::cached_user_info::CachedUserInfo;


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
        description: "Task status information.",
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
        description: "Users can access their own tasks, or need Sys.Audit on /system/tasks.",
        permission: &Permission::Anybody,
    },
)]
/// Get task status.
async fn get_task_status(
    param: Value,
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<Value, Error> {

    let upid = extract_upid(&param)?;

    let userid: Userid = rpcenv.get_user().unwrap().parse()?;

    if userid != upid.userid {
        let user_info = CachedUserInfo::new()?;
        user_info.check_privs(&userid, &["system", "tasks"], PRIV_SYS_AUDIT, false)?;
    }

    let mut result = json!({
        "upid": param["upid"],
        "node": upid.node,
        "pid": upid.pid,
        "pstart": upid.pstart,
        "starttime": upid.starttime,
        "type": upid.worker_type,
        "id": upid.worker_id,
        "user": upid.userid,
    });

    if crate::server::worker_is_active(&upid).await? {
        result["status"] = Value::from("running");
    } else {
        let exitstatus = crate::server::upid_read_status(&upid).unwrap_or(TaskState::Unknown { endtime: 0 });
        result["status"] = Value::from("stopped");
        result["exitstatus"] = Value::from(exitstatus.to_string());
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
        description: "Users can access there own tasks, or need Sys.Audit on /system/tasks.",
        permission: &Permission::Anybody,
    },
)]
/// Read task log.
async fn read_task_log(
    param: Value,
    mut rpcenv: &mut dyn RpcEnvironment,
) -> Result<Value, Error> {

    let upid = extract_upid(&param)?;

    let userid: Userid = rpcenv.get_user().unwrap().parse()?;

    if userid != upid.userid {
        let user_info = CachedUserInfo::new()?;
        user_info.check_privs(&userid, &["system", "tasks"], PRIV_SYS_AUDIT, false)?;
    }

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

    rpcenv["total"] = Value::from(count);

    if test_status {
        let active = crate::server::worker_is_active(&upid).await?;
        rpcenv["active"] = Value::from(active);
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
        description: "Users can stop there own tasks, or need Sys.Modify on /system/tasks.",
        permission: &Permission::Anybody,
    },
)]
/// Try to stop a task.
fn stop_task(
    param: Value,
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<Value, Error> {

    let upid = extract_upid(&param)?;

    let userid: Userid = rpcenv.get_user().unwrap().parse()?;

    if userid != upid.userid {
        let user_info = CachedUserInfo::new()?;
        user_info.check_privs(&userid, &["system", "tasks"], PRIV_SYS_MODIFY, false)?;
    }

    server::abort_worker_async(upid);

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
                default: false,
            },
            errors: {
                type: bool,
                description: "Only list erroneous tasks.",
                optional:true,
                default: false,
            },
            userfilter: {
                optional: true,
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
        description: "Users can only see there own tasks, unless the have Sys.Audit on /system/tasks.",
        permission: &Permission::Anybody,
    },
)]
/// List tasks.
pub fn list_tasks(
    start: u64,
    limit: u64,
    errors: bool,
    running: bool,
    userfilter: Option<String>,
    param: Value,
    mut rpcenv: &mut dyn RpcEnvironment,
) -> Result<Vec<TaskListItem>, Error> {

    let userid: Userid = rpcenv.get_user().unwrap().parse()?;
    let user_info = CachedUserInfo::new()?;
    let user_privs = user_info.lookup_privs(&userid, &["system", "tasks"]);

    let list_all = (user_privs & PRIV_SYS_AUDIT) != 0;

    let store = param["store"].as_str();

    let list = TaskListInfoIterator::new(running)?;

    let result: Vec<TaskListItem> = list
        .take_while(|info| !info.is_err())
        .filter_map(|info| {
        let info = match info {
            Ok(info) => info,
            Err(_) => return None,
        };

        if !list_all && info.upid.userid != userid { return None; }

        if let Some(userid) = &userfilter {
            if !info.upid.userid.as_str().contains(userid) { return None; }
        }

        if let Some(store) = store {
            // Note: useful to select all tasks spawned by proxmox-backup-client
            let worker_id = match &info.upid.worker_id {
                Some(w) => w,
                None => return None, // skip
            };

            if info.upid.worker_type == "backup" || info.upid.worker_type == "restore" ||
                info.upid.worker_type == "prune"
            {
                let prefix = format!("{}_", store);
                if !worker_id.starts_with(&prefix) { return None; }
            } else if info.upid.worker_type == "garbage_collection" {
                if worker_id != store { return None; }
            } else {
                return None; // skip
            }
        }

        match info.state {
            Some(_) if running => return None,
            Some(crate::server::TaskState::OK { .. }) if errors => return None,
            _ => {},
        }

        Some(info.into())
    }).skip(start as usize)
        .take(limit as usize)
        .collect();

    let mut count = result.len() + start as usize;
    if result.len() > 0 && result.len() >= limit as usize { // we have a 'virtual' entry as long as we have any new
        count += 1;
    }

    rpcenv["total"] = Value::from(count);

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
