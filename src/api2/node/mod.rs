//! Server/Node Configuration and Administration

use std::net::TcpListener;
use std::os::unix::io::AsRawFd;

use anyhow::{bail, format_err, Error};
use futures::future::{FutureExt, TryFutureExt};
use hyper::body::Body;
use hyper::http::request::Parts;
use hyper::upgrade::Upgraded;
use hyper::Request;
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, BufReader};

use proxmox_auth_api::ticket::{Empty, Ticket};
use proxmox_auth_api::types::Authid;
use proxmox_http::websocket::WebSocket;
use proxmox_rest_server::WorkerTask;
use proxmox_router::list_subdirs_api_method;
use proxmox_router::{
    ApiHandler, ApiMethod, ApiResponseFuture, Permission, Router, RpcEnvironment, SubdirMap,
};
use proxmox_schema::*;
use proxmox_sortable_macro::sortable;
use proxmox_sys::fd::fd_change_cloexec;

use pbs_api_types::{NODE_SCHEMA, PRIV_SYS_CONSOLE};

use crate::auth::{private_auth_keyring, public_auth_keyring};
use crate::tools;

pub mod apt;
pub mod certificates;
pub mod config;
pub mod disks;
pub mod dns;
pub mod network;
pub mod subscription;
pub mod tasks;

pub(crate) mod rrd;

mod journal;
mod report;
pub(crate) mod services;
mod status;
mod syslog;
mod time;

pub const SHELL_CMD_SCHEMA: Schema = StringSchema::new("The command to run.")
    .format(&ApiStringFormat::Enum(&[
        EnumEntry::new("login", "Login"),
        EnumEntry::new("upgrade", "Upgrade"),
    ]))
    .schema();

#[api(
    protected: true,
    input: {
        properties: {
            node: {
                schema: NODE_SCHEMA,
            },
            cmd: {
                schema: SHELL_CMD_SCHEMA,
                optional: true,
            },
        },
    },
    returns: {
        type: Object,
        description: "Object with the user, ticket, port and upid",
        properties: {
            user: {
                description: "",
                type: String,
            },
            ticket: {
                description: "",
                type: String,
            },
            port: {
                description: "",
                type: String,
            },
            upid: {
                description: "",
                type: String,
            },
        }
    },
    access: {
        description: "Restricted to users on realm 'pam'",
        permission: &Permission::Privilege(&["system"], PRIV_SYS_CONSOLE, false),
    }
)]
/// Call termproxy and return shell ticket
async fn termproxy(cmd: Option<String>, rpcenv: &mut dyn RpcEnvironment) -> Result<Value, Error> {
    // intentionally user only for now
    let auth_id: Authid = rpcenv
        .get_auth_id()
        .ok_or_else(|| format_err!("no authid available"))?
        .parse()?;

    if auth_id.is_token() {
        bail!("API tokens cannot access this API endpoint");
    }

    let userid = auth_id.user();

    if userid.realm() != "pam" {
        bail!("only pam users can use the console");
    }

    let path = "/system";

    // use port 0 and let the kernel decide which port is free
    let listener = TcpListener::bind("localhost:0")?;
    let port = listener.local_addr()?.port();

    let ticket = Ticket::new(crate::auth::TERM_PREFIX, &Empty)?.sign(
        private_auth_keyring(),
        Some(&tools::ticket::term_aad(userid, path, port)),
    )?;

    let mut command = Vec::new();
    match cmd.as_deref() {
        Some("login") | None => {
            command.push("login");
            if userid == "root@pam" {
                command.push("-f");
                command.push("root");
            }
        }
        Some("upgrade") => {
            if userid != "root@pam" {
                bail!("only root@pam can upgrade");
            }
            // TODO: add nicer/safer wrapper like in PVE instead
            command.push("sh");
            command.push("-c");
            command.push("apt full-upgrade; bash -l");
        }
        _ => bail!("invalid command"),
    };

    let username = userid.name().to_owned();
    let upid = WorkerTask::spawn(
        "termproxy",
        None,
        auth_id.to_string(),
        false,
        move |worker| async move {
            // move inside the worker so that it survives and does not close the port
            // remove CLOEXEC from listenere so that we can reuse it in termproxy
            fd_change_cloexec(listener.as_raw_fd(), false)?;

            let mut arguments: Vec<&str> = Vec::new();
            let fd_string = listener.as_raw_fd().to_string();
            arguments.push(&fd_string);
            arguments.extend_from_slice(&[
                "--path",
                path,
                "--perm",
                "Sys.Console",
                "--authport",
                "82",
                "--port-as-fd",
                "--",
            ]);
            arguments.extend_from_slice(&command);

            let mut cmd = tokio::process::Command::new("/usr/bin/termproxy");

            cmd.args(&arguments)
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped());

            let mut child = cmd.spawn().expect("error executing termproxy");

            let stdout = child.stdout.take().expect("no child stdout handle");
            let stderr = child.stderr.take().expect("no child stderr handle");

            let worker_stdout = worker.clone();
            let stdout_fut = async move {
                let mut reader = BufReader::new(stdout).lines();
                while let Some(line) = reader.next_line().await? {
                    worker_stdout.log_message(line);
                }
                Ok::<(), Error>(())
            };

            let worker_stderr = worker.clone();
            let stderr_fut = async move {
                let mut reader = BufReader::new(stderr).lines();
                while let Some(line) = reader.next_line().await? {
                    worker_stderr.log_warning(line);
                }
                Ok::<(), Error>(())
            };

            let mut needs_kill = false;
            let res = tokio::select! {
                res = child.wait() => {
                    let exit_code = res?;
                    if !exit_code.success() {
                        match exit_code.code() {
                            Some(code) => bail!("termproxy exited with {}", code),
                            None => bail!("termproxy exited by signal"),
                        }
                    }
                    Ok(())
                },
                res = stdout_fut => res,
                res = stderr_fut => res,
                res = worker.abort_future() => {
                    needs_kill = true;
                    res.map_err(Error::from)
                }
            };

            if needs_kill {
                if res.is_ok() {
                    child.kill().await?;
                    return Ok(());
                }

                if let Err(err) = child.kill().await {
                    worker.log_warning(format!("error killing termproxy: {}", err));
                } else if let Err(err) = child.wait().await {
                    worker.log_warning(format!("error awaiting termproxy: {}", err));
                }
            }

            res
        },
    )?;

    // FIXME: We're returning the user NAME only?
    Ok(json!({
        "user": username,
        "ticket": ticket,
        "port": port,
        "upid": upid,
    }))
}

