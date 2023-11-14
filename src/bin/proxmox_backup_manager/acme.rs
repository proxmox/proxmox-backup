use std::io::Write;

use anyhow::{bail, Error};
use serde_json::Value;

use proxmox_router::{cli::*, ApiHandler, RpcEnvironment};
use proxmox_schema::api;
use proxmox_sys::fs::file_get_contents;

use proxmox_backup::acme::AcmeClient;
use proxmox_backup::api2;
use proxmox_backup::api2::types::AcmeAccountName;
use proxmox_backup::config::acme::plugin::DnsPluginCore;
use proxmox_backup::config::acme::KNOWN_ACME_DIRECTORIES;

pub fn acme_mgmt_cli() -> CommandLineInterface {
    let cmd_def = CliCommandMap::new()
        .insert("account", account_cli())
        .insert("cert", cert_cli())
        .insert("plugin", plugin_cli());

    cmd_def.into()
}

#[api(
    input: {
        properties: {
            "output-format": {
                schema: OUTPUT_FORMAT,
                optional: true,
            },
        }
    }
)]
/// List acme accounts.
fn list_accounts(param: Value, rpcenv: &mut dyn RpcEnvironment) -> Result<(), Error> {
    let output_format = get_output_format(&param);

    let info = &api2::config::acme::API_METHOD_LIST_ACCOUNTS;
    let mut data = match info.handler {
        ApiHandler::Sync(handler) => (handler)(param, info, rpcenv)?,
        _ => unreachable!(),
    };

    let options = default_table_format_options();
    format_and_print_result_full(&mut data, &info.returns, &output_format, &options);

    Ok(())
}

#[api(
    input: {
        properties: {
            name: { type: AcmeAccountName },
            "output-format": {
                schema: OUTPUT_FORMAT,
                optional: true,
            },
        }
    }
)]
/// Show acme account information.
async fn get_account(param: Value, rpcenv: &mut dyn RpcEnvironment) -> Result<(), Error> {
    let output_format = get_output_format(&param);

    let info = &api2::config::acme::API_METHOD_GET_ACCOUNT;
    let mut data = match info.handler {
        ApiHandler::Async(handler) => (handler)(param, info, rpcenv).await?,
        _ => unreachable!(),
    };

    let options = default_table_format_options()
        .column(
            ColumnConfig::new("account")
                .renderer(|value, _record| Ok(serde_json::to_string_pretty(value)?)),
        )
        .column(ColumnConfig::new("directory"))
        .column(ColumnConfig::new("location"))
        .column(ColumnConfig::new("tos"));
    format_and_print_result_full(&mut data, &info.returns, &output_format, &options);

    Ok(())
}

#[api(
    input: {
        properties: {
            name: { type: AcmeAccountName },
            contact: {
                description: "List of email addresses.",
            },
            directory: {
                type: String,
                description: "The ACME Directory.",
                optional: true,
            },
        }
    }
)]
/// Register an ACME account.
async fn register_account(
    name: AcmeAccountName,
    contact: String,
    directory: Option<String>,
) -> Result<(), Error> {
    let (directory_url, custom_directory) = match directory {
        Some(directory) => (directory, true),
        None => {
            println!("Directory endpoints:");
            for (i, dir) in KNOWN_ACME_DIRECTORIES.iter().enumerate() {
                println!("{}) {}", i, dir.url);
            }

            println!("{}) Custom", KNOWN_ACME_DIRECTORIES.len());
            let mut attempt = 0;
            loop {
                print!("Enter selection: ");
                std::io::stdout().flush()?;

                let mut input = String::new();
                std::io::stdin().read_line(&mut input)?;

                match input.trim().parse::<usize>() {
                    Ok(n) if n < KNOWN_ACME_DIRECTORIES.len() => {
                        break (KNOWN_ACME_DIRECTORIES[n].url.to_owned(), false);
                    }
                    Ok(n) if n == KNOWN_ACME_DIRECTORIES.len() => {
                        input.clear();
                        std::io::stdin().read_line(&mut input)?;
                        break (input.trim().to_owned(), true);
                    }
                    _ => eprintln!("Invalid selection."),
                }

                attempt += 1;
                if attempt >= 3 {
                    bail!("Aborting.");
                }
            }
        }
    };

    println!(
        "Attempting to fetch Terms of Service from {:?}",
        directory_url
    );
    let mut client = AcmeClient::new(directory_url.clone());
    let directory = client.directory().await?;
    let tos_agreed = if let Some(tos_url) = directory.terms_of_service_url() {
        println!("Terms of Service: {}", tos_url);
        print!("Do you agree to the above terms? [y|N]: ");
        std::io::stdout().flush()?;
        let mut input = String::new();
        std::io::stdin().read_line(&mut input)?;
        input.trim().eq_ignore_ascii_case("y")
    } else {
        println!("No Terms of Service found, proceeding.");
        true
    };

    let mut eab_enabled = directory.external_account_binding_required();
    if !eab_enabled && custom_directory {
        print!("Do you want to use external account binding? [y|N]: ");
        std::io::stdout().flush()?;
        let mut input = String::new();
        std::io::stdin().read_line(&mut input)?;
        eab_enabled = input.trim().eq_ignore_ascii_case("y");
    } else if eab_enabled {
        println!("The CA requires external account binding.");
    }

    let eab_creds = if eab_enabled {
        println!("You should have received a key id and a key from your CA.");

        print!("Enter EAB key id: ");
        std::io::stdout().flush()?;
        let mut eab_kid = String::new();
        std::io::stdin().read_line(&mut eab_kid)?;

        print!("Enter EAB key: ");
        std::io::stdout().flush()?;
        let mut eab_hmac_key = String::new();
        std::io::stdin().read_line(&mut eab_hmac_key)?;

        Some((eab_kid.trim().to_owned(), eab_hmac_key.trim().to_owned()))
    } else {
        None
    };

    println!("Attempting to register account with {:?}...", directory_url);

    let account = api2::config::acme::do_register_account(
        &mut client,
        &name,
        tos_agreed,
        contact,
        None,
        eab_creds,
    )
    .await?;

    println!("Registration successful, account URL: {}", account.location);

    Ok(())
}

