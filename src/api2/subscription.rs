use failure::*;
use serde_json::{json, Value};

use proxmox::api::{ApiHandler, ApiMethod, Router, RpcEnvironment};
use proxmox::api::schema::ObjectSchema;

use crate::tools;

fn get_subscription(
    _param: Value,
    _info: &ApiMethod,
    _rpcenv: &mut dyn RpcEnvironment,
) -> Result<Value, Error> {

    let url = "https://www.proxmox.com/en/proxmox-backup-server/pricing";
    Ok(json!({
        "status": "NotFound",
	"message": "There is no subscription key",
	"serverid": tools::get_hardware_address()?,
	"url":  url,
     }))
}

pub const ROUTER: Router = Router::new()
    .get(
        &ApiMethod::new(
            &ApiHandler::Sync(&get_subscription),
            &ObjectSchema::new("Read subscription info.", &[])
        )
    );