#[sortable]
pub const API_METHOD_WEBSOCKET: ApiMethod = ApiMethod::new(
    &ApiHandler::AsyncHttp(&upgrade_to_websocket),
    &ObjectSchema::new(
        "Upgraded to websocket",
        &sorted!([
            ("node", false, &NODE_SCHEMA),
            (
                "vncticket",
                false,
                &StringSchema::new("Terminal ticket").schema()
            ),
            ("port", false, &IntegerSchema::new("Terminal port").schema()),
        ]),
    ),
)
.access(
    Some("The user needs Sys.Console on /system."),
    &Permission::Privilege(&["system"], PRIV_SYS_CONSOLE, false),
);

fn upgrade_to_websocket(
    parts: Parts,
    req_body: Body,
    param: Value,
    _info: &ApiMethod,
    rpcenv: Box<dyn RpcEnvironment>,
) -> ApiResponseFuture {
    async move {
        // intentionally user only for now
        let auth_id: Authid = rpcenv
            .get_auth_id()
            .ok_or_else(|| format_err!("no authid available"))?
            .parse()?;

        if auth_id.is_token() {
            bail!("API tokens cannot access this API endpoint");
        }

        let userid = auth_id.user();
        let ticket = pbs_tools::json::required_string_param(&param, "vncticket")?;
        let port: u16 = pbs_tools::json::required_integer_param(&param, "port")? as u16;

        // will be checked again by termproxy
        Ticket::<Empty>::parse(ticket)?.verify(
            public_auth_keyring(),
            crate::auth::TERM_PREFIX,
            Some(&tools::ticket::term_aad(userid, "/system", port)),
        )?;

        let (ws, response) = WebSocket::new(parts.headers.clone())?;

        proxmox_rest_server::spawn_internal_task(async move {
            let conn: Upgraded = match hyper::upgrade::on(Request::from_parts(parts, req_body))
                .map_err(Error::from)
                .await
            {
                Ok(upgraded) => upgraded,
                _ => bail!("error"),
            };

            let local = tokio::net::TcpStream::connect(format!("localhost:{}", port)).await?;
            ws.serve_connection(conn, local).await
        });

        Ok(response)
    }
    .boxed()
}

#[api]
/// List Nodes (only for compatibility)
fn list_nodes() -> Result<Value, Error> {
    Ok(json!([ { "node": proxmox_sys::nodename().to_string() } ]))
}

pub const SUBDIRS: SubdirMap = &[
    ("apt", &apt::ROUTER),
    ("certificates", &certificates::ROUTER),
    ("config", &config::ROUTER),
    ("disks", &disks::ROUTER),
    ("dns", &dns::ROUTER),
    ("journal", &journal::ROUTER),
    ("network", &network::ROUTER),
    ("report", &report::ROUTER),
    ("rrd", &rrd::ROUTER),
    ("services", &services::ROUTER),
    ("status", &status::ROUTER),
    ("subscription", &subscription::ROUTER),
    ("syslog", &syslog::ROUTER),
    ("tasks", &tasks::ROUTER),
    ("termproxy", &Router::new().post(&API_METHOD_TERMPROXY)),
    ("time", &time::ROUTER),
    (
        "vncwebsocket",
        &Router::new().upgrade(&API_METHOD_WEBSOCKET),
    ),
];

pub const ITEM_ROUTER: Router = Router::new()
    .get(&list_subdirs_api_method!(SUBDIRS))
    .subdirs(SUBDIRS);

pub const ROUTER: Router = Router::new()
    .get(&API_METHOD_LIST_NODES)
    .match_all("node", &ITEM_ROUTER);
