use anyhow::{bail, Error};
use serde_json::{json, Value};

use proxmox_router::{ApiAccess, ApiHandler, ApiMethod, Permission, Router, SubRoute};
use proxmox_schema::format::{dump_enum_properties, get_property_string_type_text};
use proxmox_schema::{ApiStringFormat, ApiType, ObjectSchemaType, Schema};
use proxmox_section_config::dump_section_config;

use pbs_api_types::PRIVILEGES;

use proxmox_backup::api2;

fn get_args() -> (String, Vec<String>) {
    let mut args = std::env::args();
    let prefix = args.next().unwrap();
    let prefix = prefix.rsplit('/').next().unwrap().to_string(); // without path
    let args: Vec<String> = args.collect();

    (prefix, args)
}

fn main() -> Result<(), Error> {
    let (_prefix, args) = get_args();

    if args.is_empty() {
        bail!("missing arguments");
    }

    for arg in args.iter() {
        let text = match arg.as_ref() {
            "apidata.js" => generate_api_tree(),
            "datastore.cfg" => dump_section_config(&pbs_config::datastore::CONFIG),
            "domains.cfg" => dump_section_config(&pbs_config::domains::CONFIG),
            "tape.cfg" => dump_section_config(&pbs_config::drive::CONFIG),
            "tape-job.cfg" => dump_section_config(&pbs_config::tape_job::CONFIG),
            "user.cfg" => dump_section_config(&pbs_config::user::CONFIG),
            "remote.cfg" => dump_section_config(&pbs_config::remote::CONFIG),
            "sync.cfg" => dump_section_config(&pbs_config::sync::CONFIG),
            "verification.cfg" => dump_section_config(&pbs_config::verify::CONFIG),
            "media-pool.cfg" => dump_section_config(&pbs_config::media_pool::CONFIG),
            "config::acl::Role" => dump_enum_properties(&pbs_api_types::Role::API_SCHEMA)?,
            _ => bail!("docgen: got unknown type"),
        };
        println!("{}", text);
    }

    Ok(())
}

fn generate_api_tree() -> String {
    let mut tree = Vec::new();

    let mut data = dump_api_schema(&api2::ROUTER, ".");
    data["path"] = "/".into();
    // hack: add invisible space to sort as first entry
    data["text"] = "&#x200b;Management API (HTTP)".into();
    data["expanded"] = true.into();

    tree.push(data);

    let mut data = dump_api_schema(&api2::backup::BACKUP_API_ROUTER, "/backup/_upgrade_");
    data["path"] = "/backup/_upgrade_".into();
    data["text"] = "Backup API (HTTP/2)".into();
    tree.push(data);

    let mut data = dump_api_schema(&api2::reader::READER_API_ROUTER, "/reader/_upgrade_");
    data["path"] = "/reader/_upgrade_".into();
    data["text"] = "Restore API (HTTP/2)".into();
    tree.push(data);

    format!(
        "var apiSchema = {};",
        serde_json::to_string_pretty(&tree).unwrap()
    )
}

pub fn dump_schema(schema: &Schema) -> Value {
    let mut data;

    match schema {
        Schema::Null => {
            data = json!({
                "type": "null",
            });
        }
        Schema::Boolean(boolean_schema) => {
            data = json!({
                "type": "boolean",
                "description": boolean_schema.description,
            });
            if let Some(default) = boolean_schema.default {
                data["default"] = default.into();
            }
        }
        Schema::String(string_schema) => {
            data = json!({
                "type": "string",
                "description": string_schema.description,
            });
            if let Some(default) = string_schema.default {
                data["default"] = default.into();
            }
            if let Some(min_length) = string_schema.min_length {
                data["minLength"] = min_length.into();
            }
            if let Some(max_length) = string_schema.max_length {
                data["maxLength"] = max_length.into();
            }
            if let Some(type_text) = string_schema.type_text {
                data["typetext"] = type_text.into();
            }
            match string_schema.format {
                None | Some(ApiStringFormat::VerifyFn(_)) => { /* do nothing */ }
                Some(ApiStringFormat::Pattern(const_regex)) => {
                    data["pattern"] = format!("/{}/", const_regex.regex_string).into();
                }
                Some(ApiStringFormat::Enum(variants)) => {
                    let variants: Vec<String> =
                        variants.iter().map(|e| e.value.to_string()).collect();
                    data["enum"] = serde_json::to_value(variants).unwrap();
                }
                Some(ApiStringFormat::PropertyString(subschema)) => {
                    match subschema {
                        Schema::Object(_) | Schema::Array(_) => {
                            data["format"] = dump_schema(subschema);
                            data["typetext"] = get_property_string_type_text(subschema).into();
                        }
                        _ => { /* do nothing  - should not happen */ }
                    };
                }
            }
            // fixme: dump format
        }
        Schema::Integer(integer_schema) => {
            data = json!({
                "type": "integer",
                "description": integer_schema.description,
            });
            if let Some(default) = integer_schema.default {
                data["default"] = default.into();
            }
            if let Some(minimum) = integer_schema.minimum {
                data["minimum"] = minimum.into();
            }
            if let Some(maximum) = integer_schema.maximum {
                data["maximum"] = maximum.into();
            }
        }
        Schema::Number(number_schema) => {
            data = json!({
                "type": "number",
                "description": number_schema.description,
            });
            if let Some(default) = number_schema.default {
                data["default"] = default.into();
            }
            if let Some(minimum) = number_schema.minimum {
                data["minimum"] = minimum.into();
            }
            if let Some(maximum) = number_schema.maximum {
                data["maximum"] = maximum.into();
            }
        }
        Schema::Object(object_schema) => {
            data = dump_property_schema(object_schema);
            data["type"] = "object".into();
            if let Some(default_key) = object_schema.default_key {
                data["default_key"] = default_key.into();
            }
        }
        Schema::Array(array_schema) => {
            data = json!({
                "type": "array",
                "description": array_schema.description,
                "items": dump_schema(array_schema.items),
            });
            if let Some(min_length) = array_schema.min_length {
                data["minLength"] = min_length.into();
            }
            if let Some(max_length) = array_schema.min_length {
                data["maxLength"] = max_length.into();
            }
        }
        Schema::AllOf(alloff_schema) => {
            data = dump_property_schema(alloff_schema);
            data["type"] = "object".into();
        }
    };

    data
}

