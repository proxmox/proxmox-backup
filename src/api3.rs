use failure::*;
//use std::collections::HashMap;


use crate::api::schema::*;
use crate::api::router::*;
use serde_json::{json, Value};

pub mod config;
mod version;

fn test_sync_api_handler(param: Value, _info: &ApiMethod) -> Result<Value, Error> {
    println!("This is a test {}", param);

   // let force: Option<bool> = Some(false);

    //if let Some(force) = param.force {
    //}

    let _force =  param["force"].as_bool()
        .ok_or_else(|| format_err!("missing parameter 'force'"))?;

    if let Some(_force) = param["force"].as_bool() {
    }


    Ok(json!(null))
}

pub fn router() -> Router {

    let route4 = Router::new()
        .get(ApiMethod::new(
            |param, _info| {
                println!("This is a clousure handler: {}", param);

                Ok(json!(null))
            },
            ObjectSchema::new("Another Endpoint."))
             .returns(Schema::Null));


    let nodeinfo = Router::new()
        .get(ApiMethod::new(
            test_sync_api_handler,
            ObjectSchema::new("This is a simple test.")
                .optional("force", BooleanSchema::new("Test for boolean options")))
        )
        .subdir("subdir3", route4);

    let nodes = Router::new()
        .match_all("node", nodeinfo);


    let route = Router::new()
        .get(ApiMethod::new(
            |_,_| Ok(json!([
                {"subdir": "config"},
                {"subdir": "version"},
                {"subdir": "nodes"}
            ])),
            ObjectSchema::new("Directory index.")))
        .subdir("config", config::router())
        .subdir("version", version::router())
        .subdir("nodes", nodes);

    route
}
