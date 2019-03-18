extern crate proxmox_backup;

//use proxmox_backup::tools;
use proxmox_backup::api_schema::router::*;
use proxmox_backup::api_schema::config::*;
use proxmox_backup::server::rest::*;
use proxmox_backup::tools::daemon::ReexecStore;
use proxmox_backup::auth_helpers::*;
use proxmox_backup::config;

use failure::*;
use lazy_static::lazy_static;

use futures::future::Future;
use tokio::prelude::*;

use hyper;

static mut QUIT_MAIN: bool = false;

fn main() {

    if let Err(err) = run() {
        eprintln!("Error: {}", err);
        std::process::exit(-1);
    }
}

fn run() -> Result<(), Error> {
    // This manages data for reloads:
    let mut reexecer = ReexecStore::new();

    if let Err(err) = syslog::init(
        syslog::Facility::LOG_DAEMON,
        log::LevelFilter::Info,
        Some("proxmox-backup-api")) {
        bail!("unable to inititialize syslog - {}", err);
    }

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

    let listener: tokio::net::TcpListener = reexecer.restore(
        "PROXMOX_BACKUP_LISTEN_FD",
        || {
            let addr = ([127,0,0,1], 82).into();
            Ok(tokio::net::TcpListener::bind(&addr)?)
        },
    )?;

    let mut http_server = hyper::Server::builder(listener.incoming())
        .serve(rest_server)
        .map_err(|e| eprintln!("server error: {}", e));

    // signalfd future:

    let signal_handler =
        proxmox_backup::tools::daemon::default_signalfd_stream(
            reexecer,
            || {
                unsafe { QUIT_MAIN = true; }
                Ok(())
            },
        )?
        .map(|si| {
            // debugging...
            eprintln!("received signal: {}", si.ssi_signo);
        })
        .map_err(|e| {
            eprintln!("error from signalfd: {}, shutting down...", e);
            unsafe {
                QUIT_MAIN = true;
            }
        });


    // Combined future for signalfd & http server, we want to quit as soon as either of them ends.
    // Neither of them is supposed to end unless some weird error happens, so just bail out if is
    // the case...
    let mut signal_handler = signal_handler.into_future();
    let main = futures::future::poll_fn(move || {
        // Helper for some diagnostic error messages:
        fn poll_helper<S: Future>(stream: &mut S, name: &'static str) -> bool {
            match stream.poll() {
                Ok(Async::Ready(_)) => {
                    eprintln!("{} ended, shutting down", name);
                    true
                }
                Err(_) => {
                    eprintln!("{} error, shutting down", name);
                    true
                },
                _ => false,
            }
        }
        if poll_helper(&mut http_server, "http server") ||
           poll_helper(&mut signal_handler, "signalfd handler")
        {
            return Ok(Async::Ready(()));
        }

        if unsafe { QUIT_MAIN } {
            eprintln!("shutdown requested");
            Ok(Async::Ready(()))
        } else {
            Ok(Async::NotReady)
        }
    });

    hyper::rt::run(main);

    Ok(())
}
