use anyhow::{bail, format_err, Error};

use proxmox_backup::api2;
use proxmox::api::*;
use proxmox::api::schema::*;

// Simply test if api lookup tables inside Routers and Schemas are
// correctly sorted.

fn verify_object_schema(schema: &ObjectSchema) -> Result<(), Error> {

    let map = schema.properties;

    if map.len() >= 1 {

        for i in 1..map.len() {

            if map[i].0 <= map[i-1].0 {
                for (name, _, _) in map.iter() {
                    eprintln!("{}", name);
                }
                bail!("found unsorted property map ({} <= {})", map[i].0, map[i-1].0);
            }
        }
    }

    for (_name, _, sub_schema) in map.iter() {
        verify_schema(sub_schema)?;
    }

    Ok(())
}

fn verify_schema(schema: &Schema) -> Result<(), Error> {
    match schema {
        Schema::Object(obj_schema) => {
            verify_object_schema(obj_schema)?;
        }
        Schema::Array(arr_schema) => {
            verify_schema(arr_schema.items)?;
        }
        _ => {}
    }
    Ok(())
}
fn verify_api_method(
    method: &str,
    path: &str,
    info: &ApiMethod
) -> Result<(), Error>
{
    verify_object_schema(info.parameters)
        .map_err(|err| format_err!("{} {} parameters: {}", method, path, err))?;

    verify_schema(info.returns)
        .map_err(|err| format_err!("{} {} returns: {}", method, path, err))?;

    Ok(())
}

fn verify_dirmap(
    path: &str,
    dirmap: SubdirMap,
) -> Result<(), Error> {

    if dirmap.len() >= 1 {

        for i in 1..dirmap.len() {

            if dirmap[i].0 <= dirmap[i-1].0 {
                for (name, _) in dirmap.iter() {
                    eprintln!("{}/{}", path, name);
                }
                bail!("found unsorted dirmap at {:?} ({} <= {})", path, dirmap[i].0, dirmap[i-1].0);
            }

        }
    }

    for (name, router) in dirmap.iter() {
        let sub_path = format!("{}/{}", path, name);
        verify_router(&sub_path, router)?;
    }

    Ok(())
}

fn verify_router(path: &str, router: &Router) -> Result<(), Error> {

    println!("Verify {}", path);

    if let Some(api_method) = router.get {
        verify_api_method("GET", path, api_method)?;
    }
    if let Some(api_method) = router.put {
        verify_api_method("PUT", path, api_method)?;
    }
    if let Some(api_method) = router.post {
        verify_api_method("POST", path, api_method)?;
    }
    if let Some(api_method) = router.delete {
        verify_api_method("DELETE", path, api_method)?;
    }

    match router.subroute {
        Some(SubRoute::Map(dirmap)) => {
            verify_dirmap(path, dirmap)?;
        }
        Some(SubRoute::MatchAll { router, param_name }) => {
            let path = format!("{}/{{{}}}", path, param_name);
            verify_router(&path, router)?;
        }
        None => {}
    }

    Ok(())
}

#[test]
fn verify_backup_api() -> Result<(), Error> {

    let api = &api2::backup::BACKUP_API_ROUTER;
    verify_router("backup-api", api)?;

    Ok(())
}

#[test]
fn verify_reader_api() -> Result<(), Error> {

    let api = &api2::reader::READER_API_ROUTER;
    verify_router("reader-api", api)?;

    Ok(())
}

#[test]
fn verify_root_api() -> Result<(), Error> {

    let api = &api2::ROUTER;
    verify_router("root", api)?;

    Ok(())
}

#[test]
fn verify_acl_role_schema() -> Result<(), Error> {

    let list = match api2::types::ACL_ROLE_SCHEMA {
        Schema::String(StringSchema { format: Some(ApiStringFormat::Enum(list)), .. }) => list,
        _ => unreachable!(),
    };

    let map = &proxmox_backup::config::acl::ROLE_NAMES;
    for item in *list {
        if !map.contains_key(item) {
            bail!("found role '{}' without description/mapping", item);
        }
    }

    for role in map.keys() {
        if !list.contains(role) {
            bail!("role '{}' missing in ACL_ROLE_SCHEMA enum", role);
        }
    }

    Ok(())
}
