use failure::*;

use crate::api_schema::*;
use crate::section_config::*;

use lazy_static::lazy_static;

lazy_static!{
    static ref STORAGE_SECTION_CONFIG: SectionConfig = register_storage_plugins();
}

fn register_storage_plugins() -> SectionConfig {

    let plugin = SectionConfigPlugin::new(
        "lvmthin".to_string(),
        ObjectSchema::new("lvmthin properties")
            .required("thinpool", StringSchema::new("LVM thin pool name."))
            .required("vgname", StringSchema::new("LVM volume group name."))
            .optional("content", StringSchema::new("Storage content types."))
    );
    
    let id_schema = StringSchema::new("Storage ID schema.")
        .min_length(3)
        .into();

    let mut config = SectionConfig::new(id_schema);
    config.register_plugin(plugin);

    config
}

pub fn parse_config(filename: &str, raw: &str) -> Result<SectionConfigData, Error> {

    let res = STORAGE_SECTION_CONFIG.parse(filename, raw);

    res
}

pub fn write_config(filename: &str, config: &SectionConfigData) -> Result<String, Error> {

    STORAGE_SECTION_CONFIG.write(filename, config)
}
