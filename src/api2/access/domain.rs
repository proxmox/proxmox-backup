use failure::*;

use serde_json::{json, Value};

use proxmox::api::api;
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
                    description: "Realm ID.",
                    type: String,
                },
                comment: {
                    schema: SINGLE_LINE_COMMENT_SCHEMA,
                    optional: true,
                },
            },
        }
    }
)]
/// Authentication domain/realm index.
///
/// Anyone can access this, because we need that list for the login
/// box (before the user is authenticated).
fn list_domains() -> Result<Value, Error> {
    let mut list = Vec::new();
    list.push(json!({ "realm": "pam", "comment": "Linux PAM standard authentication" }));
    list.push(json!({ "realm": "pbs", "comment": "Proxmox Backup authentication server" }));
    Ok(list.into())
}

pub const ROUTER: Router = Router::new()
    .get(&API_METHOD_LIST_DOMAINS);
