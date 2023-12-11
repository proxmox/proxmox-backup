//! Datastote status

use anyhow::Error;
use serde_json::Value;

use proxmox_router::list_subdirs_api_method;
use proxmox_router::{ApiMethod, Permission, Router, RpcEnvironment, SubdirMap};
use proxmox_schema::api;

use pbs_api_types::{
    Authid, DataStoreStatusListItem, Operation, RRDMode, RRDTimeFrame, PRIV_DATASTORE_AUDIT,
    PRIV_DATASTORE_BACKUP,
};

use pbs_config::CachedUserInfo;
use pbs_datastore::DataStore;

use crate::rrd_cache::extract_rrd_data;
use crate::tools::statistics::linear_regression;

use crate::backup::can_access_any_namespace;

#[api(
    returns: {
        description: "Lists the Status of the Datastores.",
        type: Array,
        items: {
            type: DataStoreStatusListItem,
        },
    },
    access: {
        permission: &Permission::Anybody,
    },
)]
/// List Datastore usages and estimates
pub async fn datastore_status(
    _param: Value,
    _info: &ApiMethod,
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<Vec<DataStoreStatusListItem>, Error> {
    let (config, _digest) = pbs_config::datastore::config()?;

    let auth_id: Authid = rpcenv.get_auth_id().unwrap().parse()?;
    let user_info = CachedUserInfo::new()?;

    let mut list = Vec::new();

    for (store, (_, _)) in &config.sections {
        let user_privs = user_info.lookup_privs(&auth_id, &["datastore", store]);
        let allowed = (user_privs & (PRIV_DATASTORE_AUDIT | PRIV_DATASTORE_BACKUP)) != 0;
        if !allowed {
            if let Ok(datastore) = DataStore::lookup_datastore(store, Some(Operation::Lookup)) {
                if can_access_any_namespace(datastore, &auth_id, &user_info) {
                    list.push(DataStoreStatusListItem::empty(store, None));
                }
            }
            continue;
        }

        let datastore = match DataStore::lookup_datastore(store, Some(Operation::Read)) {
            Ok(datastore) => datastore,
            Err(err) => {
                list.push(DataStoreStatusListItem::empty(store, Some(err.to_string())));
                continue;
            }
        };
        let status = crate::tools::fs::fs_info(datastore.base_path()).await?;

        let mut entry = DataStoreStatusListItem {
            store: store.clone(),
            total: Some(status.total),
            used: Some(status.used),
            avail: Some(status.available),
            history: None,
            history_start: None,
            history_delta: None,
            estimated_full_date: None,
            error: None,
            gc_status: Some(datastore.last_gc_status()),
        };

        let rrd_dir = format!("datastore/{}", store);

        let get_rrd =
            |what: &str| extract_rrd_data(&rrd_dir, what, RRDTimeFrame::Month, RRDMode::Average);

        let total_res = get_rrd("total")?;
        let used_res = get_rrd("used")?;
        let avail_res = get_rrd("available")?;

        if let Some(((total_entry, used), avail)) = total_res.zip(used_res).zip(avail_res) {
            let mut usage_list: Vec<f64> = Vec::new();
            let mut time_list: Vec<u64> = Vec::new();
            let mut history = Vec::new();

            for (idx, used) in used.data.iter().enumerate() {
                let used = match used {
                    Some(used) => used,
                    _ => {
                        history.push(None);
                        continue;
                    }
                };

                let total = if let Some(avail) = avail.get(idx) {
                    avail + used
                } else if let Some(total) = total_entry.get(idx) {
                    total
                } else {
                    history.push(None);
                    continue;
                };

                let usage = used / total;
                time_list.push(total_entry.start + (idx as u64) * total_entry.resolution);
                usage_list.push(usage);
                history.push(Some(usage));
            }

            entry.history_start = Some(total_entry.start);
            entry.history_delta = Some(total_entry.resolution);
            entry.history = Some(history);

            // we skip the calculation for datastores with not enough data
            if usage_list.len() >= 7 {
                entry.estimated_full_date = match linear_regression(&time_list, &usage_list) {
                    Some((a, b)) if b != 0.0 => Some(((1.0 - a) / b).floor() as i64),
                    Some((_, b)) if b == 0.0 => Some(0), // infinite estimate, set to past for gui to detect
                    _ => None,
                };
            }
        }

        list.push(entry);
    }

    Ok(list)
}

const SUBDIRS: SubdirMap = &[(
    "datastore-usage",
    &Router::new().get(&API_METHOD_DATASTORE_STATUS),
)];

pub const ROUTER: Router = Router::new()
    .get(&list_subdirs_api_method!(SUBDIRS))
    .subdirs(SUBDIRS);
