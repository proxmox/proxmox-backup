extern crate proxmox_backup;

use std::sync::Arc;

use proxmox_backup::api::schema::*;
use proxmox_backup::api::router::*;
use proxmox_backup::api::config::*;
use proxmox_backup::server::rest::*;
use proxmox_backup::getopts;

//use failure::*;
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

    let command : Arc<Schema> = StringSchema::new("Command.")
        .format(Arc::new(ApiStringFormat::Enum(vec![
            "start".into(),
            "status".into(),
            "stop".into()
        ])))
        .into();

    let schema = ObjectSchema::new("Parameters.")
        .required("command", command);

    let args: Vec<String> = std::env::args().skip(1).collect();

    let options = match getopts::parse_arguments(&args, &vec!["command"], &schema) {
        Ok((options, rest)) => {
            if !rest.is_empty() {
                eprintln!("Error: got additional arguments: {:?}", rest);
                std::process::exit(-1);
            }
            options
        }
        Err(err) => {
            eprintln!("Error: unable to parse arguments:\n{}", err);
            std::process::exit(-1);
        }
    };

    let command = options["command"].as_str().unwrap();

    match command {
        "start" => {
            println!("Starting server.");
        },
        "stop" => {
            println!("Stopping server.");
            std::process::exit(0);
        },
        "status" => {
            println!("Server status.");
             std::process::exit(0);
       },
        _ => {
            eprintln!("got unexpected command {}", command);
            std::process::exit(-1);
        },
    }

    let addr = ([0,0,0,0,0,0,0,0], 8007).into();

    lazy_static!{
       static ref ROUTER: Router = proxmox_backup::api2::router();
    }

    let mut config = ApiConfig::new("/usr/share/javascript/proxmox-backup", &ROUTER);

    // add default dirs which includes jquery and bootstrap
    // my $base = '/usr/share/libpve-http-server-perl';
    // add_dirs($self->{dirs}, '/css/' => "$base/css/");
    // add_dirs($self->{dirs}, '/js/' => "$base/js/");
    // add_dirs($self->{dirs}, '/fonts/' => "$base/fonts/");
    config.add_alias("novnc", "/usr/share/novnc-pve");
    config.add_alias("extjs", "/usr/share/javascript/extjs");
    config.add_alias("fontawesome", "/usr/share/fonts-font-awesome");
    config.add_alias("xtermjs", "/usr/share/pve-xtermjs");
    config.add_alias("widgettoolkit", "/usr/share/javascript/proxmox-widget-toolkit");

    let rest_server = RestServer::new(config);

    let server = hyper::Server::bind(&addr)
        .serve(rest_server)
        .map_err(|e| eprintln!("server error: {}", e));


    // Run this server for... forever!
    hyper::rt::run(server);
}
