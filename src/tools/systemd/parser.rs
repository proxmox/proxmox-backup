use anyhow::Error;
use lazy_static::lazy_static;

use super::types::*;

use proxmox::api::{
    schema::*,
    section_config::{
        SectionConfig,
        SectionConfigData,
        SectionConfigPlugin,
    }
};

use proxmox::tools::{fs::replace_file, fs::CreateOptions};


lazy_static! {
    pub static ref CONFIG: SectionConfig = init();
}

fn init() -> SectionConfig {

    let mut config = SectionConfig::with_systemd_syntax(&SYSTEMD_SECTION_NAME_SCHEMA);

    match SystemdUnitSection::API_SCHEMA {
        Schema::Object(ref obj_schema) =>  {
            let plugin = SectionConfigPlugin::new("Unit".to_string(), obj_schema);
            config.register_plugin(plugin);
        }
        _ => unreachable!(),
    };
    match SystemdInstallSection::API_SCHEMA {
        Schema::Object(ref obj_schema) =>  {
            let plugin = SectionConfigPlugin::new("Install".to_string(), obj_schema);
            config.register_plugin(plugin);
        }
        _ => unreachable!(),
    };
    match SystemdServiceSection::API_SCHEMA {
        Schema::Object(ref obj_schema) =>  {
            let plugin = SectionConfigPlugin::new("Service".to_string(), obj_schema);
            config.register_plugin(plugin);
        }
        _ => unreachable!(),
    };
    match SystemdTimerSection::API_SCHEMA {
        Schema::Object(ref obj_schema) =>  {
            let plugin = SectionConfigPlugin::new("Timer".to_string(), obj_schema);
            config.register_plugin(plugin);
        }
        _ => unreachable!(),
    };

    config
}

pub fn parse_systemd_config(filename: &str) -> Result<SectionConfigData, Error> {

    let raw = proxmox::tools::fs::file_get_contents(filename)?;
    let input = String::from_utf8(raw)?;

    let data = CONFIG.parse(filename, &input)?;

    Ok(data)
}


pub fn save_systemd_config(filename: &str, config: &SectionConfigData) -> Result<(), Error> {
    let raw = CONFIG.write(filename, &config)?;

    let mode = nix::sys::stat::Mode::from_bits_truncate(0o0644);
    // set the correct owner/group/permissions while saving file, owner(rw) = root
    let options = CreateOptions::new()
        .perm(mode)
        .owner(nix::unistd::ROOT);

    replace_file(filename, raw.as_bytes(), options)?;

    Ok(())
}
