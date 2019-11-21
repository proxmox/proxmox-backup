use failure::*;

//use crate::tools;
use crate::api_schema::*;
use crate::api_schema::router::*;
use serde_json::{json, Value};

use crate::api2::types::*;

fn get_network_config(
    _param: Value,
    _info: &ApiMethod,
    _rpcenv: &mut dyn RpcEnvironment,
) -> Result<Value, Error> {

    Ok(json!({}))
}

pub const ROUTER: Router = Router::new()
    .get(
        &ApiMethod::new(
            &ApiHandler::Sync(&get_network_config),
            &ObjectSchema::new(
                "Read network configuration.",
                &[ ("node", false, &NODE_SCHEMA) ],
            )
        )
    );
  
