//! List Authentication domains/realms

use anyhow::{Error};

use serde_json::{json, Value};

use proxmox::api::{api, Permission};
use proxmox::api::router::Router;

use crate::api2::types::*;

#[api(
    returns: {
        description: "List of realms.",
        type: Array,
        items: {
            type: Object,
            description: "User configuration (without password).",
            properties: {
                realm: {
                    schema: REALM_ID_SCHEMA,
                },
                comment: {
                    schema: SINGLE_LINE_COMMENT_SCHEMA,
                    optional: true,
                },
                default: {
                    description: "Default realm.",
                    type: bool,
                }
            },
        }
    },
    access: {
        description: "Anyone can access this, because we need that list for the login box (before the user is authenticated).",
        permission: &Permission::World,
    }
)]
/// Authentication domain/realm index.
fn list_domains() -> Result<Value, Error> {

    let mut list = Vec::new();

    list.push(json!({ "realm": "pam", "comment": "Linux PAM standard authentication", "default": true }));
    list.push(json!({ "realm": "pbs", "comment": "Proxmox Backup authentication server" }));

    let (config, _digest) = crate::config::domains::config()?;

    for (realm, (section_type, v)) in config.sections.iter() {
        let mut item = json!({
            "type": section_type,
            "realm": realm,
        });

        if v["comment"].as_str().is_some() {
            item["comment"] = v["comment"].clone();
        }
        list.push(item);

    }

    Ok(list.into())



}

pub const ROUTER: Router = Router::new()
    .get(&API_METHOD_LIST_DOMAINS);
