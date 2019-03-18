use proxmox_backup::configdir;
use proxmox_backup::tools;
use proxmox_backup::tools::daemon;
use proxmox_backup::api_schema::router::*;
use proxmox_backup::api_schema::config::*;
use proxmox_backup::server::rest::*;
use proxmox_backup::auth_helpers::*;

use failure::*;
use lazy_static::lazy_static;

use futures::stream::Stream;
use tokio::prelude::*;

use hyper;

fn main() {

    if let Err(err) = run() {
        eprintln!("Error: {}", err);
        std::process::exit(-1);
    }
}

fn run() -> Result<(), Error> {
    if let Err(err) = syslog::init(
        syslog::Facility::LOG_DAEMON,
        log::LevelFilter::Info,
        Some("proxmox-backup-proxy")) {
        bail!("unable to inititialize syslog - {}", err);
    }

    let _ = public_auth_key(); // load with lazy_static
    let _ = csrf_secret(); // load with lazy_static

    lazy_static!{
       static ref ROUTER: Router = proxmox_backup::api2::router();
    }

    let mut config = ApiConfig::new(
        env!("PROXMOX_JSDIR"), &ROUTER, RpcEnvironmentType::PUBLIC);

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

    let cert_path = configdir!("/proxy.pfx");
    let raw_cert = tools::file_get_contents(cert_path)?;

    let identity = match native_tls::Identity::from_pkcs12(&raw_cert, "") {
        Ok(data) => data,
        Err(err) => bail!("unabled to decode pkcs12 identity {} - {}", cert_path, err),
    };

    let server = daemon::create_daemon(
        ([0,0,0,0,0,0,0,0], 8007).into(),
        |listener| {
            let acceptor = native_tls::TlsAcceptor::new(identity)?;
            let acceptor = std::sync::Arc::new(tokio_tls::TlsAcceptor::from(acceptor));
            let connections = listener
                .incoming()
                .map_err(Error::from)
                .and_then(move |sock| acceptor.accept(sock).map_err(|e| e.into()))
                .then(|r| match r {
                    // accept()s can fail here with an Err() when eg. the client rejects
                    // the cert and closes the connection, so we follow up with mapping
                    // it to an option and then filtering None with filter_map
                    Ok(c) => Ok::<_, Error>(Some(c)),
                    Err(e) => {
                        if let Some(_io) = e.downcast_ref::<std::io::Error>() {
                            // "real" IO errors should not simply be ignored
                            bail!("shutting down...");
                        } else {
                            // handshake errors just get filtered by filter_map() below:
                            Ok(None)
                        }
                    }
                })
                .filter_map(|r| {
                    // Filter out the Nones
                    r
                });
            Ok(hyper::Server::builder(connections)
                .serve(rest_server)
                .map_err(|e| eprintln!("server error: {}", e))
            )
        },
    )?;

    hyper::rt::run(server);
    Ok(())
}
