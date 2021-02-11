use anyhow::{bail, Error};

use proxmox::api::format::{
    dump_enum_properties,
    dump_section_config,
};

use proxmox_backup::{
    config,
};

fn get_args() -> (String, Vec<String>) {

    let mut args = std::env::args();
    let prefix = args.next().unwrap();
    let prefix = prefix.rsplit('/').next().unwrap().to_string(); // without path
    let args: Vec<String> = args.collect();

    (prefix, args)
}

fn main() -> Result<(), Error> {

    let (_prefix, args) = get_args();

    if args.len() < 1 {
        bail!("missing arguments");
    }
    
    for arg in args.iter() {
        let text = match arg.as_ref() {
            "datastore.cfg" => dump_section_config(&config::datastore::CONFIG),
            "tape.cfg" => dump_section_config(&config::drive::CONFIG),
            "user.cfg" => dump_section_config(&config::user::CONFIG),
            "remote.cfg" => dump_section_config(&config::remote::CONFIG),
            "sync.cfg" => dump_section_config(&config::sync::CONFIG),
            "verification.cfg" => dump_section_config(&config::verify::CONFIG),
            "media-pool.cfg" => dump_section_config(&config::media_pool::CONFIG),
            "config::acl::Role" => dump_enum_properties(&config::acl::Role::API_SCHEMA)?,
            _ => bail!("docgen: got unknown type"),
        };
        println!("{}", text);
    }
   
    Ok(())
}
