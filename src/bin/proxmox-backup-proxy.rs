use proxmox_backup::try_block;
use proxmox_backup::configdir;
use proxmox_backup::server;
use proxmox_backup::tools::daemon;
use proxmox_backup::api_schema::router::*;
use proxmox_backup::api_schema::config::*;
use proxmox_backup::server::rest::*;
use proxmox_backup::auth_helpers::*;

use failure::*;
use lazy_static::lazy_static;

use futures::*;
use futures::stream::Stream;

use openssl::ssl::{SslMethod, SslAcceptor, SslFiletype};
use std::sync::Arc;
use tokio_openssl::SslAcceptorExt;

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

    //openssl req -x509 -newkey rsa:4096 -keyout /etc/proxmox-backup/proxy.key -out /etc/proxmox-backup/proxy.pem -nodes
    let key_path = configdir!("/proxy.key");
    let cert_path = configdir!("/proxy.pem");

    let mut acceptor = SslAcceptor::mozilla_intermediate(SslMethod::tls()).unwrap();
    acceptor.set_private_key_file(key_path, SslFiletype::PEM)
        .map_err(|err| format_err!("unable to read proxy key {} - {}", key_path, err))?;
    acceptor.set_certificate_chain_file(cert_path)
        .map_err(|err| format_err!("unable to read proxy cert {} - {}", cert_path, err))?;
    acceptor.check_private_key().unwrap();

    let acceptor = Arc::new(acceptor.build());

    let server = daemon::create_daemon(
        ([0,0,0,0,0,0,0,0], 8007).into(),
        |listener| {
            let connections = listener
                .incoming()
                .map_err(Error::from)
                .and_then(move |sock| {
                    sock.set_nodelay(true).unwrap();
                    sock.set_send_buffer_size(1024*1024).unwrap();
                    sock.set_recv_buffer_size(1024*1024).unwrap();
                    acceptor.accept_async(sock).map_err(|e| e.into())
                })
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
               .with_graceful_shutdown(server::shutdown_future())
               .map_err(|err| eprintln!("server error: {}", err))
            )
        },
    )?;

    daemon::systemd_notify(daemon::SystemdNotify::Ready)?;

    tokio::run(lazy(||  {

        let init_result: Result<(), Error> = try_block!({
            server::create_task_control_socket()?;
            server::server_state_init()?;
            Ok(())
        });

        if let Err(err) = init_result {
            eprintln!("unable to start daemon - {}", err);
        } else {
            tokio::spawn(server.then(|_| {
                log::info!("done - exit server");
                Ok(())
            }));
        }

        Ok(())
    }));

    Ok(())
}
