use std::path::PathBuf;
use std::collections::HashMap;

use failure::*;
use serde_json::{json, Value};
use chrono::{Local, TimeZone};

use proxmox::api::{api, cli::*, RpcEnvironment, ApiHandler};

use proxmox_backup::configdir;
use proxmox_backup::tools;
use proxmox_backup::config::{self, remote::{self, Remote}};
use proxmox_backup::api2::{self, types::* };
use proxmox_backup::client::*;
use proxmox_backup::tools::ticket::*;
use proxmox_backup::auth_helpers::*;

fn render_epoch(value: &Value, _record: &Value) -> Result<String, Error> {
    if value.is_null() { return Ok(String::new()); }
    let text = match value.as_i64() {
        Some(epoch) => {
            Local.timestamp(epoch, 0).format("%c").to_string()
        }
        None => {
            value.to_string()
        }
    };
    Ok(text)
}

fn render_status(value: &Value, record: &Value) -> Result<String, Error> {
    if record["endtime"].is_null() {
        Ok(value.as_str().unwrap_or("running").to_string())
    } else {
        Ok(value.as_str().unwrap_or("unknown").to_string())
    }
}

async fn view_task_result(
    client: HttpClient,
    result: Value,
    output_format: &str,
) -> Result<(), Error> {
    let data = &result["data"];
    if output_format == "text" {
        if let Some(upid) = data.as_str() {
            display_task_log(client, upid, true).await?;
        }
    } else {
        format_and_print_result(&data, &output_format);
    }

    Ok(())
}

fn connect() -> Result<HttpClient, Error> {

    let uid = nix::unistd::Uid::current();

    let mut options = HttpClientOptions::new()
        .prefix(Some("proxmox-backup".to_string()))
        .verify_cert(false); // not required for connection to localhost

    let client = if uid.is_root()  {
        let ticket = assemble_rsa_ticket(private_auth_key(), "PBS", Some("root@pam"), None)?;
        options = options.password(Some(ticket));
        HttpClient::new("localhost", "root@pam", options)?
    } else {
        options = options.ticket_cache(true).interactive(true);
        HttpClient::new("localhost", "root@pam", options)?
    };

    Ok(client)
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
/// List configured remotes.
fn list_remotes(param: Value, rpcenv: &mut dyn RpcEnvironment) -> Result<Value, Error> {

    let output_format = param["output-format"].as_str().unwrap_or("text").to_owned();

    let info = &api2::config::remote::API_METHOD_LIST_REMOTES;
    let mut data = match info.handler {
        ApiHandler::Sync(handler) => (handler)(param, info, rpcenv)?,
        _ => unreachable!(),
    };

    let options = TableFormatOptions::new()
        .noborder(false)
        .noheader(false)
        .column(ColumnConfig::new("name"))
        .column(ColumnConfig::new("host"))
        .column(ColumnConfig::new("userid"))
        .column(ColumnConfig::new("fingerprint"))
        .column(ColumnConfig::new("comment"));

    format_and_print_result_full(&mut data, info.returns, &output_format, &options);

    Ok(Value::Null)
}

fn remote_commands() -> CommandLineInterface {

    let cmd_def = CliCommandMap::new()
        .insert("list", CliCommand::new(&&API_METHOD_LIST_REMOTES))
        .insert(
            "create",
            // fixme: howto handle password parameter?
            CliCommand::new(&api2::config::remote::API_METHOD_CREATE_REMOTE)
                .arg_param(&["name"])
        )
        .insert(
            "update",
            CliCommand::new(&api2::config::remote::API_METHOD_UPDATE_REMOTE)
                .arg_param(&["name"])
                .completion_cb("name", config::remote::complete_remote_name)
        )
        .insert(
            "remove",
            CliCommand::new(&api2::config::remote::API_METHOD_DELETE_REMOTE)
                .arg_param(&["name"])
                .completion_cb("name", config::remote::complete_remote_name)
        );

    cmd_def.into()
}

fn datastore_commands() -> CommandLineInterface {

    let cmd_def = CliCommandMap::new()
        .insert("list", CliCommand::new(&api2::config::datastore::API_METHOD_LIST_DATASTORES))
        .insert("create",
                CliCommand::new(&api2::config::datastore::API_METHOD_CREATE_DATASTORE)
                .arg_param(&["name", "path"])
        )
        .insert("update",
                CliCommand::new(&api2::config::datastore::API_METHOD_UPDATE_DATASTORE)
                .arg_param(&["name"])
                .completion_cb("name", config::datastore::complete_datastore_name)
        )
        .insert("remove",
                CliCommand::new(&api2::config::datastore::API_METHOD_DELETE_DATASTORE)
                .arg_param(&["name"])
                .completion_cb("name", config::datastore::complete_datastore_name)
        );

    cmd_def.into()
}


#[api(
   input: {
        properties: {
            store: {
                schema: DATASTORE_SCHEMA,
            },
            "output-format": {
                schema: OUTPUT_FORMAT,
                optional: true,
            },
        }
   }
)]
/// Start garbage collection for a specific datastore.
async fn start_garbage_collection(param: Value) -> Result<Value, Error> {

    let output_format = param["output-format"].as_str().unwrap_or("text").to_owned();

    let store = tools::required_string_param(&param, "store")?;

    let mut client = connect()?;

    let path = format!("api2/json/admin/datastore/{}/gc", store);

    let result = client.post(&path, None).await?;

    view_task_result(client, result, &output_format).await?;

    Ok(Value::Null)
}

