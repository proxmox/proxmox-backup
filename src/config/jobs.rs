use anyhow::{bail, Error};
use regex::Regex;
use lazy_static::lazy_static;

use proxmox::api::section_config::SectionConfigData;

use crate::PROXMOX_SAFE_ID_REGEX_STR;
use crate::tools::systemd::config::*;
use crate::tools::systemd::types::*;

const SYSTEMD_CONFIG_DIR: &str = "/etc/systemd/system";

#[derive(Debug)]
pub enum JobType {
    GarbageCollection,
    Prune,
}

#[derive(Debug)]
pub struct CalenderTimeSpec {
    pub hour: u8, // 0-23
}

#[derive(Debug)]
pub struct JobListEntry {
    job_type: JobType,
    id: String,
}

pub fn list_jobs() -> Result<Vec<JobListEntry>, Error> {

    lazy_static!{
        static ref PBS_JOB_REGEX: Regex = Regex::new(
            concat!(r"^pbs-(gc|prune)-(", PROXMOX_SAFE_ID_REGEX_STR!(), ").timer$")
        ).unwrap();
    }

    let mut list = Vec::new();

    for entry in crate::tools::fs::read_subdir(libc::AT_FDCWD, SYSTEMD_CONFIG_DIR)? {
        let entry = entry?;
        let file_type = match entry.file_type() {
            Some(file_type) => file_type,
            None => bail!("unable to detect file type"),
        };
        if file_type != nix::dir::Type::File { continue; };

        let file_name = entry.file_name().to_bytes();
        if file_name == b"." || file_name == b".." { continue; };

        let name = match std::str::from_utf8(file_name) {
            Ok(name) => name,
            Err(_) => continue,
        };
        let caps = match PBS_JOB_REGEX.captures(name) {
            Some(caps) => caps,
            None => continue,
        };

        // fixme: read config data ?
        //let config = parse_systemd_timer(&format!("{}/{}", SYSTEMD_CONFIG_DIR, name))?;

        match (&caps[1], &caps[2]) {
            ("gc", store) => {
                list.push(JobListEntry {
                    job_type: JobType::GarbageCollection,
                    id: store.to_string(),
                });
            }
            ("prune", store) => {
                list.push(JobListEntry {
                    job_type: JobType::Prune,
                    id: store.to_string(),
                });
            }
            _ => unreachable!(),
        }
    }

    Ok(list)
}

pub fn new_systemd_service_config(
    unit: &SystemdUnitSection,
    service: &SystemdServiceSection,
) -> Result<SectionConfigData, Error> {

    let mut config = SectionConfigData::new();
    config.set_data("Unit", "Unit", unit)?;
    config.record_order("Unit");
    config.set_data("Service", "Service", service)?;
    config.record_order("Service");

    Ok(config)
}

pub fn new_systemd_timer_config(
    unit: &SystemdUnitSection,
    timer: &SystemdTimerSection,
    install:  &SystemdInstallSection,
) -> Result<SectionConfigData, Error> {

    let mut config = SectionConfigData::new();
    config.set_data("Unit", "Unit", unit)?;
    config.record_order("Unit");
    config.set_data("Timer", "Timer", timer)?;
    config.record_order("Timer");
    config.set_data("Install", "Install", install)?;
    config.record_order("Install");

    Ok(config)
}

pub fn create_garbage_collection_job(
    schedule: CalenderTimeSpec,
    store: &str,
) -> Result<(), Error> {

    if schedule.hour > 23 {
        bail!("inavlid time spec: hour > 23");
    }

    let description = format!("Proxmox Backup Server Garbage Collection Job '{}'", store);

    let unit = SystemdUnitSection {
        Description: description.clone(),
        ConditionACPower: Some(true),
        ..Default::default()
    };

    let cmd = format!("/usr/sbin/proxmox-backup-manager garbage-collection start {} --output-format json", store);
    let service = SystemdServiceSection {
        Type: Some(ServiceStartup::Oneshot),
        ExecStart: Some(vec![cmd]),
        ..Default::default()
    };

    let service_config = new_systemd_service_config(&unit, &service)?;

    let timer = SystemdTimerSection {
        OnCalendar: Some(vec![format!("{}:00", schedule.hour)]),
        ..Default::default()
    };

    let install = SystemdInstallSection {
        WantedBy: Some(vec![String::from("timers.target")]),
        ..Default::default()
    };

    let timer_config = new_systemd_timer_config(&unit, &timer, &install)?;

    let basename = format!("{}/pbs-gc-{}", SYSTEMD_CONFIG_DIR, store);
    let timer_fn = format!("{}.timer", basename);
    let service_fn = format!("{}.service", basename);

    save_systemd_service(&service_fn, &service_config)?;
    save_systemd_timer(&timer_fn, &timer_config)?;

    Ok(())
}
