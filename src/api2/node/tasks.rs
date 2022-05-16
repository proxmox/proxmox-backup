use std::fs::File;
use std::io::{BufRead, BufReader};

use anyhow::{bail, Error};
use serde_json::{json, Value};

use proxmox_router::{list_subdirs_api_method, Permission, Router, RpcEnvironment, SubdirMap};
use proxmox_schema::api;
use proxmox_sys::sortable;

use pbs_api_types::{
    Authid, TaskListItem, TaskStateType, Tokenname, Userid, DATASTORE_SCHEMA, NODE_SCHEMA,
    PRIV_DATASTORE_MODIFY, PRIV_DATASTORE_VERIFY, PRIV_SYS_AUDIT, PRIV_SYS_MODIFY,
    SYNC_JOB_WORKER_ID_REGEX, UPID, UPID_SCHEMA, VERIFICATION_JOB_WORKER_ID_REGEX,
};

use crate::api2::pull::check_pull_privs;

use pbs_config::CachedUserInfo;
use proxmox_rest_server::{upid_log_path, upid_read_status, TaskListInfoIterator, TaskState};

// matches respective job execution privileges
fn check_job_privs(auth_id: &Authid, user_info: &CachedUserInfo, upid: &UPID) -> Result<(), Error> {
    match (upid.worker_type.as_str(), &upid.worker_id) {
        ("verificationjob", Some(workerid)) => {
            if let Some(captures) = VERIFICATION_JOB_WORKER_ID_REGEX.captures(workerid) {
                if let Some(store) = captures.get(1) {
                    return user_info.check_privs(
                        auth_id,
                        &["datastore", store.as_str()],
                        PRIV_DATASTORE_VERIFY,
                        true,
                    );
                }
            }
        }
        ("syncjob", Some(workerid)) => {
            if let Some(captures) = SYNC_JOB_WORKER_ID_REGEX.captures(workerid) {
                let remote = captures.get(1);
                let remote_store = captures.get(2);
                let local_store = captures.get(3);
                let local_ns = captures.get(4).map(|m| m.as_str());

                if let (Some(remote), Some(remote_store), Some(local_store)) =
                    (remote, remote_store, local_store)
                {
                    return check_pull_privs(
                        auth_id,
                        local_store.as_str(),
                        local_ns,
                        remote.as_str(),
                        remote_store.as_str(),
                        false,
                    );
                }
            }
        }
        ("garbage_collection", Some(workerid)) => {
            return user_info.check_privs(
                auth_id,
                &["datastore", workerid],
                PRIV_DATASTORE_MODIFY,
                true,
            )
        }
        ("prune", Some(workerid)) => {
            let mut acl_path = vec!["datastore"];
            acl_path.extend(workerid.split(':'));
            let acl_path = match acl_path.len() {
                4 => &acl_path[..3],    // contains group as fourth element
                2 | 3 => &acl_path[..], // store + optional NS
                _ => {
                    bail!("invalid worker ID for prune task");
                }
            };

            return user_info.check_privs(auth_id, acl_path, PRIV_DATASTORE_MODIFY, true);
        }
        _ => bail!("not a scheduled job task"),
    };

    bail!("not a scheduled job task");
}

// get the store out of the worker_id
fn check_job_store(upid: &UPID, store: &str) -> bool {
    match (upid.worker_type.as_str(), &upid.worker_id) {
        (workertype, Some(workerid)) if workertype.starts_with("verif") => {
            if let Some(captures) = VERIFICATION_JOB_WORKER_ID_REGEX.captures(workerid) {
                if let Some(jobstore) = captures.get(1) {
                    return store == jobstore.as_str();
                }
            } else {
                return workerid == store;
            }
        }
        ("syncjob", Some(workerid)) => {
            if let Some(captures) = SYNC_JOB_WORKER_ID_REGEX.captures(workerid) {
                if let Some(local_store) = captures.get(3) {
                    return store == local_store.as_str();
                }
            }
        }
        ("prune", Some(workerid))
        | ("backup", Some(workerid))
        | ("garbage_collection", Some(workerid)) => {
            return workerid == store || workerid.starts_with(&format!("{}:", store));
        }
        _ => {}
    };

    false
}

