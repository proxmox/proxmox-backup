use std::future::Future;
use std::pin::Pin;

use anyhow::{bail, Error};
use futures::*;
use http::request::Parts;
use http::Response;
use hyper::{Body, StatusCode};
use hyper::header;

use proxmox::try_block;
use proxmox::api::RpcEnvironmentType;
use proxmox::tools::fs::CreateOptions;

use proxmox_rest_server::{daemon, ApiConfig, RestServer, RestEnvironment};

use proxmox_backup::server::auth::default_api_auth;
use proxmox_backup::auth_helpers::*;
use proxmox_backup::config;

fn main() {
    proxmox_backup::tools::setup_safe_path_env();

    if let Err(err) = pbs_runtime::main(run()) {
        eprintln!("Error: {}", err);
        std::process::exit(-1);
    }
}

fn get_index<'a>(
    _env: RestEnvironment,
    _parts: Parts,
) -> Pin<Box<dyn Future<Output = Response<Body>> + Send + 'a>> {
    Box::pin(async move {

        let index = "<center><h1>Proxmox Backup API Server</h1></center>";

        Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, "text/html")
            .body(index.into())
            .unwrap()
    })
}

async fn run() -> Result<(), Error> {
    if let Err(err) = syslog::init(
        syslog::Facility::LOG_DAEMON,
        log::LevelFilter::Info,
        Some("proxmox-backup-api")) {
        bail!("unable to inititialize syslog - {}", err);
    }

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
        &get_index,
    )?;

    let backup_user = pbs_config::backup_user()?;
    let mut commando_sock = proxmox_rest_server::CommandSocket::new(proxmox_rest_server::our_ctrl_sock(), backup_user.gid);

    let dir_opts = CreateOptions::new().owner(backup_user.uid).group(backup_user.gid);
    let file_opts = CreateOptions::new().owner(backup_user.uid).group(backup_user.gid);

    config.enable_access_log(
        pbs_buildcfg::API_ACCESS_LOG_FN,
        Some(dir_opts.clone()),
        Some(file_opts.clone()),
        &mut commando_sock,
    )?;

    config.enable_auth_log(
        pbs_buildcfg::API_AUTH_LOG_FN,
        Some(dir_opts.clone()),
        Some(file_opts.clone()),
        &mut commando_sock,
    )?;


    let rest_server = RestServer::new(config);
    proxmox_rest_server::init_worker_tasks(pbs_buildcfg::PROXMOX_BACKUP_LOG_DIR_M!().into(), file_opts.clone())?;

    // http server future:
    let server = daemon::create_daemon(
        ([127,0,0,1], 82).into(),
        move |listener| {
            let incoming = hyper::server::conn::AddrIncoming::from_listener(listener)?;

            Ok(async {
                daemon::systemd_notify(daemon::SystemdNotify::Ready)?;

                hyper::Server::builder(incoming)
                    .serve(rest_server)
                    .with_graceful_shutdown(proxmox_rest_server::shutdown_future())
                    .map_err(Error::from)
                    .await
            })
        },
    );

    proxmox_rest_server::write_pid(pbs_buildcfg::PROXMOX_BACKUP_API_PID_FN)?;

    let init_result: Result<(), Error> = try_block!({
        proxmox_rest_server::register_task_control_commands(&mut commando_sock)?;
        commando_sock.spawn()?;
        proxmox_rest_server::catch_shutdown_signal()?;
        proxmox_rest_server::catch_reload_signal()?;
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
