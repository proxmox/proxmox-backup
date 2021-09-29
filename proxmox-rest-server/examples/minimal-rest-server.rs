use std::sync::{Arc, Mutex};
use std::collections::HashMap;

use anyhow::{bail, format_err, Error};
use futures::{FutureExt, TryFutureExt};
use lazy_static::lazy_static;

use proxmox::api::{api, router::SubdirMap, Router, RpcEnvironmentType, UserInformation};
use proxmox::list_subdirs_api_method;
use proxmox_rest_server::{ApiAuth, ApiConfig, AuthError, RestServer};

// Create a Dummy User info and auth system
// Normally this would check and authenticate the user
struct DummyUserInfo;

impl UserInformation for DummyUserInfo {
    fn is_superuser(&self, _userid: &str) -> bool {
        true
    }
    fn is_group_member(&self, _userid: &str, group: &str) -> bool {
        group == "Group"
    }
    fn lookup_privs(&self, _userid: &str, _path: &[&str]) -> u64 {
        u64::MAX
    }
}

struct DummyAuth;

impl ApiAuth for DummyAuth {
    fn check_auth(
        &self,
        _headers: &http::HeaderMap,
        _method: &hyper::Method,
    ) -> Result<(String, Box<dyn UserInformation + Sync + Send>), AuthError> {
        // get some global/cached userinfo
        let userinfo = DummyUserInfo;
        // Do some user checks, e.g. cookie/csrf
        Ok(("User".to_string(), Box::new(userinfo)))
    }
}

// this should return the index page of the webserver
// iow. what the user browses to

fn get_index(
    _auth_id: Option<String>,
    _language: Option<String>,
    _api: &ApiConfig,
    _parts: http::request::Parts,
) -> http::Response<hyper::Body> {
    // build an index page
    http::Response::builder()
        .body("hello world".into())
        .unwrap()
}

// a few examples on how to do api calls with the Router

#[api]
/// A simple ping method. returns "pong"
fn ping() -> Result<String, Error> {
    Ok("pong".to_string())
}

lazy_static! {
    static ref ITEM_MAP: Mutex<HashMap<String, String>> = Mutex::new(HashMap::new());
}

#[api]
/// Lists all current items
fn list_items() -> Result<Vec<String>, Error> {
    Ok(ITEM_MAP.lock().unwrap().keys().map(|k| k.clone()).collect())
}

#[api(
    input: {
        properties: {
            name: {
                type: String,
                description: "The name",
            },
            value: {
                type: String,
                description: "The value",
            },
        },
    },
)]
/// creates a new item
fn create_item(name: String, value: String) -> Result<(), Error> {
    let mut map = ITEM_MAP.lock().unwrap();
    if map.contains_key(&name) {
        bail!("{} already exists", name);
    }

    map.insert(name, value);

    Ok(())
}

#[api(
    input: {
        properties: {
            name: {
                type: String,
                description: "The name",
            },
        },
    },
)]
/// returns the value of an item
fn get_item(name: String) -> Result<String, Error> {
    ITEM_MAP.lock().unwrap().get(&name).map(|s| s.to_string()).ok_or_else(|| format_err!("no such item '{}'", name))
}

#[api(
    input: {
        properties: {
            name: {
                type: String,
                description: "The name",
            },
            value: {
                type: String,
                description: "The value",
            },
        },
    },
)]
/// updates an item
fn update_item(name: String, value: String) -> Result<(), Error> {
    if let Some(val) = ITEM_MAP.lock().unwrap().get_mut(&name) {
        *val = value;
    } else {
        bail!("no such item '{}'", name);
    }
    Ok(())
}

#[api(
    input: {
        properties: {
            name: {
                type: String,
                description: "The name",
            },
        },
    },
)]
/// deletes an item
fn delete_item(name: String) -> Result<(), Error> {
    if ITEM_MAP.lock().unwrap().remove(&name).is_none() {
        bail!("no such item '{}'", name);
    }
    Ok(())
}

const ITEM_ROUTER: Router = Router::new()
    .get(&API_METHOD_GET_ITEM)
    .put(&API_METHOD_UPDATE_ITEM)
    .delete(&API_METHOD_DELETE_ITEM);

const SUBDIRS: SubdirMap = &[
    (
        "items",
        &Router::new()
            .get(&API_METHOD_LIST_ITEMS)
            .post(&API_METHOD_CREATE_ITEM)
            .match_all("name", &ITEM_ROUTER)
    ),
    (
        "ping",
        &Router::new()
            .get(&API_METHOD_PING)
    ),
];

const ROUTER: Router = Router::new()
    .get(&list_subdirs_api_method!(SUBDIRS))
    .subdirs(SUBDIRS);

async fn run() -> Result<(), Error> {

    // we first have to configure the api environment (basedir etc.)

    let config = ApiConfig::new(
        "/var/tmp/",
        &ROUTER,
        RpcEnvironmentType::PUBLIC,
        Arc::new(DummyAuth {}),
        get_index,
    )?;
    let rest_server = RestServer::new(config);

    // then we have to create a daemon that listens, accepts and serves
    // the api to clients
    proxmox_rest_server::daemon::create_daemon(
        ([127, 0, 0, 1], 65000).into(),
        move |listener, ready| {
            let incoming = hyper::server::conn::AddrIncoming::from_listener(listener)?;

            Ok(ready
                .and_then(|_| hyper::Server::builder(incoming)
                    .serve(rest_server)
                    .map_err(Error::from)
                )
                .map_err(|err| eprintln!("ERR: {}", err))
                .map(|test| println!("OK: {}", test.is_ok())))
        },
        "example_server",
    ).await?;

    Ok(())
}

fn main() -> Result<(), Error> {
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(async { run().await })
}
