use failure::*;
use futures::*;
use lazy_static::lazy_static;

use proxmox::tools::try_block;

//use proxmox_backup::tools;
use proxmox_backup::api_schema::router::*;
use proxmox_backup::api_schema::config::*;
use proxmox_backup::server::rest::*;
use proxmox_backup::server;
use proxmox_backup::tools::daemon;
use proxmox_backup::auth_helpers::*;
use proxmox_backup::config;

#[tokio::main]
async fn main() {
    if let Err(err) = run().await {
        eprintln!("Error: {}", err);
        std::process::exit(-1);
    }
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

    if let Err(err) = generate_auth_key() {
        bail!("unable to generate auth key - {}", err);
    }
    let _ = private_auth_key(); // load with lazy_static

    if let Err(err) = generate_csrf_key() {
        bail!("unable to generate csrf key - {}", err);
    }
    let _ = csrf_secret(); // load with lazy_static

    lazy_static!{
       static ref ROUTER: Router = proxmox_backup::api2::router();
    }

    let config = ApiConfig::new(
        env!("PROXMOX_JSDIR"), &ROUTER, RpcEnvironmentType::PRIVILEGED);

    let rest_server = RestServer::new(config);

    // http server future:
    let server = daemon::create_daemon(
        ([127,0,0,1], 82).into(),
        move |listener, ready| {
            Ok(ready
                .and_then(|_| hyper::Server::builder(listener.incoming())
                    .serve(rest_server)
                    .with_graceful_shutdown(server::shutdown_future())
                    .map_err(Error::from)
                )
                .map(|e| {
                    if let Err(e) = e {
                        eprintln!("server error: {}", e);
                    }
                })
            )
        },
    );

    daemon::systemd_notify(daemon::SystemdNotify::Ready)?;

    let init_result: Result<(), Error> = try_block!({
        server::create_task_control_socket()?;
        server::server_state_init()?;
        Ok(())
    });

    if let Err(err) = init_result {
        bail!("unable to start daemon - {}", err);
    }

    server.await?;

    log::info!("done - exit server");

    Ok(())
}
