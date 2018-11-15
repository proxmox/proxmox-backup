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
            description: "Another Endpoint.",
            parameters: parameter!{},
            returns: Schema::Null,
            handler: Box::new(|param, _info| {
                println!("This is a clousure handler: {}", param);

                Ok(json!(null))
           })
        });

    let route2 = Router::new()
        .get(ApiMethod {
            handler: Box::new(test_sync_api_handler),
            description: "This is a simple test.",
            parameters: parameter!{
                force => Boolean!{
                    optional => true,
                    description => "Test for boolean options."
                }
            },
            returns: Schema::Null,
        })
        .subdirs({
            let mut map = HashMap::new();
            map.insert("subdir3".into(), route3);
            map
        });

    let route = Router::new()
        .match_all(route2);

    route
}
