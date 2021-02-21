use anyhow::{bail, Error};
use serde_json::{json, Value};

use proxmox::{
    api::{
        schema::ObjectSchemaType,
        format::{
            dump_enum_properties,
            dump_section_config,
        },
        ApiMethod,
        ApiHandler,
        Router,
        SubRoute,
    },
};

use proxmox_backup::{
    api2,
    config,
};

fn get_args() -> (String, Vec<String>) {

    let mut args = std::env::args();
    let prefix = args.next().unwrap();
    let prefix = prefix.rsplit('/').next().unwrap().to_string(); // without path
    let args: Vec<String> = args.collect();

    (prefix, args)
}

fn main() -> Result<(), Error> {

    let (_prefix, args) = get_args();

    if args.len() < 1 {
        bail!("missing arguments");
    }

    for arg in args.iter() {
        let text = match arg.as_ref() {
            "apidata.js" => generate_api_tree(),
            "datastore.cfg" => dump_section_config(&config::datastore::CONFIG),
            "tape.cfg" => dump_section_config(&config::drive::CONFIG),
            "tape-job.cfg" => dump_section_config(&config::tape_job::CONFIG),
            "user.cfg" => dump_section_config(&config::user::CONFIG),
            "remote.cfg" => dump_section_config(&config::remote::CONFIG),
            "sync.cfg" => dump_section_config(&config::sync::CONFIG),
            "verification.cfg" => dump_section_config(&config::verify::CONFIG),
            "media-pool.cfg" => dump_section_config(&config::media_pool::CONFIG),
            "config::acl::Role" => dump_enum_properties(&config::acl::Role::API_SCHEMA)?,
            _ => bail!("docgen: got unknown type"),
        };
        println!("{}", text);
    }

    Ok(())
}

fn generate_api_tree() -> String {

    //let api = api2::reader::READER_API_ROUTER;
    let api = api2::ROUTER;

    let mut tree = Vec::new();
    let mut data = dump_api_schema(&api, ".");
    data["path"] = "/".into();
    data["text"] = "/".into();
    data["expanded"] = true.into();

    tree.push(data);

    format!("var pbsapi = {};", serde_json::to_string_pretty(&tree).unwrap())
}

fn dump_api_method_schema(
    method: &str,
    api_method: &ApiMethod,
) -> Value {
    let mut data = json!({
        "description": api_method.parameters.description(),
    });

    //let param_descr = dump_properties(&api_method.parameters, "", style, &[]);

    //let return_descr = dump_api_return_schema(&api_method.returns, style);

    let mut method = method;

    if let ApiHandler::AsyncHttp(_) = api_method.handler {
        method = if method == "POST" { "UPLOAD" } else { method };
        method = if method == "GET" { "DOWNLOAD" } else { method };
    }

    data["method"] = method.into();

    data
}

pub fn dump_api_schema(
    router: &Router,
    path: &str,
) -> Value {

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
        },
        Some(SubRoute::MatchAll { router, param_name }) => {
            let sub_path = if path == "." {
                format!("/{{{}}}", param_name)
            } else {
                format!("{}/{{{}}}", path, param_name)
            };
            let mut child = dump_api_schema(router, &sub_path);
            child["path"] = sub_path.into();
            child["text"] = format!("{{{}}}", param_name).into();

            let mut children = Vec::new();
            children.push(child);
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
