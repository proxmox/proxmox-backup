use failure::*;
//use std::collections::HashMap;

use crate::api::schema::*;
use crate::api::router::*;
use serde_json::{json, Value};

use crate::config::datastore;

fn datastore_list(param: Value, _info: &ApiMethod) -> Result<Value, Error> {
    println!("This is a test {}", param);

    let config = datastore::config()?;

    Ok(config.convert_to_array("id"))
}

pub fn router() -> Router {

    let route = Router::new()
        .get(ApiMethod::new(
            datastore_list,
            ObjectSchema::new("Directory index.")));

    route
}
