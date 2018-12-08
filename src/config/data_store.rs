use failure::*;

use std::fs::{OpenOptions};
use std::io::Read;

//use std::sync::Arc;
use crate::api::schema::*;

use crate::section_config::*;

use lazy_static::lazy_static;

lazy_static!{
    static ref CONFIG: SectionConfig = init();
}

fn init() -> SectionConfig {

    let plugin = SectionConfigPlugin::new(
        "datastore".to_string(),
        ObjectSchema::new("DataStore properties")
            .required("path", StringSchema::new("Directory name"))
    );

    let id_schema = StringSchema::new("DataStore ID schema.")
        .min_length(3)
        .into();

    let mut config = SectionConfig::new(id_schema);
    config.register_plugin(plugin);

    config
}

const DATASTORE_CFG_FILENAME: &str = "/etc/proxmox-backup/datastore.cfg";

fn config() -> Result<SectionConfigData, Error> {

    let mut file = match OpenOptions::new()
        .create(true)
        .write(true)
        .open(DATASTORE_CFG_FILENAME) {
            Ok(file) => file,
            Err(err) => bail!("Unable to open '{}' - {}",
                              DATASTORE_CFG_FILENAME, err),
        };

    let mut contents = String::new();
    file.read_to_string(&mut contents).unwrap();

    CONFIG.parse(DATASTORE_CFG_FILENAME, &contents)
}