#[api(
    input: {
        properties: {
            name: { type: AcmeAccountName },
            contact: {
                description: "List of email addresses.",
                type: String,
                optional: true,
            },
        }
    }
)]
/// Update an ACME account.
async fn update_account(param: Value, rpcenv: &mut dyn RpcEnvironment) -> Result<(), Error> {
    let info = &api2::config::acme::API_METHOD_UPDATE_ACCOUNT;
    let result = match info.handler {
        ApiHandler::Sync(handler) => (handler)(param, info, rpcenv)?,
        _ => unreachable!(),
    };

    crate::wait_for_local_worker(result.as_str().unwrap()).await?;

    Ok(())
}

#[api(
    input: {
        properties: {
            name: { type: AcmeAccountName },
            force: {
                description:
                    "Delete account data even if the server refuses to deactivate the account.",
                type: Boolean,
                optional: true,
                default: false,
            },
        }
    }
)]
/// Deactivate an ACME account.
async fn deactivate_account(param: Value, rpcenv: &mut dyn RpcEnvironment) -> Result<(), Error> {
    let info = &api2::config::acme::API_METHOD_DEACTIVATE_ACCOUNT;
    let result = match info.handler {
        ApiHandler::Sync(handler) => (handler)(param, info, rpcenv)?,
        _ => unreachable!(),
    };

    crate::wait_for_local_worker(result.as_str().unwrap()).await?;

    Ok(())
}

pub fn account_cli() -> CommandLineInterface {
    let cmd_def = CliCommandMap::new()
        .insert("list", CliCommand::new(&API_METHOD_LIST_ACCOUNTS))
        .insert(
            "register",
            CliCommand::new(&API_METHOD_REGISTER_ACCOUNT).arg_param(&["name", "contact"]),
        )
        .insert(
            "deactivate",
            CliCommand::new(&API_METHOD_DEACTIVATE_ACCOUNT)
                .arg_param(&["name"])
                .completion_cb("name", crate::config::acme::complete_acme_account),
        )
        .insert(
            "info",
            CliCommand::new(&API_METHOD_GET_ACCOUNT)
                .arg_param(&["name"])
                .completion_cb("name", crate::config::acme::complete_acme_account),
        )
        .insert(
            "update",
            CliCommand::new(&API_METHOD_UPDATE_ACCOUNT)
                .arg_param(&["name"])
                .completion_cb("name", crate::config::acme::complete_acme_account),
        );

    cmd_def.into()
}

#[api(
    input: {
        properties: {
            "output-format": {
                schema: OUTPUT_FORMAT,
                optional: true,
            },
        }
    }
)]
/// List acme plugins.
fn list_plugins(param: Value, rpcenv: &mut dyn RpcEnvironment) -> Result<(), Error> {
    let output_format = get_output_format(&param);

    let info = &api2::config::acme::API_METHOD_LIST_PLUGINS;
    let mut data = match info.handler {
        ApiHandler::Sync(handler) => (handler)(param, info, rpcenv)?,
        _ => unreachable!(),
    };

    let options = default_table_format_options();
    format_and_print_result_full(&mut data, &info.returns, &output_format, &options);

    Ok(())
}

