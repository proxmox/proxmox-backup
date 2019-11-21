use failure::*;

use serde_json::{json, Value};

use proxmox::{sortable, identity};
use proxmox::api::{http_err, list_subdirs_api_method};
use proxmox::api::{ApiHandler, ApiMethod, Router, RpcEnvironment};
use proxmox::api::router::SubdirMap;
use proxmox::api::schema::*;

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

fn create_ticket(
    param: Value,
    _info: &ApiMethod,
    _rpcenv: &mut dyn RpcEnvironment,
) -> Result<Value, Error> {

    let username = tools::required_string_param(&param, "username")?;
    let password = tools::required_string_param(&param, "password")?;

    match authenticate_user(username, password) {
        Ok(_) => {

            let ticket = assemble_rsa_ticket( private_auth_key(), "PBS", Some(username), None)?;

            let token = assemble_csrf_prevention_token(csrf_secret(), username);

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
            .post(
                &ApiMethod::new(
                    &ApiHandler::Sync(&create_ticket),
                    &ObjectSchema::new(
                        "Create or verify authentication ticket.",
                        &sorted!([
                            (
                                "username",
                                false,
                                &StringSchema::new("User name.")
                                    .max_length(64)
                                    .schema()
                            ),
                            (
                                "password",
                                false,
                                &StringSchema::new("The secret password. This can also be a valid ticket.")
                                    .schema()
                            ),
                        ]),
                    )
                ).returns(
                    &ObjectSchema::new(
                        "Returns authentication ticket with additional infos.",
                        &sorted!([
                            (
                                "username",
                                false,
                                &StringSchema::new("User name.").schema()
                            ),
                            (
                                "ticket",
                                false,
                                &StringSchema::new("Auth ticket.").schema()
                            ),
                            (
                                "CSRFPreventionToken",
                                false,
                                &StringSchema::new("Cross Site Request Forgery Prevention Token.")
                                    .schema()
                            ),
                        ]),
                    ).schema()
                ).protected(true)
            )
    )
];

pub const ROUTER: Router = Router::new()
    .get(&list_subdirs_api_method!(SUBDIRS))
    .subdirs(SUBDIRS);
