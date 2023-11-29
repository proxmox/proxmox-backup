//! RRD toolkit - create/manage/update proxmox RRD (v2) file

use std::path::PathBuf;

use anyhow::{bail, Error};
use serde::{Deserialize, Serialize};
use serde_json::json;

use proxmox_router::cli::{
    complete_file_name, run_cli_command, CliCommand, CliCommandMap, CliEnvironment,
};
use proxmox_router::RpcEnvironment;
use proxmox_schema::{api, ApiStringFormat, ApiType, IntegerSchema, Schema, StringSchema};

use proxmox_sys::fs::CreateOptions;

use proxmox_rrd::rrd::{CF, DST, RRA, RRD};

pub const RRA_INDEX_SCHEMA: Schema = IntegerSchema::new("Index of the RRA.").minimum(0).schema();

pub const RRA_CONFIG_STRING_SCHEMA: Schema = StringSchema::new("RRA configuration")
    .format(&ApiStringFormat::PropertyString(&RRAConfig::API_SCHEMA))
    .schema();

#[api(
    properties: {},
    default_key: "cf",
)]
#[derive(Debug, Serialize, Deserialize)]
/// RRA configuration
pub struct RRAConfig {
    /// Time resolution
    pub r: u64,
    pub cf: CF,
    /// Number of data points
    pub n: u64,
}

#[api(
   input: {
       properties: {
          path: {
              description: "The filename."
          },
       },
   },
)]
/// Dump the RRD file in JSON format
pub fn dump_rrd(path: String) -> Result<(), Error> {
    let rrd = RRD::load(&PathBuf::from(path), false)?;
    serde_json::to_writer_pretty(std::io::stdout(), &rrd)?;
    println!();
    Ok(())
}

#[api(
   input: {
       properties: {
          path: {
              description: "The filename."
          },
       },
   },
)]
/// RRD file information
pub fn rrd_info(path: String) -> Result<(), Error> {
    let rrd = RRD::load(&PathBuf::from(path), false)?;

    println!("DST: {:?}", rrd.source.dst);

    for (i, rra) in rrd.rra_list.iter().enumerate() {
        // use RRAConfig property string format
        println!(
            "RRA[{}]: {:?},r={},n={}",
            i,
            rra.cf,
            rra.resolution,
            rra.data.len()
        );
    }

    Ok(())
}

#[api(
   input: {
       properties: {
           path: {
               description: "The filename."
           },
           time: {
               description: "Update time.",
               optional: true,
           },
           value: {
               description: "Update value.",
           },
       },
   },
)]
/// Update the RRD database
pub fn update_rrd(path: String, time: Option<u64>, value: f64) -> Result<(), Error> {
    let path = PathBuf::from(path);

    let time = time
        .map(|v| v as f64)
        .unwrap_or_else(proxmox_time::epoch_f64);

    let mut rrd = RRD::load(&path, false)?;
    rrd.update(time, value);

    rrd.save(&path, CreateOptions::new(), false)?;

    Ok(())
}

#[api(
   input: {
       properties: {
           path: {
               description: "The filename."
           },
           cf: {
               type: CF,
           },
           resolution: {
               description: "Time resolution",
           },
           start: {
               description: "Start time. If not specified, we simply extract 10 data points.",
               optional: true,
           },
           end: {
               description: "End time (Unix Epoch). Default is the last update time.",
               optional: true,
           },
       },
   },
)]
/// Fetch data from the RRD file
pub fn fetch_rrd(
    path: String,
    cf: CF,
    resolution: u64,
    start: Option<u64>,
    end: Option<u64>,
) -> Result<(), Error> {
    let rrd = RRD::load(&PathBuf::from(path), false)?;

    let data = rrd.extract_data(cf, resolution, start, end)?;

    println!("{}", serde_json::to_string_pretty(&data)?);

    Ok(())
}

#[api(
   input: {
       properties: {
           path: {
               description: "The filename."
           },
           "rra-index": {
               schema: RRA_INDEX_SCHEMA,
           },
       },
   },
)]
/// Return the Unix timestamp of the first time slot inside the
/// specified RRA (slot start time)
pub fn first_update_time(path: String, rra_index: usize) -> Result<(), Error> {
    let rrd = RRD::load(&PathBuf::from(path), false)?;

    if rra_index >= rrd.rra_list.len() {
        bail!("rra-index is out of range");
    }
    let rra = &rrd.rra_list[rra_index];
    let duration = (rra.data.len() as u64) * rra.resolution;
    let first = rra.slot_start_time((rrd.source.last_update as u64).saturating_sub(duration));

    println!("{}", first);
    Ok(())
}

#[api(
   input: {
       properties: {
           path: {
               description: "The filename."
           },
       },
   },
)]
/// Return the Unix timestamp of the last update
pub fn last_update_time(path: String) -> Result<(), Error> {
    let rrd = RRD::load(&PathBuf::from(path), false)?;

    println!("{}", rrd.source.last_update);
    Ok(())
}

