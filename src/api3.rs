use failure::*;
use std::collections::HashMap;


use crate::json_schema::*;
use crate::api_info::*;
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


fn test_subdir_api_handler(param: Value, _info: &ApiMethod) -> Result<Value, Error> {
    println!("This is a subdir {}", param);

    Ok(json!(null))
}

pub fn router() -> Router {

    let route3 = Router::new()
        .get(ApiMethod {
            handler: test_subdir_api_handler,
            description: "Another Endpoint.",
            parameters: parameter!{},
            returns: Schema::Null,
        });

    let route2 = Router::new()
        .get(ApiMethod {
            handler: test_sync_api_handler,
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
