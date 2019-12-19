use failure::*;
use serde_json::Value;

use proxmox::api::{api, cli::*};

use proxmox_backup::tools;
use proxmox_backup::config;
use proxmox_backup::api2::types::*;
use proxmox_backup::client::*;
use proxmox_backup::tools::ticket::*;
use proxmox_backup::auth_helpers::*;


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

fn datastore_commands() -> CommandLineInterface {

    use proxmox_backup::api2;

    let cmd_def = CliCommandMap::new()
        .insert("list", CliCommand::new(&api2::config::datastore::GET))
        .insert("create",
                CliCommand::new(&api2::config::datastore::POST)
                .arg_param(&["name", "path"])
        )
        .insert("remove",
                CliCommand::new(&api2::config::datastore::DELETE)
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

    let uid = nix::unistd::Uid::current();

    let mut client = if uid.is_root()  {
        let ticket = assemble_rsa_ticket(private_auth_key(), "PBS", Some("root@pam"), None)?;
        HttpClient::new("localhost", "root@pam", Some(ticket))?
    } else {
        HttpClient::new("localhost", "root@pam", None)?
    };

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

    let uid = nix::unistd::Uid::current();

    let client = if uid.is_root()  {
        let ticket = assemble_rsa_ticket(private_auth_key(), "PBS", Some("root@pam"), None)?;
        HttpClient::new("localhost", "root@pam", Some(ticket))?
    } else {
        HttpClient::new("localhost", "root@pam", None)?
    };

    let path = format!("api2/json/admin/datastore/{}/gc", store);

    let result = client.get(&path, None).await?;
    let data = &result["data"];
    if output_format == "text" {
        format_and_print_result(&data, "json-pretty");
     } else {
        format_and_print_result(&data, &output_format);
    }

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

fn main() {

    let cmd_def = CliCommandMap::new()
        .insert("datastore", datastore_commands())
        .insert("garbage-collection", garbage_collection_commands());

    run_cli_command(cmd_def);
}
