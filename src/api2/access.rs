use failure::*;

use crate::tools;
use crate::api::schema::*;
use crate::api::router::*;
use crate::tools::ticket::*;
use crate::auth_helpers::*;

use serde_json::{json, Value};

fn authenticate_user(username: &str, password: &str) -> Result<(), Error> {

    if username == "root@pam" && password == "test" {
        return Ok(());
    }

    bail!("inavlid credentials");
}

fn create_ticket(
    param: Value,
    _info: &ApiMethod,
    _rpcenv: &mut RpcEnvironment,
) -> Result<Value, Error> {

    let username = tools::required_string_param(&param, "username")?;
    let password = tools::required_string_param(&param, "password")?;

    match authenticate_user(username, password) {
        Ok(_) => {

            let ticket = assemble_rsa_ticket( private_auth_key(), "PBS", None, None)?;

            let token = assemble_csrf_prevention_token(csrf_secret(), username);

            log::info!("successful auth for user '{}'", username);

            return Ok(json!({
                "username": username,
                "ticket": ticket,
                "CSRFPreventionToken": token,
            }));
        }
        Err(err) => {
	    let client_ip = "unknown"; // $rpcenv->get_client_ip() || '';
            log::error!("authentication failure; rhost={} user={} msg={}", client_ip, username, err.to_string());
            bail!("authentication failure");
        }
    }
}

pub fn router() -> Router {

    let route = Router::new()
        .get(ApiMethod::new(
            |_,_,_| Ok(json!([
                {"subdir": "ticket"}
            ])),
            ObjectSchema::new("Directory index.")))
        .subdir(
            "ticket",
            Router::new()
                .post(
                    ApiMethod::new(
                        create_ticket,
                        ObjectSchema::new("Create or verify authentication ticket.")
                            .required(
                                "username",
                                StringSchema::new("User name.")
                                    .max_length(64)
                            )
                            .required(
                                "password",
                                StringSchema::new("The secret password. This can also be a valid ticket.")
                            )
                    ).returns(
                        ObjectSchema::new("Returns authentication ticket with additional infos.")
                            .required("username", StringSchema::new("User name."))
                            .required("ticket", StringSchema::new("Auth ticket."))
                            .required("CSRFPreventionToken", StringSchema::new("Cross Site Request Forgery Prevention Token."))
                    ).protected(true)
                )
        );

    route
}
