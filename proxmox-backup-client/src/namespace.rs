use anyhow::{bail, Error};
use serde_json::{json, Value};

use pbs_api_types::BackupNamespace;
use pbs_client::tools::REPO_URL_SCHEMA;

use proxmox_router::cli::{
    format_and_print_result, get_output_format, CliCommand, CliCommandMap, OUTPUT_FORMAT,
};
use proxmox_schema::api;

use crate::{
    complete_namespace, connect, extract_repository_from_value, optional_ns_param,
    record_repository,
};

#[api(
    input: {
        properties: {
            repository: {
                schema: REPO_URL_SCHEMA,
                optional: true,
            },
            ns: {
                type: BackupNamespace,
                optional: true,
            },
            "max-depth": {
                description: "maximum recursion depth",
                optional: true,
            },
            "output-format": {
                schema: OUTPUT_FORMAT,
                optional: true,
            },
        }
    },
)]
/// List namespaces in a repository.
async fn list_namespaces(param: Value, max_depth: Option<usize>) -> Result<(), Error> {
    let output_format = get_output_format(&param);
    let repo = extract_repository_from_value(&param)?;
    let backup_ns = optional_ns_param(&param)?;

    let path = format!("api2/json/admin/datastore/{}/namespace", repo.store());

    let mut param = json!({});

    if let Some(max_depth) = max_depth {
        param["max-depth"] = max_depth.into();
    }

    if !backup_ns.is_root() {
        param["parent"] = serde_json::to_value(backup_ns)?;
    }

    let client = connect(&repo)?;

    let mut result = client.get(&path, Some(param)).await?;

    record_repository(&repo);

    if output_format == "text" {
        let data: Vec<pbs_api_types::NamespaceListItem> =
            serde_json::from_value(result["data"].take())?;
        for entry in data {
            if entry.ns.is_root() {
                continue;
            }

            if let Some(comment) = entry.comment {
                println!("{} ({comment})", entry.ns);
            } else {
                println!("{}", entry.ns);
            }
        }
    } else {
        format_and_print_result(&result, &output_format);
    }

    Ok(())
}

#[api(
    input: {
        properties: {
            repository: {
                schema: REPO_URL_SCHEMA,
                optional: true,
            },
            ns: {
                type: BackupNamespace,
                optional: true,
            },
        }
    },
)]
/// Create a new namespace.
async fn create_namespace(param: Value) -> Result<(), Error> {
    let repo = extract_repository_from_value(&param)?;
    let mut backup_ns = optional_ns_param(&param)?;

    let path = format!("api2/json/admin/datastore/{}/namespace", repo.store());

    let name = match backup_ns.pop() {
        Some(name) => name,
        None => bail!("root namespace is always present"),
    };

    let param = json!({
        "parent": backup_ns,
        "name": name,
    });

    let client = connect(&repo)?;

    let _result = client.post(&path, Some(param)).await?;

    record_repository(&repo);

    Ok(())
}

#[api(
    input: {
        properties: {
            repository: {
                schema: REPO_URL_SCHEMA,
                optional: true,
            },
            ns: {
                type: BackupNamespace,
                optional: true,
            },
        }
    },
)]
/// Delete an existing namespace.
async fn delete_namespace(param: Value) -> Result<(), Error> {
    let repo = extract_repository_from_value(&param)?;
    let backup_ns = optional_ns_param(&param)?;

    if backup_ns.is_root() {
        bail!("root namespace cannot be deleted");
    }

    let path = format!("api2/json/admin/datastore/{}/namespace", repo.store());
    let param = json!({ "ns": backup_ns });

    let client = connect(&repo)?;

    let _result = client.delete(&path, Some(param)).await?;

    record_repository(&repo);

    Ok(())
}

pub fn cli_map() -> CliCommandMap {
    CliCommandMap::new()
        .insert(
            "list",
            CliCommand::new(&API_METHOD_LIST_NAMESPACES)
                .arg_param(&["ns"])
                .completion_cb("ns", complete_namespace),
        )
        .insert(
            "create",
            CliCommand::new(&API_METHOD_CREATE_NAMESPACE)
                .arg_param(&["ns"])
                .completion_cb("ns", complete_namespace),
        )
        .insert(
            "delete",
            CliCommand::new(&API_METHOD_DELETE_NAMESPACE)
                .arg_param(&["ns"])
                .completion_cb("ns", complete_namespace),
        )
}
