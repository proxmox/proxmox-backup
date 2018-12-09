use failure::*;
//use std::collections::HashMap;

use crate::api::schema::*;
use crate::api::router::*;
use serde_json::{json, Value};

use crate::config::datastore;

pub fn get() -> ApiMethod {
    ApiMethod::new(
        get_datastore_list,
        ObjectSchema::new("Directory index."))
}

fn get_datastore_list(param: Value, _info: &ApiMethod) -> Result<Value, Error> {

    let config = datastore::config()?;

    Ok(config.convert_to_array("name"))
}

pub fn post() -> ApiMethod {
    ApiMethod::new(
        create_datastore,
        ObjectSchema::new("Create new datastore.")
            .required("name", StringSchema::new("Datastore name."))
            .required("path", StringSchema::new("Directory path (must exist)."))
    )
}

fn create_datastore(param: Value, _info: &ApiMethod) -> Result<Value, Error> {
    println!("This is a test {}", param);

    Ok(json!({}))
}

pub fn router() -> Router {

    let route = Router::new()
        .get(get())
        .post(post());


    route
}
