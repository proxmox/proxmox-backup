use proxmox::list_subdirs_api_method;

use anyhow::{Error};
use serde_json::{json, Value};

use proxmox::api::{
    api,
    ApiMethod,
    Permission,
    Router,
    RpcEnvironment,
    SubdirMap,
};

use crate::api2::types::{
    DATASTORE_SCHEMA,
    RRDMode,
    RRDTimeFrameResolution,
    TaskListItem,
    TaskStateType,
    Userid,
};

use crate::server;
use crate::backup::{DataStore};
use crate::config::datastore;
use crate::tools::statistics::{linear_regression};
use crate::config::cached_user_info::CachedUserInfo;
use crate::config::acl::{
    PRIV_SYS_AUDIT,
    PRIV_DATASTORE_AUDIT,
    PRIV_DATASTORE_BACKUP,
};

#[api(
    returns: {
        description: "Lists the Status of the Datastores.",
        type: Array,
        items: {
            description: "Status of a Datastore",
            type: Object,
            properties: {
                store: {
                    schema: DATASTORE_SCHEMA,
                },
                total: {
                    type: Integer,
                    description: "The Size of the underlying storage in bytes",
                },
                used: {
                    type: Integer,
                    description: "The used bytes of the underlying storage",
                },
                avail: {
                    type: Integer,
                    description: "The available bytes of the underlying storage",
                },
                history: {
                    type: Array,
                    description: "A list of usages of the past (last Month).",
                    items: {
                        type: Number,
                        description: "The usage of a time in the past. Either null or between 0.0 and 1.0.",
                    }
                },
                "estimated-full-date": {
                    type: Integer,
                    optional: true,
                    description: "Estimation of the UNIX epoch when the storage will be full.\
                        This is calculated via a simple Linear Regression (Least Squares)\
                        of RRD data of the last Month. Missing if there are not enough data points yet.\
                        If the estimate lies in the past, the usage is decreasing.",
                },
            },
        },
    },
    access: {
        permission: &Permission::Anybody,
    },
)]
/// List Datastore usages and estimates
fn datastore_status(
    _param: Value,
    _info: &ApiMethod,
    rpcenv: &mut dyn RpcEnvironment,
    ) -> Result<Value, Error> {

    let (config, _digest) = datastore::config()?;

    let userid: Userid = rpcenv.get_user().unwrap().parse()?;
    let user_info = CachedUserInfo::new()?;

    let mut list = Vec::new();

    for (store, (_, _)) in &config.sections {
        let user_privs = user_info.lookup_privs(&userid, &["datastore", &store]);
        let allowed = (user_privs & (PRIV_DATASTORE_AUDIT| PRIV_DATASTORE_BACKUP)) != 0;
        if !allowed {
            continue;
        }

        let datastore = DataStore::lookup_datastore(&store)?;
        let status = crate::tools::disks::disk_usage(&datastore.base_path())?;

        let mut entry = json!({
            "store": store,
            "total": status.total,
            "used": status.used,
            "avail": status.avail,
        });

        let rrd_dir = format!("datastore/{}", store);
        let now = proxmox::tools::time::epoch_f64();
        let rrd_resolution = RRDTimeFrameResolution::Month;
        let rrd_mode = RRDMode::Average;

        let total_res = crate::rrd::extract_cached_data(
            &rrd_dir,
            "total",
            now,
            rrd_resolution,
            rrd_mode,
        );

        let used_res = crate::rrd::extract_cached_data(
            &rrd_dir,
            "used",
            now,
            rrd_resolution,
            rrd_mode,
        );

        match (total_res, used_res) {
            (Some((start, reso, total_list)), Some((_, _, used_list))) => {
                let mut usage_list: Vec<f64> = Vec::new();
                let mut time_list: Vec<u64> = Vec::new();
                let mut history = Vec::new();

                for (idx, used) in used_list.iter().enumerate() {
                    let total = if idx < total_list.len() {
                        total_list[idx]
                    } else {
                        None
                    };

                    match (total, used) {
                        (Some(total), Some(used)) if total != 0.0 => {
                            time_list.push(start + (idx as u64)*reso);
                            let usage = used/total;
                            usage_list.push(usage);
                            history.push(json!(usage));
                        },
                        _ => {
                            history.push(json!(null))
                        }
                    }
                }

                entry["history"] = history.into();

                // we skip the calculation for datastores with not enough data
                if usage_list.len() >= 7 {
                    if let Some((a,b)) = linear_regression(&time_list, &usage_list) {
                        if b != 0.0 {
                            let estimate = (1.0 - a) / b;
                            entry["estimated-full-date"] = Value::from(estimate.floor() as u64);
                        } else {
                            entry["estimated-full-date"] = Value::from(0);
                        }
                    }
                }
            },
            _ => {},
        }

        list.push(entry);
    }

    Ok(list.into())
}

#[api(
    input: {
        properties: {
            since: {
                type: i64,
                description: "Only list tasks since this UNIX epoch.",
                optional: true,
            },
            typefilter: {
                optional: true,
                type: String,
                description: "Only list tasks, whose type contains this string.",
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
    since: Option<i64>,
    typefilter: Option<String>,
    statusfilter: Option<Vec<TaskStateType>>,
    _param: Value,
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<Vec<TaskListItem>, Error> {

    let userid: Userid = rpcenv.get_user().unwrap().parse()?;
    let user_info = CachedUserInfo::new()?;
    let user_privs = user_info.lookup_privs(&userid, &["system", "tasks"]);

    let list_all = (user_privs & PRIV_SYS_AUDIT) != 0;
    let since = since.unwrap_or_else(|| 0);

    let list: Vec<TaskListItem> = server::TaskListInfoIterator::new(false)?
        .take_while(|info| {
            match info {
                Ok(info) => info.upid.starttime > since,
                Err(_) => false
            }
        })
        .filter_map(|info| {
            match info {
                Ok(info) => {
                    if list_all || info.upid.userid == userid {
                        if let Some(filter) = &typefilter {
                            if !info.upid.worker_type.contains(filter) {
                                return None;
                            }
                        }

                        if let Some(filters) = &statusfilter {
                            if let Some(state) = &info.state {
                                let statetype = match state {
                                    server::TaskState::OK { .. } => TaskStateType::OK,
                                    server::TaskState::Unknown { .. } => TaskStateType::Unknown,
                                    server::TaskState::Error { .. } => TaskStateType::Error,
                                    server::TaskState::Warning { .. } => TaskStateType::Warning,
                                };

                                if !filters.contains(&statetype) {
                                    return None;
                                }
                            }
                        }

                        Some(Ok(TaskListItem::from(info)))
                    } else {
                        None
                    }
                }
                Err(err) => Some(Err(err))
            }
        })
        .collect::<Result<Vec<TaskListItem>, Error>>()?;

    Ok(list.into())
}

const SUBDIRS: SubdirMap = &[
    ("datastore-usage", &Router::new().get(&API_METHOD_DATASTORE_STATUS)),
    ("tasks", &Router::new().get(&API_METHOD_LIST_TASKS)),
];

pub const ROUTER: Router = Router::new()
    .get(&list_subdirs_api_method!(SUBDIRS))
    .subdirs(SUBDIRS);
