extern crate proxmox_backup;

//use proxmox_backup::tools;
use proxmox_backup::api::router::*;
use proxmox_backup::api::config::*;
use proxmox_backup::server::rest::*;
use proxmox_backup::auth_helpers::*;

use lazy_static::lazy_static;

use futures::future::Future;

use hyper;

fn main() {

    if let Err(err) = syslog::init(
        syslog::Facility::LOG_DAEMON,
        log::LevelFilter::Info,
        Some("proxmox-backup-api")) {
        eprintln!("unable to inititialize syslog: {}", err);
        std::process::exit(-1);
    }

    if let Err(err) = generate_auth_key() {
        eprintln!("unable to generate auth key: {}", err);
        std::process::exit(-1);
    }
    let _ = private_auth_key(); // load with lazy_static

    if let Err(err) = generate_csrf_key() {
        eprintln!("unable to generate csrf key: {}", err);
        std::process::exit(-1);
    }
    let _ = csrf_secret(); // load with lazy_static

    let addr = ([127,0,0,1], 82).into();

    lazy_static!{
       static ref ROUTER: Router = proxmox_backup::api2::router();
    }

    let config = ApiConfig::new(
        env!("PROXMOX_JSDIR"), &ROUTER, RpcEnvironmentType::PRIVILEDGED);

    let rest_server = RestServer::new(config);

    let server = hyper::Server::bind(&addr)
        .serve(rest_server)
        .map_err(|e| eprintln!("server error: {}", e));


    // Run this server for... forever!
    hyper::rt::run(server);
}