#[api(
   input: {
        properties: {
            store: {
                schema: DATASTORE_SCHEMA,
            },
            "output-format": {
                schema: OUTPUT_FORMAT,
                optional: true,
            },
        }
   }
)]
/// Show garbage collection status for a specific datastore.
async fn garbage_collection_status(param: Value) -> Result<Value, Error> {

    let output_format = param["output-format"].as_str().unwrap_or("text").to_owned();

    let store = tools::required_string_param(&param, "store")?;

    let client = connect()?;

    let path = format!("api2/json/admin/datastore/{}/gc", store);

    let mut result = client.get(&path, None).await?;
    let mut data = result["data"].take();
    let schema = api2::admin::datastore::API_RETURN_SCHEMA_GARBAGE_COLLECTION_STATUS;

    let options = TableFormatOptions::new()
        .noborder(false)
        .noheader(false);

    format_and_print_result_full(&mut data, schema, &output_format, &options);

    Ok(Value::Null)
}

fn garbage_collection_commands() -> CommandLineInterface {

    let cmd_def = CliCommandMap::new()
        .insert("status",
                CliCommand::new(&API_METHOD_GARBAGE_COLLECTION_STATUS)
                .arg_param(&["store"])
                .completion_cb("store", config::datastore::complete_datastore_name)
        )
        .insert("start",
                CliCommand::new(&API_METHOD_START_GARBAGE_COLLECTION)
                .arg_param(&["store"])
                .completion_cb("store", config::datastore::complete_datastore_name)
        );

    cmd_def.into()
}

#[api(
    input: {
        properties: {
            limit: {
                description: "The maximal number of tasks to list.",
                type: Integer,
                optional: true,
                minimum: 1,
                maximum: 1000,
                default: 50,
            },
            "output-format": {
                schema: OUTPUT_FORMAT,
                optional: true,
            },
            all: {
                type: Boolean,
                description: "Also list stopped tasks.",
                optional: true,
            }
        }
    }
)]
/// List running server tasks.
async fn task_list(param: Value) -> Result<Value, Error> {

    let output_format = param["output-format"].as_str().unwrap_or("text").to_owned();

    let client = connect()?;

    let limit = param["limit"].as_u64().unwrap_or(50) as usize;
    let running = !param["all"].as_bool().unwrap_or(false);
    let args = json!({
        "running": running,
        "start": 0,
        "limit": limit,
    });
    let mut result = client.get("api2/json/nodes/localhost/tasks", Some(args)).await?;

    let mut data = result["data"].take();
    let schema = api2::node::tasks::API_RETURN_SCHEMA_LIST_TASKS;

    let options = TableFormatOptions::new()
        .noborder(false)
        .noheader(false)
        .column(ColumnConfig::new("starttime").right_align(false).renderer(render_epoch))
        .column(ColumnConfig::new("endtime").right_align(false).renderer(render_epoch))
        .column(ColumnConfig::new("upid"))
        .column(ColumnConfig::new("status").renderer(render_status));

    format_and_print_result_full(&mut data, schema, &output_format, &options);

    Ok(Value::Null)
}

