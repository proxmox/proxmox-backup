use anyhow::{bail, format_err, Error};
use hyper::Method;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use std::collections::HashMap;

use proxmox_router::{cli::*, ApiHandler, ApiMethod, RpcEnvironment, SubRoute};
use proxmox_schema::format::DocumentationFormat;
use proxmox_schema::{api, ApiType, ParameterSchema, Schema};

use pbs_api_types::PROXMOX_UPID_REGEX;
use pbs_client::view_task_result;
use proxmox_rest_server::normalize_path_with_components;

use proxmox_backup::client_helpers::connect_to_localhost;

const PROG_NAME: &str = "proxmox-backup-debug api";
const URL_ASCIISET: percent_encoding::AsciiSet = percent_encoding::NON_ALPHANUMERIC.remove(b'/');

macro_rules! complete_api_path {
    ($capability:expr) => {
        |complete_me: &str, _map: &HashMap<String, String>| {
            proxmox_async::runtime::block_on(async {
                complete_api_path_do(complete_me, $capability).await
            })
        }
    };
}

async fn complete_api_path_do(mut complete_me: &str, capability: Option<&str>) -> Vec<String> {
    if complete_me.is_empty() {
        complete_me = "/";
    }

    let mut list = Vec::new();

    let mut lookup_path = complete_me.to_string();
    let mut filter = "";
    let last_path_index = complete_me.rfind('/');
    if let Some(index) = last_path_index {
        if index != complete_me.len() - 1 {
            lookup_path = complete_me[..(index + 1)].to_string();
            if index < complete_me.len() - 1 {
                filter = &complete_me[(index + 1)..];
            }
        }
    }

    let uid = nix::unistd::Uid::current();

    let username = match nix::unistd::User::from_uid(uid) {
        Ok(Some(user)) => user.name,
        _ => "root@pam".to_string(),
    };
    let mut rpcenv = CliEnvironment::new();
    rpcenv.set_auth_id(Some(format!("{}@pam", username)));

    while let Ok(children) = get_api_children(lookup_path.clone(), &mut rpcenv).await {
        let old_len = list.len();
        for entry in children {
            let name = entry.name;
            let caps = entry.capabilities;

            if filter.is_empty() || name.starts_with(filter) {
                let mut path = format!("{}{}", lookup_path, name);
                if caps.contains('D') {
                    path.push('/');
                    list.push(path.clone());
                } else if let Some(cap) = capability {
                    if caps.contains(cap) {
                        list.push(path);
                    }
                } else {
                    list.push(path);
                }
            }
        }

        if list.len() == 1 && old_len != 1 && list[0].ends_with('/') {
            // we added only one match and it was a directory, lookup again
            lookup_path = list[0].clone();
            filter = "";
            continue;
        }

        break;
    }

    list
}

