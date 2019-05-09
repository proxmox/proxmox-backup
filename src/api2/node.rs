use crate::api_schema::router::*;

mod tasks;
mod time;
mod network;
mod dns;
mod syslog;
mod services;

pub fn router() -> Router {

    let route = Router::new()
        .subdir("dns", dns::router())
        .subdir("network", network::router())
        .subdir("services", services::router())
        .subdir("syslog", syslog::router())
        .subdir("tasks", tasks::router())
        .subdir("time", time::router())
        .list_subdirs();

    route
}