#[api(
    input: {
        properties: {
            upid: {
                schema: UPID_SCHEMA,
            },
        }
    }
)]
/// Display the task log.
async fn task_log(param: Value) -> Result<Value, Error> {

    let upid = tools::required_string_param(&param, "upid")?;

    let client = connect()?;

    display_task_log(client, upid, true).await?;

    Ok(Value::Null)
}

#[api(
    input: {
        properties: {
            upid: {
                schema: UPID_SCHEMA,
            },
        }
    }
)]
/// Try to stop a specific task.
async fn task_stop(param: Value) -> Result<Value, Error> {

    let upid_str = tools::required_string_param(&param, "upid")?;

    let mut client = connect()?;

    let path = format!("api2/json/nodes/localhost/tasks/{}", upid_str);
    let _ = client.delete(&path, None).await?;

    Ok(Value::Null)
}

fn task_mgmt_cli() -> CommandLineInterface {

    let task_log_cmd_def = CliCommand::new(&API_METHOD_TASK_LOG)
        .arg_param(&["upid"]);

    let task_stop_cmd_def = CliCommand::new(&API_METHOD_TASK_STOP)
        .arg_param(&["upid"]);

    let cmd_def = CliCommandMap::new()
        .insert("list", CliCommand::new(&API_METHOD_TASK_LIST))
        .insert("log", task_log_cmd_def)
        .insert("stop", task_stop_cmd_def);

    cmd_def.into()
}

fn x509name_to_string(name: &openssl::x509::X509NameRef) -> Result<String, Error> {
    let mut parts = Vec::new();
    for entry in name.entries() {
        parts.push(format!("{} = {}", entry.object().nid().short_name()?, entry.data().as_utf8()?));
    }
    Ok(parts.join(", "))
}

#[api]
/// Diplay node certificate information.
fn cert_info() -> Result<(), Error> {

    let cert_path = PathBuf::from(configdir!("/proxy.pem"));

    let cert_pem = proxmox::tools::fs::file_get_contents(&cert_path)?;

    let cert = openssl::x509::X509::from_pem(&cert_pem)?;

    println!("Subject: {}", x509name_to_string(cert.subject_name())?);

    if let Some(san) = cert.subject_alt_names() {
        for name in san.iter() {
            if let Some(v) = name.dnsname() {
                println!("    DNS:{}", v);
            } else if let Some(v) = name.ipaddress() {
                println!("    IP:{:?}", v);
            } else if let Some(v) = name.email() {
                println!("    EMAIL:{}", v);
            } else if let Some(v) = name.uri() {
                println!("    URI:{}", v);
            }
        }
    }

    println!("Issuer: {}", x509name_to_string(cert.issuer_name())?);
    println!("Validity:");
    println!("    Not Before: {}", cert.not_before());
    println!("    Not After : {}", cert.not_after());

    let fp = cert.digest(openssl::hash::MessageDigest::sha256())?;
    let fp_string = proxmox::tools::digest_to_hex(&fp);
    let fp_string = fp_string.as_bytes().chunks(2).map(|v| std::str::from_utf8(v).unwrap())
        .collect::<Vec<&str>>().join(":");

    println!("Fingerprint (sha256): {}", fp_string);

    let pubkey = cert.public_key()?;
    println!("Public key type: {}", openssl::nid::Nid::from_raw(pubkey.id().as_raw()).long_name()?);
    println!("Public key bits: {}", pubkey.bits());

    Ok(())
}