#[api(
   input: {
       properties: {
           path: {
               description: "The filename."
           },
       },
   },
)]
/// Return the time and value from the last update
pub fn last_update(path: String) -> Result<(), Error> {
    let rrd = RRD::load(&PathBuf::from(path), false)?;

    let result = json!({
        "time": rrd.source.last_update,
        "value": rrd.source.last_value,
    });

    println!("{}", serde_json::to_string_pretty(&result)?);

    Ok(())
}

#[api(
   input: {
       properties: {
           dst: {
               type: DST,
           },
           path: {
               description: "The filename to create."
           },
           rra: {
               description: "Configuration of contained RRAs.",
               type: Array,
               items: {
                   schema:  RRA_CONFIG_STRING_SCHEMA,
               }
           },
       },
   },
)]
/// Create a new RRD file
pub fn create_rrd(dst: DST, path: String, rra: Vec<String>) -> Result<(), Error> {
    let mut rra_list = Vec::new();

    for item in rra.iter() {
        let rra: RRAConfig =
            serde_json::from_value(RRAConfig::API_SCHEMA.parse_property_string(item)?)?;
        println!("GOT {:?}", rra);
        rra_list.push(RRA::new(rra.cf, rra.r, rra.n as usize));
    }

    let path = PathBuf::from(path);

    let rrd = RRD::new(dst, rra_list);

    rrd.save(&path, CreateOptions::new(), false)?;

    Ok(())
}

#[api(
   input: {
       properties: {
           path: {
               description: "The filename."
           },
           "rra-index": {
               schema: RRA_INDEX_SCHEMA,
           },
           slots: {
               description: "The number of slots you want to add or remove.",
               type: i64,
           },
       },
   },
)]
/// Resize. Change the number of data slots for the specified RRA.
pub fn resize_rrd(path: String, rra_index: usize, slots: i64) -> Result<(), Error> {
    let path = PathBuf::from(&path);

    let mut rrd = RRD::load(&path, false)?;

    if rra_index >= rrd.rra_list.len() {
        bail!("rra-index is out of range");
    }

    let rra = &rrd.rra_list[rra_index];

    let new_slots = (rra.data.len() as i64) + slots;

    if new_slots < 1 {
        bail!("number of new slots is too small ('{}' < 1)", new_slots);
    }

    if new_slots > 1024 * 1024 {
        bail!("number of new slots is too big ('{}' > 1M)", new_slots);
    }

    let rra_end = rra.slot_end_time(rrd.source.last_update as u64);
    let rra_start = rra_end - rra.resolution * (rra.data.len() as u64);
    let (start, reso, data) = rra
        .extract_data(rra_start, rra_end, rrd.source.last_update)
        .into();

    let mut new_rra = RRA::new(rra.cf, rra.resolution, new_slots as usize);
    new_rra.last_count = rra.last_count;

    new_rra.insert_data(start, reso, data)?;

    rrd.rra_list[rra_index] = new_rra;

    rrd.save(&path, CreateOptions::new(), false)?;

    Ok(())
}

fn main() -> Result<(), Error> {
    let uid = nix::unistd::Uid::current();

    let username = match nix::unistd::User::from_uid(uid)? {
        Some(user) => user.name,
        None => bail!("unable to get user name"),
    };

    let cmd_def = CliCommandMap::new()
        .insert(
            "create",
            CliCommand::new(&API_METHOD_CREATE_RRD)
                .arg_param(&["path"])
                .completion_cb("path", complete_file_name),
        )
        .insert(
            "dump",
            CliCommand::new(&API_METHOD_DUMP_RRD)
                .arg_param(&["path"])
                .completion_cb("path", complete_file_name),
        )
        .insert(
            "fetch",
            CliCommand::new(&API_METHOD_FETCH_RRD)
                .arg_param(&["path"])
                .completion_cb("path", complete_file_name),
        )
        .insert(
            "first",
            CliCommand::new(&API_METHOD_FIRST_UPDATE_TIME)
                .arg_param(&["path"])
                .completion_cb("path", complete_file_name),
        )
        .insert(
            "info",
            CliCommand::new(&API_METHOD_RRD_INFO)
                .arg_param(&["path"])
                .completion_cb("path", complete_file_name),
        )
        .insert(
            "last",
            CliCommand::new(&API_METHOD_LAST_UPDATE_TIME)
                .arg_param(&["path"])
                .completion_cb("path", complete_file_name),
        )
        .insert(
            "lastupdate",
            CliCommand::new(&API_METHOD_LAST_UPDATE)
                .arg_param(&["path"])
                .completion_cb("path", complete_file_name),
        )
        .insert(
            "resize",
            CliCommand::new(&API_METHOD_RESIZE_RRD)
                .arg_param(&["path"])
                .completion_cb("path", complete_file_name),
        )
        .insert(
            "update",
            CliCommand::new(&API_METHOD_UPDATE_RRD)
                .arg_param(&["path"])
                .completion_cb("path", complete_file_name),
        );

    let mut rpcenv = CliEnvironment::new();
    rpcenv.set_auth_id(Some(format!("{}@pam", username)));

    run_cli_command(cmd_def, rpcenv, None);

    Ok(())
}
