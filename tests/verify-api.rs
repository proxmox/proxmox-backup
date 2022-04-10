use std::collections::HashSet;

use anyhow::{bail, format_err, Error};

use proxmox_router::{ApiMethod, Permission, Router, SubRoute, SubdirMap};
use proxmox_schema::*;

use proxmox_backup::api2;

// Simply test if api lookup tables inside Routers and Schemas are
// correctly sorted.

fn verify_object_schema(schema: &ObjectSchema) -> Result<(), Error> {
    let map = schema.properties;

    if !map.is_empty() {
        for i in 1..map.len() {
            if map[i].0 <= map[i - 1].0 {
                for (name, _, _) in map.iter() {
                    eprintln!("{}", name);
                }
                bail!(
                    "found unsorted property map ({} <= {})",
                    map[i].0,
                    map[i - 1].0
                );
            }
        }
    }

    for (_name, _, sub_schema) in map.iter() {
        verify_schema(sub_schema)?;
    }

    Ok(())
}

// verify entries in an AllOf schema are actually object schemas and that they don't contain
// duplicate keys
fn verify_all_of_schema(schema: &AllOfSchema) -> Result<(), Error> {
    for entry in schema.list {
        match entry {
            Schema::Object(obj) => verify_object_schema(obj)?,
            Schema::AllOf(allof) => verify_all_of_schema(allof)?,
            _ => bail!("AllOf schema with a non-object schema entry!"),
        }
    }

    let mut keys = HashSet::<&'static str>::new();
    let mut dupes = String::new();
    for property in schema.properties() {
        if !keys.insert(property.0) {
            if !dupes.is_empty() {
                dupes.push_str(", ");
            }
            dupes.push_str(property.0);
        }
    }
    if !dupes.is_empty() {
        bail!("Duplicate keys found in AllOf schema: {}", dupes);
    }

    Ok(())
}

fn verify_schema(schema: &Schema) -> Result<(), Error> {
    match schema {
        Schema::Object(obj_schema) => {
            verify_object_schema(obj_schema)?;
        }
        Schema::AllOf(all_of_schema) => {
            verify_all_of_schema(all_of_schema)?;
        }
        Schema::Array(arr_schema) => {
            verify_schema(arr_schema.items)?;
        }
        _ => {}
    }
    Ok(())
}

fn verify_access_permissions(permission: &Permission) -> Result<(), Error> {
    match permission {
        Permission::Or(list) => {
            for perm in list.iter() {
                verify_access_permissions(perm)?;
            }
        }
        Permission::And(list) => {
            for perm in list.iter() {
                verify_access_permissions(perm)?;
            }
        }
        Permission::Privilege(path_comp, ..) => {
            let path = format!("/{}", path_comp.join("/"));
            pbs_config::acl::check_acl_path(&path)?;
        }
        _ => {}
    }
    Ok(())
}

fn verify_api_method(method: &str, path: &str, info: &ApiMethod) -> Result<(), Error> {
    match &info.parameters {
        ParameterSchema::Object(obj) => {
            verify_object_schema(obj)
                .map_err(|err| format_err!("{} {} parameters: {}", method, path, err))?;
        }
        ParameterSchema::AllOf(all_of) => {
            verify_all_of_schema(all_of)
                .map_err(|err| format_err!("{} {} parameters: {}", method, path, err))?;
        }
    }

    verify_schema(info.returns.schema)
        .map_err(|err| format_err!("{} {} returns: {}", method, path, err))?;

    verify_access_permissions(info.access.permission)
        .map_err(|err| format_err!("{} {} access: {}", method, path, err))?;

    Ok(())
}

fn verify_dirmap(path: &str, dirmap: SubdirMap) -> Result<(), Error> {
    if !dirmap.is_empty() {
        for i in 1..dirmap.len() {
            if dirmap[i].0 <= dirmap[i - 1].0 {
                for (name, _) in dirmap.iter() {
                    eprintln!("{}/{}", path, name);
                }
                bail!(
                    "found unsorted dirmap at {:?} ({} <= {})",
                    path,
                    dirmap[i].0,
                    dirmap[i - 1].0
                );
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