fn check_task_access(auth_id: &Authid, upid: &UPID) -> Result<(), Error> {
    let task_auth_id: Authid = upid.auth_id.parse()?;
    if auth_id == &task_auth_id
        || (task_auth_id.is_token() && &Authid::from(task_auth_id.user().clone()) == auth_id)
    {
        // task owner can always read
        Ok(())
    } else {
        let user_info = CachedUserInfo::new()?;

        // access to all tasks
        // or task == job which the user/token could have configured/manually executed

        user_info
            .check_privs(auth_id, &["system", "tasks"], PRIV_SYS_AUDIT, false)
            .or_else(|_| check_job_privs(auth_id, &user_info, upid))
            .or_else(|_| bail!("task access not allowed"))
    }
}

pub fn tasktype(state: &TaskState) -> TaskStateType {
    match state {
        TaskState::OK { .. } => TaskStateType::OK,
        TaskState::Unknown { .. } => TaskStateType::Unknown,
        TaskState::Error { .. } => TaskStateType::Error,
        TaskState::Warning { .. } => TaskStateType::Warning,
    }
}

fn into_task_list_item(info: proxmox_rest_server::TaskListInfo) -> pbs_api_types::TaskListItem {
    let (endtime, status) = info.state.map_or_else(
        || (None, None),
        |a| (Some(a.endtime()), Some(a.to_string())),
    );

    pbs_api_types::TaskListItem {
        upid: info.upid_str,
        node: "localhost".to_string(),
        pid: info.upid.pid as i64,
        pstart: info.upid.pstart,
        starttime: info.upid.starttime,
        worker_type: info.upid.worker_type,
        worker_id: info.upid.worker_id,
        user: info.upid.auth_id,
        endtime,
        status,
    }
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
                type: Userid,
            },
            tokenid: {
                type: Tokenname,
                optional: true,
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
async fn get_task_status(param: Value, rpcenv: &mut dyn RpcEnvironment) -> Result<Value, Error> {
    let upid = extract_upid(&param)?;

    let auth_id: Authid = rpcenv.get_auth_id().unwrap().parse()?;
    check_task_access(&auth_id, &upid)?;

    let task_auth_id: Authid = upid.auth_id.parse()?;

    let mut result = json!({
        "upid": param["upid"],
        "node": upid.node,
        "pid": upid.pid,
        "pstart": upid.pstart,
        "starttime": upid.starttime,
        "type": upid.worker_type,
        "id": upid.worker_id,
        "user": task_auth_id.user(),
    });

    if task_auth_id.is_token() {
        result["tokenid"] = Value::from(task_auth_id.tokenname().unwrap().as_str());
    }

    if proxmox_rest_server::worker_is_active(&upid).await? {
        result["status"] = Value::from("running");
    } else {
        let exitstatus = upid_read_status(&upid).unwrap_or(TaskState::Unknown { endtime: 0 });
        result["status"] = Value::from("stopped");
        result["exitstatus"] = Value::from(exitstatus.to_string());
    };

    Ok(result)
}

fn extract_upid(param: &Value) -> Result<UPID, Error> {
    let upid_str = pbs_tools::json::required_string_param(param, "upid")?;

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
        description: "Users can access their own tasks, or need Sys.Audit on /system/tasks.",
        permission: &Permission::Anybody,
    },
)]
/// Read task log.
async fn read_task_log(param: Value, rpcenv: &mut dyn RpcEnvironment) -> Result<Value, Error> {
    let upid = extract_upid(&param)?;

    let auth_id: Authid = rpcenv.get_auth_id().unwrap().parse()?;

    check_task_access(&auth_id, &upid)?;

    let test_status = param["test-status"].as_bool().unwrap_or(false);

    let start = param["start"].as_u64().unwrap_or(0);
    let mut limit = param["limit"].as_u64().unwrap_or(50);

    let mut count: u64 = 0;

    let path = upid_log_path(&upid)?;

    let file = File::open(path)?;

    let mut lines: Vec<Value> = vec![];

    for line in BufReader::new(file).lines() {
        match line {
            Ok(line) => {
                count += 1;
                if count < start {
                    continue;
                };
                if limit == 0 {
                    continue;
                };

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
        let active = proxmox_rest_server::worker_is_active(&upid).await?;
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
        description: "Users can stop their own tasks, or need Sys.Modify on /system/tasks.",
        permission: &Permission::Anybody,
    },
)]
/// Try to stop a task.
fn stop_task(param: Value, rpcenv: &mut dyn RpcEnvironment) -> Result<Value, Error> {
    let upid = extract_upid(&param)?;

    let auth_id = rpcenv.get_auth_id().unwrap();

    if auth_id != upid.auth_id {
        let user_info = CachedUserInfo::new()?;
        let auth_id: Authid = auth_id.parse()?;
        user_info.check_privs(&auth_id, &["system", "tasks"], PRIV_SYS_MODIFY, false)?;
    }

    proxmox_rest_server::abort_worker_nowait(upid);

    Ok(Value::Null)
}

#[api(
    streaming: true,
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
                description: "Only list this amount of tasks. (0 means no limit)",
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
            since: {
                type: i64,
                description: "Only list tasks since this UNIX epoch.",
                optional: true,
            },
            until: {
                type: i64,
                description: "Only list tasks until this UNIX epoch.",
                optional: true,
            },
            typefilter: {
                optional: true,
                type: String,
                description: "Only list tasks whose type contains this.",
            },
            statusfilter: {
                optional: true,
                type: Array,
                description: "Only list tasks which have any one of the listed status.",
                items: {
                    type: TaskStateType,
                },
            },
        },
    },
    returns: pbs_api_types::NODE_TASKS_LIST_TASKS_RETURN_TYPE,
    access: {
        description: "Users can only see their own tasks, unless they have Sys.Audit on /system/tasks.",
        permission: &Permission::Anybody,
    },
)]
/// List tasks.
#[allow(clippy::too_many_arguments)]
pub fn list_tasks(
    start: u64,
    limit: u64,
    errors: bool,
    running: bool,
    userfilter: Option<String>,
    since: Option<i64>,
    until: Option<i64>,
    typefilter: Option<String>,
    statusfilter: Option<Vec<TaskStateType>>,
    param: Value,
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<Vec<TaskListItem>, Error> {
    let auth_id: Authid = rpcenv.get_auth_id().unwrap().parse()?;
    let user_info = CachedUserInfo::new()?;
    let user_privs = user_info.lookup_privs(&auth_id, &["system", "tasks"]);

    let list_all = (user_privs & PRIV_SYS_AUDIT) != 0;

    let store = param["store"].as_str();

    let list = TaskListInfoIterator::new(running)?;
    let limit = if limit > 0 {
        limit as usize
    } else {
        usize::MAX
    };

    let mut skipped = 0;
    let mut result: Vec<TaskListItem> = Vec::new();

    for info in list {
        let info = match info {
            Ok(info) => info,
            Err(_) => break,
        };

        if let Some(until) = until {
            if info.upid.starttime > until {
                continue;
            }
        }

        if let Some(since) = since {
            if let Some(ref state) = info.state {
                if state.endtime() < since {
                    // we reached the tasks that ended before our 'since'
                    // so we can stop iterating
                    break;
                }
            }
            if info.upid.starttime < since {
                continue;
            }
        }

        if !list_all && check_task_access(&auth_id, &info.upid).is_err() {
            continue;
        }

        if let Some(needle) = &userfilter {
            if !info.upid.auth_id.to_string().contains(needle) {
                continue;
            }
        }

        if let Some(store) = store {
            if !check_job_store(&info.upid, store) {
                continue;
            }
        }

        if let Some(typefilter) = &typefilter {
            if !info.upid.worker_type.contains(typefilter) {
                continue;
            }
        }

        match (&info.state, &statusfilter) {
            (Some(_), _) if running => continue,
            (Some(TaskState::OK { .. }), _) if errors => continue,
            (Some(state), Some(filters)) => {
                if !filters.contains(&tasktype(state)) {
                    continue;
                }
            }
            (None, Some(_)) => continue,
            _ => {}
        }

        if skipped < start as usize {
            skipped += 1;
            continue;
        }

        result.push(into_task_list_item(info));

        if result.len() >= limit {
            break;
        }
    }

    let mut count = result.len() + start as usize;
    if !result.is_empty() && result.len() >= limit {
        // we have a 'virtual' entry as long as we have any new
        count += 1;
    }

    rpcenv["total"] = Value::from(count);

    Ok(result)
}

#[sortable]
const UPID_API_SUBDIRS: SubdirMap = &sorted!([
    ("log", &Router::new().get(&API_METHOD_READ_TASK_LOG)),
    ("status", &Router::new().get(&API_METHOD_GET_TASK_STATUS))
]);

pub const UPID_API_ROUTER: Router = Router::new()
    .get(&list_subdirs_api_method!(UPID_API_SUBDIRS))
    .delete(&API_METHOD_STOP_TASK)
    .subdirs(UPID_API_SUBDIRS);

pub const ROUTER: Router = Router::new()
    .get(&API_METHOD_LIST_TASKS)
    .match_all("upid", &UPID_API_ROUTER);
