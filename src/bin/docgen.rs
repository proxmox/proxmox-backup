use anyhow::{bail, Error};

use proxmox::api::format::dump_section_config;

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
        match arg.as_ref() {
            "datastore.cfg" => println!("{}", dump_section_config(&config::datastore::CONFIG)),
            "tape.cfg" => println!("{}", dump_section_config(&config::drive::CONFIG)),
            "user.cfg" => println!("{}", dump_section_config(&config::user::CONFIG)),
            "remote.cfg" => println!("{}", dump_section_config(&config::remote::CONFIG)),
            "sync.cfg" => println!("{}", dump_section_config(&config::sync::CONFIG)),
            "media-pool.cfg" => println!("{}", dump_section_config(&config::media_pool::CONFIG)),
            _ => bail!("docgen: got unknown type"),
        }
    }
   
    Ok(())
}
