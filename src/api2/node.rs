use crate::api::schema::*;
use crate::api::router::*;
use serde_json::{json};

mod time;
mod network;
mod dns;
mod syslog;

pub fn router() -> Router {

    let route = Router::new()
        .get(ApiMethod::new(
            |_,_| Ok(json!([
                {"subdir": "network"},
                {"subdir": "syslog"},
                {"subdir": "time"},
           ])),
            ObjectSchema::new("Directory index.")))
        .subdir("dns", dns::router())
        .subdir("network", network::router())
        .subdir("syslog", syslog::router())
        .subdir("time", time::router());

    route
}
