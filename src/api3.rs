use failure::*;
use std::collections::HashMap;


use crate::api::schema::*;
use crate::api::router::*;
use serde_json::{json, Value};

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

    let route3 = Router::new()
        .get(ApiMethod {
            parameters: ObjectSchema::new("Another Endpoint."),
            returns: Schema::Null,
            handler: |param, _info| {
                println!("This is a clousure handler: {}", param);

                Ok(json!(null))
           },
        });

    let route2 = Router::new()
        .get(ApiMethod {
            handler: test_sync_api_handler,
             parameters: ObjectSchema::new("This is a simple test.")
                .optional("force", BooleanSchema::new("Test for boolean options")),
            returns: Schema::Null,
        })
        .subdirs({
            let mut map = HashMap::new();
            map.insert("subdir3".into(), route3);
            map
        });

    let route = Router::new()
        .match_all("node", route2);

    route
}
