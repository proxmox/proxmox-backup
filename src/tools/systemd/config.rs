use anyhow::Error;
use lazy_static::lazy_static;

use super::types::*;

use proxmox_schema::*;
use proxmox_section_config::{SectionConfig, SectionConfigData, SectionConfigPlugin};

use proxmox_sys::{fs::replace_file, fs::CreateOptions};

lazy_static! {
    pub static ref SERVICE_CONFIG: SectionConfig = init_service();
    pub static ref TIMER_CONFIG: SectionConfig = init_timer();
    pub static ref MOUNT_CONFIG: SectionConfig = init_mount();
}

fn init_service() -> SectionConfig {
    let mut config = SectionConfig::with_systemd_syntax(&SYSTEMD_SECTION_NAME_SCHEMA);

    match SystemdUnitSection::API_SCHEMA {
        Schema::Object(ref obj_schema) => {
            let plugin = SectionConfigPlugin::new("Unit".to_string(), None, obj_schema);
            config.register_plugin(plugin);
        }
        _ => unreachable!(),
    };
    match SystemdInstallSection::API_SCHEMA {
        Schema::Object(ref obj_schema) => {
            let plugin = SectionConfigPlugin::new("Install".to_string(), None, obj_schema);
            config.register_plugin(plugin);
        }
        _ => unreachable!(),
    };
    match SystemdServiceSection::API_SCHEMA {
        Schema::Object(ref obj_schema) => {
            let plugin = SectionConfigPlugin::new("Service".to_string(), None, obj_schema);
            config.register_plugin(plugin);
        }
        _ => unreachable!(),
    };

    config
}

fn init_timer() -> SectionConfig {
    let mut config = SectionConfig::with_systemd_syntax(&SYSTEMD_SECTION_NAME_SCHEMA);

    match SystemdUnitSection::API_SCHEMA {
        Schema::Object(ref obj_schema) => {
            let plugin = SectionConfigPlugin::new("Unit".to_string(), None, obj_schema);
            config.register_plugin(plugin);
        }
        _ => unreachable!(),
    };
    match SystemdInstallSection::API_SCHEMA {
        Schema::Object(ref obj_schema) => {
            let plugin = SectionConfigPlugin::new("Install".to_string(), None, obj_schema);
            config.register_plugin(plugin);
        }
        _ => unreachable!(),
    };
    match SystemdTimerSection::API_SCHEMA {
        Schema::Object(ref obj_schema) => {
            let plugin = SectionConfigPlugin::new("Timer".to_string(), None, obj_schema);
            config.register_plugin(plugin);
        }
        _ => unreachable!(),
    };

    config
}

fn init_mount() -> SectionConfig {
    let mut config = SectionConfig::with_systemd_syntax(&SYSTEMD_SECTION_NAME_SCHEMA);

    match SystemdUnitSection::API_SCHEMA {
        Schema::Object(ref obj_schema) => {
            let plugin = SectionConfigPlugin::new("Unit".to_string(), None, obj_schema);
            config.register_plugin(plugin);
        }
        _ => unreachable!(),
    };
    match SystemdInstallSection::API_SCHEMA {
        Schema::Object(ref obj_schema) => {
            let plugin = SectionConfigPlugin::new("Install".to_string(), None, obj_schema);
            config.register_plugin(plugin);
        }
        _ => unreachable!(),
    };
    match SystemdMountSection::API_SCHEMA {
        Schema::Object(ref obj_schema) => {
            let plugin = SectionConfigPlugin::new("Mount".to_string(), None, obj_schema);
            config.register_plugin(plugin);
        }
        _ => unreachable!(),
    };

    config
}

fn parse_systemd_config(
    config: &SectionConfig,
    filename: &str,
) -> Result<SectionConfigData, Error> {
    let raw = proxmox_sys::fs::file_get_contents(filename)?;
    let input = String::from_utf8(raw)?;

    let data = config.parse(filename, &input)?;

    Ok(data)
}

pub fn parse_systemd_service(filename: &str) -> Result<SectionConfigData, Error> {
    parse_systemd_config(&SERVICE_CONFIG, filename)
}

pub fn parse_systemd_timer(filename: &str) -> Result<SectionConfigData, Error> {
    parse_systemd_config(&TIMER_CONFIG, filename)
}

pub fn parse_systemd_mount(filename: &str) -> Result<SectionConfigData, Error> {
    parse_systemd_config(&MOUNT_CONFIG, filename)
}

fn save_systemd_config(
    config: &SectionConfig,
    filename: &str,
    data: &SectionConfigData,
) -> Result<(), Error> {
    let raw = config.write(filename, data)?;

    let mode = nix::sys::stat::Mode::from_bits_truncate(0o0644);
    // set the correct owner/group/permissions while saving file, owner(rw) = root
    let options = CreateOptions::new().perm(mode).owner(nix::unistd::ROOT);

    replace_file(filename, raw.as_bytes(), options, true)?;

    Ok(())
}

pub fn save_systemd_service(filename: &str, data: &SectionConfigData) -> Result<(), Error> {
    save_systemd_config(&SERVICE_CONFIG, filename, data)
}

pub fn save_systemd_timer(filename: &str, data: &SectionConfigData) -> Result<(), Error> {
    save_systemd_config(&TIMER_CONFIG, filename, data)
}

pub fn save_systemd_mount(filename: &str, data: &SectionConfigData) -> Result<(), Error> {
    save_systemd_config(&MOUNT_CONFIG, filename, data)
}
