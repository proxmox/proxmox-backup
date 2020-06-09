use proxmox::list_subdirs_api_method;

use anyhow::{Error};
use serde_json::{json, Value};

use proxmox::api::{api, ApiMethod, Router, RpcEnvironment, UserInformation, SubdirMap};

use crate::api2::types::{DATASTORE_SCHEMA, RRDMode, RRDTimeFrameResolution};
use crate::backup::{DataStore};
use crate::config::datastore;
use crate::tools::statistics::{linear_regression};
use crate::config::cached_user_info::CachedUserInfo;
use crate::config::acl::{
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
)]
/// List Datastore usages and estimates
fn datastore_status(
    _param: Value,
    _info: &ApiMethod,
    rpcenv: &mut dyn RpcEnvironment,
    ) -> Result<Value, Error> {

    let (config, _digest) = datastore::config()?;

    let username = rpcenv.get_user().unwrap();
    let user_info = CachedUserInfo::new()?;

    let mut list = Vec::new();

    for (store, (_, _)) in &config.sections {
        let user_privs = user_info.lookup_privs(&username, &["datastore", &store]);
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

        let (times, lists) = crate::rrd::extract_lists(
            &rrd_dir,
            &[ "total", "used", ],
            RRDTimeFrameResolution::Month,
            RRDMode::Average,
        )?;

        if !lists.contains_key("total") || !lists.contains_key("used") {
            // we do not have the info, so we can skip calculating
            continue;
        }

        let mut usage_list: Vec<f64> = Vec::new();
        let mut time_list: Vec<u64> = Vec::new();
        let mut history = Vec::new();

        for (idx, used) in lists["used"].iter().enumerate() {
            let total = if idx < lists["total"].len() {
                lists["total"][idx]
            } else {
                None
            };

            match (total, used) {
                (Some(total), Some(used)) if total != 0.0 => {
                    time_list.push(times[idx]);
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
                }
            }
        }

        list.push(entry);
    }


    Ok(list.into())
}

const SUBDIRS: SubdirMap = &[
    ("datastore-usage", &Router::new().get(&API_METHOD_DATASTORE_STATUS)),
];

pub const ROUTER: Router = Router::new()
    .get(&list_subdirs_api_method!(SUBDIRS))
    .subdirs(SUBDIRS);
