use proxmox_backup::configdir;
use proxmox_backup::tools;
use proxmox_backup::tools::daemon::ReexecStore;
use proxmox_backup::api_schema::router::*;
use proxmox_backup::api_schema::config::*;
use proxmox_backup::server::rest::*;
use proxmox_backup::auth_helpers::*;

use failure::*;
use lazy_static::lazy_static;

use futures::stream::Stream;
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

    // This manages data for reloads:
    let mut reexecer = ReexecStore::new();

    // http server future:

    let listener: tokio::net::TcpListener = reexecer.restore(
        "PROXMOX_BACKUP_LISTEN_FD",
        || {
            let addr = ([0,0,0,0,0,0,0,0], 8007).into();
            Ok(tokio::net::TcpListener::bind(&addr)?)
        },
    )?;
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

    let mut http_server = hyper::Server::builder(connections)
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
