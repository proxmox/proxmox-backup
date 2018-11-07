use failure::*;
use std::collections::HashMap;


use crate::json_schema::*;
use crate::api_info::*;
use serde_json::{json, Value};


fn test_api_handler(param: Value, info: &ApiMethod) -> Result<Value, Error> {
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


pub fn router() -> MethodInfo {

    let route = MethodInfo::new()
        .get(ApiMethod {
            handler: test_api_handler,
            description: "This is a simple test.",
            parameters: parameter!{
                force => Boolean!{
                    optional => true,
                    description => "Test for boolean options."
                }
            },
            returns: Jss::Null,
        });

    route
}