async fn get_child_links(
    path: &str,
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<Vec<String>, Error> {
    let (path, components) = normalize_path_with_components(path)?;

    let info = &proxmox_backup::api2::ROUTER
        .find_route(&components, &mut HashMap::new())
        .ok_or_else(|| format_err!("no such resource"))?;

    match info.subroute {
        Some(SubRoute::Map(map)) => Ok(map.iter().map(|(name, _)| name.to_string()).collect()),
        Some(SubRoute::MatchAll { param_name, .. }) => {
            let list = call_api("get", &path, rpcenv, None).await?;
            Ok(list
                .as_array()
                .ok_or_else(|| format_err!("{} did not return an array", path))?
                .iter()
                .map(|item| {
                    item[param_name]
                        .as_str()
                        .map(|c| c.to_string())
                        .ok_or_else(|| format_err!("no such property {}", param_name))
                })
                .collect::<Result<Vec<_>, _>>()?)
        }
        None => bail!("link does not define child links"),
    }
}

fn get_api_method(
    method: &str,
    path: &str,
) -> Result<(&'static ApiMethod, HashMap<String, String>), Error> {
    let method = match method {
        "get" => Method::GET,
        "set" => Method::PUT,
        "create" => Method::POST,
        "delete" => Method::DELETE,
        _ => unreachable!(),
    };
    let mut uri_param = HashMap::new();
    let (path, components) = normalize_path_with_components(path)?;
    if let Some(method) =
        &proxmox_backup::api2::ROUTER.find_method(&components, method.clone(), &mut uri_param)
    {
        Ok((method, uri_param))
    } else {
        bail!("no {} handler defined for '{}'", method, path);
    }
}

fn merge_parameters(
    uri_param: &HashMap<String, String>,
    param: Option<Value>,
    schema: ParameterSchema,
) -> Result<Value, Error> {
    let mut param_list: Vec<(String, String)> = uri_param
        .iter()
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();

    if let Some(Value::Object(map)) = param {
        param_list.extend(map.into_iter().map(|(k, v)| {
            (
                k,
                match v {
                    Value::String(s) => s,
                    _ => unreachable!(), // we're in the CLI
                },
            )
        }));
    }

    let params = schema.parse_parameter_strings(&param_list, true)?;

    Ok(params)
}

fn use_http_client() -> bool {
    match std::env::var("PROXMOX_DEBUG_API_CODE") {
        Ok(var) => var != "1",
        _ => true,
    }
}

async fn call_api(
    method: &str,
    path: &str,
    rpcenv: &mut dyn RpcEnvironment,
    params: Option<Value>,
) -> Result<Value, Error> {
    let (api_method, uri_params) = get_api_method(method, path)?;
    let mut params = merge_parameters(&uri_params, params, api_method.parameters)?;

    if use_http_client() {
        // remove url parameters here
        for (param, _) in uri_params {
            params.as_object_mut().unwrap().remove(&param);
        }
        return call_api_http(method, path, Some(params)).await;
    }

    call_api_code(api_method, rpcenv, params).await
}

async fn call_api_http(method: &str, path: &str, params: Option<Value>) -> Result<Value, Error> {
    let client = connect_to_localhost()?;

    let path = format!(
        "api2/json/{}",
        percent_encoding::utf8_percent_encode(path, &URL_ASCIISET)
    );

    match method {
        "get" => client.get(&path, params).await,
        "create" => client.post(&path, params).await,
        "set" => client.put(&path, params).await,
        "delete" => client.delete(&path, params).await,
        _ => unreachable!(),
    }
    .map(|mut res| res["data"].take())
}

async fn call_api_code(
    method: &'static ApiMethod,
    rpcenv: &mut dyn RpcEnvironment,
    params: Value,
) -> Result<Value, Error> {
    if !method.protected {
        // drop privileges if we call non-protected code directly
        let backup_user = pbs_config::backup_user()?;
        nix::unistd::setgid(backup_user.gid)?;
        nix::unistd::setuid(backup_user.uid)?;
    }
    match method.handler {
        ApiHandler::StreamingSync(handler) => {
            let res = (handler)(params, method, rpcenv)?.to_value()?;
            Ok(res)
        }
        ApiHandler::StreamingAsync(handler) => {
            let res = (handler)(params, method, rpcenv).await?.to_value()?;
            Ok(res)
        }
        ApiHandler::AsyncHttp(_handler) => {
            bail!("not implemented");
        }
        ApiHandler::Sync(handler) => (handler)(params, method, rpcenv),
        ApiHandler::Async(handler) => (handler)(params, method, rpcenv).await,
        _ => {
            bail!("Unknown API handler type");
        }
    }
}

async fn call_api_and_format_result(
    method: String,
    path: String,
    mut param: Value,
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<(), Error> {
    let mut output_format = extract_output_format(&mut param);
    let mut result = call_api(&method, &path, rpcenv, Some(param)).await?;

    if let Some(upid) = result.as_str() {
        if PROXMOX_UPID_REGEX.is_match(upid) {
            if use_http_client() {
                let client = connect_to_localhost()?;
                view_task_result(&client, json!({ "data": upid }), &output_format).await?;
                return Ok(());
            }

            proxmox_rest_server::handle_worker(upid).await?;

            if output_format == "text" {
                return Ok(());
            }
        }
    }

    let (method, _) = get_api_method(&method, &path)?;
    let options = default_table_format_options();
    let return_type = &method.returns;
    if matches!(return_type.schema, Schema::Null) {
        output_format = "json-pretty".to_string();
    }

    format_and_print_result_full(&mut result, return_type, &output_format, &options);

    Ok(())
}

#[api(
    input: {
        additional_properties: true,
        properties: {
            method: {
                type: String,
                description: "The Method",
            },
            "api-path": {
                type: String,
                description: "API path.",
            },
            "output-format": {
                schema: OUTPUT_FORMAT,
                optional: true,
            },
        },
    },
)]
/// Call API on `<api-path>`
async fn api_call(
    method: String,
    api_path: String,
    param: Value,
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<(), Error> {
    call_api_and_format_result(method, api_path, param, rpcenv).await
}

#[api(
    input: {
        properties: {
            path: {
                type: String,
                description: "API path.",
            },
            verbose: {
                type: Boolean,
                description: "Verbose output format.",
                optional: true,
                default: false,
            }
        },
    },
)]
/// Get API usage information for `<path>`
async fn usage(
    path: String,
    verbose: bool,
    _param: Value,
    _rpcenv: &mut dyn RpcEnvironment,
) -> Result<(), Error> {
    let docformat = if verbose {
        DocumentationFormat::Full
    } else {
        DocumentationFormat::Short
    };
    let mut found = false;
    for command in &["get", "set", "create", "delete"] {
        let (info, uri_params) = match get_api_method(command, &path) {
            Ok(some) => some,
            Err(_) => continue,
        };
        found = true;

        let skip_params: Vec<&str> = uri_params.keys().map(|s| &**s).collect();

        let cmd = CliCommand::new(info);
        let prefix = format!("USAGE: {} {} {}", PROG_NAME, command, path);

        print!(
            "{}",
            generate_usage_str(&prefix, &cmd, docformat, "", &skip_params)
        );
    }

    if !found {
        bail!("no such resource '{}'", path);
    }
    Ok(())
}

