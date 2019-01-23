use crate::api::schema::*;
use crate::api::router::*;
use serde_json::{json};

mod time;
mod network;
mod dns;

pub fn router() -> Router {

    let route = Router::new()
        .get(ApiMethod::new(
            |_,_| Ok(json!([
                {"subdir": "network"},
               {"subdir": "time"},
           ])),
            ObjectSchema::new("Directory index.")))
        .subdir("dns", dns::router())
        .subdir("network", network::router())
        .subdir("time", time::router());

    route
}
