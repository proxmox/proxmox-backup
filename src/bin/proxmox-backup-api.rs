use anyhow::{bail, Error};
use futures::*;
use http::request::Parts;
use http::Response;
use hyper::{Body, StatusCode};
use hyper::header;

use proxmox::try_block;
use proxmox::api::RpcEnvironmentType;
use proxmox::tools::fs::CreateOptions;

use pbs_tools::auth::private_auth_key;
use proxmox_rest_server::ApiConfig;

use proxmox_backup::server::{
    self,
    auth::default_api_auth,
    rest::*,
};
use proxmox_rest_server::daemon;

use proxmox_backup::auth_helpers::*;
use proxmox_backup::config;

fn main() {
    proxmox_backup::tools::setup_safe_path_env();

    if let Err(err) = pbs_runtime::main(run()) {
        eprintln!("Error: {}", err);
        std::process::exit(-1);
    }
}

fn get_index(
    _auth_id: Option<String>,
    _language: Option<String>,
    _api: &ApiConfig,
    _parts: Parts,
) -> Response<Body> {

    let index = "<center><h1>Proxmox Backup API Server</h1></center>";

    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "text/html")
        .body(index.into())
        .unwrap()
}

async fn run() -> Result<(), Error> {
    if let Err(err) = syslog::init(
        syslog::Facility::LOG_DAEMON,
        log::LevelFilter::Info,
        Some("proxmox-backup-api")) {
        bail!("unable to inititialize syslog - {}", err);
    }

    server::create_task_log_dirs()?;

    config::create_configdir()?;

    config::update_self_signed_cert(false)?;

    proxmox_backup::server::create_run_dir()?;

    proxmox_backup::rrd::create_rrdb_dir()?;
    proxmox_backup::server::jobstate::create_jobstate_dir()?;
    proxmox_backup::tape::create_tape_status_dir()?;
    proxmox_backup::tape::create_drive_state_dir()?;
    proxmox_backup::tape::create_changer_state_dir()?;
    proxmox_backup::tape::create_drive_lock_dir()?;

    if let Err(err) = generate_auth_key() {
        bail!("unable to generate auth key - {}", err);
    }
    let _ = private_auth_key(); // load with lazy_static

    if let Err(err) = generate_csrf_key() {
        bail!("unable to generate csrf key - {}", err);
    }
    let _ = csrf_secret(); // load with lazy_static

    let mut config = ApiConfig::new(
        pbs_buildcfg::JS_DIR,
        &proxmox_backup::api2::ROUTER,
        RpcEnvironmentType::PRIVILEGED,
        default_api_auth(),
        get_index,
    )?;

    let backup_user = pbs_config::backup_user()?;
    let mut commando_sock = proxmox_rest_server::CommandoSocket::new(crate::server::our_ctrl_sock(), backup_user.gid);

    let dir_opts = CreateOptions::new().owner(backup_user.uid).group(backup_user.gid);
    let file_opts = CreateOptions::new().owner(backup_user.uid).group(backup_user.gid);

    config.enable_file_log(
        pbs_buildcfg::API_ACCESS_LOG_FN,
        Some(dir_opts),
        Some(file_opts),
        &mut commando_sock,
    )?;

    let rest_server = RestServer::new(config);

    // http server future:
    let server = daemon::create_daemon(
        ([127,0,0,1], 82).into(),
        move |listener, ready| {
            let incoming = proxmox_backup::tools::async_io::StaticIncoming::from(listener);
            Ok(ready
                .and_then(|_| hyper::Server::builder(incoming)
                    .serve(rest_server)
                    .with_graceful_shutdown(proxmox_rest_server::shutdown_future())
                    .map_err(Error::from)
                )
                .map(|e| {
                    if let Err(e) = e {
                        eprintln!("server error: {}", e);
                    }
                })
            )
        },
        "proxmox-backup.service",
    );

    server::write_pid(pbs_buildcfg::PROXMOX_BACKUP_API_PID_FN)?;
    daemon::systemd_notify(daemon::SystemdNotify::Ready)?;

    let init_result: Result<(), Error> = try_block!({
        server::register_task_control_commands(&mut commando_sock)?;
        commando_sock.spawn()?;
        proxmox_rest_server::server_state_init()?;
        Ok(())
    });

    if let Err(err) = init_result {
        bail!("unable to start daemon - {}", err);
    }

    server.await?;
    log::info!("server shutting down, waiting for active workers to complete");
    proxmox_rest_server::last_worker_future().await?;

    log::info!("done - exit server");

    Ok(())
}
