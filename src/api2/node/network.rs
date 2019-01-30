use failure::*;

//use crate::tools;
use crate::api::schema::*;
use crate::api::router::*;
use serde_json::{json, Value};


fn get_network_config(
    _param: Value,
    _info: &ApiMethod,
    _rpcenv: &mut RpcEnvironment,
) -> Result<Value, Error> {

    Ok(json!({}))
}

pub fn router() -> Router {

    let route = Router::new()
        .get(ApiMethod::new(
            get_network_config,
            ObjectSchema::new("Read network configuration.")));

    route
}
