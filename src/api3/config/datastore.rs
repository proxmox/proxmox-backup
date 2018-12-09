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

    // fixme: locking ?

    let mut config = datastore::config()?;

    let name = param["name"].as_str().unwrap();

    if let Some(_) = config.sections.get(name) {
        bail!("datastore '{}' already exists.", name);
    }

    let datastore = json!({
        "path": param["path"]
    });

    config.set_data(name, "datastore", datastore);

    datastore::save_config(&config)?;

    Ok(Value::Null)
}

pub fn router() -> Router {

    let route = Router::new()
        .get(get())
        .post(post());


    route
}
