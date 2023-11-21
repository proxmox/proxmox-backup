use std::fs::File;
use std::io::{BufRead, BufReader};

use anyhow::{bail, Error};
use futures::FutureExt;
use http::request::Parts;
use http::{header, Response, StatusCode};
use hyper::Body;
use serde_json::{json, Value};

use proxmox_async::stream::AsyncReaderStream;
use proxmox_router::{
    list_subdirs_api_method, ApiHandler, ApiMethod, ApiResponseFuture, Permission, Router,
    RpcEnvironment, SubdirMap,
};
use proxmox_schema::{api, BooleanSchema, IntegerSchema, ObjectSchema, Schema};
use proxmox_sortable_macro::sortable;

use pbs_api_types::{
    Authid, TaskListItem, TaskStateType, Tokenname, Userid, DATASTORE_SCHEMA, NODE_SCHEMA,
    PRIV_DATASTORE_MODIFY, PRIV_DATASTORE_VERIFY, PRIV_SYS_AUDIT, PRIV_SYS_MODIFY,
    SYNC_JOB_WORKER_ID_REGEX, UPID, UPID_SCHEMA, VERIFICATION_JOB_WORKER_ID_REGEX,
};

use crate::api2::pull::check_pull_privs;

use pbs_config::CachedUserInfo;
use proxmox_rest_server::{upid_log_path, upid_read_status, TaskListInfoIterator, TaskState};

pub const START_PARAM_SCHEMA: Schema =
    IntegerSchema::new("Start at this line when reading the tasklog")
        .minimum(0)
        .default(0)
        .schema();

pub const LIMIT_PARAM_SCHEMA: Schema = IntegerSchema::new(
    "The amount of lines to read from the tasklog. \
         Setting this parameter to 0 will return all lines until the end of the file.",
)
.minimum(0)
.default(50)
.schema();

pub const DOWNLOAD_PARAM_SCHEMA: Schema = BooleanSchema::new(
    "Whether the tasklog file should be downloaded. \
        This parameter can't be used in conjunction with other parameters",
)
.default(false)
.schema();

pub const TEST_STATUS_PARAM_SCHEMA: Schema =
    BooleanSchema::new("Test task status, and set result attribute \"active\" accordingly.")
        .schema();

// matches respective job execution privileges
fn check_job_privs(auth_id: &Authid, user_info: &CachedUserInfo, upid: &UPID) -> Result<(), Error> {
    match (upid.worker_type.as_str(), &upid.worker_id) {
        // FIXME: parse namespace here?
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
                    let remote_str = remote.as_str();
                    return check_pull_privs(
                        auth_id,
                        local_store.as_str(),
                        local_ns,
                        (remote_str != "-").then_some(remote_str),
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
        ("prune", Some(workerid)) | ("prunejob", Some(workerid)) => {
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
        | ("prunejob", Some(workerid))
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
    pbs_tools::json::required_string_param(param, "upid")?.parse::<UPID>()
}

#[sortable]
pub const API_METHOD_READ_TASK_LOG: ApiMethod = ApiMethod::new(
    &ApiHandler::AsyncHttp(&read_task_log),
    &ObjectSchema::new(
        "Read the task log",
        &sorted!([
            ("node", false, &NODE_SCHEMA),
            ("upid", false, &UPID_SCHEMA),
            ("start", true, &START_PARAM_SCHEMA),
            ("limit", true, &LIMIT_PARAM_SCHEMA),
            ("download", true, &DOWNLOAD_PARAM_SCHEMA),
            ("test-status", true, &TEST_STATUS_PARAM_SCHEMA)
        ]),
    ),
)
.access(
    Some("Users can access their own tasks, or need Sys.Audit on /system/tasks."),
    &Permission::Anybody,
);
fn read_task_log(
    _parts: Parts,
    _req_body: Body,
    param: Value,
    _info: &ApiMethod,
    rpcenv: Box<dyn RpcEnvironment>,
) -> ApiResponseFuture {
    async move {
        let upid: UPID = extract_upid(&param)?;
        let auth_id: Authid = rpcenv.get_auth_id().unwrap().parse()?;
        check_task_access(&auth_id, &upid)?;

        let download = param["download"].as_bool().unwrap_or(false);
        let path = upid_log_path(&upid)?;

        if download {
            if !param["start"].is_null()
                || !param["limit"].is_null()
                || !param["test-status"].is_null()
            {
                bail!("Parameter 'download' cannot be used with other parameters");
            }

            let header_disp = format!(
                "attachment; filename=task-{}-{}-{}.log",
                upid.node,
                upid.worker_type,
                proxmox_time::epoch_to_rfc3339_utc(upid.starttime)?
            );
            let stream = AsyncReaderStream::new(tokio::fs::File::open(path).await?);

            return Ok(Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, "text/plain")
                .header(header::CONTENT_DISPOSITION, &header_disp)
                .body(Body::wrap_stream(stream))
                .unwrap());
        }
        let start = param["start"].as_u64().unwrap_or(0);
        let mut limit = param["limit"].as_u64().unwrap_or(50);
        let test_status = param["test-status"].as_bool().unwrap_or(false);

        let file = File::open(path)?;

        let mut count: u64 = 0;
        let mut lines: Vec<Value> = vec![];
        let read_until_end = limit == 0;

        for line in BufReader::new(file).lines() {
            match line {
                Ok(line) => {
                    count += 1;
                    if count < start {
                        continue;
                    };
                    if !read_until_end {
                        if limit == 0 {
                            continue;
                        };
                        limit -= 1;
                    }

                    lines.push(json!({ "n": count, "t": line }));
                }
                Err(err) => {
                    log::error!("reading task log failed: {}", err);
                    break;
                }
            }
        }

        let mut json = json!({
            "data": lines,
            "total": count,
            "success": 1,
        });

        if test_status {
            let active = proxmox_rest_server::worker_is_active(&upid).await?;
            json["active"] = Value::from(active);
        }

        Ok(Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(json.to_string()))
            .unwrap())
    }
    .boxed()
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
                    // we reached the tasks that ended before our 'since' so we can stop iterating
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