#[api(
    input: {
        properties: {
            id: {
                type: String,
                description: "Plugin ID",
            },
            "output-format": {
                schema: OUTPUT_FORMAT,
                optional: true,
            },
        }
    }
)]
/// Show acme account information.
fn get_plugin(param: Value, rpcenv: &mut dyn RpcEnvironment) -> Result<(), Error> {
    let output_format = get_output_format(&param);

    let info = &api2::config::acme::API_METHOD_GET_PLUGIN;
    let mut data = match info.handler {
        ApiHandler::Sync(handler) => (handler)(param, info, rpcenv)?,
        _ => unreachable!(),
    };

    let options = default_table_format_options();
    format_and_print_result_full(&mut data, &info.returns, &output_format, &options);

    Ok(())
}

#[api(
    input: {
        properties: {
            type: {
                type: String,
                description: "The ACME challenge plugin type.",
            },
            core: {
                type: DnsPluginCore,
                flatten: true,
            },
            data: {
                type: String,
                description: "File containing the plugin data.",
            },
        }
    }
)]
/// Show acme account information.
fn add_plugin(r#type: String, core: DnsPluginCore, data: String) -> Result<(), Error> {
    let data = base64::encode(file_get_contents(data)?);
    api2::config::acme::add_plugin(r#type, core, data)?;
    Ok(())
}

pub fn plugin_cli() -> CommandLineInterface {
    use proxmox_backup::api2::config::acme;
    let cmd_def = CliCommandMap::new()
        .insert("list", CliCommand::new(&API_METHOD_LIST_PLUGINS))
        .insert(
            "config", // name comes from pve/pmg
            CliCommand::new(&API_METHOD_GET_PLUGIN)
                .arg_param(&["id"])
                .completion_cb("id", crate::config::acme::complete_acme_plugin),
        )
        .insert(
            "add",
            CliCommand::new(&API_METHOD_ADD_PLUGIN)
                .arg_param(&["type", "id"])
                .completion_cb("api", crate::config::acme::complete_acme_api_challenge_type)
                .completion_cb("type", crate::config::acme::complete_acme_plugin_type),
        )
        .insert(
            "remove",
            CliCommand::new(&acme::API_METHOD_DELETE_PLUGIN)
                .arg_param(&["id"])
                .completion_cb("id", crate::config::acme::complete_acme_plugin),
        )
        .insert(
            "set",
            CliCommand::new(&acme::API_METHOD_UPDATE_PLUGIN)
                .arg_param(&["id"])
                .completion_cb("id", crate::config::acme::complete_acme_plugin),
        );

    cmd_def.into()
}

#[api(
    input: {
        properties: {
            force: {
                description: "Force renewal even if the certificate does not expire soon.",
                type: Boolean,
                optional: true,
                default: false,
            },
        },
    },
)]
/// Order a new ACME certificate.
async fn order_acme_cert(param: Value, rpcenv: &mut dyn RpcEnvironment) -> Result<(), Error> {
    if !param["force"].as_bool().unwrap_or(false) && !api2::node::certificates::cert_expires_soon()?
    {
        println!("Certificate does not expire within the next 30 days, not renewing.");
        return Ok(());
    }

    let info = &api2::node::certificates::API_METHOD_RENEW_ACME_CERT;
    let result = match info.handler {
        ApiHandler::Sync(handler) => (handler)(param, info, rpcenv)?,
        _ => unreachable!(),
    };

    crate::wait_for_local_worker(result.as_str().unwrap()).await?;

    Ok(())
}

#[api]
/// Order a new ACME certificate.
async fn revoke_acme_cert(param: Value, rpcenv: &mut dyn RpcEnvironment) -> Result<(), Error> {
    let info = &api2::node::certificates::API_METHOD_REVOKE_ACME_CERT;
    let result = match info.handler {
        ApiHandler::Sync(handler) => (handler)(param, info, rpcenv)?,
        _ => unreachable!(),
    };

    crate::wait_for_local_worker(result.as_str().unwrap()).await?;

    Ok(())
}

pub fn cert_cli() -> CommandLineInterface {
    let cmd_def = CliCommandMap::new()
        .insert("order", CliCommand::new(&API_METHOD_ORDER_ACME_CERT))
        .insert("revoke", CliCommand::new(&API_METHOD_REVOKE_ACME_CERT));

    cmd_def.into()
}
