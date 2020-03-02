use failure::*;
use lazy_static::lazy_static;

use proxmox::api::{
    schema::*,
    section_config::{
        SectionConfig,
        SectionConfigData,
        SectionConfigPlugin,
    }
};

lazy_static!{
    static ref STORAGE_SECTION_CONFIG: SectionConfig = register_storage_plugins();
}

const ID_SCHEMA: Schema = StringSchema::new("Storage ID schema.")
    .min_length(3)
    .schema();

const LVMTHIN_PROPERTIES: ObjectSchema = ObjectSchema::new(
    "lvmthin properties",
    &[
        ("thinpool", false, &StringSchema::new("LVM thin pool name.").schema()),
        ("vgname", false, &StringSchema::new("LVM volume group name.").schema()),
        ("content", true, &StringSchema::new("Storage content types.").schema()),
    ],
);

fn register_storage_plugins() -> SectionConfig {

    let plugin = SectionConfigPlugin::new("lvmthin".to_string(), &LVMTHIN_PROPERTIES);
    let mut config = SectionConfig::new(&ID_SCHEMA);
    config.register_plugin(plugin);

    config
}

pub fn parse_config(filename: &str, raw: &str) -> Result<SectionConfigData, Error> {

    STORAGE_SECTION_CONFIG.parse(filename, raw)
}

pub fn write_config(filename: &str, config: &SectionConfigData) -> Result<String, Error> {

    STORAGE_SECTION_CONFIG.write(filename, config)
}
