use proxmox::api::router::{Router, SubdirMap};
use proxmox::api::list_subdirs_api_method;

mod tasks;
mod time;
mod network;
mod dns;
mod syslog;
mod journal;
mod services;
mod status;

pub const SUBDIRS: SubdirMap = &[
    ("dns", &dns::ROUTER),
    ("journal", &journal::ROUTER),
    ("network", &network::ROUTER),
    ("services", &services::ROUTER),
    ("status", &status::ROUTER),
    ("syslog", &syslog::ROUTER),
    ("tasks", &tasks::ROUTER),
    ("time", &time::ROUTER),
];

pub const ROUTER: Router = Router::new()
    .get(&list_subdirs_api_method!(SUBDIRS))
    .subdirs(SUBDIRS);
