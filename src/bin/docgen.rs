use anyhow::{bail, Error};

use proxmox::api::{
    format::*,
    section_config::*,
};

use proxmox_backup::{
    config,
};

fn dump_section_config(config: &SectionConfig) -> String {

    let mut res = String::new();

    let plugin_count = config.plugins().len();
    
    for plugin in config.plugins().values() {

        let name = plugin.type_name();
        let properties = plugin.properties();
        let skip = match plugin.id_property() {
            Some(id) => vec![id],
            None => Vec::new(),
        };

        if plugin_count > 1 {
            res.push_str(&format!("\n**Section type** \'``{}``\'\n\n", name));
        }
        
        res.push_str(&dump_api_parameters(properties, "", ParameterDisplayStyle::Config, &skip));
    }
    
    res
}

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