#[api()]
#[derive(Debug, Serialize, Deserialize)]
/// A child link with capabilities
struct ApiDirEntry {
    /// The name of the link
    name: String,
    /// The capabilities of the path (format Drwcd)
    capabilities: String,
}

const LS_SCHEMA: &proxmox_schema::Schema =
    &proxmox_schema::ArraySchema::new("List of child links", &ApiDirEntry::API_SCHEMA).schema();

async fn get_api_children(
    path: String,
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<Vec<ApiDirEntry>, Error> {
    let mut res = Vec::new();
    for link in get_child_links(&path, rpcenv).await? {
        let path = format!("{}/{}", path, link);
        let (path, _) = normalize_path_with_components(&path)?;
        let mut cap = String::new();

        if get_child_links(&path, rpcenv).await.is_ok() {
            cap.push('D');
        } else {
            cap.push('-');
        }

        let cap_list = &[("get", 'r'), ("set", 'w'), ("create", 'c'), ("delete", 'd')];

        for (method, c) in cap_list {
            if get_api_method(method, &path).is_ok() {
                cap.push(*c);
            } else {
                cap.push('-');
            }
        }

        res.push(ApiDirEntry {
            name: link.to_string(),
            capabilities: cap,
        });
    }

    Ok(res)
}

#[api(
    input: {
        properties: {
            path: {
                type: String,
                description: "API path.",
                optional: true,
            },
            "output-format": {
                schema: OUTPUT_FORMAT,
                optional: true,
            },
        },
    },
)]
/// Get API usage information for `<path>`
async fn ls(
    path: Option<String>,
    mut param: Value,
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<(), Error> {
    let output_format = extract_output_format(&mut param);

    let options = TableFormatOptions::new()
        .noborder(true)
        .noheader(true)
        .sortby("name", false);

    let path = path.unwrap_or_else(|| "".into());
    let res = get_api_children(path, rpcenv).await?;

    format_and_print_result_full(
        &mut serde_json::to_value(res)?,
        &proxmox_schema::ReturnType {
            optional: false,
            schema: LS_SCHEMA,
        },
        &output_format,
        &options,
    );

    Ok(())
}

pub fn api_commands() -> CommandLineInterface {
    let cmd_def = CliCommandMap::new()
        .insert(
            "get",
            CliCommand::new(&API_METHOD_API_CALL)
                .fixed_param("method", "get".to_string())
                .arg_param(&["api-path"])
                .completion_cb("api-path", complete_api_path!(Some("r"))),
        )
        .insert(
            "set",
            CliCommand::new(&API_METHOD_API_CALL)
                .fixed_param("method", "set".to_string())
                .arg_param(&["api-path"])
                .completion_cb("api-path", complete_api_path!(Some("w"))),
        )
        .insert(
            "create",
            CliCommand::new(&API_METHOD_API_CALL)
                .fixed_param("method", "create".to_string())
                .arg_param(&["api-path"])
                .completion_cb("api-path", complete_api_path!(Some("c"))),
        )
        .insert(
            "delete",
            CliCommand::new(&API_METHOD_API_CALL)
                .fixed_param("method", "delete".to_string())
                .arg_param(&["api-path"])
                .completion_cb("api-path", complete_api_path!(Some("d"))),
        )
        .insert(
            "ls",
            CliCommand::new(&API_METHOD_LS)
                .arg_param(&["path"])
                .completion_cb("path", complete_api_path!(Some("D"))),
        )
        .insert(
            "usage",
            CliCommand::new(&API_METHOD_USAGE)
                .arg_param(&["path"])
                .completion_cb("path", complete_api_path!(None)),
        );

    cmd_def.into()
}
