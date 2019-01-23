use failure::*;

use crate::tools;
use crate::api::schema::*;
use crate::api::router::*;
use serde_json::{json, Value};


fn get_dns(_param: Value, _info: &ApiMethod) -> Result<Value, Error> {

    Ok(json!({
        "search": "test.com",
        "dns1": "1.2.3.4",
        "dns2": "1.2.3.4",
        "dns3": "1.2.3.4",
    }))
}

pub fn router() -> Router {

    let route = Router::new()
        .get(ApiMethod::new(
            get_dns,
            ObjectSchema::new("Read DNS settings.")));

    route
}
