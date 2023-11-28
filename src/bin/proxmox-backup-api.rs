use std::future::Future;
use std::pin::Pin;

use anyhow::{bail, Error};
use futures::*;
use http::Response;
use hyper::{Body, StatusCode};

use proxmox_lang::try_block;
use proxmox_router::RpcEnvironmentType;
use proxmox_sys::fs::CreateOptions;

use proxmox_rest_server::{daemon, ApiConfig, RestServer};

use proxmox_backup::auth_helpers::*;
use proxmox_backup::config;
use proxmox_backup::server::auth::check_pbs_auth;

fn main() {
    pbs_tools::setup_libc_malloc_opts();

    proxmox_backup::tools::setup_safe_path_env();

    if let Err(err) = proxmox_async::runtime::main(run()) {
        eprintln!("Error: {}", err);
        std::process::exit(-1);
    }
}

fn get_index() -> Pin<Box<dyn Future<Output = Response<Body>> + Send>> {
    Box::pin(async move {
        let index = "<center><h1>Proxmox Backup API Server</h1></center>";

        Response::builder()
            .status(StatusCode::OK)
            .header(hyper::header::CONTENT_TYPE, "text/html")
            .body(index.into())
            .unwrap()
    })
}

async fn run() -> Result<(), Error> {
    if let Err(err) = syslog::init(
        syslog::Facility::LOG_DAEMON,
        log::LevelFilter::Info,
        Some("proxmox-backup-api"),
    ) {
        bail!("unable to inititialize syslog - {}", err);
    }

    config::create_configdir()?;

    config::update_self_signed_cert(false)?;

    proxmox_backup::server::create_run_dir()?;
    proxmox_backup::server::create_state_dir()?;
    proxmox_backup::server::create_active_operations_dir()?;
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

    proxmox_backup::auth_helpers::setup_auth_context(true);

    let backup_user = pbs_config::backup_user()?;
    let mut command_sock = proxmox_rest_server::CommandSocket::new(
        proxmox_rest_server::our_ctrl_sock(),
        backup_user.gid,
    );

    let dir_opts = CreateOptions::new()
        .owner(backup_user.uid)
        .group(backup_user.gid);
    let file_opts = CreateOptions::new()
        .owner(backup_user.uid)
        .group(backup_user.gid);

    let config = ApiConfig::new(pbs_buildcfg::JS_DIR, RpcEnvironmentType::PRIVILEGED)
        .index_handler_func(|_, _| get_index())
        .auth_handler_func(|h, m| Box::pin(check_pbs_auth(h, m)))
        .default_api2_handler(&proxmox_backup::api2::ROUTER)
        .enable_access_log(
            pbs_buildcfg::API_ACCESS_LOG_FN,
            Some(dir_opts.clone()),
            Some(file_opts.clone()),
            &mut command_sock,
        )?
        .enable_auth_log(
            pbs_buildcfg::API_AUTH_LOG_FN,
            Some(dir_opts.clone()),
            Some(file_opts.clone()),
            &mut command_sock,
        )?;

    let rest_server = RestServer::new(config);
    proxmox_rest_server::init_worker_tasks(
        pbs_buildcfg::PROXMOX_BACKUP_LOG_DIR_M!().into(),
        file_opts.clone(),
    )?;

    // http server future:
    let server = daemon::create_daemon(
        ([127, 0, 0, 1], 82).into(),
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
        Some(pbs_buildcfg::PROXMOX_BACKUP_API_PID_FN),
    );

    proxmox_rest_server::write_pid(pbs_buildcfg::PROXMOX_BACKUP_API_PID_FN)?;

    let init_result: Result<(), Error> = try_block!({
        proxmox_rest_server::register_task_control_commands(&mut command_sock)?;
        command_sock.spawn()?;
        proxmox_rest_server::catch_shutdown_signal()?;
        proxmox_rest_server::catch_reload_signal()?;
        Ok(())
    });

    if let Err(err) = init_result {
        bail!("unable to start daemon - {}", err);
    }

    // stop gap for https://github.com/tokio-rs/tokio/issues/4730 where the thread holding the
    // IO-driver may block progress completely if it starts polling its own tasks (blocks).
    // So, trigger a notify to parked threads, as we're immediately ready the woken up thread will
    // acquire the IO driver, if blocked, before going to sleep, which allows progress again
    // TODO: remove once tokio solves this at their level (see proposals in linked comments)
    let rt_handle = tokio::runtime::Handle::current();
    std::thread::spawn(move || loop {
        rt_handle.spawn(std::future::ready(()));
        std::thread::sleep(std::time::Duration::from_secs(3));
    });

    server.await?;
    log::info!("server shutting down, waiting for active workers to complete");
    proxmox_rest_server::last_worker_future().await?;

    log::info!("done - exit server");

    Ok(())
}
