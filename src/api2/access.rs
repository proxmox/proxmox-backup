use failure::*;

use serde_json::{json, Value};

use proxmox::api::api;
use proxmox::api::router::{Router, SubdirMap};
use proxmox::sortable;
use proxmox::{http_err, list_subdirs_api_method};

use crate::tools;
use crate::tools::ticket::*;
use crate::auth_helpers::*;

fn authenticate_user(username: &str, password: &str) -> Result<(), Error> {

    let ticket_lifetime = tools::ticket::TICKET_LIFETIME;

    if password.starts_with("PBS:") {
        if let Ok((_age, Some(ticket_username))) = tools::ticket::verify_rsa_ticket(public_auth_key(), "PBS", password, None, -300, ticket_lifetime) {
            if ticket_username == username {
                return Ok(());
            } else {
                bail!("ticket login failed - wrong username");
            }
        }
    }

    if username == "root@pam" {
        let mut auth = pam::Authenticator::with_password("proxmox-backup-auth").unwrap();
        auth.get_handler().set_credentials("root", password);
        auth.authenticate()?;
        return Ok(());
    }

    bail!("inavlid credentials");
}

#[api(
    input: {
        properties: {
            username: {
                type: String,
                description: "User name.",
                max_length: 64,
            },
            password: {
                type: String,
                description: "The secret password. This can also be a valid ticket.",
            },
        },
    },
    returns: {
        properties: {
            username: {
                type: String,
                description: "User name.",
            },
            ticket: {
                type: String,
                description: "Auth ticket.",
            },
            CSRFPreventionToken: {
                type: String,
                description: "Cross Site Request Forgery Prevention Token.",
            },
        },
    },
    protected: true,
)]
/// Create or verify authentication ticket.
///
/// Returns: An authentication ticket with additional infos.
fn create_ticket(username: String, password: String) -> Result<Value, Error> {
    match authenticate_user(&username, &password) {
        Ok(_) => {

            let ticket = assemble_rsa_ticket( private_auth_key(), "PBS", Some(&username), None)?;

            let token = assemble_csrf_prevention_token(csrf_secret(), &username);

            log::info!("successful auth for user '{}'", username);

            Ok(json!({
                "username": username,
                "ticket": ticket,
                "CSRFPreventionToken": token,
            }))
        }
        Err(err) => {
            let client_ip = "unknown"; // $rpcenv->get_client_ip() || '';
            log::error!("authentication failure; rhost={} user={} msg={}", client_ip, username, err.to_string());
            Err(http_err!(UNAUTHORIZED, "permission check failed.".into()))
        }
    }
}

#[sortable]
const SUBDIRS: SubdirMap = &[
    (
        "ticket", &Router::new()
            .post(&API_METHOD_CREATE_TICKET)
    )
];

pub const ROUTER: Router = Router::new()
    .get(&list_subdirs_api_method!(SUBDIRS))
    .subdirs(SUBDIRS);
