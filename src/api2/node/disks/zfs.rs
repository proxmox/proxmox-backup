use anyhow::{bail, Error};
use serde_json::{json, Value};

use proxmox_router::{Permission, Router, RpcEnvironment, RpcEnvironmentType};
use proxmox_schema::api;
use proxmox_sys::{task_error, task_log};

use pbs_api_types::{
    DataStoreConfig, ZfsCompressionType, ZfsRaidLevel, ZpoolListItem, DATASTORE_SCHEMA,
    DISK_ARRAY_SCHEMA, DISK_LIST_SCHEMA, NODE_SCHEMA, PRIV_SYS_AUDIT, PRIV_SYS_MODIFY, UPID_SCHEMA,
    ZFS_ASHIFT_SCHEMA, ZPOOL_NAME_SCHEMA,
};

use crate::tools::disks::{
    parse_zpool_status_config_tree, vdev_list_to_tree, zpool_list, zpool_status, DiskUsageType,
};

use proxmox_rest_server::WorkerTask;

#[api(
    protected: true,
    input: {
        properties: {
            node: {
                schema: NODE_SCHEMA,
            },
        },
    },
    returns: {
        description: "List of zpools.",
        type: Array,
        items: {
            type: ZpoolListItem,
        },
    },
    access: {
        permission: &Permission::Privilege(&["system", "disks"], PRIV_SYS_AUDIT, false),
    },
)]
/// List zfs pools.
pub fn list_zpools() -> Result<Vec<ZpoolListItem>, Error> {
    let data = zpool_list(None, false)?;

    let mut list = Vec::new();

    for item in data {
        if let Some(usage) = item.usage {
            list.push(ZpoolListItem {
                name: item.name,
                health: item.health,
                size: usage.size,
                alloc: usage.alloc,
                free: usage.free,
                frag: usage.frag,
                dedup: usage.dedup,
            });
        }
    }

    Ok(list)
}

#[api(
    protected: true,
    input: {
        properties: {
            node: {
                schema: NODE_SCHEMA,
            },
            name: {
                schema: ZPOOL_NAME_SCHEMA,
            },
        },
    },
    returns: {
        description: "zpool vdev tree with status",
        properties: {

        },
    },
    access: {
        permission: &Permission::Privilege(&["system", "disks"], PRIV_SYS_AUDIT, false),
    },
)]
/// Get zpool status details.
pub fn zpool_details(name: String) -> Result<Value, Error> {
    let key_value_list = zpool_status(&name)?;

    let config = match key_value_list.iter().find(|(k, _)| k == "config") {
        Some((_, v)) => v,
        None => bail!("got zpool status without config key"),
    };

    let vdev_list = parse_zpool_status_config_tree(config)?;
    let mut tree = vdev_list_to_tree(&vdev_list)?;

    for (k, v) in key_value_list {
        if k != "config" {
            tree[k] = v.into();
        }
    }

    tree["name"] = tree
        .as_object_mut()
        .unwrap()
        .remove("pool")
        .unwrap_or_else(|| name.into());

    Ok(tree)
}

