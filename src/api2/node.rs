use crate::api_schema::router::*;

mod tasks;
mod time;
mod network;
mod dns;
mod syslog;
mod services;

pub const SUBDIRS: SubdirMap = &[
    ("dns", &dns::ROUTER),
    ("network", &network::ROUTER),
    ("services", &services::ROUTER),
    ("syslog", &syslog::ROUTER),
    ("tasks", &tasks::ROUTER),
    ("time", &time::ROUTER),
];

pub const ROUTER: Router = Router::new()
    .get(&list_subdirs_api_method!(SUBDIRS))
    .subdirs(SUBDIRS);
