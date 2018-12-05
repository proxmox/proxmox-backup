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

const PROXMOX_PKG_VERSION: &'static str = env!("PROXMOX_PKG_VERSION");
const PROXMOX_PKG_RELEASE: &'static str = env!("PROXMOX_PKG_RELEASE");
const PROXMOX_PKG_REPOID: &'static str = env!("PROXMOX_PKG_REPOID");


fn get_version(param: Value, _info: &ApiMethod) -> Result<Value, Error> {

    Ok(json!({
        "version": PROXMOX_PKG_VERSION,
        "release": PROXMOX_PKG_RELEASE,
        "repoid": PROXMOX_PKG_REPOID
    }))
}


pub fn router() -> Router {

    let route4 = Router::new()
        .get(ApiMethod {
            parameters: ObjectSchema::new("Another Endpoint."),
            returns: Schema::Null,
            handler: |param, _info| {
                println!("This is a clousure handler: {}", param);

                Ok(json!(null))
           },
        });


    let nodeinfo = Router::new()
        .get(ApiMethod {
            handler: test_sync_api_handler,
            parameters: ObjectSchema::new("This is a simple test.")
                .optional("force", BooleanSchema::new("Test for boolean options")),
            returns: Schema::Null,
        })
        .subdirs({
            let mut map = HashMap::new();
            map.insert("subdir3".into(), route4);
            map
        });

    let nodes = Router::new()
        .match_all("node", nodeinfo);

    let version = Router::new()
        .get(ApiMethod {
            handler: get_version,
            parameters: ObjectSchema::new("Proxmox Backup Server API version."),
            returns: Schema::Null,
        });

     let route = Router::new()
        .get(ApiMethod {
            handler: get_version,
            parameters: ObjectSchema::new("Directory index."),
            returns: Schema::Null,
        })
        .subdirs({
            let mut map = HashMap::new();
            map.insert("version".into(), version);
            map.insert("nodes".into(), nodes);
           map
        });

    route
}
