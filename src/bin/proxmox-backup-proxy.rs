use std::sync::Arc;

use failure::*;
use futures::*;
use hyper;
use openssl::ssl::{SslMethod, SslAcceptor, SslFiletype};

use proxmox::tools::try_block;
use proxmox::api::RpcEnvironmentType;

use proxmox_backup::configdir;
use proxmox_backup::buildcfg;
use proxmox_backup::server;
use proxmox_backup::tools::daemon;
use proxmox_backup::server::{ApiConfig, rest::*};
use proxmox_backup::auth_helpers::*;

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
        Some("proxmox-backup-proxy")) {
        bail!("unable to inititialize syslog - {}", err);
    }

    let _ = public_auth_key(); // load with lazy_static
    let _ = csrf_secret(); // load with lazy_static

    let mut config = ApiConfig::new(
        buildcfg::JS_DIR, &proxmox_backup::api2::ROUTER, RpcEnvironmentType::PUBLIC);

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
        |listener, ready| {
            let connections = proxmox_backup::tools::async_io::StaticIncoming::from(listener)
                .map_err(Error::from)
                .try_filter_map(move |(sock, _addr)| {
                    let acceptor = Arc::clone(&acceptor);
                    async move {
                        sock.set_nodelay(true).unwrap();
                        sock.set_send_buffer_size(1024*1024).unwrap();
                        sock.set_recv_buffer_size(1024*1024).unwrap();
                        Ok(tokio_openssl::accept(&acceptor, sock)
                            .await
                            .ok() // handshake errors aren't be fatal, so return None to filter
                        )
                    }
                });
            let connections = proxmox_backup::tools::async_io::HyperAccept(connections);

            Ok(ready
                .and_then(|_| hyper::Server::builder(connections)
                    .serve(rest_server)
                    .with_graceful_shutdown(server::shutdown_future())
                    .map_err(Error::from)
                )
                .map_err(|err| eprintln!("server error: {}", err))
                .map(|_| ())
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
