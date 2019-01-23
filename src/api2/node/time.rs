use failure::*;

use crate::tools;
use crate::api::schema::*;
use crate::api::router::*;
use serde_json::{json, Value};


fn get_time(_param: Value, _info: &ApiMethod) -> Result<Value, Error> {

    Ok(json!({
        "timezone": "Europe/Vienna",
        "time": 1297163644,
        "localtime": 1297163644,
    }))
}

pub fn router() -> Router {

    let route = Router::new()
        .get(ApiMethod::new(
            get_time,
            ObjectSchema::new("Read server time and time zone settings.")));

    route
}
