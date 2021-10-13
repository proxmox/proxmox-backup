//! RRD toolkit - create/manage/update proxmox RRD (v2) file

use std::path::PathBuf;

use anyhow::{bail, Error};
use serde::{Serialize, Deserialize};

use proxmox_router::RpcEnvironment;
use proxmox_router::cli::{run_cli_command, CliCommand, CliCommandMap, CliEnvironment};
use proxmox_schema::{api, parse_property_string};
use proxmox_schema::{ApiStringFormat, ApiType, Schema, StringSchema};

use proxmox::tools::fs::CreateOptions;

use proxmox_rrd::rrd::{CF, DST, RRA, RRD};

pub const RRA_CONFIG_STRING_SCHEMA: Schema = StringSchema::new(
    "RRA configuration")
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
/// Dump the RRDB database in JSON format
pub fn dump_rrdb(path: String) -> Result<(), Error> {

    let rrd = RRD::load(&PathBuf::from(path))?;
    serde_json::to_writer_pretty(std::io::stdout(), &rrd)?;
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
/// Update the RRDB database
pub fn update_rrdb(
    path: String,
    time: Option<u64>,
    value: f64,
) -> Result<(), Error> {

    let path = PathBuf::from(path);

    let time = time.map(|v| v as f64)
        .unwrap_or_else(proxmox_time::epoch_f64);

    let mut rrd = RRD::load(&path)?;
    rrd.update(time, value);

    rrd.save(&path, CreateOptions::new())?;

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
               description: "Time resulution",
           },
           start: {
               description: "Start time. If not sepecified, we simply extract 10 data points.",
               optional: true,
           },
           end: {
               description: "End time (Unix Epoch). Default is the last update time.",
               optional: true,
           },
       },
   },
)]
/// Fetch data from the RRDB database
pub fn fetch_rrdb(
    path: String,
    cf: CF,
    resolution: u64,
    start: Option<u64>,
    end: Option<u64>,
) -> Result<(), Error> {

    let rrd = RRD::load(&PathBuf::from(path))?;

    let data = rrd.extract_data(cf, resolution, start, end)?;

    println!("{}", serde_json::to_string_pretty(&data)?);

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
/// Create a new RRDB database file
pub fn create_rrdb(
    dst: DST,
    path: String,
    rra: Vec<String>,
) -> Result<(), Error> {

    let mut rra_list = Vec::new();

    for item in rra.iter() {
        let rra: RRAConfig = serde_json::from_value(
            parse_property_string(item, &RRAConfig::API_SCHEMA)?
        )?;
        println!("GOT {:?}", rra);
        rra_list.push(RRA::new(rra.cf, rra.r, rra.n as usize));
    }

    let path = PathBuf::from(path);

    let rrd = RRD::new(dst, rra_list);

    rrd.save(&path, CreateOptions::new())?;

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
            CliCommand::new(&API_METHOD_CREATE_RRDB)
                .arg_param(&["path"])
        )
        .insert(
            "update",
            CliCommand::new(&API_METHOD_UPDATE_RRDB)
                .arg_param(&["path"])
        )
        .insert(
            "fetch",
            CliCommand::new(&API_METHOD_FETCH_RRDB)
                .arg_param(&["path"])
        )
        .insert(
            "dump",
            CliCommand::new(&API_METHOD_DUMP_RRDB)
                .arg_param(&["path"])
        )
        ;

    let mut rpcenv = CliEnvironment::new();
    rpcenv.set_auth_id(Some(format!("{}@pam", username)));

    run_cli_command(cmd_def, rpcenv, None);

    Ok(())

}
