use std::collections::HashMap;
use std::io::Read;

use failure::*;
use lazy_static::lazy_static;

use proxmox::tools::{fs::file_set_contents, try_block};

use crate::api_schema::{ObjectSchema, StringSchema};
use crate::section_config::{SectionConfig, SectionConfigData, SectionConfigPlugin};

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

pub fn config() -> Result<SectionConfigData, Error> {

    let mut contents = String::new();

    try_block!({
        match std::fs::File::open(DATASTORE_CFG_FILENAME) {
            Ok(mut file) => file.read_to_string(&mut contents),
            Err(err) => {
                if err.kind() == std::io::ErrorKind::NotFound {
                    contents = String::from("");
                    Ok(0)
                } else {
                    Err(err)
                }
            }
        }
    }).map_err(|e| format_err!("unable to read '{}' - {}", DATASTORE_CFG_FILENAME, e))?;

    CONFIG.parse(DATASTORE_CFG_FILENAME, &contents)
}

pub fn save_config(config: &SectionConfigData) -> Result<(), Error> {

    let raw = CONFIG.write(DATASTORE_CFG_FILENAME, &config)?;

    file_set_contents(DATASTORE_CFG_FILENAME, raw.as_bytes(), None)?;

    Ok(())
}

// shell completion helper
pub fn complete_datastore_name(_arg: &str, _param: &HashMap<String, String>) -> Vec<String> {
    match config() {
        Ok(data) => data.sections.iter().map(|(id,_)| id.to_string()).collect(),
        Err(_) => return vec![],
    }
}