#[api(
    input: {
        properties: {
            force: {
	        description: "Force generation of new SSL certifate.",
	        type:  Boolean,
	        optional:true,
	    },
        }
    },
)]
/// Update node certificates and generate all needed files/directories.
fn update_certs(force: Option<bool>) -> Result<(), Error> {

    config::create_configdir()?;

    if let Err(err) = generate_auth_key() {
        bail!("unable to generate auth key - {}", err);
    }

    if let Err(err) = generate_csrf_key() {
        bail!("unable to generate csrf key - {}", err);
    }

    config::update_self_signed_cert(force.unwrap_or(false))?;

    Ok(())
}

fn cert_mgmt_cli() -> CommandLineInterface {

    let cmd_def = CliCommandMap::new()
        .insert("info", CliCommand::new(&API_METHOD_CERT_INFO))
        .insert("update", CliCommand::new(&API_METHOD_UPDATE_CERTS));

    cmd_def.into()
}

// fixme: avoid API redefinition
#[api(
   input: {
        properties: {
            "local-store": {
                schema: DATASTORE_SCHEMA,
            },
            remote: {
                schema: REMOTE_ID_SCHEMA,
            },
            "remote-store": {
                schema: DATASTORE_SCHEMA,
            },
            delete: {
                description: "Delete vanished backups. This remove the local copy if the remote backup was deleted.",
                type: Boolean,
                optional: true,
                default: true,
            },
            "output-format": {
                schema: OUTPUT_FORMAT,
                optional: true,
            },
        }
   }
)]
/// Sync datastore from another repository
async fn pull_datastore(
    remote: String,
    remote_store: String,
    local_store: String,
    delete: Option<bool>,
    output_format: Option<String>,
) -> Result<Value, Error> {

    let output_format = output_format.unwrap_or("text".to_string());

    let mut client = connect()?;

    let mut args = json!({
        "store": local_store,
        "remote": remote,
        "remote-store": remote_store,
    });

    if let Some(delete) = delete {
        args["delete"] = delete.into();
    }

    let result = client.post("api2/json/pull", Some(args)).await?;

    view_task_result(client, result, &output_format).await?;

    Ok(Value::Null)
}

fn main() {

    let cmd_def = CliCommandMap::new()
        .insert("datastore", datastore_commands())
        .insert("remote", remote_commands())
        .insert("garbage-collection", garbage_collection_commands())
        .insert("cert", cert_mgmt_cli())
        .insert("task", task_mgmt_cli())
        .insert(
            "pull",
            CliCommand::new(&API_METHOD_PULL_DATASTORE)
                .arg_param(&["remote", "remote-store", "local-store"])
                .completion_cb("local-store", config::datastore::complete_datastore_name)
                .completion_cb("remote", config::remote::complete_remote_name)
                .completion_cb("remote-store", complete_remote_datastore_name)
        );

    proxmox_backup::tools::runtime::main(run_async_cli_command(cmd_def));
}

// shell completion helper
pub fn complete_remote_datastore_name(_arg: &str, param: &HashMap<String, String>) -> Vec<String> {

    let mut list = Vec::new();

    let _ = proxmox::try_block!({
        let remote = param.get("remote").ok_or_else(|| format_err!("no remote"))?;
        let (remote_config, _digest) = remote::config()?;

        let remote: Remote = remote_config.lookup("remote", &remote)?;

        let options = HttpClientOptions::new()
            .password(Some(remote.password.clone()))
            .fingerprint(remote.fingerprint.clone());

        let client = HttpClient::new(
            &remote.host,
            &remote.userid,
            options,
        )?;

        let mut rt = tokio::runtime::Runtime::new().unwrap();
        let result = rt.block_on(client.get("api2/json/admin/datastore", None))?;

        if let Some(data) = result["data"].as_array() {
            for item in data {
                if let Some(store) = item["store"].as_str() {
                    list.push(store.to_owned());
                }
            }
        }

        Ok(())
    }).map_err(|_err: Error| { /* ignore */ });

    list
}