pub fn dump_property_schema(param: &dyn ObjectSchemaType) -> Value {
    let mut properties = json!({});

    for (prop, optional, schema) in param.properties() {
        let mut property = dump_schema(schema);
        if *optional {
            property["optional"] = 1.into();
        }
        properties[prop] = property;
    }

    let data = json!({
        "description": param.description(),
        "additionalProperties": param.additional_properties(),
        "properties": properties,
    });

    data
}

fn dump_api_permission(permission: &Permission) -> Value {
    match permission {
        Permission::Superuser => json!({ "user": "root@pam" }),
        Permission::User(user) => json!({ "user": user }),
        Permission::Anybody => json!({ "user": "all" }),
        Permission::World => json!({ "user": "world" }),
        Permission::UserParam(param) => json!({ "userParam": param }),
        Permission::Group(group) => json!({ "group": group }),
        Permission::WithParam(param, sub_permission) => {
            json!({
                "withParam": {
                    "name": param,
                    "permissions": dump_api_permission(sub_permission),
                },
            })
        }
        Permission::Privilege(name, value, partial) => {
            let mut privs = Vec::new();
            for (name, v) in PRIVILEGES {
                if (value & v) != 0 {
                    privs.push(name.to_string());
                }
            }

            json!({
                "check": {
                    "path": name,
                    "privs": privs,
                    "partial": partial,
                }
            })
        }
        Permission::And(list) => {
            let list: Vec<Value> = list.iter().map(|p| dump_api_permission(p)).collect();
            json!({ "and": list })
        }
        Permission::Or(list) => {
            let list: Vec<Value> = list.iter().map(|p| dump_api_permission(p)).collect();
            json!({ "or": list })
        }
    }
}

fn dump_api_method_schema(method: &str, api_method: &ApiMethod) -> Value {
    let mut data = json!({
        "description": api_method.parameters.description(),
    });

    data["parameters"] = dump_property_schema(&api_method.parameters);

    let mut returns = dump_schema(api_method.returns.schema);
    if api_method.returns.optional {
        returns["optional"] = 1.into();
    }
    data["returns"] = returns;

    match api_method.access {
        ApiAccess {
            description: None,
            permission: Permission::Superuser,
        } => {
            // no need to output default
        }
        ApiAccess {
            description,
            permission,
        } => {
            let mut permissions = dump_api_permission(permission);
            if let Some(description) = description {
                permissions["description"] = description.into();
            }
            data["permissions"] = permissions;
        }
    }

    let mut method = method;

    if let ApiHandler::AsyncHttp(_) = api_method.handler {
        method = if method == "POST" { "UPLOAD" } else { method };
        method = if method == "GET" { "DOWNLOAD" } else { method };
    }

    data["method"] = method.into();

    data
}

pub fn dump_api_schema(router: &Router, path: &str) -> Value {
    let mut data = json!({});

    let mut info = json!({});
    if let Some(api_method) = router.get {
        info["GET"] = dump_api_method_schema("GET", api_method);
    }
    if let Some(api_method) = router.post {
        info["POST"] = dump_api_method_schema("POST", api_method);
    }
    if let Some(api_method) = router.put {
        info["PUT"] = dump_api_method_schema("PUT", api_method);
    }
    if let Some(api_method) = router.delete {
        info["DELETE"] = dump_api_method_schema("DELETE", api_method);
    }

    data["info"] = info;

    match &router.subroute {
        None => {
            data["leaf"] = 1.into();
        }
        Some(SubRoute::MatchAll { router, param_name }) => {
            let sub_path = if path == "." {
                format!("/{{{}}}", param_name)
            } else {
                format!("{}/{{{}}}", path, param_name)
            };
            let mut child = dump_api_schema(router, &sub_path);
            child["path"] = sub_path.into();
            child["text"] = format!("{{{}}}", param_name).into();

            let children = vec![child];
            data["children"] = children.into();
            data["leaf"] = 0.into();
        }
        Some(SubRoute::Map(dirmap)) => {
            let mut children = Vec::new();

            for (key, sub_router) in dirmap.iter() {
                let sub_path = if path == "." {
                    format!("/{}", key)
                } else {
                    format!("{}/{}", path, key)
                };
                let mut child = dump_api_schema(sub_router, &sub_path);
                child["path"] = sub_path.into();
                child["text"] = key.to_string().into();
                children.push(child);
            }

            data["children"] = children.into();
            data["leaf"] = 0.into();
        }
    }

    data
}