#[api(
    protected: true,
    input: {
        properties: {
            node: {
                schema: NODE_SCHEMA,
            },
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
    returns: {
        schema: UPID_SCHEMA,
    },
    access: {
        permission: &Permission::Privilege(&["system", "disks"], PRIV_SYS_MODIFY, false),
    },
)]
/// Create a new ZFS pool. Will be mounted under `/mnt/datastore/<name>`.
pub fn create_zpool(
    name: String,
    devices: String,
    raidlevel: ZfsRaidLevel,
    compression: Option<String>,
    ashift: Option<usize>,
    add_datastore: Option<bool>,
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<String, Error> {
    let to_stdout = rpcenv.env_type() == RpcEnvironmentType::CLI;

    let auth_id = rpcenv.get_auth_id().unwrap();

    let add_datastore = add_datastore.unwrap_or(false);

    let ashift = ashift.unwrap_or(12);

    let devices_text = devices.clone();
    let devices = DISK_ARRAY_SCHEMA.parse_property_string(&devices)?;
    let devices: Vec<String> = devices
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();

    let disk_map = crate::tools::disks::DiskUsageQuery::new().query()?;
    for disk in devices.iter() {
        match disk_map.get(disk) {
            Some(info) => {
                if info.used != DiskUsageType::Unused {
                    bail!("disk '{}' is already in use.", disk);
                }
            }
            None => {
                bail!("no such disk '{}'", disk);
            }
        }
    }

    let min_disks = match raidlevel {
        ZfsRaidLevel::Single => 1,
        ZfsRaidLevel::Mirror => 2,
        ZfsRaidLevel::Raid10 => 4,
        ZfsRaidLevel::RaidZ => 3,
        ZfsRaidLevel::RaidZ2 => 4,
        ZfsRaidLevel::RaidZ3 => 5,
    };

    // Sanity checks
    if raidlevel == ZfsRaidLevel::Raid10 && devices.len() % 2 != 0 {
        bail!("Raid10 needs an even number of disks.");
    }

    if raidlevel == ZfsRaidLevel::Single && devices.len() > 1 {
        bail!("Please give only one disk for single disk mode.");
    }

    if devices.len() < min_disks {
        bail!("{:?} needs at least {} disks.", raidlevel, min_disks);
    }

    let mount_point = format!("/mnt/datastore/{}", &name);

    // check if the default path does exist already and bail if it does
    // otherwise 'zpool create' aborts after partitioning, but before creating the pool
    let default_path = std::path::PathBuf::from(&mount_point);

    match std::fs::metadata(&default_path) {
        Err(_) => {} // path does not exist
        Ok(_) => {
            bail!("path {:?} already exists", default_path);
        }
    }

    let upid_str = WorkerTask::new_thread(
        "zfscreate",
        Some(name.clone()),
        auth_id,
        to_stdout,
        move |worker| {
            task_log!(
                worker,
                "create {:?} zpool '{}' on devices '{}'",
                raidlevel,
                name,
                devices_text
            );

            let mut command = std::process::Command::new("zpool");
            command.args([
                "create",
                "-o",
                &format!("ashift={}", ashift),
                "-m",
                &mount_point,
                &name,
            ]);

            match raidlevel {
                ZfsRaidLevel::Single => {
                    command.arg(&devices[0]);
                }
                ZfsRaidLevel::Mirror => {
                    command.arg("mirror");
                    command.args(devices);
                }
                ZfsRaidLevel::Raid10 => {
                    devices.chunks(2).for_each(|pair| {
                        command.arg("mirror");
                        command.args(pair);
                    });
                }
                ZfsRaidLevel::RaidZ => {
                    command.arg("raidz");
                    command.args(devices);
                }
                ZfsRaidLevel::RaidZ2 => {
                    command.arg("raidz2");
                    command.args(devices);
                }
                ZfsRaidLevel::RaidZ3 => {
                    command.arg("raidz3");
                    command.args(devices);
                }
            }

            task_log!(worker, "# {:?}", command);

            match proxmox_sys::command::run_command(command, None) {
                Ok(output) => task_log!(worker, "{output}"),
                Err(err) => {
                    task_error!(worker, "{err}");
                    bail!("Error during 'zpool create', see task log for more details");
                }
            };

            if std::path::Path::new("/lib/systemd/system/zfs-import@.service").exists() {
                let import_unit = format!(
                    "zfs-import@{}.service",
                    proxmox_sys::systemd::escape_unit(&name, false)
                );
                crate::tools::systemd::enable_unit(&import_unit)?;
            }

            let mut command = std::process::Command::new("zfs");
            command.arg("set");
            if let Some(compression) = compression {
                command.arg(&format!("compression={}", compression));
            }
            command.args(["relatime=on", &name]);
            task_log!(worker, "# {:?}", command);
            match proxmox_sys::command::run_command(command, None) {
                Ok(output) => task_log!(worker, "{output}"),
                Err(err) => {
                    task_error!(worker, "{err}");
                    bail!("Error during 'zfs set', see task log for more details");
                }
            };

            if add_datastore {
                let lock = pbs_config::datastore::lock_config()?;
                let datastore: DataStoreConfig =
                    serde_json::from_value(json!({ "name": name, "path": mount_point }))?;

                let (config, _digest) = pbs_config::datastore::config()?;

                if config.sections.get(&datastore.name).is_some() {
                    bail!("datastore '{}' already exists.", datastore.name);
                }

                crate::api2::config::datastore::do_create_datastore(
                    lock,
                    config,
                    datastore,
                    Some(&worker),
                )?;
            }

            Ok(())
        },
    )?;

    Ok(upid_str)
}

pub const POOL_ROUTER: Router = Router::new().get(&API_METHOD_ZPOOL_DETAILS);

pub const ROUTER: Router = Router::new()
    .get(&API_METHOD_LIST_ZPOOLS)
    .post(&API_METHOD_CREATE_ZPOOL)
    .match_all("name", &POOL_ROUTER);
