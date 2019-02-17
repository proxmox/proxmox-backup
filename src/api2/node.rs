use crate::api_schema::schema::*;
use crate::api_schema::router::*;
use serde_json::{json};

mod time;
mod network;
mod dns;
mod syslog;
mod services;

pub fn router() -> Router {

    let route = Router::new()
        .get(ApiMethod::new(
            |_,_,_| Ok(json!([
                {"subdir": "dns"},
                {"subdir": "network"},
                {"subdir": "services"},
                {"subdir": "syslog"},
                {"subdir": "time"},
           ])),
            ObjectSchema::new("Directory index.")))
        .subdir("dns", dns::router())
        .subdir("network", network::router())
        .subdir("services", services::router())
        .subdir("syslog", syslog::router())
        .subdir("time", time::router());

    route
}
